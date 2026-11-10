//! The terminal's modes: taken on attach, restored on the way out.
//!
//! On Unix through `ttyr`'s guard (`unix`); on Windows through crossterm
//! (`windows`), whose console modes `ttyr` does not set yet (its phase
//! 07). Both give [`Guard`] the same methods.

use std::io::{self, Write};

use tmxr_proto::KeyboardReport;
use ttyr::cursor::{ResetCursorColor, SetCursorStyle};
use ttyr::style::{Attribute, SetAttribute};

use super::probe::Probe;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix::MODIFY_OTHER_KEYS;
#[cfg(windows)]
use windows::MODIFY_OTHER_KEYS;

/// The terminal, taken by [`Guard::enter`]; dropped, it is given back as it
/// was: on detach, on error, and (through a panic hook installed once) on
/// panic.
pub struct Guard {
    probe: Probe,
    /// Whether the terminal speaks the kitty keyboard protocol, so it is
    /// asked for the flags the server says.
    kitty: bool,
    /// Whether kitty flags tmxr asked for are on the terminal's stack, for
    /// a hand-over to tell whether it popped them.
    #[cfg(unix)]
    pushed: bool,
}

impl Guard {
    /// What the terminal said of its keyboard when last asked, for the
    /// server, which decides from it what the terminal is asked for.
    pub fn keyboard(&self) -> KeyboardReport {
        KeyboardReport {
            kitty: self.kitty,
            version: self.probe.version.clone(),
            secondary: self.probe.secondary,
            modify_other_keys: MODIFY_OTHER_KEYS,
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
