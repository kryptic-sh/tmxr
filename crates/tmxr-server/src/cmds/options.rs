//! Option and config commands.

use std::path::PathBuf;

use tmxr_command::Parsed;
use tmxr_config::{SessionOptions, WindowOptions};

use super::{Ctx, Outcome, Res, attached_client, expand_for, scoped};
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
        "set-option" | "set-window-option" => set_option(srv, ctx, p)?,
        "show-options" | "show-window-options" => {
            let window_cmd = p.name() == "show-window-options";
            let name = pos.first().map(String::as_str);
            let scoped = match name {
                Some(n) => scoped::handles(n),
                None => !a.has('s'),
            };
            if scoped {
                return scoped::show(srv, ctx, a, window_cmd, name, out).map(|()| true);
            }
            // The rest are server options: one table, as tmux's.
            let mut found = false;
            for l in server_option_lines(srv) {
                if name.is_none_or(|n| l.split(' ').next() == Some(n)) {
                    found = true;
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
            if let Some(n) = name.filter(|_| !found && !a.has('q')) {
                return Err(format!("invalid option: {n}"));
            }
        }
        "source-file" => {
            // -F: the path is a format, expanded first.
            let path = pos.first().map(|p| {
                PathBuf::from(if a.has('F') {
                    expand_for(srv, ctx, ctx.pane, p)
                } else {
                    p.clone()
                })
            });
            // -q: a file that is not there is no error, as tmux's.
            if a.has('q') && path.as_ref().is_some_and(|p| !p.exists()) {
                return Ok(true);
            }
            // -n: only check that the file loads (else the config's own),
            // changing nothing, as tmux's parse-only source-file.
            if a.has('n') {
                let check = path.as_deref().or(srv.cfg_path.as_deref());
                tmxr_config::load(check).map_err(|e| e.to_string())?;
                return Ok(true);
            }
            if let Some(path) = path {
                // One that does not load changes nothing, the config path
                // included, so the next reload still finds the old file.
                tmxr_config::load(Some(&path)).map_err(|e| e.to_string())?;
                srv.cfg_path = Some(path);
            }
            srv.reload_config()?;
            if let Some(c) = attached_client(srv, ctx) {
                srv.show_message(c, "config reloaded".into());
            }
        }

        _ => return Ok(false),
    }
    Ok(true)
}

fn set_option(srv: &mut Server, ctx: &Ctx, p: &Parsed) -> Res {
    let a = &p.args;
    let pos = a.positional();
    let name = pos[0].as_str();
    // -F: the value is a format, expanded first for the target (else the
    // current pane), as tmux's.
    let expanded = pos.get(1).filter(|_| a.has('F')).map(|v| {
        let pane = target::pane(srv, ctx, a.value('t')).ok().map(|(_, _, p)| p);
        super::expand_for(srv, ctx, pane, v)
    });
    let value = expanded
        .as_deref()
        .or_else(|| pos.get(1).map(String::as_str));
    if scoped::handles(name) {
        return scoped::set(srv, ctx, a, p.name() == "set-window-option", name, value);
    }
    if !SERVER_OPTIONS.contains(&name) {
        // -q: an unknown option is no error, as tmux's.
        return if a.has('q') {
            Ok(())
        } else {
            Err(format!("invalid option: {name}"))
        };
    }
    // A server option always has a value, so -o refuses every one.
    if a.has('o') {
        return Err(format!("already set: {name}"));
    }
    let need = || value.ok_or_else(|| format!("{name}: needs a value"));
    let num = |v: &str| {
        v.parse::<u64>()
            .map_err(|_| format!("{name}: bad number {v}"))
    };
    let c = &mut srv.cfg;
    match name {
        "prompt-history-limit" => c.prompt_history_limit = num(need()?)? as usize,
        // As tmux's: at least one buffer, any number of messages.
        "buffer-limit" => match num(need()?)? {
            0 => return Err("buffer-limit: value is too small".into()),
            n => c.buffer_limit = n as usize,
        },
        "message-limit" => c.message_limit = num(need()?)? as usize,
        "prefix-timeout" => c.prefix_timeout = num(need()?)?,
        "exit-empty" => c.exit_empty = super::on_off(value, c.exit_empty)?,
        "exit-unattached" => c.exit_unattached = super::on_off(value, c.exit_unattached)?,
        "client-away-time" => c.client_away_time = num(need()?)?,
        "escape-time" => c.escape_time = num(need()?)?,
        "focus-events" => c.focus_events = super::on_off(value, c.focus_events)?,
        "default-terminal" => need()?.clone_into(&mut c.default_terminal),
        "extended-keys" => need()?.clone_into(&mut c.extended_keys),
        "set-clipboard" => need()?.clone_into(&mut c.set_clipboard),
        "rgb-colour" => match need()? {
            v @ ("on" | "off" | "16" | "auto") => v.clone_into(&mut c.rgb_colour),
            v => {
                return Err(format!(
                    "rgb-colour: unknown value: {v} (on, off, 16 or auto)"
                ));
            }
        },
        "copy-command" => need()?.clone_into(&mut c.copy_command),
        "editor" => need()?.clone_into(&mut c.editor),
        "history-file" => need()?.clone_into(&mut c.history_file),
        "socket-access" => {
            return Err(
                "socket-access is read when the server starts: set it in the config file".into(),
            );
        }
        _ => return Err(format!("invalid option: {name}")),
    }
    if name == "rgb-colour" {
        srv.apply_rgb_colour();
    }
    srv.mark_all_dirty();
    Ok(())
}

/// The server options `set-option` takes, as tmux's server table holds
/// them (and tmxr's `rgb-colour` and `socket-access`).
pub const SERVER_OPTIONS: &[&str] = &[
    "buffer-limit",
    "client-away-time",
    "copy-command",
    "default-terminal",
    "editor",
    "escape-time",
    "exit-empty",
    "exit-unattached",
    "extended-keys",
    "focus-events",
    "history-file",
    "message-limit",
    "prefix-timeout",
    "prompt-history-limit",
    "rgb-colour",
    "set-clipboard",
    "socket-access",
];

/// A flag as `show-options` prints it.
const fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

/// A server option's value, as a format reads it: unquoted.
pub fn server_option(srv: &Server, name: &str) -> Option<String> {
    let c = &srv.cfg;
    Some(match name {
        "buffer-limit" => c.buffer_limit.to_string(),
        "client-away-time" => c.client_away_time.to_string(),
        "copy-command" => c.copy_command.clone(),
        "default-terminal" => c.default_terminal.clone(),
        "editor" => c.editor.clone(),
        "escape-time" => c.escape_time.to_string(),
        "exit-empty" => on_off(c.exit_empty).to_owned(),
        "exit-unattached" => on_off(c.exit_unattached).to_owned(),
        "extended-keys" => c.extended_keys.clone(),
        "focus-events" => on_off(c.focus_events).to_owned(),
        "history-file" => c.history_file.clone(),
        "message-limit" => c.message_limit.to_string(),
        "prefix-timeout" => c.prefix_timeout.to_string(),
        "prompt-history-limit" => c.prompt_history_limit.to_string(),
        "rgb-colour" => c.rgb_colour.clone(),
        "set-clipboard" => c.set_clipboard.clone(),
        "socket-access" => c.socket_access.clone(),
        _ => return None,
    })
}

/// Each server option as `show-options -s` prints it: `name value`, the
/// ones tmux types as strings quoted as its.
fn server_option_lines(srv: &Server) -> Vec<String> {
    SERVER_OPTIONS
        .iter()
        .map(|&name| {
            let value = server_option(srv, name).unwrap_or_default();
            let value = match name {
                "copy-command" | "default-terminal" | "editor" | "history-file" => {
                    tmxr_command::escape_arg(&value)
                }
                _ => value,
            };
            format!("{name} {value}")
        })
        .collect()
}

/// Every global option as `show-options` prints it, `name value`: the
/// server's, the sessions' and the windows', then the user options.
pub fn option_lines(srv: &Server) -> Vec<String> {
    let c = &srv.cfg;
    let mut all = server_option_lines(srv);
    all.extend(
        SessionOptions::NAMES
            .iter()
            .filter_map(|n| Some(format!("{n} {}", scoped::shown(n, c.session.show(n)?)))),
    );
    all.extend(
        WindowOptions::NAMES
            .iter()
            .filter_map(|n| Some(format!("{n} {}", scoped::shown(n, c.window.show(n)?)))),
    );
    all.extend(
        c.options
            .iter()
            .map(|(k, v)| format!("{k} {}", scoped::shown(k, v.clone()))),
    );
    all
}
