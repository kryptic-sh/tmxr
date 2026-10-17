//! tmux's notification hooks that follow a change of state: a session or
//! window renamed, a session's current window or a window's active pane
//! changed, a window linked into or out of a session, a window's layout or
//! size changed, a pane entering or leaving a mode or getting a new title, a
//! client switching session or resizing.
//!
//! tmux fires these where each change is made. tmxr compares what it saw
//! after the last event ([`Watched`]) with the state now, once each event is
//! handled, so every path that makes a change fires its hook: a window
//! killed or a key binding moving the current window, an automatic rename.
//! What the hooks' own commands change is taken in without firing, as tmxr
//! fires no hook from a hook.
//!
//! Pane focus is brought up to date at the same moments
//! ([`Server::update_pane_focus`]).

use std::collections::{BTreeMap, BTreeSet};

use hjkl_layout::LayoutRect;

use crate::cmds::Ctx;
use crate::hooks::HookScope;
use crate::model::{ClientId, PaneId, SessionId, WindowId};
use crate::server::Server;

/// The state the notification hooks watch, as it was when last seen.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Watched {
    sessions: BTreeMap<SessionId, SessionSeen>,
    windows: BTreeMap<WindowId, WindowSeen>,
    panes: BTreeMap<PaneId, PaneSeen>,
    clients: BTreeMap<ClientId, ClientSeen>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionSeen {
    name: String,
    current: Option<WindowId>,
    windows: BTreeSet<WindowId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowSeen {
    name: String,
    active: PaneId,
    size: (u16, u16),
    layout: Vec<(PaneId, LayoutRect)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PaneSeen {
    /// In a mode (copy or clock).
    in_mode: bool,
    title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClientSeen {
    session: SessionId,
    size: (u16, u16),
}

impl Watched {
    /// What `srv` holds now.
    pub fn of(srv: &Server) -> Self {
        Self {
            sessions: srv
                .sessions
                .values()
                .map(|s| {
                    let seen = SessionSeen {
                        name: s.name.clone(),
                        current: s.current_window(),
                        windows: s.windows.values().copied().collect(),
                    };
                    (s.id, seen)
                })
                .collect(),
            windows: srv
                .windows
                .values()
                .map(|w| {
                    let seen = WindowSeen {
                        name: w.name.clone(),
                        active: w.active,
                        size: (w.cols, w.rows),
                        layout: crate::layout::pane_rects(&w.layout, w.cols, w.rows),
                    };
                    (w.id, seen)
                })
                .collect(),
            panes: srv
                .panes
                .values()
                .map(|p| {
                    let seen = PaneSeen {
                        in_mode: p.copy.is_some() || p.clock,
                        title: p.emu.title().map(str::to_owned),
                    };
                    (p.id, seen)
                })
                .collect(),
            clients: srv
                .clients
                .values()
                .filter_map(|c| {
                    let a = c.att.as_ref()?;
                    let seen = ClientSeen {
                        session: a.session,
                        size: (a.cols, a.rows),
                    };
                    Some((c.id, seen))
                })
                .collect(),
        }
    }
}

impl Server {
    /// Queue the hooks for what changed since [`Watched`] was last taken.
    /// Something new since then (a session just made, a client just
    /// attached) has hooks of its own and fires none of these.
    pub fn queue_changes(&mut self) {
        let now = Watched::of(self);
        let before = std::mem::replace(&mut self.watched, now.clone());
        let mut fired = Vec::new();
        for (id, s) in &now.sessions {
            let Some(old) = before.sessions.get(id) else {
                continue;
            };
            let scope = HookScope {
                session: Some(*id),
                ..HookScope::default()
            };
            if old.name != s.name {
                fired.push(("session-renamed", scope));
            }
            if old.current != s.current {
                fired.push(("session-window-changed", scope));
            }
            for w in s.windows.difference(&old.windows) {
                fired.push((
                    "window-linked",
                    HookScope {
                        window: Some(*w),
                        ..scope
                    },
                ));
            }
            for w in old.windows.difference(&s.windows) {
                fired.push((
                    "window-unlinked",
                    HookScope {
                        window: Some(*w),
                        ..scope
                    },
                ));
            }
        }
        for (id, w) in &now.windows {
            let Some(old) = before.windows.get(id) else {
                continue;
            };
            let scope = HookScope {
                window: Some(*id),
                ..HookScope::default()
            };
            if old.name != w.name {
                fired.push(("window-renamed", scope));
            }
            if old.active != w.active {
                fired.push(("window-pane-changed", scope));
            }
            if old.layout != w.layout {
                fired.push(("window-layout-changed", scope));
            }
            if old.size != w.size {
                fired.push(("window-resized", scope));
            }
        }
        for (id, p) in &now.panes {
            let Some(old) = before.panes.get(id) else {
                continue;
            };
            let scope = HookScope {
                pane: Some(*id),
                ..HookScope::default()
            };
            if old.in_mode != p.in_mode {
                fired.push(("pane-mode-changed", scope));
            }
            if old.title != p.title {
                fired.push(("pane-title-changed", scope));
            }
        }
        for (name, scope) in fired {
            let ctx = Ctx {
                pane: self.scope_pane(scope),
                ..Ctx::default()
            };
            self.queue_hook(name, ctx, scope);
        }
        for (id, c) in &now.clients {
            let Some(old) = before.clients.get(id) else {
                continue;
            };
            let ctx = Ctx {
                client: Some(*id),
                ..Ctx::default()
            };
            if old.session != c.session {
                self.queue_hook("client-session-changed", ctx.clone(), HookScope::default());
            }
            if old.size != c.size {
                self.queue_hook("client-resized", ctx, HookScope::default());
            }
        }
    }

    /// Bring each pane's focus up to date, as tmux's
    /// `window_pane_update_focus`: a pane is focused while it is the active
    /// pane of a window that a focused client shows with no popup or menu
    /// over it. A pane that gains or loses focus is told so (`CSI I` /
    /// `CSI O`) when its program asked for focus reports, and fires
    /// `pane-focus-in` / `pane-focus-out`.
    pub fn update_pane_focus(&mut self) {
        let shown: BTreeSet<WindowId> = self
            .clients
            .values()
            .filter_map(|c| c.att.as_ref())
            .filter(|a| {
                a.focused
                    && !matches!(
                        a.overlay,
                        Some(crate::overlay::Overlay::Popup(_) | crate::overlay::Overlay::Menu(_))
                    )
            })
            .filter_map(|a| self.sessions.get(&a.session)?.current_window())
            .collect();
        let mut changed = Vec::new();
        for p in self.panes.values_mut() {
            let focused = p.dead.is_none()
                && shown.contains(&p.window)
                && self
                    .windows
                    .get(&p.window)
                    .is_some_and(|w| w.active == p.id);
            if focused != p.focused {
                p.focused = focused;
                if p.emu.focus_reports() {
                    let report: &[u8] = if focused { b"\x1b[I" } else { b"\x1b[O" };
                    let _ = p.pty.write(report);
                }
                changed.push((p.id, focused));
            }
        }
        for (pane, focused) in changed {
            let ctx = Ctx {
                pane: Some(pane),
                ..Ctx::default()
            };
            let hook = if focused {
                "pane-focus-in"
            } else {
                "pane-focus-out"
            };
            self.queue_hook(hook, ctx, HookScope::default());
        }
    }

    /// The pane a hook's commands run for: the scope's pane, else its
    /// window's active pane, else its session's.
    fn scope_pane(&self, scope: HookScope) -> Option<PaneId> {
        scope
            .pane
            .or_else(|| {
                scope
                    .window
                    .and_then(|w| self.windows.get(&w))
                    .map(|w| w.active)
            })
            .or_else(|| scope.session.and_then(|s| self.active_pane_of_session(s)))
    }
}
