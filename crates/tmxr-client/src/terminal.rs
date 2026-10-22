//! The attached client's terminal: modes in, modes out, input pump.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
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
    probe: Probe,
}

/// What the terminal said when asked: whether it takes synchronized
/// updates (DEC mode 2026), and its default foreground and background
/// colours (OSC 10 / 11) as RGB.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Probe {
    sync: bool,
    fg: Option<u32>,
    bg: Option<u32>,
}

/// The terminal's answers, asked once, at the first [`Guard::enter`]: a
/// later one runs beside the input thread, which would take them.
static PROBE: OnceLock<Probe> = OnceLock::new();

/// How long the synchronized-update probe waits for the terminal's answers.
#[cfg(unix)]
const PROBE_WAIT: Duration = Duration::from_secs(1);

/// What the terminal's answers in `reply` say of synchronized updates:
/// `None` until its DA1 answer (`CSI ? … c`), which every terminal sends and
/// comes after DECRPM's, has arrived; then whether DECRPM (`CSI ? 2026 ; Ps
/// $ y`) said the mode is set, reset or permanently set (`Ps` 1 to 3).
#[cfg(unix)]
fn sync_answer(reply: &[u8]) -> Option<bool> {
    let text = String::from_utf8_lossy(reply);
    let da1 = text.match_indices("\x1b[?").any(|(i, _)| {
        let rest = &text[i + 3..];
        let end = rest.find(|c: char| !(c.is_ascii_digit() || c == ';'));
        end.is_some_and(|e| rest[e..].starts_with('c'))
    });
    if !da1 {
        return None;
    }
    Some(
        ["1", "2", "3"]
            .iter()
            .any(|ps| text.contains(&format!("\x1b[?2026;{ps}$y"))),
    )
}

/// The colour the terminal's answer to `OSC which ; ?` in `reply` names
/// (`OSC which ; spec`, ended by a BEL or `ESC \`), read as tmux's
/// tty_keys_colours reads it.
#[cfg(unix)]
fn colour_answer(reply: &[u8], which: u8) -> Option<u32> {
    let text = String::from_utf8_lossy(reply);
    let head = format!("\x1b]{which};");
    let rest = &text[text.find(&head)? + head.len()..];
    let end = rest.find(['\x07', '\x1b'])?;
    tmxr_command::colour::parse_x11(&rest[..end])
}

/// Ask the terminal whether it takes synchronized updates and what its
/// default colours are, as tmux asks at attach, reading its answers
/// straight from the terminal before crossterm reads it (what is read here
/// crossterm never sees).
#[cfg(unix)]
fn probe(out: &mut impl Write) -> io::Result<Probe> {
    // DECRQM for mode 2026, OSC 10 and 11, then DA1 to know when the
    // answers are in: terminals answer in order.
    out.write_all(b"\x1b[?2026$p\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b[c")?;
    out.flush()?;
    let deadline = std::time::Instant::now() + PROBE_WAIT;
    let mut reply = Vec::new();
    let answers = |reply: &[u8], sync: bool| Probe {
        sync,
        fg: colour_answer(reply, 10),
        bg: colour_answer(reply, 11),
    };
    loop {
        if let Some(sync) = sync_answer(&reply) {
            return Ok(answers(&reply, sync));
        }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Ok(answers(&reply, false));
        }
        let mut pfd = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = libc::c_int::try_from(left.as_millis() + 1).unwrap_or(libc::c_int::MAX);
        // SAFETY: polls one live pollfd for at most `millis`.
        let ready = unsafe { libc::poll(&mut pfd, 1, millis) };
        if ready < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if ready == 0 {
            return Ok(answers(&reply, false));
        }
        let mut buf = [0u8; 256];
        // SAFETY: reads into a live buffer of its own length; the terminal
        // is readable, so this does not block.
        let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
        let n = usize::try_from(n).map_err(|_| io::Error::last_os_error())?;
        if n == 0 {
            return Ok(answers(&reply, false));
        }
        reply.extend_from_slice(&buf[..n]);
    }
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
    // changed.
    let _ = out.write_all(b"\x1b[0m\x1b[0 q\x1b]112\x07");
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
        // Before crossterm's keyboard probe below starts its reader. The
        // Windows console hands over input records, so it is not asked.
        #[cfg(unix)]
        let probe = *PROBE.get_or_init(|| probe(&mut out).unwrap_or_default());
        #[cfg(windows)]
        let probe = *PROBE.get_or_init(Probe::default);
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
        Ok(Self { kitty, probe })
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
                let Ok(ev) = crossterm::event::read() else {
                    break;
                };
                // With the events read with it, as the server takes them
                // together (a click's press and release).
                let mut events = vec![ev];
                while let Ok(true) = crossterm::event::poll(Duration::ZERO) {
                    match crossterm::event::read() {
                        Ok(ev) => events.push(ev),
                        Err(_) => break,
                    }
                }
                if console_bytes {
                    events = events.into_iter().map(console_key).collect();
                }
                let Ok(mut send) = send.lock() else { break };
                if write_msg(&mut *send, &ClientMsg::Input(events)).is_err() {
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

    #[cfg(unix)]
    #[test]
    fn the_sync_probe_reads_decrpm_once_da1_is_in() {
        assert_eq!(sync_answer(b""), None);
        // Colours, ended either way, before DA1.
        let reply = b"\x1b]10;rgb:c0c0/caca/f5f5\x1b\\\x1b]11;rgb:1a1a/1b1b/2626\x07\x1b[?6c";
        assert_eq!(colour_answer(reply, 10), Some(0xc0caf5));
        assert_eq!(colour_answer(reply, 11), Some(0x1a1b26));
        assert_eq!(colour_answer(b"\x1b[?6c", 11), None);
        // DECRPM alone: DA1 is still to come.
        assert_eq!(sync_answer(b"\x1b[?2026;2$y"), None);
        assert_eq!(sync_answer(b"\x1b[?2026;2$y\x1b[?62;22c"), Some(true));
        assert_eq!(sync_answer(b"\x1b[?2026;1$y\x1b[?1;2c"), Some(true));
        // Not recognized, or permanently reset.
        assert_eq!(sync_answer(b"\x1b[?2026;0$y\x1b[?62c"), Some(false));
        assert_eq!(sync_answer(b"\x1b[?2026;4$y\x1b[?62c"), Some(false));
        // A terminal that ignores DECRQM answers DA1 alone.
        assert_eq!(sync_answer(b"\x1b[?62;22c"), Some(false));
    }
}
