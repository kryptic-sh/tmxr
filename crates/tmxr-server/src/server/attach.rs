//! Attaching clients, and sizing sessions to them.

use crate::cmds::Ctx;
use std::time::Instant;

use ratatui::{Terminal, TerminalOptions, Viewport, layout::Rect};
use tmxr_proto::ServerMsg;
use tracing::debug;

use super::{Attached, Server};
use crate::backend::{AnsiBackend, ColourDepth};
use crate::model::{ClientId, SessionId, WindowId};

impl Server {
    pub fn attach(&mut self, id: ClientId, session: SessionId) {
        let Some(term) = self.client_terminal(id).cloned() else {
            return;
        };
        let (cols, rows) = (term.cols.max(1), term.rows.max(1));
        let area = Rect::new(0, 0, cols, rows);
        let mut backend = AnsiBackend::new(cols, rows);
        backend.set_depth(self.client_depth(id));
        let terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )
        .ok();
        let table = self.default_key_table(session);
        let Some(c) = self.clients.get_mut(&id) else {
            return;
        };
        c.att = Some(Attached {
            session,
            cols,
            rows,
            term: terminal,
            table,
            table_at: Instant::now(),
            cursor_style: 0,
            repeat_until: None,
            overlay: None,
            message: None,
            message_keep: false,
            last_session: None,
            dirty: true,
            full_redraw: true,
            locked: false,
            last_input: Instant::now(),
            key_at: None,
            assume_pasting: false,
            drag: None,
            press: None,
            click: None,
            status_ranges: crate::render::StatusRanges::default(),
            mouse: false,
            focus_events: false,
            // As tmux's: focused until its terminal reports otherwise.
            focused: true,
            title: None,
            pan: None,
            status_at: Instant::now(),
        });
        self.send(id, ServerMsg::Attached);
        self.make_latest(id);
        self.touch_session(session);
        self.size_session(session);
        let ctx = Ctx {
            client: Some(id),
            pane: self.active_pane_of_session(session),
            ..Ctx::default()
        };
        self.queue_hook("client-attached", ctx, crate::hooks::HookScope::default());
        // A config file that could not be used is said where it is seen, as
        // tmux shows its config errors to the first client.
        if let Some(e) = self.config_error.take() {
            self.show_message(id, e);
        }
        self.check_unattached();
    }

    pub fn detach(&mut self, id: ClientId, reason: &str) {
        self.detach_as(id, reason, false, None);
    }

    /// Detach client `id` and have it run the shell command `line` in its
    /// place (`detach-client -E`), with its session's `default-shell`, as
    /// tmux's server_client_exec. An empty line does nothing.
    pub fn detach_and_exec(&mut self, id: ClientId, line: &str) {
        if line.is_empty() {
            return;
        }
        let shell = self.client_session_options(id).default_shell.clone();
        let argv = crate::util::shell_command(line, shell.as_deref());
        self.detach_as(id, "detached", false, Some(argv));
    }

    /// Detach client `id` and have it send its parent process SIGHUP, which
    /// usually closes the terminal it ran in (`detach-client -P`).
    pub fn detach_and_hang_up(&mut self, id: ClientId) {
        self.detach_as(id, "detached and SIGHUP", true, None);
    }

    fn detach_as(&mut self, id: ClientId, reason: &str, hangup: bool, exec: Option<Vec<String>>) {
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
        let reason = if reason.starts_with("detached") {
            format!("{reason} (from session {name})")
        } else {
            reason.to_owned()
        };
        // Leave the alternate screen state as the client found it; the client
        // restores its own terminal modes on receiving this.
        self.send(
            id,
            ServerMsg::Detached {
                reason,
                hangup,
                exec,
            },
        );
        self.clients.remove(&id);
        let ctx = Ctx {
            pane: self.active_pane_of_session(att.session),
            ..Ctx::default()
        };
        self.queue_hook("client-detached", ctx, crate::hooks::HookScope::default());
        self.latest_gone(id);
        self.check_unattached();
    }

    /// Point an attached client at another session.
    pub fn switch_client(&mut self, id: ClientId, session: SessionId) {
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        if att.session != session {
            att.last_session = Some(att.session);
            att.session = session;
            // A new session starts the status timer again, as tmux's.
            att.status_at = Instant::now();
        }
        att.dirty = true;
        att.full_redraw = true;
        self.make_latest(id);
        self.touch_session(session);
        self.size_session(session);
        self.check_unattached();
    }

    /// The colours client `id` gets (`rgb-colour`): `auto` trusts what its
    /// environment advertises, 24-bit colour, else the 256 for a `TERM`
    /// naming them (or none), else the 16, as tmux reads a terminal with
    /// fewer from its terminfo.
    pub fn client_depth(&self, id: ClientId) -> ColourDepth {
        match self.cfg.rgb_colour.as_str() {
            "off" => ColourDepth::Colours256,
            "16" => ColourDepth::Colours16,
            "auto" => {
                let env = self
                    .clients
                    .get(&id)
                    .and_then(|c| c.hello.as_ref())
                    .map(|h| h.env.as_slice())
                    .unwrap_or_default();
                let var = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
                let term = var("TERM").filter(|t| !t.is_empty());
                if var("COLORTERM").is_some_and(|v| v == "truecolor" || v == "24bit")
                    || term.is_some_and(|t| t.ends_with("-direct"))
                    || var("WT_SESSION").is_some()
                {
                    ColourDepth::TrueColour
                } else if term.is_none_or(|t| t.contains("256col")) {
                    ColourDepth::Colours256
                } else {
                    ColourDepth::Colours16
                }
            }
            _ => ColourDepth::TrueColour,
        }
    }

    /// Re-read `rgb-colour` for every attached client.
    pub fn apply_rgb_colour(&mut self) {
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();
        for id in ids {
            let depth = self.client_depth(id);
            if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                if let Some(t) = a.term.as_mut() {
                    t.backend_mut().set_depth(depth);
                }
                a.full_redraw = true;
                a.dirty = true;
            }
        }
    }

    /// Hand each client's terminal to its session's `lock-command` until it
    /// exits; a client already locked is left as it is, and one whose
    /// `lock-command` is empty is not locked.
    pub fn lock_clients(&mut self, clients: &[ClientId]) -> Result<(), String> {
        let mut unlockable = false;
        for &c in clients {
            let command = self.client_session_options(c).lock_command.clone();
            let Some(att) = self.clients.get_mut(&c).and_then(|c| c.att.as_mut()) else {
                continue;
            };
            if command.is_empty() {
                unlockable = true;
            } else if !att.locked {
                att.locked = true;
                self.send(c, ServerMsg::Lock { command });
            }
        }
        if unlockable {
            return Err("lock-command is empty".into());
        }
        Ok(())
    }

    /// `lock-after-time`: lock each client idle as long as its session's
    /// says (none when that is 0 or its `lock-command` is empty).
    pub fn lock_idle_clients(&mut self) {
        let idle: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| {
                let opts = self.client_session_options(c.id);
                let secs = opts.lock_after_time;
                secs != 0
                    && !opts.lock_command.is_empty()
                    && c.att.as_ref().is_some_and(|a| {
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
            s.activity = std::time::SystemTime::now();
            s.last_attached = Some(s.activity);
        }
    }

    /// Size every window of `session` for the attached client that used it
    /// most recently (tmux's `window-size latest`). A client attached with
    /// `-f ignore-size` counts only while every attached client has it, as
    /// tmux's `ignore_client_size`.
    pub fn size_session(&mut self, session: SessionId) {
        let windows: Vec<WindowId> = self
            .sessions
            .get(&session)
            .map(|s| s.windows.values().copied().collect())
            .unwrap_or_default();
        for w in windows {
            let Some((cols, rows)) = self.window_client_size(w) else {
                continue;
            };
            if let Some(win) = self.windows.get_mut(&w)
                && (win.cols, win.rows) != (cols, rows)
            {
                win.cols = cols;
                win.rows = rows;
                self.relayout(w);
            }
        }
    }

    /// The size window `w` takes from its clients, as tmux's
    /// recalculate_size and clients_calculate_size: every attached client
    /// whose session holds it (with `aggressive-resize`, only those whose
    /// session shows it), less its status line; by `window-size`, the
    /// largest or smallest each way, or the window's latest client when
    /// more than one has it. `None` keeps the size: `manual`, or no client.
    fn window_client_size(&self, w: WindowId) -> Option<(u16, u16)> {
        let options = self.window_options(w);
        let attached = || {
            self.clients
                .values()
                .filter_map(|c| Some((c, c.att.as_ref()?)))
        };
        let all_ignore = attached().all(|(c, _)| c.ignore_size);
        let holds = |session: SessionId| {
            self.sessions
                .get(&session)
                .is_some_and(|s| s.windows.values().any(|&x| x == w))
        };
        let shows = |session: SessionId| {
            self.sessions
                .get(&session)
                .and_then(crate::model::Session::current_window)
                == Some(w)
        };
        // As tmux's clients_with_window: the clients that size windows.
        let with_window = attached()
            .filter(|(c, a)| (all_ignore || !c.ignore_size) && holds(a.session))
            .count();
        let latest = self.windows.get(&w).and_then(|win| win.latest);
        let sizes: Vec<(u16, u16)> = attached()
            .filter(|(c, _)| all_ignore || !c.ignore_size)
            .filter(|(_, a)| {
                if options.aggressive_resize {
                    shows(a.session)
                } else {
                    holds(a.session)
                }
            })
            .filter(|(c, _)| {
                options.window_size != tmxr_config::WindowSize::Latest
                    || with_window <= 1
                    || latest == Some(c.id)
            })
            .map(|(_, a)| {
                (
                    a.cols,
                    a.rows.saturating_sub(self.status_rows(a.session)).max(1),
                )
            })
            .collect();
        if sizes.is_empty() {
            return None;
        }
        match options.window_size {
            tmxr_config::WindowSize::Largest => Some(
                sizes
                    .iter()
                    .fold((0, 0), |(c, r), &(x, y)| (c.max(x), r.max(y))),
            ),
            tmxr_config::WindowSize::Smallest | tmxr_config::WindowSize::Latest => Some(
                sizes
                    .iter()
                    .fold((u16::MAX, u16::MAX), |(c, r), &(x, y)| (c.min(x), r.min(y))),
            ),
            tmxr_config::WindowSize::Manual => None,
        }
    }

    /// Recompute pane rects for a window and resize panes whose size changed.
    pub fn relayout(&mut self, window: WindowId) {
        let Some(win) = self.windows.get(&window) else {
            return;
        };
        // Each pane gets its layout cell less its scrollbar, if it shows one.
        let rects: Vec<_> = win
            .visible_rects()
            .into_iter()
            .map(|(pid, cell)| {
                (
                    pid,
                    crate::scrollbar::content(cell, crate::scrollbar::bar(self, pid)),
                )
            })
            .collect();
        for (pid, r) in rects {
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

    /// Lay out `pane`'s window again if scrollbars are in use: whether it
    /// shows one can change with copy mode and the alternate screen, and
    /// its width with it, as tmux's `layout_fix_panes` calls do.
    pub fn refit_scrollbar(&mut self, pane: crate::model::PaneId) {
        if let Some(w) = self.panes.get(&pane).map(|p| p.window)
            && self.window_options(w).pane_scrollbars != tmxr_config::Scrollbars::Off
        {
            self.relayout(w);
        }
    }
}
