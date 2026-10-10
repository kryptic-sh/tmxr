//! Window commands.

use std::fmt::Write as _;

use tmxr_command::Parsed;

use super::{Ctx, Outcome, client_size, cwd_arg, dir_flag, list_item};
use crate::layout::Dir;
use crate::model::SessionId;
use crate::server::STATUS_ROWS;
use crate::server::Server;
use crate::target;
use crate::vars::Vars;

/// Largest `resize-window` size, tmux's `WINDOW_MAXIMUM`.
const WINDOW_MAXIMUM: u16 = 10_000;

pub(super) fn run(
    srv: &mut Server,
    ctx: &Ctx,
    p: &Parsed,
    out: &mut Outcome,
) -> Result<bool, String> {
    let a = &p.args;
    let pos = a.positional();
    match p.name() {
        "new-window" => {
            let (sid, cur_idx, _) = target::window(srv, ctx, None)?;
            let (sid, index) = match a.value('t') {
                Some(t) if !t.is_empty() => target::destination(srv, ctx, t)?,
                _ if a.has('a') => {
                    let s = &srv.sessions[&sid];
                    let idx = ((cur_idx + 1)..).find(|i| !s.windows.contains_key(i));
                    (sid, idx)
                }
                _ => (sid, None),
            };
            let pane = srv.active_pane_of_session(sid);
            let cwd = cwd_arg(srv, ctx, pane, a);
            let size = srv
                .sessions
                .get(&sid)
                .and_then(|s| s.current_window())
                .and_then(|w| srv.windows.get(&w))
                .map_or_else(|| client_size(srv, ctx), |w| (w.cols, w.rows));
            srv.new_window(
                sid,
                index,
                a.value('n').map(str::to_owned),
                cwd,
                pos.to_vec(),
                &super::env_flags(a)?,
                size,
                !a.has('d'),
            )?;
        }
        // -r: renumber the target session's windows; nothing moves.
        "move-window" if a.has('r') => {
            let sid = target::session(srv, ctx, a.value('t'))?;
            srv.renumber_windows(sid);
        }
        "move-window" | "link-window" => {
            let (src, _, wid) = target::window(srv, ctx, a.value('s'))?;
            let (dst, mut index) = match a.value('t') {
                Some(t) if !t.is_empty() => target::destination(srv, ctx, t)?,
                _ => (target::session(srv, ctx, None)?, None),
            };
            // -a: the first free index after the target.
            if a.has('a') {
                let s = &srv.sessions[&dst];
                let after = index.unwrap_or(s.current);
                index = ((after + 1)..).find(|i| !s.windows.contains_key(i));
            }
            // -k: a window already at the index makes room. It is taken out
            // of the map directly, not unlinked, so a session whose only
            // window it is does not end before the new one arrives.
            let replaced = match index {
                Some(i) if a.has('k') => srv
                    .sessions
                    .get_mut(&dst)
                    .and_then(|s| s.windows.remove_entry(&i))
                    .filter(|(_, old)| *old != wid),
                _ => None,
            };
            let placed = if p.name() == "link-window" {
                srv.link_window(wid, dst, index, !a.has('d'))
            } else {
                srv.move_window(wid, src, dst, index, !a.has('d'))
            };
            if let Some((i, old)) = replaced {
                if placed.is_err()
                    && let Some(s) = srv.sessions.get_mut(&dst)
                {
                    s.windows.insert(i, old);
                } else if srv.session_of_window(old).is_none() {
                    srv.kill_window(old);
                }
            }
            placed?;
        }
        "unlink-window" => {
            let (sid, _, wid) = target::window(srv, ctx, a.value('t'))?;
            if !a.has('k') && srv.sessions_of_window(wid).len() < 2 {
                return Err("window only linked to one session".into());
            }
            srv.unlink_or_kill(sid, wid);
        }
        "swap-window" => {
            let src = match (a.value('s'), srv.marked_pane()) {
                (Some(s), _) => {
                    let (sid, idx, _) = target::window(srv, ctx, Some(s))?;
                    (sid, idx)
                }
                (None, Some(m)) => {
                    let w = srv.panes[&m].window;
                    let sid = srv.session_of_window(w).ok_or("window has no session")?;
                    let idx = srv.sessions[&sid]
                        .index_of(w)
                        .ok_or("window not in session")?;
                    (sid, idx)
                }
                // tmux's CMD_FIND_DEFAULT_MARKED: the marked window, else the
                // current one.
                (None, None) => {
                    let (sid, idx, _) = target::window(srv, ctx, None)?;
                    (sid, idx)
                }
            };
            let (sid, idx, _) = target::window(srv, ctx, a.value('t'))?;
            srv.swap_windows(src, (sid, idx), a.has('d'))?;
        }
        "kill-window" => {
            let (sid, _, wid) = target::window(srv, ctx, a.value('t'))?;
            if a.has('a') {
                let others: Vec<u32> = srv.sessions[&sid]
                    .windows
                    .values()
                    .copied()
                    .filter(|w| *w != wid)
                    .collect();
                for w in others {
                    srv.kill_window(w);
                }
            } else {
                srv.kill_window(wid);
            }
        }
        "select-window" => {
            let (sid, idx, _) = target::window(srv, ctx, a.value('t'))?;
            let idx = if a.has('n') {
                target::window(srv, ctx, Some(&format!("{}:+", srv.sessions[&sid].name)))?.1
            } else if a.has('p') {
                target::window(srv, ctx, Some(&format!("{}:-", srv.sessions[&sid].name)))?.1
            } else if a.has('l') {
                srv.sessions[&sid].last.ok_or("no last window")?
            } else {
                idx
            };
            srv.select_window(sid, idx)?;
        }
        // -a: the next / previous window with an alert (a bell, activity or
        // silence), cycling.
        "next-window" | "previous-window" if a.has('a') => {
            let sid = target::session(srv, ctx, a.value('t'))?;
            let s = &srv.sessions[&sid];
            let alerted = |i: &&u32| {
                srv.windows
                    .get(&s.windows[*i])
                    .is_some_and(crate::model::Window::alerted)
            };
            let after = s.windows.keys().filter(|i| **i > s.current);
            let before = s.windows.keys().filter(|i| **i < s.current);
            let found = if p.name() == "next-window" {
                after.chain(before).find(alerted)
            } else {
                before.rev().chain(after.rev()).find(alerted)
            };
            let idx = *found.ok_or("no window with an alert")?;
            srv.select_window(sid, idx)?;
        }
        "next-window" | "previous-window" | "last-window" => {
            let sid = target::session(srv, ctx, a.value('t'))?;
            let spec = match p.name() {
                "next-window" => "+",
                "previous-window" => "-",
                _ => "!",
            };
            let name = srv.sessions[&sid].name.clone();
            let (_, idx, _) = target::window(srv, ctx, Some(&format!("{name}:{spec}")))?;
            srv.select_window(sid, idx)?;
        }
        "resize-window" => {
            let (sid, _, wid) = target::window(srv, ctx, a.value('t'))?;
            let win = srv.windows.get(&wid).ok_or("no window")?;
            let (mut cols, mut rows) = (win.cols, win.rows);
            if a.has('A') || a.has('a') {
                let sizes: Vec<(u16, u16)> = srv
                    .clients
                    .values()
                    .filter_map(|c| c.att.as_ref())
                    .filter(|att| att.session == sid)
                    .map(|att| (att.cols, att.rows.saturating_sub(STATUS_ROWS)))
                    .collect();
                // The largest (-A) or smallest (-a) client, each way.
                let both = if a.has('A') {
                    |x: (u16, u16), y: (u16, u16)| (x.0.max(y.0), x.1.max(y.1))
                } else {
                    |x: (u16, u16), y: (u16, u16)| (x.0.min(y.0), x.1.min(y.1))
                };
                (cols, rows) = sizes
                    .into_iter()
                    .reduce(both)
                    .ok_or("resize-window: no client attached")?;
            }
            let number = |v: &str| v.parse::<u16>().map_err(|_| format!("bad size: {v}"));
            if let Some(x) = a.value('x') {
                cols = number(x)?;
            }
            if let Some(y) = a.value('y') {
                rows = number(y)?;
            }
            let n = pos.first().map_or(Ok(1), |n| number(n))?;
            match dir_flag(a) {
                Some(Dir::Left) => cols = cols.saturating_sub(n),
                Some(Dir::Right) => cols = cols.saturating_add(n),
                Some(Dir::Up) => rows = rows.saturating_sub(n),
                Some(Dir::Down) => rows = rows.saturating_add(n),
                None => {}
            }
            let win = srv.windows.get_mut(&wid).ok_or("no window")?;
            win.manual_size = true;
            win.cols = cols.clamp(1, WINDOW_MAXIMUM);
            win.rows = rows.clamp(1, WINDOW_MAXIMUM);
            srv.relayout(wid);
        }
        "rename-window" => {
            let (_, _, wid) = target::window(srv, ctx, a.value('t'))?;
            if let Some(w) = srv.windows.get_mut(&wid) {
                w.name.clone_from(&pos[0]);
                w.auto_name = false;
            }
            srv.mark_window_dirty(wid);
        }
        "list-windows" => {
            let sessions: Vec<SessionId> = if a.has('a') {
                srv.sessions_by_name()
            } else {
                vec![target::session(srv, ctx, a.value('t'))?]
            };
            for sid in sessions {
                let s = &srv.sessions[&sid];
                for (idx, wid) in &s.windows {
                    let w = &srv.windows[wid];
                    let vars = Vars {
                        srv,
                        session: Some(sid),
                        window: Some(*wid),
                        pane: Some(w.active),
                        client: None,
                    };
                    let line = list_item(a, &vars, || {
                        let prefix = if a.has('a') {
                            format!("{}:", s.name)
                        } else {
                            String::new()
                        };
                        format!(
                            "{prefix}{idx}: {}{} ({} panes) [{}x{}]",
                            w.name,
                            w.flags(
                                *idx == s.current,
                                Some(*idx) == s.last,
                                srv.window_is_marked(*wid)
                            ),
                            w.panes().len(),
                            w.cols,
                            w.rows
                        )
                    });
                    if let Some(line) = line {
                        let _ = writeln!(out.stdout, "{line}");
                    }
                }
            }
        }
        "select-layout" if a.has('E') => {
            let (_, _, wid) = target::window(srv, ctx, a.value('t'))?;
            let win = srv.windows.get_mut(&wid).ok_or("no window")?;
            win.layout = crate::layout::spread(&win.layout, win.active);
            win.zoomed = false;
            srv.relayout(wid);
        }
        "next-layout" | "previous-layout" | "select-layout" => {
            let sizes = crate::layout::MainPane {
                height: srv.cfg.main_pane_height.clone(),
                width: srv.cfg.main_pane_width.clone(),
                other_height: srv.cfg.other_pane_height.clone(),
                other_width: srv.cfg.other_pane_width.clone(),
            };
            let (_, _, wid) = target::window(srv, ctx, a.value('t'))?;
            let win = srv.windows.get_mut(&wid).ok_or("no window")?;
            let name = if p.name() == "next-layout" || a.has('n') {
                win.preset = (win.preset + 1) % crate::layout::PRESETS.len();
                crate::layout::PRESETS[win.preset].to_owned()
            } else if p.name() == "previous-layout" || a.has('p') {
                let n = crate::layout::PRESETS.len();
                win.preset = (win.preset + n - 1) % n;
                crate::layout::PRESETS[win.preset].to_owned()
            } else {
                pos.first()
                    .cloned()
                    .unwrap_or_else(|| crate::layout::PRESETS[win.preset].to_owned())
            };
            let panes = win.panes();
            let tree = crate::layout::preset(&name, &panes, (win.cols, win.rows), &sizes)
                .ok_or_else(|| format!("unknown layout: {name}"))?;
            // The panes keep their numbers, as tmux's, though a mirrored
            // layout lays the first out last.
            let laid: Vec<crate::model::PaneId> = tree
                .leaves()
                .into_iter()
                .map(|p| p as crate::model::PaneId)
                .collect();
            win.order = if laid == panes { Vec::new() } else { panes };
            if let Some(i) = crate::layout::PRESETS.iter().position(|p| *p == name) {
                win.preset = i;
            }
            win.layout = tree;
            win.zoomed = false;
            srv.relayout(wid);
        }

        _ => return Ok(false),
    }
    Ok(true)
}
