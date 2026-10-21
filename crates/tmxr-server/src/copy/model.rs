//! The copy of a pane copy mode works on: its history and screen captured as
//! lines of styled cells when copy mode starts.

use std::borrow::Cow;

use ratatui::style::{Color, Modifier, Style};

/// One captured cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// Borrowed for a blank, a wide character's second column and any one
    /// ASCII character, so a snapshot of a long history allocates only for
    /// the rest.
    pub symbol: Cow<'static, str>,
    pub style: Style,
}

/// Every ASCII character, each at its own code, for [`symbol`] to borrow.
static ASCII: [u8; 128] = {
    let mut all = [0; 128];
    let mut i = 0;
    while i < all.len() {
        all[i] = i as u8;
        i += 1;
    }
    all
};

/// A cell's contents as a [`Cell::symbol`], borrowed when it is one ASCII
/// character.
fn symbol(contents: &str) -> Cow<'static, str> {
    match contents.as_bytes() {
        &[b] if b.is_ascii() => {
            let at = usize::from(b);
            std::str::from_utf8(&ASCII[at..=at])
                .map_or_else(|_| Cow::Owned(contents.to_owned()), Cow::Borrowed)
        }
        _ => Cow::Owned(contents.to_owned()),
    }
}

/// A cell nothing was written to, as [`cell_of`] copies one.
const BLANK: Cell = Cell {
    symbol: Cow::Borrowed(" "),
    style: Style::new().fg(Color::Reset).bg(Color::Reset),
};

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The cells up to the last one that is not [`BLANK`]; the rest of the
    /// line's width is blank, which a long history mostly is.
    cells: Vec<Cell>,
    width: usize,
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
    /// The pane's width when the line was captured.
    pub fn width(&self) -> usize {
        self.width
    }

    /// The cell at column `x`, if the line is that wide.
    pub fn cell(&self, x: usize) -> Option<&Cell> {
        (x < self.width).then(|| self.cells.get(x).unwrap_or(&BLANK))
    }

    /// Every cell, across the line's width.
    pub fn cells(&self) -> impl ExactSizeIterator<Item = &Cell> + DoubleEndedIterator {
        (0..self.width).map(|x| self.cells.get(x).unwrap_or(&BLANK))
    }

    pub(super) fn chars(&self) -> Vec<char> {
        self.cells()
            .map(|c| c.symbol.chars().next().unwrap_or(' '))
            .collect()
    }

    /// Text of columns `from..to`, trailing blanks removed.
    pub(super) fn text(&self, from: usize, to: usize) -> String {
        let s: String = self
            .cells()
            .enumerate()
            .filter(|(i, _)| *i >= from && *i < to)
            .map(|(_, c)| if c.symbol.is_empty() { "" } else { &c.symbol })
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
            let written = |cell: &vt100::Cell| {
                cell.has_contents()
                    || cell.is_wide_continuation()
                    || cell.bgcolor() != vt100::Color::Default
            };
            // One scan from the right finds both ends: the last cell that is
            // not blank, and the last written one, before which every cell is
            // kept (a written space reads as blank, so it can come first).
            let (mut used, mut kept) = (0, 0);
            for c in (0..cols).rev() {
                let cell = screen.cell(r, c);
                if kept == 0 && cell_of(cell) != BLANK {
                    kept = c + 1;
                }
                if cell.is_some_and(written) {
                    used = usize::from(c) + 1;
                    kept = kept.max(c + 1);
                    break;
                }
            }
            let cells = (0..kept).map(|c| cell_of(screen.cell(r, c))).collect();
            lines[idx - range.start] = Some(Line {
                cells,
                width: usize::from(cols),
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
                width: 0,
                wrapped: false,
                used: 0,
            })
        })
        .collect()
}

/// A copy of `cell`, one of what a screen shows.
fn cell_of(cell: Option<&vt100::Cell>) -> Cell {
    match cell {
        Some(cell) => Cell {
            symbol: if cell.is_wide_continuation() {
                Cow::Borrowed("")
            } else if cell.has_contents() {
                symbol(cell.contents())
            } else {
                Cow::Borrowed(" ")
            },
            style: cell_style(cell),
        },
        None => Cell {
            symbol: Cow::Borrowed(" "),
            style: Style::default(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ranged capture is the same slice of the full one, wherever the
    /// range falls: in history, across its end, on the screen, or past it.
    /// A line keeps only its cells up to the last that is not blank, so a
    /// blank has to be what a cell never written reads as, or trimming would
    /// change the line.
    #[test]
    fn a_cell_never_written_is_blank() {
        let parser = vt100::Parser::new(2.try_into().unwrap(), 4.try_into().unwrap(), 0);
        assert_eq!(cell_of(parser.screen().cell(1, 3)), BLANK);
    }

    #[test]
    fn a_line_reads_as_wide_as_the_pane_with_blanks_after_its_cells() {
        let mut parser = vt100::Parser::new(2.try_into().unwrap(), 6.try_into().unwrap(), 0);
        parser.process(b"ab\x1b[7m \x1b[m");
        let lines = snapshot(parser.screen_mut());
        let line = &lines[0];
        assert_eq!(line.width(), 6);
        assert_eq!(line.cells.len(), 3, "the reversed blank is kept");
        assert_eq!(line.cells().count(), 6);
        assert_eq!(line.cell(5), Some(&BLANK));
        assert_eq!(line.cell(6), None);
        assert_eq!(line.chars(), vec!['a', 'b', ' ', ' ', ' ', ' ']);
        assert!(lines[1].cells.is_empty());
        assert_eq!(lines[1].cells().count(), 6);
    }

    /// Cells erased while bold is on are not blank, so they are kept, but
    /// nothing was written to them, so `used` stops before them.
    #[test]
    fn a_line_keeps_cells_past_the_last_written_one() {
        let mut parser = vt100::Parser::new(2.try_into().unwrap(), 6.try_into().unwrap(), 0);
        parser.process(b"ab\x1b[1m\x1b[K\x1b[m");
        let line = &snapshot(parser.screen_mut())[0];
        assert_eq!(line.used, 2);
        assert_eq!(line.cells.len(), 6);
        assert!(
            line.cell(5)
                .unwrap()
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    /// A space written with no attributes reads as a blank, but it is still
    /// written: the line keeps everything before it, as a prompt ends in one.
    #[test]
    fn a_line_ending_in_a_written_space_keeps_its_text() {
        let mut parser = vt100::Parser::new(2.try_into().unwrap(), 6.try_into().unwrap(), 0);
        parser.process(b"$ ");
        let line = &snapshot(parser.screen_mut())[0];
        assert_eq!(line.used, 2);
        assert_eq!(line.text(0, 6), "$");
    }

    #[test]
    fn an_ascii_symbol_is_borrowed_and_reads_the_same() {
        for b in 0..128u8 {
            let s = char::from(b).to_string();
            let sym = symbol(&s);
            assert!(matches!(sym, Cow::Borrowed(_)), "{b}");
            assert_eq!(sym, s);
        }
        assert!(matches!(symbol("é"), Cow::Owned(ref s) if s == "é"));
        assert!(matches!(symbol("ab"), Cow::Owned(ref s) if s == "ab"));
    }

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
