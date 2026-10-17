//! The command prompt's line editor: the input, its cursor, the template it
//! fills, and history recall.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::OverlayAction;
use crate::cmds::join_args;

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
    /// The prompt type (`command-prompt -T`) its history is kept under.
    pub kind: String,
    /// Earlier entries of this type, oldest first, for Up / Down.
    pub history: Vec<String>,
    /// Which entry Up / Down show, and the input typed before the first Up.
    pub(super) recall: Option<(usize, Vec<char>)>,
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

    pub(super) fn key(&mut self, ev: &KeyEvent) -> OverlayAction {
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

    /// Up (`older`) / Down through the history; Down past the newest entry
    /// brings back what was typed.
    fn recall(&mut self, older: bool) {
        let n = self.history.len();
        let next = match (&self.recall, older) {
            (None, true) if n > 0 => Some(n - 1),
            (Some((i, _)), true) => Some(i.saturating_sub(1)),
            (Some((i, _)), false) if i + 1 < n => Some(i + 1),
            _ => None,
        };
        match next {
            Some(i) => {
                let draft = match self.recall.take() {
                    Some((_, draft)) => draft,
                    None => self.input.clone(),
                };
                self.input = self.history[i].chars().collect();
                self.recall = Some((i, draft));
            }
            None if !older => {
                if let Some((_, draft)) = self.recall.take() {
                    self.input = draft;
                }
            }
            None => return,
        }
        self.cursor = self.input.len();
    }

    /// The text typed, for the history once the prompt is submitted.
    pub fn text(&self) -> String {
        self.input.iter().collect()
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
            KeyCode::Up => self.recall(true),
            KeyCode::Down => self.recall(false),
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
