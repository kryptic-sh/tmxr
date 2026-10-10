//! Starting, splitting and closing panes, and their PTY events.

use std::path::PathBuf;
use std::sync::Arc;

use hjkl_layout::{LayoutTree, SplitDir};
use tmxr_term::{Emulator, Pty, PtyEvent, SpawnSpec};
use tracing::debug;

use super::{Event, Server, SplitSize};
use crate::model::{Pane, PaneId, SessionId, WindowId};

/// A program [`Server::start_pty`] started.
pub struct Started {
    pub pty: Pty,
    pub output: crate::output::OutputHandle,
    /// Which start this is, to tell its events from a previous one's.
    pub spawn: u64,
    /// The argv actually run (the shell filled in).
    pub argv: Vec<String>,
    /// The directory it started in.
    pub cwd: PathBuf,
}

impl Server {
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_pane(
        &mut self,
        pid: PaneId,
        window: WindowId,
        session: SessionId,
        argv: &[String],
        cwd: PathBuf,
        cols: u16,
        rows: u16,
    ) -> Result<(), String> {
        let started = self.start_pty(pid, session, argv, cwd, &[], cols, rows)?;
        self.panes.insert(
            pid,
            Pane {
                id: pid,
                window,
                pty: started.pty,
                emu: Emulator::new(rows.max(1), cols.max(1), self.cfg.history_limit),
                rect: hjkl_layout::LayoutRect::new(0, 0, cols, rows),
                start_cwd: started.cwd,
                copy: None,
                spawn: started.spawn,
                argv: started.argv,
                dead: None,
                clock: false,
                output: started.output,
                pipe: None,
            },
        );
        Ok(())
    }

    /// Start `argv` (the default shell when empty, a shell command line when
    /// one word with spaces) on a PTY whose events arrive as pane `pid`'s,
    /// in `session`'s environment plus `extra_env`.
    #[allow(clippy::too_many_arguments)]
    pub fn start_pty(
        &mut self,
        pid: PaneId,
        session: SessionId,
        argv: &[String],
        cwd: PathBuf,
        extra_env: &[(String, String)],
        cols: u16,
        rows: u16,
    ) -> Result<Started, String> {
        // The server's environment, then the global changes, then the
        // session's, as tmux layers them.
        let layered = self
            .sessions
            .get(&session)
            .map_or_else(|| self.global_env.clone(), |s| s.env.over(&self.global_env));
        let mut env: Vec<(String, String)> = Vec::new();
        let mut env_remove = Vec::new();
        for (k, v) in layered.iter() {
            match v {
                Some(v) => env.push((k.to_owned(), v.to_owned())),
                None => env_remove.push(k.to_owned()),
            }
        }
        env.push((
            "TERM".into(),
            crate::util::pane_term(&self.cfg.default_terminal),
        ));
        env.push(("COLORTERM".into(), "truecolor".into()));
        env.push((
            tmxr_proto::socket::ENV_TMXR.into(),
            tmxr_proto::socket::format_tmxr_env(&self.endpoint, self.pid, session),
        ));
        env.push((tmxr_proto::socket::ENV_TMXR_PANE.into(), format!("%{pid}")));
        env.extend(extra_env.iter().cloned());
        let argv = if argv.is_empty() {
            self.cfg
                .default_shell
                .clone()
                .map(|s| vec![s])
                .unwrap_or_default()
        } else if argv.len() == 1 && argv[0].contains(' ') {
            crate::util::shell_command(&argv[0], self.cfg.default_shell.as_deref())
        } else {
            argv.to_vec()
        };
        let cwd = if cwd.is_dir() {
            cwd
        } else {
            crate::util::home_dir()
        };
        let spec = SpawnSpec {
            argv: argv.clone(),
            cwd: Some(cwd.clone()),
            env,
            env_remove,
            rows,
            cols,
        };
        let events = self.events.clone();
        let spawn = self.next_spawn;
        self.next_spawn += 1;
        let output = crate::output::OutputHandle::default();
        let queue = Arc::clone(&output.0);
        let sink: Arc<dyn Fn(PtyEvent) + Send + Sync> = Arc::new(move |e| {
            let ev = match e {
                PtyEvent::Output(bytes) if queue.push(&bytes) => Event::PtyOutput(pid, spawn),
                PtyEvent::Output(_) => return,
                other => Event::Pty(pid, spawn, other),
            };
            let _ = events.send(ev);
        });
        let pty = Pty::spawn(&spec, sink).map_err(|e| format!("could not start pane: {e}"))?;
        self.next_pane = self.next_pane.max(pid + 1);
        Ok(Started {
            pty,
            output,
            spawn,
            argv,
            cwd,
        })
    }

    /// `respawn-pane -k`: end the pane's program and start `argv` (else the
    /// command it was started with) in its place, keeping its id and cell.
    pub fn respawn_pane(
        &mut self,
        pid: PaneId,
        argv: Vec<String>,
        cwd: Option<PathBuf>,
    ) -> Result<(), String> {
        let p = self.panes.get(&pid).ok_or("no such pane")?;
        let (wid, rect) = (p.window, p.rect);
        let argv = if argv.is_empty() {
            p.argv.clone()
        } else {
            argv
        };
        let cwd = cwd.unwrap_or_else(|| p.start_cwd.clone());
        let session = self.session_of_window(wid).ok_or("window has no session")?;
        if let Some(mut old) = self.panes.remove(&pid) {
            let _ = old.pty.kill();
        }
        self.commands.remove(&pid);
        if let Err(e) = self.spawn_pane(pid, wid, session, &argv, cwd, rect.w, rect.h) {
            // The old program is gone: take its cell out of the layout too.
            self.remove_pane_from_window(wid, pid);
            return Err(e);
        }
        self.relayout(wid);
        Ok(())
    }

    /// Split `target` and start a new pane. `horizontal` is tmux's `-h`
    /// (side by side); `before` is `-b`; `size` is the new pane's cells.
    #[allow(clippy::too_many_arguments)]
    pub fn split(
        &mut self,
        target: PaneId,
        horizontal: bool,
        before: bool,
        size: Option<SplitSize>,
        cwd: PathBuf,
        argv: Vec<String>,
        focus: bool,
    ) -> Result<PaneId, String> {
        let wid = self.panes.get(&target).ok_or("no such pane")?.window;
        let session = self.session_of_window(wid).ok_or("window has no session")?;
        let new = self.next_pane;
        self.insert_leaf(target, new, horizontal, before, size)?;
        let win = self.windows.get_mut(&wid).ok_or("no such window")?;
        let rects = crate::layout::pane_rects(&win.layout, win.cols, win.rows);
        let r = rects
            .iter()
            .find(|(p, _)| *p == new)
            .map(|(_, r)| *r)
            .unwrap_or_default();
        if let Err(e) = self.spawn_pane(new, wid, session, &argv, cwd, r.w, r.h) {
            if let Some(win) = self.windows.get_mut(&wid) {
                let _ = win.layout.remove_leaf(new as usize);
            }
            return Err(e);
        }
        if focus {
            self.select_pane(new);
        }
        self.relayout(wid);
        Ok(new)
    }

    /// Split `target`'s cell in its window's layout and put `new` in the new
    /// half (the layout half of `split-window`, shared with `join-pane`).
    pub fn insert_leaf(
        &mut self,
        target: PaneId,
        new: PaneId,
        horizontal: bool,
        before: bool,
        size: Option<SplitSize>,
    ) -> Result<(), String> {
        let wid = self.panes.get(&target).ok_or("no such pane")?.window;
        let win = self.windows.get_mut(&wid).ok_or("no such window")?;
        win.zoomed = false;
        let dir = if horizontal {
            SplitDir::Vertical
        } else {
            SplitDir::Horizontal
        };
        let target_rect = self.panes.get(&target).map(|p| p.rect).unwrap_or_default();
        let len = if horizontal {
            target_rect.w
        } else {
            target_rect.h
        };
        if len < 3 {
            return Err("pane too small".into());
        }
        // Fraction of the split given to the *first* child.
        let new_cells = match size {
            Some(SplitSize::Cells(n)) => f32::from(n.min(len - 2)),
            Some(SplitSize::Percent(p)) => f32::from(len) * f32::from(p.min(100)) / 100.0,
            None => f32::from(len) / 2.0,
        };
        let new_frac = (new_cells / f32::from(len)).clamp(0.05, 0.95);
        let ratio = if before { new_frac } else { 1.0 - new_frac };
        let t = target as usize;
        let n = new as usize;
        win.layout.replace_leaf(t, move |id| {
            let (a, b) = if before {
                (LayoutTree::Leaf(n), LayoutTree::Leaf(id))
            } else {
                (LayoutTree::Leaf(id), LayoutTree::Leaf(n))
            };
            LayoutTree::split(dir, ratio, a, b)
        });
        Ok(())
    }

    pub fn select_pane(&mut self, pane: PaneId) {
        let Some(wid) = self.panes.get(&pane).map(|p| p.window) else {
            return;
        };
        if let Some(win) = self.windows.get_mut(&wid)
            && win.active != pane
        {
            win.last_pane = Some(win.active);
            win.active = pane;
            if win.zoomed {
                win.zoomed = false;
                self.relayout(wid);
            }
        }
        self.mark_window_dirty(wid);
    }

    pub fn kill_pane(&mut self, pane: PaneId) {
        if let Some(mut p) = self.panes.remove(&pane) {
            let _ = p.pty.kill();
            self.commands.remove(&pane);
            self.remove_pane_from_window(p.window, pane);
        }
    }

    pub(crate) fn remove_pane_from_window(&mut self, wid: WindowId, pane: PaneId) {
        let Some(win) = self.windows.get_mut(&wid) else {
            return;
        };
        match win.layout.remove_leaf(pane as usize) {
            Ok(focus) => {
                if win.active == pane {
                    win.active = win
                        .last_pane
                        .filter(|l| *l != pane && win.layout.contains(*l as usize))
                        .unwrap_or(focus as PaneId);
                }
                if win.last_pane == Some(pane) {
                    win.last_pane = None;
                }
                win.zoomed = false;
                self.relayout(wid);
            }
            Err(_) => self.kill_window(wid),
        }
    }

    pub(super) fn pty_event(&mut self, pid: PaneId, ev: PtyEvent) {
        match ev {
            PtyEvent::Output(bytes) => {
                let Some(p) = self.panes.get_mut(&pid) else {
                    return;
                };
                let replies = p.emu.process(&bytes);
                if !replies.is_empty() {
                    let _ = p.pty.write(&replies);
                }
                if let Some(pipe) = &p.pipe {
                    pipe.send(&bytes);
                }
                let bell = p.emu.take_bell();
                let clips = p.emu.take_clipboard();
                let passthrough = p.emu.take_passthrough();
                let wid = p.window;
                if bell && let Some(w) = self.windows.get_mut(&wid) {
                    w.bell = true;
                }
                let watched = self
                    .windows
                    .get(&wid)
                    .is_some_and(|w| w.monitor_activity.unwrap_or(self.cfg.monitor_activity));
                let seen = self.window_is_current(wid);
                if let Some(w) = self.windows.get_mut(&wid) {
                    w.last_output = std::time::Instant::now();
                    w.silence = false;
                    if watched && !seen {
                        w.activity = true;
                    }
                }
                for (_, data) in clips {
                    self.forward_osc52(&data);
                }
                // tmux drops passthrough unless it is allowed.
                if self.cfg.allow_passthrough {
                    for data in passthrough {
                        self.forward_passthrough(pid, &data);
                    }
                }
                self.mark_window_dirty(wid);
            }
            PtyEvent::Exited(code) => {
                debug!(pane = pid, ?code, "pane exited");
                let ctx = crate::cmds::Ctx {
                    pane: Some(pid),
                    ..crate::cmds::Ctx::default()
                };
                self.queue_hook("pane-exited", ctx, None);
                if self.cfg.remain_on_exit
                    && let Some(p) = self.panes.get_mut(&pid)
                {
                    p.dead = Some(code);
                    let window = p.window;
                    self.commands.remove(&pid);
                    self.mark_window_dirty(window);
                } else if self.panes.contains_key(&pid) {
                    self.panes.remove(&pid);
                    self.commands.remove(&pid);
                    let wid = self
                        .windows
                        .values()
                        .find(|w| w.layout.contains(pid as usize))
                        .map(|w| w.id);
                    if let Some(w) = wid {
                        self.remove_pane_from_window(w, pid);
                    }
                }
            }
            PtyEvent::Eof => {}
        }
    }
}
