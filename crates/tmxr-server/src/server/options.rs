//! Options as a window or pane sees them: its own over what it inherits, as
//! tmux's pane → window → global chain.

use std::borrow::Cow;

use tmxr_config::WindowOptions;

use super::Server;
use crate::model::{ClientId, PaneId, WindowId};

impl Server {
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

    /// User option `name` as a pane sees it: the pane's own, its window's,
    /// or the global one.
    pub fn user_option(
        &self,
        pane: Option<PaneId>,
        window: Option<WindowId>,
        name: &str,
    ) -> Option<String> {
        let pane = pane.and_then(|p| self.panes.get(&p));
        let window = window
            .or_else(|| pane.map(|p| p.window))
            .and_then(|w| self.windows.get(&w));
        pane.and_then(|p| p.options.show(name))
            .or_else(|| window.and_then(|w| w.options.show(name)))
            .or_else(|| self.cfg.options.get(name).cloned())
    }
}
