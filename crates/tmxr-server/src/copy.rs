//! vi copy mode.
//!
//! Entering copy mode snapshots the pane's history and screen into a grid;
//! the program keeps running underneath while the user moves around the
//! frozen copy, as in tmux. Motions are tmux's `copy-mode-vi` commands
//! (`send-keys -X <name>`), so every key is an ordinary bind.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};

use crate::cmds::Ctx;
use crate::model::PaneId;
use crate::server::Server;

/// One captured cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub symbol: String,
    pub style: Style,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub cells: Vec<Cell>,
    /// The line continues on the next one (soft wrap): no newline between
    /// them when copied.
    pub wrapped: bool,
}

impl Line {
    fn chars(&self) -> Vec<char> {
        self.cells
            .iter()
            .map(|c| c.symbol.chars().next().unwrap_or(' '))
            .collect()
    }

    /// Text of columns `from..to`, trailing blanks removed.
    fn text(&self, from: usize, to: usize) -> String {
        let s: String = self
            .cells
            .iter()
            .enumerate()
            .filter(|(i, _)| *i >= from && *i < to)
            .map(|(_, c)| {
                if c.symbol.is_empty() {
                    ""
                } else {
                    c.symbol.as_str()
                }
            })
            .collect();
        s.trim_end().to_owned()
    }
}

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
    pub search: Option<(String, bool)>,
    /// Repeat count typed before a command (`5j`), 0 for none.
    pub count: usize,
    /// An `f` / `F` / `t` / `T` waiting for the character to jump to.
    pub pending_jump: Option<Jump>,
    /// The last jump and its character, for `;` and `,`.
    last_jump: Option<(Jump, char)>,
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

pub fn colour(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

pub fn cell_style(cell: &vt100::Cell) -> Style {
    let mut s = Style::default()
        .fg(colour(cell.fgcolor()))
        .bg(colour(cell.bgcolor()));
    let mut m = Modifier::empty();
    if cell.bold() {
        m |= Modifier::BOLD;
    }
    if cell.dim() {
        m |= Modifier::DIM;
    }
    if cell.italic() {
        m |= Modifier::ITALIC;
    }
    if cell.underline() {
        m |= Modifier::UNDERLINED;
    }
    if cell.inverse() {
        m |= Modifier::REVERSED;
    }
    s = s.add_modifier(m);
    s
}

/// Capture every line of history plus the visible screen.
fn snapshot(screen: &mut vt100::Screen) -> Vec<Line> {
    let (rows, cols) = screen.size();
    let original = screen.scrollback();
    screen.set_scrollback(usize::MAX);
    let history = screen.scrollback();
    let total = history + usize::from(rows);
    let mut lines: Vec<Option<Line>> = vec![None; total];
    let mut offset = history;
    loop {
        screen.set_scrollback(offset);
        for r in 0..rows {
            let idx = history - offset + usize::from(r);
            if idx >= total || lines[idx].is_some() {
                continue;
            }
            let cells = (0..cols)
                .map(|c| match screen.cell(r, c) {
                    Some(cell) => Cell {
                        symbol: if cell.is_wide_continuation() {
                            String::new()
                        } else if cell.has_contents() {
                            cell.contents().to_owned()
                        } else {
                            " ".to_owned()
                        },
                        style: cell_style(cell),
                    },
                    None => Cell {
                        symbol: " ".into(),
                        style: Style::default(),
                    },
                })
                .collect();
            lines[idx] = Some(Line {
                cells,
                wrapped: screen.row_wrapped(r),
            });
        }
        if offset == 0 {
            break;
        }
        offset = offset.saturating_sub(usize::from(rows));
    }
    screen.set_scrollback(original);
    lines
        .into_iter()
        .map(|l| {
            l.unwrap_or(Line {
                cells: Vec::new(),
                wrapped: false,
            })
        })
        .collect()
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
        }
    }

    /// Keys copy mode takes before its key table: the character a pending
    /// jump waits for (any other key cancels the wait) and count digits.
    /// Returns whether the key was used.
    pub fn take_key(&mut self, ev: &KeyEvent) -> bool {
        let plain = !ev
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
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
        match (ev.code, plain) {
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

    fn clamp_x(&mut self) {
        self.cx = self.cx.min(usize::from(self.cols.saturating_sub(1)));
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
        let c = self.char_at(y, x);
        if c.is_whitespace() {
            0
        } else if big || c.is_alphanumeric() || c == '_' {
            1
        } else {
            2
        }
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

    fn find(&mut self, needle: &str, forward: bool, skip_current: bool) -> bool {
        if needle.is_empty() {
            return false;
        }
        let n = self.lines.len();
        let needle_chars: Vec<char> = needle.chars().collect();
        let smart_case = needle.chars().any(char::is_uppercase);
        let eq = |a: char, b: char| {
            if smart_case {
                a == b
            } else {
                a.to_lowercase().eq(b.to_lowercase())
            }
        };
        let matches_at = |line: &[char], x: usize| {
            x + needle_chars.len() <= line.len()
                && needle_chars.iter().zip(&line[x..]).all(|(a, b)| eq(*a, *b))
        };
        for step in 0..=n {
            let y = if forward {
                (self.cy + step) % n
            } else {
                (self.cy + n - step % n) % n
            };
            let chars = self.lines[y].chars();
            let hits: Vec<usize> = (0..chars.len())
                .filter(|x| matches_at(&chars, *x))
                .collect();
            let pick = if forward {
                hits.into_iter()
                    .find(|x| step > 0 || *x > self.cx || (!skip_current && *x == self.cx))
            } else {
                hits.into_iter()
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
            "cursor-left" => self.cx = self.cx.saturating_sub(1),
            "cursor-right" => self.cx += 1,
            "cursor-up" => self.cy = self.cy.saturating_sub(1),
            "cursor-down" => self.cy = (self.cy + 1).min(last),
            "start-of-line" => self.cx = 0,
            "end-of-line" => self.cx = self.line_len(self.cy).saturating_sub(1),
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
            "halfpage-up" => {
                self.top = self.top.saturating_sub(rows / 2);
                self.cy = self.cy.saturating_sub(rows / 2);
            }
            "halfpage-down" => {
                self.top = (self.top + rows / 2).min(self.lines.len().saturating_sub(rows));
                self.cy = (self.cy + rows / 2).min(last);
            }
            "page-up" => {
                self.top = self.top.saturating_sub(rows);
                self.cy = self.cy.saturating_sub(rows);
            }
            "page-down" => {
                self.top = (self.top + rows).min(self.lines.len().saturating_sub(rows));
                self.cy = (self.cy + rows).min(last);
            }
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
                self.kind = SelKind::Char;
            }
            "select-line" => {
                self.anchor = Some((self.cy, 0));
                self.kind = SelKind::Line;
            }
            "rectangle-toggle" => {
                self.kind = if self.kind == SelKind::Rect {
                    SelKind::Char
                } else {
                    SelKind::Rect
                };
            }
            "clear-selection" => self.anchor = None,
            "search-forward" | "search-backward" => {
                let forward = name == "search-forward";
                let needle = arg.unwrap_or_default().to_owned();
                self.find(&needle, forward, true);
                self.search = Some((needle, forward));
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
                if let Some((needle, fwd)) = self.search.clone() {
                    let forward = if name == "search-again" { fwd } else { !fwd };
                    self.find(&needle, forward, true);
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

/// Enter copy mode in `pane` (no-op if already in it).
pub fn enter(srv: &mut Server, pane: PaneId, page_up: bool) {
    let Some(p) = srv.panes.get_mut(&pane) else {
        return;
    };
    if p.copy.is_none() {
        p.copy = Some(CopyMode::new(p.emu.screen_mut()));
    }
    if page_up && let Some(cm) = p.copy.as_mut() {
        cm.apply("page-up", None);
    }
    let w = p.window;
    srv.mark_window_dirty(w);
}

fn leave(srv: &mut Server, pane: PaneId) {
    if let Some(p) = srv.panes.get_mut(&pane) {
        p.copy = None;
        let w = p.window;
        srv.mark_window_dirty(w);
    }
}

/// `send-keys -X <name>` in a copy-mode pane.
pub fn command(
    srv: &mut Server,
    _ctx: &Ctx,
    pane: PaneId,
    name: &str,
    args: &[String],
) -> Result<(), String> {
    let Some(cm) = srv.panes.get_mut(&pane).and_then(|p| p.copy.as_mut()) else {
        return Err("not in a mode".into());
    };
    match name {
        "cancel" => leave(srv, pane),
        "copy-selection"
        | "copy-selection-and-cancel"
        | "copy-selection-no-newlines-and-cancel" => {
            let text = cm.selection_text();
            let cancel = name.ends_with("-and-cancel");
            if let Some(mut text) = text {
                if name.contains("no-newlines") {
                    text = text.replace(['\r', '\n'], "");
                }
                srv.set_clipboard(&text);
                srv.add_buffer(text, None);
            }
            if cancel {
                leave(srv, pane);
            } else if let Some(cm) = srv.panes.get_mut(&pane).and_then(|p| p.copy.as_mut()) {
                cm.anchor = None;
            }
        }
        other => {
            if !cm.apply_counted(other, args.first().map(String::as_str)) {
                return Err(format!("unknown copy-mode command: {other}"));
            }
        }
    }
    if let Some(w) = srv.panes.get(&pane).map(|p| p.window) {
        srv.mark_window_dirty(w);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(text: &str) -> CopyMode {
        let mut p = vt100::Parser::new(5, 20, 100);
        p.process(text.as_bytes());
        CopyMode::new(p.screen_mut())
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    /// Copy mode over `text` with the cursor at the start of its first line.
    fn at_start(text: &str) -> CopyMode {
        let mut cm = mode(text);
        (cm.cy, cm.cx) = (0, 0);
        cm
    }

    #[test]
    fn jumps_find_characters_on_the_line_and_repeat() {
        // x at 0, 2, 4, 6.
        let mut cm = at_start("x.x.x.x");
        cm.apply("jump-forward", Some("."));
        assert_eq!(cm.cx, 1);
        cm.apply("jump-again", None);
        assert_eq!(cm.cx, 3);
        cm.apply("jump-reverse", None);
        assert_eq!(cm.cx, 1);

        // t stops before the x; ; then steps past the one beside it.
        let mut cm = at_start("x.x.x.x");
        cm.apply("jump-to-forward", Some("x"));
        assert_eq!(cm.cx, 1);
        cm.apply("jump-again", None);
        assert_eq!(cm.cx, 3);

        // F / T go back.
        cm.cx = 6;
        cm.apply("jump-backward", Some("x"));
        assert_eq!(cm.cx, 4);
        cm.apply("jump-to-backward", Some("x"));
        assert_eq!(cm.cx, 3);
        // No such character: the cursor stays.
        cm.apply("jump-forward", Some("z"));
        assert_eq!(cm.cx, 3);
    }

    #[test]
    fn counts_repeat_commands_and_jumps_wait_for_their_character() {
        let mut cm = at_start("abcdefghijklmnop");
        // 0 alone is start-of-line, for the key table.
        assert!(!cm.take_key(&key('0')));
        assert!(cm.take_key(&key('1')));
        assert!(cm.take_key(&key('0')));
        assert_eq!(cm.count, 10);
        cm.apply_counted("cursor-right", None);
        assert_eq!((cm.cx, cm.count), (10, 0));

        // 2fx: the count waits with the jump for its character.
        let mut cm = at_start("x.x.x.x");
        cm.take_key(&key('2'));
        cm.apply_counted("jump-forward", None);
        assert_eq!(cm.pending_jump, Some(Jump::Forward));
        assert!(cm.take_key(&key('x')));
        assert_eq!((cm.cx, cm.pending_jump), (4, None));

        // Escape cancels a waiting jump.
        cm.apply_counted("jump-forward", None);
        assert!(cm.take_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert_eq!((cm.cx, cm.pending_jump), (4, None));
    }

    #[test]
    fn snapshot_includes_history_and_screen() {
        let lines: String = (0..12).map(|i| format!("line{i}\r\n")).collect();
        let cm = mode(&lines);
        let texts: Vec<String> = cm.lines.iter().map(|l| l.text(0, 20)).collect();
        assert_eq!(texts[0], "line0");
        assert_eq!(texts[11], "line11");
        assert_eq!(cm.position(), (0, cm.lines.len() - 5));
    }

    #[test]
    fn word_motions() {
        let mut cm = mode("foo bar.baz  qux");
        (cm.cy, cm.cx) = (0, 0);
        cm.apply("next-word", None);
        assert_eq!(cm.cx, 4);
        cm.apply("next-word", None);
        assert_eq!(cm.cx, 7, "punctuation is its own word");
        cm.apply("next-space", None);
        assert_eq!(cm.cx, 13, "WORD skips punctuation");
        cm.apply("previous-word", None);
        assert_eq!(cm.cx, 8);
        cm.apply("next-word-end", None);
        assert_eq!(cm.cx, 10);
        cm.apply("start-of-line", None);
        cm.apply("end-of-line", None);
        assert_eq!(cm.cx, 15);
    }

    #[test]
    fn selections_copy_char_line_and_rectangle_text() {
        let mut cm = mode("abcdef\r\nghijkl\r\nmnopqr");
        (cm.cy, cm.cx) = (0, 2);
        cm.apply("begin-selection", None);
        (cm.cy, cm.cx) = (1, 1);
        assert_eq!(cm.selection_text().unwrap(), "cdef\ngh");
        cm.apply("rectangle-toggle", None);
        assert_eq!(cm.selection_text().unwrap(), "bc\nhi");
        // select-line anchors on the cursor's line (1, after the move above).
        cm.apply("select-line", None);
        (cm.cy, cm.cx) = (2, 0);
        assert_eq!(cm.selection_text().unwrap(), "ghijkl\nmnopqr\n");
    }

    #[test]
    fn search_finds_forward_backward_and_wraps() {
        let mut cm = mode("one\r\ntwo\r\none two");
        (cm.cy, cm.cx) = (0, 0);
        cm.apply("search-forward", Some("two"));
        assert_eq!((cm.cy, cm.cx), (1, 0));
        cm.apply("search-again", None);
        assert_eq!((cm.cy, cm.cx), (2, 4));
        cm.apply("search-again", None);
        assert_eq!((cm.cy, cm.cx), (1, 0), "wraps around");
        cm.apply("search-reverse", None);
        assert_eq!((cm.cy, cm.cx), (2, 4));
    }
}
