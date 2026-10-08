//! Window commands.

use std::fmt::Write as _;

use tmxr_command::Parsed;

use super::{Ctx, Outcome, client_size, cwd_arg};
use crate::model::SessionId;
use crate::server::Server;
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
                size,
                !a.has('d'),
            )?;
        }
        "move-window" => {
            let (_, _, wid) = target::window(srv, ctx, a.value('s'))?;
            let (dst, index) = match a.value('t') {
                Some(t) if !t.is_empty() => target::destination(srv, ctx, t)?,
                _ => (target::session(srv, ctx, None)?, None),
            };
            srv.move_window(wid, dst, index, !a.has('d'))?;
        }
        "swap-window" => {
            let s = a.value('s').ok_or("swap-window needs -s src-window")?;
            let (_, _, src) = target::window(srv, ctx, Some(s))?;
            let (_, _, dst) = target::window(srv, ctx, a.value('t'))?;
            srv.swap_windows(src, dst, a.has('d'))?;
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
        // -a: the next / previous window with an alert (a bell), cycling.
        "next-window" | "previous-window" if a.has('a') => {
            let sid = target::session(srv, ctx, a.value('t'))?;
            let s = &srv.sessions[&sid];
            let alerted = |i: &&u32| srv.windows.get(&s.windows[*i]).is_some_and(|w| w.bell);
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
                srv.sessions.keys().copied().collect()
            } else {
                vec![target::session(srv, ctx, a.value('t'))?]
            };
            for sid in sessions {
                let s = &srv.sessions[&sid];
                for (idx, wid) in &s.windows {
                    let w = &srv.windows[wid];
                    let prefix = if a.has('a') {
                        format!("{}:", s.name)
                    } else {
                        String::new()
                    };
                    let _ = writeln!(
                        out.stdout,
                        "{prefix}{idx}: {}{} ({} panes) [{}x{}]",
                        w.name,
                        w.flags(*idx == s.current, Some(*idx) == s.last),
                        w.panes().len(),
                        w.cols,
                        w.rows
                    );
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
        "next-layout" | "select-layout" => {
            let (_, _, wid) = target::window(srv, ctx, a.value('t'))?;
            let win = srv.windows.get_mut(&wid).ok_or("no window")?;
            let name = if p.name() == "next-layout" || a.has('n') {
                win.preset = (win.preset + 1) % crate::layout::PRESETS.len();
                crate::layout::PRESETS[win.preset].to_owned()
            } else if a.has('p') {
                let n = crate::layout::PRESETS.len();
                win.preset = (win.preset + n - 1) % n;
                crate::layout::PRESETS[win.preset].to_owned()
            } else {
                pos.first()
                    .cloned()
                    .unwrap_or_else(|| crate::layout::PRESETS[win.preset].to_owned())
            };
            let tree = crate::layout::preset(&name, &win.panes())
                .ok_or_else(|| format!("unknown layout: {name}"))?;
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
