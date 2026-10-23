//! Copy mode's words: the classes of cells `word-separators` makes, and
//! the moves between words, as tmux's grid reader makes them.

use super::CopyMode;

/// The blanks tmux's grid reader steps over (tmux's WHITESPACE).
const WHITESPACE: &str = "\t ";

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

    /// Where `next-word-end` from `(y, x)` lands, as tmux's
    /// window_copy_cursor_next_word_end_pos: under vi keys on the word's
    /// last character, under emacs keys just past it. `big` takes only
    /// blanks as separators (`next-space-end`).
    pub(super) fn word_end_pos(&self, at: (usize, usize), big: bool) -> (usize, usize) {
        let separators = if big {
            ""
        } else {
            self.word_separators.as_str()
        };
        if self.emacs {
            return self.reader_next_word_end(at, separators);
        }
        let mut at = at;
        if !self.in_set(at, WHITESPACE) {
            at = self.reader_right(at);
        }
        self.reader_left(self.reader_next_word_end(at, separators))
    }

    /// Whether `(y, x)` holds a blank, as tmux's WHITESPACE.
    pub(super) fn blank_at(&self, at: (usize, usize)) -> bool {
        self.in_set(at, WHITESPACE)
    }

    /// Whether the character at `(y, x)` is one of `set`, as tmux's
    /// grid_in_set: a wide character's padding never is.
    fn in_set(&self, (y, x): (usize, usize), set: &str) -> bool {
        !self.padding(y, x) && set.contains(self.char_at(y, x))
    }

    /// The last column a grid reader stays on in line `y`, as tmux's
    /// grid_reader_cursor_next_word_end bounds it: the screen's last for
    /// a wrapped line, else one past the text.
    fn reader_bound(&self, y: usize) -> usize {
        match self.lines.get(y) {
            Some(line) if line.wrapped => line.width().saturating_sub(1),
            _ => self.line_len(y),
        }
    }

    /// tmux's grid_reader_cursor_next_word_end: past blanks, then past a
    /// run of `separators` or of other characters, onto the cell after it,
    /// across line ends up to the last line.
    pub(super) fn reader_next_word_end(
        &self,
        (y, x): (usize, usize),
        separators: &str,
    ) -> (usize, usize) {
        let (mut y, mut x) = (y, x);
        let mut bound = self.reader_bound(y);
        // tmux's grid_reader_handle_wrap: on to the next line's start past
        // this one's bound; false at the last line.
        let mut wrap = |y: &mut usize, x: &mut usize| {
            while *x > bound {
                if *y >= self.last_line() {
                    return false;
                }
                (*y, *x) = (*y + 1, 0);
                bound = self.reader_bound(*y);
            }
            true
        };
        while wrap(&mut y, &mut x) {
            if self.in_set((y, x), WHITESPACE) {
                x += 1;
            } else if self.in_set((y, x), separators) {
                loop {
                    x += 1;
                    if !(wrap(&mut y, &mut x)
                        && self.in_set((y, x), separators)
                        && !self.in_set((y, x), WHITESPACE))
                    {
                        break;
                    }
                }
                break;
            } else {
                loop {
                    x += 1;
                    if !(wrap(&mut y, &mut x)
                        && !self.in_set((y, x), WHITESPACE)
                        && !self.in_set((y, x), separators))
                    {
                        break;
                    }
                }
                break;
            }
        }
        (y, x)
    }

    /// tmux's grid_reader_cursor_right without wrapping: a cell right, past
    /// a wide character's padding, not beyond the line's text.
    fn reader_right(&self, (y, x): (usize, usize)) -> (usize, usize) {
        let end = self.line_len(y);
        let mut x = x;
        if x < end {
            x += 1;
            while x < end && self.padding(y, x) {
                x += 1;
            }
        }
        (y, x)
    }

    /// tmux's grid_reader_cursor_left with wrapping: a cell left, back
    /// over padding, and from a line's start to one past the text of the
    /// line above.
    fn reader_left(&self, (y, x): (usize, usize)) -> (usize, usize) {
        let mut x = x;
        while x > 0 && self.padding(y, x) {
            x -= 1;
        }
        if x == 0 && y > 0 {
            (y - 1, self.line_len(y - 1))
        } else {
            (y, x.saturating_sub(1))
        }
    }

    /// `previous-word` / `previous-space` (`big`): to the start of the
    /// word before the cursor, as tmux's window_copy_cursor_previous_word,
    /// stopping at a line's end under emacs keys.
    pub(super) fn prev_word(&mut self, big: bool) {
        let separators = if big {
            ""
        } else {
            self.word_separators.as_str()
        };
        (self.cy, self.cx) =
            self.reader_previous_word((self.cy, self.cx), separators, true, self.emacs);
    }

    /// The cell after `(y, x)`, on to the next line when this one wraps.
    pub(super) fn cell_after(&self, (y, x): (usize, usize)) -> Option<(usize, usize)> {
        let line = self.lines.get(y)?;
        if x + 1 < line.width() {
            return Some((y, x + 1));
        }
        (line.wrapped && y + 1 < self.lines.len()).then_some((y + 1, 0))
    }

    /// The start of the word at `(y, x)`, or on a blank the previous word's,
    /// as tmux's window_copy_cursor_previous_word_pos: what a word
    /// selection snaps back to.
    pub(super) fn word_start_pos(&self, at: (usize, usize)) -> (usize, usize) {
        self.reader_previous_word(at, &self.word_separators, false, true)
    }

    /// tmux's grid_reader_cursor_previous_word: back over blanks (from the
    /// cell before, when `already`) to a word or run of `separators`, then
    /// to that run's start, across wrapped lines; `stop_at_eol` stops at a
    /// line ending in a blank.
    pub(super) fn reader_previous_word(
        &self,
        (y, x): (usize, usize),
        separators: &str,
        already: bool,
        stop_at_eol: bool,
    ) -> (usize, usize) {
        let (mut y, mut x) = (y, x);
        let letters = |at| !self.in_set(at, separators);
        let word_is_letters;
        if already || self.in_set((y, x), WHITESPACE) {
            loop {
                if x > 0 {
                    x -= 1;
                    if !self.in_set((y, x), WHITESPACE) {
                        word_is_letters = letters((y, x));
                        break;
                    }
                } else {
                    if y == 0 {
                        return (y, x);
                    }
                    y -= 1;
                    x = self.line_len(y);
                    if stop_at_eol && x > 0 && self.in_set((y, x - 1), WHITESPACE) {
                        word_is_letters = false;
                        break;
                    }
                }
            }
        } else {
            word_is_letters = letters((y, x));
        }
        loop {
            let (oldy, oldx) = (y, x);
            if x == 0 {
                if y == 0 || !self.lines.get(y - 1).is_some_and(|l| l.wrapped) {
                    return (oldy, oldx);
                }
                y -= 1;
                x = self.lines[y].width();
            }
            x = x.saturating_sub(1);
            if self.in_set((y, x), WHITESPACE) || word_is_letters != letters((y, x)) {
                return (oldy, oldx);
            }
        }
    }
}
