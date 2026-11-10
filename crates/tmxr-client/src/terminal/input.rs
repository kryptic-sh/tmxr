//! The input pump: the thread forwarding the terminal's input to the server.

use std::io;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::Thread;
use std::time::Duration;

use interprocess::local_socket::Stream;
use tmxr_proto::{ClientMsg, write_msg};
use ttyr::event::Event;

use super::Keys;

/// How often a paused input thread looks whether it may read again, if the
/// pause ending does not wake it first. Reading, it waits for an event
/// without a timeout, and a pause wakes it ([`ttyr::reader::Waker`]).
pub(super) const INPUT_POLL: Duration = Duration::from_millis(50);

/// What [`Input::kitty_keyboard`] holds when the reader has been told all
/// there is; else it holds [`kitty_wanted`] of what it is to be told.
const KITTY_TOLD: u8 = 0;

/// What [`Input::kitty_keyboard`] holds for a reader to be told the kitty
/// protocol is `on`.
fn kitty_wanted(on: bool) -> u8 {
    if on { 2 } else { 1 }
}

/// Tell `keys`' reader what `wanted` holds for it, if anything, and clear
/// it.
fn tell_reader(wanted: &AtomicU8, keys: &mut Keys) {
    let told = wanted.swap(KITTY_TOLD, Ordering::AcqRel);
    if told != KITTY_TOLD {
        keys.set_kitty_keyboard(told == kitty_wanted(true));
    }
}

/// The input thread, which can be paused while another program (the lock
/// command, the shell a suspend returns to) owns the terminal, so that
/// program gets the keys and not the panes, and while the terminal is asked
/// again what it speaks.
pub struct Input {
    paused: Arc<AtomicBool>,
    /// The terminal's input, held by the thread while it reads.
    keys: Arc<Mutex<Keys>>,
    /// Ends a read the thread is waiting in.
    waker: ttyr::reader::Waker,
    /// Whether the reader is to be told the kitty protocol is on or off
    /// before its next read ([`Input::set_kitty_keyboard`]).
    kitty_keyboard: Arc<AtomicU8>,
    /// The input thread, woken as a pause ends.
    thread: Thread,
}

/// While held, the input thread reads nothing, and the terminal's input is
/// lent here.
pub struct Paused<'a> {
    keys: MutexGuard<'a, Keys>,
    paused: &'a AtomicBool,
    thread: &'a Thread,
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
        self.thread.unpark();
    }
}

impl Input {
    /// Stop reading the terminal; returns once the thread has stopped.
    pub fn pause(&self) -> io::Result<Paused<'_>> {
        // The flag keeps the thread from taking the input again before
        // this does; the lock waits out a read already under way.
        self.paused.store(true, Ordering::Release);
        self.waker.wake()?;
        let mut keys = self
            .keys
            .lock()
            .map_err(|_| io::Error::other("input thread panicked"))?;
        // Told now, so what the borrower tells the reader is not undone
        // when the thread reads again.
        tell_reader(&self.kitty_keyboard, &mut keys);
        Ok(Paused {
            keys,
            paused: &self.paused,
            thread: &self.thread,
        })
    }

    /// Have the reader told whether the kitty keyboard protocol is on (see
    /// [`Keys::set_kitty_keyboard`]) before it reads again, without waiting
    /// for the thread: a read under way is woken, and the thread tells it
    /// before the next.
    pub fn set_kitty_keyboard(&self, on: bool) -> io::Result<()> {
        self.kitty_keyboard
            .store(kitty_wanted(on), Ordering::Release);
        self.waker.wake()
    }
}

/// The connection to the server, shared by the threads that write to it,
/// each frame written whole under the lock.
pub type Sender = Arc<Mutex<Arc<Stream>>>;

/// Forward terminal events to the server until either side goes away,
/// `first` (what [`Keys::resized_since`] and [`Guard::enter`](super::Guard::enter) read) ahead of
/// the rest.
pub fn spawn_input(send: Sender, keys: Keys, first: Vec<Event>) -> io::Result<Input> {
    let (input, _thread) = spawn_pump(keys, first, move |events| forward(&send, events))?;
    Ok(input)
}

/// The input thread, handing each batch it reads to `forward` until that
/// or the read fails; and its handle, for a test to wait for it to end.
fn spawn_pump(
    keys: Keys,
    first: Vec<Event>,
    mut forward: impl FnMut(Vec<Event>) -> io::Result<()> + Send + 'static,
) -> io::Result<(Input, std::thread::JoinHandle<()>)> {
    let waker = keys.reader.waker();
    let (paused, keys) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(keys)));
    let kitty_keyboard = Arc::new(AtomicU8::new(KITTY_TOLD));
    let handle = {
        let (paused, keys, kitty_keyboard) = (
            Arc::clone(&paused),
            Arc::clone(&keys),
            Arc::clone(&kitty_keyboard),
        );
        std::thread::Builder::new()
            .name("tmxr-input".into())
            .spawn(move || {
                if !first.is_empty() && forward(first).is_err() {
                    return;
                }
                loop {
                    if paused.load(Ordering::Acquire) {
                        std::thread::park_timeout(INPUT_POLL);
                        continue;
                    }
                    let Ok(events) = read_batch(&keys, &kitty_keyboard) else {
                        break;
                    };
                    if !events.is_empty() && forward(events).is_err() {
                        break;
                    }
                }
            })?
    };
    let input = Input {
        paused,
        keys,
        waker,
        kitty_keyboard,
        thread: handle.thread().clone(),
    };
    Ok((input, handle))
}

/// One batch of events from `keys`, its reader told first what
/// `kitty_keyboard` holds for it.
fn read_batch(keys: &Mutex<Keys>, kitty_keyboard: &AtomicU8) -> io::Result<Vec<Event>> {
    let mut reading = keys
        .lock()
        .map_err(|_| io::Error::other("a borrower of the input panicked"))?;
    tell_reader(kitty_keyboard, &mut reading);
    reading.read_batch()
}

/// Send `events` to the server as input.
pub fn forward(send: &Sender, events: Vec<Event>) -> io::Result<()> {
    let send = send
        .lock()
        .map_err(|_| io::Error::other("a writer to the server panicked"))?;
    write_msg(&mut &**send, &ClientMsg::Input(events)).map_err(|e| io::Error::other(e.to_string()))
}

/// The input's switches over a socket pair in place of a terminal. Unix
/// only: a socket pair stands in for a terminal only there; the Windows
/// console's are below.
#[cfg(all(test, unix))]
mod tests {
    use std::io::Write as _;
    use std::sync::mpsc;
    use std::time::Instant;

    use ttyr::event::KeyModifiers;

    use super::*;
    use crate::terminal::probe::reader_tests::{keys, turn};

    /// An input over `keys` with no thread reading it: the test reads, and
    /// is the thread a pause ending wakes.
    fn input(keys: Keys) -> Input {
        Input {
            paused: Arc::new(AtomicBool::new(false)),
            waker: keys.reader.waker(),
            keys: Arc::new(Mutex::new(keys)),
            kitty_keyboard: Arc::new(AtomicU8::new(KITTY_TOLD)),
            thread: std::thread::current(),
        }
    }

    /// Telling the reader the protocol is on waits for no read under way:
    /// it returns while the input is held, as by the thread in a read, and
    /// the reader is told before it reads next.
    #[test]
    fn the_kitty_switch_does_not_wait_for_the_reader() {
        let _turn = turn();
        let (keys, mut terminal) = keys();
        let input = Arc::new(input(keys));
        let reading = input.keys.lock().unwrap();
        let (done, told) = mpsc::channel();
        // Apart, so a switch that waited fails the test rather than hang it.
        let switching = Arc::clone(&input);
        std::thread::spawn(move || {
            switching.set_kitty_keyboard(true).unwrap();
            let _ = done.send(());
        });
        told.recv_timeout(Duration::from_secs(5))
            .expect("waited for the reader");
        drop(reading);
        terminal.write_all(b"\x1b[1;9A").unwrap();
        let mods = loop {
            let events = read_batch(&input.keys, &input.kitty_keyboard).unwrap();
            if let Some(Event::Key(key)) = events.into_iter().next() {
                break key.modifiers;
            }
        };
        assert_eq!(mods, KeyModifiers::SUPER);
    }

    /// A pause tells the reader what it was to be told, so its borrower's
    /// word is not undone after; and its end wakes the thread at once.
    #[test]
    fn a_pause_tells_the_reader_first_and_its_end_wakes_the_thread() {
        let _turn = turn();
        let (keys, _terminal) = keys();
        let input = input(keys);
        input.set_kitty_keyboard(true).unwrap();
        let paused = input.pause().unwrap();
        assert!(paused.kitty_keyboard);
        assert_eq!(input.kitty_keyboard.load(Ordering::Acquire), KITTY_TOLD);
        drop(paused);
        let start = Instant::now();
        std::thread::park_timeout(Duration::from_secs(5));
        assert!(start.elapsed() < INPUT_POLL, "{:?}", start.elapsed());
    }
}

/// The input's pause on a real Windows console: the reader is woken out of
/// its wait. Windows only; first run in CI.
#[cfg(all(test, windows))]
mod console_tests {
    use std::time::Instant;

    use super::*;
    use crate::terminal::keys::console_tests::{console, inject, keys, record};

    /// A pause returns at once while the input thread waits for a key, as
    /// the waker ends the wait; once it ends the thread reads again.
    #[test]
    fn a_pause_wakes_a_thread_waiting_for_a_key() {
        let _turn = console();
        let (read, reads) = std::sync::mpsc::channel();
        let (input, thread) = spawn_pump(keys(), Vec::new(), move |events| {
            read.send(events).map_err(|_| io::Error::other("test over"))
        })
        .unwrap();
        // Long enough for the thread to be waiting in the reader.
        std::thread::sleep(Duration::from_millis(200));
        let start = Instant::now();
        let lent = input.pause().unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
        drop(lent);
        inject(&[record(u16::from(b'X'), u16::from(b'x'), true, 0)]);
        let events = reads.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            matches!(&events[..], [Event::Key(k)] if k.key == ttyr::event::KeyCode::Char('x')),
            "{events:?}"
        );
        // The next key it reads has nowhere to go, which ends the thread,
        // and the reader with it, for the next test to open its own.
        drop(reads);
        inject(&[record(u16::from(b'Y'), u16::from(b'y'), true, 0)]);
        thread.join().unwrap();
        drop(input);
    }
}
