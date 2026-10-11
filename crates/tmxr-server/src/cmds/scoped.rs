//! Window options and user options where tmux keeps them: global, a window's
//! own (`-w`) or a pane's own (`-p`), each inheriting from the one above
//! (`options_scope_from_name` and `cmd_show_options` in tmux 3.6).

use std::borrow::Cow;

use tmxr_command::Args;
use tmxr_config::{PANE_OPTIONS, WindowOptions};

use super::{Ctx, Outcome, Res, join_args};
use crate::model::{PaneId, SessionId, WindowId};
use crate::server::Server;
use crate::target;

/// Whose value an option command reads or changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Global,
    Window(WindowId),
    Pane(PaneId),
}

/// Whether `name` is kept by scope here: a window option or a user option.
pub(super) fn handles(name: &str) -> bool {
    name.starts_with('@') || WindowOptions::NAMES.contains(&name)
}

/// The scope a command's flags pick, as tmux's: `-p` a pane (for the options
/// a pane may hold), `-g` the global table, else the target window. A user
/// option without `-w` (or `set-window-option`) is a session's in tmux; tmxr
/// keeps those globally. `name` is `None` when listing.
fn scope(
    srv: &Server,
    ctx: &Ctx,
    a: &Args,
    window_cmd: bool,
    name: Option<&str>,
) -> Result<Scope, String> {
    let user = name.is_none_or(|n| n.starts_with('@'));
    let pane_scoped = name.is_none_or(|n| user || PANE_OPTIONS.contains(&n));
    if a.has('p') && pane_scoped {
        return Ok(Scope::Pane(target::pane(srv, ctx, a.value('t'))?.2));
    }
    if a.has('g') || (user && name.is_some() && !window_cmd && !a.has('w')) {
        return Ok(Scope::Global);
    }
    Ok(Scope::Window(target::window(srv, ctx, a.value('t'))?.2))
}

/// The options in effect at `scope`.
fn effective(srv: &Server, scope: Scope) -> Cow<'_, WindowOptions> {
    match scope {
        Scope::Global => Cow::Borrowed(&srv.cfg.window),
        Scope::Window(w) => srv.window_options(w),
        Scope::Pane(p) => srv.pane_options(p),
    }
}

/// `name`'s value set at `scope` itself; every global window option is.
fn own(srv: &Server, scope: Scope, name: &str) -> Option<String> {
    match scope {
        Scope::Global if name.starts_with('@') => srv.cfg.options.get(name).cloned(),
        Scope::Global => srv.cfg.window.show(name),
        Scope::Window(w) => srv.windows.get(&w)?.options.show(name),
        Scope::Pane(p) => srv.panes.get(&p)?.options.show(name),
    }
}

/// The scope `scope` inherits from: a pane's window, a window's globals.
fn parent(srv: &Server, scope: Scope) -> Option<Scope> {
    match scope {
        Scope::Global => None,
        Scope::Window(_) => Some(Scope::Global),
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
    if name == "allow-passthrough"
        && !unset
        && !tmxr_term::emulator::passthrough_supported()
        && super::on_off(value, effective(srv, scope).allow_passthrough)?
    {
        return Err(crate::server::PASSTHROUGH_UNSUPPORTED.into());
    }
    let append = a.has('a');
    let user = name.starts_with('@');
    match scope {
        Scope::Global if unset && user => {
            srv.cfg.options.remove(name);
        }
        // As tmux's, a global option goes back to its default.
        Scope::Global if unset => {
            let default = tmxr_config::defaults().window.show(name);
            srv.cfg.window.set(name, default.as_deref(), false)?;
        }
        Scope::Global if user => {
            let value = value.ok_or("empty value")?;
            let v = srv.cfg.options.entry(name.to_owned()).or_default();
            if !append {
                v.clear();
            }
            v.push_str(value);
        }
        Scope::Global => srv.cfg.window.set(name, value, append)?,
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
        Scope::Global => srv.windows.keys().copied().collect(),
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
/// a window or pane (`-w`, `-p`, `show-window-options`): its own values,
/// with `-A` the inherited ones too, marked `*`.
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
        // As tmux's: an unset user option is an error, an unset window
        // option prints nothing.
        if !print(out, name) && name.starts_with('@') {
            return Err(format!("invalid option: {name}"));
        }
        return Ok(());
    }
    let users: Vec<String> = match scope {
        Scope::Global => Vec::new(),
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
    let names = WindowOptions::NAMES
        .iter()
        .filter(|n| !matches!(scope, Scope::Pane(_)) || PANE_OPTIONS.contains(n));
    for n in names {
        print(out, n);
    }
    Ok(())
}
