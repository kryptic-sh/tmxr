//! Input from attached clients: keys, mouse, paste, binds.

use std::time::{Duration, Instant};

use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use tmxr_command::Key;
use tmxr_term::{conpty_key, encode_key, encode_paste};

use super::Server;
use crate::cmds::{Ctx, Outcome};
use crate::model::{ClientId, PaneId, SessionId};
use crate::overlay::{Overlay, OverlayAction};

/// The most names a prompt's completion menu shows, as tmux 3.6's (its
/// items' keys are the digits).
const MENU_ITEMS: usize = 10;

impl Server {
    pub(super) fn input(&mut self, id: ClientId, ev: TermEvent) {
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        att.last_input = Instant::now();
        // Back from away, or focus changing, sizes the windows again.
        let came_back = std::mem::take(&mut att.away);
        let focused = att.focused;
        let session = att.session;
        // Everything but a key's release and losing focus makes the client
        // its window's latest, as tmux's server_client_key_callback and
        // its resize do.
        let latest = !matches!(
            ev,
            TermEvent::FocusLost
                | TermEvent::Key(KeyEvent {
                    kind: KeyEventKind::Release,
                    ..
                })
        );
        let Some(ev) = self.escape_time(id, ev) else {
            return;
        };
        match ev {
            TermEvent::Key(k) if k.kind != KeyEventKind::Release => self.client_key(id, k, false),
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
                } else if !self.client_read_only(id) {
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
                    if let Some(crate::overlay::Overlay::Popup(p)) = att.overlay.as_mut() {
                        p.fit(att.cols, att.rows);
                    }
                }
                self.size_session(session);
            }
            TermEvent::Mouse(m) => crate::mouse::handle(self, id, m),
            TermEvent::FocusGained | TermEvent::FocusLost => {
                let gained = matches!(ev, TermEvent::FocusGained);
                if let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                    att.focused = gained;
                }
                let ctx = crate::cmds::Ctx {
                    client: Some(id),
                    ..crate::cmds::Ctx::default()
                };
                let hook = if gained {
                    "client-focus-in"
                } else {
                    "client-focus-out"
                };
                self.queue_hook(hook, ctx, crate::hooks::HookScope::default());
            }
        }
        if latest {
            self.update_latest(id);
        }
        let refocused = self
            .clients
            .get(&id)
            .and_then(|c| c.att.as_ref())
            .is_some_and(|a| a.focused != focused);
        if came_back || refocused {
            self.size_all_sessions();
        }
    }

    /// Make client `id` its current window's latest client, running
    /// `client-active` when that changes it, as tmux's
    /// server_client_update_latest.
    pub(crate) fn update_latest(&mut self, id: ClientId) {
        if self.make_latest(id) {
            let ctx = crate::cmds::Ctx {
                client: Some(id),
                ..crate::cmds::Ctx::default()
            };
            self.queue_hook("client-active", ctx, crate::hooks::HookScope::default());
        }
    }

    /// Make client `id` its current window's latest client with no hook,
    /// as attaching and switching do in tmux; whether that changed it.
    pub(crate) fn make_latest(&mut self, id: ClientId) -> bool {
        let Some(session) = self
            .clients
            .get(&id)
            .and_then(|c| c.att.as_ref())
            .map(|a| a.session)
        else {
            return false;
        };
        let Some(w) = self
            .sessions
            .get(&session)
            .and_then(crate::model::Session::current_window)
            .and_then(|w| self.windows.get_mut(&w))
        else {
            return false;
        };
        w.latest.replace(id) != Some(id)
    }

    /// Client `id` has gone: each window it was the latest client of goes
    /// to the most recently active other client showing it, as tmux's
    /// server_client_lost does.
    pub(crate) fn latest_gone(&mut self, id: ClientId) {
        let windows: Vec<_> = self
            .windows
            .values_mut()
            .filter(|w| w.latest == Some(id))
            .map(|w| {
                w.latest = None;
                w.id
            })
            .collect();
        for wid in windows {
            let next = self
                .clients
                .values()
                .filter(|c| c.id != id)
                .filter_map(|c| c.att.as_ref().map(|a| (c.id, a.session, a.last_input)))
                .filter(|(_, s, _)| {
                    self.sessions
                        .get(s)
                        .and_then(crate::model::Session::current_window)
                        == Some(wid)
                })
                .max_by_key(|(_, _, t)| *t)
                .map(|(c, _, _)| c);
            if let Some(c) = next {
                self.update_latest(c);
            }
        }
    }

    /// Add a submitted prompt entry to its type's history: the same entry
    /// twice in a row is kept once, and the oldest go past the limit.
    /// The names client `id`'s prompt's Tab completes from: commands and
    /// their aliases, option names and layouts, the sessions and their
    /// windows; and how many items its menu has room for.
    fn prompt_completions(&self, id: ClientId) -> crate::overlay::Completions {
        let commands = tmxr_command::table::COMMANDS
            .iter()
            .flat_map(|c| std::iter::once(c.name).chain(c.alias))
            .map(str::to_owned)
            .collect();
        let others = crate::cmds::SERVER_OPTIONS
            .iter()
            .chain(tmxr_config::SessionOptions::NAMES)
            .chain(tmxr_config::WindowOptions::NAMES)
            .chain(tmxr_config::PANE_OPTIONS)
            .chain(crate::layout::PRESETS)
            .map(|n| (*n).to_owned())
            .collect();
        let sessions = self
            .sessions
            .values()
            .map(|s| (s.name.clone(), s.id))
            .collect();
        let windows = self
            .sessions
            .values()
            .flat_map(|s| {
                s.windows.iter().filter_map(|(i, w)| {
                    let name = self.windows.get(w)?.name.clone();
                    Some((s.name.clone(), *i, name))
                })
            })
            .collect();
        let att = self.clients.get(&id).and_then(|c| c.att.as_ref());
        let current = att
            .and_then(|a| self.sessions.get(&a.session))
            .map(|s| s.name.clone());
        // tmux's: the rows off the status line, less the menu's border.
        let free = att.map_or(0, |a| {
            usize::from(a.rows.saturating_sub(self.client_status_rows(id)))
        });
        crate::overlay::Completions {
            commands,
            others,
            sessions,
            windows,
            current,
            menu_rows: free.saturating_sub(2).min(MENU_ITEMS),
        }
    }

    pub fn remember_prompt(&mut self, kind: &str, entry: String) {
        if entry.is_empty() {
            return;
        }
        let limit = self.cfg.prompt_history_limit;
        let list = self.prompt_history.entry(kind.to_owned()).or_default();
        if list.last() != Some(&entry) {
            list.push(entry);
        }
        let excess = list.len().saturating_sub(limit);
        list.drain(..excess);
    }

    /// The key table for `pane` in copy mode.
    pub fn copy_table(&self, pane: PaneId) -> &'static str {
        if self.emacs_keys(pane) {
            "copy-mode"
        } else {
            "copy-mode-vi"
        }
    }

    /// `mode-keys emacs` for `pane`.
    fn emacs_keys(&self, pane: PaneId) -> bool {
        self.pane_options(pane).mode_keys == tmxr_config::ModeKeys::Emacs
    }

    pub fn active_pane_of_session(&self, session: SessionId) -> Option<PaneId> {
        let w = self.sessions.get(&session)?.current_window()?;
        self.windows.get(&w).map(|w| w.active)
    }

    /// A mouse event on client `id`'s open menu: tmux's rules in
    /// [`crate::menu::Menu::mouse`], an item it chooses run against the
    /// menu's target.
    pub fn menu_mouse(&mut self, id: ClientId, m: crossterm::event::MouseEvent) {
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        // The whole client, as the menu is drawn in.
        let (cols, rows) = (att.cols, att.rows);
        let Some(Overlay::Menu(mut menu)) = att.overlay.take() else {
            return;
        };
        let action = menu.mouse(&m, cols, rows);
        att.dirty = true;
        let ctx = Ctx {
            client: Some(id),
            pane: menu.pane,
            mouse: menu.mouse,
            ..Ctx::default()
        };
        match action {
            OverlayAction::Run(cmd) => self.run_bind_ctx(id, &ctx, &cmd),
            OverlayAction::Close => {}
            _ => att.overlay = Some(Overlay::Menu(menu)),
        }
    }

    /// A key from client `id`, through its overlay and key tables;
    /// `send-keys -K` gives keys here too.
    /// A key from client `id`'s terminal, or (`sent`) `send-keys -K`.
    pub(crate) fn client_key(&mut self, id: ClientId, ev: KeyEvent, sent: bool) {
        // A key is activity in the client's session, as tmux's
        // session_update_activity.
        if let Some(s) = self
            .clients
            .get(&id)
            .and_then(|c| c.att.as_ref())
            .and_then(|a| self.sessions.get_mut(&a.session))
        {
            s.activity = std::time::SystemTime::now();
        }
        // What a prompt's Tab completes from, gathered for a Tab alone.
        let completions =
            (ev.code == crossterm::event::KeyCode::Tab).then(|| self.prompt_completions(id));
        // A prompt's keys, word separators and newest buffer as they are
        // now, as tmux reads them for each key.
        let prompt_env = self
            .clients
            .get(&id)
            .and_then(|c| c.att.as_ref())
            .filter(|a| matches!(a.overlay, Some(Overlay::Prompt(_))))
            .map(|a| {
                let opts = self.session_options(a.session);
                (
                    opts.status_keys == tmxr_config::ModeKeys::Vi,
                    opts.word_separators.clone(),
                    self.buffers.front().map(|b| b.data.clone()),
                )
            });
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        // A message lasts display-time or until a key is pressed, as in tmux;
        // otherwise it would hide a prompt being typed into. -N keeps it.
        if !att.message_keep && att.message.take().is_some() {
            att.dirty = true;
        }
        if let Some(mut ov) = att.overlay.take() {
            if let (Overlay::Prompt(p), Some((vi, separators, paste))) = (&mut ov, prompt_env) {
                (p.vi, p.separators, p.paste) = (vi, separators, paste);
                p.completions = completions;
            }
            let action = ov.key(&ev);
            // display-panes -N: the overlay stays and the key goes on.
            if action == OverlayAction::Pass {
                att.overlay = Some(ov);
            } else {
                // A menu's item runs against the menu's target, as tmux's.
                let menu_ctx = match &ov {
                    Overlay::Menu(m) => Some(Ctx {
                        client: Some(id),
                        pane: m.pane,
                        mouse: m.mouse,
                        ..Ctx::default()
                    }),
                    _ => None,
                };
                if let Overlay::Prompt(p) = &ov
                    && p.submitted
                    && !p.key
                {
                    self.remember_prompt(&p.kind, p.text());
                }
                match action {
                    OverlayAction::Keep => {
                        if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                            a.overlay = Some(ov);
                        }
                    }
                    OverlayAction::Close => {}
                    // Answered before this match.
                    OverlayAction::Pass => {}
                    OverlayAction::Run(cmd) => match menu_ctx {
                        Some(ctx) => self.run_bind_ctx(id, &ctx, &cmd),
                        None => self.run_bind(id, &cmd, None),
                    },
                    OverlayAction::Preview(cmd) => {
                        self.run_bind(id, &cmd, None);
                        // The prompt stays, unless the command opened something
                        // in its place.
                        if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut())
                            && a.overlay.is_none()
                        {
                            a.overlay = Some(ov);
                        }
                    }
                    OverlayAction::Edit(line) => {
                        if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                            a.overlay = Some(Overlay::prompt(":".into(), line, None));
                        }
                    }
                    OverlayAction::EditBuffer(name) => {
                        if let Err(e) = self.edit_buffer(id, &name) {
                            self.show_message_for(id, e, None, false, false);
                        }
                    }
                    OverlayAction::Switch(sid) => self.switch_client(id, sid),
                    OverlayAction::SwitchWindow(sid, idx) => {
                        self.switch_client(id, sid);
                        let _ = self.select_window(sid, idx);
                    }
                }
                self.mark_client_dirty(id);
                return;
            }
        }
        let key = Key::from_event(&ev);
        let session = att.session;
        let now = Instant::now();
        let since = self
            .sessions
            .get_mut(&session)
            .and_then(|s| s.key_at.replace(now))
            .map(|t| now.duration_since(t));
        if !sent && self.assume_paste(session, since) {
            self.paste_key(id, session, &ev);
            return;
        }
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        let repeating = att.repeat_until.is_some_and(|t| t > now);
        // prefix-timeout: a prefix table waited on too long gives the key
        // to the root table, unless it repeats a repeatable binding, as
        // tmux's `server_client_key_callback`.
        let timeout = self.cfg.prefix_timeout;
        let expired = timeout > 0
            && att.table == "prefix"
            && now.duration_since(att.table_at) > Duration::from_millis(timeout)
            && !(repeating
                && self
                    .keys
                    .get(&att.table, &key.into())
                    .is_some_and(|b| b.repeat));
        let root = self.default_key_table(session);
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        let table = std::mem::replace(&mut att.table, root.clone());
        att.repeat_until = None;
        if expired {
            att.dirty = true;
        } else if table != root {
            att.dirty = true;
            if let Some(b) = self.keys.get(&table, &key.into()).cloned() {
                if !repeating || b.repeat {
                    let repeat_time = self.session_options(session).repeat_time;
                    if b.repeat
                        && let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut())
                    {
                        a.table.clone_from(&table);
                        a.repeat_until = Some(now + Duration::from_millis(repeat_time));
                    }
                    self.run_bind(id, &b.cmd, Some(ev));
                    return;
                }
            } else if !repeating {
                // As tmux's: an unbound key gets a second try in the default
                // table, and is not passed to the pane either way.
                if let Some(b) = self.keys.get(&root, &key.into()).cloned() {
                    self.run_bind(id, &b.cmd, Some(ev));
                }
                return;
            }
            // A non-repeatable key while repeating is handled as a fresh key.
        }
        if self.is_prefix(session, &key) {
            if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                a.table = "prefix".into();
                a.table_at = now;
                a.dirty = true;
            }
            return;
        }
        let active = self.active_pane_of_session(session);
        // Any key leaves clock mode, as in tmux, and is not passed on.
        if let Some(p) = active.and_then(|p| self.panes.get_mut(&p))
            && std::mem::take(&mut p.clock)
        {
            let window = p.window;
            self.mark_window_dirty(window);
            return;
        }
        let in_copy = active
            .and_then(|p| self.panes.get(&p))
            .is_some_and(|p| p.copy.is_some());
        if in_copy {
            // A count digit or the character a jump waits for goes to copy
            // mode itself, ahead of its key table.
            let pane = active.expect("copy mode is in the active pane");
            let emacs = self.emacs_keys(pane);
            if let Some(p) = self.panes.get_mut(&pane)
                && p.copy.as_mut().is_some_and(|cm| cm.take_key(&ev, emacs))
            {
                let window = p.window;
                self.mark_window_dirty(window);
                return;
            }
            if let Some(b) = self.keys.get(self.copy_table(pane), &key.into()).cloned() {
                self.run_bind(id, &b.cmd, Some(ev));
            }
            return;
        }
        if let Some(b) = self.keys.get(&root, &key.into()).cloned() {
            self.run_bind(id, &b.cmd, Some(ev));
            return;
        }
        if let Some(p) = active
            && !self.client_read_only(id)
        {
            self.send_key_to_pane(p, &ev);
        }
    }

    /// Whether `id` is a read-only client, whose input reaches no pane.
    pub fn client_read_only(&self, id: ClientId) -> bool {
        self.clients.get(&id).is_some_and(super::Client::read_only)
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
            ..Ctx::default()
        };
        self.run_bind_ctx(id, &ctx, cmd);
    }

    /// Run a bind's command list in `ctx`, reporting to client `id`.
    pub fn run_bind_ctx(&mut self, id: ClientId, ctx: &Ctx, cmd: &str) {
        let out = crate::cmds::run_string(self, ctx, cmd);
        self.report(id, &out);
    }

    /// Show what a command run for an attached client printed: an error or a
    /// one-line result in the status line, longer output in a text view.
    pub fn report(&mut self, id: ClientId, out: &Outcome) {
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

    /// Whether a key `since` the last one is pasted text, as tmux's
    /// server_client_is_assume_paste: the first key within
    /// `assume-paste-time` of the one before starts assuming a paste and is
    /// still a key, each one after it is pasted, and a slower key ends it.
    fn assume_paste(&mut self, session: SessionId, since: Option<Duration>) -> bool {
        let t = self.session_options(session).assume_paste_time;
        let Some(s) = self.sessions.get_mut(&session) else {
            return false;
        };
        if t == 0 {
            return false;
        }
        if since.is_some_and(|d| d < Duration::from_millis(t)) {
            std::mem::replace(&mut s.pasting, true)
        } else {
            s.pasting = false;
            false
        }
    }

    /// A key taken as pasted text, as tmux's paste_key: to the session's
    /// active pane (and its synchronized siblings) as it is, unless the
    /// client is read-only or the pane is in a mode or takes no input.
    fn paste_key(&mut self, id: ClientId, session: SessionId, ev: &KeyEvent) {
        if self.client_read_only(id) {
            return;
        }
        let Some(pane) = self.active_pane_of_session(session) else {
            return;
        };
        if self
            .panes
            .get(&pane)
            .is_none_or(|p| p.copy.is_some() || p.clock || p.input_off || p.dead.is_some())
        {
            return;
        }
        self.send_key_to_pane(pane, ev);
    }

    /// `escape-time`, as tmux's: a lone Escape is held that long, and a key
    /// pressed before it is up goes as Meta and that key; anything else
    /// sends the Escape first. Returns the event to handle, none while one
    /// is held. (A key's release, which the kitty protocol and Windows
    /// send, changes nothing.)
    fn escape_time(&mut self, id: ClientId, ev: TermEvent) -> Option<TermEvent> {
        let wait = Duration::from_millis(self.cfg.escape_time);
        let att = self.clients.get_mut(&id)?.att.as_mut()?;
        let press = match &ev {
            TermEvent::Key(k) if k.kind == KeyEventKind::Release => return Some(ev),
            TermEvent::Key(k) => Some(*k),
            _ => None,
        };
        let now = Instant::now();
        if let Some(deadline) = att.pending_escape.take() {
            match press {
                Some(mut k) if now < deadline && !k.modifiers.contains(KeyModifiers::ALT) => {
                    k.modifiers |= KeyModifiers::ALT;
                    return Some(TermEvent::Key(k));
                }
                _ => self.client_key(id, escape_key(), false),
            }
        }
        let lone = press.is_some_and(|k| k.code == KeyCode::Esc && k.modifiers.is_empty());
        if lone && !wait.is_zero() {
            let att = self.clients.get_mut(&id)?.att.as_mut()?;
            att.pending_escape = Some(now + wait);
            return None;
        }
        Some(ev)
    }

    /// Escapes whose `escape-time` is up, sent as Escape.
    pub(super) fn flush_escapes(&mut self) {
        let now = Instant::now();
        let due: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| {
                c.att
                    .as_ref()
                    .and_then(|a| a.pending_escape)
                    .is_some_and(|d| d <= now)
            })
            .map(|c| c.id)
            .collect();
        for id in due {
            if let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                att.pending_escape = None;
            }
            self.client_key(id, escape_key(), false);
        }
    }

    /// The soonest a held Escape is due, for the server loop to wake.
    pub(super) fn next_escape(&self) -> Option<Instant> {
        self.clients
            .values()
            .filter_map(|c| c.att.as_ref()?.pending_escape)
            .min()
    }

    /// The `extended-keys` option.
    pub fn extended_keys(&self) -> tmxr_term::ExtendedKeys {
        tmxr_term::ExtendedKeys::from_option(&self.cfg.extended_keys)
    }

    /// A terminal for a new pane or popup, under the `extended-keys`
    /// option as tmux's new screen.
    pub fn new_emulator(&self, rows: u16, cols: u16, history: usize) -> tmxr_term::Emulator {
        let mut emu = tmxr_term::Emulator::new(rows, cols, history);
        emu.set_extended_keys(self.extended_keys());
        emu.reset_keys();
        emu
    }

    /// Encode a key for a pane (and, with synchronize-panes, its siblings).
    pub fn send_key_to_pane(&mut self, pane: PaneId, ev: &KeyEvent) {
        let option = self.extended_keys();
        let targets = self.input_targets(pane);
        for t in targets {
            if let Some(p) = self.panes.get_mut(&t) {
                let mut bytes = encode_key(ev, p.emu.input_modes(), option);
                // Every Windows pane is a ConPTY.
                if cfg!(windows)
                    && let Some(win32) = conpty_key(ev, &bytes)
                {
                    bytes = win32;
                }
                if !bytes.is_empty() {
                    let _ = p.pty.write(&bytes);
                }
            }
        }
    }

    /// Type `text` into a pane as it is (`send-keys` words that name no
    /// key), and into its siblings with synchronize-panes, as tmux's.
    pub fn type_to_pane(&mut self, pane: PaneId, text: &str) {
        self.type_bytes_to_pane(pane, text.as_bytes());
    }

    /// Write `bytes` to a pane (and its synchronized siblings) as they are:
    /// [`Server::type_to_pane`], and `send-keys -H`.
    pub fn type_bytes_to_pane(&mut self, pane: PaneId, bytes: &[u8]) {
        for t in self.input_targets(pane) {
            if let Some(p) = self.panes.get_mut(&t) {
                let _ = p.pty.write(bytes);
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

    /// The pane itself, plus, when it synchronizes input, its siblings that
    /// do too and can take it: not in a mode (copy or clock mode), not dead,
    /// and not hidden by a zoom (tmux's `window_pane_copy_key`).
    fn input_targets(&self, pane: PaneId) -> Vec<PaneId> {
        // A pane with its input off takes none, and gives none to its
        // synchronized siblings, as tmux's `window_pane_key`.
        let Some(wid) = self
            .panes
            .get(&pane)
            .filter(|p| !p.input_off)
            .map(|p| p.window)
        else {
            return Vec::new();
        };
        let synchronized = |p: PaneId| self.pane_options(p).synchronize_panes;
        match self.windows.get(&wid) {
            Some(w) if synchronized(pane) => {
                let takes_input = |p: PaneId| {
                    self.panes.get(&p).is_some_and(|q| {
                        q.copy.is_none() && !q.clock && q.dead.is_none() && !q.input_off
                    }) && (!w.zoomed || p == w.active)
                        && synchronized(p)
                };
                w.panes()
                    .into_iter()
                    .filter(|&p| p == pane || takes_input(p))
                    .collect()
            }
            _ => vec![pane],
        }
    }
}

/// An Escape key press.
fn escape_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
}
