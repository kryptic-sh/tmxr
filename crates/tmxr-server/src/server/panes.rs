//! Starting, splitting and closing panes, and their PTY events.

use std::path::PathBuf;
use std::sync::Arc;

use hjkl_layout::{LayoutTree, SplitDir};
use tmxr_term::{Emulator, Pty, PtyEvent, SpawnSpec};
use tracing::debug;

use super::{ChildEnv, Event, Server, SplitSize};
use crate::model::{Pane, PaneId, SessionId, WindowId};

/// A program [`Server::start_pty`] started.
pub struct Started {
    pub pty: Pty,
    pub output: crate::output::OutputHandle,
    /// Which start this is, to tell its events from a previous one's.
    pub spawn: u64,
    /// The directory it started in.
    pub cwd: PathBuf,
}

impl Server {
    /// What a new pane runs: `argv`, else `session`'s `default-command`,
    /// else the shell, as tmux's `spawn_pane`. A respawned pane keeps its own
    /// command and does not come here.
    pub fn new_pane_argv(&self, session: SessionId, argv: Vec<String>) -> Vec<String> {
        if !argv.is_empty() {
            return argv;
        }
        let command = self.session_options(session).default_command.clone();
        if command.is_empty() {
            Vec::new()
        } else {
            vec![command]
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_pane(
        &mut self,
        pid: PaneId,
        window: WindowId,
        session: SessionId,
        argv: &[String],
        cwd: PathBuf,
        env: &[(String, String)],
        cols: u16,
        rows: u16,
    ) -> Result<(), String> {
        let started = self.start_pty(pid, session, argv, cwd, env, cols, rows)?;
        let history_limit = self.session_options(session).history_limit;
        self.panes.insert(
            pid,
            Pane {
                id: pid,
                window,
                pty: started.pty,
                emu: Emulator::new(rows.max(1), cols.max(1), history_limit),
                rect: hjkl_layout::LayoutRect::new(0, 0, cols, rows),
                start_cwd: started.cwd,
                copy: None,
                spawn: started.spawn,
                argv: argv.to_vec(),
                dead: None,
                dead_time: None,
                clock: false,
                input_off: false,
                focused: false,
                output: started.output,
                pipe: None,
                hooks: crate::hooks::HookTable::new(),
                options: tmxr_config::OwnOptions::default(),
            },
        );
        Ok(())
    }

    /// The environment a child of the server starts in for `session`, as
    /// tmux's `environ_for_session`: the server's own with the global
    /// changes and the session's over it, `TERM` from `default-terminal`,
    /// and `TMXR` naming this server and the session.
    pub fn child_env(&self, session: Option<SessionId>) -> ChildEnv {
        let layered = session
            .and_then(|s| self.sessions.get(&s))
            .map_or_else(|| self.global_env.clone(), |s| s.env.over(&self.global_env));
        let mut env = ChildEnv::default();
        for (k, v) in layered.iter() {
            match v {
                Some(v) => env.set.push((k.to_owned(), v.to_owned())),
                None => env.remove.push(k.to_owned()),
            }
        }
        env.set.push((
            "TERM".into(),
            crate::util::pane_term(&self.cfg.default_terminal),
        ));
        env.set.push((
            tmxr_proto::socket::ENV_TMXR.into(),
            tmxr_proto::socket::format_tmxr_env(&self.endpoint, self.pid, session),
        ));
        env
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
        let ChildEnv {
            set: mut env,
            remove: env_remove,
        } = self.child_env(Some(session));
        env.push(("COLORTERM".into(), "truecolor".into()));
        env.push((tmxr_proto::socket::ENV_TMXR_PANE.into(), format!("%{pid}")));
        env.extend(extra_env.iter().cloned());
        let shell = self
            .session_options(session)
            .default_shell
            .clone()
            .filter(|s| !s.is_empty());
        let argv = if argv.is_empty() {
            shell.map(|s| vec![s]).unwrap_or_default()
        } else if argv.len() == 1 && argv[0].contains(' ') {
            crate::util::shell_command(&argv[0], shell.as_deref())
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
            cwd,
        })
    }

    /// `respawn-pane -k`: end the pane's program and start `argv` (else the
    /// command it was started with) in its place, keeping its id and cell;
    /// `env` is added to its environment (`-e`).
    pub fn respawn_pane(
        &mut self,
        pid: PaneId,
        argv: Vec<String>,
        cwd: Option<PathBuf>,
        env: &[(String, String)],
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
        // The pane's hooks belong to the pane, not its program.
        let mut hooks = crate::hooks::HookTable::new();
        if let Some(mut old) = self.panes.remove(&pid) {
            let _ = old.pty.kill();
            hooks = std::mem::take(&mut old.hooks);
        }
        self.commands.remove(&pid);
        if let Err(e) = self.spawn_pane(pid, wid, session, &argv, cwd, env, rect.w, rect.h) {
            // The old program is gone: take its cell out of the layout too.
            self.remove_pane_from_window(wid, pid);
            return Err(e);
        }
        if let Some(p) = self.panes.get_mut(&pid) {
            p.hooks = hooks;
        }
        self.relayout(wid);
        Ok(())
    }

    /// Split `target` and start a new pane. `horizontal` is tmux's `-h`
    /// (side by side); `before` is `-b`; `full` is `-f` (the whole window
    /// splits, not the target); `size` is the new pane's cells; `env` is
    /// added to its environment (`-e`).
    #[allow(clippy::too_many_arguments)]
    pub fn split(
        &mut self,
        target: PaneId,
        horizontal: bool,
        before: bool,
        full: bool,
        size: Option<SplitSize>,
        cwd: PathBuf,
        argv: Vec<String>,
        env: &[(String, String)],
        focus: bool,
    ) -> Result<PaneId, String> {
        let wid = self.panes.get(&target).ok_or("no such pane")?.window;
        let session = self.session_of_window(wid).ok_or("window has no session")?;
        let argv = self.new_pane_argv(session, argv);
        let new = self.next_pane;
        self.insert_leaf(target, new, horizontal, before, full, size)?;
        let win = self.windows.get_mut(&wid).ok_or("no such window")?;
        let rects = crate::layout::pane_rects(&win.layout, win.cols, win.rows);
        let r = rects
            .iter()
            .find(|(p, _)| *p == new)
            .map(|(_, r)| *r)
            .unwrap_or_default();
        if let Err(e) = self.spawn_pane(new, wid, session, &argv, cwd, env, r.w, r.h) {
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
    ///
    /// With `full` (tmux's `-f`, `SPAWN_FULLSIZE`) the whole window splits:
    /// the new pane spans it, sized against its width or height, and goes
    /// first or last in the pane list.
    pub fn insert_leaf(
        &mut self,
        target: PaneId,
        new: PaneId,
        horizontal: bool,
        before: bool,
        full: bool,
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
        let len = match (full, horizontal) {
            (true, true) => win.cols,
            (true, false) => win.rows,
            (false, true) => target_rect.w,
            (false, false) => target_rect.h,
        };
        if len < 3 {
            return Err("pane too small".into());
        }
        // Cells for each side as tmux 3.6's layout_split_pane: `second` is
        // the bottom / right pane, the border one cell between them. With no
        // size the new pane gets the smaller half.
        let asked = match size {
            Some(SplitSize::Cells(n)) => Some(n),
            Some(SplitSize::Percent(p)) => {
                Some(u16::try_from(u32::from(len) * u32::from(p.min(100)) / 100).unwrap_or(len))
            }
            None => None,
        };
        let second = match asked {
            None => len.div_ceil(2) - 1,
            Some(n) if before => len.saturating_sub(n).saturating_sub(1),
            Some(n) => n,
        }
        .clamp(1, len - 2);
        let first = len - 1 - second;
        // hjkl-layout gives the first child round(len × ratio) cells, its
        // border among them.
        let ratio = f32::from(first + 1) / f32::from(len);
        let t = target as usize;
        let n = new as usize;
        let pair = move |old: LayoutTree| {
            let (a, b) = if before {
                (LayoutTree::Leaf(n), old)
            } else {
                (old, LayoutTree::Leaf(n))
            };
            LayoutTree::split(dir, ratio, a, b)
        };
        if full {
            let root = std::mem::replace(&mut win.layout, LayoutTree::Leaf(n));
            win.layout = pair(root);
            // tmux's pane list: first with -b, else last.
            if !win.order.is_empty() {
                if before {
                    win.order.insert(0, new);
                } else {
                    win.order.push(new);
                }
            }
        } else {
            win.layout
                .replace_leaf(t, move |id| pair(LayoutTree::Leaf(id)));
            // tmux's pane list: the new pane after its target, before with -b.
            if let Some(i) = win.order.iter().position(|p| *p == target) {
                win.order.insert(if before { i } else { i + 1 }, new);
            }
        }
        Ok(())
    }

    /// Unzoom `wid` for a command that rearranges it, as tmux's
    /// `window_push_zoom`. Returns whether to zoom it again afterwards
    /// ([`Server::pop_zoom`]): with `keep` (`-Z`), when it was zoomed, or
    /// with `always` whether it was or not.
    pub fn push_zoom(&mut self, wid: WindowId, always: bool, keep: bool) -> bool {
        let Some(win) = self.windows.get_mut(&wid) else {
            return false;
        };
        let again = keep && (always || win.zoomed);
        if win.zoomed {
            win.zoomed = false;
            self.relayout(wid);
        }
        again
    }

    /// Zoom `wid`'s active pane again after [`Server::push_zoom`] said to,
    /// as tmux's `window_pop_zoom` (a window of one pane does not zoom).
    pub fn pop_zoom(&mut self, wid: WindowId, again: bool) {
        if let Some(win) = self.windows.get_mut(&wid)
            && again
            && win.panes().len() > 1
        {
            win.zoomed = true;
            self.relayout(wid);
        }
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

    /// A program's `ESC k name ESC \`, as tmux's `input_exit_rename`: a
    /// name renames the window and stops automatic renaming; an empty one
    /// gives automatic-rename back to the inherited value, and empties the
    /// name if that is off.
    fn program_rename(&mut self, wid: WindowId, name: &[u8]) {
        let Ok(name) = std::str::from_utf8(name) else {
            return;
        };
        let Some(win) = self.windows.get_mut(&wid) else {
            return;
        };
        if name.is_empty() {
            win.options.automatic_rename = None;
            if !self.window_options(wid).automatic_rename
                && let Some(win) = self.windows.get_mut(&wid)
            {
                win.name.clear();
            }
        } else {
            win.options.automatic_rename = Some(false);
            name.clone_into(&mut win.name);
        }
        self.mark_window_dirty(wid);
    }

    pub(super) fn pty_event(&mut self, pid: PaneId, ev: PtyEvent) {
        match ev {
            PtyEvent::Output(bytes) => {
                let allow_title = self.pane_options(pid).allow_set_title;
                let Some(p) = self.panes.get_mut(&pid) else {
                    return;
                };
                let alternate = p.emu.screen().alternate_screen();
                let title = p.emu.title().map(str::to_owned);
                let replies = p.emu.process(&bytes);
                // allow-set-title off: the program's OSC 0 / 2 is ignored.
                if !allow_title && p.emu.title() != title.as_deref() {
                    match title {
                        Some(t) => p.emu.set_title(t),
                        None => p.emu.clear_title(),
                    }
                }
                if !replies.is_empty() {
                    let _ = p.pty.write(&replies);
                }
                // A scrollbar hides on the alternate screen, as tmux's.
                if p.emu.screen().alternate_screen() != alternate {
                    self.refit_scrollbar(pid);
                }
                self.copy_line_output(pid);
                let Some(p) = self.panes.get_mut(&pid) else {
                    return;
                };
                if let Some(pipe) = &p.pipe {
                    pipe.send(&bytes);
                }
                let bell = p.emu.take_bell();
                let clips = p.emu.take_clipboard();
                let passthrough = p.emu.take_passthrough();
                let rename = p.emu.take_rename();
                let wid = p.window;
                if let Some(name) = rename
                    && self.pane_options(pid).allow_rename
                {
                    self.program_rename(wid, &name);
                }
                if bell && self.window_options(wid).monitor_bell {
                    crate::alerts::raise(self, wid, crate::alerts::Alert::Bell);
                }
                if let Some(w) = self.windows.get_mut(&wid) {
                    w.last_output = std::time::Instant::now();
                    w.silence.clear();
                    w.silence_raised = false;
                }
                if self.window_options(wid).monitor_activity {
                    crate::alerts::raise(self, wid, crate::alerts::Alert::Activity);
                }
                for (_, data) in clips {
                    self.forward_osc52(&data);
                }
                // tmux drops passthrough unless it is allowed.
                if self.pane_options(pid).allow_passthrough {
                    for (data, cursor) in passthrough {
                        self.forward_passthrough(pid, &data, cursor);
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
                // A pane kept dead fires pane-died, one that goes
                // pane-exited, as tmux's.
                let remain = self.pane_options(pid).remain_on_exit.keeps(code);
                let hook = if remain { "pane-died" } else { "pane-exited" };
                self.queue_hook(hook, ctx, crate::hooks::HookScope::default());
                if remain && let Some(p) = self.panes.get_mut(&pid) {
                    p.dead = Some(code);
                    p.dead_time = Some(std::time::SystemTime::now());
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
