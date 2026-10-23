//! Copy mode's searches: what a search is, where its needle matches a
//! line, and finding the next match from the cursor.

use super::CopyMode;

/// The characters that make a search a regular expression; one without
/// them is searched for as text, as tmux's window_copy_search decides.
const REGEX_CHARS: &[char] = &['^', '$', '*', '+', '(', ')', '?', '[', ']', '.', '\\'];

/// The last search, for `search-again` and highlighting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    pub needle: String,
    pub forward: bool,
    /// `search-forward` / `-backward` take a regular expression, the `-text`
    /// and incremental searches the text itself, as tmux's searchregex.
    pub regex: bool,
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
            Self::Regex(re) => {
                let text: String = line.iter().collect();
                // Byte offsets in the text to columns of the line.
                let column = |byte: usize| text[..byte].chars().count();
                re.find_iter(&text)
                    .filter(|m| !m.is_empty())
                    .map(|m| (column(m.start()), column(m.end())))
                    .collect()
            }
            Self::Text(needle) => {
                let len = needle.chars().count();
                match_starts(line, needle)
                    .into_iter()
                    .map(|x| (x, x + len))
                    .collect()
            }
            Self::Invalid => Vec::new(),
        }
    }
}

/// Columns where `needle` starts in `line`: literal, and case-insensitive
/// unless the needle has an uppercase letter (smart case).
pub(super) fn match_starts(line: &[char], needle: &str) -> Vec<usize> {
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

impl CopyMode {
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
        let n = self.lines.len();
        let matcher = Needle::new(needle, regex);
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
            let hits = matcher
                .find_in(&self.lines[y].chars())
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
    /// highlighting while they are marked.
    pub fn match_spans(&self, y: usize) -> Vec<(usize, usize)> {
        let (Some(search), Some(line), true) = (&self.search, self.lines.get(y), self.marks) else {
            return Vec::new();
        };
        Needle::new(&search.needle, search.regex).find_in(&line.chars())
    }

    /// Mark the last search's matches, as tmux's window_copy_search_marks
    /// after a search that `found` one, and count them across every line
    /// (`#{search_count}`); one that found none leaves nothing marked.
    pub(super) fn mark_matches(&mut self, found: bool) {
        self.marks = found;
        self.search_count = self.search.as_ref().map_or(0, |search| {
            let needle = Needle::new(&search.needle, search.regex);
            self.lines
                .iter()
                .map(|l| needle.find_in(&l.chars()).len())
                .sum()
        });
    }

    /// `#{search_match}`: the marked match the cursor is on, or just past,
    /// as tmux's window_copy_match_at_cursor.
    pub fn match_at_cursor(&self) -> Option<String> {
        let line = self.lines.get(self.cy)?;
        let (start, end) = self
            .match_spans(self.cy)
            .into_iter()
            .find(|&(start, end)| (start..=end).contains(&self.cx))?;
        Some(
            line.cells()
                .skip(start)
                .take(end - start)
                .map(|c| c.symbol.as_ref())
                .collect(),
        )
    }
}
