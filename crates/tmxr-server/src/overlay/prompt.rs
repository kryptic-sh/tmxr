//! The command prompt's line editor: the input, its cursor, the template it
//! fills, and history recall.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::OverlayAction;
use crate::cmds::join_args;

pub struct Prompt {
    pub prompt: String,
    pub input: Vec<char>,
    pub cursor: usize,
    /// The answers fill the template's `%%` and `%1` … `%9` as tmux's do;
    /// no template runs the input itself.
    pub template: Option<String>,
    /// The prompts still to come after this one (`command-prompt -p a,b`),
    /// each with its starting input.
    pub pending: std::collections::VecDeque<(String, String)>,
    /// The answers to the prompts already done.
    pub answers: Vec<String>,
    /// `command-prompt -k`: the next key pressed is the input, as its tmux
    /// name, quoted so any key stays one argument.
    pub key: bool,
    /// `command-prompt -i`: the template runs again after every edit, for
    /// incremental search.
    pub incremental: bool,
    /// `command-prompt -1`: the first character typed is the answer.
    pub single: bool,
    /// `command-prompt -N`: digits are typed, any other key answers.
    pub numeric: bool,
    /// The prompt type (`command-prompt -T`) its history is kept under.
    pub kind: String,
    /// Earlier entries of this type, oldest first, for Up / Down.
    pub history: Vec<String>,
    /// Which entry Up / Down show, and the input typed before the first Up.
    pub(super) recall: Option<(usize, Vec<char>)>,
    /// `status-keys vi`, read before each key.
    pub vi: bool,
    /// In vi's command mode (Escape), not typing.
    pub command_mode: bool,
    /// `word-separators`, read before each key, for the word moves.
    pub separators: String,
    /// The newest paste buffer, read before each key, for `C-y`.
    pub paste: Option<String>,
    /// What the last `C-w` deleted, which `C-y` puts back first.
    pub(super) saved: Option<Vec<char>>,
    /// After `C-v`: the next key is typed as it is.
    pub(super) quote_next: bool,
}

/// What a key does to the input, as tmux 3.6's status_prompt_key does it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edit {
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    /// `C-k`: from the cursor to the end.
    KillToEnd,
    /// `C-u`: the whole input.
    Clear,
    /// `C-w`: the word before the cursor, kept for `C-y`.
    DeleteWord,
    /// A word on: vi's (to the next word's start) or emacs's (to this one's
    /// end), by `word-separators` or by spaces alone.
    ForwardWord {
        vi: bool,
        separators: bool,
    },
    /// vi's `e` / `E`: to the end of the word.
    EndWord {
        separators: bool,
    },
    BackwardWord {
        separators: bool,
    },
    Older,
    Newer,
    Paste,
    Transpose,
    Submit,
    Cancel,
    QuoteNext,
    Type(char),
    Ignore,
}

/// tmux's emacs keys (status_prompt_key).
fn emacs(ev: &KeyEvent) -> Edit {
    let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
    let alt = ev.modifiers.contains(KeyModifiers::ALT);
    match ev.code {
        KeyCode::Left if ctrl => Edit::BackwardWord { separators: true },
        KeyCode::Right if ctrl => Edit::ForwardWord {
            vi: false,
            separators: true,
        },
        KeyCode::Left => Edit::Left,
        KeyCode::Right => Edit::Right,
        KeyCode::Home => Edit::Home,
        KeyCode::End => Edit::End,
        KeyCode::Backspace => Edit::Backspace,
        KeyCode::Delete => Edit::Delete,
        KeyCode::Up => Edit::Older,
        KeyCode::Down => Edit::Newer,
        KeyCode::Enter => Edit::Submit,
        KeyCode::Esc => Edit::Cancel,
        KeyCode::Char(c) if ctrl => match c {
            'b' => Edit::Left,
            'f' => Edit::Right,
            'a' => Edit::Home,
            'e' => Edit::End,
            'h' => Edit::Backspace,
            'd' => Edit::Delete,
            'u' => Edit::Clear,
            'k' => Edit::KillToEnd,
            'w' => Edit::DeleteWord,
            'p' => Edit::Older,
            'n' => Edit::Newer,
            'y' => Edit::Paste,
            't' => Edit::Transpose,
            'c' | 'g' => Edit::Cancel,
            'j' | 'm' => Edit::Submit,
            'v' => Edit::QuoteNext,
            _ => Edit::Ignore,
        },
        KeyCode::Char('f') if alt => Edit::ForwardWord {
            vi: false,
            separators: true,
        },
        KeyCode::Char('b') if alt => Edit::BackwardWord { separators: true },
        KeyCode::Char(c) if !alt => Edit::Type(c),
        _ => Edit::Ignore,
    }
}

/// The keys typing passes on as tmux's emacs keys in vi's insert mode
/// (status_prompt_translate_key); others only type a character.
fn vi_insert_passes(ev: &KeyEvent) -> bool {
    let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
    match ev.code {
        KeyCode::Char(c) if ctrl => "aceghknptuvwyjm".contains(c),
        KeyCode::Left
        | KeyCode::Right
        | KeyCode::Up
        | KeyCode::Down
        | KeyCode::Home
        | KeyCode::End
        | KeyCode::Backspace
        | KeyCode::Delete
        | KeyCode::Enter
        | KeyCode::Tab => true,
        _ => false,
    }
}

/// A key in vi's command mode, as tmux's status_prompt_translate_key: the
/// edit it stands for, and whether it goes back to typing.
fn vi_command(ev: &KeyEvent) -> (Edit, bool) {
    let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
    let words = |separators| Edit::ForwardWord {
        vi: true,
        separators,
    };
    match ev.code {
        KeyCode::Char('h') if ctrl => (Edit::Backspace, false),
        KeyCode::Char('c') if ctrl => (Edit::Cancel, false),
        KeyCode::Char(_) if ctrl => (Edit::Ignore, false),
        KeyCode::Enter => (Edit::Submit, false),
        KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => (Edit::Left, false),
        KeyCode::Right | KeyCode::Char('l') => (Edit::Right, false),
        KeyCode::Up | KeyCode::Char('k') => (Edit::Older, false),
        KeyCode::Down | KeyCode::Char('j') => (Edit::Newer, false),
        KeyCode::Delete | KeyCode::Char('x') => (Edit::Delete, false),
        KeyCode::Esc | KeyCode::Char('i') => (Edit::Ignore, true),
        KeyCode::Char(c) => match c {
            'A' => (Edit::End, true),
            'I' => (Edit::Home, true),
            'C' => (Edit::KillToEnd, true),
            's' => (Edit::Delete, true),
            'a' => (Edit::Right, true),
            'S' => (Edit::Clear, true),
            '$' => (Edit::End, false),
            '0' | '^' => (Edit::Home, false),
            'D' => (Edit::KillToEnd, false),
            'X' => (Edit::Backspace, false),
            'b' => (Edit::BackwardWord { separators: true }, false),
            'B' => (Edit::BackwardWord { separators: false }, false),
            'd' => (Edit::Clear, false),
            'e' => (Edit::EndWord { separators: true }, false),
            'E' => (Edit::EndWord { separators: false }, false),
            'w' => (words(true), false),
            'W' => (words(false), false),
            'p' => (Edit::Paste, false),
            'q' => (Edit::Cancel, false),
            _ => (Edit::Ignore, false),
        },
        _ => (Edit::Ignore, false),
    }
}

fn space(c: char) -> bool {
    c == ' '
}

impl Prompt {
    /// The command the answers run: the template with them put in, or the
    /// input itself.
    fn command(&self) -> String {
        let input: String = self.input.iter().collect();
        match &self.template {
            Some(t) => {
                let mut answers = self.answers.clone();
                answers.push(input);
                tmxr_command::fill_template(t, &answers)
            }
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

    /// The input answered: the next prompt, if any, takes over with its own
    /// input; after the last, the template runs.
    fn submit(&mut self) -> OverlayAction {
        if let Some((prompt, input)) = self.pending.pop_front() {
            self.answers.push(self.text());
            self.prompt = prompt;
            self.input = input.chars().collect();
            self.cursor = self.input.len();
            self.recall = None;
            return OverlayAction::Keep;
        }
        if self.input.is_empty() && self.template.is_none() {
            return OverlayAction::Close;
        }
        OverlayAction::Run(self.command())
    }

    /// The text typed, for the history once the prompt is submitted.
    pub fn text(&self) -> String {
        self.input.iter().collect()
    }

    fn edit(&mut self, ev: &KeyEvent) -> OverlayAction {
        if self.key {
            let name = join_args(&[tmxr_command::Key::from_event(ev).to_string()]);
            let template = self.template.as_deref().unwrap_or("%%");
            return OverlayAction::Run(tmxr_command::fill_template(template, &[name]));
        }
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        // -N: a digit is typed; anything else answers with what is there.
        if self.numeric {
            return match ev.code {
                KeyCode::Char(c @ '0'..='9') if !ctrl => {
                    self.input.insert(self.cursor, c);
                    self.cursor += 1;
                    OverlayAction::Keep
                }
                _ => self.submit(),
            };
        }
        // -1, or after C-v: the character typed (a control one with Ctrl)
        // goes in as it is; with -1 it is the whole answer, and with
        // something there already the prompt goes, as tmux's.
        if self.single || self.quote_next {
            let typed = match ev.code {
                KeyCode::Char(c) if ctrl && c.is_ascii() => char::from(c as u8 & 0x1f),
                KeyCode::Char(c) => c,
                KeyCode::Backspace => '\x7f',
                _ => return OverlayAction::Keep,
            };
            self.quote_next = false;
            self.input.insert(self.cursor, typed);
            self.cursor += 1;
            if !self.single {
                return OverlayAction::Keep;
            }
            return if self.input.len() == 1 {
                self.submit()
            } else {
                OverlayAction::Close
            };
        }
        let edit = if !self.vi {
            emacs(ev)
        } else if self.command_mode {
            let (edit, typing) = vi_command(ev);
            if typing {
                self.command_mode = false;
            }
            edit
        } else if ev.code == KeyCode::Esc {
            self.command_mode = true;
            Edit::Ignore
        } else if vi_insert_passes(ev) {
            emacs(ev)
        } else {
            match ev.code {
                KeyCode::Char(c) if !ctrl && !ev.modifiers.contains(KeyModifiers::ALT) => {
                    Edit::Type(c)
                }
                _ => Edit::Ignore,
            }
        };
        self.apply(edit)
    }

    /// Do `edit` to the input, as tmux's status_prompt_key.
    fn apply(&mut self, edit: Edit) -> OverlayAction {
        let len = self.input.len();
        match edit {
            Edit::Submit => return self.submit(),
            Edit::Cancel => return OverlayAction::Close,
            Edit::Left => self.cursor = self.cursor.saturating_sub(1),
            Edit::Right => self.cursor = (self.cursor + 1).min(len),
            Edit::Home => self.cursor = 0,
            Edit::End => self.cursor = len,
            Edit::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.input.remove(self.cursor);
                }
            }
            Edit::Delete => {
                if self.cursor < len {
                    self.input.remove(self.cursor);
                }
            }
            Edit::KillToEnd => self.input.truncate(self.cursor),
            Edit::Clear => {
                self.input.clear();
                self.cursor = 0;
            }
            Edit::DeleteWord => {
                let start = self.backward_word(&self.separators.clone());
                self.saved = Some(self.input.drain(start..self.cursor).collect());
                self.cursor = start;
            }
            Edit::ForwardWord { vi, separators } => {
                let seps = if separators {
                    self.separators.clone()
                } else {
                    String::new()
                };
                self.cursor = self.forward_word(vi, &seps);
            }
            Edit::EndWord { separators } => {
                let seps = if separators {
                    self.separators.clone()
                } else {
                    String::new()
                };
                self.cursor = self.end_word(&seps);
            }
            Edit::BackwardWord { separators } => {
                let seps = if separators {
                    self.separators.clone()
                } else {
                    String::new()
                };
                self.cursor = self.backward_word(&seps);
            }
            Edit::Older => self.recall(true),
            Edit::Newer => self.recall(false),
            Edit::Paste => {
                // The last C-w first, else the newest buffer up to its
                // first control character.
                let text: Vec<char> = match &self.saved {
                    Some(saved) => saved.clone(),
                    None => self
                        .paste
                        .as_deref()
                        .unwrap_or_default()
                        .chars()
                        .take_while(|c| !c.is_control())
                        .collect(),
                };
                let n = text.len();
                self.input.splice(self.cursor..self.cursor, text);
                self.cursor += n;
            }
            Edit::Transpose => {
                let at = if self.cursor < len {
                    self.cursor + 1
                } else {
                    self.cursor
                };
                if at >= 2 {
                    self.input.swap(at - 2, at - 1);
                    self.cursor = at;
                }
            }
            Edit::QuoteNext => self.quote_next = true,
            Edit::Type(c) => {
                self.input.insert(self.cursor, c);
                self.cursor += 1;
            }
            Edit::Ignore => {}
        }
        OverlayAction::Keep
    }

    fn in_list(separators: &str, c: char) -> bool {
        c.is_ascii() && separators.contains(c)
    }

    /// tmux's status_prompt_forward_word: where a word move forward lands.
    fn forward_word(&self, vi: bool, separators: &str) -> usize {
        let (input, size) = (&self.input, self.input.len());
        let mut idx = self.cursor;
        // In emacs mode, skip until the first non-whitespace character.
        if !vi {
            while idx != size && space(input[idx]) {
                idx += 1;
            }
        }
        if idx == size {
            return idx;
        }
        let word_is_separators = Self::in_list(separators, input[idx]) && !space(input[idx]);
        // Skip ahead until the first space or opposite character class.
        loop {
            idx += 1;
            if idx < size && space(input[idx]) {
                // In vi mode, go to the start of the next word.
                if vi {
                    while idx != size && space(input[idx]) {
                        idx += 1;
                    }
                }
                break;
            }
            if idx == size || word_is_separators != Self::in_list(separators, input[idx]) {
                break;
            }
        }
        idx
    }

    /// tmux's status_prompt_end_word: the end of this word or the next.
    fn end_word(&self, separators: &str) -> usize {
        let (input, size) = (&self.input, self.input.len());
        let mut idx = self.cursor;
        if idx == size {
            return idx;
        }
        // Find the next word.
        loop {
            idx += 1;
            if idx == size {
                return idx;
            }
            if !space(input[idx]) {
                break;
            }
        }
        let word_is_separators = Self::in_list(separators, input[idx]);
        // Skip ahead until the next space or opposite character class.
        loop {
            idx += 1;
            if idx == size
                || space(input[idx])
                || word_is_separators != Self::in_list(separators, input[idx])
            {
                break;
            }
        }
        // Back up to the previous character, the end of the word.
        idx - 1
    }

    /// tmux's status_prompt_backward_word: the start of this word or the
    /// one before.
    fn backward_word(&self, separators: &str) -> usize {
        let input = &self.input;
        let mut idx = self.cursor;
        // Find non-whitespace.
        while idx != 0 {
            idx -= 1;
            if !space(input[idx]) {
                break;
            }
        }
        let word_is_separators = input
            .get(idx)
            .is_some_and(|&c| Self::in_list(separators, c));
        // Find the character before the beginning of the word.
        while idx != 0 {
            idx -= 1;
            if space(input[idx]) || word_is_separators != Self::in_list(separators, input[idx]) {
                // Go back to the word.
                idx += 1;
                break;
            }
        }
        idx
    }
}
