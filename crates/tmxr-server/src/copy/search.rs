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

/// The `(start, end)` columns of `needle`'s matches in `line`: as an
/// extended regular expression when `regex` asks and the needle has one of
/// [`REGEX_CHARS`] (a pattern that does not compile matches nothing), else
/// literally; case-insensitive unless the needle has an uppercase letter
/// (smart case), as tmux's searches are.
pub(super) fn matches(line: &[char], needle: &str, regex: bool) -> Vec<(usize, usize)> {
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
}
