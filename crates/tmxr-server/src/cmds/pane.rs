//! Pane commands, including the vim/hjkl navigator.

use std::fmt::Write as _;

use crossterm::event::KeyEvent;
use tmxr_command::{Args, Key, Parsed};

use super::{Ctx, Outcome, Res, attached_client, cwd_arg, dir_flag, list_item};
use crate::model::{PaneId, SessionId, WindowId};
use crate::overlay::Overlay;
use crate::server::{Server, SplitSize};
use crate::target;
use crate::vars::Vars;

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
            let (sid, wid, pid) = target::pane(srv, ctx, a.value('t'))?;
            let size = split_size(a)?;
            let cwd = cwd_arg(srv, ctx, Some(pid), a)
                .or_else(|| srv.panes.get(&pid).map(|p| p.start_cwd.clone()))
                .unwrap_or_else(crate::util::home_dir);
            // -Z: the window zoomed on the active pane after, zoomed before
            // or not, as tmux's.
            let zoom = srv.push_zoom(wid, true, a.has('Z'));
            let new = srv.split(
                pid,
                a.has('h'),
                a.has('b'),
                size,
                cwd,
                pos.to_vec(),
                &super::env_flags(a)?,
                !a.has('d'),
            )?;
            srv.pop_zoom(wid, zoom);
            super::print_new(srv, ctx, a, out, sid, Some(new), super::NEW_PANE_TEMPLATE);
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
        "select-pane" if a.has('d') || a.has('e') => {
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            // -e wins, as tmux's.
            set_input_off(srv, pid, !a.has('e'));
        }
        "select-pane" => {
            let (_, wid, pid) = target::pane(srv, ctx, a.value('t'))?;
            // -T titles the pane (the program may title it again).
            if let Some(title) = a.value('T') {
                let title = super::expand_for(srv, ctx, Some(pid), title);
                let p = srv.panes.get_mut(&pid).ok_or("no such pane")?;
                p.emu.set_title(title);
                srv.mark_window_dirty(wid);
                return Ok(true);
            }
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
            // -Z: a zoomed window stays zoomed, now on the selected pane.
            let zoom = srv.push_zoom(wid, false, a.has('Z'));
            srv.select_pane(to);
            srv.pop_zoom(wid, zoom);
        }
        "last-pane" => {
            let (_, wid, _) = target::window(srv, ctx, a.value('t'))?;
            let last = srv.windows[&wid].last_pane.ok_or("no last pane")?;
            // -d / -e: the last pane's input off or on, and it stays last.
            if a.has('d') || a.has('e') {
                set_input_off(srv, last, a.has('d'));
            } else {
                let zoom = srv.push_zoom(wid, false, a.has('Z'));
                srv.select_pane(last);
                srv.pop_zoom(wid, zoom);
            }
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
                    // -U / -D: the previous / next pane is the source, as
                    // tmux's.
                    let panes = srv.windows.get(&wid).ok_or("no window")?.panes();
                    let i = panes.iter().position(|p| *p == pid).unwrap_or(0);
                    let n = panes.len();
                    let other = if a.has('U') {
                        panes[(i + n - 1) % n]
                    } else {
                        panes[(i + 1) % n]
                    };
                    (other, pid)
                }
            };
            // -Z: each window zoomed before stays zoomed, as tmux's.
            let (src_w, dst_w) = (srv.panes[&src].window, srv.panes[&dst].window);
            let zoom_dst = srv.push_zoom(dst_w, false, a.has('Z'));
            let zoom_src = src_w != dst_w && srv.push_zoom(src_w, false, a.has('Z'));
            let swapped = srv.swap_panes(src, dst, a.has('d'));
            srv.pop_zoom(src_w, zoom_src);
            srv.pop_zoom(dst_w, zoom_dst);
            swapped?;
        }
        "respawn-pane" => {
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            // A running program is only replaced with -k; a dead pane (kept
            // by remain-on-exit) is respawned as is.
            if !a.has('k') && srv.panes[&pid].dead.is_none() {
                return Err(format!("respawn pane failed: pane %{pid} still active"));
            }
            let cwd = cwd_arg(srv, ctx, Some(pid), a);
            srv.respawn_pane(pid, pos.to_vec(), cwd, &super::env_flags(a)?)?;
        }
        "respawn-window" => {
            // tmux: the window goes back to one pane, its first, respawned.
            let (_, _, wid) = target::window(srv, ctx, a.value('t'))?;
            let panes = srv.windows.get(&wid).ok_or("no window")?.panes();
            let first = *panes.first().ok_or("window has no panes")?;
            if !a.has('k') && panes.iter().any(|p| srv.panes[p].dead.is_none()) {
                return Err(format!("respawn window failed: window @{wid} still active"));
            }
            for p in &panes[1..] {
                srv.kill_pane(*p);
            }
            let cwd = cwd_arg(srv, ctx, Some(first), a);
            srv.respawn_pane(first, pos.to_vec(), cwd, &super::env_flags(a)?)?;
        }
        "copy-command-line" => {
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            srv.copy_command_line(pid)?;
        }
        "pipe-pane" => {
            let (sid, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            let line = pos
                .first()
                .filter(|l| !l.is_empty())
                .map(|l| super::expand_for(srv, ctx, Some(pid), l));
            // Neither -I nor -O: the pane's output, as tmux's default.
            let dir = crate::pipe::Direction {
                output: a.has('O') || !a.has('I'),
                input: a.has('I'),
            };
            let shell = srv.session_options(sid).default_shell.clone();
            let env = srv.child_env(Some(sid));
            let (events, id) = (srv.events.clone(), srv.next_pipe);
            srv.next_pipe += 1;
            let p = srv.panes.get_mut(&pid).ok_or("no such pane")?;
            // An open pipe always closes first; -o makes the command a toggle.
            let had = p.pipe.take().is_some();
            if let Some(line) = line.filter(|_| !(a.has('o') && had)) {
                let pipe = crate::pipe::PanePipe::open(
                    id,
                    (&line, shell.as_deref(), &env),
                    dir,
                    pid,
                    events,
                )
                .map_err(|e| format!("pipe-pane: {line}: {e}"))?;
                p.pipe = Some(pipe);
            }
        }
        "clear-history" => {
            // -H also drops hyperlinks in tmux; tmxr keeps none.
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            let p = srv.panes.get_mut(&pid).ok_or("no such pane")?;
            p.emu.clear_history();
            let window = p.window;
            srv.mark_window_dirty(window);
        }
        // tmux's move-pane is join-pane under another name.
        "join-pane" | "move-pane" => {
            let src = match (a.value('s'), srv.marked_pane()) {
                (Some(s), _) => target::pane(srv, ctx, Some(s))?.2,
                (None, Some(m)) => m,
                // The marked pane, else the current one, as tmux's.
                (None, None) => target::pane(srv, ctx, None)?.2,
            };
            let dst = target::pane(srv, ctx, a.value('t'))?.2;
            let size = split_size(a)?;
            srv.join_pane(src, dst, a.has('h'), a.has('b'), size, !a.has('d'))?;
        }
        "rotate-window" => {
            let (_, wid, _) = target::window(srv, ctx, a.value('t'))?;
            // It unzooms, or with -Z zooms again after, as tmux's.
            let zoom = srv.push_zoom(wid, false, a.has('Z'));
            let w = srv.windows.get_mut(&wid).ok_or("no window")?;
            // tmux rotates up unless -D: its pane list turns, each place in
            // it keeping its cell.
            let panes = w.panes();
            w.layout = crate::layout::rotate(&w.layout, &panes, !a.has('D'));
            let mut order = panes;
            if a.has('D') {
                order.rotate_right(1);
            } else {
                order.rotate_left(1);
            }
            // The pane after the active one in the turned list takes over
            // (before it with -D), as tmux's.
            let n = order.len();
            let i = order.iter().position(|p| *p == w.active).unwrap_or(0);
            let next = if a.has('D') {
                order[(i + n - 1) % n]
            } else {
                order[(i + 1) % n]
            };
            if !w.order.is_empty() {
                w.order = order;
            }
            srv.relayout(wid);
            srv.select_pane(next);
            srv.pop_zoom(wid, zoom);
        }
        "break-pane" => {
            // As tmux's: -s the pane, -t the window it becomes.
            let (src, wid, pid) = target::pane(srv, ctx, a.value('s'))?;
            let (dst, mut index) = match a.value('t') {
                Some(t) if !t.is_empty() => target::destination(srv, ctx, t)?,
                _ => (src, None),
            };
            // -a / -b: next to the target window, the later ones moving up.
            if a.has('a') || a.has('b') {
                index = srv.index_beside(dst, index, a.has('b'));
            }
            let name = a.value('n');
            // A window's only pane: the window itself moves.
            if srv.windows[&wid].panes().len() < 2 {
                srv.move_window(wid, src, dst, index, !a.has('d'))?;
                if let Some(n) = name
                    && let Some(w) = srv.windows.get_mut(&wid)
                {
                    n.clone_into(&mut w.name);
                    w.options.automatic_rename = Some(false);
                }
                srv.mark_window_dirty(wid);
                super::print_new(srv, ctx, a, out, dst, Some(pid), super::NEW_PANE_TEMPLATE);
                return Ok(true);
            }
            if let Some(i) = index
                && srv.sessions[&dst].windows.contains_key(&i)
            {
                return Err(format!("index in use: {i}"));
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
            let title = name.map_or_else(
                || srv.commands.get(&pid).cloned().unwrap_or_default(),
                str::to_owned,
            );
            let new = srv.adopt_pane(dst, pid, title, index, cols, rows, !a.has('d'))?;
            // -n names it for good, as rename-window does.
            if name.is_some()
                && let Some(w) = srv.windows.get_mut(&new)
            {
                w.options.automatic_rename = Some(false);
            }
            srv.relayout(new);
            super::print_new(srv, ctx, a, out, dst, Some(pid), super::NEW_PANE_TEMPLATE);
        }
        "list-panes" => {
            // -a every pane on the server, -s every pane in the session,
            // else the target window's; each line says as much as it needs
            // to name its pane, as tmux's.
            let windows: Vec<(SessionId, u32, WindowId)> = if a.has('a') || a.has('s') {
                let only = if a.has('a') {
                    None
                } else {
                    Some(target::session(srv, ctx, a.value('t'))?)
                };
                srv.sessions_by_name()
                    .iter()
                    .filter_map(|s| srv.sessions.get(s))
                    .filter(|s| only.is_none_or(|o| o == s.id))
                    .flat_map(|s| s.windows.iter().map(|(i, w)| (s.id, *i, *w)))
                    .collect()
            } else {
                vec![target::window(srv, ctx, a.value('t'))?]
            };
            for (sid, idx, wid) in windows {
                let w = &srv.windows[&wid];
                for (i, p) in w.panes().iter().enumerate() {
                    let pane = &srv.panes[p];
                    let vars = Vars {
                        srv,
                        session: Some(sid),
                        window: Some(wid),
                        pane: Some(*p),
                        client: None,
                    };
                    let line = list_item(a, &vars, || {
                        let prefix = if a.has('a') {
                            format!("{}:{idx}.", srv.sessions[&sid].name)
                        } else if a.has('s') {
                            format!("{idx}.")
                        } else {
                            String::new()
                        };
                        format!(
                            "{prefix}{}: [{}x{}] %{}{}",
                            i as u32 + srv.window_options(wid).pane_base_index,
                            pane.rect.w,
                            pane.rect.h,
                            p,
                            if *p == w.active { " (active)" } else { "" }
                        )
                    });
                    if let Some(line) = line {
                        let _ = writeln!(out.stdout, "{line}");
                    }
                }
            }
        }
        "display-panes" => {
            let (_, _, wid) = target::window(srv, ctx, None)?;
            let base = srv.window_options(wid).pane_base_index;
            let labels: Vec<(u32, PaneId)> = srv.windows[&wid]
                .panes()
                .into_iter()
                .enumerate()
                .map(|(i, p)| (i as u32 + base, p))
                .collect();
            match attached_client(srv, ctx) {
                // On a client: numbers over the panes; a number selects.
                Some(c) => {
                    let time = std::time::Duration::from_millis(
                        srv.client_session_options(c).display_panes_time,
                    );
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
            // -S / -E may be formats (`-S -#{history_size}`), as tmux's.
            let start = a
                .value('S')
                .map(|v| super::expand_for(srv, ctx, Some(pid), v));
            let end = a
                .value('E')
                .map(|v| super::expand_for(srv, ctx, Some(pid), v));
            let pane = srv.panes.get_mut(&pid).ok_or("no such pane")?;
            let range = (start.as_deref(), end.as_deref());
            let text = capture(pane.emu.screen_mut(), range, a.has('J'), a.has('N'));
            if a.has('p') {
                out.stdout.push_str(&text);
            } else {
                srv.add_buffer(text, a.value('b').map(str::to_owned));
            }
        }
        "send-keys" => {
            if a.has('M') {
                return crate::mouse::forward(srv, ctx).map(|()| true);
            }
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            // -F: each argument is a format, expanded for the target pane.
            let expanded: Vec<String>;
            let pos = if a.has('F') {
                expanded = pos
                    .iter()
                    .map(|w| super::expand_for(srv, ctx, Some(pid), w))
                    .collect();
                &expanded[..]
            } else {
                pos
            };
            if a.has('X') {
                let (name, rest) = pos.split_first().ok_or("send-keys -X needs a command")?;
                // A copy command's own arguments may start with `--`, as
                // tmux's default `#` / `*` binds do; it marks their end.
                let rest = rest.strip_prefix(&["--".to_owned()][..]).unwrap_or(rest);
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
            // -N: the keys again and again, as tmux's.
            let repeat = match a.value('N') {
                Some(n) => n
                    .parse::<u32>()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| format!("repeat count {n}"))?,
                None => 1,
            };
            for _ in 0..repeat {
                for word in pos {
                    // -H: each word is a byte in hex; one that is not is
                    // skipped, as tmux's.
                    if a.has('H') {
                        if let Ok(b) = u8::from_str_radix(word, 16) {
                            srv.type_bytes_to_pane(pid, &[b]);
                        }
                        continue;
                    }
                    match word.parse::<Key>() {
                        Ok(k) if !a.has('l') && word.chars().count() > 1 => {
                            srv.send_key_to_pane(pid, &KeyEvent::new(k.code, k.mods));
                        }
                        _ => srv.type_to_pane(pid, word),
                    }
                }
            }
        }
        "send-prefix" => {
            let (sid, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            let k = srv.prefix_key(sid);
            srv.send_key_to_pane(pid, &KeyEvent::new(k.code, k.mods));
        }

        _ => return Ok(false),
    }
    Ok(true)
}

/// `-l size` of `split-window` / `join-pane`: cells, or `N%` of the pane.
/// A pane's visible rows as `capture-pane` gives them, as tmux's: one line
/// per row, blank rows kept, with its trailing spaces dropped; with `join`
/// (`-J`) a wrapped row runs on into the next and trailing spaces stay.
/// The pane's lines from `start` to `end` (`-S` / `-E`), as tmux's
/// `cmd_capture_pane_history`: 0 is the screen's first row, a negative line
/// is in the history, `-` is the history's top or the screen's bottom, and
/// the default (or a value that is not a number) is the screen. Trailing
/// blanks go unless `keep_blanks` (`-N`); `join` (`-J`) runs a wrapped line
/// into the next. (tmux's `-J` keeps spaces a program wrote at a line's end,
/// which the history snapshot cannot tell from blank cells.)
fn capture(
    screen: &mut vt100::Screen,
    (start, end): (Option<&str>, Option<&str>),
    join: bool,
    keep_blanks: bool,
) -> String {
    let lines = crate::copy::snapshot(screen);
    let rows = usize::from(screen.size().0);
    let history = lines.len().saturating_sub(rows);
    let last = lines.len().saturating_sub(1);
    let line = |value: Option<&str>, dash: usize, default: usize| match value {
        Some("-") => dash,
        Some(v) => v.parse::<i64>().map_or(default, |n| {
            let at = i64::try_from(history).unwrap_or(i64::MAX).saturating_add(n);
            usize::try_from(at).unwrap_or(0).min(last)
        }),
        None => default,
    };
    let top = line(start, 0, history);
    let bottom = line(end, last, last);
    let (top, bottom) = (top.min(bottom), top.max(bottom));
    let mut out = String::new();
    for l in lines.get(top..=bottom).unwrap_or_default() {
        let text: String = l.cells.iter().map(|c| c.symbol.as_str()).collect();
        if keep_blanks {
            out.push_str(&text);
        } else {
            out.push_str(text.trim_end_matches(' '));
        }
        if !(join && l.wrapped) {
            out.push('\n');
        }
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// `select-pane -d` / `-e`: turn `pane`'s input off or back on.
fn set_input_off(srv: &mut Server, pane: PaneId, off: bool) {
    if let Some(p) = srv.panes.get_mut(&pane) {
        p.input_off = off;
        let window = p.window;
        srv.mark_window_dirty(window);
    }
}

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
