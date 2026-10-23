//! A ratatui backend that renders into a byte buffer instead of a terminal.
//!
//! Each attached client gets a `ratatui::Terminal<AnsiBackend>` sized like
//! the client's terminal; ratatui's double buffering then produces only the
//! cells that changed, which this backend encodes as ANSI escapes for the
//! client to write out. Escapes are generated here directly rather than
//! through crossterm's `queue!`, because on Windows crossterm falls back to
//! console API calls when it cannot confirm ANSI support for the process's
//! own console — and the server, detached, has none.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::io;
use std::rc::Rc;

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::{Color, Modifier};

use crate::copy::Underline;

pub struct AnsiBackend {
    out: String,
    size: Size,
    cursor: Position,
    /// The colours the terminal takes; colours beyond them are sent as their
    /// nearest it has.
    depth: ColourDepth,
    /// The hyperlinks the renderer put in cells, shared with it.
    links: Rc<RefCell<Links>>,
    /// The terminal draws underline styles and colours (tmux's `usstyle`);
    /// without, any underline is sent as a plain one.
    underline_styles: bool,
}

/// What a client is sent beside a cell's style, each distinct one numbered
/// once for good: its OSC 8 hyperlink and how it is underlined beyond a
/// plain underline. A cell carries the number in ratatui's
/// `underline_color`, which nothing else uses: so the frame diff redraws a
/// cell whose decoration changed, and [`AnsiBackend`] writes it as OSC 8 and
/// SGR, never as that colour.
#[derive(Debug, Default)]
pub struct Links {
    table: Vec<Decoration>,
}

#[derive(Debug, Clone, PartialEq)]
struct Decoration {
    /// The link as OSC 8 writes it: its parameters (`id=…`) and URI.
    link: Option<(String, String)>,
    underline: Underline,
}

impl Links {
    /// The `underline_color` that carries `link` and `underline`; the
    /// default colour, carrying nothing, for neither.
    pub fn carrier(&mut self, link: Option<&vt100::Hyperlink>, underline: Underline) -> Color {
        let underline = if underline.is_plain() {
            Underline::NONE
        } else {
            underline
        };
        if link.is_none() && underline == Underline::NONE {
            return Color::Reset;
        }
        let link = link.map(|link| {
            let params = link
                .id
                .as_ref()
                .map_or_else(String::new, |id| format!("id={id}"));
            (params, link.uri.clone())
        });
        let entry = Decoration { link, underline };
        let index = self
            .table
            .iter()
            .position(|e| *e == entry)
            .unwrap_or_else(|| {
                self.table.push(entry);
                self.table.len() - 1
            });
        // Numbered from 1, so the default colour carries none.
        let [_, r, g, b] = u32::try_from(index + 1).unwrap_or(0).to_be_bytes();
        Color::Rgb(r, g, b)
    }

    /// What a cell's `underline_color` carries, if anything.
    fn of(&self, c: Color) -> Option<&Decoration> {
        let Color::Rgb(r, g, b) = c else {
            return None;
        };
        let n = u32::from_be_bytes([0, r, g, b]);
        self.table.get(usize::try_from(n).ok()?.checked_sub(1)?)
    }
}

/// How many colours a client's terminal takes (`rgb-colour`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourDepth {
    /// 24-bit colour.
    TrueColour,
    /// The xterm 256.
    Colours256,
    /// The 16 ANSI colours.
    Colours16,
}

impl AnsiBackend {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            out: String::new(),
            size: Size::new(cols, rows),
            cursor: Position::new(0, 0),
            depth: ColourDepth::TrueColour,
            underline_styles: false,
            links: Rc::default(),
        }
    }

    /// The hyperlink table, for the renderer to fill as it draws.
    pub fn links(&self) -> Rc<RefCell<Links>> {
        Rc::clone(&self.links)
    }

    /// The colours to send (`rgb-colour`).
    /// Send underline styles and colours, for a terminal that takes them.
    pub fn set_underline_styles(&mut self, on: bool) {
        self.underline_styles = on;
    }

    pub fn set_depth(&mut self, depth: ColourDepth) {
        self.depth = depth;
    }

    pub fn set_size(&mut self, cols: u16, rows: u16) {
        self.size = Size::new(cols, rows);
    }

    /// Bytes produced since the last call.
    pub fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out).into_bytes()
    }

    /// Append raw bytes (mode switches, OSC sequences) to the next output.
    pub fn push_raw(&mut self, s: &str) {
        self.out.push_str(s);
    }
}

/// The xterm-256 colour nearest `(r, g, b)`: tmux's `colour_find_rgb`
/// (colour.c), which picks the closer of the nearest 6x6x6 cube colour and
/// the nearest of the 24 greys.
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    const Q2C: [i32; 6] = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
    let to_6cube = |v: i32| match v {
        ..48 => 0,
        48..114 => 1,
        _ => (v - 35) / 40,
    };
    let dist = |(r1, g1, b1): (i32, i32, i32), (r2, g2, b2): (i32, i32, i32)| {
        (r1 - r2).pow(2) + (g1 - g2).pow(2) + (b1 - b2).pow(2)
    };
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let (qr, qg, qb) = (to_6cube(r), to_6cube(g), to_6cube(b));
    let cube = (Q2C[qr as usize], Q2C[qg as usize], Q2C[qb as usize]);
    let cube_idx = 16 + 36 * qr + 6 * qg + qb;
    if cube == (r, g, b) {
        return cube_idx as u8;
    }
    let avg = (r + g + b) / 3;
    let grey_idx = if avg > 238 { 23 } else { (avg - 3) / 10 };
    let grey = 8 + 10 * grey_idx;
    let idx = if dist((grey, grey, grey), (r, g, b)) < dist(cube, (r, g, b)) {
        232 + grey_idx
    } else {
        cube_idx
    };
    idx as u8
}

/// The ANSI colour (0 to 15) nearest xterm-256 colour `c`: tmux's
/// `colour_256to16` (colour.c), whose table this is.
pub fn nearest_16(c: u8) -> u8 {
    const TABLE: [u8; 256] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0, 4, 4, 4, 12, 12, 2, 6, 4, 4, 12,
        12, 2, 2, 6, 4, 12, 12, 2, 2, 2, 6, 12, 12, 10, 10, 10, 10, 14, 12, 10, 10, 10, 10, 10, 14,
        1, 5, 4, 4, 12, 12, 3, 8, 4, 4, 12, 12, 2, 2, 6, 4, 12, 12, 2, 2, 2, 6, 12, 12, 10, 10, 10,
        10, 14, 12, 10, 10, 10, 10, 10, 14, 1, 1, 5, 4, 12, 12, 1, 1, 5, 4, 12, 12, 3, 3, 8, 4, 12,
        12, 2, 2, 2, 6, 12, 12, 10, 10, 10, 10, 14, 12, 10, 10, 10, 10, 10, 14, 1, 1, 1, 5, 12, 12,
        1, 1, 1, 5, 12, 12, 1, 1, 1, 5, 12, 12, 3, 3, 3, 7, 12, 12, 10, 10, 10, 10, 14, 12, 10, 10,
        10, 10, 10, 14, 9, 9, 9, 9, 13, 12, 9, 9, 9, 9, 13, 12, 9, 9, 9, 9, 13, 12, 9, 9, 9, 9, 13,
        12, 11, 11, 11, 11, 7, 12, 10, 10, 10, 10, 10, 14, 9, 9, 9, 9, 9, 13, 9, 9, 9, 9, 9, 13, 9,
        9, 9, 9, 9, 13, 9, 9, 9, 9, 9, 13, 9, 9, 9, 9, 9, 13, 11, 11, 11, 11, 11, 15, 0, 0, 0, 0,
        0, 0, 8, 8, 8, 8, 8, 8, 7, 7, 7, 7, 7, 7, 15, 15, 15, 15, 15, 15,
    ];
    TABLE[usize::from(c)]
}

/// ANSI colour `n` (0 to 15) as ratatui names it.
fn ansi_colour(n: u8) -> Color {
    const NAMED: [Color; 16] = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];
    NAMED[usize::from(n & 15)]
}

fn sgr_colour(out: &mut String, c: Color, fg: bool, depth: ColourDepth) {
    let base = if fg { 30 } else { 40 };
    let c = match c {
        Color::Rgb(r, g, b) if depth != ColourDepth::TrueColour => {
            Color::Indexed(nearest_256(r, g, b))
        }
        c => c,
    };
    let c = match c {
        Color::Indexed(i) if depth == ColourDepth::Colours16 => ansi_colour(nearest_16(i)),
        c => c,
    };
    let _ = match c {
        Color::Reset => Ok(()),
        Color::Black => write!(out, ";{}", base),
        Color::Red => write!(out, ";{}", base + 1),
        Color::Green => write!(out, ";{}", base + 2),
        Color::Yellow => write!(out, ";{}", base + 3),
        Color::Blue => write!(out, ";{}", base + 4),
        Color::Magenta => write!(out, ";{}", base + 5),
        Color::Cyan => write!(out, ";{}", base + 6),
        Color::Gray => write!(out, ";{}", base + 7),
        Color::DarkGray => write!(out, ";{}", base + 60),
        Color::LightRed => write!(out, ";{}", base + 61),
        Color::LightGreen => write!(out, ";{}", base + 62),
        Color::LightYellow => write!(out, ";{}", base + 63),
        Color::LightBlue => write!(out, ";{}", base + 64),
        Color::LightMagenta => write!(out, ";{}", base + 65),
        Color::LightCyan => write!(out, ";{}", base + 66),
        Color::White => write!(out, ";{}", base + 67),
        Color::Indexed(i) => write!(out, ";{};5;{i}", base + 8),
        Color::Rgb(r, g, b) => write!(out, ";{};2;{r};{g};{b}", base + 8),
    };
}

/// The bytes that draw `text`, a format's styled runs, from the cursor in
/// at most `cols` cells: each cell's SGR, from a reset, then its character.
pub(crate) fn styled_bytes(text: &str, cols: u16) -> Vec<u8> {
    let mut line = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, cols, 1));
    let end = crate::render::draw_runs(
        &mut line,
        0,
        0,
        cols,
        ratatui::style::Style::default(),
        text,
    );
    let mut out = String::new();
    let mut x = 0;
    while x < end {
        let Some(cell) = line.cell((x, 0)) else {
            break;
        };
        out.push_str(&sgr(cell, ColourDepth::TrueColour, Underline::NONE));
        out.push_str(cell.symbol());
        // A wide character covers the cells after it.
        x += (ratatui::text::Line::raw(cell.symbol()).width() as u16).max(1);
    }
    out.into_bytes()
}

/// A full SGR sequence (starting from a reset) for one cell's style, its
/// underline's style and colour as `underline` says (sent only to a
/// terminal that takes them).
fn sgr(cell: &Cell, depth: ColourDepth, underline: Underline) -> String {
    let mut s = String::from("\x1b[0");
    let m = cell.modifier;
    for (flag, code) in [
        (Modifier::BOLD, 1),
        (Modifier::DIM, 2),
        (Modifier::ITALIC, 3),
        (Modifier::UNDERLINED, 4),
        (Modifier::SLOW_BLINK, 5),
        (Modifier::RAPID_BLINK, 6),
        (Modifier::REVERSED, 7),
        (Modifier::HIDDEN, 8),
        (Modifier::CROSSED_OUT, 9),
    ] {
        if m.contains(flag) {
            let _ = write!(s, ";{code}");
            // Another style than a single line, as tmux's Smulx: 4:n.
            if flag == Modifier::UNDERLINED && !underline.is_plain() {
                let n = underline.style.sgr();
                if n > 1 {
                    let _ = write!(s, ":{n}");
                }
            }
        }
    }
    sgr_colour(&mut s, cell.fg, true, depth);
    sgr_colour(&mut s, cell.bg, false, depth);
    if m.contains(Modifier::UNDERLINED) {
        underline_colour(&mut s, underline.colour, depth);
    }
    s.push('m');
    s
}

/// An underline colour's parameters, as tmux's Setulc / Setulc1 send them:
/// `58:2:r:g:b`, or `58:5:n` for a terminal without 24-bit colour.
fn underline_colour(out: &mut String, c: vt100::Color, depth: ColourDepth) {
    let index = match c {
        vt100::Color::Default => return,
        vt100::Color::Rgb(r, g, b) if depth == ColourDepth::TrueColour => {
            let _ = write!(out, ";58:2:{r}:{g}:{b}");
            return;
        }
        vt100::Color::Rgb(r, g, b) => nearest_256(r, g, b),
        vt100::Color::Idx(i) => i,
    };
    let index = if depth == ColourDepth::Colours16 {
        nearest_16(index)
    } else {
        index
    };
    let _ = write!(out, ";58:5:{index}");
}

impl Backend for AnsiBackend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let links = Rc::clone(&self.links);
        let links = links.borrow();
        let mut last: Option<Position> = None;
        let mut style: Option<String> = None;
        let mut link: Option<&(String, String)> = None;
        for (x, y, cell) in content {
            if !matches!(last, Some(p) if p.y == y && p.x + 1 == x) {
                let _ = write!(self.out, "\x1b[{};{}H", y + 1, x + 1);
            }
            last = Some(Position::new(x, y));
            let decoration = links.of(cell.underline_color);
            let underline = decoration
                .map(|d| d.underline)
                .filter(|_| self.underline_styles)
                .unwrap_or(Underline::NONE);
            let want = sgr(cell, self.depth, underline);
            if style.as_deref() != Some(want.as_str()) {
                self.out.push_str(&want);
                style = Some(want);
            }
            let cell_link = decoration.and_then(|d| d.link.as_ref());
            if cell_link != link {
                let (params, uri) = cell_link.map_or(("", ""), |(p, u)| (p.as_str(), u.as_str()));
                let _ = write!(self.out, "\x1b]8;{params};{uri}\x1b\\");
                link = cell_link;
            }
            self.out.push_str(cell.symbol());
        }
        if link.is_some() {
            self.out.push_str("\x1b]8;;\x1b\\");
        }
        if style.is_some() {
            self.out.push_str("\x1b[0m");
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.out.push_str("\x1b[?25l");
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.out.push_str("\x1b[?25h");
        Ok(())
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(self.cursor)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        let p = position.into();
        self.cursor = p;
        let _ = write!(self.out, "\x1b[{};{}H", p.y + 1, p.x + 1);
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        self.out.push_str("\x1b[0m\x1b[H\x1b[2J");
        Ok(())
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.out.push_str(match clear_type {
            ClearType::All => "\x1b[2J",
            ClearType::AfterCursor => "\x1b[J",
            ClearType::BeforeCursor => "\x1b[1J",
            ClearType::CurrentLine => "\x1b[2K",
            ClearType::UntilNewLine => "\x1b[K",
        });
        Ok(())
    }

    fn size(&self) -> io::Result<Size> {
        Ok(self.size)
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: self.size,
            pixels: Size::new(0, 0),
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_carried_and_read_back() {
        let mut links = Links::default();
        let a = vt100::Hyperlink {
            uri: "u".into(),
            id: None,
        };
        let b = vt100::Hyperlink {
            uri: "u".into(),
            id: Some("k".into()),
        };
        let none = Underline::NONE;
        let (ca, cb) = (links.carrier(Some(&a), none), links.carrier(Some(&b), none));
        assert_ne!(ca, cb, "an id makes another link");
        assert_eq!(
            links.carrier(Some(&a), none),
            ca,
            "the same link, the same number"
        );
        let link = |c| links.of(c).and_then(|d| d.link.clone());
        assert_eq!(link(ca), Some((String::new(), "u".to_owned())));
        assert_eq!(link(cb), Some(("id=k".to_owned(), "u".to_owned())));
        assert_eq!(links.of(Color::Reset), None);
        // A plain underline says nothing a style does not, so is not carried.
        let single = Underline {
            style: vt100::UnderlineStyle::Single,
            colour: vt100::Color::Default,
        };
        assert_eq!(links.carrier(None, single), Color::Reset);
    }

    /// A curly red underline reaches a terminal that takes underline
    /// styles as one, read back by vt100; another gets a plain underline.
    #[test]
    fn underline_styles_and_colours_reach_a_terminal_that_takes_them() {
        let area = Rect::new(0, 0, 10, 1);
        let curly = Underline {
            style: vt100::UnderlineStyle::Curly,
            colour: vt100::Color::Rgb(255, 0, 0),
        };
        let drawn = |styles: bool, depth: ColourDepth| -> (vt100::UnderlineStyle, vt100::Color) {
            let mut backend = AnsiBackend::new(10, 1);
            backend.set_underline_styles(styles);
            backend.set_depth(depth);
            let links = backend.links();
            let mut term = Terminal::with_options(
                backend,
                TerminalOptions {
                    viewport: Viewport::Fixed(area),
                },
            )
            .unwrap();
            term.draw(|f| {
                let buf = f.buffer_mut();
                buf.set_string(
                    0,
                    0,
                    "x",
                    Style::default().add_modifier(Modifier::UNDERLINED),
                );
                if let Some(cell) = buf.cell_mut((0, 0)) {
                    cell.underline_color = links.borrow_mut().carrier(None, curly);
                }
            })
            .unwrap();
            let mut vt = vt100::Parser::new(
                std::num::NonZeroU16::MIN,
                std::num::NonZeroU16::new(10).unwrap(),
                0,
            );
            vt.process(&term.backend_mut().take());
            let cell = vt.screen().cell(0, 0).unwrap().clone();
            (cell.underline_style(), cell.underline_color())
        };
        assert_eq!(
            drawn(true, ColourDepth::TrueColour),
            (vt100::UnderlineStyle::Curly, vt100::Color::Rgb(255, 0, 0))
        );
        assert_eq!(
            drawn(true, ColourDepth::Colours256),
            (vt100::UnderlineStyle::Curly, vt100::Color::Idx(196))
        );
        assert_eq!(
            drawn(false, ColourDepth::TrueColour),
            (vt100::UnderlineStyle::Single, vt100::Color::Default)
        );
    }
    use ratatui::Terminal;
    use ratatui::TerminalOptions;
    use ratatui::Viewport;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    #[test]
    fn output_reproduces_the_frame_in_a_real_emulator() {
        let area = Rect::new(0, 0, 20, 3);
        let mut term = Terminal::with_options(
            AnsiBackend::new(20, 3),
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )
        .unwrap();
        term.draw(|f| {
            let buf = f.buffer_mut();
            buf.set_string(0, 0, "hello", Style::default().fg(Color::Rgb(1, 2, 3)));
            buf.set_string(2, 2, "wide 漢字", Style::default().bg(Color::Indexed(200)));
        })
        .unwrap();
        let bytes = term.backend_mut().take();

        let size = |n| std::num::NonZeroU16::new(n).unwrap();
        let mut vt = vt100::Parser::new(size(3), size(20), 0);
        vt.process(&bytes);
        let s = vt.screen();
        assert_eq!(s.contents_between(0, 0, 0, 20).trim_end(), "hello");
        assert_eq!(s.contents_between(2, 2, 2, 20).trim_end(), "wide 漢字");
        assert_eq!(s.cell(0, 0).unwrap().fgcolor(), vt100::Color::Rgb(1, 2, 3));
        assert_eq!(s.cell(2, 2).unwrap().bgcolor(), vt100::Color::Idx(200));

        // A second identical frame sends no cells at all.
        term.draw(|f| {
            let buf = f.buffer_mut();
            buf.set_string(0, 0, "hello", Style::default().fg(Color::Rgb(1, 2, 3)));
            buf.set_string(2, 2, "wide 漢字", Style::default().bg(Color::Indexed(200)));
        })
        .unwrap();
        let again = String::from_utf8(term.backend_mut().take()).unwrap();
        assert!(!again.contains("hello"), "{again:?}");
    }

    #[test]
    fn nearest_256_matches_tmux() {
        // Cube corners and exact cube colours map to themselves.
        assert_eq!(nearest_256(0, 0, 0), 16);
        assert_eq!(nearest_256(255, 255, 255), 231);
        assert_eq!(nearest_256(255, 0, 0), 196);
        assert_eq!(nearest_256(0x5f, 0x87, 0xaf), 67);
        // Mid grey is nearer a grey ramp entry than any cube colour.
        assert_eq!(nearest_256(128, 128, 128), 244);
        assert_eq!(nearest_256(8, 8, 8), 232);
        // Near black stays on the cube.
        assert_eq!(nearest_256(1, 2, 3), 16);
        // Tokyo Night's blue, #7aa2f7.
        assert_eq!(nearest_256(0x7a, 0xa2, 0xf7), 111);
    }

    #[test]
    fn rgb_off_sends_the_nearest_of_the_256() {
        let area = Rect::new(0, 0, 4, 1);
        let mut term = Terminal::with_options(
            AnsiBackend::new(4, 1),
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )
        .unwrap();
        term.backend_mut().set_depth(ColourDepth::Colours256);
        term.draw(|f| {
            let style = Style::default().fg(Color::Rgb(255, 0, 0));
            f.buffer_mut().set_string(0, 0, "x", style);
        })
        .unwrap();
        let out = String::from_utf8(term.backend_mut().take()).unwrap();
        assert!(out.contains(";38;5;196"), "{out:?}");
        assert!(!out.contains(";38;2;"), "{out:?}");
    }

    #[test]
    fn sixteen_colours_send_the_nearest_ansi_colour() {
        // Pure red, blue, green, yellow and white, the darkest and
        // lightest greys, and the first 16 as themselves.
        for (c, want) in [(196, 9), (21, 12), (46, 10), (226, 11), (231, 15)] {
            assert_eq!(nearest_16(c), want, "colour {c}");
        }
        assert_eq!((nearest_16(232), nearest_16(255)), (0, 15));
        assert!((0..16).all(|c| nearest_16(c) == c));

        let area = Rect::new(0, 0, 4, 1);
        let mut term = Terminal::with_options(
            AnsiBackend::new(4, 1),
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )
        .unwrap();
        term.backend_mut().set_depth(ColourDepth::Colours16);
        term.draw(|f| {
            let style = Style::default()
                .fg(Color::Rgb(255, 0, 0))
                .bg(Color::Indexed(21));
            f.buffer_mut().set_string(0, 0, "x", style);
        })
        .unwrap();
        let out = String::from_utf8(term.backend_mut().take()).unwrap();
        assert!(out.contains(";91;104"), "{out:?}");
        assert!(!out.contains(";38;") && !out.contains(";48;"), "{out:?}");
    }
}
