//! The tmxr server.
//!
//! One thread owns every session, window and pane and processes events in
//! order ([`server`]); connection threads ([`conn`]) and pane threads feed it
//! through a channel. Each attached client gets frames composed by
//! [`render`] into a ratatui buffer and diffed into ANSI by [`backend`].
//! See `docs/plan/02-architecture.md`.

pub mod access;
pub mod alerts;
pub mod backend;
pub mod clipboard;
pub mod cmds;
pub mod conn;
pub mod copy;
pub mod copyline;
pub mod format_draw;
pub mod format_jobs;
pub mod groups;
pub mod hooks;
pub mod jobs;
pub mod keys;
pub mod layout;
pub mod menu;
pub mod model;
pub mod mouse;
pub mod moves;
pub mod notify;
pub mod output;
pub mod overlay;
pub mod pipe;
pub mod popup;
pub mod render;
pub mod requests;
pub mod resurrect;
pub mod scrollbar;
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
    // Settings the file gets wrong are left out and named, the rest kept,
    // as tmux keeps the config lines that work.
    let (cfg, cfg_file, errors) = match tmxr_config::load_keeping(config.as_deref()) {
        Ok((cfg, source, left_out)) if left_out.is_empty() => {
            (cfg, server::config_file(source), None)
        }
        Ok((cfg, source, left_out)) => (
            cfg,
            server::config_file(source),
            Some(format!("settings left out: {}", left_out.join("; "))),
        ),
        Err(e) => (
            tmxr_config::defaults(),
            None,
            Some(format!("using defaults: {e}")),
        ),
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
    // The server is made and runs on a thread of its own so its stack is
    // SERVER_STACK on every platform: the main thread's is 1 MiB on
    // Windows, which a config nested to source-file's limit, or a deep
    // layout, runs past.
    std::thread::Builder::new()
        .name("server".into())
        .stack_size(SERVER_STACK)
        .spawn(move || {
            serve(
                endpoint,
                (cfg, config, cfg_file, errors),
                socket_access,
                listener,
                owner,
            )
        })?
        .join()
        .map_err(|_| std::io::Error::other("the server thread panicked"))?
}

/// Make the server and run it until it exits.
fn serve(
    endpoint: Endpoint,
    (cfg, config, cfg_file, errors): (
        tmxr_config::Config,
        Option<PathBuf>,
        Option<PathBuf>,
        Option<String>,
    ),
    socket_access: tmxr_proto::socket::Access,
    listener: interprocess::local_socket::Listener,
    owner: access::UserId,
) -> std::io::Result<()> {
    let (tx, rx) = mpsc::channel();
    let mut srv = Server::new(endpoint, cfg, config, tx);
    srv.owner.clone_from(&owner);
    srv.cfg_file = cfg_file;
    srv.socket_access = socket_access;
    if let Some(e) = errors {
        tracing::warn!("config: {e}");
        let message = format!("config error, {e}");
        srv.log_message(message.clone());
        srv.config_error = Some(message);
    }
    // After the config, as tmux's cfg_done.
    srv.load_prompt_history();
    if srv.cfg.resurrect.restore_on_start
        && let Err(e) = resurrect::restore_on_start(&mut srv)
    {
        srv.log_message(format!("resurrect: {e}"));
    }
    srv.spawn_accept(listener)?;
    let writers = std::sync::Arc::clone(&srv.writers);
    server::signals::install();
    // Takes the server, so its clients' queues are closed when it returns.
    srv.run(rx);
    conn::flush(&writers);
    Ok(())
}

/// The server thread's stack: what Linux gives a main thread.
const SERVER_STACK: usize = 8 * 1024 * 1024;
