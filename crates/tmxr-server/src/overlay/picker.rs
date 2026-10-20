//! The fuzzy picker behind the session, window, client and buffer pickers,
//! `find-window` and `customize-mode`.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread::JoinHandle;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use hjkl_picker::{Picker, PickerAction, PickerEvent, PickerLogic};

use super::OverlayAction;
use crate::cmds::join_args;
use crate::model::{ClientId, SessionId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Target {
    Session(SessionId, String),
    /// A window, by session, index and the session's name.
    Window(SessionId, u32, String),
    /// A paste buffer, by name.
    Buffer(String),
    /// An attached client.
    Client(ClientId),
    /// An option or bind, by the command line that sets it.
    Edit(String),
}

pub(super) struct Item {
    pub(super) label: String,
    pub(super) matches: String,
    pub(super) target: Target,
}

/// Move the current session to the top of a most-recently-used list. The
/// session picker opens on the second row, so Enter straight away jumps to
/// the previous session (like `switch-client -l`).
pub(super) fn current_first<T>(sessions: &mut [T], is_current: impl Fn(&T) -> bool) {
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

/// The hjkl fuzzy picker, with a normal mode in front of hjkl's own keys.
/// It opens in normal mode: `j`/`k`, arrows and `C-n`/`C-p` move, `Enter`
/// picks, `q`/`Escape`/`C-c` close, and `i`, `I`, `a`, `A` or `/` start
/// typing. Typing is driven the way hjkl drives it
/// (`hjkl_picker_tui::handle_key`), except that `Escape` goes back to normal
/// mode. The session picker adds `C-x` (kill) and `C-r` (rename) in both.
pub struct PickerOverlay {
    pub picker: Picker,
    /// Whether keys go to the query (typing) or move the selection (normal).
    typing: bool,
    /// Each row's label and target, to find the highlighted row's target
    /// (the picker owns its source and exposes rows only by label).
    rows: Vec<(String, Target)>,
    /// A command for the chosen item, its `%%` the item's target, as the
    /// template `choose-tree`, `choose-client` and `choose-buffer` take.
    pub template: Option<String>,
    /// `-N`: no preview under the list.
    pub no_preview: bool,
    /// `-y`: a kill goes without asking first.
    pub no_confirm: bool,
    /// Each row's key (`-K`) and what it chooses.
    pub(super) keys: Vec<(String, Target)>,
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
    pub(super) fn new(title: &'static str, items: Vec<Item>) -> Self {
        Self::with_query(title, items, "")
    }

    /// A picker that opens already filtered by `query`.
    pub(super) fn with_query(title: &'static str, items: Vec<Item>, query: &str) -> Self {
        let many = items.len() > 1 && title == "sessions";
        let rows = items
            .iter()
            .map(|i| (i.label.clone(), i.target.clone()))
            .collect();
        let mut picker = Picker::new(Box::new(Source { title, items }));
        if !query.is_empty() {
            picker.query.set_text(query);
            picker.query.enter_insert_at_end();
            picker.refresh();
        }
        if many {
            picker.selected = 1;
        }
        Self {
            picker,
            rows,
            typing: false,
            template: None,
            no_preview: false,
            no_confirm: false,
            keys: Vec::new(),
        }
    }

    /// Highlight row `row`, when there is one.
    pub(super) fn select(&mut self, row: Option<usize>) {
        if let Some(row) = row {
            self.picker.selected = row;
        }
    }

    /// Whether keys go to the query rather than move the selection.
    pub const fn typing(&self) -> bool {
        self.typing
    }

    /// The window the highlighted row stands for, if it is a session or a
    /// window.
    pub fn previewed(&self) -> Option<Previewed> {
        if self.no_preview {
            return None;
        }
        let sel = self.picker.selected;
        if sel >= self.picker.matched() {
            return None;
        }
        let label = self.picker.visible_rows(sel..sel + 1).into_iter().next()?.0;
        match self.rows.iter().find(|(l, _)| *l == label)?.1 {
            Target::Session(id, _) => Some(Previewed::Session(id)),
            Target::Window(s, i, _) => Some(Previewed::Window(s, i)),
            Target::Buffer(_) | Target::Client(_) | Target::Edit(_) => None,
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
                Ok(t) => self.choose_target(*t),
                Err(_) => OverlayAction::Close,
            },
            PickerEvent::Select(PickerAction::None) | PickerEvent::Cancel => OverlayAction::Close,
            PickerEvent::None => OverlayAction::Keep,
        }
    }

    /// What choosing `t` does: the template run with its target, as tmux's
    /// window_tree_get_target writes it, or the picker's own action.
    fn choose_target(&self, t: Target) -> OverlayAction {
        if let Some(template) = self.template.as_deref()
            && !matches!(t, Target::Edit(_))
        {
            let target = match t {
                Target::Session(_, name) => format!("={name}:"),
                Target::Window(_, i, name) => format!("={name}:{i}."),
                Target::Client(id) => id.to_string(),
                Target::Buffer(name) | Target::Edit(name) => name,
            };
            return OverlayAction::Run(tmxr_command::fill_template(template, &[target]));
        }
        match t {
            Target::Session(s, _) => OverlayAction::Switch(s),
            Target::Window(s, i, _) => OverlayAction::SwitchWindow(s, i),
            Target::Client(id) => OverlayAction::Run(format!("detach-client -t {id}")),
            Target::Buffer(name) => {
                OverlayAction::Run(format!("paste-buffer -p -b {}", join_args(&[name])))
            }
            Target::Edit(line) => OverlayAction::Edit(line),
        }
    }

    fn is_sessions(&self) -> bool {
        self.picker.title() == "sessions"
    }

    /// The highlighted row's target.
    fn selected_target(&mut self) -> Option<Target> {
        match self.picker.accept() {
            PickerEvent::Select(PickerAction::Custom(any)) => {
                any.downcast::<Target>().ok().map(|t| *t)
            }
            _ => None,
        }
    }

    /// The highlighted session, for the session picker's actions.
    fn selected_session(&mut self) -> Option<(SessionId, String)> {
        match self.selected_target()? {
            Target::Session(id, name) => Some((id, name)),
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
        OverlayAction::Run(if kill && self.no_confirm {
            format!("kill-session -t {target}")
        } else if kill {
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

    pub(super) fn key(&mut self, ev: &KeyEvent) -> OverlayAction {
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        match ev.code {
            KeyCode::Enter => return self.accept(),
            KeyCode::Char('x') if ctrl && self.is_sessions() => return self.session_action(true),
            KeyCode::Char('r') if ctrl && self.is_sessions() => return self.session_action(false),
            KeyCode::Esc if self.typing => {
                self.typing = false;
                return OverlayAction::Keep;
            }
            _ => {}
        }
        if !self.typing {
            return self.normal_key(ev, ctrl);
        }
        if let PickerEvent::Select(_) | PickerEvent::Cancel =
            hjkl_picker_tui::handle_key(&mut self.picker, *ev)
        {
            return OverlayAction::Close;
        }
        self.picker.refresh();
        OverlayAction::Keep
    }

    fn normal_key(&mut self, ev: &KeyEvent, ctrl: bool) -> OverlayAction {
        // A row's key chooses it, as tmux's mode tree.
        let pressed = tmxr_command::Key::from_event(ev).to_string();
        if let Some((_, target)) = self.keys.iter().find(|(k, _)| *k == pressed) {
            return self.choose_target(target.clone());
        }
        match ev.code {
            KeyCode::Char('j') | KeyCode::Down => self.picker.select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.picker.select_prev(),
            KeyCode::Char('n') if ctrl => self.picker.select_next(),
            KeyCode::Char('p') if ctrl => self.picker.select_prev(),
            KeyCode::Char('c') if ctrl => return OverlayAction::Close,
            KeyCode::Char('q') | KeyCode::Esc => return OverlayAction::Close,
            // tmux's choose-buffer: the buffer in `editor`.
            KeyCode::Char('e') if self.picker.title() == "buffers" => {
                if let Some(Target::Buffer(name)) = self.selected_target() {
                    return OverlayAction::EditBuffer(name);
                }
            }
            KeyCode::Char('i' | 'I' | 'a' | 'A' | '/') => {
                self.typing = true;
                self.picker.query.enter_insert_at_end();
            }
            _ => {}
        }
        OverlayAction::Keep
    }
}
