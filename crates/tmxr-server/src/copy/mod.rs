//! vi copy mode.
//!
//! Entering copy mode snapshots the pane's history and screen into a grid;
//! the program keeps running underneath while the user moves around the
//! frozen copy, as in tmux. Motions are tmux's `copy-mode-vi` commands
//! (`send-keys -X <name>`), so every key is an ordinary bind.

mod commands;
mod model;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub use commands::{command, enter};
pub use model::{Cell, Line, cell_style, colour, snapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelKind {
    Char,
    Line,
    Rect,
}

#[derive(Debug, Clone)]
pub struct CopyMode {
    pub lines: Vec<Line>,
    /// First line shown.
    pub top: usize,
    /// Cursor (line, column).
    pub cy: usize,
    pub cx: usize,
    pub anchor: Option<(usize, usize)>,
    pub kind: SelKind,
    pub rows: u16,
    pub cols: u16,
    pub search: Option<Search>,
    /// Repeat count typed before a command (`5j`), 0 for none.
    pub count: usize,
    /// An `f` / `F` / `t` / `T` waiting for the character to jump to.
    pub pending_jump: Option<Jump>,
    /// The last jump and its character, for `;` and `,`.
    last_jump: Option<(Jump, char)>,
    /// `set-mark`: a position `jump-to-mark` returns to.
    mark: Option<(usize, usize)>,
    /// `copy-mode -e`: scrolling back to the bottom leaves copy mode.
    pub scroll_exit: bool,
    /// `toggle-position`: the position indicator is not drawn.
    pub hide_position: bool,
    /// The characters that end a word (`word-separators`), besides blanks.
    pub word_separators: String,
    /// A search goes on from the other end when it reaches one
    /// (`wrap-search`).
    pub wrap_search: bool,
    /// The column up, down and page moves return to, and the length of the
    /// line it was taken on: tmux's lastcx and lastsx.
    last_cx: usize,
    last_sx: usize,
    /// Selections are rectangles (tmux's rectflag): the mode's, not one
    /// selection's, so it lasts until toggled off or a line or word is
    /// selected.
    rect: bool,
}

/// The last search, for `search-again` and highlighting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    pub needle: String,
    pub forward: bool,
    /// `search-forward` / `-backward` take a regular expression, the `-text`
    /// and incremental searches the text itself, as tmux's searchregex.
    pub regex: bool,
}

/// tmux's default `word-separators`: ASCII punctuation but `_`.
pub const WORD_SEPARATORS: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^`{|}~";

/// Columns where `needle` starts in `line`: literal, and case-insensitive
/// unless the needle has an uppercase letter (smart case).
/// The characters that make a search a regular expression; one without
/// them is searched for as text, as tmux's window_copy_search decides.
const REGEX_CHARS: &[char] = &['^', '$', '*', '+', '(', ')', '?', '[', ']', '.', '\\'];

/// The `(start, end)` columns of `needle`'s matches in `line`: as an
/// extended regular expression when `regex` asks and the needle has one of
/// [`REGEX_CHARS`] (a pattern that does not compile matches nothing), else
/// literally; case-insensitive unless the needle has an uppercase letter
/// (smart case), as tmux's searches are.
fn matches(line: &[char], needle: &str, regex: bool) -> Vec<(usize, usize)> {
    if regex && needle.contains(REGEX_CHARS) {
        let Ok(re) = regex::RegexBuilder::new(needle)
            .case_insensitive(!needle.chars().any(char::is_uppercase))
            .build()
        else {
            return Vec::new();
        };
        let text: String = line.iter().collect();
        // Byte offsets in the text to columns of the line.
        let column = |byte: usize| text[..byte].chars().count();
        return re
            .find_iter(&text)
            .filter(|m| !m.is_empty())
            .map(|m| (column(m.start()), column(m.end())))
            .collect();
    }
    let len = needle.chars().count();
    match_starts(line, needle)
        .into_iter()
        .map(|x| (x, x + len))
        .collect()
}

fn match_starts(line: &[char], needle: &str) -> Vec<usize> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() || needle.len() > line.len() {
        return Vec::new();
    }
    let smart_case = needle.iter().any(|c| c.is_uppercase());
    let eq = |a: char, b: char| {
        if smart_case {
            a == b
        } else {
            a.to_lowercase().eq(b.to_lowercase())
        }
    };
    (0..=line.len() - needle.len())
        .filter(|x| needle.iter().zip(&line[*x..]).all(|(a, b)| eq(*a, *b)))
        .collect()
}

/// Whether `c` ends a word: a blank, as tmux always counts, or one of
/// `separators` (`word-separators`).
fn is_word_separator(c: char, separators: &str) -> bool {
    c == ' ' || c == '\t' || separators.contains(c)
}

/// The word at column `x` of `line` as tmux 3.6's `format_grid_word`
/// finds it: the run of non-separators around `x`, or on a separator the
/// word just after it. tmux also follows a word across a wrapped line; this
/// keeps to `line`.
pub fn word_at(line: &[char], x: usize, separators: &str) -> String {
    let separator = |x: usize| {
        line.get(x)
            .is_none_or(|c| is_word_separator(*c, separators))
    };
    let mut start = x;
    if separator(start) {
        start += 1;
    } else {
        while start > 0 && !separator(start - 1) {
            start -= 1;
        }
    }
    let mut end = start;
    while !separator(end) {
        end += 1;
    }
    line.get(start..end)
        .map(String::from_iter)
        .unwrap_or_default()
}

/// Largest repeat count, so a stray run of digits cannot spin the server.
pub const MAX_COUNT: usize = 9999;

/// vi's in-line character jumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jump {
    /// `f`: onto the next occurrence.
    Forward,
    /// `F`: onto the previous occurrence.
    Backward,
    /// `t`: just before the next occurrence.
    ToForward,
    /// `T`: just after the previous occurrence.
    ToBackward,
}

impl Jump {
    fn from_command(name: &str) -> Option<Self> {
        Some(match name {
            "jump-forward" => Self::Forward,
            "jump-backward" => Self::Backward,
            "jump-to-forward" => Self::ToForward,
            "jump-to-backward" => Self::ToBackward,
            _ => return None,
        })
    }

    fn command(self) -> &'static str {
        match self {
            Self::Forward => "jump-forward",
            Self::Backward => "jump-backward",
            Self::ToForward => "jump-to-forward",
            Self::ToBackward => "jump-to-backward",
        }
    }

    fn reversed(self) -> Self {
        match self {
            Self::Forward => Self::Backward,
            Self::Backward => Self::Forward,
            Self::ToForward => Self::ToBackward,
            Self::ToBackward => Self::ToForward,
        }
    }
}

impl CopyMode {
    pub fn new(screen: &mut vt100::Screen) -> Self {
        let (rows, cols) = screen.size();
        let lines = snapshot(screen);
        let (crow, ccol) = screen.cursor_position();
        let top = lines.len().saturating_sub(usize::from(rows));
        Self {
            top,
            cy: top + usize::from(crow),
            cx: usize::from(ccol),
            lines,
            anchor: None,
            kind: SelKind::Char,
            rows,
            cols,
            search: None,
            count: 0,
            pending_jump: None,
            last_jump: None,
            mark: None,
            scroll_exit: false,
            hide_position: false,
            word_separators: WORD_SEPARATORS.to_owned(),
            wrap_search: true,
            last_cx: 0,
            last_sx: 0,
            rect: false,
        }
    }

    /// Keep the view, cursor, selection, search and mark of `old` (a
    /// snapshot of the same pane taken earlier), clamped to this one.
    fn carry_over(&mut self, old: &Self) {
        let last = self.last_line();
        let clamp = |(y, x): (usize, usize)| (y.min(last), x);
        (self.cy, self.cx) = clamp((old.cy, old.cx));
        self.top = old.top.min(last);
        self.anchor = old.anchor.map(clamp);
        self.kind = old.kind;
        self.search.clone_from(&old.search);
        self.mark = old.mark.map(clamp);
        self.last_jump = old.last_jump;
        self.scroll_exit = old.scroll_exit;
        self.hide_position = old.hide_position;
        self.scroll_to_cursor();
    }

    /// vi's `%`: from the first bracket at or after the cursor on its line,
    /// to the bracket that matches it, across lines.
    /// `previous-matching-bracket` (emacs `C-M-b`) looks for the bracket at
    /// or before the cursor instead.
    fn matching_bracket(&mut self, before: bool) {
        const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
        let line = self.lines.get(self.cy).map(Line::chars).unwrap_or_default();
        let is_bracket = |c: &char| PAIRS.iter().any(|(o, cl)| c == o || c == cl);
        let found = if before {
            line.iter()
                .enumerate()
                .take(self.cx + 1)
                .rev()
                .find(|(_, c)| is_bracket(c))
        } else {
            line.iter()
                .enumerate()
                .skip(self.cx)
                .find(|(_, c)| is_bracket(c))
        };
        let Some((x, c)) = found else {
            return;
        };
        let (forward, open, close) = match PAIRS.iter().find(|(o, cl)| c == o || c == cl) {
            Some(&(o, cl)) => (*c == o, o, cl),
            None => return,
        };
        let mut depth = 0usize;
        let mut pos = Some((self.cy, x));
        while let Some((y, px)) = pos {
            let ch = self.char_at(y, px);
            if ch == open {
                depth = if forward {
                    depth + 1
                } else {
                    depth.saturating_sub(1)
                };
            } else if ch == close {
                depth = if forward {
                    depth.saturating_sub(1)
                } else {
                    depth + 1
                };
            }
            if depth == 0 {
                (self.cy, self.cx) = (y, px);
                return;
            }
            pos = if forward {
                self.next_pos(y, px)
            } else {
                self.prev_pos(y, px)
            };
        }
    }

    /// Keys copy mode takes before its key table: the character a pending
    /// jump waits for (any other key cancels the wait) and count digits,
    /// typed plain in vi mode and with Alt (`M-1`) in emacs mode, as tmux's
    /// tables have them. Returns whether the key was used.
    pub fn take_key(&mut self, ev: &KeyEvent, emacs: bool) -> bool {
        let plain = !ev
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        let digit_mods = if emacs {
            ev.modifiers.contains(KeyModifiers::ALT)
                && !ev.modifiers.contains(KeyModifiers::CONTROL)
        } else {
            plain
        };
        if let Some(jump) = self.pending_jump.take() {
            if let (KeyCode::Char(c), true) = (ev.code, plain) {
                let times = std::mem::take(&mut self.count).max(1);
                self.apply(jump.command(), Some(&c.to_string()));
                for _ in 1..times {
                    self.apply("jump-again", None);
                }
            }
            self.count = 0;
            return true;
        }
        match (ev.code, digit_mods) {
            // `0` is start-of-line unless it continues a count.
            (KeyCode::Char(d @ '0'..='9'), true) if d != '0' || self.count > 0 => {
                let digit = d as usize - '0' as usize;
                self.count = (self.count * 10 + digit).min(MAX_COUNT);
                true
            }
            _ => false,
        }
    }

    /// Run a command as many times as the pending count says (once without
    /// one). A jump still waiting for its character keeps the count for it.
    pub fn apply_counted(&mut self, name: &str, arg: Option<&str>) -> bool {
        if Jump::from_command(name).is_some() && arg.is_none() {
            return self.apply(name, None);
        }
        let times = std::mem::take(&mut self.count).max(1);
        (0..times).all(|_| self.apply(name, arg))
    }

    /// Move to `c` on the cursor's line; `again` is a `;` / `,` repeat, which
    /// steps past a `t` / `T` target the cursor already sits beside.
    fn jump(&mut self, jump: Jump, c: char, again: bool) {
        let line = self.lines.get(self.cy).map(Line::chars).unwrap_or_default();
        let len = self.line_len(self.cy);
        let till = matches!(jump, Jump::ToForward | Jump::ToBackward);
        let skip = usize::from(again && till);
        match jump {
            Jump::Forward | Jump::ToForward => {
                let from = self.cx + 1 + skip;
                if let Some(x) = (from..len).find(|x| line.get(*x) == Some(&c)) {
                    self.cx = if till { x - 1 } else { x };
                }
            }
            Jump::Backward | Jump::ToBackward => {
                let to = self.cx.saturating_sub(skip);
                if let Some(x) = (0..to).rev().find(|x| line.get(*x) == Some(&c)) {
                    self.cx = if till { x + 1 } else { x };
                }
            }
        }
    }

    /// A page (or half a page) up or down, as tmux's window_copy_pageup1
    /// and window_copy_pagedown1: the view moves `rows - 2` lines (half the
    /// rows for half a page), the cursor keeping its row on screen; where
    /// the view reaches the top or bottom first, the cursor moves on by the
    /// page instead.
    fn page(&mut self, down: bool, half: bool) {
        // The column is kept as for up and down, then put at the new line's
        // end where it was at the old one's, as tmux's pageup1.
        let ox = self.line_len(self.cy);
        if self.cx != ox {
            (self.last_cx, self.last_sx) = (self.cx, ox);
        }
        self.cx = self.last_cx;
        self.page_lines(down, half);
        if !self.rect_selecting() && self.past_line_end() {
            self.end_of_line();
        }
    }

    fn page_lines(&mut self, down: bool, half: bool) {
        let rows = usize::from(self.rows.max(1));
        let n = match rows {
            0..=2 => 1,
            _ if half => rows / 2,
            _ => rows - 2,
        };
        let max_top = self.lines.len().saturating_sub(rows);
        let row = self.cy.saturating_sub(self.top);
        if down {
            if max_top - self.top.min(max_top) < n {
                self.top = max_top;
                self.cy = (max_top + (row + n).min(rows - 1)).min(self.last_line());
            } else {
                self.top += n;
                self.cy += n;
            }
        } else if self.top < n {
            self.top = 0;
            self.cy = row.saturating_sub(n);
        } else {
            self.top -= n;
            self.cy -= n;
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        self.scroll_to_cursor();
    }

    /// Lines of history above the bottom of the view (tmux's `[n/total]`).
    pub fn position(&self) -> (usize, usize) {
        let bottom_top = self.lines.len().saturating_sub(usize::from(self.rows));
        (bottom_top.saturating_sub(self.top), bottom_top)
    }

    /// `#{copy_cursor_word}`: the word at the cursor as tmux 3.6's
    /// `format_grid_word` finds it, the run of non-separators around the
    /// cursor, or on a separator the word just after it. tmux also follows a
    /// word across a wrapped line; this keeps to the cursor's line.
    pub fn cursor_word(&self) -> String {
        let line = self.lines.get(self.cy).map(Line::chars).unwrap_or_default();
        word_at(&line, self.cx, &self.word_separators)
    }

    /// The text of the line shown `row` rows down the view.
    pub fn view_line(&self, row: usize) -> Option<Vec<char>> {
        self.lines.get(self.top + row).map(Line::chars)
    }

    fn last_line(&self) -> usize {
        self.lines.len().saturating_sub(1)
    }

    fn line_len(&self, y: usize) -> usize {
        self.lines.get(y).map_or(0, |l| {
            l.cells
                .iter()
                .rposition(|c| !c.symbol.trim().is_empty())
                .map_or(0, |i| i + 1)
        })
    }

    fn scroll_to_cursor(&mut self) {
        let rows = usize::from(self.rows.max(1));
        if self.cy < self.top {
            self.top = self.cy;
        } else if self.cy >= self.top + rows {
            self.top = self.cy + 1 - rows;
        }
        self.top = self.top.min(self.lines.len().saturating_sub(rows));
    }

    /// The cursor may sit one past a full line's last cell, as tmux's.
    fn clamp_x(&mut self) {
        self.cx = self.cx.min(usize::from(self.cols));
    }

    /// A rectangle is being selected: tmux's `sel && rectflag`, under which
    /// up and down keep the column and right goes to the screen's edge.
    fn rect_selecting(&self) -> bool {
        self.anchor.is_some() && self.kind == SelKind::Rect
    }

    /// Turn rectangle selection on or off, for the selection there is and
    /// the ones after it, the cursor kept within its line, as tmux's
    /// window_copy_rectangle_set.
    fn set_rect(&mut self, on: bool) {
        self.rect = on;
        self.kind = if on { SelKind::Rect } else { SelKind::Char };
        self.cx = self.cx.min(self.line_len(self.cy));
    }

    /// The cell is the right half of a wide character (tmux's padding).
    fn padding(&self, y: usize, x: usize) -> bool {
        self.lines
            .get(y)
            .and_then(|l| l.cells.get(x))
            .is_some_and(|c| c.symbol.is_empty())
    }

    /// Whether the column, taken back to after an up, down or page move,
    /// belongs at the new line's end: it was at the old line's end, or the
    /// new line is shorter, as tmux's test after those moves.
    fn past_line_end(&self) -> bool {
        let px = self.line_len(self.cy);
        (self.cx >= self.last_sx && self.cx != px) || self.cx > px
    }

    /// Up or down a line (`down`), the column as tmux's
    /// window_copy_cursor_up / _down keep it.
    fn vertical(&mut self, down: bool) {
        let rect = self.rect_selecting();
        let ox = self.line_len(self.cy);
        if !rect && self.cx != ox {
            (self.last_cx, self.last_sx) = (self.cx, ox);
        }
        self.cy = if down {
            (self.cy + 1).min(self.last_line())
        } else {
            self.cy.saturating_sub(1)
        };
        if !rect {
            self.cx = self.last_cx;
            if self.past_line_end() {
                self.cx = self.line_len(self.cy);
            }
        }
    }

    /// To the end of the line, following its soft wraps, as tmux's
    /// grid_reader_cursor_end_of_line with wrap: the screen's edge under a
    /// rectangle selection, else one past the last character.
    fn end_of_line(&mut self) {
        while self.cy < self.last_line() && self.lines[self.cy].wrapped {
            self.cy += 1;
        }
        self.cx = if self.rect_selecting() {
            usize::from(self.cols)
        } else {
            self.line_len(self.cy)
        };
    }

    /// Right a character, past a wide one's padding, and on to the next
    /// line's start from the end, as tmux's grid_reader_cursor_right.
    fn cursor_right(&mut self) {
        let px = if self.rect_selecting() {
            usize::from(self.cols)
        } else {
            self.line_len(self.cy)
        };
        if self.cx >= px && self.cy < self.last_line() {
            (self.cy, self.cx) = (self.cy + 1, 0);
        } else if self.cx < px {
            self.cx += 1;
            while self.cx < px && self.padding(self.cy, self.cx) {
                self.cx += 1;
            }
        }
    }

    /// Left a character, past a wide one's padding, and on to the previous
    /// line's end from the start, as tmux's grid_reader_cursor_left.
    fn cursor_left(&mut self) {
        while self.cx > 0 && self.padding(self.cy, self.cx) {
            self.cx -= 1;
        }
        if self.cx == 0 && self.cy > 0 {
            self.cy -= 1;
            self.cx = self.line_len(self.cy);
        } else if self.cx > 0 {
            self.cx -= 1;
        }
    }

    fn char_at(&self, y: usize, x: usize) -> char {
        self.lines
            .get(y)
            .and_then(|l| l.cells.get(x))
            .and_then(|c| c.symbol.chars().next())
            .unwrap_or(' ')
    }

    /// Word class: 0 blank, 1 word char, 2 punctuation; `big` (WORD) makes
    /// every non-blank the same class.
    fn class(&self, y: usize, x: usize, big: bool) -> u8 {
        // Blanks, word characters and separators, as tmux's
        // `window_copy_in_set` reads `word-separators`.
        let c = self.char_at(y, x);
        if c.is_whitespace() {
            0
        } else if big || !self.word_separators.contains(c) {
            1
        } else {
            2
        }
    }

    /// vi's `}` / `{`: past the blank lines next to the cursor, then to the
    /// next blank line that way (or the first / last line).
    fn paragraph(&mut self, forward: bool) {
        let last = self.last_line();
        let step = |y: usize| {
            if forward {
                (y < last).then_some(y + 1)
            } else {
                y.checked_sub(1)
            }
        };
        let blank = |y: usize| self.line_len(y) == 0;
        let mut y = self.cy;
        while blank(y)
            && let Some(n) = step(y)
        {
            y = n;
        }
        while !blank(y)
            && let Some(n) = step(y)
        {
            y = n;
        }
        (self.cy, self.cx) = (y, 0);
    }

    fn next_pos(&self, y: usize, x: usize) -> Option<(usize, usize)> {
        if x + 1 < usize::from(self.cols) {
            Some((y, x + 1))
        } else if y < self.last_line() {
            Some((y + 1, 0))
        } else {
            None
        }
    }

    fn prev_pos(&self, y: usize, x: usize) -> Option<(usize, usize)> {
        if x > 0 {
            Some((y, x - 1))
        } else if y > 0 {
            Some((y - 1, usize::from(self.cols.saturating_sub(1))))
        } else {
            None
        }
    }

    fn next_word(&mut self, big: bool) {
        let (mut y, mut x) = (self.cy, self.cx);
        let start = self.class(y, x, big);
        while let Some((ny, nx)) = self.next_pos(y, x) {
            let line_change = ny != y;
            (y, x) = (ny, nx);
            let c = self.class(y, x, big);
            if c != 0 && (c != start || line_change) {
                break;
            }
            if c == 0 {
                // Skip the blank run, then stop at the next word.
                while let Some((ny, nx)) = self.next_pos(y, x) {
                    if self.class(y, x, big) != 0 {
                        break;
                    }
                    (y, x) = (ny, nx);
                }
                break;
            }
        }
        (self.cy, self.cx) = (y, x);
    }

    fn word_end(&mut self, big: bool) {
        let (mut y, mut x) = (self.cy, self.cx);
        if let Some(p) = self.next_pos(y, x) {
            (y, x) = p;
        }
        while self.class(y, x, big) == 0 {
            match self.next_pos(y, x) {
                Some(p) => (y, x) = p,
                None => break,
            }
        }
        let c = self.class(y, x, big);
        while let Some((ny, nx)) = self.next_pos(y, x) {
            if ny != y || self.class(ny, nx, big) != c {
                break;
            }
            (y, x) = (ny, nx);
        }
        (self.cy, self.cx) = (y, x);
    }

    fn prev_word(&mut self, big: bool) {
        let (mut y, mut x) = (self.cy, self.cx);
        if let Some(p) = self.prev_pos(y, x) {
            (y, x) = p;
        }
        while self.class(y, x, big) == 0 {
            match self.prev_pos(y, x) {
                Some(p) => (y, x) = p,
                None => break,
            }
        }
        let c = self.class(y, x, big);
        while let Some((py, px)) = self.prev_pos(y, x) {
            if py != y || self.class(py, px, big) != c {
                break;
            }
            (y, x) = (py, px);
        }
        (self.cy, self.cx) = (y, x);
    }

    fn find(&mut self, needle: &str, forward: bool, regex: bool, skip_current: bool) -> bool {
        if needle.is_empty() {
            return false;
        }
        let n = self.lines.len();
        for step in 0..=n {
            // Without `wrap-search` the search stops at the end it heads for.
            if !self.wrap_search
                && (if forward {
                    self.cy + step >= n
                } else {
                    step > self.cy
                })
            {
                break;
            }
            let y = if forward {
                (self.cy + step) % n
            } else {
                (self.cy + n - step % n) % n
            };
            let hits = matches(&self.lines[y].chars(), needle, regex)
                .into_iter()
                .map(|(x, _)| x);
            let pick = if forward {
                hits.into_iter()
                    .find(|x| step > 0 || *x > self.cx || (!skip_current && *x == self.cx))
            } else {
                hits.collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .find(|x| step > 0 || *x < self.cx || (!skip_current && *x == self.cx))
            };
            if let Some(x) = pick {
                (self.cy, self.cx) = (y, x);
                return true;
            }
        }
        false
    }

    /// Columns `(start, end)` of the last search's matches on line `y`, for
    /// highlighting.
    pub fn match_spans(&self, y: usize) -> Vec<(usize, usize)> {
        let (Some(search), Some(line)) = (&self.search, self.lines.get(y)) else {
            return Vec::new();
        };
        matches(&line.chars(), &search.needle, search.regex)
    }

    /// Selected text, or `None` without a selection.
    pub fn selection_text(&self) -> Option<String> {
        let (ay, ax) = self.anchor?;
        let ((sy, sx), (ey, ex)) = if (ay, ax) <= (self.cy, self.cx) {
            ((ay, ax), (self.cy, self.cx))
        } else {
            ((self.cy, self.cx), (ay, ax))
        };
        let cols = usize::from(self.cols);
        let mut out = String::new();
        for y in sy..=ey {
            let Some(line) = self.lines.get(y) else { break };
            let (from, to) = match self.kind {
                SelKind::Line => (0, cols),
                SelKind::Rect => (ax.min(self.cx), ax.max(self.cx) + 1),
                SelKind::Char => (
                    if y == sy { sx } else { 0 },
                    if y == ey { ex + 1 } else { cols },
                ),
            };
            out.push_str(&line.text(from, to));
            let soft = line.wrapped && self.kind == SelKind::Char && y != ey;
            if y != ey && !soft {
                out.push('\n');
            }
        }
        if self.kind == SelKind::Line {
            out.push('\n');
        }
        Some(out)
    }

    /// Whether `(y, x)` is inside the selection, for drawing.
    pub fn selected(&self, y: usize, x: usize) -> bool {
        let Some((ay, ax)) = self.anchor else {
            return false;
        };
        let (s, e) = if (ay, ax) <= (self.cy, self.cx) {
            ((ay, ax), (self.cy, self.cx))
        } else {
            ((self.cy, self.cx), (ay, ax))
        };
        match self.kind {
            SelKind::Line => y >= s.0 && y <= e.0,
            SelKind::Rect => y >= s.0 && y <= e.0 && x >= ax.min(self.cx) && x <= ax.max(self.cx),
            SelKind::Char => (y, x) >= s && (y, x) <= e,
        }
    }

    /// Apply a non-copying motion or selection command. Returns `false` for
    /// an unknown command.
    pub fn apply(&mut self, name: &str, arg: Option<&str>) -> bool {
        let rows = usize::from(self.rows.max(1));
        let last = self.last_line();
        match name {
            "cursor-left" => self.cursor_left(),
            "cursor-right" => self.cursor_right(),
            "cursor-up" => self.vertical(false),
            "cursor-down" => self.vertical(true),
            "start-of-line" => self.cx = 0,
            "end-of-line" => self.end_of_line(),
            "back-to-indentation" => {
                self.cx = self.lines[self.cy]
                    .chars()
                    .iter()
                    .position(|c| !c.is_whitespace())
                    .unwrap_or(0);
            }
            "next-word" => self.next_word(false),
            "next-space" => self.next_word(true),
            "previous-word" => self.prev_word(false),
            "previous-space" => self.prev_word(true),
            "next-word-end" => self.word_end(false),
            "next-space-end" => self.word_end(true),
            "history-top" => (self.cy, self.cx) = (0, 0),
            "history-bottom" => self.cy = last,
            "top-line" => self.cy = self.top,
            "middle-line" => self.cy = (self.top + rows / 2).min(last),
            "bottom-line" => self.cy = (self.top + rows - 1).min(last),
            "halfpage-up" => self.page(false, true),
            "halfpage-down" => self.page(true, true),
            "page-up" => self.page(false, false),
            "page-down" => self.page(true, false),
            "scroll-up" => {
                self.top = self.top.saturating_sub(1);
                if self.cy >= self.top + rows {
                    self.cy = self.top + rows - 1;
                }
            }
            "scroll-down" => {
                self.top = (self.top + 1).min(self.lines.len().saturating_sub(rows));
                self.cy = self.cy.max(self.top);
            }
            "begin-selection" => {
                self.anchor = Some((self.cy, self.cx));
                self.kind = if self.rect {
                    SelKind::Rect
                } else {
                    SelKind::Char
                };
            }
            // From the line's start to its end, the cursor at the end, as
            // tmux's window_copy_cmd_select_line.
            "select-line" => {
                self.rect = false;
                self.anchor = Some((self.cy, 0));
                self.kind = SelKind::Line;
                self.end_of_line();
            }
            "select-word" => {
                // The run of same-class cells (word, punctuation or blank)
                // under the cursor, on its line; not a rectangle.
                self.rect = false;
                let class = self.class(self.cy, self.cx, false);
                let len = self.line_len(self.cy).max(self.cx + 1);
                let start = (0..self.cx)
                    .rev()
                    .take_while(|&x| self.class(self.cy, x, false) == class)
                    .last()
                    .unwrap_or(self.cx);
                let end = (self.cx..len)
                    .take_while(|&x| self.class(self.cy, x, false) == class)
                    .last()
                    .unwrap_or(self.cx);
                self.anchor = Some((self.cy, start));
                self.kind = SelKind::Char;
                self.cx = end;
            }
            "rectangle-toggle" => self.set_rect(!self.rect),
            "rectangle-on" => self.set_rect(true),
            "rectangle-off" => self.set_rect(false),
            "clear-selection" => {
                self.anchor = None;
                self.cx = self.cx.min(self.line_len(self.cy));
            }
            // tmux's: the cursor goes to the selection's other end, which is
            // where the selection is now held from. A count swaps that many
            // times.
            "other-end" => {
                if let Some(other) = self.anchor {
                    self.anchor = Some((self.cy, self.cx));
                    (self.cy, self.cx) = other;
                }
            }
            "cursor-centre-vertical" => self.cy = (self.top + rows / 2).min(last),
            "cursor-centre-horizontal" => self.cx = usize::from(self.cols) / 2,
            // The view moves so the cursor's line is in its middle.
            "scroll-middle" => {
                self.top = self
                    .cy
                    .saturating_sub((rows - 1) / 2)
                    .min(self.lines.len().saturating_sub(rows));
            }
            // The cursor's line to the top or bottom of the view, only when
            // there is room to scroll all the way, as tmux's
            // window_copy_cmd_scroll_to.
            "scroll-top" if self.cy <= self.lines.len().saturating_sub(rows) => self.top = self.cy,
            "scroll-bottom" if self.cy + 1 >= rows => self.top = self.cy + 1 - rows,
            "scroll-top" | "scroll-bottom" => {}
            "toggle-position" => self.hide_position = !self.hide_position,
            "search-forward"
            | "search-backward"
            | "search-forward-text"
            | "search-backward-text" => {
                let forward = name.starts_with("search-forward");
                let regex = !name.ends_with("-text");
                let needle = arg.unwrap_or_default().to_owned();
                self.find(&needle, forward, regex, true);
                self.search = Some(Search {
                    needle,
                    forward,
                    regex,
                });
            }
            // Run again as each character is typed: a match at the cursor
            // counts, so the cursor stays while the longer text still
            // matches there.
            "search-forward-incremental" | "search-backward-incremental" => {
                let forward = name == "search-forward-incremental";
                let needle = arg.unwrap_or_default().to_owned();
                self.find(&needle, forward, false, false);
                self.search = Some(Search {
                    needle,
                    forward,
                    regex: false,
                });
            }
            "jump-forward" | "jump-backward" | "jump-to-forward" | "jump-to-backward" => {
                let jump = Jump::from_command(name).expect("matched a jump command");
                match arg.and_then(|a| a.chars().next()) {
                    Some(c) => {
                        self.last_jump = Some((jump, c));
                        self.jump(jump, c, false);
                    }
                    None => self.pending_jump = Some(jump),
                }
            }
            "set-mark" => self.mark = Some((self.cy, self.cx)),
            "jump-to-mark" => {
                if let Some(m) = self.mark.replace((self.cy, self.cx)) {
                    (self.cy, self.cx) = m;
                }
            }
            "next-matching-bracket" => self.matching_bracket(false),
            "previous-matching-bracket" => self.matching_bracket(true),
            "next-paragraph" => self.paragraph(true),
            "previous-paragraph" => self.paragraph(false),
            "goto-line" => {
                // tmux's line number: how far above the bottom of the
                // history the view starts, the cursor keeping its row.
                let Some(n) = arg.and_then(|a| a.trim().parse::<usize>().ok()) else {
                    return false;
                };
                let (_, bottom_top) = self.position();
                let row = self.cy.saturating_sub(self.top);
                self.top = bottom_top - n.min(bottom_top);
                self.cy = (self.top + row).min(last);
                return true;
            }
            "jump-again" | "jump-reverse" => {
                if let Some((jump, c)) = self.last_jump {
                    let jump = if name == "jump-again" {
                        jump
                    } else {
                        jump.reversed()
                    };
                    self.jump(jump, c, true);
                }
            }
            "search-again" | "search-reverse" => {
                if let Some(search) = self.search.clone() {
                    let forward = if name == "search-again" {
                        search.forward
                    } else {
                        !search.forward
                    };
                    self.find(&search.needle, forward, search.regex, true);
                }
            }
            _ => return false,
        }
        if !matches!(name, "scroll-up" | "scroll-down") {
            self.clamp_x();
            self.scroll_to_cursor();
        }
        true
    }
}

#[cfg(test)]
mod tests;
