//! Option and config commands.

use std::fmt::Write as _;
use std::path::PathBuf;

use tmxr_command::Parsed;
use tmxr_config::{OptionValue, SessionOptions, WindowOptions};

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
            let (name, index) = match name {
                // A malformed index went to `scoped`, which refuses it.
                Some(arg) => {
                    scoped::split_index(arg).map_or((Some(arg), None), |(n, i)| (Some(n), i))
                }
                None => (None, None),
            };
            let lines = server_option_lines(srv, name, index);
            if let Some(n) = pos.first().filter(|_| lines.is_empty() && !a.has('q')) {
                return Err(format!("invalid option: {n}"));
            }
            for line in lines {
                let text = if a.has('v') {
                    // An empty array's bare name has no value to print.
                    let Some(value) = line.value else {
                        continue;
                    };
                    value
                } else {
                    line.text()
                };
                out.stdout.push_str(&text);
                out.stdout.push('\n');
            }
        }
        "source-file" => {
            // With no path, the config file the server reads.
            if pos.is_empty() {
                let file = srv
                    .cfg_path
                    .clone()
                    .or_else(|| match tmxr_config::load(None) {
                        Ok((_, tmxr_config::ConfigSource::File(p))) => Some(p),
                        _ => None,
                    })
                    .ok_or("no config file to source")?;
                // -n: only check that it loads, changing nothing, as tmux's
                // parse-only source-file.
                if a.has('n') {
                    tmxr_config::load(Some(&file)).map_err(|e| e.to_string())?;
                } else {
                    srv.overlay_config(&file)?;
                    if let Some(c) = attached_client(srv, ctx) {
                        srv.show_message(c, "config reloaded".into());
                    }
                }
                return Ok(true);
            }
            // Each path (a format with -F) is a glob, as tmux's; each file
            // it names goes over what is running in turn, and an error in
            // one leaves the others to run. Which file is the config does
            // not change, as in tmux.
            let mut files = Vec::new();
            let mut errors = Vec::new();
            for p in pos {
                let p = if a.has('F') {
                    expand_for(srv, ctx, ctx.pane, p)
                } else {
                    p.clone()
                };
                match glob::glob(&p) {
                    Ok(found) => {
                        let found: Vec<PathBuf> = found.filter_map(Result::ok).collect();
                        // -q: a path matching nothing is no error, as tmux's.
                        if found.is_empty() && !a.has('q') {
                            errors.push(format!("{p}: No such file or directory"));
                        }
                        files.extend(found);
                    }
                    Err(e) => errors.push(format!("{p}: {e}")),
                }
            }
            for file in &files {
                // A `.toml` file is a tmxr config; any other, tmux commands.
                let toml = file.extension().is_some_and(|e| e == "toml");
                if !toml {
                    if !a.has('n') {
                        errors.extend(super::source::run_command_file(
                            srv,
                            ctx,
                            file,
                            &mut out.stdout,
                        ));
                    }
                    continue;
                }
                let done = if a.has('n') {
                    tmxr_config::load(Some(file))
                        .map(drop)
                        .map_err(|e| e.to_string())
                } else {
                    srv.overlay_config(file)
                };
                if let Err(e) = done {
                    errors.push(e);
                } else if a.has('v') {
                    // -v: what the file set, as the commands that would set
                    // it, as tmux's shows a file's commands.
                    for line in applied(srv, file) {
                        let _ = writeln!(out.stdout, "{}: {line}", file.display());
                    }
                }
            }
            if !errors.is_empty() {
                return Err(errors.join("; "));
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
    let arg = name;
    // A malformed index went to `scoped`, which refuses it.
    let (name, index) = scoped::split_index(arg).unwrap_or((arg, None));
    if !SERVER_OPTIONS.contains(&name) {
        // -q: an unknown option is no error, as tmux's.
        return if a.has('q') {
            Ok(())
        } else {
            Err(format!("invalid option: {arg}"))
        };
    }
    if name == "terminal-features" {
        return set_terminal_features(srv, a, (arg, index), value);
    }
    if index.is_some() {
        return Err(format!("not an array: {arg}"));
    }
    // A server option always has a value, so -o refuses every one.
    if a.has('o') {
        return Err(format!("already set: {name}"));
    }
    // -u: back to the built-in value; -a: after what a string option
    // holds (any other takes the value alone), as tmux's.
    let changed = if a.has('u') {
        config_option(&tmxr_config::defaults(), name)
    } else if a.has('a') && STRING_SERVER_OPTIONS.contains(&name) {
        value.map(|v| server_option(srv, name).unwrap_or_default() + v)
    } else {
        value.map(str::to_owned)
    };
    let value = changed.as_deref();
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
        "extended-keys-format" => match need()? {
            v @ ("csi-u" | "xterm") => v.clone_into(&mut c.extended_keys_format),
            v => {
                return Err(format!(
                    "extended-keys-format: unknown value: {v} (csi-u or xterm)"
                ));
            }
        },
        "kitty-keys" => c.kitty_keys = super::on_off(value, c.kitty_keys)?,
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
    match name {
        "rgb-colour" => srv.apply_rgb_colour(),
        "kitty-keys" => {
            srv.apply_kitty_keys();
            srv.decide_keyboards();
        }
        _ => {}
    }
    srv.mark_all_dirty();
    Ok(())
}

/// `set-option` for `terminal-features` (`arg` as given, with its
/// `index`), as tmux's for an array: whole, the value split at commas
/// replaces it (`-a` adds to it) and `-u` gives back the default; an item,
/// the value sets it (`-a` appends to it) and `-u` removes it. Each
/// attached client's keyboard tier follows at once.
fn set_terminal_features(
    srv: &mut Server,
    a: &tmxr_command::Args,
    (arg, index): (&str, Option<u32>),
    value: Option<&str>,
) -> Res {
    let features = &mut srv.cfg.terminal_features;
    // As tmux's: an option is always set as a whole, an item where it is.
    let set = index.is_none_or(|i| features.items().is_some_and(|items| items.contains_key(&i)));
    if !a.has('u') && a.has('o') && set {
        return Err(format!("already set: {arg}"));
    }
    let append = a.has('a');
    *features = match (index, a.has('u')) {
        (None, true) => tmxr_config::defaults().terminal_features,
        (Some(i), true) => features.set_item(i, None, false)?,
        (index, false) => {
            let value = value.ok_or("empty value")?;
            match index {
                Some(i) => features.set_item(i, Some(value), append)?,
                None => {
                    let mut array = if append {
                        features.clone()
                    } else {
                        tmxr_config::Array::default()
                    };
                    array.assign_split(value, tmxr_config::TERMINAL_FEATURES_SEPARATORS);
                    array
                }
            }
        }
    };
    srv.decide_keyboards();
    srv.mark_all_dirty();
    Ok(())
}

/// The server options `set-option` takes, as tmux's server table holds
/// them (and tmxr's `kitty-keys`, `rgb-colour` and `socket-access`).
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
    "extended-keys-format",
    "focus-events",
    "history-file",
    "kitty-keys",
    "message-limit",
    "prefix-timeout",
    "prompt-history-limit",
    "rgb-colour",
    "set-clipboard",
    "socket-access",
    "terminal-features",
];

/// A flag as `show-options` prints it.
const fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

/// What the config file at `path` sets, as the commands that set it to what
/// the server now has: `set-option -g`, `bind-key` / `unbind-key` and
/// `set-hook -g` / `-gu`.
fn applied(srv: &Server, path: &std::path::Path) -> Vec<String> {
    let Ok((_, sets, _)) = tmxr_config::load_file(path) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for name in sets.options.iter().chain(&sets.user_options) {
        // An array as a whole, its items joined at the commas a whole
        // value splits at.
        let array = (name == "terminal-features").then(|| {
            let items: Vec<&str> = srv.cfg.terminal_features.values().collect();
            tmxr_command::escape_arg(&items.join(","))
        });
        let value = array
            .or_else(|| {
                let mut lines = server_option_lines(srv, Some(name), None);
                lines
                    .pop()
                    .map(Line::text)?
                    .strip_prefix(&format!("{name} "))
                    .map(str::to_owned)
            })
            .or_else(|| {
                let value = srv
                    .cfg
                    .session
                    .show(name)
                    .or_else(|| srv.cfg.window.show(name))
                    .or_else(|| srv.cfg.options.get(name).cloned())?;
                Some(scoped::shown(name, value))
            });
        if let Some(value) = value {
            lines.push(format!("set-option -g {name} {value}"));
        }
    }
    for (table, key) in &sets.binds {
        let Ok(k) = key.parse::<tmxr_command::BindKey>() else {
            continue;
        };
        lines.push(match srv.keys.get(table, &k) {
            Some(spec) => super::binds::bind_line(table, &k, spec),
            None => format!(
                "unbind-key -T {table} {}",
                tmxr_command::join_args(std::slice::from_ref(key))
            ),
        });
    }
    for name in &sets.hooks {
        match srv.hooks.get(name) {
            Some(commands) => lines.extend(
                commands
                    .iter()
                    .map(|c| format!("set-hook -g {name} {}", tmxr_command::escape_arg(c))),
            ),
            None => lines.push(format!("set-hook -gu {name}")),
        }
    }
    lines
}

/// A server option's value, as a format reads it: unquoted.
pub fn server_option(srv: &Server, name: &str) -> Option<String> {
    config_option(&srv.cfg, name)
}

/// The server options tmux types as strings: `-a` appends to them (an
/// array's own `-a` is [`set_terminal_features`]'s), and `show-options`
/// quotes them.
const STRING_SERVER_OPTIONS: &[&str] = &[
    "copy-command",
    "default-terminal",
    "editor",
    "history-file",
    "terminal-features",
];

/// Server option `name`'s value in `c`, unquoted.
fn config_option(c: &tmxr_config::Config, name: &str) -> Option<String> {
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
        "extended-keys-format" => c.extended_keys_format.clone(),
        "focus-events" => on_off(c.focus_events).to_owned(),
        "history-file" => c.history_file.clone(),
        "kitty-keys" => on_off(c.kitty_keys).to_owned(),
        "message-limit" => c.message_limit.to_string(),
        "prefix-timeout" => c.prefix_timeout.to_string(),
        "prompt-history-limit" => c.prompt_history_limit.to_string(),
        "rgb-colour" => c.rgb_colour.clone(),
        "set-clipboard" => c.set_clipboard.clone(),
        "socket-access" => c.socket_access.clone(),
        // The items joined by spaces, as tmux's `options_to_string`.
        "terminal-features" => c.terminal_features.show(),
        _ => return None,
    })
}

/// One line `show-options -s` prints: its label, a name or an array's
/// `name[index]`, and its value, unquoted; `None` for an empty array,
/// printed as its bare name.
struct Line {
    label: String,
    name: &'static str,
    value: Option<String>,
}

impl Line {
    /// The line as printed: the value quoted as tmux's where tmux types the
    /// option a string.
    fn text(self) -> String {
        let Some(value) = self.value else {
            return self.label;
        };
        let value = if STRING_SERVER_OPTIONS.contains(&self.name) {
            tmxr_command::escape_arg(&value)
        } else {
            value
        };
        format!("{} {value}", self.label)
    }
}

/// The lines `show-options -s` prints, as tmux's `cmd_show_options_print`:
/// every server option's, or `name`'s alone (none for a name that is no
/// server option). An array's are one per item, `name[index] value`, or its
/// bare name when it has none; with `index`, that item's alone, empty where
/// it is not set. An index on an option that is not an array is shown and
/// ignored, as tmux's.
fn server_option_lines(srv: &Server, name: Option<&str>, index: Option<u32>) -> Vec<Line> {
    let mut lines = Vec::new();
    for &option in SERVER_OPTIONS {
        if name.is_some_and(|n| n != option) {
            continue;
        }
        let label = |i: u32| format!("{option}[{i}]");
        let line = |label, value| Line {
            label,
            name: option,
            value,
        };
        match (option, index) {
            ("terminal-features", Some(i)) => {
                let items = srv.cfg.terminal_features.items();
                let value = items.and_then(|items| items.get(&i)).cloned();
                lines.push(line(label(i), Some(value.unwrap_or_default())));
            }
            ("terminal-features", None) => {
                let items = srv.cfg.terminal_features.items().into_iter().flatten();
                let before = lines.len();
                lines.extend(items.map(|(&i, v)| line(label(i), Some(v.clone()))));
                if lines.len() == before {
                    lines.push(line(option.to_owned(), None));
                }
            }
            (_, index) => {
                let value = server_option(srv, option).unwrap_or_default();
                let label = index.map_or_else(|| option.to_owned(), label);
                lines.push(line(label, Some(value)));
            }
        }
    }
    lines
}

/// Every global option as `show-options` prints it, `name value`: the
/// server's, the sessions' and the windows', then the user options.
pub fn option_lines(srv: &Server) -> Vec<String> {
    let c = &srv.cfg;
    let mut all: Vec<String> = server_option_lines(srv, None, None)
        .into_iter()
        .map(Line::text)
        .collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_keys_is_a_server_option_shown_on_or_off() {
        assert!(SERVER_OPTIONS.contains(&"kitty-keys"));
        let mut c = tmxr_config::defaults();
        assert_eq!(config_option(&c, "kitty-keys").as_deref(), Some("on"));
        c.kitty_keys = false;
        assert_eq!(config_option(&c, "kitty-keys").as_deref(), Some("off"));
    }

    /// `line`'s output on `srv`, or its error.
    fn run(srv: &mut Server, line: &str) -> Result<String, String> {
        let out = super::super::run_line(srv, &Ctx::default(), line);
        if out.status == 0 {
            Ok(out.stdout)
        } else {
            Err(out.stderr.trim_end().to_owned())
        }
    }

    /// `terminal-features` sets and shows as tmux's array: whole values
    /// split at commas, items by index, `-a`, `-u`, `-o` and `-v`.
    #[test]
    fn terminal_features_set_and_show_as_tmuxs_array() {
        let (mut srv, _rx) = crate::server::testing::with_client(None);
        let tmux = "terminal-features[0] xterm*:clipboard:ccolour:cstyle:focus:title\n\
                    terminal-features[1] screen*:title\n\
                    terminal-features[2] rxvt*:ignorefkeys\n";
        assert_eq!(run(&mut srv, "show -s terminal-features").unwrap(), tmux);
        assert!(run(&mut srv, "show -s").unwrap().contains(tmux));
        assert_eq!(
            run(&mut srv, "show -s terminal-features[1]").unwrap(),
            "terminal-features[1] screen*:title\n"
        );
        assert_eq!(
            run(&mut srv, "show -s terminal-features[7]").unwrap(),
            "terminal-features[7] ''\n"
        );
        run(
            &mut srv,
            "set -as terminal-features ',foo*:extkeys,a b:title'",
        )
        .unwrap();
        assert_eq!(
            run(&mut srv, "show -sv terminal-features").unwrap(),
            "xterm*:clipboard:ccolour:cstyle:focus:title\nscreen*:title\n\
             rxvt*:ignorefkeys\nfoo*:extkeys\na b:title\n"
        );
        assert_eq!(
            run(&mut srv, "show -s terminal-features[4]").unwrap(),
            "terminal-features[4] \"a b:title\"\n"
        );
        run(&mut srv, "set -s terminal-features 'x:extkeys,,y:RGB'").unwrap();
        run(&mut srv, "set -s terminal-features[5] z:title").unwrap();
        run(&mut srv, "set -as terminal-features[5] :focus").unwrap();
        run(&mut srv, "set -su terminal-features[0]").unwrap();
        assert_eq!(
            run(&mut srv, "show -s terminal-features").unwrap(),
            "terminal-features[1] y:RGB\nterminal-features[5] z:title:focus\n"
        );
        assert_eq!(
            run(&mut srv, "set -so terminal-features[5] w").unwrap_err(),
            "already set: terminal-features[5]"
        );
        run(&mut srv, "set -so terminal-features[6] w:title").unwrap();
        assert_eq!(
            run(&mut srv, "set -so terminal-features w").unwrap_err(),
            "already set: terminal-features"
        );
        assert_eq!(
            run(&mut srv, "set -s terminal-features").unwrap_err(),
            "empty value"
        );
        assert_eq!(
            run(&mut srv, "set -s buffer-limit[0] 5").unwrap_err(),
            "not an array: buffer-limit[0]"
        );
        // Emptied, it shows as its bare name, and nothing with -v.
        run(&mut srv, "set -s terminal-features ''").unwrap();
        assert_eq!(
            run(&mut srv, "show -s terminal-features").unwrap(),
            "terminal-features\n"
        );
        assert_eq!(run(&mut srv, "show -sv terminal-features").unwrap(), "");
        run(&mut srv, "set -su terminal-features").unwrap();
        assert_eq!(run(&mut srv, "show -s terminal-features").unwrap(), tmux);
    }
}
