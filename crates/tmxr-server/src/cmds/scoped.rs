//! Session, window and user options where tmux keeps them: global, a
//! session's own, a window's own (`-w`) or a pane's own (`-p`), each
//! inheriting from the one above (`options_scope_from_name` and
//! `cmd_show_options` in tmux 3.6).

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use tmxr_command::Args;
use tmxr_command::keys::Key;
use tmxr_config::{PANE_OPTIONS, SessionOptions, WindowOptions};

use super::{Ctx, Outcome, Res};
use crate::hooks::PANE_HOOKS;
use crate::model::{PaneId, SessionId, WindowId};
use crate::server::Server;
use crate::target;

/// Whose value an option command reads or changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    GlobalSession,
    GlobalWindow,
    Session(SessionId),
    Window(WindowId),
    Pane(PaneId),
}

/// Whether `name` is kept by scope here: a session, window or user option,
/// or an item of an array one (`name[index]`).
pub(super) fn handles(name: &str) -> bool {
    let name = split_index(name).map_or(name, |(n, _)| n);
    name.starts_with('@') || is_session_option(name) || WindowOptions::NAMES.contains(&name)
}

/// An option argument as tmux's `options_parse` reads it: `name`, or
/// `name[index]` for an item of an array option.
fn split_index(arg: &str) -> Result<(&str, Option<u32>), String> {
    match arg.strip_suffix(']').and_then(|s| s.split_once('[')) {
        Some((name, i)) => i
            .parse()
            .map(|i| (name, Some(i)))
            .map_err(|_| format!("invalid option: {arg}")),
        None => Ok((arg, None)),
    }
}

fn is_array(name: &str) -> bool {
    SessionOptions::is_array(name) || WindowOptions::is_array(name)
}

/// `value` as `show-options` prints option `name`'s: quoted as tmux's when
/// tmux types it a string (as every user option), else as it is.
pub(super) fn shown(name: &str, value: String) -> String {
    if name.starts_with('@') || SessionOptions::is_string(name) || WindowOptions::is_string(name) {
        tmxr_command::escape_arg(&value)
    } else {
        value
    }
}

fn is_session_option(name: &str) -> bool {
    SessionOptions::NAMES.contains(&name)
}

/// The scope a command's flags pick, as tmux's. A session or window option
/// is found in its own table whatever `-w` / `-p` say: `-g` the global one,
/// else the target session's or window's (`-p` a pane's, for the options a
/// pane may hold). A user option, or a list (`name` `None`), goes by the
/// flags: `-p` a pane, `-w` (or `set-window-option`) a window, else a
/// session.
fn scope(
    srv: &Server,
    ctx: &Ctx,
    a: &Args,
    window_cmd: bool,
    name: Option<&str>,
) -> Result<Scope, String> {
    let global = a.has('g');
    let session = |srv| {
        Ok(if global {
            Scope::GlobalSession
        } else {
            Scope::Session(target::session(srv, ctx, a.value('t'))?)
        })
    };
    let window = |srv| {
        Ok(if global {
            Scope::GlobalWindow
        } else {
            Scope::Window(target::window(srv, ctx, a.value('t'))?.2)
        })
    };
    let pane = |srv| Ok(Scope::Pane(target::pane(srv, ctx, a.value('t'))?.2));
    match name {
        Some(n) if is_session_option(n) => session(srv),
        Some(n) if !n.starts_with('@') => {
            if a.has('p') && PANE_OPTIONS.contains(&n) {
                pane(srv)
            } else {
                window(srv)
            }
        }
        _ if a.has('p') => pane(srv),
        _ if window_cmd || a.has('w') => window(srv),
        _ => session(srv),
    }
}

/// The window options in effect at `scope`.
fn effective_window(srv: &Server, scope: Scope) -> Cow<'_, WindowOptions> {
    match scope {
        Scope::Window(w) => srv.window_options(w),
        Scope::Pane(p) => srv.pane_options(p),
        _ => Cow::Borrowed(&srv.cfg.window),
    }
}

/// `name`'s value set at `scope` itself; every global option is.
fn own(srv: &Server, scope: Scope, name: &str) -> Option<String> {
    match scope {
        Scope::GlobalSession if name.starts_with('@') => srv.cfg.options.get(name).cloned(),
        Scope::GlobalWindow if name.starts_with('@') => srv.cfg.window_user.get(name).cloned(),
        Scope::GlobalSession => srv.cfg.session.show(name),
        Scope::GlobalWindow => srv.cfg.window.show(name),
        Scope::Session(s) => srv.sessions.get(&s)?.options.show(name),
        Scope::Window(w) => srv.windows.get(&w)?.options.show(name),
        Scope::Pane(p) => srv.panes.get(&p)?.options.show(name),
    }
}

/// Array option `name`'s items set at `scope` itself; `None` when it has
/// none of its own.
fn own_items<'a>(srv: &'a Server, scope: Scope, name: &str) -> Option<&'a BTreeMap<u32, String>> {
    match scope {
        Scope::GlobalSession => srv.cfg.session.items(name),
        Scope::GlobalWindow => srv.cfg.window.items(name),
        Scope::Session(s) => srv.sessions.get(&s)?.options.items(name),
        Scope::Window(w) => srv.windows.get(&w)?.options.items(name),
        Scope::Pane(p) => srv.panes.get(&p)?.options.items(name),
    }
}

/// The scope `scope` inherits from: a pane's window, a window's or a
/// session's globals.
fn parent(srv: &Server, scope: Scope) -> Option<Scope> {
    match scope {
        Scope::GlobalSession | Scope::GlobalWindow => None,
        Scope::Session(_) => Some(Scope::GlobalSession),
        Scope::Window(_) => Some(Scope::GlobalWindow),
        Scope::Pane(p) => srv.panes.get(&p).map(|p| Scope::Window(p.window)),
    }
}

/// `name`'s value at `scope`, from the nearest scope that sets it.
fn inherited(srv: &Server, scope: Scope, name: &str) -> Option<String> {
    own(srv, scope, name).or_else(|| inherited(srv, parent(srv, scope)?, name))
}

/// Array option `name`'s items at `scope`, from the nearest scope that has
/// its own.
fn inherited_items<'a>(
    srv: &'a Server,
    scope: Scope,
    name: &str,
) -> Option<&'a BTreeMap<u32, String>> {
    own_items(srv, scope, name).or_else(|| inherited_items(srv, parent(srv, scope)?, name))
}

/// `set-option` and `set-window-option` for an option [`handles`] takes.
pub(super) fn set(
    srv: &mut Server,
    ctx: &Ctx,
    a: &Args,
    window_cmd: bool,
    name: &str,
    value: Option<&str>,
) -> Res {
    let arg = name;
    let (name, index) = split_index(arg)?;
    let scope = scope(srv, ctx, a, window_cmd, Some(name))?;
    let unset = a.has('u') || a.has('U');
    if let Some(index) = index {
        return set_item(srv, a, scope, (arg, name, index), value, unset);
    }
    if !unset && a.has('o') && own(srv, scope, name).is_some() {
        return Err(format!("already set: {name}"));
    }
    if (name == "prefix" || name == "prefix2") && !unset {
        let key = value.ok_or_else(|| format!("{name}: needs a value"))?;
        // prefix2 may be `None`, as tmux's default is.
        if !(name == "prefix2" && key == "None") {
            key.parse::<Key>().map_err(|e| e.to_string())?;
        }
    }
    if name == "allow-passthrough"
        && !unset
        && !tmxr_term::emulator::passthrough_supported()
        && super::on_off(value, effective_window(srv, scope).allow_passthrough)?
    {
        return Err(crate::server::PASSTHROUGH_UNSUPPORTED.into());
    }
    let append = a.has('a');
    let user = name.starts_with('@');
    match scope {
        Scope::GlobalSession if unset && user => {
            srv.cfg.options.remove(name);
        }
        Scope::GlobalWindow if unset && user => {
            srv.cfg.window_user.remove(name);
        }
        // As tmux's, a global option goes back to its default.
        Scope::GlobalSession if unset => {
            let default = tmxr_config::defaults().session.show(name);
            srv.cfg.session.set(name, default.as_deref(), false)?;
        }
        Scope::GlobalWindow if unset => {
            let default = tmxr_config::defaults().window.show(name);
            srv.cfg.window.set(name, default.as_deref(), false)?;
        }
        Scope::GlobalSession | Scope::GlobalWindow if user => {
            let value = value.ok_or("empty value")?;
            let table = if scope == Scope::GlobalSession {
                &mut srv.cfg.options
            } else {
                &mut srv.cfg.window_user
            };
            let v = table.entry(name.to_owned()).or_default();
            if !append {
                v.clear();
            }
            v.push_str(value);
        }
        Scope::GlobalSession => srv.cfg.session.set(name, value, append)?,
        Scope::GlobalWindow => srv.cfg.window.set(name, value, append)?,
        Scope::Session(s) => {
            let inherited = srv.cfg.session.clone();
            let session = srv.sessions.get_mut(&s).ok_or("no such session")?;
            if unset {
                session.options.unset(name);
            } else {
                session.options.set(name, value, append, &inherited)?;
            }
        }
        Scope::Window(w) => {
            let inherited = srv.cfg.window.clone();
            let win = srv.windows.get_mut(&w).ok_or("no such window")?;
            if unset {
                win.options.unset(name);
            } else {
                win.options.set(name, value, append, &inherited)?;
            }
            // -U: the window's panes' own values go too.
            if a.has('U') {
                for p in win.panes() {
                    if let Some(pane) = srv.panes.get_mut(&p) {
                        pane.options.unset(name);
                    }
                }
            }
        }
        Scope::Pane(p) => {
            let window = srv.panes.get(&p).ok_or("no such pane")?.window;
            let inherited = srv.window_options(window).into_owned();
            let pane = srv.panes.get_mut(&p).ok_or("no such pane")?;
            if unset {
                pane.options.unset(name);
            } else {
                pane.options.set(name, value, append, &inherited)?;
            }
        }
    }
    changed(srv, scope, name);
    Ok(())
}

/// `set-option` for item `index` of array option `name` (`arg` as given),
/// as tmux's: set it (`-a` appends to it) or with `-u` remove it, at
/// `scope`.
fn set_item(
    srv: &mut Server,
    a: &Args,
    scope: Scope,
    (arg, name, index): (&str, &str, u32),
    value: Option<&str>,
    unset: bool,
) -> Res {
    if name.starts_with('@') || !is_array(name) {
        return Err(format!("not an array: {arg}"));
    }
    if !unset && a.has('o') && own_items(srv, scope, name).is_some_and(|i| i.contains_key(&index)) {
        return Err(format!("already set: {arg}"));
    }
    let append = a.has('a');
    let value = if unset {
        None
    } else {
        Some(value.ok_or("empty value")?)
    };
    match (scope, value) {
        (Scope::GlobalSession, v) => srv.cfg.session.set_item(name, index, v, append)?,
        (Scope::GlobalWindow, v) => srv.cfg.window.set_item(name, index, v, append)?,
        (Scope::Session(s), v) => {
            let inherited = srv.cfg.session.clone();
            let own = &mut srv.sessions.get_mut(&s).ok_or("no such session")?.options;
            match v {
                Some(v) => own.set_item(name, index, v, append, &inherited)?,
                None => own.unset_item(name, index)?,
            }
        }
        (Scope::Window(w), v) => {
            let inherited = srv.cfg.window.clone();
            let own = &mut srv.windows.get_mut(&w).ok_or("no such window")?.options;
            match v {
                Some(v) => own.set_item(name, index, v, append, &inherited)?,
                None => own.unset_item(name, index)?,
            }
        }
        (Scope::Pane(p), v) => {
            let window = srv.panes.get(&p).ok_or("no such pane")?.window;
            let inherited = srv.window_options(window).into_owned();
            let own = &mut srv.panes.get_mut(&p).ok_or("no such pane")?.options;
            match v {
                Some(v) => own.set_item(name, index, v, append, &inherited)?,
                None => own.unset_item(name, index)?,
            }
        }
    }
    changed(srv, scope, name);
    Ok(())
}

/// Bring the windows `scope` reaches up to date with `name`'s new value.
fn changed(srv: &mut Server, scope: Scope, name: &str) {
    let windows: Vec<WindowId> = match scope {
        Scope::GlobalSession | Scope::GlobalWindow => srv.windows.keys().copied().collect(),
        Scope::Session(s) => srv
            .sessions
            .get(&s)
            .map(|s| s.windows.values().copied().collect())
            .unwrap_or_default(),
        Scope::Window(w) => vec![w],
        Scope::Pane(p) => srv.panes.get(&p).map(|p| p.window).into_iter().collect(),
    };
    if name.starts_with("pane-scrollbars") || name == "pane-border-status" {
        for &w in &windows {
            srv.relayout(w);
        }
    }
    if name == "window-size" || name == "status" {
        let sessions: Vec<SessionId> = srv
            .sessions
            .values()
            .filter(|s| s.windows.values().any(|w| windows.contains(w)))
            .map(|s| s.id)
            .collect();
        for s in sessions {
            srv.size_session(s);
        }
    }
    srv.mark_all_dirty();
}

/// `show-options` for one option [`handles`] takes, or for every option of
/// a session, window or pane: its own values, with `-A` the inherited ones
/// too, marked `*`. `-q` makes an unknown or unset name no error.
pub(super) fn show(
    srv: &Server,
    ctx: &Ctx,
    a: &Args,
    window_cmd: bool,
    name: Option<&str>,
    out: &mut Outcome,
) -> Res {
    let (name, index) = match name {
        Some(arg) => {
            let (name, index) = split_index(arg)?;
            (Some(name), index)
        }
        None => (None, None),
    };
    let scope = scope(srv, ctx, a, window_cmd, name)?;
    let listing = name.is_none();
    let print = |out: &mut Outcome, name: &str| {
        if is_array(name) {
            return show_array(srv, scope, a, (name, index, listing), out);
        }
        // An index on an option that is not an array is shown and
        // ignored, as tmux's.
        let label = index.map_or_else(|| name.to_owned(), |i| format!("{name}[{i}]"));
        let line = match own(srv, scope, name) {
            Some(v) if a.has('v') => v,
            Some(v) => format!("{label} {}", shown(name, v)),
            None if !a.has('A') => return false,
            None => match inherited(srv, scope, name) {
                Some(v) if a.has('v') => v,
                Some(v) => format!("{label}* {}", shown(name, v)),
                None => return false,
            },
        };
        out.stdout.push_str(&line);
        out.stdout.push('\n');
        true
    };
    if let Some(name) = name {
        // As tmux's: an unset user option is an error, an unset session or
        // window option prints nothing.
        if !print(out, name) && name.starts_with('@') && !a.has('q') {
            return Err(format!("invalid option: {name}"));
        }
        return Ok(());
    }
    let users: Vec<String> = match scope {
        Scope::GlobalSession => srv.cfg.options.keys().cloned().collect(),
        Scope::GlobalWindow => srv.cfg.window_user.keys().cloned().collect(),
        Scope::Session(s) => srv.sessions.get(&s).map_or_else(Vec::new, |s| {
            s.options.user().map(|(k, _)| k.to_owned()).collect()
        }),
        Scope::Window(w) => srv.windows.get(&w).map_or_else(Vec::new, |w| {
            w.options.user().map(|(k, _)| k.to_owned()).collect()
        }),
        Scope::Pane(p) => srv.panes.get(&p).map_or_else(Vec::new, |p| {
            p.options.user().map(|(k, _)| k.to_owned()).collect()
        }),
    };
    for n in &users {
        print(out, n);
    }
    let names: Vec<&str> = match scope {
        Scope::GlobalSession | Scope::Session(_) => SessionOptions::NAMES.to_vec(),
        Scope::Pane(_) => PANE_OPTIONS.to_vec(),
        Scope::GlobalWindow | Scope::Window(_) => WindowOptions::NAMES.to_vec(),
    };
    for n in names {
        print(out, n);
    }
    if a.has('H') {
        show_hooks(srv, scope, a, out);
    }
    Ok(())
}

/// `show-options` for array option `name` at `scope`, as tmux's
/// `cmd_show_options_print`: one `name[index] value` line per item (with
/// `-A` inherited ones marked `*`), or just item `index`; an empty array is
/// its bare name, marked `*` only in a `listing` of every option, as tmux's.
/// Returns whether it has a value there.
fn show_array(
    srv: &Server,
    scope: Scope,
    a: &Args,
    (name, index, listing): (&str, Option<u32>, bool),
    out: &mut Outcome,
) -> bool {
    let (items, mark) = match own_items(srv, scope, name) {
        Some(items) => (items, ""),
        None if a.has('A') => match inherited_items(srv, scope, name) {
            Some(items) => (items, "*"),
            None => return false,
        },
        None => return false,
    };
    let mut line = |i: u32, value: &str| {
        let _ = if a.has('v') {
            writeln!(out.stdout, "{value}")
        } else {
            writeln!(
                out.stdout,
                "{name}[{i}]{mark} {}",
                shown(name, value.to_owned())
            )
        };
    };
    match index {
        Some(i) => line(i, items.get(&i).map_or("", String::as_str)),
        None if items.is_empty() => {
            if !a.has('v') {
                let mark = if listing { mark } else { "" };
                let _ = writeln!(out.stdout, "{name}{mark}");
            }
        }
        None => {
            for (&i, value) in items {
                line(i, value);
            }
        }
    }
    true
}

/// The hooks set at `scope`, by name. The global table holds every global
/// hook, so a global scope has those tmux keeps there: the pane hooks for
/// the window options, the rest for the session options.
fn hooks_at(srv: &Server, scope: Scope) -> Vec<(&str, &[String])> {
    let table = match scope {
        Scope::GlobalSession | Scope::GlobalWindow => Some(&srv.hooks),
        Scope::Session(s) => srv.sessions.get(&s).map(|s| &s.hooks),
        Scope::Window(w) => srv.windows.get(&w).map(|w| &w.hooks),
        Scope::Pane(p) => srv.panes.get(&p).map(|p| &p.hooks),
    };
    let keeps = |name: &str| match scope {
        Scope::GlobalSession => !PANE_HOOKS.contains(&name),
        Scope::GlobalWindow => PANE_HOOKS.contains(&name),
        _ => true,
    };
    table
        .into_iter()
        .flatten()
        .filter(|(name, _)| keeps(name))
        .map(|(name, cmds)| (name.as_str(), cmds.as_slice()))
        .collect()
}

/// What `-H` adds to a list, as tmux's: each hook set at `scope` as
/// `name[index] command`, with `-A` the inherited ones too, marked `*`.
/// Hooks set nowhere are left out, as `show-hooks` leaves them.
fn show_hooks(srv: &Server, scope: Scope, a: &Args, out: &mut Outcome) {
    let mut hooks: BTreeMap<&str, (&str, &[String])> = hooks_at(srv, scope)
        .into_iter()
        .map(|(name, cmds)| (name, ("", cmds)))
        .collect();
    if a.has('A') {
        let mut up = parent(srv, scope);
        while let Some(s) = up {
            for (name, cmds) in hooks_at(srv, s) {
                hooks.entry(name).or_insert(("*", cmds));
            }
            up = parent(srv, s);
        }
    }
    for (name, (mark, cmds)) in hooks {
        for (i, cmd) in cmds.iter().enumerate() {
            if a.has('v') {
                let _ = writeln!(out.stdout, "{cmd}");
            } else {
                let _ = writeln!(out.stdout, "{name}[{i}]{mark} {cmd}");
            }
        }
    }
}
