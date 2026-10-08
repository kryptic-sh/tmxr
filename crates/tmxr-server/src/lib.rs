//! The tmxr server.
//!
//! One thread owns every session, window and pane and processes events in
//! order ([`server`]); connection threads ([`conn`]) and pane threads feed it
//! through a channel. Each attached client gets frames composed by
//! [`render`] into a ratatui buffer and diffed into ANSI by [`backend`].
//! See `docs/plan/02-architecture.md`.

pub mod backend;
pub mod cmds;
pub mod conn;
pub mod copy;
pub mod keys;
pub mod layout;
pub mod model;
pub mod mouse;
pub mod moves;
pub mod overlay;
pub mod render;
pub mod resurrect;
pub mod server;
pub mod target;
pub mod util;
pub mod vars;

use std::path::PathBuf;
use std::sync::mpsc;

use tmxr_proto::socket::Endpoint;

pub use server::Server;

/// Bind `endpoint` and serve until the last session exits or `kill-server`.
/// `config` is the `-f` path, if any.
pub fn run(endpoint: Endpoint, config: Option<PathBuf>) -> std::io::Result<()> {
    let listener = endpoint.listen()?;
    let (cfg, errors) = match tmxr_config::load(config.as_deref()) {
        Ok((cfg, _)) => (cfg, None),
        Err(e) => (tmxr_config::defaults(), Some(e.to_string())),
    };
    let (tx, rx) = mpsc::channel();
    let mut srv = Server::new(endpoint, cfg, config, tx.clone());
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
        .spawn(move || conn::accept_loop(listener, tx))?;
    srv.run(rx);
    Ok(())
}
