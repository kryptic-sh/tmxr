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
pub use import::{Imported, from_tmux_conf};
pub use session::{OwnSessionOptions, SessionOptions};
pub use table::{Array, OptionValue, Word};
pub use window::{
    BoxLines, ClockModeStyle, CursorStyle, ModeKeys, OwnOptions, PANE_OPTIONS, PaneIndicators,
    PaneLines, PaneStatus, RemainOnExit, WindowOptions, WindowSize,
};

mod import;
mod session;
mod table;
mod window;

/// The embedded defaults.
pub const DEFAULTS: &str = include_str!("../defaults.toml");

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "File")]
pub struct Config {
    /// Entries kept per prompt type for Up/Down recall (tmux's default 100).
    pub prompt_history_limit: usize,
    /// Automatic paste buffers kept (`buffer-limit`); named ones are never
    /// dropped for it.
    pub buffer_limit: usize,
    /// Messages `show-messages` keeps (`message-limit`).
    pub message_limit: usize,
    /// `exit-empty`: the server exits once it has no sessions (and no
    /// attached clients); off, it stays.
    pub exit_empty: bool,
    /// `exit-unattached`: the server exits once no client is attached,
    /// sessions or not.
    pub exit_unattached: bool,
    /// `client-away-time`: seconds without input before a client is away
    /// and stops sizing windows; 0, never.
    pub client_away_time: u64,
    /// `prefix-timeout`: milliseconds the prefix table waits for its next
    /// key before giving it to the root table; 0 waits forever.
    pub prefix_timeout: u64,
    pub escape_time: u64,
    /// `focus-events`: whether clients' focus reports reach the server, and
    /// a pane's focus follows the active pane and window, as tmux's.
    pub focus_events: bool,
    pub default_terminal: String,
    pub extended_keys: String,
    pub set_clipboard: String,
    /// `on`, `off`, `16` or `auto`: whether clients get 24-bit colour, or
    /// colours mapped to the 256 (`off`) or the 16 ANSI ones.
    pub rgb_colour: String,
    /// Shell command `copy-pipe` sends the copied text to when it names none.
    pub copy_command: String,
    /// `editor`: what edits a paste buffer (`choose-buffer`'s `e`).
    pub editor: String,
    /// `history-file`: where the prompts' history is read from when the
    /// server starts and written when it exits; empty, nowhere.
    pub history_file: String,
    /// `owner` or `users`: who may open the server's endpoint, read when
    /// the server starts (`server-access` admits users by name).
    pub socket_access: String,
    pub navigator: Navigator,
    pub resurrect: Resurrect,
    /// `@name` user options: tmux's global session ones (`set -g`).
    pub options: BTreeMap<String, String>,
    /// `@name` user options set with `set -wg`: tmux's global window ones,
    /// apart from the session ones. Set at run time only.
    pub window_user: BTreeMap<String, String>,
    /// key table → key name → bind.
    pub keys: BTreeMap<String, BTreeMap<String, BindEntry>>,
    /// Global hooks (tmux's `set-hook -g`): hook name → its commands.
    pub hooks: BTreeMap<String, Vec<String>>,
    /// The global session options. The file keeps them with the rest (and
    /// some under `[status]`); here they are one table, as tmux's.
    pub session: SessionOptions,
    /// The global window options, kept in the file as the session ones are.
    pub window: WindowOptions,
}

/// The configuration file's layout, made into a [`Config`].
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct File {
    prefix: String,
    prefix2: String,
    key_table: String,
    mouse: bool,
    base_index: u32,
    pane_base_index: u16,
    pane_border_lines: PaneLines,
    pane_border_indicators: PaneIndicators,
    pane_border_status: PaneStatus,
    pane_border_format: String,
    renumber_windows: bool,
    set_titles: bool,
    set_titles_string: String,
    mode_keys: ModeKeys,
    status_keys: ModeKeys,
    history_limit: usize,
    prompt_history_limit: usize,
    buffer_limit: usize,
    message_limit: usize,
    exit_empty: bool,
    prefix_timeout: u64,
    exit_unattached: bool,
    client_away_time: u64,
    destroy_unattached: DestroyUnattached,
    detach_on_destroy: DetachOnDestroy,
    escape_time: u64,
    focus_events: bool,
    display_time: u64,
    display_panes_time: u64,
    status_interval: u64,
    repeat_time: u64,
    default_terminal: String,
    default_command: String,
    default_size: String,
    #[serde(default)]
    default_shell: Option<String>,
    extended_keys: String,
    set_clipboard: String,
    rgb_colour: String,
    allow_passthrough: bool,
    allow_rename: bool,
    allow_set_title: bool,
    alternate_screen: bool,
    remain_on_exit: RemainOnExit,
    remain_on_exit_format: String,
    synchronize_panes: bool,
    automatic_rename: bool,
    aggressive_resize: bool,
    automatic_rename_format: String,
    monitor_bell: bool,
    wrap_search: bool,
    word_separators: String,
    clock_mode_style: ClockModeStyle,
    cursor_colour: String,
    cursor_style: CursorStyle,
    window_size: WindowSize,
    window_style: String,
    window_active_style: String,
    fill_character: String,
    monitor_activity: bool,
    monitor_silence: u64,
    bell_action: AlertAction,
    activity_action: AlertAction,
    assume_paste_time: u64,
    silence_action: AlertAction,
    visual_bell: Visual,
    visual_activity: Visual,
    visual_silence: Visual,
    pane_scrollbars: Scrollbars,
    pane_scrollbars_position: ScrollbarPosition,
    pane_scrollbars_style: String,
    main_pane_height: String,
    main_pane_width: String,
    other_pane_height: String,
    other_pane_width: String,
    copy_command: String,
    editor: String,
    history_file: String,
    lock_command: String,
    lock_after_time: u64,
    socket_access: String,
    update_environment: Vec<String>,
    navigator: Navigator,
    status: StatusFile,
    resurrect: Resurrect,
    options: BTreeMap<String, String>,
    keys: BTreeMap<String, BTreeMap<String, BindEntry>>,
    #[serde(default)]
    hooks: BTreeMap<String, HookEntry>,
}

/// A hook in the file: one command, or several run in order (as
/// `set-hook -a` adds them).
#[derive(Deserialize)]
#[serde(untagged)]
enum HookEntry {
    One(String),
    Many(Vec<String>),
}

impl From<File> for Config {
    fn from(f: File) -> Self {
        let s = f.status;
        Self {
            window: WindowOptions {
                allow_passthrough: f.allow_passthrough,
                automatic_rename: f.automatic_rename,
                aggressive_resize: f.aggressive_resize,
                clock_mode_colour: Word(s.clock_mode_colour),
                copy_mode_current_match_style: s.copy_mode_current_match_style,
                copy_mode_match_style: s.copy_mode_match_style,
                copy_mode_mark_style: s.copy_mode_mark_style,
                copy_mode_position_format: s.copy_mode_position_format,
                copy_mode_position_style: s.copy_mode_position_style,
                copy_mode_selection_style: s.copy_mode_selection_style,
                main_pane_height: f.main_pane_height,
                main_pane_width: f.main_pane_width,
                menu_border_lines: s.menu_border_lines,
                menu_border_style: s.menu_border_style,
                menu_selected_style: s.menu_selected_style,
                menu_style: s.menu_style,
                mode_keys: f.mode_keys,
                mode_style: s.mode_style,
                monitor_activity: f.monitor_activity,
                monitor_silence: f.monitor_silence,
                other_pane_height: f.other_pane_height,
                other_pane_width: f.other_pane_width,
                pane_active_border_style: s.pane_active_border_style,
                pane_base_index: f.pane_base_index,
                pane_border_lines: f.pane_border_lines,
                pane_border_indicators: f.pane_border_indicators,
                pane_border_status: f.pane_border_status,
                pane_border_format: f.pane_border_format,
                pane_border_style: s.pane_border_style,
                pane_scrollbars: f.pane_scrollbars,
                pane_scrollbars_position: f.pane_scrollbars_position,
                pane_scrollbars_style: f.pane_scrollbars_style,
                pane_status_current_style: s.pane_status_current_style,
                pane_status_style: s.pane_status_style,
                session_status_current_style: s.session_status_current_style,
                session_status_style: s.session_status_style,
                popup_border_lines: s.popup_border_lines,
                popup_border_style: s.popup_border_style,
                popup_style: s.popup_style,
                remain_on_exit: f.remain_on_exit,
                remain_on_exit_format: f.remain_on_exit_format,
                allow_set_title: f.allow_set_title,
                alternate_screen: f.alternate_screen,
                allow_rename: f.allow_rename,
                automatic_rename_format: f.automatic_rename_format,
                monitor_bell: f.monitor_bell,
                wrap_search: f.wrap_search,
                clock_mode_style: f.clock_mode_style,
                cursor_colour: Word(f.cursor_colour),
                cursor_style: f.cursor_style,
                synchronize_panes: f.synchronize_panes,
                window_size: f.window_size,
                window_style: f.window_style,
                window_active_style: f.window_active_style,
                fill_character: f.fill_character,
                window_status_current_format: s.window_current_format,
                window_status_activity_style: s.window_activity_style,
                window_status_bell_style: s.window_bell_style,
                window_status_current_style: s.window_current_style,
                window_status_format: s.window_format,
                window_status_last_style: s.window_last_style,
                window_status_separator: s.window_separator,
                window_status_style: s.window_style,
            },
            session: SessionOptions {
                activity_action: f.activity_action,
                assume_paste_time: f.assume_paste_time,
                base_index: f.base_index,
                bell_action: f.bell_action,
                default_command: f.default_command,
                default_size: f.default_size,
                default_shell: f.default_shell,
                destroy_unattached: f.destroy_unattached,
                detach_on_destroy: f.detach_on_destroy,
                display_panes_active_colour: Word(s.display_panes_active_colour),
                display_panes_colour: Word(s.display_panes_colour),
                display_panes_time: f.display_panes_time,
                display_time: f.display_time,
                status: s.status,
                status_format: s.format.into_iter().collect(),
                status_bg: Word(s.bg),
                status_fg: Word(s.fg),
                history_limit: f.history_limit,
                lock_after_time: f.lock_after_time,
                lock_command: f.lock_command,
                message_line: s.message_line,
                message_style: s.message_style,
                message_command_style: s.message_command_style,
                mouse: f.mouse,
                prefix: Word(f.prefix),
                prefix2: Word(f.prefix2),
                key_table: f.key_table,
                renumber_windows: f.renumber_windows,
                repeat_time: f.repeat_time,
                set_titles: f.set_titles,
                set_titles_string: f.set_titles_string,
                silence_action: f.silence_action,
                status_interval: f.status_interval,
                status_keys: f.status_keys,
                status_left: s.left,
                status_left_length: s.left_length,
                status_left_style: s.left_style,
                status_right: s.right,
                status_right_length: s.right_length,
                status_right_style: s.right_style,
                status_justify: s.justify,
                status_position: s.position,
                status_style: s.style,
                update_environment: f.update_environment.into_iter().collect(),
                visual_activity: f.visual_activity,
                visual_bell: f.visual_bell,
                visual_silence: f.visual_silence,
                word_separators: f.word_separators,
            },
            prompt_history_limit: f.prompt_history_limit,
            buffer_limit: f.buffer_limit,
            message_limit: f.message_limit,
            exit_empty: f.exit_empty,
            prefix_timeout: f.prefix_timeout,
            exit_unattached: f.exit_unattached,
            client_away_time: f.client_away_time,
            escape_time: f.escape_time,
            focus_events: f.focus_events,
            default_terminal: f.default_terminal,
            extended_keys: f.extended_keys,
            set_clipboard: f.set_clipboard,
            rgb_colour: f.rgb_colour,
            copy_command: f.copy_command,
            editor: if f.editor.is_empty() {
                default_editor()
            } else {
                f.editor
            },
            history_file: f.history_file,
            socket_access: f.socket_access,
            navigator: f.navigator,
            resurrect: f.resurrect,
            options: f.options,
            window_user: BTreeMap::new(),
            keys: f.keys,
            hooks: f
                .hooks
                .into_iter()
                .map(|(name, entry)| match entry {
                    HookEntry::One(cmd) => (name, vec![cmd]),
                    HookEntry::Many(cmds) => (name, cmds),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Navigator {
    pub pattern: String,
    pub disable_when_zoomed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct StatusFile {
    status: StatusLines,
    /// `status-format`: each status line's format, by line.
    format: Vec<String>,
    message_line: MessageLine,
    style: String,
    fg: String,
    bg: String,
    justify: StatusJustify,
    position: StatusPosition,
    left: String,
    right: String,
    left_length: u16,
    right_length: u16,
    window_format: String,
    window_current_format: String,
    left_style: String,
    right_style: String,
    window_style: String,
    window_current_style: String,
    window_last_style: String,
    window_activity_style: String,
    window_bell_style: String,
    window_separator: String,
    pane_border_style: String,
    pane_status_current_style: String,
    pane_status_style: String,
    session_status_current_style: String,
    session_status_style: String,
    pane_active_border_style: String,
    message_style: String,
    message_command_style: String,
    mode_style: String,
    copy_mode_match_style: String,
    copy_mode_current_match_style: String,
    copy_mode_mark_style: String,
    copy_mode_position_format: String,
    copy_mode_position_style: String,
    copy_mode_selection_style: String,
    clock_mode_colour: String,
    display_panes_colour: String,
    display_panes_active_colour: String,
    menu_style: String,
    menu_selected_style: String,
    menu_border_style: String,
    popup_style: String,
    popup_border_style: String,
    menu_border_lines: BoxLines,
    popup_border_lines: BoxLines,
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

/// `destroy-unattached`: whether a session no client is attached to goes,
/// as tmux's. tmxr has no session groups, so `keep-last` keeps every
/// `status`: how many status lines a client shows, as tmux's: `off`, `on`
/// (one) or 2 to 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusLines(u8);

impl StatusLines {
    /// The most status lines tmux takes.
    const MAX: u8 = 5;

    pub const NAMES: &str = "off, on, 2, 3, 4 or 5";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self(0)),
            "on" => Some(Self(1)),
            n => n
                .parse::<u8>()
                .ok()
                .filter(|n| (2..=Self::MAX).contains(n))
                .map(Self),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self.0 {
            0 => "off",
            1 => "on",
            2 => "2",
            3 => "3",
            4 => "4",
            _ => "5",
        }
    }

    /// The rows they take.
    pub const fn rows(self) -> u16 {
        self.0 as u16
    }
}

impl<'de> Deserialize<'de> for StatusLines {
    /// A name as `set-option` takes it, or (as tmxr's file once had it) a
    /// flag.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Flag(bool),
            Name(String),
        }
        match Raw::deserialize(d)? {
            Raw::Flag(on) => Ok(Self(u8::from(on))),
            Raw::Name(n) => Self::parse(&n).ok_or_else(|| {
                serde::de::Error::custom(format!("unknown status: {n} ({})", Self::NAMES))
            }),
        }
    }
}

/// `message-line`: which status line a message or prompt takes, as tmux's:
/// 0 to 4, the last there is when there are fewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageLine(u8);

impl MessageLine {
    /// The last line tmux names.
    const MAX: u8 = 4;

    pub const NAMES: &str = "0, 1, 2, 3 or 4";

    pub fn parse(s: &str) -> Option<Self> {
        s.parse::<u8>().ok().filter(|n| *n <= Self::MAX).map(Self)
    }

    pub const fn as_str(self) -> &'static str {
        match self.0 {
            0 => "0",
            1 => "1",
            2 => "2",
            3 => "3",
            _ => "4",
        }
    }

    /// The line, from the status line's first.
    pub const fn line(self) -> u16 {
        self.0 as u16
    }
}

impl<'de> Deserialize<'de> for MessageLine {
    /// A name as `set-option` takes it, or a number.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Number(u8),
            Name(String),
        }
        let name = match Raw::deserialize(d)? {
            Raw::Number(n) => n.to_string(),
            Raw::Name(n) => n,
        };
        Self::parse(&name).ok_or_else(|| {
            serde::de::Error::custom(format!("unknown message-line: {name} ({})", Self::NAMES))
        })
    }
}

/// `status-position`: the client's row the status line takes, as tmux's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusPosition {
    Top,
    Bottom,
}

impl StatusPosition {
    pub const NAMES: &str = "top or bottom";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }
}

/// `status-justify`: where the window list sits on the status line, as
/// tmux's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatusJustify {
    /// After `status-left`.
    Left,
    /// Centred between `status-left` and `status-right`.
    Centre,
    /// At the right edge, `status-right` before it.
    Right,
    /// Centred on the whole line.
    AbsoluteCentre,
}

impl StatusJustify {
    pub const NAMES: &str = "left, centre, right or absolute-centre";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "left" => Some(Self::Left),
            "centre" => Some(Self::Centre),
            "right" => Some(Self::Right),
            "absolute-centre" => Some(Self::AbsoluteCentre),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Centre => "centre",
            Self::Right => "right",
            Self::AbsoluteCentre => "absolute-centre",
        }
    }
}

/// session and `keep-group` keeps none, as tmux's do for a session in no
/// group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DestroyUnattached {
    Off,
    On,
    KeepLast,
    KeepGroup,
}

impl DestroyUnattached {
    pub const NAMES: &str = "off, on, keep-last or keep-group";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "on" => Some(Self::On),
            "keep-last" => Some(Self::KeepLast),
            "keep-group" => Some(Self::KeepGroup),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::KeepLast => "keep-last",
            Self::KeepGroup => "keep-group",
        }
    }

    /// Whether an unattached session goes.
    pub const fn destroys(self) -> bool {
        matches!(self, Self::On | Self::KeepGroup)
    }
}

/// Where a destroyed session's clients go (`detach-on-destroy`), as tmux's
/// `server_destroy_session`: `on` detaches them, `off` moves them to the
/// session used last, `no-detached` to the one used last that has no client,
/// and `previous` / `next` to the session before or after it by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DetachOnDestroy {
    Off,
    On,
    NoDetached,
    Previous,
    Next,
}

impl DetachOnDestroy {
    pub const NAMES: &str = "off, on, no-detached, previous or next";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "on" => Some(Self::On),
            "no-detached" => Some(Self::NoDetached),
            "previous" => Some(Self::Previous),
            "next" => Some(Self::Next),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::NoDetached => "no-detached",
            Self::Previous => "previous",
            Self::Next => "next",
        }
    }
}

/// Which windows' alerts act, relative to their session's current window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertAction {
    Any,
    None,
    Current,
    Other,
}

impl AlertAction {
    pub const NAMES: &str = "any, none, current or other";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "any" => Some(Self::Any),
            "none" => Some(Self::None),
            "current" => Some(Self::Current),
            "other" => Some(Self::Other),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::None => "none",
            Self::Current => "current",
            Self::Other => "other",
        }
    }

    /// Whether an alert in a window that is (or is not) its session's
    /// current one acts.
    pub const fn applies(self, current: bool) -> bool {
        match self {
            Self::Any => true,
            Self::None => false,
            Self::Current => current,
            Self::Other => !current,
        }
    }
}

/// When panes show a scrollbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scrollbars {
    Off,
    Modal,
    On,
}

impl Scrollbars {
    pub const NAMES: &str = "off, modal or on";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "modal" => Some(Self::Modal),
            "on" => Some(Self::On),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Modal => "modal",
            Self::On => "on",
        }
    }
}

/// Which side of a pane its scrollbar is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScrollbarPosition {
    Right,
    Left,
}

impl ScrollbarPosition {
    pub const NAMES: &str = "right or left";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "right" => Some(Self::Right),
            "left" => Some(Self::Left),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Left => "left",
        }
    }
}

/// How an alert acts: the terminal's bell (`off`), a status message (`on`),
/// or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visual {
    Off,
    On,
    Both,
}

impl Visual {
    pub const NAMES: &str = "off, on or both";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "on" => Some(Self::On),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Both => "both",
        }
    }

    pub const fn bell(self) -> bool {
        matches!(self, Self::Off | Self::Both)
    }

    pub const fn message(self) -> bool {
        matches!(self, Self::On | Self::Both)
    }
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

impl Config {
    /// Give option `name` the value it has in `from`: a server option, or a
    /// global session or window one. False for a name that is none of them.
    pub fn copy_option(&mut self, from: &Self, name: &str) -> bool {
        match name {
            "prompt-history-limit" => self.prompt_history_limit = from.prompt_history_limit,
            "buffer-limit" => self.buffer_limit = from.buffer_limit,
            "message-limit" => self.message_limit = from.message_limit,
            "exit-empty" => self.exit_empty = from.exit_empty,
            "exit-unattached" => self.exit_unattached = from.exit_unattached,
            "client-away-time" => self.client_away_time = from.client_away_time,
            "prefix-timeout" => self.prefix_timeout = from.prefix_timeout,
            "escape-time" => self.escape_time = from.escape_time,
            "focus-events" => self.focus_events = from.focus_events,
            "default-terminal" => self.default_terminal.clone_from(&from.default_terminal),
            "extended-keys" => self.extended_keys.clone_from(&from.extended_keys),
            "set-clipboard" => self.set_clipboard.clone_from(&from.set_clipboard),
            "rgb-colour" => self.rgb_colour.clone_from(&from.rgb_colour),
            "copy-command" => self.copy_command.clone_from(&from.copy_command),
            "editor" => self.editor.clone_from(&from.editor),
            "history-file" => self.history_file.clone_from(&from.history_file),
            "socket-access" => self.socket_access.clone_from(&from.socket_access),
            _ => {
                return self.session.copy_from(&from.session, name)
                    || self.window.copy_from(&from.window, name);
            }
        }
        true
    }
}

/// A config file's `[status]` keys and the options each sets, as `File`
/// turns them into a [`Config`].
const STATUS_OPTIONS: &[(&str, &str)] = &[
    ("clock-mode-colour", "clock-mode-colour"),
    (
        "copy-mode-current-match-style",
        "copy-mode-current-match-style",
    ),
    ("copy-mode-match-style", "copy-mode-match-style"),
    ("copy-mode-mark-style", "copy-mode-mark-style"),
    ("copy-mode-position-format", "copy-mode-position-format"),
    ("copy-mode-position-style", "copy-mode-position-style"),
    ("copy-mode-selection-style", "copy-mode-selection-style"),
    ("menu-border-lines", "menu-border-lines"),
    ("menu-border-style", "menu-border-style"),
    ("menu-selected-style", "menu-selected-style"),
    ("menu-style", "menu-style"),
    ("mode-style", "mode-style"),
    ("pane-active-border-style", "pane-active-border-style"),
    ("pane-border-style", "pane-border-style"),
    ("pane-status-current-style", "pane-status-current-style"),
    ("pane-status-style", "pane-status-style"),
    (
        "session-status-current-style",
        "session-status-current-style",
    ),
    ("session-status-style", "session-status-style"),
    ("popup-border-lines", "popup-border-lines"),
    ("popup-border-style", "popup-border-style"),
    ("popup-style", "popup-style"),
    ("window-current-format", "window-status-current-format"),
    ("window-activity-style", "window-status-activity-style"),
    ("window-bell-style", "window-status-bell-style"),
    ("window-current-style", "window-status-current-style"),
    ("window-format", "window-status-format"),
    ("window-last-style", "window-status-last-style"),
    ("window-separator", "window-status-separator"),
    ("window-style", "window-status-style"),
    ("display-panes-active-colour", "display-panes-active-colour"),
    ("display-panes-colour", "display-panes-colour"),
    ("status", "status"),
    ("format", "status-format"),
    ("bg", "status-bg"),
    ("fg", "status-fg"),
    ("message-line", "message-line"),
    ("message-style", "message-style"),
    ("message-command-style", "message-command-style"),
    ("left", "status-left"),
    ("left-length", "status-left-length"),
    ("left-style", "status-left-style"),
    ("right", "status-right"),
    ("right-length", "status-right-length"),
    ("right-style", "status-right-style"),
    ("justify", "status-justify"),
    ("position", "status-position"),
    ("style", "status-style"),
];

/// The parts of a config file that hold no option of their own name.
const SECTIONS: &[&str] = &[
    "keys",
    "hooks",
    "navigator",
    "options",
    "resurrect",
    "status",
];

/// What a config file sets, as written: for `source-file` to apply only
/// that over what is running, as tmux runs a file's commands.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FileSets {
    /// Options by tmux name ([`Config::copy_option`]).
    pub options: Vec<String>,
    /// `[options]`: user options (`@name`).
    pub user_options: Vec<String>,
    /// `[keys.TABLE]`: each (table, key) it binds or unbinds.
    pub binds: Vec<(String, String)>,
    /// `[hooks]`: each hook it names.
    pub hooks: Vec<String>,
    /// Whether it has a `[navigator]` / `[resurrect]` section.
    pub navigator: bool,
    pub resurrect: bool,
}

fn sets(table: &toml::Table) -> FileSets {
    let keys_of = |section: &str| -> Vec<String> {
        table
            .get(section)
            .and_then(toml::Value::as_table)
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default()
    };
    let binds = table
        .get("keys")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
        .flat_map(|(t, entries)| {
            entries
                .as_table()
                .into_iter()
                .flat_map(|e| e.keys())
                .map(move |k| (t.clone(), k.clone()))
        })
        .collect();
    FileSets {
        options: options_set(table),
        user_options: keys_of("options"),
        binds,
        hooks: keys_of("hooks"),
        navigator: table.contains_key("navigator"),
        resurrect: table.contains_key("resurrect"),
    }
}

/// The options a config file sets, by tmux name: its top-level keys, each
/// an option's name, and its `[status]` keys, under the options they set.
fn options_set(table: &toml::Table) -> Vec<String> {
    let mut names: Vec<String> = table
        .keys()
        .filter(|k| !SECTIONS.contains(&k.as_str()))
        .cloned()
        .collect();
    if let Some(status) = table.get("status").and_then(toml::Value::as_table) {
        names.extend(status.keys().filter_map(|k| {
            STATUS_OPTIONS
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, name)| (*name).to_owned())
        }));
    }
    names
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

/// [`load`], keeping what works: where the file reads as TOML but some of
/// its settings do not (an unknown key, a value of the wrong kind), the
/// others are kept and each one left out is named, as tmux runs a config
/// line by line and keeps what worked. A file that is not TOML, or cannot
/// be read, is still an error, as there are no settings in it to keep.
pub fn load_keeping(
    path: Option<&Path>,
) -> Result<(Config, ConfigSource, Vec<String>), ConfigError> {
    match load(path) {
        Ok((cfg, source)) => Ok((cfg, source, Vec::new())),
        Err(ConfigError::Invalid { .. }) => {
            let file = match path {
                Some(p) => p.to_path_buf(),
                None => hjkl_config::config_path::<Config>()?,
            };
            let kept = Kept::read(&file)?;
            Ok((kept.config, ConfigSource::File(file), kept.left_out))
        }
        Err(e) => Err(e),
    }
}

/// The config file at `path` as [`load_keeping`] keeps it, what the
/// settings it kept set ([`FileSets`], for `source-file` to apply them and
/// nothing else), and the settings left out.
pub fn load_file(path: &Path) -> Result<(Config, FileSets, Vec<String>), String> {
    let kept = Kept::read(path).map_err(|e| e.to_string())?;
    Ok((kept.config, sets(&kept.from_file), kept.left_out))
}

/// A config file read setting by setting.
struct Kept {
    config: Config,
    /// The file's settings that were kept.
    from_file: toml::Table,
    left_out: Vec<String>,
}

impl Kept {
    fn read(file: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(file).map_err(|e| ConfigError::Io {
            path: file.to_path_buf(),
            source: e,
        })?;
        let user: toml::Table = toml::from_str(&text).map_err(|e| ConfigError::Invalid {
            path: file.to_path_buf(),
            message: e.to_string(),
        })?;
        let mut kept = Self {
            config: defaults(),
            from_file: toml::Table::new(),
            left_out: Vec::new(),
        };
        let mut merged: toml::Table = DEFAULTS
            .parse()
            .unwrap_or_else(|e| panic!("bundled defaults.toml is invalid: {e}"));
        for (key, value) in user {
            kept.keep(&mut merged, &[key.as_str()], value);
        }
        kept.config = toml::Value::Table(merged)
            .try_into()
            .map_err(|e: toml::de::Error| ConfigError::Invalid {
                path: file.to_path_buf(),
                message: e.to_string(),
            })?;
        Ok(kept)
    }

    /// Merge `value` at `path` into `merged` if the result still reads as a
    /// [`Config`]; else, for a table, each of its keys in turn, and for
    /// anything else an error naming `path`.
    fn keep(&mut self, merged: &mut toml::Table, path: &[&str], value: toml::Value) {
        let mut layer = value.clone();
        for key in path.iter().rev() {
            layer = toml::Value::Table(toml::Table::from_iter([((*key).to_owned(), layer)]));
        }
        let toml::Value::Table(layer) = layer else {
            return;
        };
        let mut candidate = merged.clone();
        deep_merge(&mut candidate, layer.clone());
        let tried: Result<Config, _> = toml::Value::Table(candidate.clone()).try_into();
        match (tried, value) {
            (Ok(_), _) => {
                *merged = candidate;
                deep_merge(&mut self.from_file, layer);
            }
            (Err(_), toml::Value::Table(table)) if !table.is_empty() => {
                for (key, value) in table {
                    let mut deeper = path.to_vec();
                    deeper.push(&key);
                    self.keep(merged, &deeper, value);
                }
            }
            (Err(e), _) => self
                .left_out
                .push(format!("{}: {}", path.join("."), e.message())),
        }
    }
}

/// `from` merged into `into` as hjkl_config layers a file over its defaults
/// (tables key by key, anything else replaced), which that crate does not
/// export.
fn deep_merge(into: &mut toml::Table, from: toml::Table) {
    for (key, value) in from {
        match (into.get_mut(&key), value) {
            (Some(toml::Value::Table(into)), toml::Value::Table(from)) => deep_merge(into, from),
            (_, value) => {
                into.insert(key, value);
            }
        }
    }
}

/// The editor tmux starts with: `$VISUAL`, else `$EDITOR`, else tmux's
/// `/usr/bin/vi` (`_PATH_VI`); on Windows, which has neither, Notepad.
fn default_editor() -> String {
    let fallback = if cfg!(windows) {
        "notepad"
    } else {
        "/usr/bin/vi"
    };
    ["VISUAL", "EDITOR"]
        .into_iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| fallback.to_owned())
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Settings a file gets wrong are left out and named; the rest stay.
    #[test]
    fn a_files_good_settings_stay_beside_its_bad_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "escape-time = 7\nstatus-interval = \"often\"\nno-such-option = 1\n\
             [status]\nleft = \"LEFT\"\nleft-length = \"long\"\n",
        )
        .unwrap();
        assert!(load(Some(&path)).is_err(), "one error fails the whole file");
        let (cfg, _, errors) = load_keeping(Some(&path)).unwrap();
        assert_eq!(cfg.escape_time, 7);
        assert_eq!(
            cfg.session.status_interval,
            defaults().session.status_interval
        );
        assert_eq!(cfg.session.status_left, "LEFT");
        let named: Vec<&str> = errors
            .iter()
            .map(|e| e.split(':').next().unwrap())
            .collect();
        assert_eq!(
            named,
            ["no-such-option", "status.left-length", "status-interval"]
        );
        // A file that is not TOML has nothing to keep.
        std::fs::write(&path, "escape-time = = 7\n").unwrap();
        assert!(load_keeping(Some(&path)).is_err());
    }

    /// Every option the defaults set is found by [`options_set`] and known to
    /// [`Config::copy_option`], and every `[status]` key has its option, so
    /// a file's settings cannot go unnoticed.
    #[test]
    fn options_set_names_every_option_a_file_can_set() {
        let table: toml::Table = DEFAULTS.parse().unwrap();
        let status = table["status"].as_table().unwrap();
        for key in status.keys() {
            assert!(
                STATUS_OPTIONS.iter().any(|(k, _)| k == key),
                "[status] {key} sets no option"
            );
        }
        let names = options_set(&table);
        let options = table
            .keys()
            .filter(|k| !SECTIONS.contains(&k.as_str()))
            .count();
        assert_eq!(names.len(), options + status.len());
        let (mut to, from) = (defaults(), defaults());
        for name in &names {
            assert!(to.copy_option(&from, name), "{name} cannot be copied");
        }
    }

    #[test]
    fn copy_option_takes_one_option_only() {
        let mut to = defaults();
        let mut from = defaults();
        from.session
            .set("status-left", Some("from"), false)
            .unwrap();
        from.session
            .set("status-right", Some("from"), false)
            .unwrap();
        from.escape_time = 999;
        assert!(to.copy_option(&from, "status-left"));
        assert!(to.copy_option(&from, "escape-time"));
        assert_eq!(to.session.show("status-left").as_deref(), Some("from"));
        assert_ne!(to.session.show("status-right").as_deref(), Some("from"));
        assert_eq!(to.escape_time, 999);
        assert!(!to.copy_option(&from, "no-such-option"));
    }

    #[test]
    fn a_file_says_what_it_sets() {
        let table: toml::Table = "escape-time = 1\n[options]\n'@a' = 'x'\n[keys.prefix]\nr = false\nx = 'kill-pane'\n[hooks]\nafter-new-window = 'x'\n[navigator]\npattern = 'vim'\n"
            .parse()
            .unwrap();
        let s = sets(&table);
        assert_eq!(s.options, ["escape-time"]);
        assert_eq!(s.user_options, ["@a"]);
        assert_eq!(
            s.binds,
            [
                ("prefix".to_owned(), "r".to_owned()),
                ("prefix".to_owned(), "x".to_owned())
            ]
        );
        assert_eq!(s.hooks, ["after-new-window"]);
        assert!(s.navigator && !s.resurrect);
    }

    #[test]
    fn options_set_reads_status_keys_under_their_names() {
        let table: toml::Table =
            "history-limit = 5\n[status]\nleft = 'x'\nwindow-format = 'y'\n[keys.root]\n"
                .parse()
                .unwrap();
        let mut names = options_set(&table);
        names.sort();
        assert_eq!(
            names,
            ["history-limit", "status-left", "window-status-format"]
        );
    }

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
        assert_eq!(&*cfg.session.prefix, "C-b");
        assert!(cfg.session.mouse);
        assert_eq!((cfg.session.base_index, cfg.window.pane_base_index), (0, 0));
        assert!(cfg.session.renumber_windows);
        assert_eq!(cfg.window.mode_keys, ModeKeys::Vi);
        assert_eq!(cfg.extended_keys, "always");
        assert!(cfg.window.allow_passthrough);
        assert_eq!(cfg.session.status_left, "");
        assert!(
            cfg.session
                .update_environment
                .values()
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
        assert!(!cfg.session.mouse);
        assert!(bind(&cfg, "prefix", "x").is_none());
        assert_eq!(bind(&cfg, "prefix", "|").unwrap().cmd, "split-window -h");
        // Untouched defaults survive the merge.
        assert_eq!(bind(&cfg, "prefix", "h").unwrap().cmd, "select-pane -L");
    }

    #[test]
    fn hooks_take_one_command_or_several() {
        assert!(defaults().hooks.is_empty());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[hooks]
session-created = ["new-window -d", "new-window -d"]
client-attached = "display-message hi"
"#,
        )
        .unwrap();
        let (cfg, _) = load(Some(&path)).unwrap();
        assert_eq!(
            cfg.hooks["session-created"],
            ["new-window -d", "new-window -d"]
        );
        assert_eq!(cfg.hooks["client-attached"], ["display-message hi"]);
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
