//! The attached client's terminal: modes in, modes out, input pump.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use interprocess::local_socket::SendHalf;
use tmxr_proto::{ClientMsg, write_msg};

/// Restores the terminal when dropped — on detach, on error, and (through
/// the panic hook installed in [`Guard::enter`]) on panic.
pub struct Guard {
    kitty: bool,
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
    // Reset attributes and the cursor shape the panes may have changed.
    let _ = out.write_all(b"\x1b[0m\x1b[0 q");
    let _ = out.flush();
    let _ = disable_raw_mode();
}

impl Guard {
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut out = io::stdout();
        // Mouse capture waits for the server's `Mouse` (the `mouse` option).
        execute!(
            out,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableFocusChange
        )?;
        // Unambiguous key reports (C-h vs Backspace, C-i vs Tab, Escape vs
        // Alt) where the terminal supports the kitty protocol.
        let kitty = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
        if kitty {
            execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
        }
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore(kitty);
            prev(info);
        }));
        Ok(Self { kitty })
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

/// Forward terminal events to the server until either side goes away.
pub fn spawn_input(send: Arc<Mutex<SendHalf>>) {
    let _ = std::thread::Builder::new()
        .name("tmxr-input".into())
        .spawn(move || {
            while let Ok(ev) = crossterm::event::read() {
                let Ok(mut send) = send.lock() else { break };
                if write_msg(&mut *send, &ClientMsg::Input(ev)).is_err() {
                    break;
                }
            }
        });
}

/// Stop this process until the shell continues it (`suspend-client`), as
/// `C-z` stops a program; returns once it runs again.
#[cfg(unix)]
pub fn suspend() -> io::Result<()> {
    // SAFETY: `raise` only sends a signal to this process; SIGTSTP's default
    // action stops it until SIGCONT.
    if unsafe { libc::raise(libc::SIGTSTP) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Windows has no job control; the server does not ask a client that said
/// so in its hello, so reaching this is a protocol error.
#[cfg(windows)]
pub fn suspend() -> io::Result<()> {
    Err(io::Error::other(
        "suspend-client is not supported on Windows",
    ))
}
