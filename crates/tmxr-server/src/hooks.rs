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
    "alert-activity",
    "alert-bell",
    "alert-silence",
    "client-attached",
    "client-detached",
    "pane-exited",
    "session-closed",
    "session-created",
];

/// The events above that tmux keeps with the window and pane options
/// (`OPTIONS_TABLE_PANE_HOOK`); the rest, and every `after-` hook, it keeps
/// with the session options.
pub const PANE_HOOKS: &[&str] = &["pane-exited"];

/// Whether `name` is a hook tmxr fires.
pub fn known(name: &str) -> bool {
    EVENTS.contains(&name)
        || name
            .strip_prefix("after-")
            .is_some_and(|cmd| tmxr_command::lookup(cmd).is_ok_and(|c| c.name == cmd))
}

/// The global hooks the config's `[hooks]` sets, and an error for each name
/// tmxr does not fire (that one is left out).
pub fn from_config(cfg: &tmxr_config::Config) -> (HookTable, Vec<String>) {
    let mut errors = Vec::new();
    let mut table = HookTable::new();
    for (name, commands) in &cfg.hooks {
        if known(name) {
            table.insert(name.clone(), commands.clone());
        } else {
            errors.push(format!("hooks.{name}: unknown hook"));
        }
    }
    (table, errors)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_hooks_are_global_and_unknown_names_are_errors() {
        let mut cfg = tmxr_config::defaults();
        cfg.hooks
            .insert("session-created".into(), vec!["new-window -d".into()]);
        cfg.hooks.insert(
            "after-split-window".into(),
            vec!["select-layout tiled".into()],
        );
        cfg.hooks
            .insert("no-such-hook".into(), vec!["kill-server".into()]);
        let (table, errors) = from_config(&cfg);
        assert_eq!(
            table.keys().map(String::as_str).collect::<Vec<_>>(),
            ["after-split-window", "session-created"]
        );
        assert_eq!(errors, ["hooks.no-such-hook: unknown hook"]);
    }
}
