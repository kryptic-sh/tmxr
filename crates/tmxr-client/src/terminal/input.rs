//! The input pump: the thread forwarding the terminal's input to the server.

use std::io;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use interprocess::local_socket::Stream;
use tmxr_proto::{ClientMsg, write_msg};
use ttyr::event::Event;

use super::Keys;

/// How often a paused input thread looks whether it may read again, and on
/// Windows how long it waits for an event before checking whether it should
/// pause (the most an [`Input::pause`] waits there). On Unix it waits for
/// an event without a timeout, and a pause wakes it.
pub(super) const INPUT_POLL: Duration = Duration::from_millis(50);

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

/// The connection to the server, shared by the threads that write to it,
/// each frame written whole under the lock.
pub type Sender = Arc<Mutex<Arc<Stream>>>;

/// Forward terminal events to the server until either side goes away,
/// `first` (what [`Keys::resized_since`] and [`Guard::enter`](super::Guard::enter) read) ahead of
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
