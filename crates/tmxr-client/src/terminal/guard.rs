//! The terminal's modes: taken on attach, restored on the way out.
//!
//! On Unix through `ttyr`'s guard (`unix`); on Windows through crossterm
//! (`windows`), whose console modes `ttyr` does not set yet (its phase
//! 07). Both give [`Guard`] the same methods.

use std::io::{self, Write};

use tmxr_proto::Keyboard;
use ttyr::cursor::{ResetCursorColor, SetCursorStyle};
use ttyr::style::{Attribute, SetAttribute};

use super::probe::Probe;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

/// The terminal, taken by [`Guard::enter`]; dropped, it is given back as it
/// was: on detach, on error, and (through a panic hook installed once) on
/// panic.
pub struct Guard {
    probe: Probe,
    /// Whether the terminal speaks the kitty keyboard protocol, so it is
    /// asked for the flags the server says.
    kitty: bool,
}

impl Guard {
    /// The keyboard tier the terminal has, for the server. Never
    /// [`Keyboard::Xterm`]: telling modifyOtherKeys apart takes the
    /// terminal's name (plan 18 step 5), so such a terminal is legacy here.
    pub fn keyboard(&self) -> Keyboard {
        if self.kitty {
            Keyboard::Kitty
        } else {
            Keyboard::Legacy
        }
    }

    /// The terminal's default foreground and background colours, as RGB,
    /// where it said them.
    pub fn colours(&self) -> (Option<u32>, Option<u32>) {
        (self.probe.fg, self.probe.bg)
    }

    /// Write one frame from the server, as one synchronized update where
    /// the terminal takes them, so it is never shown half drawn.
    pub fn write_frame(&self, out: &mut impl Write, frame: &[u8]) -> io::Result<()> {
        if self.probe.sync {
            out.write_all(b"\x1b[?2026h")?;
        }
        out.write_all(frame)?;
        if self.probe.sync {
            out.write_all(b"\x1b[?2026l")?;
        }
        out.flush()
    }
}

/// Put back what the panes may have changed and no mode turns off: the
/// attributes, and the cursor's shape and colour.
fn write_resets(out: &mut impl Write) -> io::Result<()> {
    ttyr::queue!(
        out,
        SetAttribute(Attribute::Reset),
        SetCursorStyle::DefaultUserShape,
        ResetCursorColor,
    )
}
