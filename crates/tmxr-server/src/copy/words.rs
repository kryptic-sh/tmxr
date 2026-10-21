//! Copy mode's words: the classes of cells `word-separators` makes, and
//! the moves between words, as tmux's grid reader makes them.

use super::CopyMode;

/// Whether `c` ends a word: a blank, as tmux always counts, or one of
/// `separators` (`word-separators`).
pub(super) fn is_word_separator(c: char, separators: &str) -> bool {
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

impl CopyMode {
    /// Word class: 0 blank, 1 word char, 2 punctuation; `big` (WORD) makes
    /// every non-blank the same class.
    pub(super) fn class(&self, y: usize, x: usize, big: bool) -> u8 {
        // Blanks, word characters and separators, as tmux's
        // `window_copy_in_set` reads `word-separators`. A wide character's
        // padding is in neither set there, so it counts as a word character.
        if self.padding(y, x) {
            return 1;
        }
        let c = self.char_at(y, x);
        if c.is_whitespace() {
            0
        } else if big || !self.word_separators.contains(c) {
            1
        } else {
            2
        }
    }

    pub(super) fn next_pos(&self, y: usize, x: usize) -> Option<(usize, usize)> {
        if x + 1 < usize::from(self.cols) {
            Some((y, x + 1))
        } else if y < self.last_line() {
            Some((y + 1, 0))
        } else {
            None
        }
    }

    pub(super) fn prev_pos(&self, y: usize, x: usize) -> Option<(usize, usize)> {
        if x > 0 {
            Some((y, x - 1))
        } else if y > 0 {
            Some((y - 1, usize::from(self.cols.saturating_sub(1))))
        } else {
            None
        }
    }

    pub(super) fn next_word(&mut self, big: bool) {
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

    pub(super) fn word_end(&mut self, big: bool) {
        (self.cy, self.cx) = self.word_end_pos((self.cy, self.cx), big);
    }

    /// Where `next-word-end` from `(y, x)` lands.
    pub(super) fn word_end_pos(&self, (y, x): (usize, usize), big: bool) -> (usize, usize) {
        let (mut y, mut x) = (y, x);
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
        (y, x)
    }

    pub(super) fn prev_word(&mut self, big: bool) {
        (self.cy, self.cx) = self.prev_word_pos((self.cy, self.cx), big);
    }

    /// The cell before `(y, x)`, across the end of a wrapped line above, as
    /// tmux's grid reader steps.
    pub(super) fn cell_before(&self, (y, x): (usize, usize)) -> Option<(usize, usize)> {
        if x > 0 {
            return Some((y, x - 1));
        }
        let above = y.checked_sub(1)?;
        let line = self.lines.get(above).filter(|l| l.wrapped)?;
        Some((above, line.width().checked_sub(1)?))
    }

    /// The cell after `(y, x)`, on to the next line when this one wraps.
    pub(super) fn cell_after(&self, (y, x): (usize, usize)) -> Option<(usize, usize)> {
        let line = self.lines.get(y)?;
        if x + 1 < line.width() {
            return Some((y, x + 1));
        }
        (line.wrapped && y + 1 < self.lines.len()).then_some((y + 1, 0))
    }

    /// The first and last cells of the run of `(y, x)`'s class around it,
    /// across wrapped lines.
    pub(super) fn class_run(&self, at: (usize, usize)) -> ((usize, usize), (usize, usize)) {
        let class = self.class(at.0, at.1, false);
        let same = |p: &(usize, usize)| self.class(p.0, p.1, false) == class;
        let mut start = at;
        while let Some(p) = self.cell_before(start).filter(same) {
            start = p;
        }
        let mut end = at;
        while let Some(p) = self.cell_after(end).filter(same) {
            end = p;
        }
        (start, end)
    }

    /// The start of the word at `(y, x)`, or on a blank the previous word's,
    /// as tmux's grid_reader_cursor_previous_word without `already` (what
    /// word selection snaps back to), across wrapped lines.
    pub(super) fn word_start_pos(&self, (y, x): (usize, usize)) -> (usize, usize) {
        if self.class(y, x, false) == 0 {
            return self.prev_word_pos((y, x), false);
        }
        self.class_run((y, x)).0
    }

    /// Where `previous-word` from `(y, x)` lands.
    pub(super) fn prev_word_pos(&self, (y, x): (usize, usize), big: bool) -> (usize, usize) {
        let (mut y, mut x) = (y, x);
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
        (y, x)
    }
}
