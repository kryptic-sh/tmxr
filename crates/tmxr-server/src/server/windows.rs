//! Creating, selecting and closing sessions and windows.

use crate::cmds::Ctx;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use hjkl_layout::LayoutTree;

use super::Server;
use crate::model::{ClientId, Environment, PaneId, Session, SessionId, Window, WindowId};

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
        env: Environment,
        window_name: Option<String>,
        argv: Vec<String>,
        size: (u16, u16),
    ) -> Result<SessionId, String> {
        let name = match name {
            Some(n) if self.sessions.values().any(|s| s.name == n) => {
                return Err(format!("duplicate session: {n}"));
            }
            Some(n) => {
                crate::target::check_session_name(&n)?;
                n
            }
            None => self.next_session_name(),
        };
        let id = self.alloc_session_id();
        self.sessions.insert(
            id,
            Session {
                id,
                name,
                windows: BTreeMap::new(),
                current: self.cfg.session.base_index,
                last: None,
                cwd: cwd.clone(),
                env,
                hooks: crate::hooks::HookTable::new(),
                options: tmxr_config::OwnSessionOptions::default(),
                created: SystemTime::now(),
                last_used: Instant::now(),
            },
        );
        if let Err(e) = self.new_window(id, None, window_name, Some(cwd), argv, &[], size, true) {
            self.sessions.remove(&id);
            return Err(e);
        }
        self.had_session = true;
        let ctx = Ctx {
            pane: self.active_pane_of_session(id),
            ..Ctx::default()
        };
        self.queue_hook("session-created", ctx, crate::hooks::HookScope::default());
        Ok(id)
    }

    /// Create a window in `session` at `index` (or the first free index);
    /// `env` is added to its pane's environment (`-e`).
    #[allow(clippy::too_many_arguments)]
    pub fn new_window(
        &mut self,
        session: SessionId,
        index: Option<u32>,
        name: Option<String>,
        cwd: Option<PathBuf>,
        argv: Vec<String>,
        env: &[(String, String)],
        size: (u16, u16),
        select: bool,
    ) -> Result<WindowId, String> {
        let base = self.session_options(session).base_index;
        let argv = self.new_pane_argv(session, argv);
        let s = self.sessions.get(&session).ok_or("no such session")?;
        let index = match index {
            Some(i) if s.windows.contains_key(&i) => return Err(format!("index {i} in use")),
            Some(i) => i,
            None => (base..)
                .find(|i| !s.windows.contains_key(i))
                .unwrap_or(base),
        };
        let cwd = cwd.unwrap_or_else(|| s.cwd.clone());
        let wid = self.next_window;
        self.next_window += 1;
        let pid = self.next_pane;
        let (cols, rows) = size;
        // A window named when made keeps its name, as tmux's.
        let mut options = tmxr_config::OwnOptions::default();
        options.automatic_rename = name.is_some().then_some(false);
        self.windows.insert(
            wid,
            Window {
                id: wid,
                name: name.unwrap_or_default(),
                options,
                layout: LayoutTree::Leaf(pid as usize),
                order: Vec::new(),
                active: pid,
                last_pane: None,
                zoomed: false,
                bell: BTreeSet::new(),
                cols,
                rows,
                preset: 0,
                activity: BTreeSet::new(),
                silence: BTreeSet::new(),
                last_output: Instant::now(),
                silence_raised: false,
                hooks: crate::hooks::HookTable::new(),
            },
        );
        if let Err(e) = self.spawn_pane(pid, wid, session, &argv, cwd, env, cols, rows) {
            self.windows.remove(&wid);
            return Err(e);
        }
        let shell = self.session_options(session).default_shell.clone();
        if self.window_options(wid).automatic_rename
            && let Some(win) = self.windows.get_mut(&wid)
        {
            win.name = crate::util::program_name(&argv, shell.as_deref());
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

    /// A session holding `window`: the first of them for a linked window.
    /// The session `pane`'s window is in, as [`Server::session_of_window`].
    pub fn session_of_pane(&self, pane: Option<PaneId>) -> Option<SessionId> {
        self.session_of_window(self.panes.get(&pane?)?.window)
    }

    pub fn session_of_window(&self, window: WindowId) -> Option<SessionId> {
        self.sessions
            .values()
            .find(|s| s.windows.values().any(|w| *w == window))
            .map(|s| s.id)
    }

    /// Whether `window` is the current window of a session holding it:
    /// alerts are for windows out of sight.
    /// Every session, in tmux's order: by name (`strcmp`), which lists show
    /// and `switch-client -n` / `-p` step through.
    pub fn sessions_by_name(&self) -> Vec<SessionId> {
        let mut all: Vec<_> = self.sessions.values().collect();
        all.sort_by(|a, b| a.name.cmp(&b.name));
        all.into_iter().map(|s| s.id).collect()
    }

    pub fn window_is_current(&self, window: WindowId) -> bool {
        self.sessions
            .values()
            .any(|s| s.current_window() == Some(window))
    }

    /// Every session holding `window` (more than one when it is linked).
    pub fn sessions_of_window(&self, window: WindowId) -> Vec<SessionId> {
        self.sessions
            .values()
            .filter(|s| s.windows.values().any(|w| *w == window))
            .map(|s| s.id)
            .collect()
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
            win.clear_alerts();
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

    /// Take `wid` out of every session holding it. The window itself and
    /// its panes are untouched.
    pub fn unlink_window(&mut self, wid: WindowId) {
        for sid in self.sessions_of_window(wid) {
            self.unlink_window_from(sid, wid);
        }
    }

    /// Take `wid` out of `sid`, and kill it when no other session holds it
    /// (the `-k` of `move-window`, `link-window` and `unlink-window`).
    pub fn unlink_or_kill(&mut self, sid: SessionId, wid: WindowId) {
        if self.sessions_of_window(wid).len() > 1 {
            self.unlink_window_from(sid, wid);
        } else {
            self.kill_window(wid);
        }
    }

    /// Take `wid` out of `sid`'s window list, choosing the session's next
    /// current window and renumbering like tmux. A session left without
    /// windows is killed. The window itself and its panes are untouched.
    pub fn unlink_window_from(&mut self, sid: SessionId, wid: WindowId) {
        let Some(s) = self.sessions.get_mut(&sid) else {
            return;
        };
        let index = s.index_of(wid);
        if let Some(i) = index {
            s.windows.remove(&i);
        }
        if let Some(w) = self.windows.get_mut(&wid) {
            w.forget_session(sid);
        }
        let Some(s) = self.sessions.get_mut(&sid) else {
            return;
        };
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
        if self.session_options(sid).renumber_windows {
            self.renumber_windows(sid);
        }
        self.mark_session_dirty(sid);
    }

    /// Close the gaps in `sid`'s window indices, from `base-index` up and in
    /// order, keeping its current and last window (`renumber-windows`,
    /// `move-window -r`).
    pub fn renumber_windows(&mut self, sid: SessionId) {
        let base = self.session_options(sid).base_index;
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
        self.queue_hook(
            "session-closed",
            Ctx::default(),
            crate::hooks::HookScope::default(),
        );
        for w in s.windows.values().copied().collect::<Vec<_>>() {
            // A window linked to another session lives on there.
            if self.session_of_window(w).is_some() {
                if let Some(win) = self.windows.get_mut(&w) {
                    win.forget_session(sid);
                }
                continue;
            }
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
        // Now rather than at the end of this pass, so the endpoint is gone
        // before the reply to whatever asked.
        self.check_exit();
    }

    /// Destroy every session no client is attached to whose
    /// `destroy-unattached` asks for it, as tmux's `server_check_unattached`;
    /// run when a client leaves or changes session, as there.
    pub fn check_unattached(&mut self) {
        let doomed: Vec<SessionId> = self
            .sessions
            .keys()
            .copied()
            .filter(|&s| {
                !self
                    .clients
                    .values()
                    .any(|c| c.att.as_ref().is_some_and(|a| a.session == s))
                    && self.session_options(s).destroy_unattached.destroys()
            })
            .collect();
        for s in doomed {
            self.kill_session(s);
        }
    }

    /// Exit when nothing holds the server, as tmux's `server_loop`: with
    /// `exit-empty` on, once no client is attached and (unless
    /// `exit-unattached`) no session is left. Not before a first session,
    /// so a server started for a client's command waits for it.
    pub fn check_exit(&mut self) {
        if !self.had_session || !self.cfg.exit_empty {
            return;
        }
        if !self.cfg.exit_unattached && !self.sessions.is_empty() {
            return;
        }
        if self.clients.values().any(|c| c.att.is_some()) {
            return;
        }
        self.begin_exit();
    }

    /// Give `pane` (already removed from its old window's layout) a window of
    /// its own in `session`, at `index` or else the first free one.
    #[allow(clippy::too_many_arguments)]
    pub fn adopt_pane(
        &mut self,
        session: SessionId,
        pane: PaneId,
        name: String,
        index: Option<u32>,
        cols: u16,
        rows: u16,
        select: bool,
    ) -> Result<WindowId, String> {
        let base = self.session_options(session).base_index;
        let s = self.sessions.get(&session).ok_or("no such session")?;
        let index = index.unwrap_or_else(|| {
            (base..)
                .find(|i| !s.windows.contains_key(i))
                .unwrap_or(base)
        });
        let wid = self.next_window;
        self.next_window += 1;
        self.windows.insert(
            wid,
            Window {
                id: wid,
                name,
                options: tmxr_config::OwnOptions::default(),
                layout: LayoutTree::Leaf(pane as usize),
                order: Vec::new(),
                active: pane,
                last_pane: None,
                zoomed: false,
                bell: BTreeSet::new(),
                cols,
                rows,
                preset: 0,
                activity: BTreeSet::new(),
                silence: BTreeSet::new(),
                last_output: Instant::now(),
                silence_raised: false,
                hooks: crate::hooks::HookTable::new(),
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
