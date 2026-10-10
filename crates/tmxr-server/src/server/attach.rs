//! Attaching clients, and sizing sessions to them.

use crate::cmds::Ctx;
use std::time::Instant;

use ratatui::{Terminal, TerminalOptions, Viewport, layout::Rect};
use tmxr_proto::ServerMsg;
use tracing::debug;

use super::{Attached, STATUS_ROWS, Server};
use crate::backend::AnsiBackend;
use crate::model::{ClientId, SessionId, WindowId};

impl Server {
    pub fn attach(&mut self, id: ClientId, session: SessionId) {
        let Some(term) = self.client_terminal(id).cloned() else {
            return;
        };
        let (cols, rows) = (term.cols.max(1), term.rows.max(1));
        let area = Rect::new(0, 0, cols, rows);
        let mut backend = AnsiBackend::new(cols, rows);
        backend.set_rgb(self.client_rgb(id));
        let terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )
        .ok();
        let Some(c) = self.clients.get_mut(&id) else {
            return;
        };
        c.att = Some(Attached {
            session,
            cols,
            rows,
            term: terminal,
            table: "root".into(),
            repeat_until: None,
            overlay: None,
            message: None,
            last_session: None,
            dirty: true,
            full_redraw: true,
            locked: false,
            last_input: Instant::now(),
            drag: None,
            press: None,
            click: None,
            status_ranges: crate::render::StatusRanges::default(),
            mouse: false,
        });
        self.restored_pending = false;
        self.send(id, ServerMsg::Attached);
        self.touch_session(session);
        self.size_session(session);
        let ctx = Ctx {
            client: Some(id),
            pane: self.active_pane_of_session(session),
            ..Ctx::default()
        };
        self.queue_hook("client-attached", ctx, crate::hooks::HookScope::default());
    }

    pub fn detach(&mut self, id: ClientId, reason: &str) {
        let Some(c) = self.clients.get_mut(&id) else {
            return;
        };
        let Some(att) = c.att.take() else {
            return;
        };
        let name = self
            .sessions
            .get(&att.session)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let reason = if reason == "detached" {
            format!("detached (from session {name})")
        } else {
            reason.to_owned()
        };
        // Leave the alternate screen state as the client found it; the client
        // restores its own terminal modes on receiving this.
        self.send(id, ServerMsg::Detached { reason });
        self.clients.remove(&id);
        let ctx = Ctx {
            pane: self.active_pane_of_session(att.session),
            ..Ctx::default()
        };
        self.queue_hook("client-detached", ctx, crate::hooks::HookScope::default());
    }

    /// Point an attached client at another session.
    pub fn switch_client(&mut self, id: ClientId, session: SessionId) {
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        if att.session != session {
            att.last_session = Some(att.session);
            att.session = session;
        }
        att.dirty = true;
        att.full_redraw = true;
        self.touch_session(session);
        self.size_session(session);
    }

    /// Whether client `id` gets 24-bit colour (`rgb-colour`): `auto` trusts
    /// what its environment advertises.
    pub fn client_rgb(&self, id: ClientId) -> bool {
        match self.cfg.rgb_colour.as_str() {
            "off" => false,
            "auto" => {
                let env = self
                    .clients
                    .get(&id)
                    .and_then(|c| c.hello.as_ref())
                    .map(|h| h.env.as_slice())
                    .unwrap_or_default();
                let var = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
                var("COLORTERM").is_some_and(|v| v == "truecolor" || v == "24bit")
                    || var("TERM").is_some_and(|t| t.ends_with("-direct"))
                    || var("WT_SESSION").is_some()
            }
            _ => true,
        }
    }

    /// Re-read `rgb-colour` for every attached client.
    pub fn apply_rgb_colour(&mut self) {
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();
        for id in ids {
            let rgb = self.client_rgb(id);
            if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                if let Some(t) = a.term.as_mut() {
                    t.backend_mut().set_rgb(rgb);
                }
                a.full_redraw = true;
                a.dirty = true;
            }
        }
    }

    /// Hand each client's terminal to `lock-command` until it exits; a
    /// client already locked is left as it is.
    pub fn lock_clients(&mut self, clients: &[ClientId]) -> Result<(), String> {
        if self.cfg.lock_command.is_empty() {
            return Err("lock-command is empty".into());
        }
        let command = self.cfg.lock_command.clone();
        for &c in clients {
            let Some(att) = self.clients.get_mut(&c).and_then(|c| c.att.as_mut()) else {
                continue;
            };
            if !att.locked {
                att.locked = true;
                self.send(
                    c,
                    ServerMsg::Lock {
                        command: command.clone(),
                    },
                );
            }
        }
        Ok(())
    }

    /// `lock-after-time`: lock each client idle that long.
    pub fn lock_idle_clients(&mut self) {
        let secs = self.cfg.lock_after_time;
        if secs == 0 || self.cfg.lock_command.is_empty() {
            return;
        }
        let idle: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| {
                c.att.as_ref().is_some_and(|a| {
                    !a.locked && a.last_input.elapsed() >= std::time::Duration::from_secs(secs)
                })
            })
            .map(|c| c.id)
            .collect();
        if !idle.is_empty()
            && let Err(e) = self.lock_clients(&idle)
        {
            self.log_message(format!("lock-after-time: {e}"));
        }
    }

    pub fn touch_session(&mut self, session: SessionId) {
        if let Some(s) = self.sessions.get_mut(&session) {
            s.last_used = Instant::now();
        }
    }

    /// Size every window of `session` for the attached client that used it
    /// most recently (tmux's `window-size latest`).
    pub fn size_session(&mut self, session: SessionId) {
        let size = self
            .clients
            .values()
            .filter_map(|c| c.att.as_ref())
            .filter(|a| a.session == session)
            .max_by_key(|a| a.last_input)
            .map(|a| (a.cols, a.rows.saturating_sub(STATUS_ROWS).max(1)));
        let Some((cols, rows)) = size else {
            return;
        };
        let windows: Vec<WindowId> = self
            .sessions
            .get(&session)
            .map(|s| s.windows.values().copied().collect())
            .unwrap_or_default();
        for w in windows {
            if let Some(win) = self.windows.get_mut(&w)
                && !win.manual_size
                && (win.cols, win.rows) != (cols, rows)
            {
                win.cols = cols;
                win.rows = rows;
                self.relayout(w);
            }
        }
    }

    /// Recompute pane rects for a window and resize panes whose size changed.
    pub fn relayout(&mut self, window: WindowId) {
        let Some(win) = self.windows.get(&window) else {
            return;
        };
        for (pid, r) in win.visible_rects() {
            if let Some(p) = self.panes.get_mut(&pid) {
                let resized = (p.rect.w, p.rect.h) != (r.w, r.h);
                p.rect = r;
                if resized && r.w > 0 && r.h > 0 {
                    p.emu.resize(r.h, r.w);
                    if let Err(e) = p.pty.resize(r.h, r.w) {
                        debug!(pane = pid, error = %e, "pty resize failed");
                    }
                    if let Some(cm) = p.copy.as_mut() {
                        cm.resize(r.w, r.h);
                    }
                }
            }
        }
        self.mark_window_dirty(window);
    }
}
