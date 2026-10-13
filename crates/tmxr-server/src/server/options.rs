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

    /// User option `name` as a pane in `session` sees it: the pane's own,
    /// its window's, the session's, or the global one.
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
            .or_else(|| session.and_then(|s| s.options.show(name)))
            .or_else(|| self.cfg.options.get(name).cloned())
    }
}
