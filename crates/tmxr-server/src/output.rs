//! A pane's output on its way from the reader thread to the server loop.
//!
//! The reader appends to a bounded queue and blocks while it is full, so a
//! program printing faster than the server can parse is slowed down rather
//! than queuing output without limit. The event channel only carries a
//! wake-up when the queue goes from empty to non-empty, so a client's key
//! waits behind at most one batch of each pane's output.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

/// Bytes a pane may have queued before its reader waits for the server.
pub const QUEUE_LIMIT: usize = 256 * 1024;

#[derive(Debug, Default)]
struct State {
    bytes: Vec<u8>,
    closed: bool,
}

#[derive(Debug, Default)]
pub struct OutputQueue {
    state: Mutex<State>,
    space: Condvar,
}

impl OutputQueue {
    fn lock(&self) -> MutexGuard<'_, State> {
        // The state is a byte buffer and a flag, valid whatever a panicking
        // holder left behind.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Append `bytes`, first waiting while the queue is full. Whether the
    /// queue was empty, so the server needs waking; `false` once closed.
    pub fn push(&self, bytes: &[u8]) -> bool {
        let mut s = self.lock();
        while s.bytes.len() >= QUEUE_LIMIT && !s.closed {
            s = self.space.wait(s).unwrap_or_else(PoisonError::into_inner);
        }
        if s.closed {
            return false;
        }
        let was_empty = s.bytes.is_empty();
        s.bytes.extend_from_slice(bytes);
        was_empty
    }

    /// Everything queued, making room for the reader.
    pub fn take(&self) -> Vec<u8> {
        let bytes = std::mem::take(&mut self.lock().bytes);
        self.space.notify_all();
        bytes
    }

    fn close(&self) {
        let mut s = self.lock();
        s.closed = true;
        s.bytes = Vec::new();
        drop(s);
        self.space.notify_all();
    }
}

/// The server's end of a queue: closing it when the pane goes releases a
/// reader blocked on a full queue, which would otherwise wait forever.
#[derive(Debug, Default)]
pub struct OutputHandle(pub Arc<OutputQueue>);

impl Drop for OutputHandle {
    fn drop(&mut self) {
        self.0.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn only_the_first_push_into_an_empty_queue_wakes_the_server() {
        let q = OutputQueue::default();
        assert!(q.push(b"a"));
        assert!(!q.push(b"b"));
        assert_eq!(q.take(), b"ab");
        assert!(q.push(b"c"));
    }

    #[test]
    fn a_full_queue_blocks_the_reader_until_taken_or_closed() {
        let handle = OutputHandle::default();
        let q = Arc::clone(&handle.0);
        q.push(&vec![0; QUEUE_LIMIT]);
        let reader = {
            let q = Arc::clone(&q);
            std::thread::spawn(move || q.push(b"more"))
        };
        std::thread::sleep(Duration::from_millis(100));
        assert!(!reader.is_finished(), "pushed into a full queue");
        assert_eq!(q.take().len(), QUEUE_LIMIT);
        assert!(reader.join().unwrap(), "woke the server after the take");
        assert_eq!(q.take(), b"more");

        q.push(&vec![0; QUEUE_LIMIT]);
        let reader = {
            let q = Arc::clone(&q);
            std::thread::spawn(move || q.push(b"late"))
        };
        std::thread::sleep(Duration::from_millis(100));
        drop(handle);
        assert!(!reader.join().unwrap(), "closed queues take nothing");
        assert!(q.take().is_empty());
    }
}
