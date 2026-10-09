//! TOML configuration for tmxr.
//!
//! The built-in defaults ([`DEFAULTS`], `defaults.toml`) are the port of the
//! owner's tmux config. A user file at `~/.config/tmxr/config.toml` (or the
//! `-f` path) is deep-merged over them by `hjkl-config`: tables merge key by
//! key, everything else is replaced. A bind set to `false` removes the
//! default bind. See `docs/plan/10-config-and-commands.md`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

pub use hjkl_config::{ConfigError, ConfigSource};

/// The embedded defaults.
pub const DEFAULTS: &str = include_str!("../defaults.toml");

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Config {
    pub prefix: String,
    pub mouse: bool,
    pub base_index: u32,
    pub pane_base_index: u32,
    pub renumber_windows: bool,
    pub mode_keys: String,
    pub history_limit: usize,
    /// Entries kept per prompt type for Up/Down recall (tmux's default 100).
    pub prompt_history_limit: usize,
    pub escape_time: u64,
    pub display_time: u64,
    /// How long `display-panes` shows pane numbers, in milliseconds.
    pub display_panes_time: u64,
    pub status_interval: u64,
    pub repeat_time: u64,
    pub default_terminal: String,
    #[serde(default)]
    pub default_shell: Option<String>,
    pub extended_keys: String,
    pub set_clipboard: String,
    /// Forward programs' `ESC P tmux; … ESC \` passthrough to the outer
    /// terminal (inline images and the like), as tmux's `allow-passthrough`.
    pub allow_passthrough: bool,
    /// Keep a pane whose program exited, shown as dead, until it is
    /// respawned or killed (tmux's `remain-on-exit`).
    pub remain_on_exit: bool,
    /// Shell command `copy-pipe` sends the copied text to when it names none.
    pub copy_command: String,
    pub update_environment: Vec<String>,
    pub navigator: Navigator,
    pub status: Status,
    pub resurrect: Resurrect,
    /// `@name` user options.
    pub options: BTreeMap<String, String>,
    /// key table → key name → bind.
    pub keys: BTreeMap<String, BTreeMap<String, BindEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Navigator {
    pub pattern: String,
    pub disable_when_zoomed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Status {
    pub style: String,
    pub left: String,
    pub right: String,
    /// Most cells `left` may take (tmux's `status-left-length`).
    pub left_length: u16,
    /// Most cells `right` may take (tmux's `status-right-length`); a longer
    /// right side is cut at its end so the window list keeps its room.
    pub right_length: u16,
    pub window_format: String,
    pub window_current_format: String,
    pub pane_border_style: String,
    pub pane_active_border_style: String,
    pub message_style: String,
    pub mode_style: String,
    /// Copy mode search matches, and the one under the cursor.
    pub copy_mode_match_style: String,
    pub copy_mode_current_match_style: String,
    /// The colour of clock mode's digits.
    pub clock_mode_colour: String,
    /// `display-panes` number colour for the other panes, and the active one.
    pub display_panes_colour: String,
    pub display_panes_active_colour: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Resurrect {
    pub auto_save_minutes: u64,
    pub restore_on_start: bool,
    pub keep: usize,
    pub processes: Vec<String>,
    /// Programs from `processes` restored with their arguments, not just by
    /// name (tmux-resurrect restores its default list this way).
    pub restore_args: Vec<String>,
}

/// One entry in a key table: a bind, or `false` to remove a default bind.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum BindEntry {
    Bind(Bind),
    Enabled(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bind {
    /// Command list in tmux's command language.
    pub cmd: String,
    #[serde(default)]
    pub note: Option<String>,
    /// Repeatable within `repeat-time` without pressing the prefix again
    /// (tmux's `bind -r`).
    #[serde(default)]
    pub repeat: bool,
}

impl Config {
    /// The binds of every table, with `false` entries removed.
    pub fn binds(&self) -> impl Iterator<Item = (&str, &str, &Bind)> {
        self.keys.iter().flat_map(|(table, keys)| {
            keys.iter().filter_map(move |(key, entry)| match entry {
                BindEntry::Bind(b) => Some((table.as_str(), key.as_str(), b)),
                BindEntry::Enabled(_) => None,
            })
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        defaults()
    }
}

impl hjkl_config::AppConfig for Config {
    const APPLICATION: &'static str = "tmxr";
}

/// The built-in configuration.
pub fn defaults() -> Config {
    toml::from_str(DEFAULTS).unwrap_or_else(|e| panic!("bundled defaults.toml is invalid: {e}"))
}

/// Load the configuration: `path` if given (it must exist), else the user's
/// config file if present, layered over the defaults.
pub fn load(path: Option<&Path>) -> Result<(Config, ConfigSource), ConfigError> {
    match path {
        Some(p) => Ok((
            hjkl_config::load_layered_from::<Config>(DEFAULTS, p)?,
            ConfigSource::File(p.to_path_buf()),
        )),
        None => hjkl_config::load_layered::<Config>(DEFAULTS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind<'a>(cfg: &'a Config, table: &str, key: &str) -> Option<&'a Bind> {
        cfg.binds()
            .find(|(t, k, _)| *t == table && *k == key)
            .map(|(_, _, b)| b)
    }

    /// Every bind the tmux config, its plugins and plugin-notes.sh set up
    /// (docs/plan/06), as (table, key, command).
    const TMUX_CONF_BINDS: &[(&str, &str, &str)] = &[
        ("root", "C-h", "navigate-pane -L"),
        ("root", "C-j", "navigate-pane -D"),
        ("root", "C-k", "navigate-pane -U"),
        ("root", "C-l", "navigate-pane -R"),
        ("root", "C-\\", "navigate-pane -l"),
        ("root", "M-h", "previous-window"),
        ("root", "M-l", "next-window"),
        ("prefix", "h", "select-pane -L"),
        ("prefix", "j", "select-pane -D"),
        ("prefix", "k", "select-pane -U"),
        ("prefix", "l", "select-pane -R"),
        ("prefix", "'", "split-window -v -c \"#{pane_current_path}\""),
        (
            "prefix",
            "\"",
            "split-window -v -c \"#{pane_current_path}\"",
        ),
        ("prefix", ";", "split-window -h -c \"#{pane_current_path}\""),
        ("prefix", "%", "split-window -h -c \"#{pane_current_path}\""),
        ("prefix", "c", "new-window -c \"#{pane_current_path}\""),
        ("prefix", "x", "set-window-option synchronize-panes"),
        ("prefix", "C-l", "send-keys C-l"),
        ("prefix", "b", "last-window"),
        ("prefix", "C-n", "next-window"),
        ("prefix", "C-p", "previous-window"),
        ("prefix", "R", "source-file"),
        ("prefix", "C-s", "resurrect-save"),
        ("prefix", "C-r", "resurrect-restore"),
        ("prefix", "s", "choose-tree -Zs"),
        ("prefix", "d", "detach-client"),
        ("copy-mode-vi", "v", "send-keys -X begin-selection"),
        ("copy-mode-vi", "C-v", "send-keys -X rectangle-toggle"),
        (
            "copy-mode-vi",
            "y",
            "send-keys -X copy-selection-and-cancel",
        ),
        ("copy-mode-vi", "C-h", "select-pane -L"),
        ("copy-mode-vi", "C-j", "select-pane -D"),
        ("copy-mode-vi", "C-k", "select-pane -U"),
        ("copy-mode-vi", "C-l", "select-pane -R"),
        ("copy-mode-vi", "C-\\", "select-pane -l"),
    ];

    #[test]
    fn defaults_carry_every_tmux_conf_bind() {
        let cfg = defaults();
        for (table, key, cmd) in TMUX_CONF_BINDS {
            let b = bind(&cfg, table, key).unwrap_or_else(|| panic!("{table} {key} unbound"));
            assert_eq!(b.cmd, *cmd, "{table} {key}");
            assert!(b.note.is_some(), "{table} {key} has no note");
        }
    }

    /// tmux's own default prefix binds that the tmux config leaves alone
    /// (docs/plan/06), as (key, command name). The config overrides `'`, `;`,
    /// `l` and `x`.
    const TMUX_BUILTIN_BINDS: &[(&str, &str)] = &[
        ("C-b", "send-prefix"),
        ("C-o", "rotate-window"),
        ("M-o", "rotate-window"),
        ("Space", "next-layout"),
        ("!", "break-pane"),
        ("#", "list-buffers"),
        ("$", "command-prompt"),
        ("&", "confirm-before"),
        ("(", "switch-client"),
        (")", "switch-client"),
        (",", "command-prompt"),
        ("-", "delete-buffer"),
        (".", "command-prompt"),
        ("0", "select-window"),
        ("9", "select-window"),
        (":", "command-prompt"),
        ("?", "list-keys"),
        ("L", "switch-client"),
        ("[", "copy-mode"),
        ("]", "paste-buffer"),
        ("d", "detach-client"),
        ("i", "display-message"),
        ("n", "next-window"),
        ("M-n", "next-window"),
        ("o", "select-pane"),
        ("p", "previous-window"),
        ("M-p", "previous-window"),
        ("q", "display-panes"),
        ("r", "refresh-client"),
        ("s", "choose-tree"),
        ("w", "choose-tree"),
        ("z", "resize-pane"),
        ("{", "swap-pane"),
        ("}", "swap-pane"),
        ("~", "show-messages"),
        ("PPage", "copy-mode"),
        ("Up", "select-pane"),
        ("C-Up", "resize-pane"),
        ("M-Up", "resize-pane"),
        ("M-1", "select-layout"),
        ("E", "select-layout"),
        ("t", "clock-mode"),
        ("D", "choose-client"),
        ("C-z", "suspend-client"),
        ("/", "command-prompt"),
        ("m", "select-pane"),
        ("M", "select-pane"),
        ("=", "choose-buffer"),
        ("f", "command-prompt"),
    ];

    #[test]
    fn defaults_keep_tmux_builtin_binds() {
        let cfg = defaults();
        for (key, cmd) in TMUX_BUILTIN_BINDS {
            let b = bind(&cfg, "prefix", key).unwrap_or_else(|| panic!("prefix {key} unbound"));
            assert_eq!(b.cmd.split(' ').next(), Some(*cmd), "prefix {key}");
            assert!(b.note.is_some(), "prefix {key} has no note");
        }
    }

    #[test]
    fn defaults_match_tmux_conf_options() {
        let cfg = defaults();
        assert_eq!(cfg.prefix, "C-b");
        assert!(cfg.mouse);
        assert_eq!((cfg.base_index, cfg.pane_base_index), (0, 0));
        assert!(cfg.renumber_windows);
        assert_eq!(cfg.mode_keys, "vi");
        assert_eq!(cfg.extended_keys, "always");
        assert!(cfg.allow_passthrough);
        assert_eq!(cfg.status.left, "");
        assert!(
            cfg.update_environment
                .iter()
                .any(|v| v == "WAYLAND_DISPLAY")
        );
        assert!(cfg.navigator.pattern.contains("hjkl"));
    }

    #[test]
    fn every_default_bind_parses_as_commands_and_keys() {
        let cfg = defaults();
        let env = |_: &str| None;
        for (table, key, b) in cfg.binds() {
            key.parse::<tmxr_command::BindKey>()
                .unwrap_or_else(|e| panic!("{table} {key}: {e}"));
            let cmds = tmxr_command::tokenize(&b.cmd, &env)
                .unwrap_or_else(|e| panic!("{table} {key}: {e}"));
            assert!(!cmds.is_empty(), "{table} {key}");
            for c in cmds {
                tmxr_command::parse(&c).unwrap_or_else(|e| panic!("{table} {key}: {e}"));
            }
        }
    }

    #[test]
    fn user_file_overrides_and_removes_binds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
mouse = false
[keys.prefix]
x = false
"|" = { cmd = "split-window -h", note = "Split" }
"#,
        )
        .unwrap();
        let (cfg, source) = load(Some(&path)).unwrap();
        assert_eq!(source, ConfigSource::File(path));
        assert!(!cfg.mouse);
        assert!(bind(&cfg, "prefix", "x").is_none());
        assert_eq!(bind(&cfg, "prefix", "|").unwrap().cmd, "split-window -h");
        // Untouched defaults survive the merge.
        assert_eq!(bind(&cfg, "prefix", "h").unwrap().cmd, "select-pane -L");
    }

    #[test]
    fn unknown_keys_are_rejected_with_a_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "nonsense = 1\n").unwrap();
        let err = load(Some(&path)).unwrap_err().to_string();
        assert!(err.contains("nonsense"), "{err}");
    }
}
