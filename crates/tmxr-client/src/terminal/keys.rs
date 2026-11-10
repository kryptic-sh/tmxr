//! The terminal's input, through ttyr's reader: on Unix the terminal's
//! bytes; on Windows the console's, as key records or as the bytes the
//! terminal sends ([`Keys::reads_vt`]).

use std::io;
#[cfg(unix)]
use std::os::fd::OwnedFd;
use std::time::Duration;

use ttyr::event::{Event, Report};
#[cfg(any(windows, test))]
use ttyr::event::{KeyCode, KeyEventKind, KeyModifiers};

use super::probe::Probe;

/// The terminal's input, read only by the input thread once it runs; it
/// lends it out while paused ([`Input::pause`](super::Input::pause)):
/// ttyr's reader, the one reader of the terminal in the process, which also
/// answers the probe.
pub struct Keys {
    pub(super) reader: ttyr::reader::Reader,
    /// The terminal the reader reads, for raw mode to be set on.
    #[cfg(unix)]
    pub(super) tty: OwnedFd,
    /// What the terminal said when last asked, kept where it does not
    /// answer in time when asked again.
    pub(super) last: Option<Probe>,
    /// Whether the reader takes the kitty keyboard protocol to be on, as
    /// its decoder does: set with [`Keys::set_kitty_keyboard`], and by
    /// each answer to the flags query it decodes. It decides whether a
    /// key's Meta is xterm's ([`tmxr_command::keys::xterm_meta_as_alt`]).
    pub(super) kitty_keyboard: bool,
    /// The console's key records read as the bytes a terminal sent, where
    /// it hands over records ([`Keys::reads_vt`] is false).
    #[cfg(windows)]
    console: Option<ConsoleBytes>,
}

/// The terminal's size, columns then rows: on Unix, that of the terminal
/// [`Keys::open`] reads; on Windows, the console's.
pub fn size() -> io::Result<(u16, u16)> {
    #[cfg(unix)]
    return ttyr::terminal::size_of(tty()?);
    #[cfg(windows)]
    return ttyr::terminal::size();
}

/// Whether the client reads the bytes its terminal sends, and so its
/// answers to queries (the probe, colours, the colour scheme), rather than
/// key records. Always on Unix. On Windows, under Windows Terminal
/// (`WT_SESSION`), which the client asks for VT input with its
/// win32-input-mode; elsewhere (conhost, or ConPTY behind another terminal)
/// the console's key records, whose keys are fuller than the VT input
/// conhost makes of them, and through which no query is asked. A client
/// inside a tmxr pane (`TMXR` set) inherits `WT_SESSION` from the server
/// but reads tmxr's ConPTY, not Windows Terminal: records.
pub fn reads_vt() -> bool {
    #[cfg(unix)]
    return true;
    #[cfg(windows)]
    return std::env::var_os("WT_SESSION").is_some()
        && std::env::var_os(tmxr_proto::socket::ENV_TMXR).is_none_or(|v| v.is_empty());
}

/// The client's terminal: standard input where it is one, as crossterm took
/// it, else the controlling terminal. A client under a launcher that leaves
/// it no controlling terminal (`setsid`) still has its terminal on standard
/// input.
#[cfg(unix)]
fn tty() -> io::Result<OwnedFd> {
    use std::io::IsTerminal as _;
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return stdin.as_fd().try_clone_to_owned();
    }
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY)
        .open("/dev/tty")?;
    Ok(tty.into())
}

impl Keys {
    /// Start reading the terminal on standard input (the controlling
    /// terminal, where standard input is not one), the terminal raw mode is
    /// set on.
    /// A resize from now on is read as an event; the size here is the one
    /// to compare a size from before with.
    #[cfg(unix)]
    pub fn open() -> io::Result<Self> {
        let tty = tty()?;
        let reader = ttyr::reader::Reader::from_fd(tty.try_clone()?)?;
        Ok(Self {
            reader,
            tty,
            last: None,
            kitty_keyboard: false,
        })
    }

    /// Start reading the process's console (`CONIN$`). A resize is read as
    /// an event once raw mode is on ([`Guard::enter`](super::Guard::enter)),
    /// which turns the console's resize records on.
    #[cfg(windows)]
    pub fn open() -> io::Result<Self> {
        Ok(Self {
            reader: ttyr::reader::Reader::open()?,
            last: None,
            kitty_keyboard: false,
            console: (!reads_vt()).then(ConsoleBytes::default),
        })
    }

    /// Whether this client reads its terminal's bytes ([`reads_vt`]).
    pub(super) fn reads_vt(&self) -> bool {
        #[cfg(unix)]
        return true;
        #[cfg(windows)]
        return self.console.is_none();
    }

    /// Tell the reader whether the kitty keyboard protocol is on, as the
    /// client pushes or pops the flags that turn it on: modifier bit 8 is
    /// Super under it and xterm's Meta outside it. The terminal's answer to
    /// the probe's flags query says so too, but only for the flags it had
    /// when asked: a probe after a hand-over hears the flags put back,
    /// which the client may pop just after.
    pub(super) fn set_kitty_keyboard(&mut self, on: bool) {
        self.reader.set_kitty_keyboard(on);
        self.kitty_keyboard = on;
    }

    /// `events`, read in order, as tmux reads their Meta: xterm's is Alt.
    /// An answer to the flags query among them turns the protocol on or
    /// off for the keys after it, as the reader's decoder took it.
    pub(super) fn read_meta(&mut self, events: &mut [Event]) {
        for ev in events {
            match ev {
                Event::Report(Report::KeyboardEnhancementFlags(flags)) => {
                    self.kitty_keyboard = !flags.is_empty();
                }
                Event::Key(key) => {
                    *key = tmxr_command::keys::xterm_meta_as_alt(key.clone(), self.kitty_keyboard);
                }
                _ => {}
            }
        }
    }

    /// The size of the terminal the reader reads, columns then rows: what
    /// [`Keys::resized_since`] compares with.
    pub fn size(&self) -> io::Result<(u16, u16)> {
        self.reader.size()
    }

    /// A resize to tell the server about: the terminal's size now, where it
    /// is not `size`, the one from before. A resize while nothing watched
    /// for them is otherwise lost: before [`Keys::open`] (a tiling window
    /// manager placing the new window), and on Windows while raw mode was
    /// off for a lock, with the console's resize records off with it.
    pub fn resized_since(&self, size: Option<(u16, u16)>) -> Option<Event> {
        let now = self.size().ok()?;
        (size != Some(now)).then_some(Event::Resize(now.0, now.1))
    }

    /// The events the terminal sent together, as the server takes them (a
    /// click's press and release): waiting for the first, then each one
    /// already waiting. Empty when there was nothing but a wake.
    pub(super) fn read_batch(&mut self) -> io::Result<Vec<Event>> {
        let mut read = vec![self.reader.read()?];
        while self.reader.poll(Duration::ZERO)? {
            read.push(self.reader.read()?);
        }
        read.retain(|ev| *ev != Event::Wake);
        self.read_meta(&mut read);
        #[cfg(windows)]
        if let Some(console) = &mut self.console {
            for ev in &mut read {
                console.key(ev);
            }
        }
        Ok(read)
    }
}

/// The keys a terminal meant when the Windows console turned its bytes
/// into others. It reports C-j (0x0a) as Ctrl+Enter and C-h (0x08) as
/// Ctrl+Backspace, so binds for C-j and C-h never matched; and it makes a
/// C0 byte's record from the layout's key for it (`VkKeyScanW`), so NUL,
/// 0x1e and 0x1f come with Shift (Ctrl+Shift+2, Ctrl+Shift+6, Ctrl+Shift+-
/// on a US layout), which since Ctrl keeps Shift apart (C-S-2) fired no
/// C-Space, C-^ or C-_ bind. Read them as tmux reads those bytes; a
/// terminal sending bytes cannot send those chords another way. A key's
/// release is read as its press was, Ctrl let go first or not, so it
/// releases the key its press reached. Built for tests everywhere; only
/// Windows reads keys through a console.
#[cfg(any(windows, test))]
#[derive(Debug, Default)]
struct ConsoleBytes {
    /// The keys pressed and read as others, as the console reported each,
    /// until their release.
    held: Vec<(KeyCode, Meant)>,
}

/// What a key held with Ctrl is read as: `key`, without Shift where
/// `shift_dropped` (the console's own, the byte having none).
#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Meant {
    key: KeyCode,
    shift_dropped: bool,
}

/// What the console's `key` held with Ctrl is read as, where it made it of
/// a terminal's byte; `None` for a key read as it is. A NUL record reads as
/// the layout's key for it (`2` on a US one), never as `@`.
#[cfg(any(windows, test))]
fn meant(key: KeyCode) -> Option<Meant> {
    let (key, shift_dropped) = match key {
        KeyCode::Enter => (KeyCode::Char('j'), false),
        KeyCode::Backspace => (KeyCode::Char('h'), false),
        KeyCode::Char('2' | ' ') => (KeyCode::Char(' '), true),
        KeyCode::Char('6' | '^') => (KeyCode::Char('^'), true),
        KeyCode::Char('-' | '_') => (KeyCode::Char('_'), true),
        _ => return None,
    };
    Some(Meant { key, shift_dropped })
}

/// Whether the release of `key` is that of a press read as `held`: by the
/// key the console reported for it, or, with Ctrl let go first and Shift
/// still down, by its shifted character (`@` for the 2 of NUL's record).
#[cfg(any(windows, test))]
fn releases(key: KeyCode, (pressed, held): (KeyCode, Meant)) -> bool {
    let shifted = match key {
        KeyCode::Char('@') => Some(KeyCode::Char(' ')),
        key => meant(key).map(|m| m.key),
    };
    key == pressed || (held.shift_dropped && shifted == Some(held.key))
}

#[cfg(any(windows, test))]
impl ConsoleBytes {
    /// `ev` as the terminal meant it.
    fn key(&mut self, ev: &mut Event) {
        let Event::Key(k) = ev else {
            return;
        };
        let read = match k.kind {
            KeyEventKind::Release => {
                let Some(i) = self.held.iter().position(|&h| releases(k.key, h)) else {
                    return;
                };
                self.held.remove(i).1
            }
            _ if k.modifiers.contains(KeyModifiers::CONTROL) => {
                let Some(read) = meant(k.key) else {
                    return;
                };
                if !self.held.iter().any(|&(from, _)| from == k.key) {
                    self.held.push((k.key, read));
                }
                read
            }
            _ => return,
        };
        if read.shift_dropped {
            k.modifiers.remove(KeyModifiers::SHIFT);
        }
        k.key = read.key;
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use ttyr::event::KeyEvent;

    pub(in crate::terminal) fn key(code: KeyCode, mods: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, mods))
    }

    fn release(code: KeyCode, mods: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, mods).with_kind(KeyEventKind::Release))
    }

    #[test]
    fn console_bytes_are_read_as_tmux_reads_the_bytes() {
        let ctrl = KeyModifiers::CONTROL;
        let ctrl_alt = KeyModifiers::CONTROL | KeyModifiers::ALT;
        let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        let shift = KeyModifiers::SHIFT;
        let none = KeyModifiers::NONE;
        let mut console = ConsoleBytes::default();
        for (from, to) in [
            (key(KeyCode::Enter, ctrl), key(KeyCode::Char('j'), ctrl)),
            (
                release(KeyCode::Enter, ctrl),
                release(KeyCode::Char('j'), ctrl),
            ),
            (key(KeyCode::Backspace, ctrl), key(KeyCode::Char('h'), ctrl)),
            (
                release(KeyCode::Backspace, ctrl),
                release(KeyCode::Char('h'), ctrl),
            ),
            (
                key(KeyCode::Enter, ctrl_alt),
                key(KeyCode::Char('j'), ctrl_alt),
            ),
            // Ctrl let go before Enter: the release is still C-j's.
            (
                release(KeyCode::Enter, none),
                release(KeyCode::Char('j'), none),
            ),
            // Enter, Backspace and the other chords are left alone, their
            // releases too.
            (key(KeyCode::Enter, none), key(KeyCode::Enter, none)),
            (release(KeyCode::Enter, none), release(KeyCode::Enter, none)),
            (key(KeyCode::Backspace, none), key(KeyCode::Backspace, none)),
            (key(KeyCode::Char('k'), ctrl), key(KeyCode::Char('k'), ctrl)),
            // The console's records of NUL, 0x1e and 0x1f, as the layout
            // types them: C-Space, C-^ and C-_, their releases too.
            (
                key(KeyCode::Char('2'), ctrl_shift),
                key(KeyCode::Char(' '), ctrl),
            ),
            (
                release(KeyCode::Char('2'), ctrl_shift),
                release(KeyCode::Char(' '), ctrl),
            ),
            // Ctrl let go first, Shift still held: the key-up is the
            // shifted character's, and still C-Space's or C-^'s release,
            // leaving nothing held to misread a later plain 2 or 6.
            (
                key(KeyCode::Char('2'), ctrl_shift),
                key(KeyCode::Char(' '), ctrl),
            ),
            (
                release(KeyCode::Char('@'), shift),
                release(KeyCode::Char(' '), none),
            ),
            (key(KeyCode::Char('2'), none), key(KeyCode::Char('2'), none)),
            (
                release(KeyCode::Char('2'), none),
                release(KeyCode::Char('2'), none),
            ),
            (
                key(KeyCode::Char('6'), ctrl_shift),
                key(KeyCode::Char('^'), ctrl),
            ),
            (
                release(KeyCode::Char('^'), shift),
                release(KeyCode::Char('^'), none),
            ),
            (key(KeyCode::Char('6'), none), key(KeyCode::Char('6'), none)),
            (
                release(KeyCode::Char('6'), none),
                release(KeyCode::Char('6'), none),
            ),
            // No NUL record reads as `@` with Ctrl: one that does (AltGr+q
            // misread as Ctrl+Alt) is left as it came.
            (
                key(KeyCode::Char('@'), ctrl_alt),
                key(KeyCode::Char('@'), ctrl_alt),
            ),
            (
                key(KeyCode::Char('6'), ctrl_shift),
                key(KeyCode::Char('^'), ctrl),
            ),
            (
                key(KeyCode::Char('_'), ctrl_shift),
                key(KeyCode::Char('_'), ctrl),
            ),
            (
                release(KeyCode::Char('6'), ctrl_shift),
                release(KeyCode::Char('^'), ctrl),
            ),
            (
                release(KeyCode::Char('_'), ctrl_shift),
                release(KeyCode::Char('_'), ctrl),
            ),
            // Without Ctrl, typed as they are.
            (key(KeyCode::Char('2'), none), key(KeyCode::Char('2'), none)),
            (
                key(KeyCode::Char('@'), shift),
                key(KeyCode::Char('@'), shift),
            ),
            (Event::FocusGained, Event::FocusGained),
        ] {
            let mut read = from.clone();
            console.key(&mut read);
            assert_eq!(read, to, "{from:?}");
        }
        assert!(console.held.is_empty(), "{:?}", console.held);
    }
}

/// The reader on a real Windows console, with records written into its
/// input as a user's keys would be, or as ConPTY makes them of a
/// terminal's bytes. Windows only; first run in CI.
#[cfg(all(test, windows))]
pub(super) mod console_tests {
    use std::os::windows::io::AsRawHandle as _;
    use std::sync::{Mutex, MutexGuard, PoisonError};

    use windows_sys::Win32::System::Console::{
        AllocConsole, FlushConsoleInputBuffer, INPUT_RECORD, INPUT_RECORD_0, KEY_EVENT,
        KEY_EVENT_RECORD, KEY_EVENT_RECORD_0, WriteConsoleInputW,
    };

    use super::*;

    /// The console's input, opened afresh.
    fn conin() -> std::fs::File {
        std::fs::File::options()
            .read(true)
            .write(true)
            .open("CONIN$")
            .unwrap()
    }

    /// The process has one console and one ttyr reader at a time, so these
    /// tests take turns; each starts with the console's input empty.
    pub(in crate::terminal) fn console() -> MutexGuard<'static, ()> {
        static CONSOLE: Mutex<()> = Mutex::new(());
        let turn = CONSOLE.lock().unwrap_or_else(PoisonError::into_inner);
        // SAFETY: no arguments; it fails, harmlessly, when the process has
        // a console already.
        unsafe { AllocConsole() };
        // SAFETY: the handle is open for the call.
        assert_ne!(
            unsafe { FlushConsoleInputBuffer(conin().as_raw_handle()) },
            0
        );
        turn
    }

    /// A key record: virtual-key code `vk`, the UTF-16 unit it typed, down
    /// or up, and its control key state.
    pub(in crate::terminal) fn record(vk: u16, unit: u16, down: bool, state: u32) -> INPUT_RECORD {
        INPUT_RECORD {
            EventType: u16::try_from(KEY_EVENT).unwrap(),
            Event: INPUT_RECORD_0 {
                KeyEvent: KEY_EVENT_RECORD {
                    bKeyDown: i32::from(down),
                    wRepeatCount: 1,
                    wVirtualKeyCode: vk,
                    wVirtualScanCode: 0,
                    uChar: KEY_EVENT_RECORD_0 { UnicodeChar: unit },
                    dwControlKeyState: state,
                },
            },
        }
    }

    /// Writes `records` into the console's input.
    pub(in crate::terminal) fn inject(records: &[INPUT_RECORD]) {
        let mut written = 0;
        // SAFETY: the handle is open, `records` holds `len` records, and
        // `written` is a valid out pointer.
        let ok = unsafe {
            WriteConsoleInputW(
                conin().as_raw_handle(),
                records.as_ptr(),
                u32::try_from(records.len()).unwrap(),
                &mut written,
            )
        };
        assert_ne!(ok, 0, "{}", io::Error::last_os_error());
        assert_eq!(written as usize, records.len());
    }

    /// Keys reading the console's records, as outside Windows Terminal.
    pub(in crate::terminal) fn keys() -> Keys {
        Keys {
            reader: ttyr::reader::Reader::open().unwrap(),
            last: None,
            kitty_keyboard: false,
            console: Some(ConsoleBytes::default()),
        }
    }

    /// The key events read until `n` have come.
    fn read_keys(keys: &mut Keys, n: usize) -> Vec<ttyr::event::KeyEvent> {
        let mut got = Vec::new();
        while got.len() < n {
            got.extend(
                keys.read_batch()
                    .unwrap()
                    .into_iter()
                    .filter_map(|ev| match ev {
                        Event::Key(k) => Some(k),
                        _ => None,
                    }),
            );
        }
        got
    }

    /// ConPTY's records for a terminal's C-j byte (Ctrl+Enter, down and up)
    /// read as C-j's press and release; an AltGr record (right Alt with
    /// left Ctrl, as the console reports AltGr) types its character with
    /// neither; Ctrl+4 stays Ctrl+4; ConPTY's record of a NUL byte is
    /// C-Space.
    #[test]
    fn console_records_read_as_the_keys_meant() {
        // wincon.h's control key state bits and winuser.h's key codes.
        const RIGHT_ALT: u32 = 0x1;
        const LEFT_CTRL: u32 = 0x8;
        const SHIFT: u32 = 0x10;
        const VK_RETURN: u16 = 0x0D;
        let _turn = console();
        let mut keys = keys();
        inject(&[
            record(VK_RETURN, 0x0A, true, LEFT_CTRL),
            record(VK_RETURN, 0x0A, false, LEFT_CTRL),
            record(
                u16::from(b'Q'),
                u16::from(b'@'),
                true,
                RIGHT_ALT | LEFT_CTRL,
            ),
            record(u16::from(b'4'), 0x1C, true, LEFT_CTRL),
            // NUL as ConPTY makes it of a terminal's byte on a US layout.
            record(u16::from(b'2'), 0, true, LEFT_CTRL | SHIFT),
        ]);
        let got: Vec<_> = read_keys(&mut keys, 5)
            .into_iter()
            .map(|k| (k.key, k.modifiers, k.kind))
            .collect();
        let ctrl = KeyModifiers::CONTROL;
        assert_eq!(
            got,
            [
                (KeyCode::Char('j'), ctrl, KeyEventKind::Press),
                (KeyCode::Char('j'), ctrl, KeyEventKind::Release),
                (KeyCode::Char('@'), KeyModifiers::NONE, KeyEventKind::Press),
                (KeyCode::Char('4'), ctrl, KeyEventKind::Press),
                (KeyCode::Char(' '), ctrl, KeyEventKind::Press),
            ]
        );
    }
}
