//! `wait-for` channels: command clients wait on a channel until another
//! command signals it (`-S`), or take turns holding its lock (`-L` / `-U`).

use std::collections::{HashMap, VecDeque};

use crate::model::ClientId;

/// What a command client waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wait {
    /// A signal on the channel.
    Signal(String),
    /// The channel's lock.
    Lock(String),
}

#[derive(Debug, Default)]
struct Channel {
    /// Clients waiting for a signal.
    waiters: Vec<ClientId>,
    /// Signalled with nobody waiting: the next wait returns at once.
    woken: bool,
    locked: bool,
    /// Clients waiting for the lock, first come first served.
    lockers: VecDeque<ClientId>,
}

impl Channel {
    fn idle(&self) -> bool {
        self.waiters.is_empty() && !self.woken && !self.locked && self.lockers.is_empty()
    }
}

/// Every channel in use.
#[derive(Debug, Default)]
pub struct Waits(HashMap<String, Channel>);

impl Waits {
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

    /// Queue `client` for what it waits for.
    pub fn enqueue(&mut self, client: ClientId, wait: &Wait) {
        match wait {
            Wait::Signal(name) => self.0.entry(name.clone()).or_default().waiters.push(client),
            Wait::Lock(name) => self
                .0
                .entry(name.clone())
                .or_default()
                .lockers
                .push_back(client),
        }
    }

    /// `wait-for -S name`: the clients to release. With none waiting, the
    /// next wait returns at once.
    pub fn signal(&mut self, name: &str) -> Vec<ClientId> {
        let ch = self.0.entry(name.to_owned()).or_default();
        let woken = std::mem::take(&mut ch.waiters);
        ch.woken = woken.is_empty();
        woken
    }

    /// `wait-for -U name`: the client the lock passes to, if one waits.
    pub fn unlock(&mut self, name: &str) -> Result<Option<ClientId>, String> {
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

    /// A client that went away waits for nothing any more.
    pub fn forget(&mut self, client: ClientId) {
        for ch in self.0.values_mut() {
            ch.waiters.retain(|c| *c != client);
            ch.lockers.retain(|c| *c != client);
        }
        self.0.retain(|_, ch| !ch.idle());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_releases_waiters_or_wakes_the_next_wait() {
        let mut w = Waits::default();
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
        let mut w = Waits::default();
        assert!(w.lock("l"));
        assert!(!w.lock("l"));
        w.enqueue(7, &Wait::Lock("l".into()));
        assert_eq!(w.unlock("l"), Ok(Some(7)), "client 7 holds it now");
        assert_eq!(w.unlock("l"), Ok(None), "and frees it");
        assert!(w.unlock("l").is_err());
    }

    #[test]
    fn a_departed_client_is_forgotten() {
        let mut w = Waits::default();
        w.enqueue(3, &Wait::Signal("x".into()));
        w.forget(3);
        assert!(w.signal("x").is_empty());
    }
}
