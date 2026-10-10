//! The tmxr server.
//!
//! One thread owns every session, window and pane and processes events in
//! order ([`server`]); connection threads ([`conn`]) and pane threads feed it
//! through a channel. Each attached client gets frames composed by
//! [`render`] into a ratatui buffer and diffed into ANSI by [`backend`].
//! See `docs/plan/02-architecture.md`.

pub mod access;
pub mod backend;
pub mod cmds;
pub mod conn;
pub mod copy;
pub mod hooks;
pub mod jobs;
pub mod keys;
pub mod layout;
pub mod menu;
pub mod model;
pub mod mouse;
pub mod moves;
pub mod output;
pub mod overlay;
pub mod pipe;
pub mod popup;
pub mod render;
pub mod resurrect;
pub mod server;
pub mod target;
pub mod util;
pub mod vars;
pub mod waits;

use std::path::PathBuf;
use std::sync::mpsc;

use tmxr_proto::socket::Endpoint;

pub use server::Server;

/// Close every file descriptor the server inherited beyond stdin, stdout and
/// stderr, as tmux's daemon does: one its starter left open (a pipe a script
/// reads to end-of-file) would otherwise stay open as long as the server
/// runs. Call first thing in the server process, before anything opens a
/// descriptor of its own. On Windows the client starts the server inheriting
/// no handles at all.
#[cfg(unix)]
pub fn close_inherited_fds() {
    /// The highest descriptor tried when the limit is unknown or very large.
    const SCAN_LIMIT: libc::c_long = 65_536;
    // SAFETY: sysconf has no preconditions.
    let open_max = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
    let limit = if open_max < 0 {
        SCAN_LIMIT
    } else {
        open_max.min(SCAN_LIMIT)
    };
    for fd in 3..limit {
        let Ok(fd) = libc::c_int::try_from(fd) else {
            break;
        };
        // SAFETY: nothing in this process owns a descriptor yet; closing one
        // that is not open fails harmlessly with EBADF.
        unsafe { libc::close(fd) };
    }
}

/// Bind `endpoint` and serve until the last session exits or `kill-server`.
/// `config` is the `-f` path, if any.
pub fn run(endpoint: Endpoint, config: Option<PathBuf>) -> std::io::Result<()> {
    let (cfg, errors) = match tmxr_config::load(config.as_deref()) {
        Ok((cfg, _)) => (cfg, None),
        Err(e) => (tmxr_config::defaults(), Some(e.to_string())),
    };
    let socket_access = match cfg.socket_access.as_str() {
        "users" => tmxr_proto::socket::Access::Users,
        "owner" => tmxr_proto::socket::Access::Owner,
        other => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("socket-access: unknown value: {other} (owner or users)"),
            ));
        }
    };
    let listener = endpoint.listen(socket_access)?;
    let owner = access::current_user()?;
    let (tx, rx) = mpsc::channel();
    let mut srv = Server::new(endpoint, cfg, config, tx.clone());
    srv.owner.clone_from(&owner);
    srv.socket_access = socket_access;
    let acl = std::sync::Arc::clone(&srv.acl);
    if let Some(e) = errors {
        tracing::warn!("config: {e}");
        srv.log_message(format!("config error, using defaults: {e}"));
    }
    if srv.cfg.resurrect.restore_on_start
        && let Err(e) = resurrect::restore_on_start(&mut srv)
    {
        srv.log_message(format!("resurrect: {e}"));
    }
    std::thread::Builder::new()
        .name("tmxr-accept".into())
        .spawn(move || conn::accept_loop(listener, tx, &acl, &owner))?;
    server::signals::install();
    srv.run(rx);
    Ok(())
}
