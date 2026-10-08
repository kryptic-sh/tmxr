//! Moving windows between sessions and panes between windows
//! (`move-window`, `swap-window`, `join-pane`, `swap-pane -s`).

use crate::model::{PaneId, SessionId, WindowId};
use crate::server::{Server, SplitSize};

impl Server {
    /// Move `wid` to `index` in session `dst` (the first free index when
    /// `None`). Moving a session's last window elsewhere ends that session.
    pub fn move_window(
        &mut self,
        wid: WindowId,
        dst: SessionId,
        index: Option<u32>,
        select: bool,
    ) -> Result<u32, String> {
        let src = self.session_of_window(wid).ok_or("window has no session")?;
        let base = self.cfg.base_index;
        let s = self.sessions.get(&dst).ok_or("no such session")?;
        let index = match index {
            Some(i) if s.windows.get(&i).is_some_and(|w| *w != wid) => {
                return Err(format!("index {i} in use"));
            }
            Some(i) => i,
            None => (base..)
                .find(|i| !s.windows.contains_key(i))
                .unwrap_or(base),
        };
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
            self.unlink_window(wid);
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

    /// Exchange two windows' places, which may be in different sessions.
    /// Unless `keep`, the window moved into `b`'s place becomes current there.
    pub fn swap_windows(&mut self, a: WindowId, b: WindowId, keep: bool) -> Result<(), String> {
        if a == b {
            return Ok(());
        }
        let place = |srv: &Self, w: WindowId| -> Result<(SessionId, u32), String> {
            let s = srv.session_of_window(w).ok_or("window has no session")?;
            let i = srv.sessions[&s]
                .index_of(w)
                .ok_or("window not in session")?;
            Ok((s, i))
        };
        let (sa, ia) = place(self, a)?;
        let (sb, ib) = place(self, b)?;
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
    pub fn join_pane(
        &mut self,
        src: PaneId,
        dst: PaneId,
        horizontal: bool,
        before: bool,
        size: Option<SplitSize>,
        focus: bool,
    ) -> Result<(), String> {
        if src == dst {
            return Err("source and target panes must be different".into());
        }
        let from = self.panes.get(&src).ok_or("no such pane")?.window;
        let to = self.panes.get(&dst).ok_or("no such pane")?.window;
        let r = self.panes[&dst].rect;
        if (if horizontal { r.w } else { r.h }) < 3 {
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
        if let Err(e) = self.insert_leaf(dst, src, horizontal, before, size) {
            // The pane is out of its old window already: give it a window of
            // its own rather than lose it.
            let session = self.session_of_window(to).ok_or("window has no session")?;
            let (cols, rows) = (self.windows[&to].cols, self.windows[&to].rows);
            let name = self.commands.get(&src).cloned().unwrap_or_default();
            let new = self.adopt_pane(session, src, name, cols, rows, false)?;
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
        if !keep && let Some(w) = self.windows.get_mut(&wd) {
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
