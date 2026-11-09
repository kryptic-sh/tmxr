//! The terminal's input: ttyr's reader on Unix, crossterm's on Windows.

use std::io;
use std::time::Duration;

use ttyr::event::Event;
#[cfg(any(windows, test))]
use ttyr::event::{KeyCode, KeyModifiers};

#[cfg(windows)]
use super::input::INPUT_POLL;
#[cfg(unix)]
use super::probe::Probe;

/// The terminal's input, read only by the input thread once it runs; it
/// lends it out while paused ([`Input::pause`](super::Input::pause)). On Unix, ttyr's reader:
/// the one reader of the terminal in the process, which also answers the
/// probe. On Windows, crossterm's reader of the console, which ttyr does
/// not read yet (its phase 07).
pub struct Keys {
    #[cfg(unix)]
    pub(super) reader: ttyr::reader::Reader,
    /// What the terminal said when last asked, kept where it does not
    /// answer in time when asked again.
    #[cfg(unix)]
    pub(super) last: Option<Probe>,
    /// Whether keys arrive as the bytes a terminal sends, turned into key
    /// events by the console ([`keys_from_console_bytes`]).
    #[cfg(windows)]
    console_bytes: bool,
}

/// The terminal's size, columns then rows: the controlling terminal's on
/// Unix, the console's on Windows.
pub fn size() -> io::Result<(u16, u16)> {
    #[cfg(unix)]
    return ttyr::terminal::size();
    #[cfg(windows)]
    return crossterm::terminal::size();
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

    /// The events the terminal sent together, as the server takes them (a
    /// click's press and release): waiting for the first, then each one
    /// already waiting. Empty when there was nothing but a wake, or (on
    /// Windows) nothing within [`INPUT_POLL`](super::input::INPUT_POLL).
    #[cfg(unix)]
    pub(super) fn read_batch(&mut self) -> io::Result<Vec<Event>> {
        let mut read = vec![self.reader.read()?];
        while self.reader.poll(Duration::ZERO)? {
            read.push(self.reader.read()?);
        }
        read.retain(|ev| *ev != Event::Wake);
        Ok(read)
    }

    /// As the Unix one; the console's reader cannot be woken, so it waits
    /// no longer than [`INPUT_POLL`](super::input::INPUT_POLL), for the thread to look for a pause.
    #[cfg(windows)]
    pub(super) fn read_batch(&mut self) -> io::Result<Vec<Event>> {
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

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use ttyr::event::KeyEvent;

    pub(in crate::terminal) fn key(code: KeyCode, mods: KeyModifiers) -> Event {
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
