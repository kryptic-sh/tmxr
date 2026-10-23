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
use interprocess::local_socket::traits::{ListenerExt as _, Stream as _};
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
        let (mut recv, mut send) = stream.split();
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
                    match read_msg::<_, ClientMsg>(&mut recv) {
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
                            match write_msg(&mut send, &msg) {
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
                                        && write_in_pieces(&mut send, status, &stdout, stderr, n)
                                            .is_err()
                                    {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
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
fn wait_readable(recv: &interprocess::local_socket::RecvHalf) {
    use std::os::fd::{AsFd as _, AsRawFd as _};
    let interprocess::local_socket::RecvHalf::UdSocket(half) = recv;
    let mut pfd = libc::pollfd {
        fd: half.as_fd().as_raw_fd(),
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
fn wait_readable(_: &interprocess::local_socket::RecvHalf) {}

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
