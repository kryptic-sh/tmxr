//! Options as a session, window or pane sees them: its own over what it
//! inherits, as tmux's session → global and pane → window → global chains.

use std::borrow::Cow;

use tmxr_command::keys::Key;
use tmxr_config::{SessionOptions, WindowOptions};

use super::Server;
use crate::model::{ClientId, PaneId, SessionId, WindowId};

impl Server {
    /// A session's options: its own over the global ones.
    pub fn session_options(&self, session: SessionId) -> Cow<'_, SessionOptions> {
        let global = Cow::Borrowed(&self.cfg.session);
        match self.sessions.get(&session) {
            Some(s) => s.options.over(global),
            None => global,
        }
    }

    /// The options of the session client `client` is attached to; the
    /// global ones for a client that is not.
    /// The global `default-size`, `WxH`, a detached session's windows get;
    /// `None` when it does not parse.
    pub fn default_size(&self) -> Option<(u16, u16)> {
        let (w, h) = self.cfg.session.default_size.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    }

    /// The rows `session`'s status line takes: one, or none with `status`
    /// off.
    pub fn status_rows(&self, session: SessionId) -> u16 {
        u16::from(self.session_options(session).status)
    }

    /// Each visible pane's area in window `w`: its layout cell less a row
    /// for its status line when it is on the window's top (bottom) edge
    /// with `pane-border-status top` (`bottom`), as tmux's
    /// layout_fix_panes; other panes' lines go on the borders between.
    pub fn pane_cells(
        &self,
        w: crate::model::WindowId,
    ) -> Vec<(crate::model::PaneId, hjkl_layout::LayoutRect)> {
        let Some(win) = self.windows.get(&w) else {
            return Vec::new();
        };
        let mut cells = win.visible_rects();
        match self.window_options(w).pane_border_status {
            tmxr_config::PaneStatus::Off => {}
            tmxr_config::PaneStatus::Top => {
                for (_, r) in cells.iter_mut().filter(|(_, r)| r.y == 0 && r.h > 1) {
                    r.y += 1;
                    r.h -= 1;
                }
            }
            tmxr_config::PaneStatus::Bottom => {
                for (_, r) in cells
                    .iter_mut()
                    .filter(|(_, r)| r.y + r.h == win.rows && r.h > 1)
                {
                    r.h -= 1;
                }
            }
        }
        cells
    }

    /// Whether `session`'s status line is its clients' top row
    /// (`status-position top`).
    pub fn status_at_top(&self, session: SessionId) -> bool {
        self.session_options(session).status_position == tmxr_config::StatusPosition::Top
    }

    /// The client row `session`'s windows start on: below a status line at
    /// the top, else the first.
    pub fn window_top(&self, session: SessionId) -> u16 {
        if self.status_at_top(session) {
            self.status_rows(session)
        } else {
            0
        }
    }

    /// The rows client `client`'s status line takes, by its session's (the
    /// global) `status`.
    pub fn client_status_rows(&self, client: ClientId) -> u16 {
        u16::from(self.client_session_options(client).status)
    }

    pub fn client_session_options(&self, client: ClientId) -> Cow<'_, SessionOptions> {
        match self.client_session(client) {
            Some(s) => self.session_options(s),
            None => Cow::Borrowed(&self.cfg.session),
        }
    }

    /// The options of a session holding `window` (the first, for a linked
    /// window), which tmux reads a window's session options from.
    pub fn window_session_options(&self, window: WindowId) -> Cow<'_, SessionOptions> {
        match self.session_of_window(window) {
            Some(s) => self.session_options(s),
            None => Cow::Borrowed(&self.cfg.session),
        }
    }

    /// The options of the session `pane` is in, which a shell command run
    /// for it takes `default-shell` from; the global ones without a pane.
    pub fn pane_session_options(&self, pane: Option<PaneId>) -> Cow<'_, SessionOptions> {
        match pane.and_then(|p| self.panes.get(&p)) {
            Some(p) => self.window_session_options(p.window),
            None => Cow::Borrowed(&self.cfg.session),
        }
    }

    /// The session client `client` is attached to.
    pub fn client_session(&self, client: ClientId) -> Option<SessionId> {
        Some(self.clients.get(&client)?.att.as_ref()?.session)
    }

    /// `session`'s prefix key. A `prefix` that names no key (`set-option`
    /// refuses one; a config file can still hold one) is C-b.
    /// Whether `key` is one of `session`'s prefix keys (`prefix`, or
    /// `prefix2` unless `None`).
    pub fn is_prefix(&self, session: SessionId, key: &Key) -> bool {
        *key == self.prefix_key(session)
            || self
                .session_options(session)
                .prefix2
                .parse::<Key>()
                .is_ok_and(|p| p == *key)
    }

    /// The key table `session`'s clients start in (`key-table`, `root`
    /// when empty), as tmux's `server_client_get_key_table`.
    pub fn default_key_table(&self, session: SessionId) -> String {
        let table = &self.session_options(session).key_table;
        if table.is_empty() {
            "root".to_owned()
        } else {
            table.clone()
        }
    }

    pub fn prefix_key(&self, session: SessionId) -> Key {
        self.session_options(session)
            .prefix
            .parse()
            .unwrap_or_else(|_| {
                Key::new(
                    crossterm::event::KeyCode::Char('b'),
                    crossterm::event::KeyModifiers::CONTROL,
                )
            })
    }

    /// A window's options: its own over the global ones.
    pub fn window_options(&self, window: WindowId) -> Cow<'_, WindowOptions> {
        let global = Cow::Borrowed(&self.cfg.window);
        match self.windows.get(&window) {
            Some(w) => w.options.over(global),
            None => global,
        }
    }

    /// A pane's options: its own over its window's.
    pub fn pane_options(&self, pane: PaneId) -> Cow<'_, WindowOptions> {
        match self.panes.get(&pane) {
            Some(p) => p.options.over(self.window_options(p.window)),
            None => Cow::Borrowed(&self.cfg.window),
        }
    }

    /// The options of the window client `client` shows, which menus, popups
    /// and the other overlays take theirs from, as tmux's.
    pub fn client_window_options(&self, client: ClientId) -> Cow<'_, WindowOptions> {
        let window = self
            .clients
            .get(&client)
            .and_then(|c| c.att.as_ref())
            .and_then(|a| self.sessions.get(&a.session))
            .and_then(|s| s.current_window());
        match window {
            Some(w) => self.window_options(w),
            None => Cow::Borrowed(&self.cfg.window),
        }
    }

    /// User option `name` as a pane in `session` sees it, found as tmux's
    /// `format_find` looks: the pane's own, its window's, the global window
    /// ones (`set -wg`), the session's, then the global session ones.
    pub fn user_option(
        &self,
        session: Option<SessionId>,
        pane: Option<PaneId>,
        window: Option<WindowId>,
        name: &str,
    ) -> Option<String> {
        let pane = pane.and_then(|p| self.panes.get(&p));
        let window = window
            .or_else(|| pane.map(|p| p.window))
            .and_then(|w| self.windows.get(&w));
        let session = session
            .or_else(|| self.session_of_window(window?.id))
            .and_then(|s| self.sessions.get(&s));
        pane.and_then(|p| p.options.show(name))
            .or_else(|| window.and_then(|w| w.options.show(name)))
            .or_else(|| self.cfg.window_user.get(name).cloned())
            .or_else(|| session.and_then(|s| s.options.show(name)))
            .or_else(|| self.cfg.options.get(name).cloned())
    }
}
