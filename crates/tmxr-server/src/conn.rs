//! Accepting connections and moving frames between sockets and the server's
//! event channel.
//!
//! Each connection gets a reader thread (frames → [`Event::Msg`]) and a
//! writer thread draining a bounded queue. The bounded queue is the
//! back-pressure: when a slow client's queue is full the server skips frames
//! for it and sends a full redraw once it catches up, instead of buffering
//! without limit.

use std::sync::mpsc::{Receiver, Sender, SyncSender, sync_channel};

use interprocess::local_socket::Listener;
use interprocess::local_socket::traits::{ListenerExt as _, Stream as _};
use tmxr_proto::{ClientMsg, ServerMsg, read_msg, write_msg};
use tracing::{debug, warn};

use crate::access::SharedAcl;
use crate::model::ClientId;
use crate::server::Event;

/// Frames queued per client before the server starts skipping frames.
const CLIENT_QUEUE: usize = 256;

/// Accept connections forever, announcing each one `acl` admits to
/// `owner`'s server.
pub fn accept_loop(listener: Listener, events: Sender<Event>, acl: &SharedAcl, owner: &str) {
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
        if let Err(e) = spawned {
            warn!(error = %e, "could not start client threads");
            let _ = events.send(Event::Disconnected(id));
        }
    }
}
