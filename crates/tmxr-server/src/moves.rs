//! Moving windows between sessions and panes between windows
//! (`move-window`, `swap-window`, `join-pane`, `swap-pane -s`).

use crate::model::{PaneId, SessionId, WindowId};
use crate::server::{Server, SplitSize};

impl Server {
    /// Where `wid` goes in `dst`: `index` when it is free (or `wid`'s own),
    /// else the first free index.
    fn place_in(&self, wid: WindowId, dst: SessionId, index: Option<u32>) -> Result<u32, String> {
        let base = self.session_options(dst).base_index;
        let s = self.sessions.get(&dst).ok_or("no such session")?;
        Ok(match index {
            Some(i) if s.windows.get(&i).is_some_and(|w| *w != wid) => {
                return Err(format!("index {i} in use"));
            }
            Some(i) => i,
            None => (base..)
                .find(|i| !s.windows.contains_key(i))
                .unwrap_or(base),
        })
    }

    /// Move `wid` from session `src` to `index` in session `dst` (the first
    /// free index when `None`). Moving a session's last window elsewhere
    /// ends that session.
    pub fn move_window(
        &mut self,
        wid: WindowId,
        src: SessionId,
        dst: SessionId,
        index: Option<u32>,
        select: bool,
    ) -> Result<u32, String> {
        if src != dst
            && self
                .sessions
                .get(&dst)
                .and_then(|s| s.index_of(wid))
                .is_some()
        {
            return Err("window is already linked to that session".into());
        }
        let index = self.place_in(wid, dst, index)?;
        if src == dst {
            // Re-index in place: unlinking would end a one-window session.
            let s = self.sessions.get_mut(&dst).ok_or("no such session")?;
            let old = s.index_of(wid).ok_or("window not in session")?;
            s.windows.remove(&old);
            s.windows.insert(index, wid);
            if s.current == old {
                s.current = index;
            }
            if s.last == Some(old) {
                s.last = Some(index);
            }
        } else {
            self.unlink_window_from(src, wid);
            self.sessions
                .get_mut(&dst)
                .ok_or("no such session")?
                .windows
                .insert(index, wid);
            self.fit_window(wid, dst);
            self.mark_session_dirty(src);
        }
        if select {
            self.select_window(dst, index)?;
        }
        self.mark_session_dirty(dst);
        Ok(index)
    }

    /// Link `wid` into session `dst` as well, at `index` (the first free
    /// index when `None`): one window, shown in both.
    /// Make room for a window next to the one at index `at` of session
    /// `sid`: after it, or with `before` in its place. The windows from
    /// there to the next free index move up by one, as tmux's
    /// `winlink_shuffle_up`, and the current and last window stay the same
    /// windows. Returns the index made free.
    pub fn shuffle_up(&mut self, sid: SessionId, at: u32, before: bool) -> Option<u32> {
        let s = self.sessions.get_mut(&sid)?;
        let idx = if before { at } else { at.checked_add(1)? };
        let free = (idx..u32::MAX).find(|i| !s.windows.contains_key(i))?;
        let current = s.windows.get(&s.current).copied();
        let last = s.last.and_then(|l| s.windows.get(&l).copied());
        for i in (idx..free).rev() {
            if let Some(w) = s.windows.remove(&i) {
                s.windows.insert(i + 1, w);
            }
        }
        if let Some(c) = current.and_then(|w| s.index_of(w)) {
            s.current = c;
        }
        s.last = last.and_then(|w| s.index_of(w));
        self.mark_session_dirty(sid);
        Some(idx)
    }

    /// The index `-a` / `-b` give a window put in session `sid` next to
    /// `index` (the current window when that holds none): the index
    /// [`Server::shuffle_up`] frees, else `index` itself, as tmux's.
    /// `-k`: take the window at `index` out of `session` to make room. It
    /// is taken out of the map directly, not unlinked, so a session whose
    /// only window it is does not end before the new one arrives;
    /// [`Server::finish_replace`] settles it after.
    pub fn take_index(
        &mut self,
        session: SessionId,
        index: Option<u32>,
    ) -> Option<(u32, WindowId)> {
        self.sessions
            .get_mut(&session)?
            .windows
            .remove_entry(&index?)
    }

    /// After a window was placed where [`Server::take_index`] made room: the
    /// taken window goes back if placing failed, else it is killed unless
    /// another session links it.
    pub fn finish_replace(
        &mut self,
        session: SessionId,
        taken: Option<(u32, WindowId)>,
        placed: bool,
    ) {
        let Some((i, old)) = taken else {
            return;
        };
        if !placed {
            if let Some(s) = self.sessions.get_mut(&session) {
                s.windows.insert(i, old);
            }
        } else if self.session_of_window(old).is_none() {
            self.kill_window(old);
        }
    }

    pub fn index_beside(
        &mut self,
        sid: SessionId,
        index: Option<u32>,
        before: bool,
    ) -> Option<u32> {
        let s = self.sessions.get(&sid)?;
        let at = index
            .filter(|i| s.windows.contains_key(i))
            .unwrap_or(s.current);
        self.shuffle_up(sid, at, before).or(index)
    }

    pub fn link_window(
        &mut self,
        wid: WindowId,
        dst: SessionId,
        index: Option<u32>,
        select: bool,
    ) -> Result<u32, String> {
        if self
            .sessions
            .get(&dst)
            .and_then(|s| s.index_of(wid))
            .is_some()
        {
            return Err("window is already linked to that session".into());
        }
        let index = self.place_in(wid, dst, index)?;
        self.sessions
            .get_mut(&dst)
            .ok_or("no such session")?
            .windows
            .insert(index, wid);
        if select {
            self.select_window(dst, index)?;
        }
        self.mark_session_dirty(dst);
        Ok(index)
    }

    /// Exchange the windows at two places (session, index), which may be in
    /// different sessions. Unless `keep`, the window moved into `b`'s place
    /// becomes current there.
    pub fn swap_windows(
        &mut self,
        (sa, ia): (SessionId, u32),
        (sb, ib): (SessionId, u32),
        keep: bool,
    ) -> Result<(), String> {
        let at = |srv: &Self, s: SessionId, i: u32| {
            srv.sessions
                .get(&s)
                .and_then(|s| s.windows.get(&i).copied())
                .ok_or_else(|| format!("window not found: {i}"))
        };
        let (a, b) = (at(self, sa, ia)?, at(self, sb, ib)?);
        if a == b {
            return Ok(());
        }
        if let Some(s) = self.sessions.get_mut(&sa) {
            s.windows.insert(ia, b);
        }
        if let Some(s) = self.sessions.get_mut(&sb) {
            s.windows.insert(ib, a);
        }
        if sa != sb {
            self.fit_window(a, sb);
            self.fit_window(b, sa);
        }
        if !keep {
            self.select_window(sb, ib)?;
        }
        self.mark_session_dirty(sa);
        self.mark_session_dirty(sb);
        Ok(())
    }

    /// Move pane `src` out of its window and split `dst` to hold it, as
    /// `split-window` would place a new pane. A window left empty is closed.
    #[allow(clippy::too_many_arguments)]
    pub fn join_pane(
        &mut self,
        src: PaneId,
        dst: PaneId,
        horizontal: bool,
        before: bool,
        full: bool,
        size: Option<SplitSize>,
        focus: bool,
    ) -> Result<(), String> {
        if src == dst {
            return Err("source and target panes must be different".into());
        }
        let from = self.panes.get(&src).ok_or("no such pane")?.window;
        let to = self.panes.get(&dst).ok_or("no such pane")?.window;
        let r = self.panes[&dst].rect;
        let len = match (full, self.windows.get(&to)) {
            (true, Some(w)) if horizontal => w.cols,
            (true, Some(w)) => w.rows,
            _ if horizontal => r.w,
            _ => r.h,
        };
        if len < 3 {
            return Err("pane too small".into());
        }
        if self
            .windows
            .get(&from)
            .is_some_and(|w| w.panes().len() == 1)
        {
            self.unlink_window(from);
            self.windows.remove(&from);
        } else {
            self.remove_pane_from_window(from, src);
        }
        if let Some(p) = self.panes.get_mut(&src) {
            p.window = to;
        }
        if let Err(e) = self.insert_leaf(dst, src, horizontal, before, full, size) {
            // The pane is out of its old window already: give it a window of
            // its own rather than lose it.
            let session = self.session_of_window(to).ok_or("window has no session")?;
            let (cols, rows) = (self.windows[&to].cols, self.windows[&to].rows);
            let name = self.commands.get(&src).cloned().unwrap_or_default();
            let new = self.adopt_pane(session, src, name, None, cols, rows, false)?;
            self.relayout(new);
            return Err(e);
        }
        if focus {
            self.select_pane(src);
        }
        self.relayout(to);
        self.mark_window_dirty(from);
        Ok(())
    }

    /// Exchange two panes, which may be in different windows. Unless `keep`,
    /// `src` becomes the active pane of the window it moved into.
    pub fn swap_panes(&mut self, src: PaneId, dst: PaneId, keep: bool) -> Result<(), String> {
        if src == dst {
            return Ok(());
        }
        let ws = self.panes.get(&src).ok_or("no such pane")?.window;
        let wd = self.panes.get(&dst).ok_or("no such pane")?.window;
        let windows = if ws == wd { vec![ws] } else { vec![ws, wd] };
        for w in windows {
            let win = self.windows.get_mut(&w).ok_or("no such window")?;
            win.layout = crate::layout::swap(&win.layout, src, dst);
            // tmux's pane list swaps them too.
            for p in &mut win.order {
                *p = swapped(*p, src, dst);
            }
            if ws != wd {
                // Only one of the two is in this window; it is replaced.
                win.active = swapped(win.active, src, dst);
                win.last_pane = win.last_pane.map(|p| swapped(p, src, dst));
            }
        }
        if ws != wd {
            if let Some(p) = self.panes.get_mut(&src) {
                p.window = wd;
            }
            if let Some(p) = self.panes.get_mut(&dst) {
                p.window = ws;
            }
        }
        // Which pane is active after, as tmux's `cmd_swap_pane_exec`: the
        // target's window takes the source and the source's the target; in
        // one window, the target. With -d, panes that moved away from being
        // active across windows were replaced above, and in one window an
        // active target hands over to the source.
        if !keep {
            if let Some(w) = self.windows.get_mut(&ws) {
                w.active = dst;
            }
            if ws != wd
                && let Some(w) = self.windows.get_mut(&wd)
            {
                w.active = src;
            }
        } else if ws == wd
            && let Some(w) = self.windows.get_mut(&ws)
            && w.active == dst
        {
            w.active = src;
        }
        self.relayout(ws);
        if ws != wd {
            self.relayout(wd);
        }
        Ok(())
    }

    /// Size a window that just joined `session` like the session's other
    /// windows, so it fits the clients attached there.
    fn fit_window(&mut self, wid: WindowId, session: SessionId) {
        let size = self.sessions.get(&session).and_then(|s| {
            s.windows
                .values()
                .filter(|w| **w != wid)
                .find_map(|w| self.windows.get(w))
                .map(|w| (w.cols, w.rows))
        });
        if let Some((cols, rows)) = size
            && let Some(win) = self.windows.get_mut(&wid)
            && (win.cols, win.rows) != (cols, rows)
        {
            win.cols = cols;
            win.rows = rows;
            self.relayout(wid);
        }
    }
}

/// `p` with `a` and `b` exchanged.
fn swapped(p: PaneId, a: PaneId, b: PaneId) -> PaneId {
    match p {
        p if p == a => b,
        p if p == b => a,
        p => p,
    }
}
