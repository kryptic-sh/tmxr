//! The attached client's terminal: modes in, modes out, input pump.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

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

/// How long the input thread waits for an event before checking whether it
/// should pause: the most a [`Input::pause`] waits.
const INPUT_POLL: Duration = Duration::from_millis(50);

/// The input thread, which can be paused while another program (the lock
/// command) owns the terminal, so that program gets the keys and not the
/// panes.
pub struct Input {
    paused: Arc<AtomicBool>,
    /// Held by the thread while it reads the terminal.
    gate: Arc<Mutex<()>>,
}

/// While held, the input thread reads nothing.
pub struct Paused<'a> {
    _gate: MutexGuard<'a, ()>,
    paused: &'a AtomicBool,
}

impl Drop for Paused<'_> {
    fn drop(&mut self) {
        self.paused.store(false, Ordering::Release);
    }
}

impl Input {
    /// Stop reading the terminal; returns once the thread has stopped.
    pub fn pause(&self) -> io::Result<Paused<'_>> {
        // The flag keeps the thread from taking the gate again before this
        // does; the gate waits out a read already under way.
        self.paused.store(true, Ordering::Release);
        let gate = self
            .gate
            .lock()
            .map_err(|_| io::Error::other("input thread panicked"))?;
        Ok(Paused {
            _gate: gate,
            paused: &self.paused,
        })
    }
}

/// Forward terminal events to the server until either side goes away.
pub fn spawn_input(send: Arc<Mutex<SendHalf>>) -> Input {
    let input = Input {
        paused: Arc::new(AtomicBool::new(false)),
        gate: Arc::new(Mutex::new(())),
    };
    let (paused, gate) = (Arc::clone(&input.paused), Arc::clone(&input.gate));
    let _ = std::thread::Builder::new()
        .name("tmxr-input".into())
        .spawn(move || {
            loop {
                if paused.load(Ordering::Acquire) {
                    std::thread::sleep(INPUT_POLL);
                    continue;
                }
                let Ok(_reading) = gate.lock() else { break };
                match crossterm::event::poll(INPUT_POLL) {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(_) => break,
                }
                let Ok(ev) = crossterm::event::read() else {
                    break;
                };
                let Ok(mut send) = send.lock() else { break };
                if write_msg(&mut *send, &ClientMsg::Input(ev)).is_err() {
                    break;
                }
            }
        });
    input
}

/// Run the lock command in the terminal and wait for it (`lock-client`), as
/// tmux runs `lock-command` with `/bin/sh -c`; an error says why it did not
/// unlock cleanly.
pub fn run_lock(command: &str) -> Result<(), String> {
    #[cfg(unix)]
    let status = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .status();
    #[cfg(windows)]
    let status = {
        use std::os::windows::process::CommandExt as _;
        // Raw, as cmd parses its own command line rather than argv.
        std::process::Command::new("cmd")
            .arg("/C")
            .raw_arg(command)
            .status()
    };
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("lock-command {command:?} exited with {s}")),
        Err(e) => Err(format!("lock-command {command:?}: {e}")),
    }
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
