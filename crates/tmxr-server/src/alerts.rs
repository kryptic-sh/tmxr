//! tmux's alerts: a window's bell, activity or silence flags it and, as the
//! `*-action` and `visual-*` options say, rings the terminal bell of the
//! clients on its session or shows them a message (tmux's `alerts.c`).

use std::collections::BTreeSet;

use tmxr_config::{AlertAction, Visual};
use tmxr_proto::ServerMsg;

use crate::cmds::Ctx;
use crate::hooks::HookScope;
use crate::model::{ClientId, SessionId, WindowId};
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

    /// How it acts in `session`.
    fn options(self, srv: &Server, session: SessionId) -> (AlertAction, Visual) {
        let c = srv.session_options(session);
        match self {
            Self::Bell => (c.bell_action, c.visual_bell),
            Self::Activity => (c.activity_action, c.visual_activity),
            Self::Silence => (c.silence_action, c.visual_silence),
        }
    }
}

/// Raise `alert` in `window`, link by link as tmux's `alerts_check_*`: each
/// session's link is flagged unless it is the session's current window and
/// a client is attached; then, where the session's action applies, its hook
/// runs and the session's clients get the bell, the message ("Bell in
/// window 1", "Bell in current window") or both. A link already flagged
/// for activity or silence is left alone until it is seen; bells always
/// ring.
pub fn raise(srv: &mut Server, window: WindowId, alert: Alert) {
    let clients_of = |srv: &Server, session| -> Vec<ClientId> {
        srv.clients
            .values()
            .filter(|c| c.att.as_ref().is_some_and(|a| a.session == session))
            .map(|c| c.id)
            .collect()
    };
    let Some(w) = srv.windows.get(&window) else {
        return;
    };
    let already = match alert {
        Alert::Bell => BTreeSet::new(),
        Alert::Activity => w.activity.clone(),
        Alert::Silence => w.silence.clone(),
    };
    let mut flag = Vec::new();
    let mut acting = Vec::new();
    for s in srv.sessions.values() {
        let Some(index) = s
            .windows
            .iter()
            .find_map(|(i, w)| (*w == window).then_some(*i))
        else {
            continue;
        };
        if already.contains(&s.id) {
            continue;
        }
        let current = s.current == index;
        if !current || clients_of(srv, s.id).is_empty() {
            flag.push(s.id);
        }
        let (action, visual) = alert.options(srv, s.id);
        if action.applies(current) {
            acting.push((s.id, index, current, visual));
        }
    }
    if !flag.is_empty()
        && let Some(w) = srv.windows.get_mut(&window)
    {
        match alert {
            Alert::Bell => w.bell.extend(flag),
            Alert::Activity => w.activity.extend(flag),
            Alert::Silence => w.silence.extend(flag),
        }
        srv.mark_window_dirty(window);
    }
    let pane = srv.windows.get(&window).map(|w| w.active);
    for (session, index, current, visual) in acting {
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
