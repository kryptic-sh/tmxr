//! Creating, selecting and closing sessions and windows.

use crate::cmds::Ctx;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use hjkl_layout::LayoutTree;

use super::Server;
use crate::model::{ClientId, PaneId, Session, SessionId, Window, WindowId};

impl Server {
    pub fn alloc_session_id(&mut self) -> SessionId {
        let id = self.next_session;
        self.next_session += 1;
        id
    }

    /// The first unused numeric session name ("0", "1", …), as tmux does.
    pub fn next_session_name(&self) -> String {
        (0..)
            .map(|n: u32| n.to_string())
            .find(|n| !self.sessions.values().any(|s| &s.name == n))
            .unwrap_or_default()
    }

    pub fn new_session(
        &mut self,
        name: Option<String>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        window_name: Option<String>,
        argv: Vec<String>,
        size: (u16, u16),
    ) -> Result<SessionId, String> {
        let name = match name {
            Some(n) if self.sessions.values().any(|s| s.name == n) => {
                return Err(format!("duplicate session: {n}"));
            }
            Some(n) if n.is_empty() || n.contains([':', '.']) => {
                return Err(format!("bad session name: {n}"));
            }
            Some(n) => n,
            None => self.next_session_name(),
        };
        let id = self.alloc_session_id();
        self.sessions.insert(
            id,
            Session {
                id,
                name,
                windows: BTreeMap::new(),
                current: self.cfg.base_index,
                last: None,
                cwd: cwd.clone(),
                env: env.into_iter().collect(),
                hooks: crate::hooks::HookTable::new(),
                created: SystemTime::now(),
                last_used: Instant::now(),
            },
        );
        if let Err(e) = self.new_window(id, None, window_name, Some(cwd), argv, size, true) {
            self.sessions.remove(&id);
            return Err(e);
        }
        self.had_session = true;
        let ctx = Ctx {
            pane: self.active_pane_of_session(id),
            ..Ctx::default()
        };
        self.queue_hook("session-created", ctx, None);
        Ok(id)
    }

    /// Create a window in `session` at `index` (or the first free index).
    #[allow(clippy::too_many_arguments)]
    pub fn new_window(
        &mut self,
        session: SessionId,
        index: Option<u32>,
        name: Option<String>,
        cwd: Option<PathBuf>,
        argv: Vec<String>,
        size: (u16, u16),
        select: bool,
    ) -> Result<WindowId, String> {
        let s = self.sessions.get(&session).ok_or("no such session")?;
        let index = match index {
            Some(i) if s.windows.contains_key(&i) => return Err(format!("index {i} in use")),
            Some(i) => i,
            None => (self.cfg.base_index..)
                .find(|i| !s.windows.contains_key(i))
                .unwrap_or(self.cfg.base_index),
        };
        let cwd = cwd.unwrap_or_else(|| s.cwd.clone());
        let wid = self.next_window;
        self.next_window += 1;
        let pid = self.next_pane;
        let (cols, rows) = size;
        let auto_name = name.is_none();
        self.windows.insert(
            wid,
            Window {
                id: wid,
                name: name.unwrap_or_default(),
                auto_name,
                layout: LayoutTree::Leaf(pid as usize),
                active: pid,
                last_pane: None,
                zoomed: false,
                synchronize: false,
                bell: false,
                cols,
                rows,
                preset: 0,
            },
        );
        if let Err(e) = self.spawn_pane(pid, wid, session, &argv, cwd, cols, rows) {
            self.windows.remove(&wid);
            return Err(e);
        }
        if let Some(win) = self.windows.get_mut(&wid)
            && win.auto_name
        {
            win.name = crate::util::program_name(&argv, self.cfg.default_shell.as_deref());
        }
        let s = self.sessions.get_mut(&session).ok_or("no such session")?;
        s.windows.insert(index, wid);
        if select || s.windows.len() == 1 {
            if s.current != index && s.windows.contains_key(&s.current) {
                s.last = Some(s.current);
            }
            s.current = index;
        }
        self.mark_session_dirty(session);
        Ok(wid)
    }

    pub fn session_of_window(&self, window: WindowId) -> Option<SessionId> {
        self.sessions
            .values()
            .find(|s| s.windows.values().any(|w| *w == window))
            .map(|s| s.id)
    }

    /// Make window `index` current in `session`.
    pub fn select_window(&mut self, session: SessionId, index: u32) -> Result<(), String> {
        let s = self.sessions.get_mut(&session).ok_or("no such session")?;
        if !s.windows.contains_key(&index) {
            return Err(format!("window not found: {index}"));
        }
        if s.current != index {
            s.last = Some(s.current);
            s.current = index;
        }
        if let Some(w) = s.windows.get(&index).copied()
            && let Some(win) = self.windows.get_mut(&w)
        {
            win.bell = false;
        }
        self.mark_session_dirty(session);
        Ok(())
    }

    pub fn kill_window(&mut self, wid: WindowId) {
        let Some(win) = self.windows.remove(&wid) else {
            return;
        };
        for p in win.panes() {
            if let Some(mut pane) = self.panes.remove(&p) {
                let _ = pane.pty.kill();
            }
            self.commands.remove(&p);
        }
        self.unlink_window(wid);
    }

    /// Take `wid` out of its session's window list, choosing the session's
    /// next current window and renumbering like tmux. A session left without
    /// windows is killed. The window itself and its panes are untouched.
    pub fn unlink_window(&mut self, wid: WindowId) {
        let Some(sid) = self.session_of_window(wid) else {
            return;
        };
        let s = self.sessions.get_mut(&sid).expect("session found above");
        let index = s.index_of(wid);
        if let Some(i) = index {
            s.windows.remove(&i);
        }
        if s.windows.is_empty() {
            self.kill_session(sid);
            return;
        }
        if index == Some(s.current) {
            // tmux moves to the last window, else the next one.
            let next = s
                .last
                .filter(|l| s.windows.contains_key(l))
                .or_else(|| s.windows.range(s.current..).next().map(|(i, _)| *i))
                .or_else(|| s.windows.keys().next_back().copied());
            if let Some(n) = next {
                s.current = n;
            }
            s.last = None;
        }
        if self.cfg.renumber_windows {
            self.renumber_windows(sid);
        }
        self.mark_session_dirty(sid);
    }

    /// Close the gaps in `sid`'s window indices, from `base-index` up and in
    /// order, keeping its current and last window (`renumber-windows`,
    /// `move-window -r`).
    pub fn renumber_windows(&mut self, sid: SessionId) {
        let base = self.cfg.base_index;
        let Some(s) = self.sessions.get_mut(&sid) else {
            return;
        };
        let current_win = s.windows.get(&s.current).copied();
        let last_win = s.last.and_then(|l| s.windows.get(&l).copied());
        let ordered: Vec<WindowId> = s.windows.values().copied().collect();
        s.windows = ordered
            .iter()
            .enumerate()
            .map(|(i, w)| (base + i as u32, *w))
            .collect();
        if let Some(c) = current_win.and_then(|w| s.index_of(w)) {
            s.current = c;
        }
        s.last = last_win.and_then(|w| s.index_of(w));
        self.mark_session_dirty(sid);
    }

    pub fn kill_session(&mut self, sid: SessionId) {
        let Some(s) = self.sessions.remove(&sid) else {
            return;
        };
        self.queue_hook("session-closed", Ctx::default(), None);
        for w in s.windows.values().copied().collect::<Vec<_>>() {
            if let Some(win) = self.windows.remove(&w) {
                for p in win.panes() {
                    if let Some(mut pane) = self.panes.remove(&p) {
                        let _ = pane.pty.kill();
                    }
                }
            }
        }
        // Clients on the dead session move to another one, or detach when
        // there is none (tmux's detach-on-destroy off / on fallback).
        let fallback = self
            .sessions
            .values()
            .max_by_key(|s| s.last_used)
            .map(|s| s.id);
        let on_it: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| c.att.as_ref().is_some_and(|a| a.session == sid))
            .map(|c| c.id)
            .collect();
        for c in on_it {
            match fallback {
                Some(f) => self.switch_client(c, f),
                None => self.detach(c, "exited"),
            }
        }
        if self.sessions.is_empty() && self.had_session {
            self.exiting = true;
        }
    }

    /// Give `pane` (already removed from its old window's layout) a window of
    /// its own in `session`.
    pub fn adopt_pane(
        &mut self,
        session: SessionId,
        pane: PaneId,
        name: String,
        cols: u16,
        rows: u16,
        select: bool,
    ) -> Result<WindowId, String> {
        let s = self.sessions.get(&session).ok_or("no such session")?;
        let index = (self.cfg.base_index..)
            .find(|i| !s.windows.contains_key(i))
            .unwrap_or(self.cfg.base_index);
        let wid = self.next_window;
        self.next_window += 1;
        self.windows.insert(
            wid,
            Window {
                id: wid,
                name,
                auto_name: true,
                layout: LayoutTree::Leaf(pane as usize),
                active: pane,
                last_pane: None,
                zoomed: false,
                synchronize: false,
                bell: false,
                cols,
                rows,
                preset: 0,
            },
        );
        if let Some(p) = self.panes.get_mut(&pane) {
            p.window = wid;
        }
        let s = self.sessions.get_mut(&session).ok_or("no such session")?;
        s.windows.insert(index, wid);
        if select {
            s.last = Some(s.current);
            s.current = index;
        }
        self.mark_session_dirty(session);
        Ok(wid)
    }
}
