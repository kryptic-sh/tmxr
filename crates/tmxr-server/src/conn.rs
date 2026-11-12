//! Accepting connections and moving frames between sockets and the server's
//! event channel.
//!
//! Each connection gets a reader thread (frames → [`Event::Msg`]) and a
//! writer thread draining a bounded queue. The bounded queue is the
//! back-pressure: when a slow client's queue is full the server skips frames
//! for it and sends a full redraw once it catches up, instead of buffering
//! without limit.

use std::sync::mpsc::{Receiver, Sender, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use interprocess::local_socket::Listener;
use interprocess::local_socket::traits::ListenerExt as _;
use tmxr_proto::{ClientMsg, ProtoError, ServerMsg, read_msg, write_msg};
use tracing::{debug, warn};

use crate::access::SharedAcl;
use crate::model::ClientId;
use crate::server::Event;

/// Frames queued per client before the server starts skipping frames.
const CLIENT_QUEUE: usize = 256;

/// The client writer threads still running.
pub type Writers = Arc<Mutex<Vec<JoinHandle<()>>>>;

/// How long an exiting server waits for its clients' last frames to go out.
const FLUSH_LIMIT: Duration = Duration::from_secs(1);

/// Let every writer send what is queued: a `kill-server`'s reply, the
/// "server exited" notice. Once the server has dropped its clients each
/// queue ends and its writer returns; without this wait the process could
/// exit first, and the command client would see the connection drop instead
/// of its answer. Bounded by [`FLUSH_LIMIT`], for a client not reading.
pub fn flush(writers: &Writers) {
    let deadline = Instant::now() + FLUSH_LIMIT;
    let handles = std::mem::take(&mut *writers.lock().unwrap_or_else(PoisonError::into_inner));
    for handle in handles {
        while !handle.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if handle.is_finished() {
            let _ = handle.join();
        }
    }
}

/// Accept connections forever, announcing each one `acl` admits to
/// `owner`'s server; each writer thread goes in `writers`.
pub fn accept_loop(
    listener: Listener,
    events: Sender<Event>,
    acl: &SharedAcl,
    owner: &str,
    writers: &Writers,
) {
    let mut next_id: ClientId = 0;
    for conn in listener.incoming() {
        let stream = match conn {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "accept failed");
                continue;
            }
        };
        let user = match crate::access::peer_user(&stream) {
            Ok(user) => user,
            Err(e) => {
                warn!(error = %e, "could not tell who connected; refusing");
                continue;
            }
        };
        let Some(read_only) = crate::access::lock(acl).admit(owner, &user) else {
            warn!(
                user,
                "refused a connection from a user server-access does not admit"
            );
            continue;
        };
        next_id += 1;
        let id = next_id;
        // Read and written by a thread each, as `&Stream` reads and writes.
        let recv = Arc::new(stream);
        let send = Arc::clone(&recv);
        let (tx, rx): (SyncSender<ServerMsg>, Receiver<ServerMsg>) = sync_channel(CLIENT_QUEUE);
        let connected = Event::Connected {
            id,
            tx,
            user,
            read_only,
        };
        if events.send(connected).is_err() {
            return;
        }
        let ev = events.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("tmxr-client-{id}-read"))
            .spawn(move || {
                loop {
                    wait_readable(&recv);
                    match read_msg::<_, ClientMsg>(&mut &*recv) {
                        Ok(Some(msg)) => {
                            if ev.send(Event::Msg(id, msg)).is_err() {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            debug!(client = id, error = %e, "client read ended");
                            break;
                        }
                    }
                }
                let _ = ev.send(Event::Disconnected(id));
            })
            .and_then(|_| {
                std::thread::Builder::new()
                    .name(format!("tmxr-client-{id}-write"))
                    .spawn(move || {
                        for msg in rx {
                            match write_msg(&mut &*send, &msg) {
                                Ok(()) => {}
                                // Nothing of a message too large was sent,
                                // so the connection still works: a reply's
                                // output goes ahead of it in pieces, and
                                // anything else is dropped.
                                Err(ProtoError::TooLarge(n)) => {
                                    if let ServerMsg::CommandResult {
                                        status,
                                        stdout,
                                        stderr,
                                    } = msg
                                        && write_in_pieces(&mut &*send, status, &stdout, stderr, n)
                                            .is_err()
                                    {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        // The server forgot the client, or the client is
                        // gone. The reader thread still holds the stream,
                        // blocked reading it, so the connection stays open
                        // until it is hung up here.
                        if let Err(e) = hang_up(&send) {
                            debug!(client = id, error = %e, "could not hang up");
                        }
                    })
            });
        match spawned {
            Ok(writer) => {
                let mut all = writers.lock().unwrap_or_else(PoisonError::into_inner);
                all.retain(|w| !w.is_finished());
                all.push(writer);
            }
            Err(e) => {
                warn!(error = %e, "could not start client threads");
                let _ = events.send(Event::Disconnected(id));
            }
        }
    }
}

/// Wait until the client has sent something, or the connection ended, so
/// the read after it does not block. A thread blocked in `read` on a Unix
/// socket is woken each time the client takes what the writer thread sent
/// (Linux wakes every waiter on the socket for the room freed, seen with a
/// socketpair on 2026-10-23: one wakeup a write, none with `poll` first),
/// so reading straight away woke the reader with every frame; `poll` for
/// input wakes only for input. Verified on Linux only.
#[cfg(unix)]
fn wait_readable(recv: &interprocess::local_socket::Stream) {
    use std::os::fd::{AsFd as _, AsRawFd as _};
    let interprocess::local_socket::Stream::UdSocket(socket) = recv;
    let mut pfd = libc::pollfd {
        fd: socket.as_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        // SAFETY: polls one pollfd naming the socket, which `recv` keeps
        // open for the call.
        let ready = unsafe { libc::poll(&mut pfd, 1, -1) };
        if ready >= 0 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return;
        }
    }
}

/// On Windows the named pipe's read waits by itself; whether it wakes
/// for frames sent the other way was not measured.
#[cfg(windows)]
fn wait_readable(_: &interprocess::local_socket::Stream) {}

/// End the connection both ways once nothing more will be written: the
/// client reads what was sent and then the end of the stream. A Unix socket
/// is shut down for reading and writing, which wakes this side's reader
/// thread; frames the client sent that are already queued may still be
/// read and delivered to the server before it reads the end of the stream
/// and drops its share of the stream.
#[cfg(unix)]
fn hang_up(conn: &interprocess::local_socket::Stream) -> std::io::Result<()> {
    use std::os::fd::{AsFd as _, AsRawFd as _};
    let interprocess::local_socket::Stream::UdSocket(socket) = conn;
    // SAFETY: shutdown takes no pointers, and names the socket `conn`
    // keeps open for the call.
    if unsafe { libc::shutdown(socket.as_fd().as_raw_fd(), libc::SHUT_RDWR) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// End the connection both ways once nothing more will be written. A named
/// pipe is flushed first, which waits until the client has read everything
/// sent, as `DisconnectNamedPipe` discards what it has not; disconnecting
/// then fails the client's read and this side's, now and later, which
/// interprocess reads as the end of the stream.
#[cfg(windows)]
fn hang_up(conn: &interprocess::local_socket::Stream) -> std::io::Result<()> {
    use std::os::windows::io::{AsHandle as _, AsRawHandle as _};
    use windows_sys::Win32::System::Pipes::DisconnectNamedPipe;
    let interprocess::local_socket::Stream::NamedPipe(pipe) = conn;
    if let Err(e) = pipe.inner().flush() {
        // A client already gone has nothing left to read; disconnect still
        // wakes the reader.
        debug!(error = %e, "pipe not flushed");
    }
    // SAFETY: takes only the pipe's handle, which `conn` keeps open for the
    // call.
    if unsafe { DisconnectNamedPipe(pipe.as_handle().as_raw_handle()) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// A reply too large for one frame (`size` bytes): its output in pieces
/// the client writes as they come, then the reply without it; if that is
/// still too large (its errors), an error saying so.
fn write_in_pieces<W: std::io::Write>(
    send: &mut W,
    status: i32,
    stdout: &str,
    stderr: String,
    size: usize,
) -> Result<(), ProtoError> {
    for piece in stdout.as_bytes().chunks(tmxr_proto::STDIN_PIECE) {
        write_msg(send, &ServerMsg::Output(piece.to_vec()))?;
    }
    let rest = ServerMsg::CommandResult {
        status,
        stdout: String::new(),
        stderr,
    };
    match write_msg(send, &rest) {
        Err(ProtoError::TooLarge(_)) => write_msg(
            send,
            &ServerMsg::CommandResult {
                status: 1,
                stdout: String::new(),
                stderr: format!(
                    "the reply, {size} bytes, is over the {}-byte limit",
                    tmxr_proto::codec::MAX_FRAME
                ),
            },
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use tmxr_proto::socket::{Access, Endpoint};

    use super::*;

    /// Long past any hang-up, short of hanging the suite.
    const WAIT: Duration = Duration::from_secs(10);

    /// A client the server forgets, as `finish_command` forgets a command
    /// client after its reply, reads what was queued and then the end of
    /// the stream, though it never hangs up itself; and the reader thread
    /// blocked on its connection ends too.
    #[test]
    fn a_connection_the_server_drops_is_closed() {
        let tmp = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let path = tmp.path().join("conn");
        #[cfg(windows)]
        let path = {
            let _ = &tmp;
            std::path::PathBuf::from(format!(r"\\.\pipe\tmxr-test-conn-{}", std::process::id()))
        };
        let endpoint = Endpoint::resolve(Some(&path), None).unwrap();
        let listener = endpoint.listen(Access::Owner).unwrap();
        let (events_tx, events) = channel();
        let owner = crate::access::current_user().unwrap();
        std::thread::spawn(move || {
            accept_loop(
                listener,
                events_tx,
                &SharedAcl::default(),
                &owner,
                &Writers::default(),
            );
        });

        let client = endpoint.connect().unwrap();
        let Ok(Event::Connected { id, tx, .. }) = events.recv_timeout(WAIT) else {
            panic!("the server did not see the client connect");
        };
        let last = ServerMsg::CommandResult {
            status: 0,
            stdout: "done\n".into(),
            stderr: String::new(),
        };
        tx.send(last.clone()).unwrap();
        // The server forgets the client: its queue goes.
        drop(tx);

        let (read_tx, read) = channel();
        std::thread::spawn(move || {
            let first = read_msg::<_, ServerMsg>(&mut &client).map_err(|e| e.to_string());
            let second = read_msg::<_, ServerMsg>(&mut &client).map_err(|e| e.to_string());
            let _ = read_tx.send((first, second));
        });
        let (first, second) = read
            .recv_timeout(WAIT)
            .expect("the client was left waiting on a connection nothing writes to");
        assert_eq!(first, Ok(Some(last)));
        assert_eq!(second, Ok(None), "the end of the stream after the reply");
        match events.recv_timeout(WAIT) {
            Ok(Event::Disconnected(gone)) => assert_eq!(gone, id),
            _ => panic!("the reader thread did not end"),
        }
    }
}
