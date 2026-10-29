//! Copy mode's searches: what a search is, where its needle matches a
//! line, and finding the next match from the cursor.

use super::CopyMode;
use super::model::{Line, PADDING};

/// The characters that make a search a regular expression; one without
/// them is searched for as text, as tmux's window_copy_search decides.
const REGEX_CHARS: &[char] = &['^', '$', '*', '+', '(', ')', '?', '[', ']', '.', '\\'];

/// The most columns a regular expression's match is looked for in, from
/// the row it starts on through the rows that row wraps into, as tmux's
/// WINDOW_COPY_SEARCH_MAX_LINE.
const SEARCH_MAX_LINE: usize = 2000;

/// The last search, for `search-again` and highlighting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    pub needle: String,
    pub forward: bool,
    /// `search-forward` / `-backward` take a regular expression, the `-text`
    /// and incremental searches the text itself, as tmux's searchregex.
    pub regex: bool,
}

impl Search {
    fn effective(&self) -> (&str, bool) {
        effective(&self.needle, self.regex)
    }
}

/// What a search looks for, made once to be matched against many lines:
/// an extended regular expression when the search takes one and the needle
/// has one of [`REGEX_CHARS`], else the text itself; case-insensitive
/// unless the needle has an uppercase letter (smart case), as tmux's
/// searches are.
pub(super) enum Needle<'a> {
    Regex(regex::Regex),
    Text(&'a str),
    /// A pattern that does not compile, which matches nothing.
    Invalid,
}

impl<'a> Needle<'a> {
    pub(super) fn new(needle: &'a str, regex: bool) -> Self {
        if !(regex && needle.contains(REGEX_CHARS)) {
            return Self::Text(needle);
        }
        regex::RegexBuilder::new(needle)
            .case_insensitive(!needle.chars().any(char::is_uppercase))
            .build()
            .map_or(Self::Invalid, Self::Regex)
    }

    /// The `(start, end)` columns of the needle's matches in `line`.
    pub(super) fn find_in(&self, line: &[char]) -> Vec<(usize, usize)> {
        match self {
            // The text without wide characters' padding, as tmux's
            // window_copy_stringify, its matches mapped back to columns.
            Self::Regex(re) => {
                let mut text = String::new();
                // The column of each byte of the text, and of its end.
                let mut columns = Vec::new();
                for (x, &c) in line.iter().enumerate().filter(|(_, c)| **c != PADDING) {
                    text.push(c);
                    columns.extend(std::iter::repeat_n(x, c.len_utf8()));
                }
                columns.push(line.len());
                let column = |byte: usize| columns[byte];
                re.find_iter(&text)
                    .filter(|m| !m.is_empty())
                    .map(|m| (column(m.start()), column(m.end())))
                    .collect()
            }
            // Each match after the last one's end, as tmux's
            // window_copy_search_marks steps over a match it marks.
            Self::Text(needle) => {
                let needle = in_columns(needle);
                let len = needle.len();
                let mut next = 0;
                match_starts(line, &needle)
                    .into_iter()
                    .filter(|&x| {
                        let free = x >= next;
                        if free {
                            next = x + len;
                        }
                        free
                    })
                    .map(|x| (x, x + len))
                    .collect()
            }
            Self::Invalid => Vec::new(),
        }
    }
}

impl Needle<'_> {
    /// The columns a match starting in a row of `width` can run to, through
    /// the rows it wraps into.
    fn reach(&self, width: usize) -> usize {
        match self {
            Self::Text(needle) => width + in_columns(needle).len().saturating_sub(1),
            Self::Regex(_) => SEARCH_MAX_LINE.max(width),
            Self::Invalid => 0,
        }
    }

    /// The `(start, end)` of the matches starting on line `y` of `lines`,
    /// as columns from its start: a match runs on into the lines it wraps
    /// into, so `end` can pass its width, as tmux's window_copy_search_lr
    /// reads on past a wrapped line's end.
    pub(super) fn row_matches(&self, lines: &[Line], y: usize) -> Vec<(usize, usize)> {
        let Some(line) = lines.get(y) else {
            return Vec::new();
        };
        let width = line.width();
        let reach = self.reach(width);
        let mut text = line.chars();
        let mut row = y;
        while text.len() < reach && lines[row].wrapped {
            row += 1;
            let Some(next) = lines.get(row) else {
                break;
            };
            text.extend(next.chars());
        }
        let mut found = self.find_in(&text);
        found.retain(|&(x, _)| x < width);
        found
    }
}

/// A marked match's cells on one line: `from..to` there, of the match that
/// starts at `start` (line and column) and is `len` columns long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: (usize, usize),
    pub len: usize,
    pub from: usize,
    pub to: usize,
}

/// `text` a character a column, as a line's [`Line::chars`]: each wide
/// character followed by [`PADDING`].
fn in_columns(text: &str) -> Vec<char> {
    let mut columns = Vec::new();
    for c in text.chars() {
        columns.push(c);
        if ratatui::text::Span::raw(c.to_string()).width() > 1 {
            columns.push(PADDING);
        }
    }
    columns
}

/// Columns where `needle`, a character a column, starts in `line`:
/// literal, and case-insensitive unless the needle has an uppercase letter
/// (smart case).
pub(super) fn match_starts(line: &[char], needle: &[char]) -> Vec<usize> {
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

impl CopyMode {
    /// To the next match of `needle` up or down from the cursor, round
    /// past the end when `wrap-search` says, as tmux's window_copy_search:
    /// a match at the cursor itself counts unless `skip_current`. Under
    /// emacs keys a search down starts at the cursor and leaves it just
    /// past the match; under vi keys one skipping the cursor starts after
    /// the marked match the cursor is on, or at the cursor when it is on
    /// none, and one cell on only while nothing is marked
    /// (`window_copy_move_after_search_mark`).
    pub(super) fn find(
        &mut self,
        needle: &str,
        forward: bool,
        regex: bool,
        skip_current: bool,
    ) -> bool {
        if needle.is_empty() {
            return false;
        }
        let past_match = forward && self.emacs;
        let skip_current = skip_current && !past_match;
        let n = self.lines.len();
        let matcher = Needle::new(needle, regex);
        // Where a search down starts, as lines on from the cursor's and
        // the column there.
        let from = if !forward || !skip_current {
            (0, self.cx)
        } else if self.marked(needle, regex) {
            self.match_spans(self.cy)
                .into_iter()
                .find(|m| (m.from..m.to).contains(&self.cx))
                .map_or((0, self.cx), |m| {
                    let (y, x) = self.at_offset((m.start.0, m.start.1 + m.len));
                    (y - self.cy, x)
                })
        } else {
            (0, self.cx + 1)
        };
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
            let hits = matcher.row_matches(&self.lines, y);
            let counts = |x: usize| {
                if forward {
                    (step, x) >= from
                } else {
                    step > 0 || x < self.cx || (!skip_current && x == self.cx)
                }
            };
            let pick = if forward {
                hits.into_iter().find(|&(x, _)| counts(x))
            } else {
                hits.into_iter().rev().find(|&(x, _)| counts(x))
            };
            if let Some((x, end)) = pick {
                self.search_forward = forward;
                self.scroll_to((y, x));
                if past_match {
                    (self.cy, self.cx) = self.after_cell(self.at_offset((y, end - 1)));
                }
                return true;
            }
        }
        false
    }

    /// Column `x` of line `y`, which can run past its width into the
    /// lines it wraps into, as a line and a column on it.
    fn at_offset(&self, (y, x): (usize, usize)) -> (usize, usize) {
        match self.lines.get(y).map(Line::width) {
            Some(width) if width > 0 => (y + x / width, x % width),
            _ => (y, x),
        }
    }

    /// The cell after `(y, x)`, on to the next line's start from the last
    /// column, and round to the first line's past the last when
    /// `wrap-search` says, as tmux's window_copy_move_right.
    fn after_cell(&self, (y, x): (usize, usize)) -> (usize, usize) {
        if x + 1 < usize::from(self.cols) {
            (y, x + 1)
        } else if y < self.lines.len().saturating_sub(1) {
            (y + 1, 0)
        } else if self.wrap_search {
            (0, 0)
        } else {
            (y, x)
        }
    }

    /// The last search's matches with cells on line `y`, for highlighting
    /// while they are marked: those starting on it, and those run on to it
    /// from the lines before that wrap into it.
    pub fn match_spans(&self, y: usize) -> Vec<Span> {
        let (Some(search), Some(line), true) = (&self.search, self.lines.get(y), self.marks) else {
            return Vec::new();
        };
        let needle = Needle::new(&search.needle, search.regex);
        let width = line.width();
        let mut first = y;
        while first > 0
            && self.lines[first - 1].wrapped
            && (y - first + 1) * width < needle.reach(width)
        {
            first -= 1;
        }
        let mut spans = Vec::new();
        for row in first..=y {
            // Line `y`'s columns, counted from line `row`'s start.
            let here = (y - row) * width..(y - row + 1) * width;
            for (start, end) in needle.row_matches(&self.lines, row) {
                let (from, to) = (start.max(here.start), end.min(here.end));
                if from < to {
                    spans.push(Span {
                        start: (row, start),
                        len: end - start,
                        from: from - here.start,
                        to: to - here.start,
                    });
                }
            }
        }
        spans
    }

    /// Mark the last search's matches after it ran, as tmux's
    /// window_copy_search: a copy mode's first search, or one for other
    /// text than the search before it (`previous`), clears the marks and,
    /// when it `found` a match, marks and counts them across every line
    /// (`#{search_count}`); the same search again keeps the marks and the
    /// count, and only adds marks when it found one.
    pub(super) fn mark_matches(&mut self, found: bool, previous: Option<&Search>) {
        let same = !std::mem::take(&mut self.search_all)
            && previous.map(Search::effective) == self.search.as_ref().map(Search::effective);
        if !same {
            self.marks = false;
        }
        if !found {
            return;
        }
        self.marks = true;
        if !same {
            self.search_count = self.search.as_ref().map_or(0, |search| {
                let needle = Needle::new(&search.needle, search.regex);
                (0..self.lines.len())
                    .map(|y| needle.row_matches(&self.lines, y).len())
                    .sum()
            });
        }
    }

    /// Whether the marks are a search for `needle` (`regex` as it takes
    /// one), which a search for it keeps rather than clears.
    fn marked(&self, needle: &str, regex: bool) -> bool {
        self.marks
            && !self.search_all
            && self.search.as_ref().map(Search::effective) == Some(effective(needle, regex))
    }

    /// The marked match tmux draws as the current one, as line and start
    /// column: the one the cursor is on, or under emacs keys after a search
    /// down the one just before it, as tmux's window_copy_update_style.
    pub fn current_match(&self) -> Option<(usize, usize)> {
        let (y, x) = if self.emacs && self.search_forward {
            match self.cx.checked_sub(1) {
                Some(x) => (self.cy, x),
                None => (
                    self.cy.checked_sub(1)?,
                    usize::from(self.cols).checked_sub(1)?,
                ),
            }
        } else {
            (self.cy, self.cx)
        };
        self.match_spans(y)
            .into_iter()
            .find(|m| (m.from..m.to).contains(&x))
            .map(|m| m.start)
    }

    /// `#{search_match}`: the marked match the cursor is on, or just past,
    /// as tmux's window_copy_match_at_cursor.
    pub fn match_at_cursor(&self) -> Option<String> {
        let m = self
            .match_spans(self.cy)
            .into_iter()
            .find(|m| (m.from..=m.to).contains(&self.cx))?;
        Some(
            self.lines[m.start.0..]
                .iter()
                .flat_map(Line::symbols)
                .skip(m.start.1)
                .take(m.len)
                .collect(),
        )
    }
}

/// What a search for `needle` looks for, as tmux compares one search with
/// the last: its text, and whether it is a regular expression.
fn effective(needle: &str, regex: bool) -> (&str, bool) {
    (needle, regex && needle.contains(REGEX_CHARS))
}
