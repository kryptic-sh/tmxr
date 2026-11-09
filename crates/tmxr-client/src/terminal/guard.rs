//! The terminal's modes: taken on attach, restored on the way out.

use std::io::{self, Write};

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ttyr::event::Event;

use super::Keys;
use super::probe::Probe;

/// Restores the terminal when dropped — on detach, on error, and (through
/// the panic hook installed in [`Guard::enter`]) on panic.
pub struct Guard {
    kitty: bool,
    probe: Probe,
}

fn restore(kitty: bool) {
    let mut out = io::stdout();
    if kitty {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(
        out,
        DisableMouseCapture,
        DisableBracketedPaste,
        DisableFocusChange,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    // Reset attributes and the cursor shape and colour the panes may have
    // changed, and stop the color-scheme reports.
    let _ = out.write_all(b"\x1b[0m\x1b[0 q\x1b]112\x07\x1b[?2031l");
    let _ = out.flush();
    let _ = disable_raw_mode();
}

impl Guard {
    /// Take the terminal, asking it what it speaks through `keys` and for
    /// the kitty keyboard `flags` where it speaks the protocol: on attach
    /// tmxr's own, and on the way back from a suspend or a lock the last
    /// the server sent. Returns the guard with the input read while the
    /// terminal answered, for the server.
    pub fn enter(flags: u8, keys: &mut Keys) -> io::Result<(Self, Vec<Event>)> {
        enable_raw_mode()?;
        // From here an error, or a panic as it unwinds, drops the guard and
        // so restores the terminal.
        let mut guard = Self {
            kitty: false,
            probe: Probe::default(),
        };
        let mut out = io::stdout();
        // Mouse capture and focus reports wait for the server's `Mouse` and
        // `FocusEvents` (the `mouse` and `focus-events` options).
        execute!(out, EnterAlternateScreen, EnableBracketedPaste)?;
        let (probe, early) = keys.probe(&mut out)?;
        // The terminal's color scheme now and on each change (mode 2031), as
        // tmux asks as its terminal starts.
        out.write_all(b"\x1b[?2031h\x1b[?996n")?;
        // Pushed even when none, so that restoring pops what this pushed
        // and leaves the terminal's own flags as they were.
        if probe.kitty {
            execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::from_bits_truncate(flags))
            )?;
            guard.kitty = true;
        }
        guard.probe = probe;
        out.flush()?;
        let kitty = guard.kitty;
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore(kitty);
            prev(info);
        }));
        Ok((guard, early))
    }

    /// Ask the terminal for kitty keyboard `flags` in place of those it has
    /// (the protocol's set, `CSI = flags ; 1 u`), as the server says; a
    /// terminal without the protocol is left alone.
    pub fn set_keyboard_flags(&self, flags: u8) -> io::Result<()> {
        if !self.kitty {
            return Ok(());
        }
        let mut out = io::stdout();
        write!(out, "\x1b[={flags};1u")?;
        out.flush()
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

impl Drop for Guard {
    fn drop(&mut self) {
        restore(self.kitty);
    }
}

/// Capture the mouse, or hand it back to the terminal.
pub fn set_mouse(on: bool) -> io::Result<()> {
    if on {
        execute!(io::stdout(), EnableMouseCapture)
    } else {
        execute!(io::stdout(), DisableMouseCapture)
    }
}

pub fn set_focus_events(on: bool) -> io::Result<()> {
    if on {
        execute!(io::stdout(), EnableFocusChange)
    } else {
        execute!(io::stdout(), DisableFocusChange)
    }
}
