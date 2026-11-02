//! Synchronized output (DEC mode 2026): a program brackets a redraw in
//! `CSI ? 2026 h` … `CSI ? 2026 l`, and its pane goes on showing what it
//! showed at the `h` until the `l`, so a half-drawn frame never reaches the
//! terminal, as tmux 3.8's screen_write_start_sync / _stop_sync hold a
//! pane's redraw. tmux also ends a hold on any full redraw of the pane
//! (screen-redraw.c); tmxr holds through those until the `l` or the
//! timeout.

use std::num::NonZeroU16;
use std::time::{Duration, Instant};

/// The longest a program holds its pane's redraw without a `CSI ? 2026 l`,
/// from its last `CSI ? 2026 h`, as tmux's `sync_timer`.
pub const SYNC_TIMEOUT: Duration = Duration::from_secs(1);

/// The screen as it was when its program began holding the redraw: the
/// cells in view, the hyperlinks they name and the cursor.
#[derive(Debug, Clone)]
pub struct Frame {
    cols: u16,
    cells: Vec<Option<vt1k::Cell>>,
    links: Vec<(NonZeroU16, vt1k::Hyperlink)>,
    cursor: (u16, u16),
    hide_cursor: bool,
    cursor_style: u8,
    cursor_colour: Option<u32>,
}

impl Frame {
    /// What `screen` shows now, its cursor `cursor_colour`.
    pub(crate) fn of(screen: &vt1k::Screen, cursor_colour: Option<u32>) -> Self {
        let (rows, cols) = crate::screen_size(screen);
        let cells: Vec<Option<vt1k::Cell>> = (0..rows)
            .flat_map(|row| (0..cols).map(move |col| screen.cell(row, col).cloned()))
            .collect();
        let mut links: Vec<(NonZeroU16, vt1k::Hyperlink)> = Vec::new();
        for id in cells.iter().flatten().filter_map(vt1k::Cell::hyperlink_id) {
            if links.iter().all(|(seen, _)| *seen != id)
                && let Some(link) = screen.hyperlink(id)
            {
                links.push((id, link.clone()));
            }
        }
        Self {
            cols,
            cells,
            links,
            cursor: screen.cursor_position(),
            hide_cursor: screen.hide_cursor(),
            cursor_style: screen.cursor_style().decscusr(),
            cursor_colour,
        }
    }
}

/// A hold in progress: the frame shown, and when it ends without a
/// `CSI ? 2026 l`.
#[derive(Debug)]
pub(crate) struct Hold {
    pub(crate) frame: Frame,
    pub(crate) until: Instant,
}

/// What a pane shows: its screen as it is, or the frame its program holds.
#[derive(Clone, Copy)]
pub enum Shown<'a> {
    Live(&'a crate::Emulator),
    Held(&'a Frame),
}

impl<'a> Shown<'a> {
    /// The cell at `(row, col)`, as [`vt1k::Screen::cell`].
    pub fn cell(self, row: u16, col: u16) -> Option<&'a vt1k::Cell> {
        match self {
            Self::Live(emu) => emu.screen().cell(row, col),
            Self::Held(frame) => {
                if col >= frame.cols {
                    return None;
                }
                let at = usize::from(row) * usize::from(frame.cols) + usize::from(col);
                frame.cells.get(at)?.as_ref()
            }
        }
    }

    /// The hyperlink a cell's [`vt1k::Cell::hyperlink_id`] names.
    pub fn hyperlink(self, id: NonZeroU16) -> Option<&'a vt1k::Hyperlink> {
        match self {
            Self::Live(emu) => emu.screen().hyperlink(id),
            Self::Held(frame) => frame
                .links
                .iter()
                .find(|(seen, _)| *seen == id)
                .map(|(_, link)| link),
        }
    }

    /// The cursor's row and column.
    pub fn cursor_position(self) -> (u16, u16) {
        match self {
            Self::Live(emu) => emu.screen().cursor_position(),
            Self::Held(frame) => frame.cursor,
        }
    }

    pub fn hide_cursor(self) -> bool {
        match self {
            Self::Live(emu) => emu.screen().hide_cursor(),
            Self::Held(frame) => frame.hide_cursor,
        }
    }

    /// The cursor shape the program asked for, as DECSCUSR's `Ps`.
    pub fn cursor_style(self) -> u8 {
        match self {
            Self::Live(emu) => emu.cursor_style(),
            Self::Held(frame) => frame.cursor_style,
        }
    }

    /// The cursor's colour the program set, as RGB.
    pub fn cursor_colour(self) -> Option<u32> {
        match self {
            Self::Live(emu) => emu.cursor_colour(),
            Self::Held(frame) => frame.cursor_colour,
        }
    }
}
