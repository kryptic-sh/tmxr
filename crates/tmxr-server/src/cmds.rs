//! Command execution.

use std::fmt::Write as _;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tmxr_command::format::expand;
use tmxr_command::{Args, Key, Parsed};

use crate::keys::BindSpec;
use crate::layout::Dir;
use crate::model::{ClientId, PaneId, SessionId};
use crate::overlay::Overlay;
use crate::server::{STATUS_ROWS, Server, SplitSize};
use crate::target;
use crate::vars::Vars;

/// Who is running a command and on whose behalf.
#[derive(Debug, Clone, Default)]
pub struct Ctx {
    /// The client that sent the command (attached or not).
    pub client: Option<ClientId>,
    /// The pane commands default to (the pane a key was pressed in, or the
    /// pane a command client runs inside).
    pub pane: Option<PaneId>,
    /// The key that triggered a bind, for commands that may pass it on.
    pub key: Option<KeyEvent>,
    /// The command client's working directory.
    pub cwd: Option<PathBuf>,
    /// The command client's environment.
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
    /// Attach the client to this session when it can.
    pub attach: Option<SessionId>,
}

impl Outcome {
    pub fn error(msg: String) -> Self {
        Self {
            status: 1,
            stderr: msg,
            ..Self::default()
        }
    }
}

type Res = Result<(), String>;

/// Run a command line written in tmux's language (a bind, the prompt).
pub fn run_string(srv: &mut Server, ctx: &Ctx, line: &str) -> Outcome {
    let env = |k: &str| std::env::var(k).ok();
    match tmxr_command::tokenize(line, &env) {
        Ok(cmds) => run_list(srv, ctx, &cmds),
        Err(e) => Outcome::error(e.to_string()),
    }
}

/// Run an argv from the command line, where a `;` word separates commands.
pub fn run_argv_list(srv: &mut Server, ctx: &Ctx, argv: &[String]) -> Outcome {
    let cmds: Vec<Vec<String>> = argv
        .split(|w| w == ";")
        .filter(|c| !c.is_empty())
        .map(<[String]>::to_vec)
        .collect();
    run_list(srv, ctx, &cmds)
}

/// Run commands in order, stopping at the first error.
pub fn run_list(srv: &mut Server, ctx: &Ctx, cmds: &[Vec<String>]) -> Outcome {
    let mut out = Outcome::default();
    for argv in cmds {
        let parsed = match tmxr_command::parse(argv) {
            Ok(p) => p,
            Err(e) => {
                out.status = 1;
                out.stderr.push_str(&e.to_string());
                return out;
            }
        };
        if let Err(e) = run_one(srv, ctx, &parsed, &mut out) {
            out.status = 1;
            out.stderr.push_str(&e);
            return out;
        }
    }
    out
}

fn attached_client(srv: &Server, ctx: &Ctx) -> Option<ClientId> {
    ctx.client
        .filter(|c| srv.clients.get(c).is_some_and(|c| c.att.is_some()))
}

fn client_size(srv: &Server, ctx: &Ctx) -> (u16, u16) {
    let from_client = ctx.client.and_then(|c| srv.clients.get(&c)).and_then(|c| {
        c.att.as_ref().map(|a| (a.cols, a.rows)).or_else(|| {
            c.hello
                .as_ref()?
                .terminal
                .as_ref()
                .map(|t| (t.cols, t.rows))
        })
    });
    let (cols, rows) = from_client.unwrap_or((80, 24));
    (cols.max(2), rows.saturating_sub(STATUS_ROWS).max(2))
}

fn expand_for(srv: &Server, ctx: &Ctx, pane: Option<PaneId>, s: &str) -> String {
    expand(s, &Vars::for_pane(srv, pane.or(ctx.pane), ctx.client))
}

fn cwd_arg(srv: &Server, ctx: &Ctx, pane: Option<PaneId>, a: &Args) -> Option<PathBuf> {
    a.value('c')
        .map(|c| PathBuf::from(expand_for(srv, ctx, pane, c)))
}

fn session_env(srv: &Server, ctx: &Ctx) -> Vec<(String, String)> {
    let wanted = &srv.cfg.update_environment;
    let from = |k: &str| -> Option<String> {
        ctx.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    };
    wanted
        .iter()
        .filter_map(|k| from(k).map(|v| (k.clone(), v)))
        .collect()
}

fn dir_flag(a: &Args) -> Option<Dir> {
    if a.has('L') {
        Some(Dir::Left)
    } else if a.has('R') {
        Some(Dir::Right)
    } else if a.has('U') {
        Some(Dir::Up)
    } else if a.has('D') {
        Some(Dir::Down)
    } else {
        None
    }
}

fn on_off(value: Option<&str>, current: bool) -> Result<bool, String> {
    match value {
        None | Some("toggle") => Ok(!current),
        Some("on" | "1" | "yes" | "true") => Ok(true),
        Some("off" | "0" | "no" | "false") => Ok(false),
        Some(v) => Err(format!("bad value: {v}")),
    }
}

/// Quote words so they survive another trip through the tokenizer.
pub fn join_args(words: &[String]) -> String {
    words
        .iter()
        .map(|w| {
            if !w.is_empty()
                && w.chars()
                    .all(|c| c.is_alphanumeric() || "-_./:%@#{}=+,^".contains(c))
                && !w.starts_with('#')
            {
                w.clone()
            } else {
                format!("'{}'", w.replace('\'', r"'\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[allow(clippy::too_many_lines)]
fn run_one(srv: &mut Server, ctx: &Ctx, p: &Parsed, out: &mut Outcome) -> Res {
    let a = &p.args;
    let pos = a.positional();
    match p.name() {
        // ── Sessions ────────────────────────────────────────────────────────
        "new-session" => {
            let size = match (a.value('x'), a.value('y')) {
                (Some(x), Some(y)) => (
                    x.parse().map_err(|_| "bad -x")?,
                    y.parse::<u16>()
                        .map_err(|_| "bad -y")?
                        .saturating_sub(STATUS_ROWS)
                        .max(1),
                ),
                _ => client_size(srv, ctx),
            };
            let cwd = cwd_arg(srv, ctx, None, a)
                .or_else(|| ctx.cwd.clone())
                .unwrap_or_else(crate::util::home_dir);
            let sid = srv.new_session(
                a.value('s').map(str::to_owned),
                cwd,
                session_env(srv, ctx),
                a.value('n').map(str::to_owned),
                pos.to_vec(),
                size,
            )?;
            if !a.has('d') {
                match attached_client(srv, ctx) {
                    Some(c) => srv.switch_client(c, sid),
                    None => out.attach = Some(sid),
                }
            }
        }
        "attach-session" => {
            if srv.sessions.is_empty() {
                return Err("no sessions".into());
            }
            let sid = target::session(srv, ctx, a.value('t'))?;
            if a.has('d') {
                let others: Vec<ClientId> = srv
                    .clients
                    .values()
                    .filter(|c| c.att.as_ref().is_some_and(|x| x.session == sid))
                    .filter(|c| Some(c.id) != ctx.client)
                    .map(|c| c.id)
                    .collect();
                for c in others {
                    srv.detach(c, "detached");
                }
            }
            match attached_client(srv, ctx) {
                Some(c) => srv.switch_client(c, sid),
                None => out.attach = Some(sid),
            }
        }
        "detach-client" => {
            let targets: Vec<ClientId> = if a.has('a') {
                srv.clients
                    .values()
                    .filter(|c| c.att.is_some() && Some(c.id) != ctx.client)
                    .map(|c| c.id)
                    .collect()
            } else if let Some(s) = a.value('s') {
                let sid = target::session(srv, ctx, Some(s))?;
                srv.clients
                    .values()
                    .filter(|c| c.att.as_ref().is_some_and(|x| x.session == sid))
                    .map(|c| c.id)
                    .collect()
            } else {
                attached_client(srv, ctx).into_iter().collect()
            };
            for c in targets {
                srv.detach(c, "detached");
            }
        }
        "has-session" => {
            target::session(srv, ctx, a.value('t'))?;
        }
        "kill-server" => srv.exiting = true,
        "kill-session" => {
            let sid = target::session(srv, ctx, a.value('t'))?;
            if a.has('a') {
                let others: Vec<SessionId> =
                    srv.sessions.keys().copied().filter(|s| *s != sid).collect();
                for s in others {
                    srv.kill_session(s);
                }
            } else {
                srv.kill_session(sid);
            }
        }
        "list-sessions" => {
            for s in srv.sessions.values() {
                let attached = srv
                    .clients
                    .values()
                    .any(|c| c.att.as_ref().is_some_and(|a| a.session == s.id));
                let _ = writeln!(
                    out.stdout,
                    "{}: {} windows{}",
                    s.name,
                    s.windows.len(),
                    if attached { " (attached)" } else { "" }
                );
            }
        }
        "rename-session" => {
            let sid = target::session(srv, ctx, a.value('t'))?;
            let name = &pos[0];
            if name.is_empty() || name.contains([':', '.']) {
                return Err(format!("bad session name: {name}"));
            }
            if srv
                .sessions
                .values()
                .any(|s| s.id != sid && &s.name == name)
            {
                return Err(format!("duplicate session: {name}"));
            }
            if let Some(s) = srv.sessions.get_mut(&sid) {
                s.name.clone_from(name);
            }
            srv.mark_session_dirty(sid);
        }
        "switch-client" => {
            let c = attached_client(srv, ctx).ok_or("no current client")?;
            if let Some(table) = a.value('T') {
                if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                    table.clone_into(&mut att.table);
                    att.dirty = true;
                }
                return Ok(());
            }
            let cur = srv.clients[&c]
                .att
                .as_ref()
                .map(|x| x.session)
                .ok_or("not attached")?;
            let sid = if a.has('l') {
                srv.clients[&c]
                    .att
                    .as_ref()
                    .and_then(|x| x.last_session)
                    .filter(|s| srv.sessions.contains_key(s))
                    .ok_or("no last session")?
            } else if a.has('n') || a.has('p') {
                let ids: Vec<SessionId> = srv.sessions.keys().copied().collect();
                let i = ids.iter().position(|s| *s == cur).unwrap_or(0);
                let n = ids.len();
                if a.has('n') {
                    ids[(i + 1) % n]
                } else {
                    ids[(i + n - 1) % n]
                }
            } else {
                target::session(srv, ctx, a.value('t'))?
            };
            srv.switch_client(c, sid);
        }

        // ── Windows ─────────────────────────────────────────────────────────
        "new-window" => {
            let (sid, cur_idx, _) = target::window(srv, ctx, None)?;
            let (sid, index) = match a.value('t') {
                Some(t) if !t.is_empty() => {
                    let (sess, rest) = t.split_once(':').unwrap_or(("", t));
                    let sid = if sess.is_empty() {
                        sid
                    } else {
                        target::session(srv, ctx, Some(sess))?
                    };
                    let idx = rest.trim_start_matches('=').parse::<u32>().ok();
                    (sid, idx)
                }
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

        // ── Panes ───────────────────────────────────────────────────────────
        "split-window" => {
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            let size = match a.value('l') {
                None => None,
                Some(l) => Some(match l.strip_suffix('%') {
                    Some(pc) => {
                        SplitSize::Percent(pc.parse().map_err(|_| format!("bad size: {l}"))?)
                    }
                    None => SplitSize::Cells(l.parse().map_err(|_| format!("bad size: {l}"))?),
                }),
            };
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
        "select-pane" => {
            let (_, wid, pid) = target::pane(srv, ctx, a.value('t'))?;
            let to = if a.has('l') {
                srv.windows[&wid].last_pane.ok_or("no last pane")?
            } else if let Some(d) = dir_flag(a) {
                let w = &srv.windows[&wid];
                let rects = crate::layout::pane_rects(&w.layout, w.cols, w.rows);
                match crate::layout::neighbour(&rects, pid, w.last_pane, d, w.cols, w.rows) {
                    Some(n) => n,
                    None => return Ok(()),
                }
            } else {
                pid
            };
            srv.select_pane(to);
        }
        "last-pane" => {
            let (_, wid, _) = target::window(srv, ctx, a.value('t'))?;
            let last = srv.windows[&wid].last_pane.ok_or("no last pane")?;
            srv.select_pane(last);
        }
        "navigate-pane" => navigate(srv, ctx, a)?,
        "resize-pane" => {
            let (_, wid, pid) = target::pane(srv, ctx, a.value('t'))?;
            if a.has('Z') {
                let w = srv.windows.get_mut(&wid).ok_or("no window")?;
                if w.panes().len() > 1 || w.zoomed {
                    w.active = pid;
                    w.zoomed = !w.zoomed;
                }
                srv.relayout(wid);
                return Ok(());
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
            let w = srv.windows.get_mut(&wid).ok_or("no window")?;
            let panes = w.panes();
            let i = panes.iter().position(|p| *p == pid).unwrap_or(0);
            let n = panes.len();
            let other = if let Some(s) = a.value('s') {
                let _ = s;
                return Err("swap-pane -s is not supported; use -U or -D".into());
            } else if a.has('U') {
                panes[(i + n - 1) % n]
            } else {
                panes[(i + 1) % n]
            };
            w.layout = crate::layout::swap(&w.layout, pid, other);
            if !a.has('d') {
                w.active = pid;
            }
            srv.relayout(wid);
        }
        "rotate-window" => {
            let (_, wid, _) = target::window(srv, ctx, a.value('t'))?;
            let w = srv.windows.get_mut(&wid).ok_or("no window")?;
            w.layout = crate::layout::rotate(&w.layout, a.has('U'));
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
            let w = &srv.windows[&wid];
            let list: Vec<String> = w
                .panes()
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{}=%{p}", i as u32 + srv.cfg.pane_base_index))
                .collect();
            let _ = writeln!(out.stdout, "panes: {}", list.join(" "));
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
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            if a.has('X') {
                let (name, rest) = pos.split_first().ok_or("send-keys -X needs a command")?;
                return crate::copy::command(srv, ctx, pid, name, rest);
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

        // ── Keys ────────────────────────────────────────────────────────────
        "bind-key" => {
            let table = if a.has('n') {
                "root"
            } else {
                a.value('T').unwrap_or("prefix")
            };
            let key: Key = pos[0]
                .parse()
                .map_err(|e: tmxr_command::keys::UnknownKey| e.to_string())?;
            let cmd = if pos.len() == 2 && pos[1].contains(' ') {
                pos[1].clone()
            } else {
                join_args(&pos[1..])
            };
            if cmd.is_empty() {
                // `bind -N note key` with no command only changes the note.
                if let Some(b) = srv.keys.get(table, &key).cloned() {
                    srv.keys.bind(
                        table,
                        key,
                        BindSpec {
                            note: a.value('N').map(str::to_owned),
                            ..b
                        },
                    );
                }
                return Ok(());
            }
            srv.keys.bind(
                table,
                key,
                BindSpec {
                    cmd,
                    note: a.value('N').map(str::to_owned),
                    repeat: a.has('r'),
                },
            );
        }
        "unbind-key" => {
            let table = if a.has('n') {
                "root"
            } else {
                a.value('T').unwrap_or("prefix")
            };
            if a.has('a') {
                srv.keys.unbind_all(table);
            } else {
                let key: Key = pos
                    .first()
                    .ok_or("unbind-key needs a key")?
                    .parse()
                    .map_err(|e: tmxr_command::keys::UnknownKey| e.to_string())?;
                srv.keys.unbind(table, &key);
            }
        }
        "list-keys" => {
            let only_notes = a.has('N');
            let table = a.value('T');
            let binds = srv.keys.list(table);
            let width = binds
                .iter()
                .map(|(_, k, _)| k.to_string().len())
                .max()
                .unwrap_or(0);
            for (table, key, b) in binds {
                if only_notes {
                    let Some(note) = &b.note else { continue };
                    let prefix = match table.as_str() {
                        "prefix" => format!("{} ", srv.prefix),
                        "root" => String::new(),
                        t => format!("[{t}] "),
                    };
                    let k = format!("{prefix}{key}");
                    let _ = writeln!(out.stdout, "{k:<w$}  {note}", w = width + 6);
                } else {
                    let r = if b.repeat { "-r " } else { "" };
                    let _ = writeln!(
                        out.stdout,
                        "bind-key {r}-T {table} {} {}",
                        join_args(&[key.to_string()]),
                        b.cmd
                    );
                }
            }
        }

        // ── Copy mode and buffers ───────────────────────────────────────────
        "copy-mode" => {
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            crate::copy::enter(srv, pid, a.has('u'));
        }
        "paste-buffer" => {
            let (_, _, pid) = target::pane(srv, ctx, a.value('t'))?;
            let idx = match a.value('b') {
                Some(n) => srv
                    .buffers
                    .iter()
                    .position(|b| b.name == n)
                    .ok_or_else(|| format!("no buffer {n}"))?,
                None if srv.buffers.is_empty() => return Ok(()),
                None => 0,
            };
            let data = srv.buffers[idx].data.clone();
            srv.paste_to_pane(pid, &data);
            if a.has('d') {
                srv.buffers.remove(idx);
            }
        }
        "set-buffer" => {
            // Unlike tmux, the data is format-expanded, so a bind can copy
            // `#{pane_current_path}` (tmux-yank's prefix-Y) without a script.
            let data = expand_for(srv, ctx, None, &pos[0]);
            let data = match (a.has('a'), a.value('b')) {
                (true, name) => {
                    let base = match name {
                        Some(n) => srv.buffers.iter().find(|b| b.name == n),
                        None => srv.buffers.front(),
                    };
                    format!(
                        "{}{data}",
                        base.map(|b| b.data.as_str()).unwrap_or_default()
                    )
                }
                _ => data,
            };
            if a.has('w') {
                srv.set_clipboard(&data);
            }
            srv.add_buffer(data, a.value('b').map(str::to_owned));
        }
        "show-buffer" => {
            let b = match a.value('b') {
                Some(n) => srv.buffers.iter().find(|b| b.name == n),
                None => srv.buffers.front(),
            };
            out.stdout.push_str(&b.ok_or("no buffers")?.data);
        }
        "list-buffers" => {
            for b in &srv.buffers {
                let preview: String = b
                    .data
                    .chars()
                    .take(50)
                    .map(|c| if c == '\n' { ' ' } else { c })
                    .collect();
                let _ = writeln!(
                    out.stdout,
                    "{}: {} bytes: \"{preview}\"",
                    b.name,
                    b.data.len()
                );
            }
        }
        "delete-buffer" => match a.value('b') {
            Some(n) => srv.buffers.retain(|b| b.name != n),
            None => {
                srv.buffers.pop_front();
            }
        },

        // ── Options and config ──────────────────────────────────────────────
        "set-option" | "set-window-option" => set_option(srv, ctx, p, out)?,
        "show-options" => {
            let c = &srv.cfg;
            let lines = [
                format!("prefix {}", c.prefix),
                format!("mouse {}", if c.mouse { "on" } else { "off" }),
                format!("base-index {}", c.base_index),
                format!("pane-base-index {}", c.pane_base_index),
                format!(
                    "renumber-windows {}",
                    if c.renumber_windows { "on" } else { "off" }
                ),
                format!("mode-keys {}", c.mode_keys),
                format!("history-limit {}", c.history_limit),
                format!("display-time {}", c.display_time),
                format!("status-interval {}", c.status_interval),
                format!("repeat-time {}", c.repeat_time),
                format!("default-terminal {}", c.default_terminal),
                format!("extended-keys {}", c.extended_keys),
                format!("set-clipboard {}", c.set_clipboard),
            ];
            let mut all: Vec<String> = lines.to_vec();
            all.extend(
                c.options
                    .iter()
                    .map(|(k, v)| format!("{k} {}", join_args(std::slice::from_ref(v)))),
            );
            for l in all {
                if pos
                    .first()
                    .is_none_or(|n| l.split(' ').next() == Some(n.as_str()))
                {
                    let l = if a.has('v') {
                        l.split_once(' ')
                            .map_or_else(|| l.clone(), |(_, v)| v.to_owned())
                    } else {
                        l
                    };
                    out.stdout.push_str(&l);
                    out.stdout.push('\n');
                }
            }
        }
        "source-file" => {
            if let Some(path) = pos.first() {
                srv.cfg_path = Some(PathBuf::from(path));
            }
            srv.reload_config()?;
            if let Some(c) = attached_client(srv, ctx) {
                srv.show_message(c, "config reloaded".into());
            }
        }

        // ── Prompts and messages ────────────────────────────────────────────
        "command-prompt" => {
            let c = attached_client(srv, ctx).ok_or("no current client")?;
            let initial = a
                .value('I')
                .map(|i| expand_for(srv, ctx, None, i))
                .unwrap_or_default();
            let prompt = a
                .value('p')
                .map_or_else(|| ":".to_owned(), |p| format!("{p} "));
            let template = pos.first().cloned();
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(Overlay::prompt(prompt, initial, template));
            }
        }
        "confirm-before" => {
            let c = attached_client(srv, ctx).ok_or("no current client")?;
            let cmd = if pos.len() == 1 {
                pos[0].clone()
            } else {
                join_args(pos)
            };
            let prompt = a.value('p').map_or_else(
                || {
                    format!(
                        "Confirm '{}'? (y/n)",
                        cmd.split_whitespace().next().unwrap_or("")
                    )
                },
                |p| expand_for(srv, ctx, None, p),
            );
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(Overlay::confirm(prompt, cmd));
            }
        }
        "display-message" => {
            let (_, _, pid) = target::pane(srv, ctx, a.value('t')).unwrap_or((0, 0, 0));
            let fmt = pos.first().map_or(
                "[#S] #I:#P [#{pane_width}x#{pane_height}] \"#{pane_title}\" #{pane_current_command} #{pane_current_path}",
                String::as_str,
            );
            let msg = expand_for(srv, ctx, Some(pid), fmt);
            match attached_client(srv, ctx) {
                Some(c) if !a.has('p') => srv.show_message(c, msg),
                _ => {
                    out.stdout.push_str(&msg);
                    out.stdout.push('\n');
                }
            }
        }
        "show-messages" => {
            for m in &srv.messages {
                out.stdout.push_str(m);
                out.stdout.push('\n');
            }
        }
        "refresh-client" => {
            if let Some(c) = attached_client(srv, ctx)
                && let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut())
            {
                att.full_redraw = true;
                att.dirty = true;
            }
        }
        "run-shell" => {
            let line = expand_for(srv, ctx, None, &pos[0]);
            crate::server::run_shell(srv, attached_client(srv, ctx), line, a.has('b'));
        }
        "choose-tree" => {
            let c = attached_client(srv, ctx).ok_or("no current client")?;
            let ov = if a.has('w') {
                Overlay::window_picker(srv, c)
            } else {
                Overlay::session_picker(srv, c)
            };
            if let Some(att) = srv.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                att.overlay = Some(ov);
            }
        }
        "list-commands" => {
            for c in tmxr_command::table::COMMANDS {
                let alias = c.alias.map(|a| format!(" ({a})")).unwrap_or_default();
                let _ = writeln!(out.stdout, "{}{alias} {}", c.name, c.usage);
            }
        }
        "resurrect-save" => {
            let path = crate::resurrect::save(srv)?;
            if let Some(c) = attached_client(srv, ctx) {
                srv.show_message(c, format!("saved sessions to {}", path.display()));
            }
        }
        "resurrect-restore" => {
            let n = crate::resurrect::restore(srv, client_size(srv, ctx))?;
            if let Some(c) = attached_client(srv, ctx) {
                srv.show_message(c, format!("restored {n} sessions"));
            }
        }
        other => return Err(format!("{other}: not implemented")),
    }
    Ok(())
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

fn set_option(srv: &mut Server, ctx: &Ctx, p: &Parsed, out: &mut Outcome) -> Res {
    let a = &p.args;
    let pos = a.positional();
    let name = pos[0].as_str();
    let value = pos.get(1).map(String::as_str);
    let _ = out;
    if name == "synchronize-panes" || name == "automatic-rename" {
        let (_, _, wid) = target::window(srv, ctx, a.value('t'))?;
        let w = srv.windows.get_mut(&wid).ok_or("no window")?;
        match name {
            "synchronize-panes" => w.synchronize = on_off(value, w.synchronize)?,
            _ => w.auto_name = on_off(value, w.auto_name)?,
        }
        srv.mark_window_dirty(wid);
        return Ok(());
    }
    if name.starts_with('@') {
        if a.has('u') {
            srv.cfg.options.remove(name);
        } else {
            let v = value.ok_or("option needs a value")?;
            let v = if a.has('a') {
                format!(
                    "{}{v}",
                    srv.cfg.options.get(name).cloned().unwrap_or_default()
                )
            } else {
                v.to_owned()
            };
            srv.cfg.options.insert(name.to_owned(), v);
        }
        srv.mark_all_dirty();
        return Ok(());
    }
    let need = || value.ok_or_else(|| format!("{name}: needs a value"));
    let num = |v: &str| {
        v.parse::<u64>()
            .map_err(|_| format!("{name}: bad number {v}"))
    };
    let c = &mut srv.cfg;
    match name {
        "prefix" => {
            let v = need()?;
            srv.prefix = v
                .parse()
                .map_err(|e: tmxr_command::keys::UnknownKey| e.to_string())?;
            v.clone_into(&mut c.prefix);
        }
        "mouse" => c.mouse = on_off(value, c.mouse)?,
        "renumber-windows" => c.renumber_windows = on_off(value, c.renumber_windows)?,
        "base-index" => c.base_index = num(need()?)? as u32,
        "pane-base-index" => c.pane_base_index = num(need()?)? as u32,
        "history-limit" => c.history_limit = num(need()?)? as usize,
        "display-time" => c.display_time = num(need()?)?,
        "status-interval" => c.status_interval = num(need()?)?,
        "repeat-time" => c.repeat_time = num(need()?)?,
        "escape-time" => c.escape_time = num(need()?)?,
        "mode-keys" => need()?.clone_into(&mut c.mode_keys),
        "default-terminal" => need()?.clone_into(&mut c.default_terminal),
        "default-shell" => c.default_shell = Some(need()?.to_owned()),
        "extended-keys" => need()?.clone_into(&mut c.extended_keys),
        "set-clipboard" => need()?.clone_into(&mut c.set_clipboard),
        "status-left" => need()?.clone_into(&mut c.status.left),
        "status-right" => need()?.clone_into(&mut c.status.right),
        "status-style" => need()?.clone_into(&mut c.status.style),
        "window-status-format" => need()?.clone_into(&mut c.status.window_format),
        "window-status-current-format" => need()?.clone_into(&mut c.status.window_current_format),
        "pane-border-style" => need()?.clone_into(&mut c.status.pane_border_style),
        "pane-active-border-style" => need()?.clone_into(&mut c.status.pane_active_border_style),
        "message-style" => need()?.clone_into(&mut c.status.message_style),
        "mode-style" => need()?.clone_into(&mut c.status.mode_style),
        _ => return Err(format!("invalid option: {name}")),
    }
    srv.mark_all_dirty();
    Ok(())
}

/// Key event for a key name, for tests and `send-keys`.
pub fn key_event(name: &str) -> Option<KeyEvent> {
    let k: Key = name.parse().ok()?;
    Some(KeyEvent::new(k.code, k.mods))
}

#[allow(dead_code)]
fn plain(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_args_survives_a_round_trip() {
        let words: Vec<String> = ["split-window", "-c", "#{pane_current_path}", "it's", ""]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let line = join_args(&words);
        let back = tmxr_command::tokenize(&line, &|_| None).unwrap();
        assert_eq!(back, vec![words]);
    }
}
