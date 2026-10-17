//! Session commands, and saving and restoring them.

use std::fmt::Write as _;

use tmxr_command::Parsed;
use tmxr_proto::ServerMsg;

use super::{Ctx, Outcome, attached_client, client_size, cwd_arg, list_item, session_env};
use crate::model::{ClientId, Environment, SessionId};
use crate::server::Server;
use crate::target;
use crate::vars::Vars;
use tmxr_command::Args;

pub(super) fn run(
    srv: &mut Server,
    ctx: &Ctx,
    p: &Parsed,
    out: &mut Outcome,
) -> Result<bool, String> {
    let a = &p.args;
    let pos = a.positional();
    match p.name() {
        "new-session" => {
            // -A: a session of that name already there is attached instead
            // (even with -d), as tmux's, -D as attach-session's -d.
            if a.has('A')
                && let Some(name) = a.value('s')
                && let Some(sid) = srv.sessions.values().find(|s| s.name == name).map(|s| s.id)
            {
                let others = Others::from_flags(a, 'D', 'X');
                attach_to(srv, ctx, a, sid, others, out)?;
                return Ok(true);
            }
            let flags = client_flags(a.value('f'))?;
            // -x / -y are the window's size, as tmux's: no status row comes
            // out of them.
            let size = match (a.value('x'), a.value('y')) {
                (Some(x), Some(y)) => (
                    x.parse().map_err(|_| "bad -x")?,
                    y.parse::<u16>().map_err(|_| "bad -y")?.max(1),
                ),
                _ => client_size(srv, ctx),
            };
            let cwd = cwd_arg(srv, ctx, None, a)
                .or_else(|| ctx.cwd.clone())
                .unwrap_or_else(crate::util::home_dir);
            // -e sets the new session's environment, after the client's
            // (none of it with -E).
            let mut env = if a.has('E') {
                Environment::default()
            } else {
                session_env(srv, ctx)
            };
            for (name, value) in super::env_flags(a)? {
                env.set(&name, Some(value));
            }
            let sid = srv.new_session(
                a.value('s').map(str::to_owned),
                cwd,
                env,
                a.value('n').map(str::to_owned),
                pos.to_vec(),
                size,
            )?;
            if !a.has('d') {
                if let Some(c) = ctx.client {
                    set_client_flags(srv, c, &flags);
                }
                match attached_client(srv, ctx) {
                    Some(c) => srv.switch_client(c, sid),
                    None => out.attach = Some(sid),
                }
            }
        }
        "attach-session" => {
            if srv.sessions.is_empty() {
                return Err("no sessions".into());
            }
            let sid = target::session(srv, ctx, a.value('t'))?;
            attach_to(srv, ctx, a, sid, Others::from_flags(a, 'd', 'x'), out)?;
        }
        "detach-client" => {
            let targets: Vec<ClientId> = if a.has('a') {
                srv.clients
                    .values()
                    .filter(|c| c.att.is_some() && Some(c.id) != ctx.client)
                    .map(|c| c.id)
                    .collect()
            } else if let Some(t) = a.value('t') {
                vec![target_client(srv, t)?]
            } else if let Some(s) = a.value('s') {
                let sid = target::session(srv, ctx, Some(s))?;
                srv.clients
                    .values()
                    .filter(|c| c.att.as_ref().is_some_and(|x| x.session == sid))
                    .map(|c| c.id)
                    .collect()
            } else {
                attached_client(srv, ctx).into_iter().collect()
            };
            // -P: their parent shells get SIGHUP too, as tmux's.
            if a.has('P') {
                hang_up(srv, &targets)?;
            } else {
                for c in targets {
                    srv.detach(c, "detached");
                }
            }
        }
        "has-session" => {
            target::session(srv, ctx, a.value('t'))?;
        }
        "kill-server" => srv.begin_exit(),
        "kill-session" => {
            let sid = target::session(srv, ctx, a.value('t'))?;
            if a.has('a') {
                let others: Vec<SessionId> =
                    srv.sessions.keys().copied().filter(|s| *s != sid).collect();
                for s in others {
                    srv.kill_session(s);
                }
            } else {
                srv.kill_session(sid);
            }
        }
        // The client started the server to run this; nothing is left to do.
        "start-server" => {}
        "lock-client" | "lock-session" | "lock-server" => {
            let clients: Vec<ClientId> = match p.name() {
                "lock-client" => vec![match a.value('t') {
                    Some(t) => target_client(srv, t)?,
                    None => super::display_client(srv, ctx).ok_or("no current client")?,
                }],
                "lock-session" => {
                    let sid = target::session(srv, ctx, a.value('t'))?;
                    clients_of(srv, |s| s == sid)
                }
                _ => clients_of(srv, |_| true),
            };
            srv.lock_clients(&clients)?;
        }
        "suspend-client" => {
            let c = match a.value('t') {
                Some(t) => target_client(srv, t)?,
                None => attached_client(srv, ctx).ok_or("no current client")?,
            };
            if !job_control(srv, c) {
                return Err("suspend-client: the client has no job control (Windows)".into());
            }
            srv.send(c, ServerMsg::Suspend);
        }
        "list-clients" => {
            let only = a
                .value('t')
                .map(|t| target::session(srv, ctx, Some(t)))
                .transpose()?;
            for c in srv.clients.values() {
                let Some(att) = c
                    .att
                    .as_ref()
                    .filter(|a| only.is_none_or(|s| s == a.session))
                else {
                    continue;
                };
                let window = srv
                    .sessions
                    .get(&att.session)
                    .and_then(|s| s.current_window());
                let vars = Vars {
                    srv,
                    session: Some(att.session),
                    window,
                    pane: window.and_then(|w| srv.windows.get(&w)).map(|w| w.active),
                    client: Some(c.id),
                };
                let line = list_item(a, &vars, || {
                    let session = srv
                        .sessions
                        .get(&att.session)
                        .map_or("", |s| s.name.as_str());
                    let term = c
                        .hello
                        .as_ref()
                        .and_then(|h| h.terminal.as_ref())
                        .map_or("", |t| t.term.as_str());
                    format!("{}: {session} [{}x{} {term}]", c.id, att.cols, att.rows)
                });
                if let Some(line) = line {
                    let _ = writeln!(out.stdout, "{line}");
                }
            }
        }
        "list-sessions" => {
            for s in srv
                .sessions_by_name()
                .iter()
                .filter_map(|s| srv.sessions.get(s))
            {
                let window = s.current_window();
                let vars = Vars {
                    srv,
                    session: Some(s.id),
                    window,
                    pane: window.and_then(|w| srv.windows.get(&w)).map(|w| w.active),
                    client: None,
                };
                let line = list_item(a, &vars, || {
                    let attached = srv
                        .clients
                        .values()
                        .any(|c| c.att.as_ref().is_some_and(|a| a.session == s.id));
                    format!(
                        "{}: {} windows{}",
                        s.name,
                        s.windows.len(),
                        if attached { " (attached)" } else { "" }
                    )
                });
                if let Some(line) = line {
                    let _ = writeln!(out.stdout, "{line}");
                }
            }
        }
        "rename-session" => {
            let sid = target::session(srv, ctx, a.value('t'))?;
            let name = &pos[0];
            target::check_session_name(name)?;
            if srv
                .sessions
                .values()
                .any(|s| s.id != sid && &s.name == name)
            {
                return Err(format!("duplicate session: {name}"));
            }
            if let Some(s) = srv.sessions.get_mut(&sid) {
                s.name.clone_from(name);
            }
            srv.mark_session_dirty(sid);
        }
        "switch-client" => {
            let c = attached_client(srv, ctx).ok_or("no current client")?;
            if let Some(table) = a.value('T') {
                if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                    table.clone_into(&mut att.table);
                    att.dirty = true;
                }
                return Ok(true);
            }
            let cur = srv.clients[&c]
                .att
                .as_ref()
                .map(|x| x.session)
                .ok_or("not attached")?;
            let sid = if a.has('l') {
                srv.clients[&c]
                    .att
                    .as_ref()
                    .and_then(|x| x.last_session)
                    .filter(|s| srv.sessions.contains_key(s))
                    .ok_or("no last session")?
            } else if a.has('n') || a.has('p') {
                let ids = srv.sessions_by_name();
                let i = ids.iter().position(|s| *s == cur).unwrap_or(0);
                let n = ids.len();
                if a.has('n') {
                    ids[(i + 1) % n]
                } else {
                    ids[(i + n - 1) % n]
                }
            } else {
                target::session(srv, ctx, a.value('t'))?
            };
            // The switching client's variables, unless -E, as tmux's.
            if !a.has('E') {
                let env = srv.clients[&c]
                    .hello
                    .as_ref()
                    .map(|h| h.env.clone())
                    .unwrap_or_default();
                update_env(srv, sid, &env);
            }
            srv.switch_client(c, sid);
        }

        "resurrect-save" => {
            let path = crate::resurrect::save(srv)?;
            if let Some(c) = attached_client(srv, ctx) {
                srv.show_message(c, format!("saved sessions to {}", path.display()));
            }
        }
        "resurrect-restore" => {
            let done = crate::resurrect::restore(srv, client_size(srv, ctx))?;
            if let Some(c) = attached_client(srv, ctx) {
                srv.show_message(c, done.message());
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// An attached client by its id (`list-clients` shows them).
/// The attached clients whose session `on` accepts.
fn clients_of(srv: &Server, on: impl Fn(SessionId) -> bool) -> Vec<ClientId> {
    srv.clients
        .values()
        .filter(|c| c.att.as_ref().is_some_and(|a| on(a.session)))
        .map(|c| c.id)
        .collect()
}

fn target_client(srv: &Server, spec: &str) -> Result<ClientId, String> {
    spec.trim_start_matches('=')
        .parse::<ClientId>()
        .ok()
        .filter(|id| srv.clients.get(id).is_some_and(|c| c.att.is_some()))
        .ok_or_else(|| format!("can't find client: {spec}"))
}

/// Attach the command's client to `sid` (or switch it there), as
/// `attach-session` and `new-session -A` do: `detach_others` (`-d`, `-D`)
/// detaches the session's other clients, `-r` or `-f read-only` makes this
/// one read-only, `-c` sets the session's working directory, and unless `-E`
/// the session takes the client's `update-environment` variables (one the
/// client does not have is marked removed), as tmux's `environ_update`.
fn attach_to(
    srv: &mut Server,
    ctx: &Ctx,
    a: &Args,
    sid: SessionId,
    others: Others,
    out: &mut Outcome,
) -> Result<(), String> {
    let mut flags = client_flags(a.value('f'))?;
    flags.read_only |= a.has('r');
    if let Some(cwd) = cwd_arg(srv, ctx, None, a)
        && let Some(s) = srv.sessions.get_mut(&sid)
    {
        s.cwd = cwd;
    }
    if !a.has('E') {
        update_env(srv, sid, &ctx.env);
    }
    if others != Others::Keep {
        let ids: Vec<ClientId> = srv
            .clients
            .values()
            .filter(|c| c.att.as_ref().is_some_and(|x| x.session == sid))
            .filter(|c| Some(c.id) != ctx.client)
            .map(|c| c.id)
            .collect();
        if others == Others::HangUp {
            hang_up(srv, &ids)?;
        } else {
            for c in ids {
                srv.detach(c, "detached");
            }
        }
    }
    if let Some(c) = ctx.client {
        set_client_flags(srv, c, &flags);
    }
    match attached_client(srv, ctx) {
        Some(c) => srv.switch_client(c, sid),
        None => out.attach = Some(sid),
    }
    Ok(())
}

/// What attaching does to the session's other clients: nothing, detach
/// them (`-d`, `-D`), or detach them and hang up their parent shells (`-x`,
/// `-X`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Others {
    Keep,
    Detach,
    HangUp,
}

impl Others {
    /// The choice the command's `detach` and `hang_up` flags make.
    fn from_flags(a: &Args, detach: char, hang_up: char) -> Self {
        if a.has(hang_up) {
            Self::HangUp
        } else if a.has(detach) {
            Self::Detach
        } else {
            Self::Keep
        }
    }
}

/// Whether client `c` said it has Unix job control (and so a parent it can
/// signal), in its hello.
fn job_control(srv: &Server, c: ClientId) -> bool {
    srv.clients
        .get(&c)
        .and_then(|c| c.hello.as_ref())
        .and_then(|h| h.terminal.as_ref())
        .is_some_and(|t| t.job_control)
}

/// Detach `clients` and have each send its parent process SIGHUP; refused,
/// with none detached, if one cannot (a Windows client has no SIGHUP).
fn hang_up(srv: &mut Server, clients: &[ClientId]) -> Result<(), String> {
    if clients.iter().any(|&c| !job_control(srv, c)) {
        return Err("a Windows client has no parent to send SIGHUP: none detached".into());
    }
    for &c in clients {
        srv.detach_and_hang_up(c);
    }
    Ok(())
}

/// Give session `sid` the `update-environment` variables of a client's
/// environment `env`, marking one it lacks removed: tmux's `environ_update`.
fn update_env(srv: &mut Server, sid: SessionId, env: &[(String, String)]) {
    let updates = super::updated_env(&srv.session_options(sid).update_environment, env);
    if let Some(s) = srv.sessions.get_mut(&sid) {
        for (name, value) in updates {
            s.env.set(&name, value);
        }
    }
}

/// The client flags `-f` asks for that tmxr has.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ClientFlags {
    /// `read-only`: it may look but not type. As in tmux, `!read-only`
    /// leaves a read-only client read-only, so it is never turned off.
    read_only: bool,
    /// `ignore-size` on or (`!ignore-size`) off; `None` leaves it.
    ignore_size: Option<bool>,
}

/// `-f`'s client flags, comma separated, each turned off by a leading `!`,
/// as tmux's `server_client_set_flags`; tmux's others are refused by name.
pub(super) fn client_flags(list: Option<&str>) -> Result<ClientFlags, String> {
    let mut flags = ClientFlags::default();
    for flag in list.into_iter().flat_map(|f| f.split(',')) {
        let flag = flag.trim();
        let (name, on) = flag.strip_prefix('!').map_or((flag, true), |n| (n, false));
        match name {
            "" => {}
            "read-only" => flags.read_only |= on,
            "ignore-size" => flags.ignore_size = Some(on),
            f => return Err(format!("unsupported client flag: {f}")),
        }
    }
    Ok(flags)
}

/// Give a client the flags asked for, resizing the windows when its size
/// stops or starts counting.
pub(super) fn set_client_flags(srv: &mut Server, client: ClientId, flags: &ClientFlags) {
    let Some(c) = srv.clients.get_mut(&client) else {
        return;
    };
    c.attach_read_only |= flags.read_only;
    let Some(ignore) = flags.ignore_size.filter(|i| *i != c.ignore_size) else {
        return;
    };
    c.ignore_size = ignore;
    // Whether a client's size counts depends on every attached client's
    // flag, so any session may change size.
    let sessions: Vec<SessionId> = srv.sessions.keys().copied().collect();
    for s in sessions {
        srv.size_session(s);
    }
}
