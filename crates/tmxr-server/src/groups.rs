//! Session groups (`new-session -t`): sessions that share one list of
//! windows, as tmux's session_group. Each member keeps its own current and
//! last window; a window made, killed, moved or renumbered in one member
//! goes to the others too.

use std::collections::BTreeMap;

use crate::model::{SessionId, WindowId};
use crate::server::Server;

/// One session group.
#[derive(Debug, Clone)]
pub struct SessionGroup {
    pub name: String,
    /// The members, in the order they joined (tmux's list).
    pub sessions: Vec<SessionId>,
    /// The window list as last synchronised: the member whose list no
    /// longer matches it is the one that changed, and the others follow it.
    windows: BTreeMap<u32, WindowId>,
}

impl Server {
    /// The group `session` belongs to.
    pub fn group_of(&self, session: SessionId) -> Option<&SessionGroup> {
        self.groups.iter().find(|g| g.sessions.contains(&session))
    }

    /// Whether `a` and `b` are two sessions of one group, between which tmux
    /// moves, links and swaps no windows: they hold the same ones.
    pub fn grouped_together(&self, a: SessionId, b: SessionId) -> bool {
        a != b && self.group_of(a).is_some_and(|g| g.sessions.contains(&b))
    }

    /// Put new session `session` in the group `-t` names, as tmux's
    /// new-session: the group `with` is in (or a new one named after it),
    /// else the group called `name` (or a new one); then it takes the
    /// group's windows in place of its own and selects the first.
    pub fn join_group(&mut self, session: SessionId, with: Option<SessionId>, name: &str) {
        let at = with
            .and_then(|w| self.groups.iter().position(|g| g.sessions.contains(&w)))
            .or_else(|| {
                with.is_none()
                    .then(|| self.groups.iter().position(|g| g.name == name))
                    .flatten()
            });
        let at = at.unwrap_or_else(|| {
            let (name, sessions, windows) = match with.and_then(|w| self.sessions.get(&w)) {
                Some(w) => (w.name.clone(), vec![w.id], w.windows.clone()),
                None => (name.to_owned(), Vec::new(), BTreeMap::new()),
            };
            self.groups.push(SessionGroup {
                name,
                sessions,
                windows,
            });
            self.groups.len() - 1
        });
        let first_other = self.groups[at].sessions.first().copied();
        self.groups[at].sessions.push(session);
        let Some(from) = first_other else {
            // A group of one so far: its windows are the list.
            if let Some(s) = self.sessions.get(&session) {
                self.groups[at].windows = s.windows.clone();
            }
            return;
        };
        self.copy_windows(from, session);
        if let Some(s) = self.sessions.get_mut(&session)
            && let Some(&first) = s.windows.keys().next()
        {
            s.current = first;
            s.last = None;
        }
    }

    /// Bring every group's members to the same windows: a member whose list
    /// changed since the last pass gives its list to the others, as tmux's
    /// session_group_synchronize_from after anything that changes one. Dead
    /// members leave their group, and a group left empty goes.
    pub fn sync_groups(&mut self) {
        let live = &self.sessions;
        for g in &mut self.groups {
            g.sessions.retain(|s| live.contains_key(s));
        }
        self.groups.retain(|g| !g.sessions.is_empty());
        for at in 0..self.groups.len() {
            let group = &self.groups[at];
            let changed = group.sessions.iter().copied().find(|s| {
                self.sessions
                    .get(s)
                    .is_some_and(|s| s.windows != group.windows)
            });
            let Some(from) = changed else {
                continue;
            };
            // A member left with no windows is about to go.
            let windows = self.sessions[&from].windows.clone();
            if windows.is_empty() {
                continue;
            }
            for to in group.sessions.clone() {
                if to != from {
                    self.copy_windows(from, to);
                }
            }
            self.groups[at].windows = windows;
        }
    }

    /// Give session `to` the windows of `from`, as tmux's
    /// session_group_synchronize1: `to` keeps its current window if it is
    /// still there (else its last, a lower one or a higher one), its last if
    /// still there, and the alert flags of `from`'s links. A window no
    /// session holds any longer is killed.
    fn copy_windows(&mut self, from: SessionId, to: SessionId) {
        let Some(windows) = self.sessions.get(&from).map(|s| s.windows.clone()) else {
            return;
        };
        let Some(s) = self.sessions.get_mut(&to) else {
            return;
        };
        if !windows.contains_key(&s.current) {
            let last = s.last.filter(|l| windows.contains_key(l));
            let lower = windows.range(..s.current).next_back().map(|(i, _)| *i);
            let higher = windows.range(s.current..).next().map(|(i, _)| *i);
            if let Some(i) = last.or(lower).or(higher) {
                s.current = i;
            }
        }
        s.last = s
            .last
            .filter(|l| windows.contains_key(l) && *l != s.current);
        let old = std::mem::replace(&mut s.windows, windows.clone());
        for w in windows.values() {
            if let Some(win) = self.windows.get_mut(w) {
                for alerts in [&mut win.bell, &mut win.activity, &mut win.silence] {
                    if alerts.contains(&from) {
                        alerts.insert(to);
                    }
                }
            }
        }
        for w in old.values().filter(|w| !windows.values().any(|n| n == *w)) {
            if self.sessions_of_window(*w).is_empty() {
                self.kill_window(*w);
            } else if let Some(win) = self.windows.get_mut(w) {
                win.forget_session(to);
            }
        }
        self.mark_session_dirty(to);
    }
}
