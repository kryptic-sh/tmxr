//! `wait-for` channels: a command list waits on a channel until another
//! command signals it (`-S`), or takes turns holding its lock (`-L` / `-U`).
//! What waits is the rest of the list, as a job (`crate::jobs`).

use std::collections::{HashMap, VecDeque};

/// What a command list waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wait {
    /// A signal on the channel.
    Signal(String),
    /// The channel's lock.
    Lock(String),
}

struct Channel<T> {
    /// Waiting for a signal.
    waiters: Vec<T>,
    /// Signalled with nobody waiting: the next wait returns at once.
    woken: bool,
    locked: bool,
    /// Waiting for the lock, first come first served.
    lockers: VecDeque<T>,
}

impl<T> Default for Channel<T> {
    fn default() -> Self {
        Self {
            waiters: Vec::new(),
            woken: false,
            locked: false,
            lockers: VecDeque::new(),
        }
    }
}

impl<T> Channel<T> {
    fn idle(&self) -> bool {
        self.waiters.is_empty() && !self.woken && !self.locked && self.lockers.is_empty()
    }
}

/// Every channel in use, with what waits on each.
pub struct Waits<T>(HashMap<String, Channel<T>>);

impl<T> Default for Waits<T> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

impl<T> Waits<T> {
    /// `wait-for name`: whether it returns at once (a signal came first).
    pub fn wait(&mut self, name: &str) -> bool {
        let ch = self.0.entry(name.to_owned()).or_default();
        std::mem::take(&mut ch.woken)
    }

    /// `wait-for -L name`: whether the lock was free and is now held.
    pub fn lock(&mut self, name: &str) -> bool {
        let ch = self.0.entry(name.to_owned()).or_default();
        !std::mem::replace(&mut ch.locked, true)
    }

    /// Queue `waiter` for what it waits for.
    pub fn enqueue(&mut self, waiter: T, wait: &Wait) {
        match wait {
            Wait::Signal(name) => self.0.entry(name.clone()).or_default().waiters.push(waiter),
            Wait::Lock(name) => self
                .0
                .entry(name.clone())
                .or_default()
                .lockers
                .push_back(waiter),
        }
    }

    /// `wait-for -S name`: what to release. With nothing waiting, the next
    /// wait returns at once.
    pub fn signal(&mut self, name: &str) -> Vec<T> {
        let ch = self.0.entry(name.to_owned()).or_default();
        let woken = std::mem::take(&mut ch.waiters);
        ch.woken = woken.is_empty();
        woken
    }

    /// `wait-for -U name`: what the lock passes to, if anything waits.
    pub fn unlock(&mut self, name: &str) -> Result<Option<T>, String> {
        let ch = self
            .0
            .get_mut(name)
            .filter(|c| c.locked)
            .ok_or_else(|| format!("channel {name} not locked"))?;
        let next = ch.lockers.pop_front();
        ch.locked = next.is_some();
        if ch.idle() {
            self.0.remove(name);
        }
        Ok(next)
    }

    /// Stop waiting for whatever `gone` matches (a client that went away).
    pub fn forget(&mut self, gone: impl Fn(&T) -> bool) {
        for ch in self.0.values_mut() {
            ch.waiters.retain(|w| !gone(w));
            ch.lockers.retain(|w| !gone(w));
        }
        self.0.retain(|_, ch| !ch.idle());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_releases_waiters_or_wakes_the_next_wait() {
        let mut w = Waits::<u32>::default();
        assert!(!w.wait("done"));
        w.enqueue(1, &Wait::Signal("done".into()));
        w.enqueue(2, &Wait::Signal("done".into()));
        assert_eq!(w.signal("done"), [1, 2]);
        // A signal nobody waits for is kept for the next wait, once.
        assert!(w.signal("early").is_empty());
        assert!(w.wait("early"));
        assert!(!w.wait("early"));
    }

    #[test]
    fn the_lock_passes_in_turn_and_unlocking_twice_fails() {
        let mut w = Waits::<u32>::default();
        assert!(w.lock("l"));
        assert!(!w.lock("l"));
        w.enqueue(7, &Wait::Lock("l".into()));
        assert_eq!(w.unlock("l"), Ok(Some(7)), "client 7 holds it now");
        assert_eq!(w.unlock("l"), Ok(None), "and frees it");
        assert!(w.unlock("l").is_err());
    }

    #[test]
    fn a_departed_client_is_forgotten() {
        let mut w = Waits::<u32>::default();
        w.enqueue(3, &Wait::Signal("x".into()));
        w.forget(|c| *c == 3);
        assert!(w.signal("x").is_empty());
    }
}
