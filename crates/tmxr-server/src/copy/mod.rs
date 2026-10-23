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
pub use model::{
    Cell, Line, Underline, cell_style, colour, link_of, snapshot, snapshot_lines,
    snapshot_normal_rows,
};
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

/// tmux's lineflag: after `select-line` / `select-word` a move up or down
/// puts the cursor at its line's end (`LeftRight`), or its start once
/// `other-end` has turned the selection round (`RightLeft`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineFlag {
    None,
    LeftRight,
    RightLeft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelKind {
    Char,
    Line,
    Rect,
}

/// The brackets `next-matching-bracket` / `previous-matching-bracket`
/// pair, each opening one at the index of its closing one, as tmux's.
const OPEN_BRACKETS: &str = "{[(";
const CLOSE_BRACKETS: &str = "}])";

#[derive(Debug, Clone)]
pub struct CopyMode {
    /// The pane whose screen and history this shows, when not its own
    /// (`copy-mode -s`): what `refresh-from-pane` reads again.
    pub source: Option<crate::model::PaneId>,
    pub lines: Vec<Line>,
    /// Lines the shell marked as starting a prompt (OSC 133 A) and an
    /// output (C), for `next-prompt` / `previous-prompt` (`-o`).
    pub prompts: Vec<usize>,
    pub outputs: Vec<usize>,
    /// When each line went into the pane's history, as
    /// [`tmxr_term::Emulator::line_time`]; lines past its end have none.
    pub times: Vec<u64>,
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
    /// `other-end` has put the cursor on the end the selection began at,
    /// so that is the other end ([`Self::selection_other_end`]) and the
    /// anchor is where it ends, as tmux's cursordrag sel.
    pub drags_start: bool,
    pub kind: SelKind,
    pub rows: u16,
    pub cols: u16,
    pub search: Option<Search>,
    /// The last search's matches are marked and highlighted, as tmux's
    /// searchmark (`#{search_present}`); the commands tmux's table says
    /// clear them do ([`clears_marks`]).
    pub marks: bool,
    /// How many matches the last search had across every line
    /// (`#{search_count}`), kept when the marks are cleared.
    pub search_count: usize,
    /// `mode-keys` is emacs, under which more commands clear the marks.
    pub emacs: bool,
    /// The last search went down, as tmux's searchdirection.
    pub search_forward: bool,
    /// No search has run in this copy mode yet, so the next marks and
    /// counts every match whatever searched before (tmux's searchall).
    search_all: bool,
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
    lineflag: LineFlag,
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
    pub fn new(screen: &mut vt1k::Screen) -> Self {
        let (rows, cols) = tmxr_term::screen_size(screen);
        let lines = snapshot(screen);
        let (crow, ccol) = screen.cursor_position();
        let top = lines.len().saturating_sub(usize::from(rows));
        Self {
            top,
            cy: top + usize::from(crow),
            cx: usize::from(ccol),
            lines,
            prompts: Vec::new(),
            outputs: Vec::new(),
            times: Vec::new(),
            anchor: None,
            sel_end: None,
            drags_start: false,
            kind: SelKind::Char,
            rows,
            cols,
            search: None,
            marks: false,
            search_count: 0,
            emacs: false,
            search_forward: false,
            search_all: true,
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
            lineflag: LineFlag::None,
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
        self.drags_start = old.drags_start;
        self.lineflag = old.lineflag;
        self.kind = old.kind;
        self.search.clone_from(&old.search);
        self.search_count = old.search_count;
        self.inherited_search.clone_from(&old.inherited_search);
        self.unseen_changes = old.unseen_changes;
        self.mark = old.mark.map(clamp);
        self.last_jump = old.last_jump;
        self.scroll_exit = old.scroll_exit;
        self.hide_position = old.hide_position;
        self.source = old.source;
        self.scroll_to_cursor();
    }

    /// The character at `(y, x)` if it is one byte, as tmux's bracket
    /// matching reads cells.
    fn byte_at(&self, (y, x): (usize, usize)) -> Option<char> {
        let c = self.lines.get(y)?.cell(x)?.symbol.chars().next()?;
        c.is_ascii().then_some(c)
    }

    /// `previous-matching-bracket`, as tmux's
    /// window_copy_cmd_previous_matching_bracket: from a closing bracket
    /// back to the bracket opening it, over each line's text. Under emacs
    /// keys a closing bracket just before the cursor counts, and off one
    /// the cursor goes back a word, with the closing brackets as its
    /// separators.
    fn previous_matching_bracket(&mut self) {
        let (mut py, mut px) = (self.cy, self.cx);
        if self.line_len(py) == 0 {
            return;
        }
        let mut tried = false;
        let found = loop {
            match self
                .byte_at((py, px))
                .filter(|c| CLOSE_BRACKETS.contains(*c))
            {
                Some(c) => break c,
                None if self.emacs && !tried && px > 0 => {
                    px -= 1;
                    tried = true;
                }
                None => {
                    if self.emacs {
                        (self.cy, self.cx) = self.reader_previous_word(
                            (self.cy, self.cx),
                            CLOSE_BRACKETS,
                            true,
                            true,
                        );
                    }
                    return;
                }
            }
        };
        let start = OPEN_BRACKETS.as_bytes()[CLOSE_BRACKETS.find(found).unwrap_or(0)] as char;
        let mut depth = 1;
        while depth != 0 {
            if px == 0 {
                if py == 0 {
                    return;
                }
                let mut len;
                loop {
                    py -= 1;
                    len = self.line_len(py);
                    if len != 0 || py == 0 {
                        break;
                    }
                }
                if len == 0 {
                    return;
                }
                px = len - 1;
            } else {
                px -= 1;
            }
            match self.byte_at((py, px)) {
                Some(c) if c == found => depth += 1,
                Some(c) if c == start => depth -= 1,
                _ => {}
            }
        }
        self.scroll_to((py, px));
    }

    /// `next-matching-bracket`, as tmux's
    /// window_copy_cmd_next_matching_bracket: from an opening bracket on to
    /// the bracket closing it. Under vi keys the first bracket on from the
    /// cursor (across wrapped lines) is taken, a closing one matched
    /// backwards; under emacs keys an opening bracket just after the cursor
    /// counts, and off one the cursor goes to a word's end, with the
    /// opening brackets as its separators.
    fn next_matching_bracket(&mut self) {
        let (mut py, mut px) = (self.cy, self.cx);
        let mut len = self.line_len(py);
        let last = self.last_line();
        if len == 0 {
            return;
        }
        let mut tried = false;
        let found = loop {
            let c = self.byte_at((py, px));
            if !self.emacs && c.is_some_and(|c| CLOSE_BRACKETS.contains(c)) {
                let back = (self.cy, self.cx);
                self.scroll_to((py, px));
                self.previous_matching_bracket();
                if self
                    .byte_at((self.cy, self.cx))
                    .is_some_and(|c| CLOSE_BRACKETS.contains(c))
                {
                    self.scroll_to(back);
                }
                return;
            }
            if let Some(c) = c.filter(|c| OPEN_BRACKETS.contains(*c)) {
                break c;
            }
            if self.emacs {
                if !tried && px <= len {
                    px += 1;
                    tried = true;
                    continue;
                }
                (self.cy, self.cx) = self.reader_next_word_end((self.cy, self.cx), OPEN_BRACKETS);
                return;
            }
            // Under vi keys on along the line, and on into the next when
            // this one wraps.
            if px > len {
                if py == last || !self.lines[py].wrapped {
                    return;
                }
                (py, px) = (py + 1, 0);
                len = self.line_len(py);
            } else {
                px += 1;
            }
        };
        let end = CLOSE_BRACKETS.as_bytes()[OPEN_BRACKETS.find(found).unwrap_or(0)] as char;
        let mut depth = 1;
        while depth != 0 {
            if px > len {
                if py == last {
                    return;
                }
                (py, px) = (py + 1, 0);
                len = self.line_len(py);
            } else {
                px += 1;
            }
            match self.byte_at((py, px)) {
                Some(c) if c == found => depth += 1,
                Some(c) if c == end => depth -= 1,
                _ => {}
            }
        }
        self.scroll_to((py, px));
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
        // A count selects that many lines down, as tmux's
        // window_copy_cmd_select_line.
        if name == "select-line" {
            self.apply(name, arg);
            for _ in 1..times {
                self.vertical(true);
                self.end_of_line();
            }
            self.scroll_to_cursor();
            return true;
        }
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
        // A new width rewraps the lines copy mode shows, as tmux's
        // window_copy_resize does its copy: the cursor keeps its place in
        // its line, the view goes to the bottom unless that would hide the
        // cursor, which is then its top row, and the selection goes.
        let width = usize::from(cols.max(1));
        if self.lines.first().is_some_and(|l| l.width() != width) {
            let (lines, map, (cy, cx)) = model::reflow(
                &self.lines,
                width,
                usize::from(rows.max(1)),
                (self.cy, self.cx),
            );
            let follow = |marks: &[usize]| -> Vec<usize> {
                let mut moved: Vec<usize> =
                    marks.iter().filter_map(|&m| map.get(m).copied()).collect();
                moved.dedup();
                moved
            };
            self.prompts = follow(&self.prompts);
            self.outputs = follow(&self.outputs);
            // A line's time goes to its first row, as tmux's grid_reflow:
            // rows joined onto it keep its time, rows split off it have none.
            let mut times = vec![0; lines.len()];
            for (old, &time) in self.times.iter().enumerate() {
                let new = map.get(old).copied();
                if (old == 0 || map.get(old - 1).copied() != new)
                    && let Some(slot) = new.and_then(|new| times.get_mut(new))
                {
                    *slot = time;
                }
            }
            self.times = times;
            self.lines = lines;
            (self.cy, self.cx) = (cy, cx);
            let bottom = self.lines.len().saturating_sub(usize::from(rows.max(1)));
            self.top = cy.min(bottom);
            self.anchor = None;
            self.sel_end = None;
            self.lineflag = LineFlag::None;
            self.search_origin = None;
        }
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
    /// cursor, or on a separator the word just after it, followed across
    /// the lines the cursor's line wraps from and into.
    pub fn cursor_word(&self) -> String {
        if self.cy >= self.lines.len() {
            return String::new();
        }
        let mut first = self.cy;
        while first > 0 && self.lines[first - 1].wrapped {
            first -= 1;
        }
        let mut last = self.cy;
        while self.lines[last].wrapped && last + 1 < self.lines.len() {
            last += 1;
        }
        let before: usize = self.lines[first..self.cy].iter().map(Line::width).sum();
        let text: Vec<char> = self.lines[first..=last]
            .iter()
            .flat_map(Line::chars)
            .collect();
        word_at(&text, before + self.cx, &self.word_separators)
    }

    /// The OSC 8 link of the cell shown at `col`, `row` rows down the view,
    /// as the pane's `Screen::hyperlink` numbers it.
    pub fn view_link(&self, row: usize, col: usize) -> Option<std::num::NonZeroU16> {
        link_of(&self.lines.get(self.top + row)?.cell(col)?.style)
    }

    /// Where the selection began and where it ends, as tmux's selx / sely
    /// and endselx / endsely: `other-end` moves the cursor between them
    /// without swapping them.
    pub fn selection_bounds(&self) -> Option<((usize, usize), (usize, usize))> {
        let (anchor, other) = self.selection_ends()?;
        Some(if self.drags_start && !self.snapping() {
            (other, anchor)
        } else {
            (anchor, other)
        })
    }

    /// The selection grows by words or lines from where it was made, so its
    /// ends are tmux's whichever end the cursor drags.
    fn snapping(&self) -> bool {
        self.selflag != SelFlag::Char
            && self.sel_end.is_none()
            && self.selection_end() != self.sel_made_at
    }

    /// Where `set-mark` put the mark, which tmux shows from then on.
    pub fn mark_at(&self) -> Option<(usize, usize)> {
        self.mark
    }

    /// `#{top_line_time}`: when the view's top line went into the
    /// history, 0 for a line of the screen.
    pub fn top_line_time(&self) -> u64 {
        self.times.get(self.top).copied().unwrap_or(0)
    }

    /// `#{rectangle_toggle}`: selections are rectangles.
    pub fn rectangle(&self) -> bool {
        self.rect
    }

    /// `#{copy_cursor_line}`: the text of the cursor's line as shown, as
    /// tmux's format_grid_line.
    pub fn cursor_line(&self) -> String {
        self.lines
            .get(self.cy)
            .map(|l| l.text(0, l.width()))
            .unwrap_or_default()
    }

    /// The link of the cell under the cursor, or of the wide character it
    /// is the right half of, as tmux's format_grid_hyperlink.
    pub fn cursor_link(&self) -> Option<std::num::NonZeroU16> {
        let line = self.lines.get(self.cy)?;
        let mut x = self.cx;
        while x > 0 && line.cell(x).is_some_and(|c| c.symbol.is_empty()) {
            x -= 1;
        }
        link_of(&line.cell(x)?.style)
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
            l.cells()
                .rposition(|c| !c.symbol.trim().is_empty())
                .map_or(0, |i| i + 1)
        })
    }

    /// To the start of the next (or previous) line the shell marked as a
    /// prompt's, or with `output` an output's, as tmux's
    /// window_copy_cursor_prompt: in history it comes to the top of the
    /// view; on the screen the view shows the screen.
    fn jump_to_prompt(&mut self, down: bool, output: bool) {
        let marks = if output { &self.outputs } else { &self.prompts };
        let found = if down {
            marks.iter().find(|&&l| l > self.cy)
        } else {
            marks.iter().rev().find(|&&l| l < self.cy)
        };
        let Some(&line) = found else {
            return;
        };
        let history = self.lines.len().saturating_sub(usize::from(self.rows));
        (self.cy, self.cx) = (line, 0);
        self.top = if line > history { history } else { line };
    }

    /// The cursor to `(y, x)`, as tmux's window_copy_scroll_to: the view
    /// stays if it shows line `y`, else moves to show it a quarter of the
    /// screen up from the bottom, or to the history's top or bottom when
    /// `y` is that near one.
    pub(super) fn scroll_to(&mut self, (y, x): (usize, usize)) {
        (self.cy, self.cx) = (y, x);
        let rows = usize::from(self.rows.max(1));
        if (self.top..self.top + rows).contains(&y) {
            return;
        }
        let bottom = self.lines.len().saturating_sub(rows);
        let gap = rows / 4;
        self.top = if y < rows {
            0
        } else if y > bottom + rows - gap {
            bottom
        } else {
            y + gap - rows
        };
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
        self.lineflag = LineFlag::None;
        self.kind = if on { SelKind::Rect } else { SelKind::Char };
        self.cx = self.cx.min(self.line_len(self.cy));
    }

    /// The cell is the right half of a wide character (tmux's padding).
    fn padding(&self, y: usize, x: usize) -> bool {
        self.lines
            .get(y)
            .and_then(|l| l.cell(x))
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
        // Leaving a line selection's first line upwards, or its last
        // downwards, turns it round, as tmux's cursor_up / _down.
        self.turn_line_selection(if down {
            LineFlag::RightLeft
        } else {
            LineFlag::LeftRight
        });
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
        match self.lineflag {
            LineFlag::LeftRight if self.rect => self.cx = usize::from(self.cols),
            LineFlag::LeftRight => self.cx = self.line_len(self.cy),
            LineFlag::RightLeft => self.cx = 0,
            LineFlag::None => {}
        }
    }

    /// Turn the selection round (`other_end`) when the line flag is `flag`
    /// and the cursor is on the selection's first line (`LeftRight`) or its
    /// last (`RightLeft`).
    fn turn_line_selection(&mut self, flag: LineFlag) {
        if self.lineflag != flag {
            return;
        }
        let Some((start, end)) = self.selection_bounds() else {
            return;
        };
        let edge = if flag == LineFlag::LeftRight {
            start
        } else {
            end
        };
        if self.cy == edge.0 {
            self.other_end();
        }
    }

    /// The cursor to the selection's other end, as tmux's
    /// window_copy_other_end: after stop-selection to the selection's end,
    /// which follows it again (cursordrag none becomes endsel); else from
    /// dragging one end to dragging the other. A line selection turns
    /// round with it.
    fn other_end(&mut self) {
        if self.anchor.is_none() && self.lineflag == LineFlag::None {
            return;
        }
        self.lineflag = match self.lineflag {
            LineFlag::LeftRight => LineFlag::RightLeft,
            LineFlag::RightLeft => LineFlag::LeftRight,
            LineFlag::None => LineFlag::None,
        };
        if let Some(stopped) = self.sel_end.take() {
            match self.anchor {
                Some(end) if self.drags_start => {
                    self.anchor = Some(stopped);
                    (self.cy, self.cx) = end;
                }
                _ => (self.cy, self.cx) = stopped,
            }
            self.drags_start = false;
        } else if let Some((start, end)) = self.selection_bounds() {
            // The end the cursor drags keeps where it was snapped to.
            self.drags_start = !self.drags_start;
            let (to, held) = if self.drags_start {
                (start, end)
            } else {
                (end, start)
            };
            self.anchor = Some(held);
            (self.cy, self.cx) = to;
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
            .and_then(|l| l.cell(x))
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
        // As tmux's window_copy_next_paragraph / _previous_paragraph:
        // forward at the line's end (a blank line's start).
        let x = if forward { self.line_len(y) } else { 0 };
        self.scroll_to((y, x));
    }

    /// The selection's moving end: the cursor, unless `stop-selection`
    /// fixed it.
    fn selection_end(&self) -> (usize, usize) {
        self.sel_end.unwrap_or((self.cy, self.cx))
    }

    /// The selection's two ends, its anchor first: in word or line mode
    /// snapped as tmux's window_copy_synchronize_cursor_end, the first word
    /// or line kept on whichever side the cursor is not, and in tmux's
    /// order (where it began, then where it ends).
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
            SelFlag::Word if backwards => (self.word_start_pos(cursor), end),
            SelFlag::Word => {
                let (y, x) = cursor;
                let snapped = if x >= self.line_len(y) || self.class(y, x + 1, false) != 0 {
                    self.word_end_pos(cursor, false)
                } else {
                    cursor
                };
                (start, snapped)
            }
            SelFlag::Line if self.sel_origin.0 > cursor.0 => ((cursor.0, 0), end),
            SelFlag::Line => {
                let y = cursor.0.max(end.0);
                (start, (y, self.line_len(y)))
            }
        })
    }

    /// Selected text, as tmux's window_copy_get_selection makes it, or
    /// `None` without a selection or with nothing in it. Under vi keys the
    /// cell at the selection's end is copied, and a selection reaching past
    /// its last line's text keeps that line's newline; under emacs keys
    /// neither. A rectangle is the columns between its other end and the
    /// cursor.
    pub fn selection_text(&self) -> Option<String> {
        let (start, end) = self.selection_bounds()?;
        let ((sy, sx), (ey, ex)) = if end < start {
            (end, start)
        } else {
            (start, end)
        };
        let ey_last = self.line_len(ey);
        let ex = ex.min(ey_last);
        let inclusive = usize::from(!self.emacs);
        let (firstsx, restsx, lastex, restex) = if self.kind == SelKind::Rect {
            // The end the cursor is not dragging, as tmux's cursordrag
            // picks it.
            let held = if self.sel_end.is_some() || self.drags_start {
                end.1
            } else {
                start.1
            };
            if held < self.cx {
                let ex = self.cx + inclusive;
                (held, held, ex, ex)
            } else {
                (self.cx, self.cx, held + 1, held + 1)
            }
        } else {
            (sx, 0, ex + inclusive, usize::from(self.cols))
        };
        let mut out = String::new();
        for y in sy..=ey {
            let (from, to) = (
                if y == sy { firstsx } else { restsx },
                if y == ey { lastex } else { restex },
            );
            self.copy_line(&mut out, y, from, to);
        }
        if out.is_empty() {
            return None;
        }
        if (self.emacs || lastex <= ey_last)
            && (!self.lines.get(ey).is_some_and(|l| l.wrapped) || lastex != ey_last)
        {
            out.pop();
        }
        Some(out)
    }

    /// Columns `sx..ex` of line `y` and its newline, as tmux's
    /// window_copy_copy_line: up to the line's text (all of a wrapped
    /// line's cells), and no newline where a wrapped line is copied to its
    /// end.
    fn copy_line(&self, out: &mut String, y: usize, sx: usize, ex: usize) {
        let Some(line) = self.lines.get(y) else {
            return;
        };
        if sx > ex {
            return;
        }
        let len = if line.wrapped {
            line.used
        } else {
            self.line_len(y)
        };
        let (sx, ex) = (sx.min(len), ex.min(len));
        for cell in line.cells().take(ex).skip(sx) {
            out.push_str(&cell.symbol);
        }
        if !line.wrapped || ex != len {
            out.push('\n');
        }
    }

    /// Whether `(y, x)` is drawn as selected, as tmux's
    /// screen_check_selection: between where the selection began and where
    /// it ends, whole lines between them; under emacs keys without the
    /// cell at its bottom-right end, and a selection upwards not on its
    /// first line when it began at that line's start. A rectangle takes the
    /// columns between its ends on each of its lines.
    pub fn selected(&self, y: usize, x: usize) -> bool {
        let Some(((sy, sx), (ey, ex))) = self.selection_bounds() else {
            return false;
        };
        // The last column of an end at column `at`.
        let last = |at: usize| {
            if self.emacs {
                at.checked_sub(1)
            } else {
                Some(at)
            }
        };
        if self.kind == SelKind::Rect {
            return (sy.min(ey)..=sy.max(ey)).contains(&y)
                && (sx.min(ex)..=sx.max(ex)).contains(&x);
        }
        match sy.cmp(&ey) {
            std::cmp::Ordering::Less => {
                (sy..=ey).contains(&y)
                    && (y != sy || x >= sx)
                    && (y != ey || x <= last(ex).unwrap_or(0))
            }
            std::cmp::Ordering::Greater => {
                (ey..=sy).contains(&y)
                    && (y != ey || x >= ex)
                    && (y != sy || (sx != 0 && last(sx).is_some_and(|l| x <= l)))
            }
            std::cmp::Ordering::Equal if ex < sx => {
                y == sy && x >= ex && last(sx).is_some_and(|l| x <= l)
            }
            std::cmp::Ordering::Equal => y == sy && x >= sx && x <= last(ex).unwrap_or(0),
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
            "next-prompt" | "previous-prompt" => {
                self.jump_to_prompt(name == "next-prompt", arg == Some("-o"));
            }
            "history-top" => {
                self.turn_line_selection(LineFlag::LeftRight);
                (self.cy, self.cx) = (0, 0);
            }
            // The last line, at its end, as tmux's window_copy_cmd_history_bottom.
            "history-bottom" => {
                self.turn_line_selection(LineFlag::RightLeft);
                (self.cy, self.cx) = (last, self.line_len(last));
            }
            // To the view's first, middle or last row, at its start, as
            // tmux's window_copy_cmd_top_line and the rest.
            "top-line" => (self.cy, self.cx) = (self.top, 0),
            "middle-line" => (self.cy, self.cx) = ((self.top + (rows - 1) / 2).min(last), 0),
            "bottom-line" => (self.cy, self.cx) = ((self.top + rows - 1).min(last), 0),
            "halfpage-up" => self.page(false, true),
            "halfpage-down" => self.page(true, true),
            "page-up" => self.page(false, false),
            "page-down" => self.page(true, false),
            // The view a line up or down, the cursor keeping its row on the
            // screen, so moving a line as cursor-up / -down move it, as
            // tmux's window_copy_cursor_up / _down with scroll_only.
            "scroll-up" => {
                if self.top > 0 {
                    self.top -= 1;
                    self.vertical(false);
                }
            }
            "scroll-down" => {
                if self.top < self.lines.len().saturating_sub(rows) {
                    self.top += 1;
                    self.vertical(true);
                }
            }
            "begin-selection" => {
                self.selflag = SelFlag::Char;
                self.lineflag = LineFlag::None;
                self.anchor = Some((self.cy, self.cx));
                self.sel_end = None;
                self.drags_start = false;
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
                self.lineflag = LineFlag::LeftRight;
                self.sel_origin = (self.cy, self.cx);
                self.anchor = Some((self.cy, 0));
                self.sel_end = None;
                self.drags_start = false;
                self.kind = SelKind::Line;
                self.end_of_line();
                self.sel_reset = ((self.cy, 0), (self.cy, self.line_len(self.cy)));
                self.sel_made_at = (self.cy, self.cx);
            }
            "select-word" => {
                // From the word's start to where next-word-end goes from
                // there, or a one-character word alone, as tmux's
                // window_copy_cmd_select_word; not a rectangle.
                self.rect = false;
                self.lineflag = LineFlag::LeftRight;
                let origin = (self.cy, self.cx);
                let start =
                    self.reader_previous_word(origin, &self.word_separators, false, self.emacs);
                let lone = start.1 < self.line_len(start.0)
                    && self
                        .cell_after(start)
                        .is_some_and(|next| self.blank_at(next));
                let end = if lone {
                    start
                } else {
                    self.word_end_pos(start, false)
                };
                self.selflag = SelFlag::Word;
                self.anchor = Some(start);
                self.sel_end = None;
                self.drags_start = false;
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
                self.lineflag = LineFlag::None;
                self.anchor = None;
                self.sel_end = None;
                self.cx = self.cx.min(self.line_len(self.cy));
            }
            // tmux's: the cursor goes to the selection's other end, which is
            // where the selection is now held from. A count swaps that many
            // times.
            // tmux's selection is snapped as it goes, so its other end is
            // where words or lines put it before the selection grows by
            // characters again; tmxr snaps when asked, so moves first.
            "other-end" => {
                self.other_end();
                self.selflag = SelFlag::Char;
            }
            // The selection stays where it is and the cursor moves alone,
            // as tmux's window_copy_cmd_stop_selection.
            "stop-selection" => {
                // The ends stay where they are, snapped as they were; the
                // anchor is where it began, so other-end goes to its end.
                if self.sel_end.is_none()
                    && let Some((start, end)) = self.selection_bounds()
                {
                    (self.anchor, self.sel_end) = (Some(start), Some(end));
                    self.drags_start = false;
                }
                self.selflag = SelFlag::Char;
                self.lineflag = LineFlag::None;
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
                let found = self.find(&needle, forward, regex, true);
                let previous = self.search.replace(Search {
                    needle,
                    forward,
                    regex,
                });
                self.mark_matches(found, previous.as_ref());
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
                // `=` is the command's own way; any other prefix searches
                // nothing, as tmux's.
                let forward = match prefix {
                    Some('+') => Some(true),
                    Some('-') => Some(false),
                    Some('=') => Some(name == "search-forward-incremental"),
                    _ => None,
                };
                if needle.is_empty() {
                    self.search = None;
                    self.marks = false;
                } else if let Some(forward) = forward {
                    // Up, the cursor's own place never counts; down, it
                    // does until the search moves on.
                    let found = self.find(needle, forward, false, !forward || repeat);
                    let previous = self.search.replace(Search {
                        needle: needle.to_owned(),
                        forward,
                        regex: false,
                    });
                    self.mark_matches(found, previous.as_ref());
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
            // To the mark, which goes where the cursor was; the mark's line
            // tops the view if it is in the history, else the view is at
            // the bottom, as tmux's window_copy_jump_to_mark.
            "jump-to-mark" => {
                if let Some((y, x)) = self.mark.replace((self.cy, self.cx)) {
                    let bottom = self.lines.len().saturating_sub(rows);
                    (self.cy, self.cx) = (y, x);
                    self.top = y.min(bottom);
                }
            }
            "next-matching-bracket" => self.next_matching_bracket(),
            "previous-matching-bracket" => self.previous_matching_bracket(),
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
                    let found = self.find(&search.needle, forward, search.regex, true);
                    let previous = self.search.replace(search);
                    self.mark_matches(found, previous.as_ref());
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

/// Whether copy-mode command `name` clears a search's marks, as tmux's
/// window_copy_cmd_table says: searches never do, nor the commands that
/// copy without clearing or only hide the position; moves keep them under
/// vi keys (`emacs` false); everything else clears them.
pub fn clears_marks(name: &str, emacs: bool) -> bool {
    const NEVER: &[&str] = &[
        "copy-pipe-no-clear",
        "copy-selection-no-clear",
        "pipe-no-clear",
        "toggle-position",
    ];
    const EMACS_ONLY: &[&str] = &[
        "bottom-line",
        "cursor-centre-horizontal",
        "cursor-centre-vertical",
        "cursor-down",
        "cursor-left",
        "cursor-right",
        "cursor-up",
        "end-of-line",
        "goto-line",
        "halfpage-down",
        "halfpage-up",
        "history-bottom",
        "history-top",
        "jump-again",
        "jump-backward",
        "jump-forward",
        "jump-reverse",
        "jump-to-backward",
        "jump-to-forward",
        "middle-line",
        "next-paragraph",
        "next-space",
        "next-space-end",
        "next-word",
        "next-word-end",
        "other-end",
        "page-down",
        "page-up",
        "previous-paragraph",
        "previous-space",
        "previous-word",
        "scroll-down",
        "scroll-up",
        "start-of-line",
        "top-line",
    ];
    if name.starts_with("search-") || NEVER.contains(&name) {
        return false;
    }
    emacs || !EMACS_ONLY.contains(&name)
}

#[cfg(test)]
mod tests;
