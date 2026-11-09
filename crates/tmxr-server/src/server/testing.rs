//! A server for unit tests, with no socket, panes or sessions.

use std::sync::mpsc;

use tmxr_proto::{Hello, ServerMsg};

use super::{Client, Server};

/// A server with client 1 connected, its hello `hello`, not attached; and
/// what the server sends the client.
pub fn with_client(hello: Option<Hello>) -> (Server, mpsc::Receiver<ServerMsg>) {
    let (events, _) = mpsc::channel();
    let endpoint =
        tmxr_proto::socket::Endpoint::resolve(Some(std::path::Path::new("test")), None).unwrap();
    let mut srv = Server::new(endpoint, tmxr_config::defaults(), None, events);
    let (tx, rx) = mpsc::sync_channel(64);
    srv.clients.insert(
        1,
        Client {
            id: 1,
            tx,
            hello,
            att: None,
            user: String::new(),
            user_read_only: false,
            attach_read_only: false,
            ignore_size: false,
            no_detach_on_destroy: false,
            stdin: None,
            created: std::time::SystemTime::now(),
            terminal_colours: (None, None),
            theme: None,
            colours_asked: None,
        },
    );
    (srv, rx)
}
