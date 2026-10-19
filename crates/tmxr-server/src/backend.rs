//! A ratatui backend that renders into a byte buffer instead of a terminal.
//!
//! Each attached client gets a `ratatui::Terminal<AnsiBackend>` sized like
//! the client's terminal; ratatui's double buffering then produces only the
//! cells that changed, which this backend encodes as ANSI escapes for the
//! client to write out. Escapes are generated here directly rather than
//! through crossterm's `queue!`, because on Windows crossterm falls back to
//! console API calls when it cannot confirm ANSI support for the process's
//! own console — and the server, detached, has none.

use std::fmt::Write as _;
use std::io;

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::{Color, Modifier};

pub struct AnsiBackend {
    out: String,
    size: Size,
    cursor: Position,
    /// The colours the terminal takes; colours beyond them are sent as their
    /// nearest it has.
    depth: ColourDepth,
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
        }
    }

    /// The colours to send (`rgb-colour`).
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
        out.push_str(&sgr(cell, ColourDepth::TrueColour));
        out.push_str(cell.symbol());
        // A wide character covers the cells after it.
        x += (ratatui::text::Line::raw(cell.symbol()).width() as u16).max(1);
    }
    out.into_bytes()
}

/// A full SGR sequence (starting from a reset) for one cell's style.
fn sgr(cell: &Cell, depth: ColourDepth) -> String {
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
        }
    }
    sgr_colour(&mut s, cell.fg, true, depth);
    sgr_colour(&mut s, cell.bg, false, depth);
    s.push('m');
    s
}

impl Backend for AnsiBackend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let mut last: Option<Position> = None;
        let mut style: Option<String> = None;
        for (x, y, cell) in content {
            if !matches!(last, Some(p) if p.y == y && p.x + 1 == x) {
                let _ = write!(self.out, "\x1b[{};{}H", y + 1, x + 1);
            }
            last = Some(Position::new(x, y));
            let want = sgr(cell, self.depth);
            if style.as_deref() != Some(want.as_str()) {
                self.out.push_str(&want);
                style = Some(want);
            }
            self.out.push_str(cell.symbol());
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

        let mut vt = vt100::Parser::new(3, 20, 0);
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
