//! Sessions, windows and panes.
//!
//! Ids are monotonic and never reused (tmux's `$N`, `@N`, `%N`), so targets in
//! scripts stay valid. Window *indices* are per session and are what the user
//! sees in the status line.

use std::collections::BTreeMap;
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
    /// Output the reader thread has queued for this spawn.
    pub output: crate::output::OutputHandle,
    /// `pipe-pane`: the command this pane's output is copied to.
    pub pipe: Option<crate::pipe::PanePipe>,
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
    /// Follow the active pane's command until the user renames the window.
    pub auto_name: bool,
    pub layout: LayoutTree,
    pub active: PaneId,
    pub last_pane: Option<PaneId>,
    pub zoomed: bool,
    pub synchronize: bool,
    pub bell: bool,
    /// Size of the area the panes share (the client minus the status line).
    pub cols: u16,
    pub rows: u16,
    /// Position in `layout::PRESETS` for `next-layout`.
    pub preset: usize,
}

impl Window {
    pub fn panes(&self) -> Vec<PaneId> {
        self.layout
            .leaves()
            .into_iter()
            .map(|p| p as PaneId)
            .collect()
    }

    /// Pane rects as displayed: only the active pane, full size, when zoomed.
    pub fn visible_rects(&self) -> Vec<(PaneId, LayoutRect)> {
        if self.zoomed {
            vec![(self.active, LayoutRect::new(0, 0, self.cols, self.rows))]
        } else {
            crate::layout::pane_rects(&self.layout, self.cols, self.rows)
        }
    }

    pub fn flags(&self, current: bool, last: bool, marked: bool) -> String {
        let mut f = String::new();
        if current {
            f.push('*');
        } else if last {
            f.push('-');
        }
        if self.bell {
            f.push('!');
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
