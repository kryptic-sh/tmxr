//! A tmux.conf as tmxr's TOML (`tmxr --import-tmux-conf`): its global
//! `set-option`, `bind-key` / `unbind-key` and `set-hook` lines become the
//! config file's keys, typed as tmxr's defaults type them, and every other
//! line is listed as left out, with why. The result is meant to be read and
//! edited, then saved as `~/.config/tmxr/config.toml`.

use tmxr_command::{join_args, tokenize};
use toml::{Table, Value};

use crate::DEFAULTS;

/// What [`from_tmux_conf`] made of a file.
#[derive(Debug, Default)]
pub struct Imported {
    /// The config file's text.
    pub toml: String,
    /// Each line left out, and why.
    pub skipped: Vec<String>,
}

/// Convert tmux.conf `text`; `env` expands its `$VAR`s and `~`, as tmux
/// does when it reads the file.
pub fn from_tmux_conf(text: &str, env: &dyn Fn(&str) -> Option<String>) -> Imported {
    let defaults: Table = DEFAULTS.parse().expect("the embedded defaults parse");
    let mut out = Out {
        defaults,
        file: Table::new(),
        skipped: Vec::new(),
    };
    for line in logical_lines(text) {
        let commands = match tokenize(&line, env) {
            Ok(c) => c,
            Err(e) => {
                out.skip(&line, &e.to_string());
                continue;
            }
        };
        for words in commands {
            out.command(&line, &words);
        }
    }
    Imported {
        toml: toml::to_string(&out.file).expect("a table of plain values serializes"),
        skipped: out.skipped,
    }
}

/// The file's lines as tmux reads them: a `\` at a line's end joins the
/// next line to it, and a `{ … }` block may run over several.
fn logical_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut pending = String::new();
    for raw in text.lines() {
        if let Some(head) = raw.strip_suffix('\\') {
            pending.push_str(head);
            continue;
        }
        pending.push_str(raw);
        let open = matches!(
            tokenize(&pending, &|_| None),
            Err(tmxr_command::tokenize::TokenizeError::UnterminatedBlock)
        );
        if open {
            pending.push('\n');
        } else {
            lines.push(std::mem::take(&mut pending));
        }
    }
    if !pending.trim().is_empty() {
        lines.push(pending);
    }
    lines
}

struct Out {
    defaults: Table,
    file: Table,
    skipped: Vec<String>,
}

impl Out {
    fn skip(&mut self, line: &str, why: &str) {
        self.skipped.push(format!("{}: {why}", line.trim()));
    }

    fn command(&mut self, line: &str, words: &[String]) {
        let parsed = match tmxr_command::parse(words) {
            Ok(p) => p,
            Err(e) => return self.skip(line, &e.to_string()),
        };
        let a = &parsed.args;
        let pos = a.positional();
        let result = match parsed.name() {
            "set-option" | "set-window-option" => self.set(a, pos),
            "bind-key" => self.bind(a, pos),
            "unbind-key" => self.unbind(a, pos),
            "set-hook" => self.hook(a, pos),
            name => Err(format!("{name} is run, not configured: left out")),
        };
        if let Err(why) = result {
            self.skip(line, &why);
        }
    }

    /// `set-option`: a global option into its key, typed as its default.
    fn set(&mut self, a: &tmxr_command::Args, pos: &[String]) -> Result<(), String> {
        if a.value('t').is_some() || a.has('p') {
            return Err("a session's, window's or pane's own option: left out".into());
        }
        if a.has('u') || a.has('U') {
            return Err("unsetting gives back the default: left out".into());
        }
        let name = pos[0].as_str();
        let value = pos.get(1).map(String::as_str);
        if name.starts_with('@') {
            let value = value.ok_or("a user option needs a value")?;
            let options = table(&mut self.file, "options");
            let old = options.get(name).and_then(Value::as_str).unwrap_or("");
            let new = if a.has('a') {
                format!("{old}{value}")
            } else {
                value.to_owned()
            };
            options.insert(name.into(), Value::String(new));
            return Ok(());
        }
        let (section, key) =
            file_key(&self.defaults, name).ok_or_else(|| format!("tmxr has no {name} option"))?;
        let default = match section {
            Some(s) => &self.defaults[s][key],
            None => &self.defaults[key],
        };
        let current = match section {
            Some(s) => self.file.get(s).and_then(|t| t.get(key)),
            None => self.file.get(key),
        }
        .unwrap_or(default)
        .clone();
        let new = typed(&current, value, a.has('a'))?;
        let place = match section {
            Some(s) => table(&mut self.file, s),
            None => &mut self.file,
        };
        place.insert(key.into(), new);
        Ok(())
    }

    /// `bind-key`: `[keys.<table>]`, the key, its command list as written.
    fn bind(&mut self, a: &tmxr_command::Args, pos: &[String]) -> Result<(), String> {
        let table_name = key_table(a);
        let key = pos[0].clone();
        let cmd = command_line(&pos[1..]).ok_or("a bind with no command: left out")?;
        let mut bind = Table::new();
        bind.insert("cmd".into(), Value::String(cmd));
        if let Some(note) = a.value('N') {
            bind.insert("note".into(), Value::String(note.into()));
        }
        if a.has('r') {
            bind.insert("repeat".into(), Value::Boolean(true));
        }
        table(table(&mut self.file, "keys"), &table_name).insert(key, Value::Table(bind));
        Ok(())
    }

    /// `unbind-key`: the key set to `false`, which removes a default bind.
    fn unbind(&mut self, a: &tmxr_command::Args, pos: &[String]) -> Result<(), String> {
        if a.has('a') {
            return Err("unbinding a whole table cannot be written as keys: left out".into());
        }
        let key = pos.first().ok_or("unbind-key needs a key")?.clone();
        let table_name = key_table(a);
        table(table(&mut self.file, "keys"), &table_name).insert(key, Value::Boolean(false));
        Ok(())
    }

    /// `set-hook -g`: `[hooks]`, the command list (`-a` adds to it).
    fn hook(&mut self, a: &tmxr_command::Args, pos: &[String]) -> Result<(), String> {
        if !a.has('g') || a.has('p') || a.has('w') || a.value('t').is_some() {
            return Err("only global hooks (set-hook -g) are configured: left out".into());
        }
        if a.has('u') || a.has('R') {
            return Err("unsetting or running a hook: left out".into());
        }
        let name = pos[0].clone();
        let cmd = command_line(&pos[1..]).ok_or("a hook with no command: left out")?;
        let hooks = table(&mut self.file, "hooks");
        let mut cmds = match hooks.get(&name) {
            Some(Value::Array(old)) if a.has('a') => old.clone(),
            _ => Vec::new(),
        };
        cmds.push(Value::String(cmd));
        hooks.insert(name, Value::Array(cmds));
        Ok(())
    }
}

/// The key table a bind or unbind is for: `-T`, `-n` for root, else prefix.
fn key_table(a: &tmxr_command::Args) -> String {
    match a.value('T') {
        Some(t) => t.to_owned(),
        None if a.has('n') => "root".into(),
        None => "prefix".into(),
    }
}

/// A command given as words after a bind's key or a hook's name, as one
/// command line: one word is already a command list (a quoted string or a
/// `{ … }` block); more are quoted again, a `;` among them kept as the
/// separator it was (`\;`).
fn command_line(words: &[String]) -> Option<String> {
    match words {
        [] => None,
        [one] => Some(one.clone()),
        many => Some(
            many.split(|w| w == ";")
                .map(join_args)
                .collect::<Vec<_>>()
                .join(" ; "),
        ),
    }
}

/// `table`'s sub-table `name`, made if it is not there.
fn table<'t>(table: &'t mut Table, name: &str) -> &'t mut Table {
    let entry = table
        .entry(name.to_owned())
        .or_insert_with(|| Value::Table(Table::new()));
    if !entry.is_table() {
        *entry = Value::Table(Table::new());
    }
    entry.as_table_mut().expect("made a table just now")
}

/// Where option `name` lives in the file: a top-level key, or one in
/// `[status]` (`status-left` is `left` there, `window-status-format` is
/// `window-format`), as the defaults have it.
fn file_key<'d>(defaults: &'d Table, name: &str) -> Option<(Option<&'static str>, &'d str)> {
    let plain = |t: &'d Table, k: &str| t.get_key_value(k).filter(|(_, v)| !v.is_table());
    if let Some((k, _)) = plain(defaults, name) {
        return Some((None, k.as_str()));
    }
    let status = defaults.get("status")?.as_table()?;
    let in_status = match name {
        "window-status-format" => "window-format",
        "window-status-current-format" => "window-current-format",
        n => n
            .strip_prefix("status-")
            .filter(|k| status.contains_key(*k))
            .unwrap_or(n),
    };
    plain(status, in_status).map(|(k, _)| (Some("status"), k.as_str()))
}

/// `value` for an option now `current` (the default when unset), typed as
/// it: a flag (no value toggles), a number, a string (`-a` appends), or a
/// list (words; `-a` adds them).
fn typed(current: &Value, value: Option<&str>, append: bool) -> Result<Value, String> {
    match current {
        Value::Boolean(b) => Ok(Value::Boolean(match value {
            None | Some("toggle") => !b,
            Some("on" | "1" | "yes" | "true") => true,
            Some("off" | "0" | "no" | "false") => false,
            Some(v) => return Err(format!("not on or off: {v}")),
        })),
        Value::Integer(_) => {
            let v = value.ok_or("needs a number")?;
            v.parse()
                .map(Value::Integer)
                .map_err(|_| format!("not a number: {v}"))
        }
        Value::Array(old) => {
            let v = value.ok_or("needs a value")?;
            let mut list = if append { old.clone() } else { Vec::new() };
            list.extend(v.split_whitespace().map(|w| Value::String(w.into())));
            Ok(Value::Array(list))
        }
        other => {
            let v = value.ok_or("needs a value")?;
            let old = other.as_str().unwrap_or("");
            Ok(Value::String(if append {
                format!("{old}{v}")
            } else {
                v.to_owned()
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn import(text: &str) -> (crate::Config, Vec<String>) {
        let imported = from_tmux_conf(text, &|k| (k == "HOME").then(|| "/home/u".into()));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, &imported.toml).unwrap();
        let (cfg, _) = crate::load(Some(&path))
            .unwrap_or_else(|e| panic!("{e}\n--- the file:\n{}", imported.toml));
        (cfg, imported.skipped)
    }

    #[test]
    fn options_go_to_their_keys_typed_as_the_defaults() {
        let (cfg, skipped) = import(
            "set -g prefix C-a\n\
             set -g mouse off\n\
             set -g history-limit 10000\n\
             set -g status-left '[#S] '\n\
             set -ga status-left 'x'\n\
             setw -g mode-keys emacs\n\
             set -g window-status-format '#I:#W'\n\
             set -g pane-active-border-style fg=red\n\
             set -g @thm_fg '#ffffff'\n\
             set -ga update-environment 'EXTRA'\n",
        );
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(&*cfg.session.prefix, "C-a");
        assert!(!cfg.session.mouse);
        assert_eq!(cfg.session.history_limit, 10000);
        assert_eq!(cfg.session.status_left, "[#S] x");
        assert_eq!(cfg.window.mode_keys, crate::ModeKeys::Emacs);
        assert_eq!(cfg.window.window_status_format, "#I:#W");
        assert_eq!(cfg.window.pane_active_border_style, "fg=red");
        assert_eq!(cfg.options["@thm_fg"], "#ffffff");
        // -a adds to the default list.
        let env: Vec<&str> = cfg.session.update_environment.values().collect();
        assert!(env.contains(&"DISPLAY"), "{env:?}");
        assert_eq!(env.last(), Some(&"EXTRA"));
    }

    #[test]
    fn binds_unbinds_and_hooks_become_keys() {
        let (cfg, skipped) = import(
            "unbind C-b\n\
             bind -r -N 'Resize' H resize-pane -L 5\n\
             bind -n M-h select-pane -L\n\
             bind -T copy-mode-vi v send -X begin-selection\n\
             bind x split-window -h \\; select-pane -L\n\
             bind y { display -p one\n  display -p two\n}\n\
             bind '|' split-window -h -c \"#{pane_current_path}\"\n\
             set-hook -g session-created 'new-window -d'\n\
             set-hook -ga session-created 'new-window -d'\n",
        );
        assert!(skipped.is_empty(), "{skipped:?}");
        let bind = |table: &str, key: &str| {
            cfg.binds()
                .find(|(t, k, _)| *t == table && *k == key)
                .map(|(_, _, b)| b.clone())
        };
        assert!(bind("prefix", "C-b").is_none());
        let h = bind("prefix", "H").unwrap();
        assert_eq!((h.cmd.as_str(), h.repeat), ("resize-pane -L 5", true));
        assert_eq!(h.note.as_deref(), Some("Resize"));
        assert_eq!(bind("root", "M-h").unwrap().cmd, "select-pane -L");
        assert_eq!(
            bind("copy-mode-vi", "v").unwrap().cmd,
            "send -X begin-selection"
        );
        assert_eq!(
            bind("prefix", "x").unwrap().cmd,
            "split-window -h ; select-pane -L"
        );
        // A block as tmux keeps it: printed back, a new line after ` ;; `.
        assert_eq!(
            bind("prefix", "y").unwrap().cmd,
            "display-message -p one ;; display-message -p two"
        );
        // A word with characters the tokenizer reads is quoted again, and
        // reads back the same.
        let pipe = bind("prefix", "|").unwrap().cmd;
        let words = tokenize(&pipe, &|_| None).unwrap();
        assert_eq!(
            words,
            [["split-window", "-h", "-c", "#{pane_current_path}"]]
        );
        assert_eq!(
            cfg.hooks["session-created"],
            ["new-window -d", "new-window -d"]
        );
    }

    #[test]
    fn what_cannot_be_configured_is_listed_with_why() {
        let (_, skipped) = import(
            "set -g @plugin 'tmux-plugins/tpm'\n\
             set -g terminal-overrides ',*:RGB'\n\
             run '~/.tmux/plugins/tpm/tpm'\n\
             set -t main status-left x\n\
             set -g status-left 'unterminated\n\
             # a comment\n\
             \n",
        );
        assert_eq!(skipped.len(), 4, "{skipped:#?}");
        assert!(skipped[0].contains("tmxr has no terminal-overrides option"));
        assert!(skipped[1].contains("run-shell is run, not configured"));
        assert!(skipped[2].contains("own option"));
        assert!(skipped[3].contains("unterminated"));
    }

    #[test]
    fn a_trailing_backslash_joins_lines() {
        let (cfg, skipped) = import("set -g status-right \\\n  'joined'\n");
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(cfg.session.status_right, "joined");
    }
}
