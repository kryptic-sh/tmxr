//! Input from attached clients: keys, mouse, paste, binds.

use std::time::{Duration, Instant};

use crossterm::event::{Event as TermEvent, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;
use tmxr_command::Key;
use tmxr_term::{encode_key, encode_paste};

use super::Server;
use crate::cmds::{Ctx, Outcome};
use crate::model::{ClientId, PaneId, SessionId};
use crate::overlay::{Overlay, OverlayAction};

impl Server {
    pub(super) fn input(&mut self, id: ClientId, ev: TermEvent) {
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        att.last_input = Instant::now();
        let session = att.session;
        match ev {
            TermEvent::Key(k) if k.kind != KeyEventKind::Release => self.key(id, k),
            TermEvent::Key(_) => {}
            TermEvent::Paste(text) => {
                if let Some(ov) = self
                    .clients
                    .get_mut(&id)
                    .and_then(|c| c.att.as_mut())
                    .and_then(|a| a.overlay.as_mut())
                {
                    ov.paste(&text);
                    self.mark_client_dirty(id);
                } else {
                    self.paste_to_active(session, &text);
                }
            }
            TermEvent::Resize(cols, rows) => {
                if let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                    att.cols = cols.max(1);
                    att.rows = rows.max(1);
                    att.full_redraw = true;
                    att.dirty = true;
                    if let Some(t) = att.term.as_mut() {
                        t.backend_mut().set_size(att.cols, att.rows);
                        let _ = t.resize(Rect::new(0, 0, att.cols, att.rows));
                    }
                }
                self.size_session(session);
            }
            TermEvent::Mouse(m) => crate::mouse::handle(self, id, m),
            TermEvent::FocusGained | TermEvent::FocusLost => {}
        }
    }

    pub fn active_pane_of_session(&self, session: SessionId) -> Option<PaneId> {
        let w = self.sessions.get(&session)?.current_window()?;
        self.windows.get(&w).map(|w| w.active)
    }

    fn key(&mut self, id: ClientId, ev: KeyEvent) {
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        // A message lasts display-time or until a key is pressed, as in tmux;
        // otherwise it would hide a prompt being typed into.
        if att.message.take().is_some() {
            att.dirty = true;
        }
        if let Some(mut ov) = att.overlay.take() {
            let action = ov.key(&ev);
            match action {
                OverlayAction::Keep => {
                    if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                        a.overlay = Some(ov);
                    }
                }
                OverlayAction::Close => {}
                OverlayAction::Run(cmd) => self.run_bind(id, &cmd, None),
                OverlayAction::Switch(sid) => self.switch_client(id, sid),
                OverlayAction::SwitchWindow(sid, idx) => {
                    self.switch_client(id, sid);
                    let _ = self.select_window(sid, idx);
                }
            }
            self.mark_client_dirty(id);
            return;
        }
        let key = Key::from_event(&ev);
        let session = att.session;
        let now = Instant::now();
        let repeating = att.repeat_until.is_some_and(|t| t > now);
        let table = std::mem::replace(&mut att.table, "root".into());
        att.repeat_until = None;
        if table != "root" {
            att.dirty = true;
            if let Some(b) = self.keys.get(&table, &key).cloned() {
                if !repeating || b.repeat {
                    if b.repeat
                        && let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut())
                    {
                        a.table.clone_from(&table);
                        a.repeat_until = Some(now + Duration::from_millis(self.cfg.repeat_time));
                    }
                    self.run_bind(id, &b.cmd, Some(ev));
                    return;
                }
            } else if !repeating {
                return;
            }
            // A non-repeatable key while repeating is handled as a fresh key.
        }
        if key == self.prefix {
            if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                a.table = "prefix".into();
                a.dirty = true;
            }
            return;
        }
        let active = self.active_pane_of_session(session);
        let in_copy = active
            .and_then(|p| self.panes.get(&p))
            .is_some_and(|p| p.copy.is_some());
        if in_copy {
            // A count digit or the character a jump waits for goes to copy
            // mode itself, ahead of its key table.
            let pane = active.expect("copy mode is in the active pane");
            if let Some(p) = self.panes.get_mut(&pane)
                && p.copy.as_mut().is_some_and(|cm| cm.take_key(&ev))
            {
                let window = p.window;
                self.mark_window_dirty(window);
                return;
            }
            if let Some(b) = self.keys.get("copy-mode-vi", &key).cloned() {
                self.run_bind(id, &b.cmd, Some(ev));
            }
            return;
        }
        if let Some(b) = self.keys.get("root", &key).cloned() {
            self.run_bind(id, &b.cmd, Some(ev));
            return;
        }
        if let Some(p) = active {
            self.send_key_to_pane(p, &ev);
        }
    }

    /// Run a bind's command list for an attached client; errors become a
    /// status-line message.
    pub fn run_bind(&mut self, id: ClientId, cmd: &str, key: Option<KeyEvent>) {
        let session = self
            .clients
            .get(&id)
            .and_then(|c| c.att.as_ref())
            .map(|a| a.session);
        let pane = session.and_then(|s| self.active_pane_of_session(s));
        let ctx = Ctx {
            client: Some(id),
            pane,
            key,
            cwd: None,
            env: Vec::new(),
        };
        let out = crate::cmds::run_string(self, &ctx, cmd);
        self.report(id, &out);
    }

    /// Show what a command run for an attached client printed: an error or a
    /// one-line result in the status line, longer output in a text view.
    pub(super) fn report(&mut self, id: ClientId, out: &Outcome) {
        if !out.stderr.is_empty() {
            self.show_message(id, out.stderr.trim_end().to_owned());
        } else if !out.stdout.is_empty() {
            let lines: Vec<String> = out.stdout.lines().map(str::to_owned).collect();
            if lines.len() == 1 {
                self.show_message(id, lines[0].clone());
            } else if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                a.overlay = Some(Overlay::text(lines));
            }
        }
        self.mark_client_dirty(id);
    }

    /// Encode a key for a pane (and, with synchronize-panes, its siblings).
    pub fn send_key_to_pane(&mut self, pane: PaneId, ev: &KeyEvent) {
        let always = self.cfg.extended_keys == "always";
        let targets = self.input_targets(pane);
        for t in targets {
            if let Some(p) = self.panes.get_mut(&t) {
                let bytes = encode_key(ev, p.emu.input_modes(), always);
                if !bytes.is_empty() {
                    let _ = p.pty.write(&bytes);
                }
            }
        }
    }

    pub fn paste_to_active(&mut self, session: SessionId, text: &str) {
        if let Some(p) = self.active_pane_of_session(session) {
            self.paste_to_pane(p, text);
        }
    }

    pub fn paste_to_pane(&mut self, pane: PaneId, text: &str) {
        for t in self.input_targets(pane) {
            if let Some(p) = self.panes.get_mut(&t) {
                let bytes = encode_paste(text, p.emu.input_modes());
                let _ = p.pty.write(&bytes);
            }
        }
    }

    /// The pane itself, plus its siblings when the window synchronizes input.
    fn input_targets(&self, pane: PaneId) -> Vec<PaneId> {
        let Some(wid) = self.panes.get(&pane).map(|p| p.window) else {
            return Vec::new();
        };
        match self.windows.get(&wid) {
            Some(w) if w.synchronize => w.panes(),
            _ => vec![pane],
        }
    }
}
