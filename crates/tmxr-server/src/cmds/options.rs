//! Option and config commands.

use std::path::PathBuf;

use tmxr_command::Parsed;

use super::{Ctx, Outcome, Res, attached_client, join_args, on_off};
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
                format!("status-left-length {}", c.status.left_length),
                format!("status-right-length {}", c.status.right_length),
                format!("repeat-time {}", c.repeat_time),
                format!("default-terminal {}", c.default_terminal),
                format!("extended-keys {}", c.extended_keys),
                format!("set-clipboard {}", c.set_clipboard),
                format!(
                    "allow-passthrough {}",
                    if c.allow_passthrough { "on" } else { "off" }
                ),
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

        _ => return Ok(false),
    }
    Ok(true)
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
    let length = |v: &str| {
        num(v).and_then(|n| u16::try_from(n).map_err(|_| format!("{name}: too large: {v}")))
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
        "mode-keys" => match need()? {
            v @ ("vi" | "emacs") => v.clone_into(&mut c.mode_keys),
            v => return Err(format!("mode-keys: unknown value: {v} (vi or emacs)")),
        },
        "default-terminal" => need()?.clone_into(&mut c.default_terminal),
        "default-shell" => c.default_shell = Some(need()?.to_owned()),
        "extended-keys" => need()?.clone_into(&mut c.extended_keys),
        "set-clipboard" => need()?.clone_into(&mut c.set_clipboard),
        "copy-command" => need()?.clone_into(&mut c.copy_command),
        "remain-on-exit" => c.remain_on_exit = on_off(value, c.remain_on_exit)?,
        "allow-passthrough" => {
            let on = on_off(value, c.allow_passthrough)?;
            if on && !tmxr_term::emulator::passthrough_supported() {
                return Err(crate::server::PASSTHROUGH_UNSUPPORTED.into());
            }
            c.allow_passthrough = on;
        }
        "status-left" => need()?.clone_into(&mut c.status.left),
        "status-right" => need()?.clone_into(&mut c.status.right),
        "status-left-length" => c.status.left_length = length(need()?)?,
        "status-right-length" => c.status.right_length = length(need()?)?,
        "status-style" => need()?.clone_into(&mut c.status.style),
        "window-status-format" => need()?.clone_into(&mut c.status.window_format),
        "window-status-current-format" => need()?.clone_into(&mut c.status.window_current_format),
        "pane-border-style" => need()?.clone_into(&mut c.status.pane_border_style),
        "pane-active-border-style" => need()?.clone_into(&mut c.status.pane_active_border_style),
        "message-style" => need()?.clone_into(&mut c.status.message_style),
        "mode-style" => need()?.clone_into(&mut c.status.mode_style),
        "clock-mode-colour" => need()?.clone_into(&mut c.status.clock_mode_colour),
        "copy-mode-match-style" => need()?.clone_into(&mut c.status.copy_mode_match_style),
        "copy-mode-current-match-style" => {
            need()?.clone_into(&mut c.status.copy_mode_current_match_style);
        }
        _ => return Err(format!("invalid option: {name}")),
    }
    srv.mark_all_dirty();
    Ok(())
}
