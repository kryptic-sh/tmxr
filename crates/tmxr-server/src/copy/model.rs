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
pub fn snapshot(screen: &vt100::Screen) -> Vec<Line> {
    let total = screen.scrollback_len() + usize::from(tmxr_term::screen_size(screen).0);
    snapshot_lines(screen, 0..total)
}

/// Capture the lines `range` of history and screen, numbered as
/// [`snapshot`]'s, the oldest line of history 0, read straight from vt100's
/// rows (`Screen::line_cells`) without moving the pane's view.
pub fn snapshot_lines(screen: &vt100::Screen, range: std::ops::Range<usize>) -> Vec<Line> {
    lines_from(screen, range, |idx| {
        Some((screen.line_cells(idx)?, screen.line_wrapped(idx)))
    })
}

/// Capture the rows `range` of the normal screen the alternate screen hides,
/// as it was when the alternate screen came on (`capture-pane -a`): no rows
/// when the alternate screen is not shown.
pub fn snapshot_normal_rows(screen: &vt100::Screen, range: std::ops::Range<usize>) -> Vec<Line> {
    lines_from(screen, range, |idx| {
        let row = u16::try_from(idx).ok()?;
        Some((
            screen.normal_screen_row_cells(row)?,
            screen.normal_screen_row_wrapped(row),
        ))
    })
}

/// The lines `range`, each read by `row` as its cells and whether it wraps;
/// a line `row` has none for is empty.
fn lines_from<'s>(
    screen: &vt100::Screen,
    range: std::ops::Range<usize>,
    row: impl Fn(usize) -> Option<(&'s [vt100::Cell], bool)>,
) -> Vec<Line> {
    let width = usize::from(tmxr_term::screen_size(screen).1);
    range
        .map(|idx| match row(idx) {
            // A row of history from before a narrowing resize can be wider
            // than the pane; it shows as wide as the pane.
            Some((cells, wrapped)) => line_of(&cells[..cells.len().min(width)], width, wrapped),
            None => Line {
                cells: Vec::new(),
                width: 0,
                wrapped: false,
                used: 0,
            },
        })
        .collect()
}

/// A copy of one row of vt100 cells, `width` wide.
fn line_of(cells: &[vt100::Cell], width: usize, wrapped: bool) -> Line {
    let written = |cell: &vt100::Cell| {
        cell.has_contents()
            || cell.is_wide_continuation()
            || cell.bgcolor() != vt100::Color::Default
    };
    let used = cells.iter().rposition(written).map_or(0, |c| c + 1);
    // Every cell before the last written one is kept (a written space reads
    // as blank), and any after it up to the last that is not blank.
    let kept = cells
        .iter()
        .rposition(|c| cell_of(Some(c)) != BLANK)
        .map_or(0, |c| c + 1)
        .max(used);
    Line {
        cells: cells[..kept].iter().map(|c| cell_of(Some(c))).collect(),
        width,
        wrapped,
        used,
    }
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
