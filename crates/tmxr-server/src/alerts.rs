//! tmux's alerts: a window's bell, activity or silence flags it and, as the
//! `*-action` and `visual-*` options say, rings the terminal bell of the
//! clients on its session or shows them a message (tmux's `alerts.c`).

use tmxr_config::{AlertAction, Visual};
use tmxr_proto::ServerMsg;

use crate::cmds::Ctx;
use crate::hooks::HookScope;
use crate::model::{ClientId, WindowId};
use crate::server::Server;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alert {
    Bell,
    Activity,
    Silence,
}

impl Alert {
    const fn label(self) -> &'static str {
        match self {
            Self::Bell => "Bell",
            Self::Activity => "Activity",
            Self::Silence => "Silence",
        }
    }

    const fn hook(self) -> &'static str {
        match self {
            Self::Bell => "alert-bell",
            Self::Activity => "alert-activity",
            Self::Silence => "alert-silence",
        }
    }

    fn options(self, srv: &Server) -> (AlertAction, Visual) {
        let c = &srv.cfg;
        match self {
            Self::Bell => (c.bell_action, c.visual_bell),
            Self::Activity => (c.activity_action, c.visual_activity),
            Self::Silence => (c.silence_action, c.visual_silence),
        }
    }
}

/// Raise `alert` in `window`. The window is flagged unless a client is
/// looking at it; then, in each session holding it whose action applies,
/// its hook runs and the session's clients get the bell, the message
/// ("Bell in window 1", "Bell in current window") or both.
pub fn raise(srv: &mut Server, window: WindowId, alert: Alert) {
    let (action, visual) = alert.options(srv);
    let clients_of = |srv: &Server, session| -> Vec<ClientId> {
        srv.clients
            .values()
            .filter(|c| c.att.as_ref().is_some_and(|a| a.session == session))
            .map(|c| c.id)
            .collect()
    };
    let mut seen = false;
    let mut acting = Vec::new();
    for s in srv.sessions.values() {
        let Some(index) = s
            .windows
            .iter()
            .find_map(|(i, w)| (*w == window).then_some(*i))
        else {
            continue;
        };
        let current = s.current == index;
        if current && !clients_of(srv, s.id).is_empty() {
            seen = true;
        }
        if action.applies(current) {
            acting.push((s.id, index, current));
        }
    }
    if !seen && let Some(w) = srv.windows.get_mut(&window) {
        match alert {
            Alert::Bell => w.bell = true,
            Alert::Activity => w.activity = true,
            Alert::Silence => w.silence = true,
        }
        srv.mark_window_dirty(window);
    }
    let pane = srv.windows.get(&window).map(|w| w.active);
    for (session, index, current) in acting {
        let scope = HookScope {
            session: Some(session),
            window: Some(window),
            pane,
        };
        let ctx = Ctx {
            pane,
            ..Ctx::default()
        };
        srv.queue_hook(alert.hook(), ctx, scope);
        let text = if current {
            format!("{} in current window", alert.label())
        } else {
            format!("{} in window {index}", alert.label())
        };
        for id in clients_of(srv, session) {
            if visual.bell() {
                srv.send(id, ServerMsg::Output(b"\x07".to_vec()));
            }
            if visual.message() {
                srv.show_message(id, text.clone());
            }
        }
    }
}
