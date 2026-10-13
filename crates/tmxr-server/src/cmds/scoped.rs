//! Session, window and user options where tmux keeps them: global, a
//! session's own, a window's own (`-w`) or a pane's own (`-p`), each
//! inheriting from the one above (`options_scope_from_name` and
//! `cmd_show_options` in tmux 3.6).

use std::borrow::Cow;

use tmxr_command::Args;
use tmxr_command::keys::Key;
use tmxr_config::{PANE_OPTIONS, SessionOptions, WindowOptions};

use super::{Ctx, Outcome, Res, join_args};
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

/// Whether `name` is kept by scope here: a session, window or user option.
pub(super) fn handles(name: &str) -> bool {
    name.starts_with('@') || is_session_option(name) || WindowOptions::NAMES.contains(&name)
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
        Scope::GlobalSession | Scope::GlobalWindow if name.starts_with('@') => {
            srv.cfg.options.get(name).cloned()
        }
        Scope::GlobalSession => srv.cfg.session.show(name),
        Scope::GlobalWindow => srv.cfg.window.show(name),
        Scope::Session(s) => srv.sessions.get(&s)?.options.show(name),
        Scope::Window(w) => srv.windows.get(&w)?.options.show(name),
        Scope::Pane(p) => srv.panes.get(&p)?.options.show(name),
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

/// `set-option` and `set-window-option` for an option [`handles`] takes.
pub(super) fn set(
    srv: &mut Server,
    ctx: &Ctx,
    a: &Args,
    window_cmd: bool,
    name: &str,
    value: Option<&str>,
) -> Res {
    let scope = scope(srv, ctx, a, window_cmd, Some(name))?;
    let unset = a.has('u') || a.has('U');
    if !unset && a.has('o') && own(srv, scope, name).is_some() {
        return Err(format!("already set: {name}"));
    }
    if name == "prefix" && !unset {
        value
            .ok_or("prefix: needs a value")?
            .parse::<Key>()
            .map_err(|e| e.to_string())?;
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
        Scope::GlobalSession | Scope::GlobalWindow if unset && user => {
            srv.cfg.options.remove(name);
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
            let v = srv.cfg.options.entry(name.to_owned()).or_default();
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
    if name.starts_with("pane-scrollbars") {
        for &w in &windows {
            srv.relayout(w);
        }
    }
    if name == "window-size" {
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
    let scope = scope(srv, ctx, a, window_cmd, name)?;
    // Values quoted to be read back, as the global list's.
    let quoted = |v: String| join_args(&[v]);
    let print = |out: &mut Outcome, name: &str| {
        let line = match own(srv, scope, name) {
            Some(v) if a.has('v') => v,
            Some(v) => format!("{name} {}", quoted(v)),
            None if !a.has('A') => return false,
            None => match inherited(srv, scope, name) {
                Some(v) if a.has('v') => v,
                Some(v) => format!("{name}* {}", quoted(v)),
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
        Scope::GlobalWindow => Vec::new(),
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
    Ok(())
}
