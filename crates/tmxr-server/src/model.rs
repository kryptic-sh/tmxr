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
    /// Which start of this pane the PTY belongs to: `respawn-pane` starts a
    /// new program under the same id, and events from the old one are stale.
    pub spawn: u64,
    /// The command the pane was started with, for `respawn-pane`.
    pub argv: Vec<String>,
    /// Set when the program exited and `remain-on-exit` kept the pane: the
    /// exit code, if the program had one.
    pub dead: Option<Option<u32>>,
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

/// An environment as tmux keeps one: variables set, and names removed from
/// new processes (`set-environment -r`, shown as `-NAME`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment(std::collections::BTreeMap<String, Option<String>>);

impl Environment {
    /// Set `name`, or with `None` mark it removed.
    pub fn set(&mut self, name: &str, value: Option<String>) {
        self.0.insert(name.to_owned(), value);
    }

    /// Forget `name` here, so an outer environment's value shows through.
    pub fn unset(&mut self, name: &str) {
        self.0.remove(name);
    }

    /// Every entry, by name: `Some(value)` set, `None` removed.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Option<&str>)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_deref()))
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
        Self(iter.into_iter().map(|(k, v)| (k, Some(v))).collect())
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
