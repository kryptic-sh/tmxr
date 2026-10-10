//! Resolving `-t` targets: `session:window.pane`.
//!
//! Sessions: `$id`, exact name, unique name prefix. Windows: `@id`, index
//! (`N` or `=N`), `+`/`-` (next/previous, optionally `+N`), `!` / `{last}`,
//! `^` / `{start}`, `$` / `{end}`, name or name prefix. Panes: `%id`, index,
//! `+`/`-`, `!` / `{last}`, `{left}`/`{right}`/`{up}`/`{down}`. Any part may be
//! empty to mean "current". In a mouse bind, `=` or `{mouse}` is the window or
//! pane under the mouse.

use crate::cmds::Ctx;
use crate::layout::{Dir, neighbour};
use crate::model::{PaneId, SessionId, WindowId};
use crate::server::Server;

pub type Res<T> = Result<T, String>;

/// The session a command acts on when `-t` is not given.
pub fn current_session(srv: &Server, ctx: &Ctx) -> Option<SessionId> {
    if let Some(s) = ctx
        .client
        .and_then(|c| srv.clients.get(&c))
        .and_then(|c| c.att.as_ref())
        .map(|a| a.session)
    {
        return Some(s);
    }
    if let Some(s) = ctx
        .pane
        .and_then(|p| srv.panes.get(&p))
        .and_then(|p| srv.session_of_window(p.window))
    {
        return Some(s);
    }
    srv.sessions
        .values()
        .max_by_key(|s| s.last_used)
        .map(|s| s.id)
}

/// `=` / `{mouse}`: the mouse target of a mouse bind.
fn is_mouse(spec: &str) -> bool {
    matches!(spec, "=" | "{mouse}")
}

fn current_pane(srv: &Server, ctx: &Ctx, session: SessionId) -> Option<PaneId> {
    // A command run from inside a pane targets that pane when it is in the
    // session being addressed.
    if let Some(p) = ctx.pane
        && srv
            .panes
            .get(&p)
            .is_some_and(|pane| srv.session_of_window(pane.window) == Some(session))
    {
        return Some(p);
    }
    srv.active_pane_of_session(session)
}

pub fn session(srv: &Server, ctx: &Ctx, spec: Option<&str>) -> Res<SessionId> {
    let spec = spec.map(|s| s.split(':').next().unwrap_or(s));
    match spec.filter(|s| !s.is_empty()) {
        None => current_session(srv, ctx).ok_or_else(|| "no current session".into()),
        Some(s) => find_session(srv, s),
    }
}

fn find_session(srv: &Server, name: &str) -> Res<SessionId> {
    let name = name.strip_prefix('=').unwrap_or(name);
    if let Some(id) = name
        .strip_prefix('$')
        .and_then(|n| n.parse::<SessionId>().ok())
        && srv.sessions.contains_key(&id)
    {
        return Ok(id);
    }
    if let Some(s) = srv.sessions.values().find(|s| s.name == name) {
        return Ok(s.id);
    }
    let prefixed: Vec<_> = srv
        .sessions
        .values()
        .filter(|s| s.name.starts_with(name))
        .collect();
    match prefixed.as_slice() {
        [one] => Ok(one.id),
        [] => Err(format!("can't find session: {name}")),
        _ => Err(format!("ambiguous session: {name}")),
    }
}

/// Split `session:window.pane` into its parts.
fn split_target(spec: &str) -> (Option<&str>, &str) {
    match spec.split_once(':') {
        Some((s, rest)) => (Some(s), rest),
        None => (None, spec),
    }
}

/// Resolve where a window should go (`new-window -t`, `move-window -t`):
/// `session:index`, where the index need not exist yet. A word without `:`
/// is an index when it is a number, else a session. A missing session means
/// the current one; a missing index means "the first free one".
pub fn destination(srv: &Server, ctx: &Ctx, spec: &str) -> Res<(SessionId, Option<u32>)> {
    let index = |s: &str| s.trim_start_matches('=').parse::<u32>().ok();
    let (sess, idx) = match spec.split_once(':') {
        Some((sess, rest)) => (sess, index(rest)),
        None if index(spec).is_some() => ("", index(spec)),
        None => (spec, None),
    };
    let sid = if sess.is_empty() {
        current_session(srv, ctx).ok_or("no current session")?
    } else {
        find_session(srv, sess)?
    };
    Ok((sid, idx))
}

/// Resolve a window target to `(session, index, window)`.
pub fn window(srv: &Server, ctx: &Ctx, spec: Option<&str>) -> Res<(SessionId, u32, WindowId)> {
    let spec = spec.unwrap_or("");
    if is_mouse(spec) {
        let t = ctx.mouse.ok_or("no mouse target")?;
        // Off any window (the status line's left part): the session's
        // current one, as tmux's cmd_mouse_window.
        let (idx, wid) = match t.window {
            Some(w) => w,
            None => {
                let s = srv
                    .sessions
                    .get(&t.session)
                    .ok_or("no session under the mouse")?;
                (
                    s.current,
                    s.current_window().ok_or("no window under the mouse")?,
                )
            }
        };
        return Ok((t.session, idx, wid));
    }
    if let Some(id) = spec
        .strip_prefix('@')
        .map(|r| r.split('.').next().unwrap_or(r))
        .and_then(|n| n.parse::<WindowId>().ok())
    {
        let sid = srv
            .session_of_window(id)
            .ok_or_else(|| format!("can't find window: @{id}"))?;
        let idx = srv.sessions[&sid].index_of(id).unwrap_or_default();
        return Ok((sid, idx, id));
    }
    let (sess, rest) = split_target(spec);
    let win_spec = rest.split('.').next().unwrap_or("");
    let sid = match sess {
        Some(s) if !s.is_empty() => find_session(srv, s)?,
        _ => {
            // A bare word with no ':' may name a session rather than a window.
            if sess.is_none() && !win_spec.is_empty() && !rest.contains('.') {
                let cur = current_session(srv, ctx);
                if let Some(cur) = cur
                    && let Ok(idx) = window_index(srv, cur, win_spec)
                {
                    let wid = srv.sessions[&cur].windows[&idx];
                    return Ok((cur, idx, wid));
                }
                if let Ok(s) = find_session(srv, win_spec) {
                    let sess = &srv.sessions[&s];
                    let wid = sess.current_window().ok_or("session has no windows")?;
                    return Ok((s, sess.current, wid));
                }
            }
            current_session(srv, ctx).ok_or("no current session")?
        }
    };
    let idx = if win_spec.is_empty() {
        // The window holding the context pane, when it is in this session.
        ctx.pane
            .and_then(|p| srv.panes.get(&p))
            .and_then(|p| srv.sessions[&sid].index_of(p.window))
            .filter(|_| {
                ctx.client
                    .is_none_or(|c| srv.clients.get(&c).is_none_or(|c| c.att.is_none()))
            })
            .unwrap_or_else(|| srv.sessions[&sid].current)
    } else {
        window_index(srv, sid, win_spec)?
    };
    let wid = *srv.sessions[&sid]
        .windows
        .get(&idx)
        .ok_or_else(|| format!("can't find window: {idx}"))?;
    Ok((sid, idx, wid))
}

fn window_index(srv: &Server, sid: SessionId, spec: &str) -> Res<u32> {
    let s = &srv.sessions[&sid];
    let indices: Vec<u32> = s.windows.keys().copied().collect();
    let pos = indices.iter().position(|i| *i == s.current).unwrap_or(0);
    let step = |sign: i64, n: &str| -> Res<u32> {
        let n: i64 = if n.is_empty() {
            1
        } else {
            n.parse().map_err(|_| format!("bad window: {spec}"))?
        };
        let len = indices.len() as i64;
        let i = (pos as i64 + sign * n).rem_euclid(len);
        Ok(indices[i as usize])
    };
    match spec {
        "!" | "{last}" => s.last.ok_or_else(|| "no last window".into()),
        "^" | "{start}" => indices.first().copied().ok_or_else(|| "no windows".into()),
        "$" | "{end}" => indices.last().copied().ok_or_else(|| "no windows".into()),
        _ if spec.starts_with('+') => step(1, &spec[1..]),
        _ if spec.starts_with('-') => step(-1, &spec[1..]),
        _ => {
            let exact_index = spec.strip_prefix('=').unwrap_or(spec);
            if let Ok(i) = exact_index.parse::<u32>() {
                return if s.windows.contains_key(&i) {
                    Ok(i)
                } else {
                    Err(format!("can't find window: {i}"))
                };
            }
            let named: Vec<u32> = s
                .windows
                .iter()
                .filter(|(_, w)| srv.windows.get(w).is_some_and(|w| w.name == spec))
                .map(|(i, _)| *i)
                .collect();
            if let [one] = named.as_slice() {
                return Ok(*one);
            }
            let prefixed: Vec<u32> = s
                .windows
                .iter()
                .filter(|(_, w)| srv.windows.get(w).is_some_and(|w| w.name.starts_with(spec)))
                .map(|(i, _)| *i)
                .collect();
            match prefixed.as_slice() {
                [one] => Ok(*one),
                [] => Err(format!("can't find window: {spec}")),
                _ => Err(format!("ambiguous window: {spec}")),
            }
        }
    }
}

/// Resolve a pane target to `(session, window, pane)`.
pub fn pane(srv: &Server, ctx: &Ctx, spec: Option<&str>) -> Res<(SessionId, WindowId, PaneId)> {
    let spec = spec.unwrap_or("");
    if is_mouse(spec) {
        let t = ctx.mouse.ok_or("no mouse target")?;
        // Off a pane (the status line): that window's active pane, as
        // tmux's cmd_mouse_pane.
        let pane = match t.pane {
            Some(p) => p,
            None => {
                let (_, _, wid) = window(srv, ctx, Some("="))?;
                srv.windows
                    .get(&wid)
                    .ok_or("no window under the mouse")?
                    .active
            }
        };
        let p = srv.panes.get(&pane).ok_or("no pane under the mouse")?;
        return Ok((t.session, p.window, pane));
    }
    if let Some(id) = spec
        .strip_prefix('%')
        .and_then(|n| n.parse::<PaneId>().ok())
    {
        let p = srv
            .panes
            .get(&id)
            .ok_or_else(|| format!("can't find pane: %{id}"))?;
        let sid = srv
            .session_of_window(p.window)
            .ok_or("pane has no session")?;
        return Ok((sid, p.window, id));
    }
    if spec.is_empty() {
        let sid = current_session(srv, ctx).ok_or("no current session")?;
        let pid = current_pane(srv, ctx, sid).ok_or("no current pane")?;
        let wid = srv.panes[&pid].window;
        return Ok((sid, wid, pid));
    }
    // `window.pane`, `session:window.pane`, a bare pane spec, or — like tmux
    // — a bare window or session name meaning its active pane.
    let pane_token = |p: &str| {
        p.chars().all(|c| c.is_ascii_digit())
            || matches!(
                p,
                "+" | "-"
                    | "!"
                    | "{last}"
                    | "{next}"
                    | "{previous}"
                    | "{left}"
                    | "{right}"
                    | "{up}"
                    | "{down}"
            )
    };
    let (win_part, pane_part) = match spec.rsplit_once('.') {
        Some((w, p)) => (Some(w), p),
        None if spec.contains(':') || !pane_token(spec) => (Some(spec), ""),
        None => (None, spec),
    };
    let (sid, _, wid) = match win_part {
        Some(w) => window(srv, ctx, Some(w))?,
        None => {
            let sid = current_session(srv, ctx).ok_or("no current session")?;
            let pid = current_pane(srv, ctx, sid).ok_or("no current pane")?;
            let wid = srv.panes[&pid].window;
            (sid, 0, wid)
        }
    };
    let win = &srv.windows[&wid];
    let panes = win.panes();
    let current = if ctx.pane.is_some_and(|p| panes.contains(&p)) {
        ctx.pane.unwrap_or(win.active)
    } else {
        win.active
    };
    let pos = panes.iter().position(|p| *p == current).unwrap_or(0);
    let n = panes.len();
    let rects = crate::layout::pane_rects(&win.layout, win.cols, win.rows);
    let dir =
        |d| neighbour(&rects, current, win.last_pane, d, win.cols, win.rows).ok_or("no pane there");
    let pid = match pane_part {
        "" => current,
        "!" | "{last}" => win.last_pane.ok_or("no last pane")?,
        "+" | "{next}" => panes[(pos + 1) % n],
        "-" | "{previous}" => panes[(pos + n - 1) % n],
        "{left}" => dir(Dir::Left)?,
        "{right}" => dir(Dir::Right)?,
        "{up}" => dir(Dir::Up)?,
        "{down}" => dir(Dir::Down)?,
        idx => {
            let i: u32 = idx.parse().map_err(|_| format!("can't find pane: {idx}"))?;
            let i = i
                .checked_sub(srv.cfg.pane_base_index)
                .ok_or_else(|| format!("can't find pane: {idx}"))?;
            *panes
                .get(i as usize)
                .ok_or_else(|| format!("can't find pane: {idx}"))?
        }
    };
    Ok((sid, wid, pid))
}
