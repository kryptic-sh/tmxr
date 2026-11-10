//! The terminal's modes: taken on attach, restored on the way out, through
//! `ttyr`'s guard, which records each mode it turns on and turns them off
//! again in reverse: on the way out, from its panic hook, and around a
//! suspend or a lock. One code path on Unix and Windows; they differ in
//! which terminal raw mode is set on, and in how the Windows console hands
//! over its input ([`Keys::reads_vt`]).

use std::fs::File;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once, PoisonError, TryLockError};

use tmxr_proto::KeyboardReport;
use ttyr::cursor::{ResetCursorColor, SetCursorStyle};
use ttyr::event::{Event, KeyboardEnhancementFlags};
use ttyr::style::{Attribute, SetAttribute};
use ttyr::terminal::{self as tty, Mode, RequestColorScheme};

use super::Keys;
use super::probe::Probe;

#[cfg(all(test, unix))]
mod tests;

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
    pushed: bool,
    /// Whether the client reads the bytes the terminal sends
    /// ([`Keys::reads_vt`]), so it can read xterm's modifyOtherKeys keys.
    vt: bool,
}

/// Whether the mouse is captured. On Unix it is written apart from
/// `ttyr`'s guard, whose mouse mode turns on one level of reports where
/// tmxr asks for every one (1000, 1002 and 1003), so turned off here.
static MOUSE: AtomicBool = AtomicBool::new(false);

/// Whether the screen is taken, so [`write_resets`] is owed on the way out.
static TAKEN: AtomicBool = AtomicBool::new(false);

/// The terminal taken, for the panic hook to write to: on Unix the one raw
/// mode is on, which need not be the controlling terminal; on Windows the
/// console's output.
static TERMINAL: Mutex<Option<File>> = Mutex::new(None);

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

/// [`give_back`] from the panic hook: to the terminal taken, as `ttyr`'s
/// hook writes, not to stdout, whose lock the panicking thread may hold.
/// Skipped if the panicking thread holds [`TERMINAL`] (a panic while it was
/// taken); a failure has nowhere to be reported.
fn give_back_after_panic() {
    let (mouse, taken) = (
        MOUSE.swap(false, Ordering::AcqRel),
        TAKEN.swap(false, Ordering::AcqRel),
    );
    if !mouse && !taken {
        return;
    }
    let mut terminal = match TERMINAL.try_lock() {
        Ok(terminal) => terminal,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    if let Some(tty) = terminal.as_mut() {
        let _ = give_back(tty, mouse, taken);
    }
}

/// Turn the mouse off where `mouse`, and write the resets where `taken`.
fn give_back(out: &mut impl Write, mouse: bool, taken: bool) -> io::Result<()> {
    if mouse {
        ttyr::queue!(out, tty::DisableMouseCapture)?;
    }
    if taken {
        write_resets(out)?;
    }
    out.flush()
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

/// The kitty keyboard `flags` as `ttyr` takes them.
fn keyboard_flags(flags: u8) -> Mode {
    Mode::KeyboardFlags(KeyboardEnhancementFlags::from_bits_truncate(flags))
}

/// Raw mode on the terminal `keys` reads, which need not be the
/// controlling one, and that terminal kept for the panic hook.
#[cfg(unix)]
fn raw_mode(keys: &Keys) -> io::Result<()> {
    *TERMINAL.lock().unwrap_or_else(PoisonError::into_inner) =
        Some(File::from(keys.tty.try_clone()?));
    tty::enable_raw_mode_on(&keys.tty)
}

/// Raw mode on the console, whose output is kept for the panic hook; and
/// where the client reads the terminal's bytes ([`Keys::reads_vt`]), VT
/// input with Windows Terminal's win32-input-mode, so keys come as console
/// key records (releases, AltGr and Ctrl+4 kept) and the rest (the mouse,
/// the probe's answers, colour reports) as the terminal sends them.
#[cfg(windows)]
fn raw_mode(keys: &Keys) -> io::Result<()> {
    *TERMINAL.lock().unwrap_or_else(PoisonError::into_inner) =
        Some(File::options().write(true).open("CONOUT$")?);
    tty::enable_raw_mode()?;
    if keys.reads_vt() {
        let mut out = io::stdout();
        tty::enable(&mut out, Mode::VirtualTerminalInput)?;
        tty::enable(&mut out, Mode::Win32Input)?;
        out.flush()?;
    }
    Ok(())
}

impl Guard {
    /// Take the terminal: raw mode, the alternate screen, bracketed paste
    /// and the colour scheme reports, asking it meanwhile what it speaks
    /// through `keys`. No kitty keyboard flags until the server says which
    /// ([`Guard::set_keyboard_flags`]). Returns the guard with the input
    /// read while the terminal answered, for the server.
    pub fn enter(keys: &mut Keys) -> io::Result<(Self, Vec<Event>)> {
        restore_on_panic();
        // From here an error, or a panic as it unwinds, drops the guard and
        // so gives the terminal back.
        let mut guard = Self {
            probe: Default::default(),
            kitty: false,
            pushed: false,
            vt: keys.reads_vt(),
        };
        raw_mode(keys)?;
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
        // tmux asks as its terminal starts; only where its reports can be
        // read, not by a console handing over key records.
        if guard.vt {
            tty::enable(&mut out, Mode::ColorSchemeReports)?;
            ttyr::queue!(out, RequestColorScheme)?;
        }
        out.flush()?;
        Ok((guard, early))
    }

    /// What the terminal said of its keyboard when last asked, for the
    /// server, which decides from it what the terminal is asked for.
    pub fn keyboard(&self) -> KeyboardReport {
        KeyboardReport {
            kitty: self.kitty,
            version: self.probe.version.clone(),
            secondary: self.probe.secondary,
            modify_other_keys: self.vt,
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

    /// Give the terminal to `run` (a suspend, the lock command) and take it
    /// back, as `ttyr`'s `suspend_with` does, then ask it again what it
    /// speaks through `keys`. A terminal that still speaks the kitty
    /// protocol keeps the `flags` the server last sent, if any; one that no
    /// longer does is asked for none, and one that newly does waits for the
    /// server. modifyOtherKeys, on again with the rest, stays on where the
    /// terminal said the same as before, and goes off where it said
    /// otherwise, until the server, told, says again. Returns what `run`
    /// returned and the input read as the terminal answered.
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
        let before = self.keyboard();
        let (probe, early) = keys.probe(&mut out)?;
        if self.vt {
            ttyr::queue!(out, RequestColorScheme)?;
        }
        let kept = self.kitty && probe.kitty;
        self.kitty = probe.kitty;
        // The probe heard the flags `suspend_with` put back, which are then
        // replaced or popped: the reader is told what is on now. Where none
        // were pushed, what the terminal answered stands, as at attach.
        match flags.filter(|_| kept) {
            Some(flags) => {
                tty::enable(&mut out, keyboard_flags(flags))?;
                self.pushed = true;
                keys.set_kitty_keyboard(flags != 0);
            }
            None if self.pushed => {
                tty::disable(&mut out, keyboard_flags(0))?;
                self.pushed = false;
                keys.set_kitty_keyboard(false);
            }
            None => {}
        }
        self.probe = probe;
        if self.keyboard() != before {
            tty::disable(&mut out, Mode::ModifyOtherKeys)?;
        }
        out.flush()?;
        Ok((back, early))
    }

    /// Ask the terminal for kitty keyboard `flags` in place of those it has
    /// (`ttyr`'s guard pops them and pushes these; the first time, it only
    /// pushes), as the server says. Returns whether the protocol is on now
    /// (flags 0 are the legacy keys), which the reader is to be told
    /// ([`Keys::set_kitty_keyboard`]); `None` for a terminal without the
    /// protocol, which is left alone.
    pub fn set_keyboard_flags(&mut self, flags: u8) -> io::Result<Option<bool>> {
        if !self.kitty {
            return Ok(None);
        }
        let mut out = io::stdout();
        tty::enable(&mut out, keyboard_flags(flags))?;
        self.pushed = true;
        out.flush()?;
        Ok(Some(flags != 0))
    }

    /// Turn xterm's modifyOtherKeys (level 2) on or off, as the server says
    /// (`ServerMsg::ExtendedKeys`), through `ttyr`'s guard, which turns it
    /// off on the way out and around a suspend or a lock, and on again
    /// after. A client reading a console's key records cannot read the
    /// keys it sends, says so ([`KeyboardReport::modify_other_keys`]), and
    /// is refused with an `Unsupported` error if asked anyway, which the
    /// client reports and stays attached through.
    pub fn set_extended_keys(&self, on: bool) -> io::Result<()> {
        if !self.vt {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "modifyOtherKeys: this client reads console key records, not a terminal's keys",
            ));
        }
        let mut out = io::stdout();
        if on {
            tty::enable(&mut out, Mode::ModifyOtherKeys)?;
        } else {
            tty::disable(&mut out, Mode::ModifyOtherKeys)?;
        }
        out.flush()
    }

    /// Capture the mouse (every button, drag and motion, in SGR's form), or
    /// hand it back to the terminal.
    #[cfg(unix)]
    pub fn set_mouse(&self, on: bool) -> io::Result<()> {
        let mut out = io::stdout();
        // Recorded first, so that a write that fails part way is still
        // turned off on the way out.
        MOUSE.store(on, Ordering::Release);
        if on {
            ttyr::execute!(out, tty::EnableMouseCapture)
        } else {
            ttyr::execute!(out, tty::DisableMouseCapture)
        }
    }

    /// Capture the mouse, or hand it back to the console, through `ttyr`'s
    /// guard: its mouse mode also turns the console's mouse records on and
    /// Quick Edit's selection off, and puts both back after. Every motion
    /// (mode 1003), which carries every button and drag too, in SGR's form.
    #[cfg(windows)]
    pub fn set_mouse(&self, on: bool) -> io::Result<()> {
        let mut out = io::stdout();
        let mode = Mode::Mouse {
            tracking: tty::MouseTracking::Any,
            sgr: true,
        };
        if on {
            tty::enable(&mut out, mode)?;
        } else {
            tty::disable(&mut out, mode)?;
        }
        out.flush()
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
        // Not held until exit: the next `Guard::enter` stores its own.
        TERMINAL
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}
