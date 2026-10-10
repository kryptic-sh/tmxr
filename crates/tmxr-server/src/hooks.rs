//! tmux's hooks: commands run when something happens, set with `set-hook`.
//!
//! Each hook is an array of commands (`set-hook -a` appends), global (`-g`),
//! per session, per window (`-w`) or per pane (`-p`); an event runs the most
//! specific array it has: its pane's, else its window's, its session's, the
//! global one. Events
//! are queued where they happen and their hooks run once the server has
//! finished with the event that caused them, so a hook never runs inside a
//! half-done operation; a hook's own commands fire no further hooks.

use std::collections::BTreeMap;

use crate::cmds::Ctx;
use crate::model::{PaneId, SessionId, WindowId};
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

/// Where an event happened, as far as it is known: whose hooks apply.
#[derive(Debug, Clone, Copy, Default)]
pub struct HookScope {
    pub session: Option<SessionId>,
    pub window: Option<WindowId>,
    pub pane: Option<PaneId>,
}

impl Server {
    /// Note that `name` happened, for its hooks to run after this event.
    /// `scope` is where, beyond what `ctx` says: a command's `-t` target,
    /// for its `after-` hook.
    pub fn queue_hook(&mut self, name: &str, ctx: Ctx, scope: HookScope) {
        if !self.in_hook {
            self.pending_hooks.push((name.to_owned(), ctx, scope));
        }
    }

    /// The commands `name` runs for an event at `scope`: the most specific
    /// table that has it.
    fn hook_commands(&self, name: &str, ctx: &Ctx, scope: HookScope) -> Vec<String> {
        let pane = scope.pane.or(ctx.pane);
        let window = scope
            .window
            .or_else(|| pane.and_then(|p| self.panes.get(&p)).map(|p| p.window));
        let session = scope
            .session
            .or_else(|| window.and_then(|w| self.session_of_window(w)))
            .or_else(|| crate::target::current_session(self, ctx));
        pane.and_then(|p| self.panes.get(&p))
            .and_then(|p| p.hooks.get(name))
            .or_else(|| {
                window
                    .and_then(|w| self.windows.get(&w))
                    .and_then(|w| w.hooks.get(name))
            })
            .or_else(|| {
                session
                    .and_then(|s| self.sessions.get(&s))
                    .and_then(|s| s.hooks.get(name))
            })
            .or_else(|| self.hooks.get(name))
            .cloned()
            .unwrap_or_default()
    }

    /// Run the hooks of everything queued, in order.
    pub fn run_pending_hooks(&mut self) {
        while !self.pending_hooks.is_empty() {
            for (name, ctx, scope) in std::mem::take(&mut self.pending_hooks) {
                let commands = self.hook_commands(&name, &ctx, scope);
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
