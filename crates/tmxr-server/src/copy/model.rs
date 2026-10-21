//! The copy of a pane copy mode works on: its history and screen captured as
//! lines of styled cells when copy mode starts.

use ratatui::style::{Color, Modifier, Style};

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
    /// How many cells from the left were written, as tmux's cellused: up to
    /// the last holding a character or a background colour. (vt100 keeps
    /// an erased cell as it keeps one never written, where tmux counts an
    /// erased cell inside what was written.)
    pub used: usize,
}

impl Line {
    pub(super) fn chars(&self) -> Vec<char> {
        self.cells
            .iter()
            .map(|c| c.symbol.chars().next().unwrap_or(' '))
            .collect()
    }

    /// Text of columns `from..to`, trailing blanks removed.
    pub(super) fn text(&self, from: usize, to: usize) -> String {
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
pub fn snapshot(screen: &mut vt100::Screen) -> Vec<Line> {
    let total = history_len(screen) + usize::from(tmxr_term::screen_size(screen).0);
    snapshot_lines(screen, 0..total)
}

/// How many lines of history the pane holds.
pub fn history_len(screen: &mut vt100::Screen) -> usize {
    let original = screen.scrollback();
    screen.set_scrollback(usize::MAX);
    let history = screen.scrollback();
    screen.set_scrollback(original);
    history
}

/// Capture the lines `range` of history and screen, numbered as
/// [`snapshot`]'s, the oldest line of history 0: only the screenfuls of
/// history that hold them are read, as a pane's history can be long.
pub fn snapshot_lines(screen: &mut vt100::Screen, range: std::ops::Range<usize>) -> Vec<Line> {
    let (rows, cols) = tmxr_term::screen_size(screen);
    let original = screen.scrollback();
    let history = history_len(screen);
    let mut lines: Vec<Option<Line>> = vec![None; range.len()];
    // The screenful whose top row is the range's first line, then each one
    // below it, to the screen itself (offset 0).
    let mut offset = history.saturating_sub(range.start);
    loop {
        screen.set_scrollback(offset);
        for r in 0..rows {
            let idx = history - offset + usize::from(r);
            if !range.contains(&idx) || lines[idx - range.start].is_some() {
                continue;
            }
            let written = |c: u16| {
                screen.cell(r, c).is_some_and(|cell| {
                    cell.has_contents()
                        || cell.is_wide_continuation()
                        || cell.bgcolor() != vt100::Color::Default
                })
            };
            let used = (0..cols)
                .rev()
                .find(|&c| written(c))
                .map_or(0, |c| usize::from(c) + 1);
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
            lines[idx - range.start] = Some(Line {
                cells,
                wrapped: screen.row_wrapped(r),
                used,
            });
        }
        if offset == 0 || history - offset + usize::from(rows) >= range.end {
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
                used: 0,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ranged capture is the same slice of the full one, wherever the
    /// range falls: in history, across its end, on the screen, or past it.
    #[test]
    fn a_range_of_lines_is_that_slice_of_the_whole_snapshot() {
        let mut parser = vt100::Parser::new(5.try_into().unwrap(), 12.try_into().unwrap(), 100);
        for n in 0..37 {
            parser
                .process(format!("\x1b[3{}mline {n} {}\r\n", n % 8, "x".repeat(n % 15)).as_bytes());
        }
        parser.screen_mut().set_scrollback(3);
        let screen = parser.screen_mut();
        let all = snapshot(screen);
        assert!(all.len() > 40, "history wrapped lines: {}", all.len());
        let total = all.len();
        for range in [
            0..1,
            0..total,
            2..9,
            4..23,
            total - 7..total,
            total - 5..total,
            total - 6..total - 2,
            17..18,
        ] {
            assert_eq!(
                snapshot_lines(screen, range.clone()),
                all[range.clone()],
                "{range:?}"
            );
        }
        assert_eq!(screen.scrollback(), 3, "the pane's own scroll is kept");
    }
}
