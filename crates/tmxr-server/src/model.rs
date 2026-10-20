//! Sessions, windows and panes.
//!
//! Ids are monotonic and never reused (tmux's `$N`, `@N`, `%N`), so targets in
//! scripts stay valid. Window *indices* are per session and are what the user
//! sees in the status line.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

use hjkl_layout::{LayoutRect, LayoutTree};
use tmxr_term::{Emulator, Pty};

pub type SessionId = u32;
pub type WindowId = u32;
pub type PaneId = u32;
pub type ClientId = u32;

pub struct Pane {
    pub id: PaneId,
    pub window: WindowId,
    pub pty: Pty,
    pub emu: Emulator,
    /// Where the pane sits inside its window's area (excluding borders).
    pub rect: LayoutRect,
    /// The directory the pane was started in; the fallback for
    /// `pane_current_path` when the process cannot be inspected.
    pub start_cwd: PathBuf,
    pub copy: Option<crate::copy::CopyMode>,
    /// The last search copy mode made here, kept after it ends, as tmux's
    /// `wp->searchstr` (`#{pane_search_string}`).
    pub last_search: Option<crate::copy::Search>,
    /// Which start of this pane the PTY belongs to: `respawn-pane` starts a
    /// new program under the same id, and events from the old one are stale.
    pub spawn: u64,
    /// The command the pane was started with, as given (`default-command`
    /// filled in, not the shell that runs it), as tmux's `wp->argv`: what
    /// `respawn-pane` starts again and `#{pane_start_command}` shows.
    pub argv: Vec<String>,
    /// Set when the program exited and `remain-on-exit` kept the pane: the
    /// exit code, if the program had one.
    pub dead: Option<Option<u32>>,
    /// When `dead` was set (`#{pane_dead_time}`).
    pub dead_time: Option<std::time::SystemTime>,
    /// The signal that ended a pane kept dead (`#{pane_dead_signal}`).
    pub dead_signal: Option<i32>,
    /// When the dead pane's `remain-on-exit-format` line is written: once
    /// its program's output has gone quiet, or on its end of output.
    pub dead_line_at: Option<std::time::Instant>,
    /// `clock-mode`: the pane shows a big clock until a key is pressed.
    pub clock: bool,
    /// `select-pane -d`: no keys, pastes or mouse reach the program, as
    /// tmux's `PANE_INPUTOFF`, until `select-pane -e`.
    pub input_off: bool,
    /// Focused: the active pane of a window a focused client shows, as
    /// tmux's `PANE_FOCUSED` (`notify`).
    pub focused: bool,
    /// Output the reader thread has queued for this spawn.
    pub output: crate::output::OutputHandle,
    /// `pipe-pane`: the command this pane's output is copied to.
    pub pipe: Option<crate::pipe::PanePipe>,
    /// `set-hook -p`: hooks for events in this pane.
    pub hooks: crate::hooks::HookTable,
    /// `set-option -p`: options set on this pane, over its window's.
    pub options: tmxr_config::OwnOptions,
}

/// An environment as tmux keeps one: variables set, names removed from new
/// processes (`set-environment -r`, shown as `-NAME`), and hidden variables
/// (`set-environment -h`) that new processes do not get.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment(std::collections::BTreeMap<String, EnvEntry>);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvEntry {
    value: Option<String>,
    hidden: bool,
}

impl Environment {
    /// Set `name`, or with `None` mark it removed.
    pub fn set(&mut self, name: &str, value: Option<String>) {
        let entry = EnvEntry {
            value,
            hidden: false,
        };
        self.0.insert(name.to_owned(), entry);
    }

    /// Set `name` hidden, as tmux's `ENVIRON_HIDDEN`: kept, and shown by
    /// `show-environment -h`, but left out of new processes.
    pub fn set_hidden(&mut self, name: &str, value: String) {
        let entry = EnvEntry {
            value: Some(value),
            hidden: true,
        };
        self.0.insert(name.to_owned(), entry);
    }

    /// Forget `name` here, so an outer environment's value shows through.
    pub fn unset(&mut self, name: &str) {
        self.0.remove(name);
    }

    /// Every entry but the hidden ones, by name: `Some(value)` set, `None`
    /// removed.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Option<&str>)> {
        self.0
            .iter()
            .filter(|(_, e)| !e.hidden)
            .map(|(k, e)| (k.as_str(), e.value.as_deref()))
    }

    /// The hidden variables, by name.
    pub fn hidden(&self) -> impl Iterator<Item = (&str, Option<&str>)> {
        self.0
            .iter()
            .filter(|(_, e)| e.hidden)
            .map(|(k, e)| (k.as_str(), e.value.as_deref()))
    }

    /// `outer` with this environment's entries over it.
    pub fn over(&self, outer: &Self) -> Self {
        let mut all = outer.0.clone();
        all.extend(self.0.clone());
        Self(all)
    }
}

impl FromIterator<(String, String)> for Environment {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(iter: I) -> Self {
        Self(
            iter.into_iter()
                .map(|(k, v)| {
                    let entry = EnvEntry {
                        value: Some(v),
                        hidden: false,
                    };
                    (k, entry)
                })
                .collect(),
        )
    }
}

pub struct Window {
    pub id: WindowId,
    pub name: String,
    pub layout: LayoutTree,
    /// tmux's pane list, which numbers the panes, where it differs from the
    /// layout's order (a mirrored layout puts the first pane last); empty
    /// while they agree.
    pub order: Vec<PaneId>,
    pub active: PaneId,
    pub last_pane: Option<PaneId>,
    /// The client that last typed into or resized while showing it, as
    /// tmux's `w->latest`; `client-active` runs when it changes.
    pub latest: Option<crate::model::ClientId>,
    pub zoomed: bool,
    /// The sessions whose link to it a bell flags: tmux keeps the alert
    /// flags per winlink, so each session holding a window has its own.
    pub bell: BTreeSet<SessionId>,
    /// Size of the area the panes share (the client minus the status line).
    pub cols: u16,
    pub rows: u16,
    /// Position in `layout::PRESETS` for `next-layout`.
    pub preset: usize,
    /// The layout before the last `select-layout`, for `select-layout -o`.
    pub old_layout: Option<LayoutTree>,
    /// `set-option -w`: options set on this window, over the global ones
    /// (`resize-window` sets `window-size manual` here, and `rename-window`
    /// `automatic-rename off`, as tmux's).
    pub options: tmxr_config::OwnOptions,
    /// The sessions whose link to it printed while out of sight
    /// (`monitor-activity`).
    pub activity: BTreeSet<SessionId>,
    /// The sessions whose link to it was quiet for `monitor-silence`
    /// seconds.
    pub silence: BTreeSet<SessionId>,
    /// `monitor-silence` has fired since the window last printed.
    pub silence_raised: bool,
    /// When one of its panes last printed.
    pub last_output: Instant,
    /// `set-hook -w`: hooks for events in this window.
    pub hooks: crate::hooks::HookTable,
}

impl Window {
    pub fn panes(&self) -> Vec<PaneId> {
        let leaves: Vec<PaneId> = self
            .layout
            .leaves()
            .into_iter()
            .map(|p| p as PaneId)
            .collect();
        if self.order.is_empty() {
            return leaves;
        }
        // Killed panes leave the list here; every way a pane is added
        // inserts it (`Server::insert_leaf`), but one that was not still
        // gets a number, last.
        let mut panes: Vec<PaneId> = self
            .order
            .iter()
            .copied()
            .filter(|p| leaves.contains(p))
            .collect();
        for p in leaves {
            if !panes.contains(&p) {
                panes.push(p);
            }
        }
        panes
    }

    /// Pane rects as displayed: only the active pane, full size, when zoomed.
    pub fn visible_rects(&self) -> Vec<(PaneId, LayoutRect)> {
        if self.zoomed {
            vec![(self.active, LayoutRect::new(0, 0, self.cols, self.rows))]
        } else {
            crate::layout::pane_rects(&self.layout, self.cols, self.rows)
        }
    }

    /// A bell, activity or silence on `session`'s link: what `next-window
    /// -a` looks for.
    pub fn alerted(&self, session: SessionId) -> bool {
        [&self.bell, &self.activity, &self.silence]
            .iter()
            .any(|flagged| flagged.contains(&session))
    }

    /// Clear the alerts on every session's link, as tmux's
    /// `winlink_clear_flags` does once the window is in sight.
    pub fn clear_alerts(&mut self) {
        self.bell.clear();
        self.activity.clear();
        self.silence.clear();
    }

    /// Forget `session`'s link, which is gone.
    pub fn forget_session(&mut self, session: SessionId) {
        self.bell.remove(&session);
        self.activity.remove(&session);
        self.silence.remove(&session);
    }

    /// The flags `window_flags` shows for `session`'s link.
    pub fn flags(&self, session: SessionId, current: bool, last: bool, marked: bool) -> String {
        let mut f = String::new();
        if current {
            f.push('*');
        } else if last {
            f.push('-');
        }
        // tmux's order: activity, bell, silence.
        if self.activity.contains(&session) {
            f.push('#');
        }
        if self.bell.contains(&session) {
            f.push('!');
        }
        if self.silence.contains(&session) {
            f.push('~');
        }
        if marked {
            f.push('M');
        }
        if self.zoomed {
            f.push('Z');
        }
        f
    }
}

pub struct Session {
    pub id: SessionId,
    pub name: String,
    /// index → window.
    pub windows: BTreeMap<u32, WindowId>,
    pub current: u32,
    pub last: Option<u32>,
    pub cwd: PathBuf,
    /// The session environment: `update-environment` variables captured from
    /// the creating client, and `set-environment` changes.
    pub env: Environment,
    /// This session's hooks, in place of the global ones of the same name.
    pub hooks: crate::hooks::HookTable,
    /// `set-option` without `-g`: options set on this session, over the
    /// global ones.
    pub options: tmxr_config::OwnSessionOptions,
    pub created: std::time::SystemTime,
    pub last_used: Instant,
    /// When it was made, last attached to, or last typed into by a client
    /// (`#{session_activity}`), as tmux's activity_time.
    pub activity: std::time::SystemTime,
    /// When a client last attached or switched to it, none before one has
    /// (`#{session_last_attached}`).
    pub last_attached: Option<std::time::SystemTime>,
}

impl Session {
    pub fn current_window(&self) -> Option<WindowId> {
        self.windows.get(&self.current).copied()
    }

    pub fn index_of(&self, window: WindowId) -> Option<u32> {
        self.windows
            .iter()
            .find(|(_, w)| **w == window)
            .map(|(i, _)| *i)
    }
}
