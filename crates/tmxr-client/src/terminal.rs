//! The attached client's terminal: modes in, modes out, input pump.

use std::io::{self, Write};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use interprocess::local_socket::Stream;
use tmxr_proto::{ClientMsg, write_msg};
use ttyr::event::Event;
// Read in the answers to the probe, which only Unix asks.
#[cfg(any(unix, test))]
use ttyr::event::{ColorSlot, ModeSetting, Report};
#[cfg(any(windows, test))]
use ttyr::event::{KeyCode, KeyModifiers};

/// Restores the terminal when dropped — on detach, on error, and (through
/// the panic hook installed in [`Guard::enter`]) on panic.
pub struct Guard {
    kitty: bool,
    probe: Probe,
}

/// What the terminal said when asked: whether it takes synchronized
/// updates (DEC mode 2026), its default foreground and background colours
/// (OSC 10 / 11) as RGB, whether it speaks the kitty keyboard protocol, and
/// its name and version (XTVERSION).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Probe {
    sync: bool,
    fg: Option<u32>,
    bg: Option<u32>,
    kitty: bool,
    /// Kept for plan 18 step 5, whose xterm tier goes by the terminal's
    /// name; nothing reads it yet.
    version: Option<String>,
}

// Only Unix asks; built for its tests everywhere.
#[cfg(any(unix, test))]
impl Probe {
    /// The answers among `read`, the events read while the terminal
    /// answered, up to (not including) its answer to DA1; and the rest of
    /// `read` in the order they came, input for the server (keys typed, a
    /// resize, a report the probe did not ask for). `complete` is whether
    /// DA1's answer came: if not, the terminal answered too slowly (over
    /// SSH, say), and `last`, what it said when last asked, if it was, is
    /// kept rather than taken from what part of its answers came in time.
    fn read(read: Vec<Event>, complete: bool, last: Option<&Self>) -> (Self, Vec<Event>) {
        let mut probe = Self::default();
        let mut others = Vec::new();
        for event in read {
            match event {
                Event::Report(Report::Mode {
                    mode: SYNC_MODE,
                    private: true,
                    setting,
                }) => {
                    probe.sync = matches!(
                        setting,
                        ModeSetting::Set | ModeSetting::Reset | ModeSetting::PermanentlySet
                    );
                }
                Event::Report(Report::TerminalColor(colour))
                    if colour.slot == ColorSlot::Foreground =>
                {
                    probe.fg = Some(tmxr_command::colour::from_rgb16(colour.rgb));
                }
                Event::Report(Report::TerminalColor(colour))
                    if colour.slot == ColorSlot::Background =>
                {
                    probe.bg = Some(tmxr_command::colour::from_rgb16(colour.rgb));
                }
                Event::Report(Report::KeyboardEnhancementFlags(_)) => probe.kitty = true,
                Event::Report(Report::TerminalVersion(version)) => probe.version = Some(version),
                event => others.push(event),
            }
        }
        match last {
            Some(last) if !complete => (last.clone(), others),
            _ => (probe, others),
        }
    }
}

/// Synchronized output's DEC private mode.
#[cfg(any(unix, test))]
const SYNC_MODE: u16 = 2026;

/// How long the probe waits for the terminal's answers.
#[cfg(unix)]
const PROBE_WAIT: Duration = Duration::from_secs(1);

/// The terminal's input, read only by the input thread once it runs; it
/// lends it out while paused ([`Input::pause`]). On Unix, ttyr's reader:
/// the one reader of the terminal in the process, which also answers the
/// probe. On Windows, crossterm's reader of the console, which ttyr does
/// not read yet (its phase 07).
pub struct Keys {
    #[cfg(unix)]
    reader: ttyr::reader::Reader,
    /// What the terminal said when last asked, kept where it does not
    /// answer in time when asked again.
    #[cfg(unix)]
    last: Option<Probe>,
    /// Whether keys arrive as the bytes a terminal sends, turned into key
    /// events by the console ([`keys_from_console_bytes`]).
    #[cfg(windows)]
    console_bytes: bool,
}

impl Keys {
    /// Start reading the terminal on standard input (the controlling
    /// terminal, where standard input is not one), the terminal raw mode is
    /// set on.
    /// A resize from now on is read as an event; the size here is the one
    /// to compare a size from before with.
    #[cfg(unix)]
    pub fn open() -> io::Result<Self> {
        use std::io::IsTerminal as _;
        use std::os::fd::AsFd as _;
        let stdin = io::stdin();
        let reader = if stdin.is_terminal() {
            ttyr::reader::Reader::from_fd(stdin.as_fd().try_clone_to_owned()?)?
        } else {
            ttyr::reader::Reader::open()?
        };
        Ok(Self { reader, last: None })
    }

    /// Start crossterm's reader of the console, whose resize records are
    /// read as events from now on.
    #[cfg(windows)]
    pub fn open() -> io::Result<Self> {
        crossterm::event::poll(Duration::ZERO)?;
        Ok(Self {
            console_bytes: keys_from_console_bytes(),
        })
    }

    /// A resize to tell the server about: the terminal's size now, where it
    /// is not `size`, what the hello said. A resize before [`Keys::open`]
    /// watched for them (a tiling window manager placing the new window) is
    /// otherwise lost.
    pub fn resized_since(&self, size: Option<(u16, u16)>) -> Option<Event> {
        #[cfg(unix)]
        let now = self.reader.size().ok()?;
        #[cfg(windows)]
        let now = crossterm::terminal::size().ok()?;
        (size != Some(now)).then_some(Event::Resize(now.0, now.1))
    }

    /// Ask the terminal what [`Probe`] holds and read its answers, as tmux
    /// asks at attach. Returns them with the events read meanwhile (keys
    /// typed, a resize, a report not asked for), in the order they came,
    /// which are input like any other.
    #[cfg(unix)]
    fn probe(&mut self, out: &mut impl Write) -> io::Result<(Probe, Vec<Event>)> {
        use ttyr::terminal::{
            RequestKeyboardEnhancementFlags, RequestMode, RequestPrimaryDeviceAttributes,
            RequestTerminalColor, RequestTerminalVersion,
        };
        // DA1 last, as every terminal answers it, and in order.
        ttyr::execute!(
            out,
            RequestMode {
                mode: SYNC_MODE,
                private: true,
            },
            RequestTerminalColor(ColorSlot::Foreground),
            RequestTerminalColor(ColorSlot::Background),
            RequestKeyboardEnhancementFlags,
            RequestTerminalVersion,
            RequestPrimaryDeviceAttributes,
        )?;
        // Read here rather than with ttyr's `collect_answers`, which keeps
        // the reports apart from the other events and so loses which came
        // first, a key or a report the probe did not ask for.
        let deadline = Instant::now() + PROBE_WAIT;
        let mut read = Vec::new();
        let mut complete = false;
        while self
            .reader
            .poll(deadline.saturating_duration_since(Instant::now()))?
        {
            match self.reader.read()? {
                Event::Report(Report::PrimaryDeviceAttributes(_)) => {
                    complete = true;
                    break;
                }
                Event::Wake => {}
                event => read.push(event),
            }
        }
        let (probe, events) = Probe::read(read, complete, self.last.as_ref());
        self.last = Some(probe.clone());
        Ok((probe, events))
    }

    /// The Windows console hands over input records, so it is not asked;
    /// whether it speaks the kitty protocol is crossterm's answer.
    #[cfg(windows)]
    fn probe(&mut self, _out: &mut impl Write) -> io::Result<(Probe, Vec<Event>)> {
        let kitty = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
        Ok((
            Probe {
                kitty,
                ..Probe::default()
            },
            Vec::new(),
        ))
    }

    /// The events the terminal sent together, as the server takes them (a
    /// click's press and release): waiting for the first, then each one
    /// already waiting. Empty when there was nothing but a wake, or (on
    /// Windows) nothing within [`INPUT_POLL`].
    #[cfg(unix)]
    fn read_batch(&mut self) -> io::Result<Vec<Event>> {
        let mut read = vec![self.reader.read()?];
        while self.reader.poll(Duration::ZERO)? {
            read.push(self.reader.read()?);
        }
        read.retain(|ev| *ev != Event::Wake);
        Ok(read)
    }

    /// As the Unix one; the console's reader cannot be woken, so it waits
    /// no longer than [`INPUT_POLL`], for the thread to look for a pause.
    #[cfg(windows)]
    fn read_batch(&mut self) -> io::Result<Vec<Event>> {
        if !crossterm::event::poll(INPUT_POLL)? {
            return Ok(Vec::new());
        }
        let mut read = vec![crossterm::event::read()?];
        while crossterm::event::poll(Duration::ZERO)? {
            read.push(crossterm::event::read()?);
        }
        let events = read.into_iter().map(crate::crossterm_bridge::ttyr_event);
        Ok(if self.console_bytes {
            events.map(console_key).collect()
        } else {
            events.collect()
        })
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

/// How often a paused input thread looks whether it may read again, and on
/// Windows how long it waits for an event before checking whether it should
/// pause (the most an [`Input::pause`] waits there). On Unix it waits for
/// an event without a timeout, and a pause wakes it.
const INPUT_POLL: Duration = Duration::from_millis(50);

/// The input thread, which can be paused while another program (the lock
/// command, the shell a suspend returns to) owns the terminal, so that
/// program gets the keys and not the panes, and while the terminal is asked
/// again what it speaks.
pub struct Input {
    paused: Arc<AtomicBool>,
    /// The terminal's input, held by the thread while it reads.
    keys: Arc<Mutex<Keys>>,
    /// Ends a read the thread is waiting in.
    #[cfg(unix)]
    waker: ttyr::reader::Waker,
}

/// While held, the input thread reads nothing, and the terminal's input is
/// lent here.
pub struct Paused<'a> {
    keys: MutexGuard<'a, Keys>,
    paused: &'a AtomicBool,
}

impl Deref for Paused<'_> {
    type Target = Keys;

    fn deref(&self) -> &Keys {
        &self.keys
    }
}

impl DerefMut for Paused<'_> {
    fn deref_mut(&mut self) -> &mut Keys {
        &mut self.keys
    }
}

impl Drop for Paused<'_> {
    fn drop(&mut self) {
        self.paused.store(false, Ordering::Release);
    }
}

impl Input {
    /// Stop reading the terminal; returns once the thread has stopped.
    pub fn pause(&self) -> io::Result<Paused<'_>> {
        // The flag keeps the thread from taking the input again before
        // this does; the lock waits out a read already under way.
        self.paused.store(true, Ordering::Release);
        #[cfg(unix)]
        self.waker.wake()?;
        let keys = self
            .keys
            .lock()
            .map_err(|_| io::Error::other("input thread panicked"))?;
        Ok(Paused {
            keys,
            paused: &self.paused,
        })
    }
}

/// Whether keys reach this client as the bytes a terminal sends, which the
/// Windows console turns into key events (ConPTY behind Alacritty, VS Code's
/// terminal, …), rather than as key events from the terminal itself
/// (Windows Terminal's win32-input-mode; it sets `WT_SESSION`).
#[cfg(windows)]
fn keys_from_console_bytes() -> bool {
    std::env::var_os("WT_SESSION").is_none()
}

/// The key a terminal meant when the Windows console turned its byte into
/// another: it reports C-j (0x0a) as Ctrl+Enter and C-h (0x08) as
/// Ctrl+Backspace, so binds for C-j and C-h never matched. Read them as
/// tmux reads those bytes. A terminal sending bytes cannot send those two
/// chords another way. Built for tests everywhere; only Windows reads keys
/// through a console.
#[cfg(any(windows, test))]
fn console_key(ev: Event) -> Event {
    match ev {
        Event::Key(mut k) if k.modifiers.contains(KeyModifiers::CONTROL) => {
            match k.key {
                KeyCode::Enter => k.key = KeyCode::Char('j'),
                KeyCode::Backspace => k.key = KeyCode::Char('h'),
                _ => {}
            }
            Event::Key(k)
        }
        ev => ev,
    }
}

/// The connection to the server, shared by the threads that write to it,
/// each frame written whole under the lock.
pub type Sender = Arc<Mutex<Arc<Stream>>>;

/// Forward terminal events to the server until either side goes away,
/// `first` (what [`Keys::resized_since`] and [`Guard::enter`] read) ahead of
/// the rest.
pub fn spawn_input(send: Sender, keys: Keys, first: Vec<Event>) -> io::Result<Input> {
    let input = Input {
        paused: Arc::new(AtomicBool::new(false)),
        #[cfg(unix)]
        waker: keys.reader.waker(),
        keys: Arc::new(Mutex::new(keys)),
    };
    let (paused, keys) = (Arc::clone(&input.paused), Arc::clone(&input.keys));
    std::thread::Builder::new()
        .name("tmxr-input".into())
        .spawn(move || {
            if !first.is_empty() && forward(&send, first).is_err() {
                return;
            }
            loop {
                if paused.load(Ordering::Acquire) {
                    std::thread::sleep(INPUT_POLL);
                    continue;
                }
                let Ok(mut reading) = keys.lock() else { break };
                let Ok(events) = reading.read_batch() else {
                    break;
                };
                drop(reading);
                if !events.is_empty() && forward(&send, events).is_err() {
                    break;
                }
            }
        })?;
    Ok(input)
}

/// Send `events` to the server as input.
pub fn forward(send: &Sender, events: Vec<Event>) -> io::Result<()> {
    let send = send
        .lock()
        .map_err(|_| io::Error::other("a writer to the server panicked"))?;
    write_msg(&mut &**send, &ClientMsg::Input(events)).map_err(|e| io::Error::other(e.to_string()))
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
    use ttyr::event::{ColorScheme, KeyEvent, KeyboardEnhancementFlags, TerminalColor};

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

    fn colour(slot: ColorSlot, rgb: (u16, u16, u16)) -> Event {
        Event::Report(Report::TerminalColor(TerminalColor { slot, rgb }))
    }

    fn sync(setting: ModeSetting) -> Event {
        Event::Report(Report::Mode {
            mode: SYNC_MODE,
            private: true,
            setting,
        })
    }

    fn kitty() -> Event {
        Event::Report(Report::KeyboardEnhancementFlags(
            KeyboardEnhancementFlags::empty(),
        ))
    }

    /// What a foot-like terminal answers, all of it.
    fn foot() -> Probe {
        Probe {
            sync: true,
            fg: Some(0xc0caf5),
            bg: Some(0x1a1b26),
            kitty: true,
            version: Some("foot(1.20.2)".into()),
        }
    }

    #[test]
    fn the_probe_takes_its_answers_and_hands_on_the_rest_in_order() {
        let scheme = Event::Report(Report::ColorScheme(ColorScheme::Dark));
        let x = key(KeyCode::Char('x'), KeyModifiers::NONE);
        let (probe, others) = Probe::read(
            vec![
                sync(ModeSetting::Reset),
                x.clone(),
                colour(ColorSlot::Foreground, (0xc0c0, 0xcaca, 0xf5f5)),
                scheme.clone(),
                colour(ColorSlot::Background, (0x1a1a, 0x1b1b, 0x2626)),
                Event::Resize(80, 24),
                kitty(),
                Event::Report(Report::TerminalVersion("foot(1.20.2)".into())),
            ],
            true,
            None,
        );
        assert_eq!(probe, foot());
        // What the probe did not ask for, in the order it came.
        assert_eq!(others, vec![x, scheme, Event::Resize(80, 24)]);
    }

    #[test]
    fn a_probe_answered_too_late_keeps_what_the_terminal_said_before() {
        // Asked again (after a suspend or a lock) over a slow link: only
        // DECRPM came in time, and nothing is taken from it.
        let late = vec![sync(ModeSetting::NotRecognized)];
        assert_eq!(
            Probe::read(late.clone(), false, Some(&foot())),
            (foot(), Vec::new())
        );
        // Answered in time, the new answers stand.
        assert_eq!(
            Probe::read(late.clone(), true, Some(&foot())).0,
            Probe::default()
        );
        // Asked for the first time, what came in time is all there is.
        let (first, _) = Probe::read(vec![kitty()], false, None);
        assert!(first.kitty && !first.sync);
    }

    #[test]
    fn the_probe_says_no_to_what_the_terminal_did_not_answer() {
        // A terminal that ignores every query but DA1.
        assert_eq!(
            Probe::read(Vec::new(), true, None),
            (Probe::default(), Vec::new())
        );
        // Synchronized output not recognized, or permanently reset.
        for setting in [ModeSetting::NotRecognized, ModeSetting::PermanentlyReset] {
            assert!(
                !Probe::read(vec![sync(setting)], true, None).0.sync,
                "{setting:?}"
            );
        }
        for setting in [
            ModeSetting::Set,
            ModeSetting::Reset,
            ModeSetting::PermanentlySet,
        ] {
            assert!(
                Probe::read(vec![sync(setting)], true, None).0.sync,
                "{setting:?}"
            );
        }
    }
}
