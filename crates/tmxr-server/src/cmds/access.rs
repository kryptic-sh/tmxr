//! `server-access`, and the commands a read-only client may run.

use std::fmt::Write as _;

use tmxr_command::Parsed;
use tmxr_proto::socket::Access;

use super::{Ctx, Outcome};
use crate::access::lock;
use crate::model::ClientId;
use crate::server::Server;

/// What a read-only client may run, as tmux's read-only commands: attach,
/// detach and switch, and the commands that only look.
pub const READ_ONLY: &[&str] = &[
    "attach-session",
    "capture-pane",
    "choose-tree",
    "detach-client",
    "display-message",
    "has-session",
    "list-buffers",
    "list-clients",
    "list-commands",
    "list-keys",
    "list-panes",
    "list-sessions",
    "list-windows",
    "show-buffer",
    "show-environment",
    "show-hooks",
    "show-messages",
    "show-options",
    "show-prompt-history",
    "show-window-options",
    "switch-client",
];

pub(super) fn run(
    srv: &mut Server,
    _ctx: &Ctx,
    p: &Parsed,
    out: &mut Outcome,
) -> Result<bool, String> {
    if p.name() != "server-access" {
        return Ok(false);
    }
    let a = &p.args;
    if a.has('l') {
        for e in lock(&srv.acl).entries() {
            let mode = if e.write { "W" } else { "R" };
            let _ = writeln!(out.stdout, "{} ({mode})", e.name);
        }
        return Ok(true);
    }
    let name = p
        .args
        .positional()
        .first()
        .ok_or("server-access: no user given")?;
    let id = crate::access::lookup_user(name).map_err(|e| format!("server-access: {name}: {e}"))?;
    if id == srv.owner {
        return Err(format!("server-access: {name} owns the server"));
    }
    if a.has('d') {
        if !lock(&srv.acl).deny(&id) {
            return Err(format!("server-access: {name} has no access"));
        }
        // Their clients go too, as in tmux.
        for c in clients_of(srv, &id) {
            if srv.clients.get(&c).is_some_and(|c| c.att.is_some()) {
                srv.detach(c, "server access removed");
            } else {
                srv.clients.remove(&c);
            }
        }
        return Ok(true);
    }
    let write = if a.has('a') {
        if srv.socket_access == Access::Owner {
            return Err(
                "server-access: other users cannot open this server: set socket-access = \"users\" \
                 in the config and restart it"
                    .into(),
            );
        }
        if srv.endpoint.in_private_dir() {
            return Err(
                "server-access: other users cannot reach this server's socket directory: \
                 start it with -S at a path they can reach"
                    .into(),
            );
        }
        let write = !a.has('r');
        lock(&srv.acl).allow(id.clone(), name.clone(), write);
        write
    } else if a.has('r') || a.has('w') {
        let write = a.has('w');
        if !lock(&srv.acl).set_write(&id, write) {
            return Err(format!("server-access: {name} has no access"));
        }
        write
    } else {
        return Err("server-access: give -a, -d, -l, -r or -w".into());
    };
    for c in clients_of(srv, &id) {
        if let Some(c) = srv.clients.get_mut(&c) {
            c.user_read_only = !write;
        }
    }
    Ok(true)
}

fn clients_of(srv: &Server, user: &str) -> Vec<ClientId> {
    srv.clients
        .values()
        .filter(|c| c.user == user)
        .map(|c| c.id)
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn read_only_names_are_commands() {
        for name in super::READ_ONLY {
            assert!(
                tmxr_command::lookup(name).is_ok_and(|c| c.name == *name),
                "{name} is not a command name"
            );
        }
    }
}
