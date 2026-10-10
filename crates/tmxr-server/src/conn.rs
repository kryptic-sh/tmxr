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
use tmxr_proto::{ClientMsg, ServerMsg, read_msg, write_msg};
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
                            if write_msg(&mut send, &msg).is_err() {
                                break;
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
