//! The attached client's terminal: modes in, modes out, input pump.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, Event, KeyCode, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
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
        // Mouse capture and focus reports wait for the server's `Mouse` and
        // `FocusEvents` (the `mouse` and `focus-events` options).
        execute!(out, EnterAlternateScreen, EnableBracketedPaste)?;
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

pub fn set_focus_events(on: bool) -> io::Result<()> {
    if on {
        execute!(io::stdout(), EnableFocusChange)
    } else {
        execute!(io::stdout(), DisableFocusChange)
    }
}

/// How often a paused input thread looks whether it may read again, and on
/// Windows how long it waits for an event before checking whether it should
/// pause (the most an [`Input::pause`] waits there). On Unix it waits for
/// an event without a timeout, and a pause wakes it.
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
        #[cfg(unix)]
        wake_reader()?;
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

/// Whether keys reach this client as the bytes a terminal sends, which the
/// Windows console turns into key events (ConPTY behind Alacritty, VS Code's
/// terminal, …), rather than as key events from the terminal itself
/// (Windows Terminal's win32-input-mode; it sets `WT_SESSION`). Unix reads
/// the bytes itself.
fn keys_from_console_bytes() -> bool {
    cfg!(windows) && std::env::var_os("WT_SESSION").is_none()
}

/// The key a terminal meant when the console turned its byte into another:
/// it reports C-j (0x0a) as Ctrl+Enter and C-h (0x08) as Ctrl+Backspace, so
/// binds for C-j and C-h never matched. Read them as tmux reads those bytes.
/// A terminal sending bytes cannot send those two chords another way.
fn console_key(ev: Event) -> Event {
    match ev {
        Event::Key(mut k) if k.modifiers.contains(KeyModifiers::CONTROL) => {
            match k.code {
                KeyCode::Enter => k.code = KeyCode::Char('j'),
                KeyCode::Backspace => k.code = KeyCode::Char('h'),
                _ => {}
            }
            Event::Key(k)
        }
        ev => ev,
    }
}

/// Wake the input thread out of a read waiting for an event: crossterm's
/// reader watches for SIGWINCH, so one raised here ends it with a `Resize`
/// to the size the terminal already has, which the server takes as a
/// redraw.
#[cfg(unix)]
fn wake_reader() -> io::Result<()> {
    // SAFETY: `raise` only sends a signal to this thread; crossterm's handler
    // for it (signal-hook's) writes to its reader's pipe, and before that
    // handler exists the signal is ignored, SIGWINCH's default.
    if unsafe { libc::raise(libc::SIGWINCH) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Forward terminal events to the server until either side goes away.
pub fn spawn_input(send: Arc<Mutex<SendHalf>>) -> Input {
    let input = Input {
        paused: Arc::new(AtomicBool::new(false)),
        gate: Arc::new(Mutex::new(())),
    };
    let (paused, gate) = (Arc::clone(&input.paused), Arc::clone(&input.gate));
    let console_bytes = keys_from_console_bytes();
    let _ = std::thread::Builder::new()
        .name("tmxr-input".into())
        .spawn(move || {
            loop {
                if paused.load(Ordering::Acquire) {
                    std::thread::sleep(INPUT_POLL);
                    continue;
                }
                let Ok(_reading) = gate.lock() else { break };
                // On Unix the read below waits for an event, a pause waking
                // it; the Windows console's reader cannot be woken, so it
                // looks for a pause between short waits.
                #[cfg(windows)]
                match crossterm::event::poll(INPUT_POLL) {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(_) => break,
                }
                let Ok(mut ev) = crossterm::event::read() else {
                    break;
                };
                if console_bytes {
                    ev = console_key(ev);
                }
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
    let status = tmxr_term::child::command("/bin/sh")
        .arg("-c")
        .arg(command)
        .status();
    #[cfg(windows)]
    let status = {
        use std::os::windows::process::CommandExt as _;
        // Raw, as cmd parses its own command line rather than argv.
        tmxr_term::child::command("cmd")
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

/// The terminal on standard input, as tmux's client takes
/// `ttyname(STDIN_FILENO)` for `#{client_tty}`; empty when it has none.
#[cfg(unix)]
pub fn tty_name() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: `ttyname_r` writes at most `buf.len()` bytes, its NUL
    // included, into `buf`, which outlives the call.
    let failed = unsafe { libc::ttyname_r(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
    if failed != 0 {
        return String::new();
    }
    std::ffi::CStr::from_bytes_until_nul(&buf)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A Windows console has no device name, so `#{client_tty}` is empty, as
/// tmux's for a client without a tty.
#[cfg(windows)]
pub fn tty_name() -> String {
    String::new()
}

/// Send this process's parent (the shell it ran from) SIGHUP, which usually
/// closes the terminal too (`detach-client -P`).
#[cfg(unix)]
pub fn hang_up_parent() -> io::Result<()> {
    // SAFETY: `kill` only sends a signal; `getppid` cannot fail.
    if unsafe { libc::kill(libc::getppid(), libc::SIGHUP) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Run `argv` in this client's place (`detach-client -E`), its shell as
/// `SHELL`, as tmux's client_exec: the process becomes it, so this returns
/// only when that fails.
#[cfg(unix)]
pub fn exec(argv: &[String]) -> io::Result<i32> {
    use std::os::unix::process::CommandExt as _;
    let (shell, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::other("empty command"))?;
    Err(tmxr_term::child::command(shell)
        .args(args)
        .env("SHELL", shell)
        .exec())
}

/// Run `argv` for this client (`detach-client -E`), its shell as `SHELL`,
/// and exit with its status: Windows cannot replace a running process, as
/// tmux's client_exec does.
#[cfg(windows)]
pub fn exec(argv: &[String]) -> io::Result<i32> {
    let (shell, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::other("empty command"))?;
    let status = tmxr_term::child::command(shell)
        .args(args)
        .env("SHELL", shell)
        .status()?;
    Ok(status.code().unwrap_or(1))
}

/// Windows has no SIGHUP; the server does not ask a client without job
/// control, so reaching this is a protocol error.
#[cfg(windows)]
pub fn hang_up_parent() -> io::Result<()> {
    Err(io::Error::other(
        "detach-client -P is not supported on Windows",
    ))
}

/// Windows has no job control; the server does not ask a client that said
/// so in its hello, so reaching this is a protocol error.
#[cfg(windows)]
pub fn suspend() -> io::Result<()> {
    Err(io::Error::other(
        "suspend-client is not supported on Windows",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    fn key(code: KeyCode, mods: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, mods))
    }

    #[test]
    fn console_bytes_for_c_j_and_c_h_are_read_as_those_keys() {
        let ctrl = KeyModifiers::CONTROL;
        let ctrl_alt = KeyModifiers::CONTROL | KeyModifiers::ALT;
        for (from, to) in [
            (key(KeyCode::Enter, ctrl), key(KeyCode::Char('j'), ctrl)),
            (key(KeyCode::Backspace, ctrl), key(KeyCode::Char('h'), ctrl)),
            (
                key(KeyCode::Enter, ctrl_alt),
                key(KeyCode::Char('j'), ctrl_alt),
            ),
            // Enter, Backspace and the other chords are left alone.
            (
                key(KeyCode::Enter, KeyModifiers::NONE),
                key(KeyCode::Enter, KeyModifiers::NONE),
            ),
            (
                key(KeyCode::Backspace, KeyModifiers::NONE),
                key(KeyCode::Backspace, KeyModifiers::NONE),
            ),
            (key(KeyCode::Char('k'), ctrl), key(KeyCode::Char('k'), ctrl)),
            (Event::FocusGained, Event::FocusGained),
        ] {
            assert_eq!(console_key(from.clone()), to, "{from:?}");
        }
    }
}
