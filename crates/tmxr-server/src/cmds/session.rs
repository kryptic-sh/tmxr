//! Session commands, and saving and restoring them.

use std::fmt::Write as _;

use tmxr_command::Parsed;
use tmxr_proto::ServerMsg;

use super::{Ctx, Outcome, attached_client, client_size, cwd_arg, session_env};
use crate::model::{ClientId, SessionId};
use crate::server::{STATUS_ROWS, Server};
use crate::target;

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
            let size = match (a.value('x'), a.value('y')) {
                (Some(x), Some(y)) => (
                    x.parse().map_err(|_| "bad -x")?,
                    y.parse::<u16>()
                        .map_err(|_| "bad -y")?
                        .saturating_sub(STATUS_ROWS)
                        .max(1),
                ),
                _ => client_size(srv, ctx),
            };
            let cwd = cwd_arg(srv, ctx, None, a)
                .or_else(|| ctx.cwd.clone())
                .unwrap_or_else(crate::util::home_dir);
            // -e sets the new session's environment, after the client's.
            let mut env = session_env(srv, ctx);
            env.extend(super::env_flags(a)?);
            let sid = srv.new_session(
                a.value('s').map(str::to_owned),
                cwd,
                env,
                a.value('n').map(str::to_owned),
                pos.to_vec(),
                size,
            )?;
            if !a.has('d') {
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
            if a.has('d') {
                let others: Vec<ClientId> = srv
                    .clients
                    .values()
                    .filter(|c| c.att.as_ref().is_some_and(|x| x.session == sid))
                    .filter(|c| Some(c.id) != ctx.client)
                    .map(|c| c.id)
                    .collect();
                for c in others {
                    srv.detach(c, "detached");
                }
            }
            // -r: this client may look but not type.
            if a.has('r')
                && let Some(c) = ctx.client.and_then(|c| srv.clients.get_mut(&c))
            {
                c.attach_read_only = true;
            }
            match attached_client(srv, ctx) {
                Some(c) => srv.switch_client(c, sid),
                None => out.attach = Some(sid),
            }
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
            for c in targets {
                srv.detach(c, "detached");
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
            let job_control = srv.clients[&c]
                .hello
                .as_ref()
                .and_then(|h| h.terminal.as_ref())
                .is_some_and(|t| t.job_control);
            if !job_control {
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
                let session = srv
                    .sessions
                    .get(&att.session)
                    .map_or("", |s| s.name.as_str());
                let term = c
                    .hello
                    .as_ref()
                    .and_then(|h| h.terminal.as_ref())
                    .map_or("", |t| t.term.as_str());
                let _ = writeln!(
                    out.stdout,
                    "{}: {session} [{}x{} {term}]",
                    c.id, att.cols, att.rows
                );
            }
        }
        "list-sessions" => {
            for s in srv.sessions.values() {
                let attached = srv
                    .clients
                    .values()
                    .any(|c| c.att.as_ref().is_some_and(|a| a.session == s.id));
                let _ = writeln!(
                    out.stdout,
                    "{}: {} windows{}",
                    s.name,
                    s.windows.len(),
                    if attached { " (attached)" } else { "" }
                );
            }
        }
        "rename-session" => {
            let sid = target::session(srv, ctx, a.value('t'))?;
            let name = &pos[0];
            if name.is_empty() || name.contains([':', '.']) {
                return Err(format!("bad session name: {name}"));
            }
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
                let ids: Vec<SessionId> = srv.sessions.keys().copied().collect();
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
