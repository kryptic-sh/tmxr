//! Resolving `-t` targets: `session:window.pane`.
//!
//! Sessions: `$id`, exact name, unique name prefix. A session name may hold
//! dots (tmux's may not): a target without `:` that is exactly a session's
//! name means that session, not `window.pane`. Windows: `@id`, index
//! (`N` or `=N`), `+`/`-` (next/previous, optionally `+N`), `!` / `{last}`,
//! `^` / `{start}`, `$` / `{end}`, name or name prefix. Panes: `%id`, index,
//! `+`/`-`, `!` / `{last}`, `{left}`/`{right}`/`{up}`/`{down}`. Any part may be
//! empty to mean "current". In a mouse bind, `=` or `{mouse}` is the window or
//! pane under the mouse. A whole target of `~` or `{marked}` is the marked
//! pane, its window and session.

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
pub fn is_mouse(spec: &str) -> bool {
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

/// The marked pane as `(session, index, window, pane)`, for a target of
/// `~` or `{marked}`, as tmux's `cmd_find_target`.
fn marked(srv: &Server) -> Res<(SessionId, u32, WindowId, PaneId)> {
    let pane = srv.marked_pane().ok_or("no marked target")?;
    let window = srv.panes[&pane].window;
    let session = srv.session_of_window(window).ok_or("no marked target")?;
    let index = srv.sessions[&session]
        .index_of(window)
        .ok_or("no marked target")?;
    Ok((session, index, window, pane))
}

fn is_marked(spec: &str) -> bool {
    matches!(spec, "~" | "{marked}")
}

pub fn session(srv: &Server, ctx: &Ctx, spec: Option<&str>) -> Res<SessionId> {
    if spec.is_some_and(is_marked) {
        return marked(srv).map(|(s, ..)| s);
    }
    let spec = spec.map(|s| s.split(':').next().unwrap_or(s));
    match spec.filter(|s| !s.is_empty()) {
        None => current_session(srv, ctx).ok_or_else(|| "no current session".into()),
        // The session under the mouse (a session range's), as tmux's
        // cmd_mouse_session.
        Some(s) if is_mouse(s) => ctx
            .mouse
            .map(|t| t.session)
            .ok_or_else(|| "no mouse target".into()),
        // A window or pane id names the session it is in, as tmux's.
        Some(s) if s.starts_with('@') || s.starts_with('%') => Ok(window(srv, ctx, Some(s))?.0),
        Some(s) => find_session(srv, s),
    }
}

/// A session name `new-session -s` and `rename-session` accept: not empty
/// and without `:`, which ends a target's session part. Dots are allowed.
pub fn check_session_name(name: &str) -> Res<()> {
    if name.is_empty() || name.contains(':') {
        return Err(format!("bad session name: {name}"));
    }
    Ok(())
}

/// Whether a target without `:` is exactly a session's name (`=` optional).
fn names_session(srv: &Server, spec: &str) -> bool {
    let name = spec.strip_prefix('=').unwrap_or(spec);
    srv.sessions.values().any(|s| s.name == name)
}

/// A session by `$id` or name, as tmux's `cmd_find_get_session`: the exact
/// name, else the one name it starts, else the one name it matches as an
/// `fnmatch` pattern. A leading `=` asks for the exact name only.
fn find_session(srv: &Server, name: &str) -> Res<SessionId> {
    let (name, exact) = match name.strip_prefix('=') {
        Some(n) => (n, true),
        None => (name, false),
    };
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
    let not_found = || format!("can't find session: {name}");
    if exact {
        return Err(not_found());
    }
    let pattern = crate::util::glob_regex(name);
    for matches in [
        &(|s: &str| s.starts_with(name)) as &dyn Fn(&str) -> bool,
        &|s: &str| pattern.is_match(s),
    ] {
        let found: Vec<_> = srv.sessions.values().filter(|s| matches(&s.name)).collect();
        match found.as_slice() {
            [one] => return Ok(one.id),
            [] => {}
            _ => return Err(format!("ambiguous session: {name}")),
        }
    }
    Err(not_found())
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
    let (sess, rest) = match spec.split_once(':') {
        Some((sess, rest)) => (sess, rest),
        None if index(spec).is_some() => ("", spec),
        None => (spec, ""),
    };
    let sid = if sess.is_empty() {
        current_session(srv, ctx).ok_or("no current session")?
    } else {
        find_session(srv, sess)?
    };
    // A number is an index that need not exist; anything else names a
    // window that does (`session:name`, `session:!`), as tmux's
    // `CMD_FIND_WINDOW_INDEX`.
    let idx = match rest {
        "" => None,
        r => match index(r) {
            Some(i) => Some(i),
            None => Some(window_index(srv, sid, r)?),
        },
    };
    Ok((sid, idx))
}

/// Resolve a window target to `(session, index, window)`.
pub fn window(srv: &Server, ctx: &Ctx, spec: Option<&str>) -> Res<(SessionId, u32, WindowId)> {
    let spec = spec.unwrap_or("");
    if is_marked(spec) {
        return marked(srv).map(|(s, i, w, _)| (s, i, w));
    }
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
    // A pane id names the window it is in, as tmux's `cmd_find_target`.
    if let Some(pid) = spec
        .strip_prefix('%')
        .and_then(|n| n.parse::<PaneId>().ok())
    {
        let wid = srv
            .panes
            .get(&pid)
            .map(|p| p.window)
            .ok_or_else(|| format!("can't find pane: %{pid}"))?;
        let sid = srv
            .session_of_window(wid)
            .ok_or_else(|| format!("can't find window: @{wid}"))?;
        let idx = srv.sessions[&sid].index_of(wid).unwrap_or_default();
        return Ok((sid, idx, wid));
    }
    let (sess, rest) = split_target(spec);
    let win_spec = rest.split('.').next().unwrap_or("");
    let sid = match sess {
        Some(s) if !s.is_empty() => find_session(srv, s)?,
        _ => {
            // A dotted word with no ':' that names a session is that
            // session, not `window.pane`.
            if sess.is_none() && rest.contains('.') && names_session(srv, rest) {
                let s = find_session(srv, rest)?;
                let sess = &srv.sessions[&s];
                let wid = sess.current_window().ok_or("session has no windows")?;
                return Ok((s, sess.current, wid));
            }
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
    // A leading `=` asks for an index or the exact name only, as tmux's
    // `cmd_find_get_window_with_session`.
    let (spec, exact) = match spec.strip_prefix('=') {
        Some(n) => (n, true),
        None => (spec, false),
    };
    match spec {
        _ if exact => {}
        "!" | "{last}" => return s.last().ok_or_else(|| "no last window".into()),
        "^" | "{start}" => return indices.first().copied().ok_or_else(|| "no windows".into()),
        "$" | "{end}" => return indices.last().copied().ok_or_else(|| "no windows".into()),
        "{next}" => return step(1, ""),
        "{previous}" => return step(-1, ""),
        _ if spec.starts_with('+') => return step(1, &spec[1..]),
        _ if spec.starts_with('-') => return step(-1, &spec[1..]),
        _ => {}
    }
    if let Ok(i) = spec.parse::<u32>() {
        return if s.windows.contains_key(&i) {
            Ok(i)
        } else {
            Err(format!("can't find window: {i}"))
        };
    }
    let pattern = crate::util::glob_regex(spec);
    let tests: [&dyn Fn(&str) -> bool; 3] = [&|n| n == spec, &|n| n.starts_with(spec), &|n| {
        pattern.is_match(n)
    }];
    let tests = if exact { &tests[..1] } else { &tests[..] };
    for matches in tests {
        let found: Vec<u32> = s
            .windows
            .iter()
            .filter(|(_, w)| srv.windows.get(w).is_some_and(|w| matches(&w.name)))
            .map(|(i, _)| *i)
            .collect();
        match found.as_slice() {
            [one] => return Ok(*one),
            [] => {}
            _ => return Err(format!("ambiguous window: {spec}")),
        }
    }
    Err(format!("can't find window: {spec}"))
}

/// The pane names tmux takes besides an index and `+N` / `-N`
/// (`cmd_find_pane_table`): the brace forms, and the edges and corners
/// `window_find_string` reads.
const PANE_WORDS: &[&str] = &[
    "!",
    "{last}",
    "{next}",
    "{previous}",
    "{up-of}",
    "{down-of}",
    "{left-of}",
    "{right-of}",
    "{top}",
    "{bottom}",
    "{left}",
    "{right}",
    "{top-left}",
    "{top-right}",
    "{bottom-left}",
    "{bottom-right}",
    "top",
    "bottom",
    "left",
    "right",
    "top-left",
    "top-right",
    "bottom-left",
    "bottom-right",
];

/// The pane at an edge or corner of a `cols` x `rows` window (`top`,
/// `{left}`, `bottom-right` and the like), as tmux's `window_find_string`:
/// the pane holding the middle of that edge, or that corner.
fn pane_at_edge(
    rects: &[(PaneId, hjkl_layout::LayoutRect)],
    cols: u16,
    rows: u16,
    word: &str,
) -> Option<PaneId> {
    let word = word.trim_start_matches('{').trim_end_matches('}');
    let (mid_x, mid_y) = (cols / 2, rows / 2);
    let (right, bottom) = (cols.saturating_sub(1), rows.saturating_sub(1));
    let (x, y) = match word.to_ascii_lowercase().as_str() {
        "top" => (mid_x, 0),
        "bottom" => (mid_x, bottom),
        "left" => (0, mid_y),
        "right" => (right, mid_y),
        "top-left" => (0, 0),
        "top-right" => (right, 0),
        "bottom-left" => (0, bottom),
        "bottom-right" => (right, bottom),
        _ => return None,
    };
    // A cell on a border counts for the pane before it, as tmux's
    // `window_get_active_at` reads one past each pane's right and bottom.
    rects
        .iter()
        .find(|(_, r)| x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h)
        .map(|(id, _)| *id)
}

/// Resolve a pane target to `(session, window, pane)`.
pub fn pane(srv: &Server, ctx: &Ctx, spec: Option<&str>) -> Res<(SessionId, WindowId, PaneId)> {
    let spec = spec.unwrap_or("");
    if is_marked(spec) {
        return marked(srv).map(|(s, _, w, p)| (s, w, p));
    }
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
        // A status line's pane range names a pane, which must be in the
        // window the mouse is on, as tmux's cmd_mouse_pane checks.
        if t.pane.is_some() && Some(p.window) != t.window.map(|(_, w)| w) {
            let (_, _, wid) = window(srv, ctx, Some("="))?;
            if p.window != wid {
                return Err("no pane under the mouse".into());
            }
        }
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
    // — a bare window or session name meaning its active pane. The pane's
    // `.` is looked for after the `:`, as the session name may hold dots.
    let pane_token = |p: &str| {
        p.chars().all(|c| c.is_ascii_digit())
            || p.starts_with('+')
            || p.starts_with('-')
            || PANE_WORDS.contains(&p)
    };
    let (win_part, pane_part) = match spec.split_once(':') {
        Some((sess, rest)) => match rest.rsplit_once('.') {
            Some((w, p)) => (Some(&spec[..sess.len() + 1 + w.len()]), p),
            None => (Some(spec), ""),
        },
        None if names_session(srv, spec) => (Some(spec), ""),
        None => match spec.rsplit_once('.') {
            Some((w, p)) => (Some(w), p),
            None if !pane_token(spec) => (Some(spec), ""),
            None => (None, spec),
        },
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
    let not_found = || format!("can't find pane: {pane_part}");
    let dir =
        |d| neighbour(&rects, current, win.last_pane, d, win.cols, win.rows).ok_or_else(not_found);
    // `+N` / `-N` count from the current pane, wrapping, as tmux's.
    let step = |forward: bool, count: &str| -> Res<PaneId> {
        let count = if count.is_empty() {
            1
        } else {
            count.parse::<usize>().map_err(|_| not_found())?
        };
        let k = count % n;
        Ok(panes[if forward {
            (pos + k) % n
        } else {
            (pos + n - k) % n
        }])
    };
    let pid = match pane_part {
        "" => current,
        "!" | "{last}" => win.last_pane.ok_or("no last pane")?,
        "{next}" => step(true, "")?,
        "{previous}" => step(false, "")?,
        "{left-of}" => dir(Dir::Left)?,
        "{right-of}" => dir(Dir::Right)?,
        "{up-of}" => dir(Dir::Up)?,
        "{down-of}" => dir(Dir::Down)?,
        p if p.starts_with('+') => step(true, &p[1..])?,
        p if p.starts_with('-') => step(false, &p[1..])?,
        p if p.parse::<u32>().is_err() => {
            pane_at_edge(&rects, win.cols, win.rows, p).ok_or_else(not_found)?
        }
        idx => {
            let i: u32 = idx.parse().map_err(|_| format!("can't find pane: {idx}"))?;
            let i = i
                .checked_sub(srv.window_options(win.id).pane_base_index)
                .ok_or_else(|| format!("can't find pane: {idx}"))?;
            *panes
                .get(i as usize)
                .ok_or_else(|| format!("can't find pane: {idx}"))?
        }
    };
    Ok((sid, wid, pid))
}
