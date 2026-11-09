//! The Windows console's modes, through crossterm, which sets the console's
//! own modes where `ttyr` (at its pinned rev) has no Windows side yet. The
//! console has no kitty keyboard protocol ([`Keys::probe`] says so), so no
//! flags are pushed here.

use std::io;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ttyr::event::Event;
use ttyr::terminal::{DisableColorSchemeReports, EnableColorSchemeReports, RequestColorScheme};

use super::{Guard, write_resets};
use crate::terminal::Keys;

/// Whether raw mode is on.
static RAW: AtomicBool = AtomicBool::new(false);

/// Whether the screen is taken, so its modes are turned off on the way out.
static TAKEN: AtomicBool = AtomicBool::new(false);

/// Install the panic hook that gives the terminal back, then runs the hook
/// that was there before. Once, however many times the terminal is taken.
fn restore_on_panic() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            give_back();
            prev(info);
        }));
    });
}

/// Turn off what is on, once: the screen's modes, then raw mode. Nowhere to
/// report a failure: the terminal is likely gone.
fn give_back() {
    let mut out = io::stdout();
    if TAKEN.swap(false, Ordering::AcqRel) {
        let _ = execute!(
            out,
            DisableMouseCapture,
            DisableBracketedPaste,
            DisableFocusChange,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        let _ = write_resets(&mut out);
        let _ = ttyr::execute!(out, DisableColorSchemeReports);
    }
    if RAW.swap(false, Ordering::AcqRel) {
        let _ = disable_raw_mode();
    }
}

/// Put the console in raw mode, recorded for whichever guard or panic hook
/// gives it back.
fn raw_mode() -> io::Result<()> {
    restore_on_panic();
    enable_raw_mode()?;
    RAW.store(true, Ordering::Release);
    Ok(())
}

impl Guard {
    /// Take the console: raw mode, the alternate screen, bracketed paste
    /// and the colour scheme reports, asking it meanwhile what it speaks
    /// through `keys`. Returns the guard with the input read meanwhile.
    pub fn enter(keys: &mut Keys) -> io::Result<(Self, Vec<Event>)> {
        raw_mode()?;
        // From here an error, or a panic as it unwinds, drops the guard and
        // so gives the console back.
        let mut guard = Self {
            probe: Default::default(),
            kitty: false,
        };
        let early = guard.take_screen(keys)?;
        Ok((guard, early))
    }

    /// The screen's modes, with the probe on the alternate screen, as on
    /// Unix; returns the input read meanwhile.
    fn take_screen(&mut self, keys: &mut Keys) -> io::Result<Vec<Event>> {
        let mut out = io::stdout();
        TAKEN.store(true, Ordering::Release);
        // Mouse capture and focus reports wait for the server's `Mouse` and
        // `FocusEvents` (the `mouse` and `focus-events` options).
        execute!(out, EnterAlternateScreen, EnableBracketedPaste)?;
        let (probe, early) = keys.probe(&mut out)?;
        self.kitty = probe.kitty;
        self.probe = probe;
        // The terminal's color scheme now and on each change (mode 2031), as
        // tmux asks as its terminal starts.
        ttyr::execute!(out, EnableColorSchemeReports, RequestColorScheme)?;
        Ok(early)
    }

    /// Give the console to `run` (the lock command) and take it back, asking
    /// it again what it speaks. `flags` are for a terminal with the kitty
    /// protocol, which the console is not. Returns what `run` returned and
    /// the input read meanwhile.
    pub fn hand_over<T>(
        &mut self,
        keys: &mut Keys,
        _flags: Option<u8>,
        run: impl FnOnce() -> T,
    ) -> io::Result<(T, Vec<Event>)> {
        give_back();
        let back = run();
        raw_mode()?;
        let early = self.take_screen(keys)?;
        Ok((back, early))
    }

    /// The console has no kitty keyboard protocol: nothing to ask for.
    pub fn set_keyboard_flags(&self, _flags: u8) -> io::Result<()> {
        Ok(())
    }

    /// Capture the mouse, or hand it back to the console.
    pub fn set_mouse(&self, on: bool) -> io::Result<()> {
        if on {
            execute!(io::stdout(), EnableMouseCapture)
        } else {
            execute!(io::stdout(), DisableMouseCapture)
        }
    }

    /// Ask the console for focus reports, or stop them.
    pub fn set_focus_events(&self, on: bool) -> io::Result<()> {
        if on {
            execute!(io::stdout(), EnableFocusChange)
        } else {
            execute!(io::stdout(), DisableFocusChange)
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        give_back();
    }
}
