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
}

impl AnsiBackend {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            out: String::new(),
            size: Size::new(cols, rows),
            cursor: Position::new(0, 0),
        }
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

fn sgr_colour(out: &mut String, c: Color, fg: bool) {
    let base = if fg { 30 } else { 40 };
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

/// A full SGR sequence (starting from a reset) for one cell's style.
fn sgr(cell: &Cell) -> String {
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
    sgr_colour(&mut s, cell.fg, true);
    sgr_colour(&mut s, cell.bg, false);
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
            let want = sgr(cell);
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
}
