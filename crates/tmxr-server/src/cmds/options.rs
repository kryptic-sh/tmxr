//! Option and config commands.

use std::path::PathBuf;

use tmxr_command::Parsed;
use tmxr_config::{AlertAction, Visual, WindowOptions};

use super::{Ctx, Outcome, Res, attached_client, join_args, on_off, scoped};
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
                None => window_cmd || a.has('w') || a.has('p'),
            };
            if scoped {
                return scoped::show(srv, ctx, a, window_cmd, name, out).map(|()| true);
            }
            // The other options are server-wide: one table, as tmux's
            // global session and server options.
            let mut found = false;
            for l in option_lines(srv) {
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
            if let Some(n) = name.filter(|_| !found) {
                return Err(format!("invalid option: {n}"));
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
        "history-limit" => c.history_limit = num(need()?)? as usize,
        "prompt-history-limit" => c.prompt_history_limit = num(need()?)? as usize,
        "display-time" => c.display_time = num(need()?)?,
        "status-interval" => c.status_interval = num(need()?)?,
        "repeat-time" => c.repeat_time = num(need()?)?,
        "escape-time" => c.escape_time = num(need()?)?,
        "default-terminal" => need()?.clone_into(&mut c.default_terminal),
        "default-shell" => c.default_shell = Some(need()?.to_owned()),
        "extended-keys" => need()?.clone_into(&mut c.extended_keys),
        "set-clipboard" => need()?.clone_into(&mut c.set_clipboard),
        "rgb-colour" => match need()? {
            v @ ("on" | "off" | "auto") => v.clone_into(&mut c.rgb_colour),
            v => return Err(format!("rgb-colour: unknown value: {v} (on, off or auto)")),
        },
        "copy-command" => need()?.clone_into(&mut c.copy_command),
        "lock-command" => need()?.clone_into(&mut c.lock_command),
        "lock-after-time" => c.lock_after_time = num(need()?)?,
        "socket-access" => {
            return Err(
                "socket-access is read when the server starts: set it in the config file".into(),
            );
        }
        "bell-action" | "activity-action" | "silence-action" => {
            let v = need()?;
            let action = AlertAction::parse(v)
                .ok_or_else(|| format!("{name}: unknown value: {v} ({})", AlertAction::NAMES))?;
            match name {
                "bell-action" => c.bell_action = action,
                "activity-action" => c.activity_action = action,
                _ => c.silence_action = action,
            }
        }
        "visual-bell" | "visual-activity" | "visual-silence" => {
            let v = need()?;
            let visual = Visual::parse(v)
                .ok_or_else(|| format!("{name}: unknown value: {v} ({})", Visual::NAMES))?;
            match name {
                "visual-bell" => c.visual_bell = visual,
                "visual-activity" => c.visual_activity = visual,
                _ => c.visual_silence = visual,
            }
        }
        "status-left" => need()?.clone_into(&mut c.status.left),
        "status-right" => need()?.clone_into(&mut c.status.right),
        "status-left-length" => c.status.left_length = length(need()?)?,
        "status-right-length" => c.status.right_length = length(need()?)?,
        "status-style" => need()?.clone_into(&mut c.status.style),
        "message-style" => need()?.clone_into(&mut c.status.message_style),
        _ => return Err(format!("invalid option: {name}")),
    }
    if name == "rgb-colour" {
        srv.apply_rgb_colour();
    }
    srv.mark_all_dirty();
    Ok(())
}

/// Each option as `show-options` prints it: `name value`.
pub fn option_lines(srv: &Server) -> Vec<String> {
    let c = &srv.cfg;
    let lines = [
        format!("prefix {}", c.prefix),
        format!("mouse {}", if c.mouse { "on" } else { "off" }),
        format!("base-index {}", c.base_index),
        format!(
            "renumber-windows {}",
            if c.renumber_windows { "on" } else { "off" }
        ),
        format!("history-limit {}", c.history_limit),
        format!("prompt-history-limit {}", c.prompt_history_limit),
        format!("display-time {}", c.display_time),
        format!("status-interval {}", c.status_interval),
        format!("status-left-length {}", c.status.left_length),
        format!("status-right-length {}", c.status.right_length),
        format!("repeat-time {}", c.repeat_time),
        format!("default-terminal {}", c.default_terminal),
        format!("extended-keys {}", c.extended_keys),
        format!("set-clipboard {}", c.set_clipboard),
        format!("rgb-colour {}", c.rgb_colour),
        format!("bell-action {}", c.bell_action.as_str()),
        format!("activity-action {}", c.activity_action.as_str()),
        format!("silence-action {}", c.silence_action.as_str()),
        format!("visual-bell {}", c.visual_bell.as_str()),
        format!("visual-activity {}", c.visual_activity.as_str()),
        format!("visual-silence {}", c.visual_silence.as_str()),
        format!("lock-after-time {}", c.lock_after_time),
        format!(
            "lock-command {}",
            join_args(std::slice::from_ref(&c.lock_command))
        ),
    ];
    let mut all: Vec<String> = lines.to_vec();
    all.extend(
        WindowOptions::NAMES
            .iter()
            .filter_map(|n| Some(format!("{n} {}", c.window.show(n)?))),
    );
    all.extend(
        c.options
            .iter()
            .map(|(k, v)| format!("{k} {}", join_args(std::slice::from_ref(v)))),
    );
    all
}
