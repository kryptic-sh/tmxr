//! The Unix terminal's modes, through `ttyr`'s guard, which records each
//! mode it turns on and turns them off again in reverse: on the way out,
//! from its panic hook, and around a suspend or a lock.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use ttyr::event::{Event, KeyboardEnhancementFlags};
use ttyr::terminal::{
    self as tty, DisableMouseCapture, EnableMouseCapture, Mode, RequestColorScheme,
};

use super::{Guard, write_resets};
use crate::terminal::Keys;

#[cfg(test)]
mod tests;

/// Whether the mouse is captured. Written apart from `ttyr`'s guard, whose
/// mouse mode turns on one level of reports where tmxr asks for every one
/// (1000, 1002 and 1003), so turned off here.
static MOUSE: AtomicBool = AtomicBool::new(false);

/// Whether the screen is taken, so [`write_resets`] is owed on the way out.
static TAKEN: AtomicBool = AtomicBool::new(false);

/// Install `ttyr`'s panic hook, which turns its recorded modes and raw
/// mode off, and tmxr's over it, which gives back what tmxr wrote itself
/// and then runs `ttyr`'s. Once, however many times the terminal is taken.
fn restore_on_panic() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        tty::restore_on_panic();
        let ttyr_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            give_back_after_panic();
            ttyr_hook(info);
        }));
    });
}

/// [`give_back`] from the panic hook: to the controlling terminal, as
/// `ttyr`'s hook writes, not to stdout, whose lock the panicking thread
/// may hold. A failure has nowhere to be reported.
fn give_back_after_panic() {
    let (mouse, taken) = (
        MOUSE.swap(false, Ordering::AcqRel),
        TAKEN.swap(false, Ordering::AcqRel),
    );
    if !mouse && !taken {
        return;
    }
    if let Ok(mut tty) = OpenOptions::new().write(true).open("/dev/tty") {
        let _ = give_back(&mut tty, mouse, taken);
    }
}

/// Turn the mouse off where `mouse`, and write the resets where `taken`.
fn give_back(out: &mut impl Write, mouse: bool, taken: bool) -> io::Result<()> {
    if mouse {
        ttyr::queue!(out, DisableMouseCapture)?;
    }
    if taken {
        write_resets(out)?;
    }
    out.flush()
}

/// The kitty keyboard `flags` as `ttyr` takes them.
fn keyboard_flags(flags: u8) -> Mode {
    Mode::KeyboardFlags(KeyboardEnhancementFlags::from_bits_truncate(flags))
}

impl Guard {
    /// Take the terminal: raw mode, the alternate screen, bracketed paste
    /// and the colour scheme reports, asking it meanwhile what it speaks
    /// through `keys`. No kitty keyboard flags until the server says which
    /// ([`Guard::set_keyboard_flags`]). Returns the guard with the input
    /// read while the terminal answered, for the server.
    pub fn enter(keys: &mut Keys) -> io::Result<(Self, Vec<Event>)> {
        restore_on_panic();
        tty::enable_raw_mode()?;
        // From here an error, or a panic as it unwinds, drops the guard and
        // so gives the terminal back.
        let mut guard = Self {
            probe: Default::default(),
            kitty: false,
        };
        let mut out = io::stdout();
        TAKEN.store(true, Ordering::Release);
        // On the alternate screen before the probe, so a terminal that shows
        // a query it does not know leaves it there, not in the scrollback.
        // Mouse capture and focus reports wait for the server's `Mouse` and
        // `FocusEvents` (the `mouse` and `focus-events` options).
        tty::enable(&mut out, Mode::AlternateScreen)?;
        tty::enable(&mut out, Mode::BracketedPaste)?;
        let (probe, early) = keys.probe(&mut out)?;
        guard.kitty = probe.kitty;
        guard.probe = probe;
        // The terminal's color scheme now and on each change (mode 2031), as
        // tmux asks as its terminal starts.
        tty::enable(&mut out, Mode::ColorSchemeReports)?;
        ttyr::queue!(out, RequestColorScheme)?;
        out.flush()?;
        Ok((guard, early))
    }

    /// Give the terminal to `run` (a suspend, the lock command) and take it
    /// back, as `ttyr`'s `suspend_with` does, then ask it again what it
    /// speaks through `keys`. A terminal that still speaks the kitty
    /// protocol keeps the `flags` the server last sent, if any; one that no
    /// longer does is asked for none, and one that newly does waits for the
    /// server. Returns what `run` returned and the input read as the
    /// terminal answered.
    pub fn hand_over<T>(
        &mut self,
        keys: &mut Keys,
        flags: Option<u8>,
        run: impl FnOnce() -> T,
    ) -> io::Result<(T, Vec<Event>)> {
        let mut out = io::stdout();
        // Off for good: the server tells the client both again once it is
        // back (`Resumed`), from what it wants then.
        self.set_mouse(false)?;
        tty::disable(&mut out, Mode::FocusChange)?;
        give_back(&mut out, false, TAKEN.swap(false, Ordering::AcqRel))?;
        let back = tty::suspend_with(&mut out, run)?;
        // Raw mode and every recorded mode are on again, the kitty flags
        // among them; the resets are owed again.
        TAKEN.store(true, Ordering::Release);
        let (probe, early) = keys.probe(&mut out)?;
        ttyr::queue!(out, RequestColorScheme)?;
        let kept = self.kitty && probe.kitty;
        self.kitty = probe.kitty;
        match flags.filter(|_| kept) {
            Some(flags) => tty::enable(&mut out, keyboard_flags(flags))?,
            None => tty::disable(&mut out, keyboard_flags(0))?,
        }
        self.probe = probe;
        out.flush()?;
        Ok((back, early))
    }

    /// Ask the terminal for kitty keyboard `flags` in place of those it has
    /// (`ttyr`'s guard pops them and pushes these; the first time, it only
    /// pushes), as the server says. A terminal without the protocol is left
    /// alone.
    pub fn set_keyboard_flags(&self, flags: u8) -> io::Result<()> {
        if !self.kitty {
            return Ok(());
        }
        let mut out = io::stdout();
        tty::enable(&mut out, keyboard_flags(flags))?;
        out.flush()
    }

    /// Capture the mouse (every button, drag and motion, in SGR's form), or
    /// hand it back to the terminal.
    pub fn set_mouse(&self, on: bool) -> io::Result<()> {
        let mut out = io::stdout();
        // Recorded first, so that a write that fails part way is still
        // turned off on the way out.
        MOUSE.store(on, Ordering::Release);
        if on {
            ttyr::execute!(out, EnableMouseCapture)
        } else {
            ttyr::execute!(out, DisableMouseCapture)
        }
    }

    /// Ask the terminal for focus reports, or stop them.
    pub fn set_focus_events(&self, on: bool) -> io::Result<()> {
        let mut out = io::stdout();
        if on {
            tty::enable(&mut out, Mode::FocusChange)?;
        } else {
            tty::disable(&mut out, Mode::FocusChange)?;
        }
        out.flush()
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let mut out = io::stdout();
        // Nowhere to report a failure: the terminal is likely gone.
        let _ = give_back(
            &mut out,
            MOUSE.swap(false, Ordering::AcqRel),
            TAKEN.swap(false, Ordering::AcqRel),
        );
        let _ = tty::restore(&mut out);
    }
}
