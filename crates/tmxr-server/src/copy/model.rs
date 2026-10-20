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
            lines[idx] = Some(Line {
                cells,
                wrapped: screen.row_wrapped(r),
                used,
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
                used: 0,
            })
        })
        .collect()
}
