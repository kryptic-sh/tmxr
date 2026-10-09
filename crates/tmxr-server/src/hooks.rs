//! tmux's hooks: commands run when something happens, set with `set-hook`.
//!
//! Each hook is an array of commands (`set-hook -a` appends), global (`-g`)
//! or per session, a session's array taking the global one's place. Events
//! are queued where they happen and their hooks run once the server has
//! finished with the event that caused them, so a hook never runs inside a
//! half-done operation; a hook's own commands fire no further hooks.

use std::collections::BTreeMap;

use crate::cmds::Ctx;
use crate::model::SessionId;
use crate::server::Server;

/// Hook name → its commands.
pub type HookTable = BTreeMap<String, Vec<String>>;

/// The named events tmxr fires, besides `after-<command>` for every command.
pub const EVENTS: &[&str] = &[
    "client-attached",
    "client-detached",
    "pane-exited",
    "session-closed",
    "session-created",
];

/// Whether `name` is a hook tmxr fires.
pub fn known(name: &str) -> bool {
    EVENTS.contains(&name)
        || name
            .strip_prefix("after-")
            .is_some_and(|cmd| tmxr_command::lookup(cmd).is_ok_and(|c| c.name == cmd))
}

impl Server {
    /// Note that `name` happened, for its hooks to run after this event.
    /// `session` is whose hooks apply, when `ctx` does not say: a command's
    /// `-t` target, for its `after-` hook.
    pub fn queue_hook(&mut self, name: &str, ctx: Ctx, session: Option<SessionId>) {
        if !self.in_hook {
            self.pending_hooks.push((name.to_owned(), ctx, session));
        }
    }

    /// Run the hooks of everything queued, in order.
    pub fn run_pending_hooks(&mut self) {
        while !self.pending_hooks.is_empty() {
            for (name, ctx, session) in std::mem::take(&mut self.pending_hooks) {
                let session = session.or_else(|| crate::target::current_session(self, &ctx));
                let commands = session
                    .and_then(|s| self.sessions.get(&s))
                    .and_then(|s| s.hooks.get(&name))
                    .or_else(|| self.hooks.get(&name))
                    .cloned()
                    .unwrap_or_default();
                self.in_hook = true;
                for cmd in commands {
                    let out = crate::cmds::run_string(self, &ctx, &cmd);
                    if !out.stderr.is_empty() {
                        self.log_message(format!("hook {name}: {}", out.stderr.trim_end()));
                    }
                }
                self.in_hook = false;
            }
        }
    }
}
