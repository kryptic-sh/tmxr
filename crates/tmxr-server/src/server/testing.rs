//! A server for unit tests, with no socket, panes or sessions.

use std::sync::mpsc;

use tmxr_proto::{ClientMsg, Hello, KeyboardReport, PROTOCOL_VERSION, ServerMsg, TerminalInfo};

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

/// A server with client 1, in a terminal whose `TERM` is `xterm-256color`,
/// attached to session 0 (which does not exist), with `kitty-keys` as
/// `kitty_keys`; and what the server sends the client.
pub fn attached(kitty_keys: bool) -> (Server, mpsc::Receiver<ServerMsg>) {
    let hello = Hello {
        protocol: PROTOCOL_VERSION,
        version: String::new(),
        cwd: String::new(),
        pid: 0,
        env: Vec::new(),
        terminal: Some(TerminalInfo {
            cols: 80,
            rows: 24,
            term: "xterm-256color".into(),
            job_control: false,
            tty: String::new(),
            colour_answers: false,
        }),
    };
    let (mut srv, rx) = with_client(Some(hello));
    srv.cfg.kitty_keys = kitty_keys;
    srv.attach(1, 0);
    (srv, rx)
}

/// What a Unix client says of a terminal that answered the kitty query
/// (`kitty`) and XTVERSION with `version`.
pub fn report(kitty: bool, version: Option<&str>) -> KeyboardReport {
    KeyboardReport {
        kitty,
        version: version.map(str::to_owned),
        secondary: None,
        modify_other_keys: true,
    }
}

/// Client 1 says `report` of its terminal's keyboard.
pub fn say_keyboard(srv: &mut Server, report: KeyboardReport) {
    srv.client_msg(1, ClientMsg::Keyboard(report));
}

/// Mark client 1 for a frame and draw it, as anything that changes what it
/// is told does.
pub fn redraw(srv: &mut Server) {
    if let Some(att) = srv.clients.get_mut(&1).and_then(|c| c.att.as_mut()) {
        att.dirty = true;
    }
    srv.render_all();
}
