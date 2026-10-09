//! Things drawn over a client's panes that take its keys while open: the
//! command prompt, y/n confirmation, scrollable text (list-keys output),
//! the session / window picker and the `display-panes` numbers.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use hjkl_picker::{Picker, PickerAction, PickerEvent, PickerLogic};

use crate::cmds::join_args;
use crate::model::{ClientId, PaneId, SessionId};
use crate::server::Server;

/// What a key did to an overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayAction {
    Keep,
    Close,
    /// Close and run this command list.
    Run(String),
    /// Close and switch the client to this session.
    Switch(SessionId),
    /// Close, switch to the session and select the window index.
    SwitchWindow(SessionId, u32),
    /// Run this command list and keep the overlay open
    /// (`command-prompt -i`, after each edit).
    Preview(String),
}

pub enum Overlay {
    Prompt(Prompt),
    Confirm {
        prompt: String,
        cmd: String,
    },
    Text {
        lines: Vec<String>,
        top: usize,
    },
    Picker(Box<PickerOverlay>),
    /// `display-panes`: each pane's number, shown until `until`.
    Panes {
        labels: Vec<(u32, PaneId)>,
        until: Instant,
    },
}

pub struct Prompt {
    pub prompt: String,
    pub input: Vec<char>,
    pub cursor: usize,
    /// `%%` in the template is replaced by the input; no template runs the
    /// input itself.
    pub template: Option<String>,
    /// `command-prompt -k`: the next key pressed is the input, as its tmux
    /// name, quoted so any key stays one argument.
    pub key: bool,
    /// `command-prompt -i`: the template runs again after every edit, for
    /// incremental search.
    pub incremental: bool,
}

impl Overlay {
    pub fn prompt(prompt: String, initial: String, template: Option<String>) -> Self {
        let input: Vec<char> = initial.chars().collect();
        let cursor = input.len();
        Self::Prompt(Prompt {
            prompt,
            input,
            cursor,
            template,
            key: false,
            incremental: false,
        })
    }

    /// A prompt answered by one key press (`command-prompt -k`).
    pub fn key_prompt(prompt: String, template: String) -> Self {
        Self::Prompt(Prompt {
            prompt,
            input: Vec::new(),
            cursor: 0,
            template: Some(template),
            key: true,
            incremental: false,
        })
    }

    pub fn confirm(prompt: String, cmd: String) -> Self {
        Self::Confirm { prompt, cmd }
    }

    pub fn text(lines: Vec<String>) -> Self {
        Self::Text { lines, top: 0 }
    }

    /// Show `labels` (pane number, pane) for `time`.
    pub fn display_panes(labels: Vec<(u32, PaneId)>, time: Duration) -> Self {
        Self::Panes {
            labels,
            until: Instant::now() + time,
        }
    }

    /// Whether the overlay has timed out and should close by itself.
    pub fn expired(&self) -> bool {
        matches!(self, Self::Panes { until, .. } if *until <= Instant::now())
    }

    pub fn session_picker(srv: &Server, client: ClientId) -> Self {
        let current = srv
            .clients
            .get(&client)
            .and_then(|c| c.att.as_ref())
            .map(|a| a.session);
        let mut sessions: Vec<_> = srv.sessions.values().collect();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_used));
        current_first(&mut sessions, |s| Some(s.id) == current);
        let items = sessions
            .iter()
            .map(|s| {
                let attached = srv
                    .clients
                    .values()
                    .any(|c| c.att.as_ref().is_some_and(|a| a.session == s.id));
                let mark = if Some(s.id) == current { "*" } else { " " };
                Item {
                    label: format!(
                        "{mark} {}: {} windows{}",
                        s.name,
                        s.windows.len(),
                        if attached { " (attached)" } else { "" }
                    ),
                    matches: s.name.clone(),
                    target: Target::Session(s.id, s.name.clone()),
                }
            })
            .collect();
        Self::Picker(Box::new(PickerOverlay::new("sessions", items)))
    }

    /// Every session's windows, the client's session first; `query`
    /// pre-filters them (`find-window`).
    pub fn window_picker(srv: &Server, client: ClientId, query: &str) -> Self {
        let current = srv
            .clients
            .get(&client)
            .and_then(|c| c.att.as_ref())
            .map(|a| a.session);
        let mut items = Vec::new();
        let mut sessions: Vec<_> = srv.sessions.values().collect();
        sessions.sort_by_key(|s| (Some(s.id) != current, s.name.clone()));
        for s in sessions {
            for (idx, wid) in &s.windows {
                let Some(w) = srv.windows.get(wid) else {
                    continue;
                };
                let mark = if Some(s.id) == current && *idx == s.current {
                    "*"
                } else {
                    " "
                };
                items.push(Item {
                    label: format!("{mark} {}:{idx} {}", s.name, w.name),
                    matches: format!("{}:{idx} {}", s.name, w.name),
                    target: Target::Window(s.id, *idx),
                });
            }
        }
        Self::Picker(Box::new(PickerOverlay::with_query("windows", items, query)))
    }

    /// The attached clients; Enter detaches one (`choose-client`).
    pub fn client_picker(srv: &Server) -> Self {
        let items = srv
            .clients
            .values()
            .filter_map(|c| {
                let att = c.att.as_ref()?;
                let session = srv
                    .sessions
                    .get(&att.session)
                    .map_or("", |s| s.name.as_str());
                Some(Item {
                    label: format!("{}: {session} [{}x{}]", c.id, att.cols, att.rows),
                    matches: format!("{} {session}", c.id),
                    target: Target::Client(c.id),
                })
            })
            .collect();
        Self::Picker(Box::new(PickerOverlay::new("clients", items)))
    }

    /// The paste buffers, newest first; Enter pastes one into the pane.
    pub fn buffer_picker(srv: &Server) -> Self {
        let items = srv
            .buffers
            .iter()
            .map(|b| {
                let preview: String = b
                    .data
                    .chars()
                    .take(50)
                    .map(|c| if c.is_control() { ' ' } else { c })
                    .collect();
                Item {
                    label: format!("{}: {} bytes: \"{preview}\"", b.name, b.data.len()),
                    matches: format!("{} {preview}", b.name),
                    target: Target::Buffer(b.name.clone()),
                }
            })
            .collect();
        Self::Picker(Box::new(PickerOverlay::new("buffers", items)))
    }

    /// Feed a key. Returns what the server should do.
    pub fn key(&mut self, ev: &KeyEvent) -> OverlayAction {
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        match self {
            Self::Prompt(p) => p.key(ev),
            Self::Confirm { cmd, .. } => match ev.code {
                KeyCode::Char('y' | 'Y') => OverlayAction::Run(cmd.clone()),
                _ => OverlayAction::Close,
            },
            Self::Text { lines, top } => {
                let max = lines.len().saturating_sub(1);
                match ev.code {
                    KeyCode::Char('q') | KeyCode::Esc => return OverlayAction::Close,
                    KeyCode::Char('c') if ctrl => return OverlayAction::Close,
                    KeyCode::Char('j') | KeyCode::Down => *top = (*top + 1).min(max),
                    KeyCode::Char('k') | KeyCode::Up => *top = top.saturating_sub(1),
                    KeyCode::Char('d') if ctrl => *top = (*top + 10).min(max),
                    KeyCode::Char('u') if ctrl => *top = top.saturating_sub(10),
                    KeyCode::PageDown | KeyCode::Char(' ') => *top = (*top + 20).min(max),
                    KeyCode::PageUp => *top = top.saturating_sub(20),
                    KeyCode::Char('g') | KeyCode::Home => *top = 0,
                    KeyCode::Char('G') | KeyCode::End => *top = max,
                    _ => {}
                }
                OverlayAction::Keep
            }
            Self::Picker(p) => p.key(ev),
            // A pane's number selects it; any other key just dismisses.
            Self::Panes { labels, .. } => {
                let pick = match ev.code {
                    KeyCode::Char(c) => c.to_digit(10).and_then(|d| {
                        labels
                            .iter()
                            .find(|(label, _)| *label == d)
                            .map(|(_, p)| *p)
                    }),
                    _ => None,
                };
                pick.map_or(OverlayAction::Close, |p| {
                    OverlayAction::Run(format!("select-pane -t %{p}"))
                })
            }
        }
    }

    pub fn paste(&mut self, text: &str) {
        if let Self::Prompt(p) = self {
            for c in text.chars().filter(|c| !c.is_control()) {
                p.input.insert(p.cursor, c);
                p.cursor += 1;
            }
        }
    }

    /// Periodic work; returns whether a redraw is needed.
    pub fn tick(&mut self) -> bool {
        match self {
            Self::Picker(p) => {
                p.picker.tick(Instant::now());
                p.picker.refresh()
            }
            _ => false,
        }
    }
}

impl Prompt {
    /// The command the input runs: the template with `%%` (and `%1`)
    /// replaced, or the input itself.
    fn command(&self) -> String {
        let input: String = self.input.iter().collect();
        match &self.template {
            Some(t) => t.replace("%%", &input).replace("%1", &input),
            None => input,
        }
    }

    fn key(&mut self, ev: &KeyEvent) -> OverlayAction {
        let before = self.input.clone();
        let action = self.edit(ev);
        if self.incremental
            && action == OverlayAction::Keep
            && self.input != before
            && !self.input.is_empty()
        {
            return OverlayAction::Preview(self.command());
        }
        action
    }

    fn edit(&mut self, ev: &KeyEvent) -> OverlayAction {
        if self.key {
            let name = join_args(&[tmxr_command::Key::from_event(ev).to_string()]);
            let template = self.template.as_deref().unwrap_or("%%");
            return OverlayAction::Run(template.replace("%%", &name));
        }
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        match ev.code {
            KeyCode::Esc => return OverlayAction::Close,
            KeyCode::Char('c' | 'g') if ctrl => return OverlayAction::Close,
            KeyCode::Enter => {
                if self.input.is_empty() && self.template.is_none() {
                    return OverlayAction::Close;
                }
                return OverlayAction::Run(self.command());
            }
            KeyCode::Backspace | KeyCode::Char('h') if ctrl || ev.code == KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.input.remove(self.cursor);
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.input.len() {
                    self.input.remove(self.cursor);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.input.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.input.len(),
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.input.len(),
            KeyCode::Char('u') if ctrl => {
                self.input.drain(..self.cursor);
                self.cursor = 0;
            }
            KeyCode::Char('k') if ctrl => self.input.truncate(self.cursor),
            KeyCode::Char('w') if ctrl => {
                let mut i = self.cursor;
                while i > 0 && self.input[i - 1] == ' ' {
                    i -= 1;
                }
                while i > 0 && self.input[i - 1] != ' ' {
                    i -= 1;
                }
                self.input.drain(i..self.cursor);
                self.cursor = i;
            }
            KeyCode::Char(c) if !ctrl => {
                self.input.insert(self.cursor, c);
                self.cursor += 1;
            }
            _ => {}
        }
        OverlayAction::Keep
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Session(SessionId, String),
    Window(SessionId, u32),
    /// A paste buffer, by name.
    Buffer(String),
    /// An attached client.
    Client(ClientId),
}

struct Item {
    label: String,
    matches: String,
    target: Target,
}

/// Move the current session to the top of a most-recently-used list. The
/// session picker opens on the second row, so Enter straight away jumps to
/// the previous session (like `switch-client -l`).
fn current_first<T>(sessions: &mut [T], is_current: impl Fn(&T) -> bool) {
    if let Some(pos) = sessions.iter().position(is_current) {
        sessions[..=pos].rotate_right(1);
    }
}

/// The picker's item source: a fixed list built when the picker opens.
struct Source {
    title: &'static str,
    items: Vec<Item>,
}

impl PickerLogic for Source {
    fn title(&self) -> &str {
        self.title
    }

    fn item_count(&self) -> usize {
        self.items.len()
    }

    fn label(&self, idx: usize) -> String {
        self.items[idx].label.clone()
    }

    fn match_text(&self, idx: usize) -> String {
        self.items[idx].matches.clone()
    }

    fn has_preview(&self) -> bool {
        false
    }

    fn select(&self, idx: usize) -> PickerAction {
        PickerAction::Custom(Box::new(self.items[idx].target.clone()))
    }

    fn preserve_source_order(&self) -> bool {
        true
    }

    fn enumerate(
        &mut self,
        _query: Option<&str>,
        _cancel: Arc<AtomicBool>,
    ) -> Option<JoinHandle<()>> {
        None
    }
}

/// The hjkl fuzzy picker, driven the way hjkl drives it
/// (`hjkl_picker_tui::handle_key`): typing filters, arrows and `C-n`/`C-p`
/// move, `Enter` picks, `Escape`/`C-c` close. The session picker adds `C-x`
/// (kill) and `C-r` (rename).
pub struct PickerOverlay {
    pub picker: Picker,
    /// Each row's label and target, to find the highlighted row's target
    /// (the picker owns its source and exposes rows only by label).
    rows: Vec<(String, Target)>,
}

/// What the highlighted picker row would show, for the preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Previewed {
    /// A session's current window.
    Session(SessionId),
    /// A window, by session and index.
    Window(SessionId, u32),
}

impl PickerOverlay {
    fn new(title: &'static str, items: Vec<Item>) -> Self {
        Self::with_query(title, items, "")
    }

    /// A picker that opens already filtered by `query`.
    fn with_query(title: &'static str, items: Vec<Item>, query: &str) -> Self {
        let many = items.len() > 1 && title == "sessions";
        let rows = items
            .iter()
            .map(|i| (i.label.clone(), i.target.clone()))
            .collect();
        let mut picker = Picker::new(Box::new(Source { title, items }));
        if !query.is_empty() {
            // Typing after the query keeps refining it.
            picker.query.set_text(query);
            picker.query.enter_insert_at_end();
            picker.refresh();
        }
        if many {
            picker.selected = 1;
        }
        Self { picker, rows }
    }

    /// The window the highlighted row stands for, if it is a session or a
    /// window.
    pub fn previewed(&self) -> Option<Previewed> {
        let sel = self.picker.selected;
        if sel >= self.picker.matched() {
            return None;
        }
        let label = self.picker.visible_rows(sel..sel + 1).into_iter().next()?.0;
        match self.rows.iter().find(|(l, _)| *l == label)?.1 {
            Target::Session(id, _) => Some(Previewed::Session(id)),
            Target::Window(s, i) => Some(Previewed::Window(s, i)),
            Target::Buffer(_) | Target::Client(_) => None,
        }
    }

    pub fn query(&self) -> String {
        self.picker.query.text()
    }

    fn accept(&mut self) -> OverlayAction {
        // Enter on a name no session matches creates that session.
        let query = self.query();
        if self.is_sessions() && self.picker.matched() == 0 && !query.is_empty() {
            return OverlayAction::Run(join_args(&["new-session".into(), "-s".into(), query]));
        }
        match self.picker.accept() {
            PickerEvent::Select(PickerAction::Custom(any)) => match any.downcast::<Target>() {
                Ok(t) => match *t {
                    Target::Session(s, _) => OverlayAction::Switch(s),
                    Target::Window(s, i) => OverlayAction::SwitchWindow(s, i),
                    Target::Client(id) => OverlayAction::Run(format!("detach-client -t {id}")),
                    Target::Buffer(name) => {
                        OverlayAction::Run(format!("paste-buffer -p -b {}", join_args(&[name])))
                    }
                },
                Err(_) => OverlayAction::Close,
            },
            PickerEvent::Select(PickerAction::None) | PickerEvent::Cancel => OverlayAction::Close,
            PickerEvent::None => OverlayAction::Keep,
        }
    }

    fn is_sessions(&self) -> bool {
        self.picker.title() == "sessions"
    }

    /// The highlighted session, for the session picker's actions.
    fn selected_session(&mut self) -> Option<(SessionId, String)> {
        match self.picker.accept() {
            PickerEvent::Select(PickerAction::Custom(any)) => {
                match *any.downcast::<Target>().ok()? {
                    Target::Session(id, name) => Some((id, name)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// `C-x` / `C-r` in the session picker: kill (after a y/n) or rename the
    /// highlighted session, addressed by its `$id` so any name is safe.
    fn session_action(&mut self, kill: bool) -> OverlayAction {
        let Some((id, name)) = self.selected_session() else {
            return OverlayAction::Keep;
        };
        let target = join_args(&[format!("${id}")]);
        OverlayAction::Run(if kill {
            join_args(&[
                "confirm-before".into(),
                "-p".into(),
                format!("kill-session {name}? (y/n)"),
                format!("kill-session -t {target}"),
            ])
        } else {
            join_args(&[
                "command-prompt".into(),
                "-I".into(),
                name,
                "-p".into(),
                "(rename-session)".into(),
                format!("rename-session -t {target} -- '%%'"),
            ])
        })
    }

    fn key(&mut self, ev: &KeyEvent) -> OverlayAction {
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        match ev.code {
            KeyCode::Enter => return self.accept(),
            KeyCode::Char('x') if ctrl && self.is_sessions() => return self.session_action(true),
            KeyCode::Char('r') if ctrl && self.is_sessions() => return self.session_action(false),
            _ => {}
        }
        if let PickerEvent::Select(_) | PickerEvent::Cancel =
            hjkl_picker_tui::handle_key(&mut self.picker, *ev)
        {
            return OverlayAction::Close;
        }
        self.picker.refresh();
        OverlayAction::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn items(names: &[&str]) -> Vec<Item> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| Item {
                label: (*n).to_owned(),
                matches: (*n).to_owned(),
                target: Target::Session(i as u32, (*n).to_owned()),
            })
            .collect()
    }

    #[test]
    fn prompt_edits_and_substitutes_the_template() {
        let mut o = Overlay::prompt(":".into(), "ab".into(), Some("rename-window -- %%".into()));
        o.key(&k(KeyCode::Backspace));
        o.key(&k(KeyCode::Char('x')));
        o.key(&k(KeyCode::Home));
        o.key(&k(KeyCode::Char('>')));
        assert_eq!(
            o.key(&k(KeyCode::Enter)),
            OverlayAction::Run("rename-window -- >ax".into())
        );
        let mut o = Overlay::prompt(":".into(), String::new(), None);
        assert_eq!(o.key(&k(KeyCode::Esc)), OverlayAction::Close);
    }

    #[test]
    fn confirm_runs_only_on_y() {
        let mut o = Overlay::confirm("kill?".into(), "kill-pane".into());
        assert_eq!(
            o.key(&k(KeyCode::Char('y'))),
            OverlayAction::Run("kill-pane".into())
        );
        let mut o = Overlay::confirm("kill?".into(), "kill-pane".into());
        assert_eq!(o.key(&k(KeyCode::Char('n'))), OverlayAction::Close);
    }

    #[test]
    fn session_picker_opens_on_the_previous_session() {
        // MRU order: the current session was used last, "work" before it.
        let mut mru = vec!["main", "work", "dots"];
        current_first(&mut mru, |s| *s == "main");
        assert_eq!(mru, ["main", "work", "dots"]);
        let o = PickerOverlay::new("sessions", items(&mru));
        assert_eq!(mru[o.picker.selected], "work");
        // The current session is not always the most recent (another client
        // switched since); it still goes first and keeps the rest in order.
        let mut mru = vec!["work", "dots", "main"];
        current_first(&mut mru, |s| *s == "main");
        assert_eq!(mru, ["main", "work", "dots"]);
    }

    #[test]
    fn picker_filters_moves_with_jk_and_selects() {
        let mut o = Overlay::Picker(Box::new(PickerOverlay::new(
            "sessions",
            items(&["main", "work", "dots"]),
        )));
        // Sessions open on the second entry (the previous session).
        let Overlay::Picker(p) = &o else {
            unreachable!()
        };
        assert_eq!(p.picker.selected, 1);
        // Typing filters.
        o.key(&k(KeyCode::Char('d')));
        o.key(&k(KeyCode::Char('o')));
        let Overlay::Picker(p) = &o else {
            unreachable!()
        };
        assert_eq!(p.picker.matched(), 1);
        assert_eq!(o.key(&k(KeyCode::Enter)), OverlayAction::Switch(2));

        // Arrows and C-n / C-p move, Enter picks; j and k are typed.
        let mut o = Overlay::Picker(Box::new(PickerOverlay::new(
            "sessions",
            items(&["main", "work", "dots"]),
        )));
        o.key(&k(KeyCode::Down));
        assert_eq!(o.key(&k(KeyCode::Enter)), OverlayAction::Switch(2));
        let mut o = Overlay::Picker(Box::new(PickerOverlay::new(
            "sessions",
            items(&["main", "work", "dots"]),
        )));
        let ctrl_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        o.key(&ctrl_p);
        o.key(&ctrl_p);
        assert_eq!(
            o.key(&k(KeyCode::Enter)),
            OverlayAction::Switch(2),
            "C-p wraps"
        );
        // Escape closes at once.
        let mut o = Overlay::Picker(Box::new(PickerOverlay::new(
            "sessions",
            items(&["main", "work", "dots"]),
        )));
        o.key(&k(KeyCode::Char('j')));
        let Overlay::Picker(p) = &o else {
            unreachable!()
        };
        assert_eq!(p.picker.query.text(), "j");
        assert_eq!(o.key(&k(KeyCode::Esc)), OverlayAction::Close);
    }
}
