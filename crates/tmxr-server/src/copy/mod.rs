//! vi copy mode.
//!
//! Entering copy mode snapshots the pane's history and screen into a grid;
//! the program keeps running underneath while the user moves around the
//! frozen copy, as in tmux. Motions are tmux's `copy-mode-vi` commands
//! (`send-keys -X <name>`), so every key is an ordinary bind.

mod commands;
mod model;
mod search;
mod words;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub use commands::{command, enter, enter_from, refuses_args};
pub use model::{Cell, Line, cell_style, colour, snapshot};
pub use search::Search;
pub use words::word_at;

/// How the selection's moving end goes as the cursor moves, as tmux's
/// selflag: by characters, or snapped to words or lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelFlag {
    Char,
    Word,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelKind {
    Char,
    Line,
    Rect,
}

#[derive(Debug, Clone)]
pub struct CopyMode {
    /// The pane whose screen and history this shows, when not its own
    /// (`copy-mode -s`): what `refresh-from-pane` reads again.
    pub source: Option<crate::model::PaneId>,
    pub lines: Vec<Line>,
    /// First line shown.
    pub top: usize,
    /// Cursor (line, column).
    pub cy: usize,
    pub cx: usize,
    pub anchor: Option<(usize, usize)>,
    /// `stop-selection`: the selection's other end, fixed where the cursor
    /// was, so the cursor moves without it (tmux's cursordrag none); `None`
    /// while the cursor is that end.
    pub sel_end: Option<(usize, usize)>,
    pub kind: SelKind,
    pub rows: u16,
    pub cols: u16,
    pub search: Option<Search>,
    /// The pane's last search from an earlier copy mode, which
    /// `search-again` / `search-reverse` repeat upwards until this mode
    /// makes its own, as tmux's window_copy_common_init takes it.
    pub inherited_search: Option<Search>,
    /// The pane printed since copy mode began (`#{pane_unseen_changes}`).
    pub unseen_changes: bool,
    /// Where an incremental search started (cursor and view top), which
    /// each change of its text searches from again, as tmux's searchx /
    /// searchy / searcho; any other command forgets it.
    search_origin: Option<(usize, usize, usize)>,
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
    /// tmux's selflag: in word or line mode the selection keeps its first
    /// word or line (`sel_reset`, tmux's selrx / endselrx) and grows by
    /// words or lines from where it was made (`sel_origin`, tmux's dx, dy).
    selflag: SelFlag,
    sel_reset: ((usize, usize), (usize, usize)),
    sel_origin: (usize, usize),
    /// Where `select-word` / `select-line` left the cursor: until it moves
    /// from there the selection is as made, as tmux makes it with no_reset.
    sel_made_at: (usize, usize),
}

/// tmux's default `word-separators`: ASCII punctuation but `_`.
pub const WORD_SEPARATORS: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^`{|}~";

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
        let (rows, cols) = tmxr_term::screen_size(screen);
        let lines = snapshot(screen);
        let (crow, ccol) = screen.cursor_position();
        let top = lines.len().saturating_sub(usize::from(rows));
        Self {
            top,
            cy: top + usize::from(crow),
            cx: usize::from(ccol),
            lines,
            anchor: None,
            sel_end: None,
            kind: SelKind::Char,
            rows,
            cols,
            search: None,
            inherited_search: None,
            unseen_changes: false,
            search_origin: None,
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
            source: None,
            selflag: SelFlag::Char,
            sel_reset: ((0, 0), (0, 0)),
            sel_origin: (0, 0),
            sel_made_at: (0, 0),
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
        self.sel_end = old.sel_end.map(clamp);
        self.kind = old.kind;
        self.search.clone_from(&old.search);
        self.inherited_search.clone_from(&old.inherited_search);
        self.unseen_changes = old.unseen_changes;
        self.mark = old.mark.map(clamp);
        self.last_jump = old.last_jump;
        self.scroll_exit = old.scroll_exit;
        self.hide_position = old.hide_position;
        self.source = old.source;
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

    /// The selection's moving end: the cursor, unless `stop-selection`
    /// fixed it.
    fn selection_end(&self) -> (usize, usize) {
        self.sel_end.unwrap_or((self.cy, self.cx))
    }

    /// The selection's two ends, its anchor first: in word or line mode
    /// snapped as tmux's window_copy_synchronize_cursor_end, the first word
    /// or line kept on whichever side the cursor is not.
    fn selection_ends(&self) -> Option<((usize, usize), (usize, usize))> {
        let anchor = self.anchor?;
        let cursor = self.selection_end();
        if self.sel_end.is_some() || cursor == self.sel_made_at {
            return Some((anchor, cursor));
        }
        let (start, end) = self.sel_reset;
        let backwards = self.sel_origin > cursor;
        Some(match self.selflag {
            SelFlag::Char => (anchor, cursor),
            SelFlag::Word if backwards => (end, self.word_start_pos(cursor)),
            SelFlag::Word => {
                let (y, x) = cursor;
                let snapped = if x >= self.line_len(y) || self.class(y, x + 1, false) != 0 {
                    self.word_end_pos(cursor, false)
                } else {
                    cursor
                };
                (start, snapped)
            }
            SelFlag::Line if self.sel_origin.0 > cursor.0 => (end, (cursor.0, 0)),
            SelFlag::Line => {
                let y = cursor.0.max(end.0);
                (start, (y, self.line_len(y)))
            }
        })
    }

    /// Selected text, or `None` without a selection.
    pub fn selection_text(&self) -> Option<String> {
        let ((ay, ax), end) = self.selection_ends()?;
        let ((sy, sx), (ey, ex)) = if (ay, ax) <= end {
            ((ay, ax), end)
        } else {
            (end, (ay, ax))
        };
        let cols = usize::from(self.cols);
        let mut out = String::new();
        for y in sy..=ey {
            let Some(line) = self.lines.get(y) else { break };
            let (from, to) = match self.kind {
                SelKind::Line => (0, cols),
                SelKind::Rect => (ax.min(end.1), ax.max(end.1) + 1),
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
        let Some(((ay, ax), end)) = self.selection_ends() else {
            return false;
        };
        let (s, e) = if (ay, ax) <= end {
            ((ay, ax), end)
        } else {
            (end, (ay, ax))
        };
        match self.kind {
            SelKind::Line => y >= s.0 && y <= e.0,
            SelKind::Rect => y >= s.0 && y <= e.0 && x >= ax.min(end.1) && x <= ax.max(end.1),
            SelKind::Char => (y, x) >= s && (y, x) <= e,
        }
    }

    /// Apply a non-copying motion or selection command. Returns `false` for
    /// an unknown command.
    pub fn apply(&mut self, name: &str, arg: Option<&str>) -> bool {
        let rows = usize::from(self.rows.max(1));
        let last = self.last_line();
        if !name.starts_with("search-") {
            self.search_origin = None;
        }
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
                self.selflag = SelFlag::Char;
                self.anchor = Some((self.cy, self.cx));
                self.sel_end = None;
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
                self.selflag = SelFlag::Line;
                self.sel_origin = (self.cy, self.cx);
                self.anchor = Some((self.cy, 0));
                self.sel_end = None;
                self.kind = SelKind::Line;
                self.end_of_line();
                self.sel_reset = ((self.cy, 0), (self.cy, self.line_len(self.cy)));
                self.sel_made_at = (self.cy, self.cx);
            }
            "select-word" => {
                // The run of same-class cells (word, punctuation or blank)
                // under the cursor, across wrapped lines as tmux's grid
                // reader; not a rectangle.
                self.rect = false;
                let origin = (self.cy, self.cx);
                let (start, end) = self.class_run(origin);
                self.selflag = SelFlag::Word;
                self.anchor = Some(start);
                self.sel_end = None;
                self.kind = SelKind::Char;
                (self.cy, self.cx) = end;
                self.sel_reset = (start, end);
                self.sel_made_at = end;
                // As tmux's: where it was made, kept within the word.
                self.sel_origin = origin.min(end);
            }
            // How the selection grows as the cursor moves, as tmux's
            // window_copy_cmd_selection_mode.
            "selection-mode" => {
                self.selflag = match arg.map(str::to_ascii_lowercase).as_deref() {
                    None | Some("char" | "c") => SelFlag::Char,
                    Some("word" | "w") => SelFlag::Word,
                    Some("line" | "l") => SelFlag::Line,
                    Some(_) => self.selflag,
                };
            }
            "rectangle-toggle" => self.set_rect(!self.rect),
            "rectangle-on" => self.set_rect(true),
            "rectangle-off" => self.set_rect(false),
            "clear-selection" => {
                self.selflag = SelFlag::Char;
                self.anchor = None;
                self.sel_end = None;
                self.cx = self.cx.min(self.line_len(self.cy));
            }
            // tmux's: the cursor goes to the selection's other end, which is
            // where the selection is now held from. A count swaps that many
            // times.
            "other-end" => {
                self.selflag = SelFlag::Char;
                // After stop-selection the cursor goes to the selection's
                // end, which follows it again, as tmux's cursordrag none
                // becomes endsel.
                if let Some(end) = self.sel_end.take() {
                    (self.cy, self.cx) = end;
                } else if let Some(other) = self.anchor {
                    self.anchor = Some((self.cy, self.cx));
                    (self.cy, self.cx) = other;
                }
            }
            // The selection stays where it is and the cursor moves alone,
            // as tmux's window_copy_cmd_stop_selection.
            "stop-selection" => {
                self.selflag = SelFlag::Char;
                if self.anchor.is_some() && self.sel_end.is_none() {
                    self.sel_end = Some((self.cy, self.cx));
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
            // Run by `command-prompt -i` as the text is typed, as tmux's:
            // the first character is the prompt's prefix, `-` or `+` for
            // `C-r` / `C-s` turning the search up or down. A changed text
            // searches again from where the search started; the same one
            // moves on to the next match.
            "search-forward-incremental" | "search-backward-incremental" => {
                let mut chars = arg.unwrap_or_default().chars();
                let prefix = chars.next();
                let needle = chars.as_str();
                let repeat = self.search.as_ref().is_some_and(|s| s.needle == needle);
                match self.search_origin {
                    None => self.search_origin = Some((self.cy, self.cx, self.top)),
                    Some((y, x, top)) if !repeat && self.search.is_some() => {
                        (self.cy, self.cx, self.top) = (y, x, top);
                    }
                    Some(_) => {}
                }
                let forward = match prefix {
                    Some('+') => true,
                    Some('-') => false,
                    _ => name == "search-forward-incremental",
                };
                if needle.is_empty() {
                    self.search = None;
                } else {
                    // Up, the cursor's own place never counts; down, it
                    // does until the search moves on.
                    self.find(needle, forward, false, !forward || repeat);
                    self.search = Some(Search {
                        needle: needle.to_owned(),
                        forward,
                        regex: false,
                    });
                }
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
                let inherited = || {
                    let search = self.inherited_search.clone()?;
                    Some(Search {
                        forward: false,
                        ..search
                    })
                };
                if let Some(search) = self.search.clone().or_else(inherited) {
                    let forward = if name == "search-again" {
                        search.forward
                    } else {
                        !search.forward
                    };
                    self.find(&search.needle, forward, search.regex, true);
                    self.search = Some(search);
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
