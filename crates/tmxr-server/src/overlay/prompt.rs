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
    /// incremental search, its input prefixed as tmux's: `=`, or `-` / `+`
    /// after `C-r` / `C-s`. Enter runs nothing more.
    pub incremental: bool,
    /// An incremental prompt's starting input (`-I`), which opens empty;
    /// `C-r` / `C-s` on an empty input bring it back, as tmux's prompt_last.
    pub last: String,
    /// Answered with Enter, for the history.
    pub submitted: bool,
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
    /// What Tab completes from, given before a Tab is handled.
    pub completions: Option<Completions>,
    /// The menu Tab opened of the names the word could be, as tmux's
    /// status_prompt_complete_list_menu.
    pub menu: Option<CompletionMenu>,
}

/// A menu of completions and the word it replaces (start and end).
#[derive(Debug, Clone)]
pub struct CompletionMenu {
    pub menu: crate::menu::Menu,
    pub word: (usize, usize),
}

impl CompletionMenu {
    /// A menu of `names`, the first selected, each with its digit as its
    /// key while there are digits.
    fn new(names: &[&str], word: (usize, usize)) -> Self {
        let items = names
            .iter()
            .enumerate()
            .map(|(i, name)| crate::menu::MenuItem::Command {
                name: (*name).to_owned(),
                key: u32::try_from(i)
                    .ok()
                    .and_then(|i| char::from_digit(i, 10))
                    .and_then(|d| d.to_string().parse().ok()),
                cmd: (*name).to_owned(),
                disabled: false,
            })
            .collect();
        let menu = crate::menu::Menu {
            title: String::new(),
            items,
            selected: Some(0),
            look: super::BoxLook::default(),
            at: None,
            pane: None,
            mouse: None,
            mouse_mode: false,
            stay_open: false,
        };
        Self { menu, word }
    }
}

/// The names the prompt's Tab completes from, as tmux's
/// status_prompt_complete_list and status_prompt_complete_session.
#[derive(Debug, Clone, Default)]
pub struct Completions {
    /// Command names and aliases, completed anywhere.
    pub commands: Vec<String>,
    /// Option names and layouts, completed after the first word.
    pub others: Vec<String>,
    /// Sessions, by name and id, for a `-t` or `-s` word.
    pub sessions: Vec<(String, u32)>,
}

/// The longest start every one of `words` shares, as tmux's
/// status_prompt_complete_prefix.
fn common_prefix<S: AsRef<str>>(words: &[S]) -> Option<String> {
    let (first, rest) = words.split_first()?;
    let mut prefix: Vec<char> = first.as_ref().chars().collect();
    for w in rest {
        let shared = prefix
            .iter()
            .zip(w.as_ref().chars())
            .take_while(|(a, b)| **a == *b)
            .count();
        prefix.truncate(shared);
    }
    Some(prefix.into_iter().collect())
}

/// The sessions `s` starts the name (or after `$`, the id) of, as
/// `name:`, and what they share, as tmux's status_prompt_complete_session.
/// None after a colon, where tmux opens a menu of the session's windows.
fn session_completion(s: &str, names: &Completions) -> Option<String> {
    if s.contains(':') {
        return None;
    }
    let list: Vec<String> = names
        .sessions
        .iter()
        .filter_map(|(name, id)| {
            if s.is_empty() || name.starts_with(s) {
                Some(format!("{name}:"))
            } else {
                let id = id.to_string();
                let n = s.strip_prefix('$')?;
                id.starts_with(n).then(|| format!("${id}:"))
            }
        })
        .collect();
    common_prefix(&list)
}

/// What tmux's status_prompt_complete gives for `word`: one name it starts
/// (with a space after) or what all of them share; for a `-t` or `-s` word,
/// or any word of a `target` prompt (`command-prompt -T target`), the
/// sessions. `None` when that is no further on (tmux then opens a menu of
/// them, which tmxr does not) or there is nothing.
fn completion(word: &str, at_start: bool, target: bool, names: &Completions) -> Option<String> {
    if word.is_empty() && !target {
        return None;
    }
    let out = if target {
        session_completion(word, names)
    } else if let Some(s) = word.strip_prefix("-t").or_else(|| word.strip_prefix("-s")) {
        let flag = word.chars().nth(1)?;
        session_completion(s, names).map(|p| format!("-{flag}{p}"))
    } else {
        let list = word_matches(word, at_start, names);
        match list.as_slice() {
            [] => None,
            [one] => Some(format!("{one} ")),
            _ => common_prefix(&list),
        }
    };
    out.filter(|o| o != word)
}

/// The names `word` starts, sorted: commands, and after the first word
/// options and layouts too, as tmux's status_prompt_complete_list.
fn word_matches<'n>(word: &str, at_start: bool, names: &'n Completions) -> Vec<&'n str> {
    let mut list: Vec<&str> = names
        .commands
        .iter()
        .chain(names.others.iter().filter(|_| !at_start))
        .map(String::as_str)
        .filter(|n| n.starts_with(word))
        .collect();
    list.sort_unstable();
    list.dedup();
    list
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
    /// Tab: the word at the cursor completed.
    Complete,
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
        KeyCode::Tab => Edit::Complete,
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

    /// The template filled with the input behind `prefix`, as an
    /// incremental prompt runs it.
    pub fn incremental_command(&self, prefix: char) -> String {
        let input: String = self.input.iter().collect();
        let template = self.template.as_deref().unwrap_or("%%");
        tmxr_command::fill_template(template, &[format!("{prefix}{input}")])
    }

    pub(super) fn key(&mut self, ev: &KeyEvent) -> OverlayAction {
        if self.menu.is_some() {
            return self.menu_key(ev);
        }
        if !self.incremental || self.key {
            return self.edit(ev);
        }
        // C-r / C-s search the other way, or on an empty input bring back
        // the starting one (emacs keys only: vi's insert mode types them).
        let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        if let (KeyCode::Char(c @ ('r' | 's')), true, false) = (ev.code, ctrl, self.vi) {
            let prefix = if self.input.is_empty() {
                self.input = self.last.chars().collect();
                self.cursor = self.input.len();
                '='
            } else if c == 'r' {
                '-'
            } else {
                '+'
            };
            return OverlayAction::Preview(self.incremental_command(prefix));
        }
        let before = self.input.clone();
        match self.edit(ev) {
            OverlayAction::Run(_) => OverlayAction::Close,
            OverlayAction::Keep if self.input != before => {
                OverlayAction::Preview(self.incremental_command('='))
            }
            action => action,
        }
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
        self.submitted = true;
        if self.input.is_empty() && self.template.is_none() {
            return OverlayAction::Close;
        }
        OverlayAction::Run(self.command())
    }

    /// Tab: the word at the cursor replaced by its completion, found as
    /// tmux's status_prompt_replace_complete finds it (the word holding the
    /// character before the cursor), the cursor after it.
    fn complete(&mut self) {
        let Some(names) = self.completions.take() else {
            return;
        };
        let space = |i: usize| self.input.get(i) == Some(&' ');
        let filled = |i: usize| i < self.input.len();
        let at = self.cursor.saturating_sub(1);
        let mut first = at;
        while first > 0 && !space(first) {
            first -= 1;
        }
        while filled(first) && space(first) {
            first += 1;
        }
        let mut last = at;
        while filled(last) && !space(last) {
            last += 1;
        }
        while last > 0 && space(last) {
            last -= 1;
        }
        if filled(last) {
            last += 1;
        }
        if last < first {
            return;
        }
        let word: String = self.input[first..last].iter().collect();
        let target = self.kind == "target";
        if let Some(out) = completion(&word, first == 0, target, &names) {
            self.input.splice(first..last, out.chars());
            self.cursor = first + out.chars().count();
            return;
        }
        // No further on with several names: a menu of them, as tmux's
        // (not yet for sessions, which tmux also offers in one).
        let sessions = target || word.starts_with("-t") || word.starts_with("-s");
        if !sessions && !word.is_empty() {
            let list = word_matches(&word, first == 0, &names);
            if list.len() > 1 {
                self.menu = Some(CompletionMenu::new(&list, (first, last)));
            }
        }
    }

    /// A key while the completion menu is open: Tab moves down, as tmux's
    /// MENU_TAB; a chosen name replaces the word (no space after it, where
    /// a single match gets one), and anything closing the menu leaves the
    /// input as it was.
    fn menu_key(&mut self, ev: &KeyEvent) -> OverlayAction {
        let Some(open) = self.menu.as_mut() else {
            return OverlayAction::Keep;
        };
        let ev = if ev.code == KeyCode::Tab {
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
        } else {
            *ev
        };
        match open.menu.key(&ev) {
            OverlayAction::Run(name) => {
                let (first, last) = open.word;
                self.input.splice(first..last, name.chars());
                self.cursor = first + name.chars().count();
                self.menu = None;
            }
            OverlayAction::Keep => {}
            _ => self.menu = None,
        }
        OverlayAction::Keep
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
            Edit::Complete => self.complete(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Completions {
        let own = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect();
        Completions {
            commands: own(&[
                "new-window",
                "neww",
                "select-pane",
                "selectp",
                "select-window",
            ]),
            others: own(&["status-left", "status-left-length", "even-horizontal"]),
            sessions: vec![("main".into(), 0), ("mail".into(), 3), ("other".into(), 12)],
        }
    }

    #[test]
    fn tab_completes_as_tmux() {
        let n = names();
        let c = |word: &str, at_start: bool| completion(word, at_start, false, &n);
        // One match, with a space after; several, what they share.
        assert_eq!(c("new-w", true).as_deref(), Some("new-window "));
        assert_eq!(c("sel", true).as_deref(), Some("select"));
        // Options and layouts only after the first word.
        assert_eq!(c("status-l", true), None);
        assert_eq!(c("status-l", false).as_deref(), Some("status-left"));
        assert_eq!(c("even", false).as_deref(), Some("even-horizontal "));
        // Sessions after -t / -s, ended by a colon; $ takes an id.
        assert_eq!(c("-tma", false).as_deref(), Some("-tmai"));
        assert_eq!(c("-sot", false).as_deref(), Some("-sother:"));
        assert_eq!(c("-t$1", false).as_deref(), Some("-t$12:"));
        // Nothing further on, nothing to complete, or a window after a colon.
        assert_eq!(c("select", true), None);
        assert_eq!(c("", true), None);
        assert_eq!(c("-tmain:", false), None);
        // A target prompt completes any word as a session, without -t.
        let t = |word: &str| completion(word, true, true, &n);
        assert_eq!(t("ot").as_deref(), Some("other:"));
        assert_eq!(t("ma").as_deref(), Some("mai"));
        assert_eq!(t("new-w"), None, "not a command there");
    }
}
