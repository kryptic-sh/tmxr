//! Pane commands, including the vim/hjkl navigator.

use std::fmt::Write as _;

use crossterm::event::KeyEvent;
use tmxr_command::{Args, Key, Parsed};

use super::{Ctx, Outcome, Res, attached_client, cwd_arg, dir_flag};
use crate::model::PaneId;
use crate::overlay::Overlay;
use crate::server::{Server, SplitSize};
use crate::target;

pub(super) fn run(
    srv: &mut Server,
    ctx: &Ctx,
    p: &Parsed,
    out: &mut Outcome,
) -> Result<bool, String> {
    let a = &p.args;
    let pos = a.positional();
    match p.name() {
        "split-window" => {
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            let size = split_size(a)?;
            let cwd = cwd_arg(srv, ctx, Some(pid), a)
                .or_else(|| srv.panes.get(&pid).map(|p| p.start_cwd.clone()))
                .unwrap_or_else(crate::util::home_dir);
            srv.split(
                pid,
                a.has('h'),
                a.has('b'),
                size,
                cwd,
                pos.to_vec(),
                !a.has('d'),
            )?;
        }
        "kill-pane" => {
            let (_, wid, pid) = target::pane(srv, ctx, a.value('t'))?;
            if a.has('a') {
                let others: Vec<PaneId> = srv.windows[&wid]
                    .panes()
                    .into_iter()
                    .filter(|p| *p != pid)
                    .collect();
                for p in others {
                    srv.kill_pane(p);
                }
            } else {
                srv.kill_pane(pid);
            }
        }
        "select-pane" if a.has('m') || a.has('M') => {
            // -m toggles the mark on the pane, -M clears it.
            let pid = target::pane(srv, ctx, a.value('t'))?.2;
            let old = srv.marked_pane();
            srv.marked = (a.has('m') && old != Some(pid)).then_some(pid);
            for p in [old, srv.marked].into_iter().flatten() {
                let w = srv.panes[&p].window;
                srv.mark_window_dirty(w);
            }
        }
        "select-pane" => {
            let (_, wid, pid) = target::pane(srv, ctx, a.value('t'))?;
            let to = if a.has('l') {
                srv.windows[&wid].last_pane.ok_or("no last pane")?
            } else if let Some(d) = dir_flag(a) {
                let w = &srv.windows[&wid];
                let rects = crate::layout::pane_rects(&w.layout, w.cols, w.rows);
                match crate::layout::neighbour(&rects, pid, w.last_pane, d, w.cols, w.rows) {
                    Some(n) => n,
                    None => return Ok(true),
                }
            } else {
                pid
            };
            let zoomed = srv.windows[&wid].zoomed;
            srv.select_pane(to);
            // -Z: a zoomed window stays zoomed, now on the selected pane.
            if a.has('Z') && zoomed {
                if let Some(w) = srv.windows.get_mut(&wid) {
                    w.zoomed = true;
                }
                srv.relayout(wid);
            }
        }
        "last-pane" => {
            let (_, wid, _) = target::window(srv, ctx, a.value('t'))?;
            let last = srv.windows[&wid].last_pane.ok_or("no last pane")?;
            srv.select_pane(last);
        }
        "navigate-pane" => navigate(srv, ctx, a)?,
        "resize-pane" => {
            if a.has('M') {
                return crate::mouse::resize_drag(srv, ctx).map(|()| true);
            }
            let (_, wid, pid) = target::pane(srv, ctx, a.value('t'))?;
            if a.has('Z') {
                let w = srv.windows.get_mut(&wid).ok_or("no window")?;
                if w.panes().len() > 1 || w.zoomed {
                    w.active = pid;
                    w.zoomed = !w.zoomed;
                }
                srv.relayout(wid);
                return Ok(true);
            }
            let cells: u16 = pos
                .first()
                .map_or(Ok(1), |n| n.parse())
                .map_err(|_| "bad adjustment")?;
            if let Some(d) = dir_flag(a) {
                let w = srv.windows.get_mut(&wid).ok_or("no window")?;
                w.zoomed = false;
                let (cols, rows) = (w.cols, w.rows);
                crate::layout::resize(&mut w.layout, pid, d, cells, cols, rows);
                srv.relayout(wid);
            }
        }
        "swap-pane" => {
            let (_, wid, pid) = target::pane(srv, ctx, a.value('t'))?;
            let marked = srv.marked_pane().filter(|_| !a.has('U') && !a.has('D'));
            let (src, dst) = match (a.value('s'), marked) {
                (Some(s), _) => (target::pane(srv, ctx, Some(s))?.2, pid),
                (None, Some(m)) => (m, pid),
                (None, None) => {
                    // -U / -D: swap with the previous / next pane.
                    let panes = srv.windows.get(&wid).ok_or("no window")?.panes();
                    let i = panes.iter().position(|p| *p == pid).unwrap_or(0);
                    let n = panes.len();
                    let other = if a.has('U') {
                        panes[(i + n - 1) % n]
                    } else {
                        panes[(i + 1) % n]
                    };
                    (pid, other)
                }
            };
            srv.swap_panes(src, dst, a.has('d'))?;
        }
        "respawn-pane" => {
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            // A running program is only replaced with -k; a dead pane (kept
            // by remain-on-exit) is respawned as is.
            if !a.has('k') && srv.panes[&pid].dead.is_none() {
                return Err(format!("respawn pane failed: pane %{pid} still active"));
            }
            let cwd = cwd_arg(srv, ctx, Some(pid), a);
            srv.respawn_pane(pid, pos.to_vec(), cwd)?;
        }
        // tmux's move-pane is join-pane under another name.
        "join-pane" | "move-pane" => {
            let src = match (a.value('s'), srv.marked_pane()) {
                (Some(s), _) => target::pane(srv, ctx, Some(s))?.2,
                (None, Some(m)) => m,
                (None, None) => return Err("join-pane needs -s or a marked pane".into()),
            };
            let dst = target::pane(srv, ctx, a.value('t'))?.2;
            let size = split_size(a)?;
            srv.join_pane(src, dst, a.has('h'), a.has('b'), size, !a.has('d'))?;
        }
        "rotate-window" => {
            let (_, wid, _) = target::window(srv, ctx, a.value('t'))?;
            let w = srv.windows.get_mut(&wid).ok_or("no window")?;
            // tmux rotates up unless -D.
            w.layout = crate::layout::rotate(&w.layout, !a.has('D'));
            srv.relayout(wid);
        }
        "break-pane" => {
            let (sid, wid, pid) = target::pane(srv, ctx, a.value('t'))?;
            if srv.windows[&wid].panes().len() < 2 {
                return Err("can't break with only one pane".into());
            }
            let (cols, rows) = (srv.windows[&wid].cols, srv.windows[&wid].rows);
            {
                let w = srv.windows.get_mut(&wid).ok_or("no window")?;
                if let Ok(focus) = w.layout.remove_leaf(pid as usize) {
                    if w.active == pid {
                        w.active = focus as PaneId;
                    }
                    w.zoomed = false;
                }
            }
            srv.relayout(wid);
            let name = srv.commands.get(&pid).cloned().unwrap_or_default();
            let new = srv.adopt_pane(sid, pid, name, cols, rows, !a.has('d'))?;
            srv.relayout(new);
        }
        "list-panes" => {
            let (_, _, wid) = target::window(srv, ctx, a.value('t'))?;
            let w = &srv.windows[&wid];
            for (i, p) in w.panes().iter().enumerate() {
                let pane = &srv.panes[p];
                let _ = writeln!(
                    out.stdout,
                    "{}: [{}x{}] %{}{}",
                    i as u32 + srv.cfg.pane_base_index,
                    pane.rect.w,
                    pane.rect.h,
                    p,
                    if *p == w.active { " (active)" } else { "" }
                );
            }
        }
        "display-panes" => {
            let (_, _, wid) = target::window(srv, ctx, None)?;
            let labels: Vec<(u32, PaneId)> = srv.windows[&wid]
                .panes()
                .into_iter()
                .enumerate()
                .map(|(i, p)| (i as u32 + srv.cfg.pane_base_index, p))
                .collect();
            match attached_client(srv, ctx) {
                // On a client: numbers over the panes; a number selects.
                Some(c) => {
                    let time = std::time::Duration::from_millis(srv.cfg.display_panes_time);
                    if let Some(a) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                        a.overlay = Some(Overlay::display_panes(labels, time));
                        a.dirty = true;
                    }
                }
                // For a script: the same mapping as text.
                None => {
                    let list: Vec<String> =
                        labels.iter().map(|(i, p)| format!("{i}=%{p}")).collect();
                    let _ = writeln!(out.stdout, "panes: {}", list.join(" "));
                }
            }
        }
        "clock-mode" => {
            let (_, wid, pid) = target::pane(srv, ctx, a.value('t'))?;
            if let Some(p) = srv.panes.get_mut(&pid) {
                p.clock = true;
            }
            srv.mark_window_dirty(wid);
        }
        "capture-pane" => {
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            let text = srv.panes[&pid].emu.screen().contents();
            if a.has('p') {
                out.stdout.push_str(&text);
                if !text.ends_with('\n') {
                    out.stdout.push('\n');
                }
            } else {
                srv.add_buffer(text, None);
            }
        }
        "send-keys" => {
            if a.has('M') {
                return crate::mouse::forward(srv, ctx).map(|()| true);
            }
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            if a.has('X') {
                let (name, rest) = pos.split_first().ok_or("send-keys -X needs a command")?;
                // -N: repeat count, as typed digits would give.
                if let Some(n) = a.value('N') {
                    let n: usize = n.parse().map_err(|_| format!("bad count: {n}"))?;
                    if let Some(cm) = srv.panes.get_mut(&pid).and_then(|p| p.copy.as_mut()) {
                        cm.count = n.min(crate::copy::MAX_COUNT);
                    }
                }
                if crate::mouse::copy_command_at_mouse(srv, ctx, pid, name) {
                    return Ok(true);
                }
                return crate::copy::command(srv, ctx, pid, name, rest).map(|()| true);
            }
            for word in pos {
                match word.parse::<Key>() {
                    Ok(k) if !a.has('l') && word.chars().count() > 1 => {
                        srv.send_key_to_pane(pid, &KeyEvent::new(k.code, k.mods));
                    }
                    _ => {
                        if let Some(p) = srv.panes.get_mut(&pid) {
                            let _ = p.pty.write(word.as_bytes());
                        }
                    }
                }
            }
        }
        "send-prefix" => {
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            let k = srv.prefix;
            srv.send_key_to_pane(pid, &KeyEvent::new(k.code, k.mods));
        }

        _ => return Ok(false),
    }
    Ok(true)
}

/// `-l size` of `split-window` / `join-pane`: cells, or `N%` of the pane.
fn split_size(a: &Args) -> Result<Option<SplitSize>, String> {
    let Some(l) = a.value('l') else {
        return Ok(None);
    };
    let bad = || format!("bad size: {l}");
    Ok(Some(match l.strip_suffix('%') {
        Some(pc) => SplitSize::Percent(pc.parse().map_err(|_| bad())?),
        None => SplitSize::Cells(l.parse().map_err(|_| bad())?),
    }))
}

/// vim-tmux-navigator: pass the key to vim/hjkl/fzf in front, else move.
fn navigate(srv: &mut Server, ctx: &Ctx, a: &Args) -> Res {
    let (_, wid, pid) = target::pane(srv, ctx, a.value('t'))?;
    let zoomed = srv.windows.get(&wid).is_some_and(|w| w.zoomed);
    if let Some(key) = ctx.key {
        let cmd = srv
            .panes
            .get(&pid)
            .and_then(|p| tmxr_term::process::foreground_command(&p.pty))
            .unwrap_or_default();
        let is_editor = srv.navigator.as_ref().is_some_and(|re| re.is_match(&cmd));
        if is_editor || (zoomed && srv.cfg.navigator.disable_when_zoomed) {
            srv.send_key_to_pane(pid, &key);
            return Ok(());
        }
    }
    let to = if a.has('l') {
        match srv.windows[&wid].last_pane {
            Some(l) => l,
            None => return Ok(()),
        }
    } else if let Some(d) = dir_flag(a) {
        let w = &srv.windows[&wid];
        let rects = crate::layout::pane_rects(&w.layout, w.cols, w.rows);
        match crate::layout::neighbour(&rects, pid, w.last_pane, d, w.cols, w.rows) {
            Some(n) => n,
            None => return Ok(()),
        }
    } else {
        return Ok(());
    };
    srv.select_pane(to);
    Ok(())
}
