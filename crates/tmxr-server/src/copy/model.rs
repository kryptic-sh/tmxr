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
    pub underline: Underline,
}

/// The lines a cell is drawn with that its style cannot say: its
/// underline's style and colour beyond the style's `UNDERLINED` (which any
/// underline sets), and an overline, as vt1k keeps them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Underline {
    pub style: vt1k::UnderlineStyle,
    pub colour: vt1k::Color,
    /// A line over the text (SGR 53), which tmxr captures but, as tmux to a
    /// terminal it has not identified, does not draw.
    pub overline: bool,
}

impl Underline {
    /// No underline or overline.
    pub const NONE: Self = Self {
        style: vt1k::UnderlineStyle::None,
        colour: vt1k::Color::Default,
        overline: false,
    };

    pub fn of(cell: &vt1k::Cell) -> Self {
        Self {
            style: cell.underline_style(),
            colour: cell.underline_color(),
            overline: cell.overline(),
        }
    }

    /// Says no more than `UNDERLINED` does: none, or a single line in the
    /// text's colour.
    pub fn is_plain(self) -> bool {
        matches!(
            self.style,
            vt1k::UnderlineStyle::None | vt1k::UnderlineStyle::Single
        ) && self.colour == vt1k::Color::Default
    }
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
    underline: Underline::NONE,
};

/// A line's cells, kept small: their symbols one after another and their
/// styles as runs, so a line of text in a few styles takes about a byte a
/// cell.
#[derive(Debug, Clone, PartialEq)]
struct Cells {
    /// Every cell's symbol, one after another.
    text: Box<str>,
    /// Where each cell's symbol begins in `text`, unless every one is a
    /// single byte (a wide character's second column has none).
    starts: Option<Box<[usize]>>,
    /// Each change of style: the first cell it takes and the style. Cells
    /// before the first are in [`BLANK`]'s.
    runs: Box<[(usize, Style, Underline)]>,
}

impl Cells {
    fn new(cells: &[Cell]) -> Self {
        let mut text = String::new();
        let mut starts = Vec::with_capacity(cells.len());
        let mut runs = Vec::new();
        let mut last = (BLANK.style, Underline::NONE);
        for (x, cell) in cells.iter().enumerate() {
            starts.push(text.len());
            text.push_str(&cell.symbol);
            if (cell.style, cell.underline) != last {
                last = (cell.style, cell.underline);
                runs.push((x, cell.style, cell.underline));
            }
        }
        let single = cells.iter().all(|c| c.symbol.len() == 1);
        Self {
            text: text.into_boxed_str(),
            starts: (!single).then(|| starts.into_boxed_slice()),
            runs: runs.into_boxed_slice(),
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.starts.as_ref().map_or(self.text.len(), |s| s.len())
    }

    fn symbol(&self, x: usize) -> Option<&str> {
        match &self.starts {
            None => self.text.get(x..=x),
            Some(starts) => {
                let start = *starts.get(x)?;
                let end = starts.get(x + 1).copied().unwrap_or(self.text.len());
                self.text.get(start..end)
            }
        }
    }

    fn get(&self, x: usize) -> Option<Cell> {
        let symbol = symbol(self.symbol(x)?);
        let run = self.runs.partition_point(|&(from, ..)| from <= x);
        let (style, underline) = run
            .checked_sub(1)
            .map_or((BLANK.style, Underline::NONE), |r| {
                (self.runs[r].1, self.runs[r].2)
            });
        Some(Cell {
            symbol,
            style,
            underline,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The cells up to the last one that is not [`BLANK`]; the rest of the
    /// line's width is blank, which a long history mostly is.
    cells: Cells,
    width: usize,
    /// The line continues on the next one (soft wrap): no newline between
    /// them when copied.
    pub wrapped: bool,
    /// How many cells from the left were written, as tmux's cellused: up to
    /// the last holding a character or a background colour. (vt1k keeps
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
    pub fn cell(&self, x: usize) -> Option<Cell> {
        (x < self.width).then(|| self.cells.get(x).unwrap_or(BLANK))
    }

    /// Every cell, across the line's width.
    pub fn cells(&self) -> impl ExactSizeIterator<Item = Cell> + DoubleEndedIterator {
        (0..self.width).map(|x| self.cells.get(x).unwrap_or(BLANK))
    }

    /// The symbol of the cell at column `x`, if the line is that wide:
    /// empty for a wide character's second column.
    pub fn symbol(&self, x: usize) -> Option<&str> {
        (x < self.width).then(|| self.cells.symbol(x).unwrap_or(" "))
    }

    /// Every cell's symbol, across the line's width.
    pub fn symbols(&self) -> impl ExactSizeIterator<Item = &str> + DoubleEndedIterator {
        (0..self.width).map(|x| self.cells.symbol(x).unwrap_or(" "))
    }

    /// A character a column: the first of its cell's symbol, and
    /// [`PADDING`] for a wide character's second column.
    pub(super) fn chars(&self) -> Vec<char> {
        self.symbols()
            .map(|s| s.chars().next().unwrap_or(PADDING))
            .collect()
    }

    /// Text of columns `from..to`, trailing blanks removed.
    pub(super) fn text(&self, from: usize, to: usize) -> String {
        let s: String = self
            .symbols()
            .enumerate()
            .filter(|(i, _)| *i >= from && *i < to)
            .map(|(_, s)| s)
            .collect();
        s.trim_end().to_owned()
    }
}

/// What [`Line::chars`] gives a wide character's second column, so text
/// searched for lines up with the columns it takes, as tmux's
/// `window_copy_search_compare` compares cells padding and all.
pub(super) const PADDING: char = '\0';

/// Copy mode's lines rewrapped to `cols` columns, as tmux's
/// window_copy_resize rewraps its copy of the pane, and by the same rules
/// the pane's own lines are (the vt1k fork's `Grid::reflow`, after tmux's
/// `grid_reflow`): each line's rows, as written, joined and split again at
/// the new width, a wide character counted three columns where a row
/// breaks. Gives the lines (at least `rows` of them), each old line's new
/// number, and where the cursor `(y, x)` goes: its place in its line, or
/// the line's end if it was past it, as tmux's `grid_wrap_position`.
pub fn reflow(
    lines: &[Line],
    cols: usize,
    rows: usize,
    (cy, cx): (usize, usize),
) -> (Vec<Line>, Vec<usize>, (usize, usize)) {
    // Each line as written: its cells, and the old lines it was made of
    // with where each began; the cursor's line and its cell, `None` past
    // the end.
    let mut logical: Vec<Written> = Vec::new();
    let mut cursor = (0, None);
    let mut joining = false;
    for (y, line) in lines.iter().enumerate() {
        if !joining {
            logical.push((Vec::new(), Vec::new()));
        }
        let n = logical.len() - 1;
        let (cells, starts) = &mut logical[n];
        starts.push((y, cells.len()));
        if y == cy {
            cursor = (n, (cx < line.used).then(|| cells.len() + cx));
        }
        cells.extend(line.cells().take(line.used));
        joining = line.wrapped;
    }
    let mut out: Vec<Line> = Vec::new();
    let mut map = vec![0; lines.len()];
    let mut new_cursor = (0, 0);
    for (n, (cells, starts)) in logical.iter().enumerate() {
        let mut row: Vec<Cell> = Vec::new();
        let mut counted = 0;
        let mut starts = starts.iter().peekable();
        for (i, cell) in cells.iter().enumerate() {
            let wide = cells.get(i + 1).is_some_and(|c| c.symbol.is_empty());
            let need = match (wide, cell.symbol.is_empty()) {
                (true, _) => 3,
                (_, true) => 0,
                _ => 1,
            };
            if need > 0 && !row.is_empty() && counted + need > cols {
                out.push(line_of_cells(std::mem::take(&mut row), cols, true));
                counted = 0;
            }
            counted += need;
            while let Some(&&(y, at)) = starts.peek() {
                if at > i {
                    break;
                }
                map[y] = out.len();
                starts.next();
            }
            if cursor == (n, Some(i)) {
                new_cursor = (out.len(), row.len());
            }
            row.push(cell.clone());
        }
        for &(y, _) in starts {
            map[y] = out.len();
        }
        if cursor == (n, None) {
            new_cursor = (out.len(), row.len().min(cols.saturating_sub(1)));
        }
        out.push(line_of_cells(row, cols, false));
    }
    while out.len() < rows {
        out.push(line_of_cells(Vec::new(), cols, false));
    }
    (out, map, new_cursor)
}

/// A line as written, before it wrapped: its cells, and each line it was
/// made of with the index of that line's first cell.
type Written = (Vec<Cell>, Vec<(usize, usize)>);

/// A line of `cols` columns holding `cells` from the left, all of them
/// used.
fn line_of_cells(cells: Vec<Cell>, cols: usize, wrapped: bool) -> Line {
    let used = cells.len().min(cols);
    let kept = cells.iter().rposition(|c| *c != BLANK).map_or(0, |i| i + 1);
    Line {
        cells: Cells::new(&cells[..kept.min(cols)]),
        width: cols,
        wrapped,
        used,
    }
}

pub fn colour(c: vt1k::Color) -> Color {
    match c {
        vt1k::Color::Default => Color::Reset,
        vt1k::Color::Idx(i) => Color::Indexed(i),
        vt1k::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

pub fn cell_style(cell: &vt1k::Cell) -> Style {
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
    if cell.blink() {
        m |= Modifier::SLOW_BLINK;
    }
    if cell.hidden() {
        m |= Modifier::HIDDEN;
    }
    if cell.strikethrough() {
        m |= Modifier::CROSSED_OUT;
    }
    s = s.add_modifier(m);
    s
}

/// Capture every line of history plus the visible screen.
pub fn snapshot(screen: &vt1k::Screen) -> Vec<Line> {
    let total = screen.scrollback_len() + usize::from(tmxr_term::screen_size(screen).0);
    snapshot_lines(screen, 0..total)
}

/// Capture the lines `range` of history and screen, numbered as
/// [`snapshot`]'s, the oldest line of history 0, read straight from vt1k's
/// rows (`Screen::line_cells`, which unpacks a row of history) without
/// moving the pane's view.
pub fn snapshot_lines(screen: &vt1k::Screen, range: std::ops::Range<usize>) -> Vec<Line> {
    lines_from(screen, range, |idx| {
        Some((screen.line_cells(idx)?, screen.line_wrapped(idx)))
    })
}

/// Capture the rows `range` of the normal screen the alternate screen hides,
/// as it was when the alternate screen came on (`capture-pane -a`): no rows
/// when the alternate screen is not shown.
pub fn snapshot_normal_rows(screen: &vt1k::Screen, range: std::ops::Range<usize>) -> Vec<Line> {
    lines_from(screen, range, |idx| {
        let row = u16::try_from(idx).ok()?;
        Some((
            std::borrow::Cow::Borrowed(screen.normal_screen_row_cells(row)?),
            screen.normal_screen_row_wrapped(row),
        ))
    })
}

/// The lines `range`, each read by `row` as its cells and whether it wraps;
/// a line `row` has none for is empty.
fn lines_from<'s>(
    screen: &vt1k::Screen,
    range: std::ops::Range<usize>,
    row: impl Fn(usize) -> Option<(std::borrow::Cow<'s, [vt1k::Cell]>, bool)>,
) -> Vec<Line> {
    let width = usize::from(tmxr_term::screen_size(screen).1);
    range
        .map(|idx| match row(idx) {
            // A row of history from before a narrowing resize can be wider
            // than the pane; it shows as wide as the pane.
            Some((cells, wrapped)) => line_of(&cells[..cells.len().min(width)], width, wrapped),
            None => Line {
                cells: Cells::new(&[]),
                width: 0,
                wrapped: false,
                used: 0,
            },
        })
        .collect()
}

/// `style` carrying the cell's OSC 8 link, as vt1k numbers it, in its
/// `underline_color`, which nothing else in a snapshot sets (the underline's
/// own colour is kept in [`Cell::underline`]): so a cell stays as small.
/// [`link_of`] reads it back,
/// and a style drawn from a snapshot must drop it first.
fn with_link(style: Style, link: Option<std::num::NonZeroU16>) -> Style {
    let Some(link) = link else {
        return style;
    };
    let [hi, lo] = link.get().to_be_bytes();
    Style {
        underline_color: Some(Color::Rgb(0, hi, lo)),
        ..style
    }
}

/// The OSC 8 link a snapshot cell's style carries (see [`with_link`]), for
/// the pane's `Screen::hyperlink`.
pub fn link_of(style: &Style) -> Option<std::num::NonZeroU16> {
    match style.underline_color {
        Some(Color::Rgb(0, hi, lo)) => std::num::NonZeroU16::new(u16::from_be_bytes([hi, lo])),
        _ => None,
    }
}

/// A copy of one row of vt1k cells, `width` wide.
fn line_of(cells: &[vt1k::Cell], width: usize, wrapped: bool) -> Line {
    let written = |cell: &vt1k::Cell| {
        cell.has_contents() || cell.is_wide_continuation() || cell.bgcolor() != vt1k::Color::Default
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
        cells: Cells::new(
            &cells[..kept]
                .iter()
                .map(|c| cell_of(Some(c)))
                .collect::<Vec<_>>(),
        ),
        width,
        wrapped,
        used,
    }
}

/// A copy of `cell`, one of what a screen shows.
fn cell_of(cell: Option<&vt1k::Cell>) -> Cell {
    match cell {
        Some(cell) => Cell {
            symbol: if cell.is_wide_continuation() {
                Cow::Borrowed("")
            } else if cell.has_contents() {
                symbol(cell.contents())
            } else {
                Cow::Borrowed(" ")
            },
            style: with_link(cell_style(cell), cell.hyperlink_id()),
            underline: Underline::of(cell),
        },
        None => Cell {
            symbol: Cow::Borrowed(" "),
            style: Style::default(),
            underline: Underline::NONE,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot keeps each cell's link in its style, and a cell stays as
    /// small as one without (its underline's style and colour beside it).
    #[test]
    fn a_snapshot_keeps_a_cells_link() {
        let mut parser = vt1k::Parser::new(2.try_into().unwrap(), 10.try_into().unwrap(), 0);
        parser.process(b"a\x1b]8;;u\x07b\x1b]8;;\x07c");
        let line = &snapshot(parser.screen())[0];
        let links: Vec<_> = (0..3)
            .map(|x| line.cell(x).and_then(|c| link_of(&c.style)))
            .collect();
        let id = parser
            .screen()
            .cell(0, 1)
            .and_then(vt1k::Cell::hyperlink_id);
        assert!(id.is_some());
        assert_eq!(links, [None, id, None]);
        assert_eq!(std::mem::size_of::<Cell>(), 48);
    }

    /// A ranged capture is the same slice of the full one, wherever the
    /// range falls: in history, across its end, on the screen, or past it.
    /// A line keeps only its cells up to the last that is not blank, so a
    /// blank has to be what a cell never written reads as, or trimming would
    /// change the line.
    #[test]
    fn a_cell_never_written_is_blank() {
        let parser = vt1k::Parser::new(2.try_into().unwrap(), 4.try_into().unwrap(), 0);
        assert_eq!(cell_of(parser.screen().cell(1, 3)), BLANK);
    }

    #[test]
    fn a_line_reads_as_wide_as_the_pane_with_blanks_after_its_cells() {
        let mut parser = vt1k::Parser::new(2.try_into().unwrap(), 6.try_into().unwrap(), 0);
        parser.process(b"ab\x1b[7m \x1b[m");
        let lines = snapshot(parser.screen_mut());
        let line = &lines[0];
        assert_eq!(line.width(), 6);
        assert_eq!(line.cells.len(), 3, "the reversed blank is kept");
        assert_eq!(line.cells().count(), 6);
        assert_eq!(line.cell(5), Some(BLANK));
        assert_eq!(line.cell(6), None);
        assert_eq!(line.chars(), vec!['a', 'b', ' ', ' ', ' ', ' ']);
        assert_eq!(lines[1].cells.len(), 0);
        assert_eq!(lines[1].cells().count(), 6);
    }

    /// Cells erased while bold is on are not blank, so they are kept, but
    /// nothing was written to them, so `used` stops before them.
    #[test]
    fn a_line_keeps_cells_past_the_last_written_one() {
        let mut parser = vt1k::Parser::new(2.try_into().unwrap(), 6.try_into().unwrap(), 0);
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
        let mut parser = vt1k::Parser::new(2.try_into().unwrap(), 6.try_into().unwrap(), 0);
        parser.process(b"$ ");
        let line = &snapshot(parser.screen_mut())[0];
        assert_eq!(line.used, 2);
        assert_eq!(line.text(0, 6), "$");
    }

    /// A line is kept as its text, with where each symbol starts only when
    /// one is not a single byte, and a run for each change of style; it
    /// reads as the cells it was made of.
    #[test]
    fn a_line_is_kept_small_and_reads_the_same() {
        let mut parser = vt1k::Parser::new(4.try_into().unwrap(), 8.try_into().unwrap(), 0);
        parser.process(b"a b~\r\n\x1b[1mbold\x1b[m\r\nx\x1b[4:3my\x1b[m\r\n\xc3\xa9");
        let screen = parser.screen();
        let lines = snapshot(screen);
        let kept: Vec<_> = lines
            .iter()
            .map(|l| (l.cells.starts.is_some(), l.cells.runs.len()))
            .collect();
        assert_eq!(kept, [(false, 0), (false, 1), (false, 1), (true, 0)]);
        for (y, line) in lines.iter().enumerate() {
            let row = screen.line_cells(y).unwrap();
            let kept = line.cells.len();
            for (x, cell) in row[..kept].iter().enumerate() {
                assert_eq!(line.cell(x), Some(cell_of(Some(cell))), "{y},{x}");
                assert_eq!(line.symbol(x), line.cell(x).map(|c| c.symbol).as_deref());
            }
        }
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
        let mut parser = vt1k::Parser::new(5.try_into().unwrap(), 12.try_into().unwrap(), 100);
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
