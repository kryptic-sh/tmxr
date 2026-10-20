//! tmux's window options: one typed table, read by `set-option` and printed
//! by `show-options` under tmux's names.

use serde::Deserialize;

use crate::table::{OptionValue, Word, options_table};
use crate::{ScrollbarPosition, Scrollbars};

/// `cursor-style`: a pane's cursor shape when its program has not asked
/// for one, as tmux's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CursorStyle {
    Default,
    BlinkingBlock,
    Block,
    BlinkingUnderline,
    Underline,
    BlinkingBar,
    Bar,
}

impl CursorStyle {
    pub const NAMES: &str =
        "default, blinking-block, block, blinking-underline, underline, blinking-bar or bar";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "default" => Some(Self::Default),
            "blinking-block" => Some(Self::BlinkingBlock),
            "block" => Some(Self::Block),
            "blinking-underline" => Some(Self::BlinkingUnderline),
            "underline" => Some(Self::Underline),
            "blinking-bar" => Some(Self::BlinkingBar),
            "bar" => Some(Self::Bar),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::BlinkingBlock => "blinking-block",
            Self::Block => "block",
            Self::BlinkingUnderline => "blinking-underline",
            Self::Underline => "underline",
            Self::BlinkingBar => "blinking-bar",
            Self::Bar => "bar",
        }
    }

    /// The shape as DECSCUSR's `Ps`.
    pub const fn ps(self) -> u8 {
        self as u8
    }
}

/// `clock-mode-style`: how clock mode tells the time, as tmux's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ClockModeStyle {
    #[serde(rename = "12")]
    Twelve,
    #[serde(rename = "24")]
    TwentyFour,
    #[serde(rename = "12-with-seconds")]
    TwelveSeconds,
    #[serde(rename = "24-with-seconds")]
    TwentyFourSeconds,
}

impl ClockModeStyle {
    pub const NAMES: &str = "12, 24, 12-with-seconds or 24-with-seconds";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "12" => Some(Self::Twelve),
            "24" => Some(Self::TwentyFour),
            "12-with-seconds" => Some(Self::TwelveSeconds),
            "24-with-seconds" => Some(Self::TwentyFourSeconds),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Twelve => "12",
            Self::TwentyFour => "24",
            Self::TwelveSeconds => "12-with-seconds",
            Self::TwentyFourSeconds => "24-with-seconds",
        }
    }

    /// The time as clock mode shows it, as tmux's `strftime` formats it:
    /// `%l:%M AM` (the hour space-padded, 12 for 0), `%H:%M`, and either
    /// with `:%S`.
    pub fn text(self, hour: u8, minute: u8, second: u8) -> String {
        let seconds = matches!(self, Self::TwelveSeconds | Self::TwentyFourSeconds);
        let tail = if seconds {
            format!(":{second:02}")
        } else {
            String::new()
        };
        if matches!(self, Self::Twelve | Self::TwelveSeconds) {
            let h12 = match hour % 12 {
                0 => 12,
                h => h,
            };
            let half = if hour >= 12 { "PM" } else { "AM" };
            format!("{h12:>2}:{minute:02}{tail} {half}")
        } else {
            format!("{hour:02}:{minute:02}{tail}")
        }
    }
}

/// `remain-on-exit`: whether a pane whose program exited stays, shown dead,
/// as tmux's: never, always, or (`failed`) when it exited with an error or
/// was killed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemainOnExit {
    Off,
    On,
    Failed,
}

impl RemainOnExit {
    pub const NAMES: &str = "off, on or failed";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "on" => Some(Self::On),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Failed => "failed",
        }
    }

    /// Whether a pane whose program ended with exit `code` (`None`: killed
    /// by a signal) stays.
    pub const fn keeps(self, code: Option<u32>) -> bool {
        match self {
            Self::Off => false,
            Self::On => true,
            Self::Failed => !matches!(code, Some(0)),
        }
    }
}

/// The config file takes `true` / `false` as well as the names.
impl<'de> Deserialize<'de> for RemainOnExit {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum File {
            Flag(bool),
            Name(String),
        }
        match File::deserialize(d)? {
            File::Flag(true) => Ok(Self::On),
            File::Flag(false) => Ok(Self::Off),
            File::Name(n) => Self::parse(&n).ok_or_else(|| {
                serde::de::Error::custom(format!("unknown value: {n} ({})", Self::NAMES))
            }),
        }
    }
}

impl OptionValue for RemainOnExit {
    /// No value toggles between off and on, as tmux toggles a choice
    /// between its first two.
    fn parse(value: Option<&str>, current: &Self) -> Result<Self, String> {
        match value {
            None => Ok(match current {
                Self::Off => Self::On,
                Self::On => Self::Off,
                Self::Failed => Self::Failed,
            }),
            Some(v) => {
                Self::parse(v).ok_or_else(|| format!("unknown value: {v} ({})", Self::NAMES))
            }
        }
    }

    fn show(&self) -> String {
        self.as_str().to_owned()
    }
}

/// The window options a pane may also set for itself (`set -p`), as tmux's
/// option table scopes them.
pub const PANE_OPTIONS: &[&str] = &[
    "allow-passthrough",
    "allow-rename",
    "allow-set-title",
    "alternate-screen",
    "cursor-style",
    "pane-border-format",
    "pane-scrollbars-style",
    "remain-on-exit",
    "remain-on-exit-format",
    "synchronize-panes",
    "window-active-style",
    "window-style",
];

/// Copy mode's and the command prompt's key set (tmux's `mode-keys`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModeKeys {
    Vi,
    Emacs,
}

impl ModeKeys {
    pub const NAMES: &str = "vi or emacs";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "vi" => Some(Self::Vi),
            "emacs" => Some(Self::Emacs),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Vi => "vi",
            Self::Emacs => "emacs",
        }
    }
}

/// How a window is sized: by the client that used it last (tmux's `latest`,
/// the only automatic sizing tmxr has), or only by `resize-window`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowSize {
    /// The size of the client that used the session last.
    Latest,
    /// The widest and the tallest of the session's clients.
    Largest,
    /// The narrowest and the shortest of the session's clients.
    Smallest,
    /// Only `resize-window` sizes the window.
    Manual,
}

impl WindowSize {
    pub const NAMES: &str = "largest, smallest, manual or latest";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "latest" => Some(Self::Latest),
            "largest" => Some(Self::Largest),
            "smallest" => Some(Self::Smallest),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Latest => "latest",
            Self::Largest => "largest",
            Self::Smallest => "smallest",
            Self::Manual => "manual",
        }
    }
}

/// tmux's box lines, for menus' and popups' borders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BoxLines {
    Single,
    Rounded,
    Double,
    Heavy,
    /// ASCII: `+`, `|` and `-`.
    Simple,
    /// Blank cells, the box keeping its size.
    Padded,
    None,
}

/// `pane-border-status`: where each pane's status line goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneStatus {
    Off,
    Top,
    Bottom,
}

impl PaneStatus {
    pub const NAMES: &str = "off, top or bottom";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }
}

/// `pane-border-indicators`: how the active pane's border shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneIndicators {
    Off,
    /// In a window of two panes, only the half of the border nearer it in
    /// `pane-active-border-style`.
    Colour,
    /// Arrows on its borders, pointing at it.
    Arrows,
    Both,
}

impl PaneIndicators {
    pub const NAMES: &str = "off, colour, arrows or both";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "colour" => Some(Self::Colour),
            "arrows" => Some(Self::Arrows),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Colour => "colour",
            Self::Arrows => "arrows",
            Self::Both => "both",
        }
    }
}

/// `pane-border-lines`: what tmux draws pane borders with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaneLines {
    Single,
    Double,
    Heavy,
    /// ASCII: `|`, `-` and `+`.
    Simple,
    /// Each border cell the index of its pane.
    Number,
    /// Blank cells.
    Spaces,
}

impl PaneLines {
    pub const NAMES: &str = "single, double, heavy, simple, number or spaces";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "single" => Some(Self::Single),
            "double" => Some(Self::Double),
            "heavy" => Some(Self::Heavy),
            "simple" => Some(Self::Simple),
            "number" => Some(Self::Number),
            "spaces" => Some(Self::Spaces),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Double => "double",
            Self::Heavy => "heavy",
            Self::Simple => "simple",
            Self::Number => "number",
            Self::Spaces => "spaces",
        }
    }
}

impl BoxLines {
    pub const NAMES: &str = "single, rounded, double, heavy, simple, padded or none";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "single" => Some(Self::Single),
            "rounded" => Some(Self::Rounded),
            "double" => Some(Self::Double),
            "heavy" => Some(Self::Heavy),
            "simple" => Some(Self::Simple),
            "padded" => Some(Self::Padded),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Rounded => "rounded",
            Self::Double => "double",
            Self::Heavy => "heavy",
            Self::Simple => "simple",
            Self::Padded => "padded",
            Self::None => "none",
        }
    }
}

options_table! {
    /// The window options every window has unless it (or its pane) sets its
    /// own: tmux's global window options (`set -wg`).
    table WindowOptions;
    /// The options one window or pane sets itself (`set -w`, `set -p`), over
    /// what it inherits, and its own `@name` user options.
    own OwnOptions;
    /// Size the window only by the clients showing it, not every client
    /// whose session holds it.
    "aggressive-resize" => aggressive_resize: bool,
    /// Forward programs' `ESC P tmux; … ESC \` passthrough to the outer
    /// terminal (inline images and the like).
    "allow-passthrough" => allow_passthrough: bool,
    /// Programs may rename their window (`ESC k name ESC \`).
    "allow-rename" => allow_rename: bool,
    /// Programs may title their pane (OSC 0 / 2).
    "allow-set-title" => allow_set_title: bool,
    /// Programs may switch to the alternate screen; off, their switches
    /// are ignored and what they draw stays in the history.
    "alternate-screen" => alternate_screen: bool,
    /// Follow the active pane's command until the window is renamed.
    "automatic-rename" => automatic_rename: bool,
    /// What `automatic-rename` names the window, expanded for its active
    /// pane.
    "automatic-rename-format" => automatic_rename_format: String,
    /// The colour of clock mode's digits.
    "clock-mode-colour" => clock_mode_colour: Word,
    /// How clock mode tells the time: 12 or 24 hours, with seconds or not.
    "clock-mode-style" => clock_mode_style: ClockModeStyle,
    /// Copy mode search matches under the cursor, and the others.
    "copy-mode-current-match-style" => copy_mode_current_match_style: String,
    "copy-mode-match-style" => copy_mode_match_style: String,
    /// A pane's cursor shape until its program asks for one.
    "cursor-style" => cursor_style: CursorStyle,
    /// What fills the client beyond a smaller window; empty, a dot.
    "fill-character" => fill_character: String,
    /// main-horizontal / main-vertical sizes: cells or `N%`.
    "main-pane-height" => main_pane_height: String,
    "main-pane-width" => main_pane_width: String,
    /// Menus' looks, for a menu whose command gives none.
    "menu-border-lines" => menu_border_lines: BoxLines,
    "menu-border-style" => menu_border_style: String,
    "menu-selected-style" => menu_selected_style: String,
    "menu-style" => menu_style: String,
    "mode-keys" => mode_keys: ModeKeys,
    /// Copy mode's selection.
    "mode-style" => mode_style: String,
    /// Flag a window that prints while not current.
    "monitor-activity" => monitor_activity: bool,
    /// Flag a window whose program rings the bell.
    "monitor-bell" => monitor_bell: bool,
    /// Flag a window quiet for this many seconds; 0 is off.
    "monitor-silence" => monitor_silence: u64,
    "other-pane-height" => other_pane_height: String,
    "other-pane-width" => other_pane_width: String,
    "pane-active-border-style" => pane_active_border_style: String,
    "pane-base-index" => pane_base_index: u32,
    /// What each pane's status line shows (`pane-border-status`).
    "pane-border-format" => pane_border_format: String,
    /// How the active pane's border shows it.
    "pane-border-indicators" => pane_border_indicators: PaneIndicators,
    /// What pane borders are drawn with.
    "pane-border-lines" => pane_border_lines: PaneLines,
    /// A status line on each pane's top or bottom border.
    "pane-border-status" => pane_border_status: PaneStatus,
    "pane-border-style" => pane_border_style: String,
    /// When panes show a scrollbar, on which side, and in what style (which
    /// also takes `width=N` and `pad=N`).
    "pane-scrollbars" => pane_scrollbars: Scrollbars,
    "pane-scrollbars-position" => pane_scrollbars_position: ScrollbarPosition,
    "pane-scrollbars-style" => pane_scrollbars_style: String,
    /// The current pane's and the other panes' styles in tmux's default
    /// second status line (`status-format[1]`).
    "pane-status-current-style" => pane_status_current_style: String,
    "pane-status-style" => pane_status_style: String,
    /// Popups' looks, for a popup whose command gives none.
    "popup-border-lines" => popup_border_lines: BoxLines,
    "popup-border-style" => popup_border_style: String,
    "popup-style" => popup_style: String,
    /// Keep a pane whose program exited, shown as dead, until it is
    /// respawned or killed: always, or only when it failed.
    "remain-on-exit" => remain_on_exit: RemainOnExit,
    /// The line written into a pane `remain-on-exit` keeps, after a
    /// linefeed at its bottom; empty, none.
    "remain-on-exit-format" => remain_on_exit_format: String,
    /// The current session's and the other sessions' styles in tmux's
    /// default third status line (`status-format[2]`).
    "session-status-current-style" => session_status_current_style: String,
    "session-status-style" => session_status_style: String,
    /// Send what is typed in a pane to all of its window's panes.
    "synchronize-panes" => synchronize_panes: bool,
    /// The colours the window's active pane takes where its program sets
    /// none, over `window-style`'s; `default`, none of its own.
    "window-active-style" => window_active_style: String,
    "window-size" => window_size: WindowSize,
    "window-status-current-format" => window_status_current_format: String,
    /// The style the current window's entry is drawn over; `default` takes
    /// `window-status-style`.
    "window-status-current-style" => window_status_current_style: String,
    "window-status-format" => window_status_format: String,
    /// Added to the last window's entry's style, unless `default`.
    "window-status-last-style" => window_status_last_style: String,
    /// Drawn between window entries, not after the last.
    "window-status-separator" => window_status_separator: String,
    /// The style each window's entry is drawn over, on the status line's.
    "window-status-style" => window_status_style: String,
    /// The colours a pane takes where its program sets none; `default`,
    /// the terminal's.
    "window-style" => window_style: String,
    /// A copy-mode search goes on from the other end when it reaches one.
    "wrap-search" => wrap_search: bool,
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    #[test]
    fn options_are_set_and_shown_by_tmux_name() {
        let mut o = crate::defaults().window;
        o.set("mode-keys", Some("emacs"), false).unwrap();
        assert_eq!(o.mode_keys, ModeKeys::Emacs);
        assert_eq!(o.show("mode-keys").as_deref(), Some("emacs"));
        // A flag without a value toggles; -a appends to a string.
        let before = o.synchronize_panes;
        o.set("synchronize-panes", None, false).unwrap();
        assert_eq!(o.synchronize_panes, !before);
        o.set("main-pane-width", Some("1"), true).unwrap();
        assert_eq!(o.main_pane_width, "801");
        o.set("pane-base-index", Some("1"), true).unwrap();
        assert_eq!(o.pane_base_index, 1);
    }

    #[test]
    fn cursor_styles_are_decscusrs_numbers() {
        let ps = |s| CursorStyle::parse(s).unwrap().ps();
        assert_eq!(ps("default"), 0);
        assert_eq!(ps("blinking-block"), 1);
        assert_eq!(ps("underline"), 4);
        assert_eq!(ps("bar"), 6);
    }

    #[test]
    fn clock_text_is_tmuxs() {
        let t = |style: &str, h, m, s| ClockModeStyle::parse(style).unwrap().text(h, m, s);
        assert_eq!(t("24", 9, 5, 7), "09:05");
        assert_eq!(t("24-with-seconds", 9, 5, 7), "09:05:07");
        assert_eq!(t("12", 15, 4, 9), " 3:04 PM");
        assert_eq!(t("12", 0, 30, 0), "12:30 AM");
        assert_eq!(t("12", 12, 0, 0), "12:00 PM");
        assert_eq!(t("12-with-seconds", 23, 59, 58), "11:59:58 PM");
    }

    #[test]
    fn bad_values_name_the_option() {
        let mut o = crate::defaults().window;
        let before = o.clone();
        let err = |o: &mut WindowOptions, name, v| o.set(name, v, false).unwrap_err();
        assert_eq!(
            err(&mut o, "mode-keys", Some("x")),
            "mode-keys: unknown value: x (vi or emacs)"
        );
        assert_eq!(
            err(&mut o, "monitor-silence", Some("x")),
            "monitor-silence: bad number x"
        );
        assert_eq!(
            err(&mut o, "remain-on-exit", Some("x")),
            "remain-on-exit: unknown value: x (off, on or failed)"
        );
        assert_eq!(err(&mut o, "menu-style", None), "menu-style: needs a value");
        assert_eq!(
            err(&mut o, "status-left", Some("x")),
            "invalid option: status-left"
        );
        assert_eq!(o, before);
    }

    #[test]
    fn own_options_override_what_is_inherited_until_unset() {
        let global = crate::defaults().window;
        let mut own = OwnOptions::default();
        assert!(matches!(own.over(Cow::Borrowed(&global)), Cow::Borrowed(_)));
        own.set("mode-keys", Some("emacs"), false, &global).unwrap();
        // A flag toggles, and -a appends to, the inherited value.
        own.set("remain-on-exit", None, false, &global).unwrap();
        own.set("main-pane-width", Some("0"), true, &global)
            .unwrap();
        let o = own.over(Cow::Borrowed(&global));
        assert_eq!(o.mode_keys, ModeKeys::Emacs);
        assert_eq!(o.remain_on_exit, RemainOnExit::On);
        assert_eq!(global.remain_on_exit, RemainOnExit::Off);
        assert_eq!(o.main_pane_width, format!("{}0", global.main_pane_width));
        assert_eq!(o.menu_style, global.menu_style);
        assert_eq!(own.show("mode-keys").as_deref(), Some("emacs"));
        assert_eq!(own.show("menu-style"), None);
        own.unset("mode-keys");
        assert_eq!(own.over(Cow::Borrowed(&global)).mode_keys, global.mode_keys);
        assert_eq!(own.show("mode-keys"), None);
        // A bad value changes nothing.
        let before = own.clone();
        assert!(
            own.set("pane-scrollbars", Some("x"), false, &global)
                .is_err()
        );
        assert_eq!(own, before);
    }

    #[test]
    fn own_user_options_append_to_their_own_value() {
        let global = crate::defaults().window;
        let mut own = OwnOptions::default();
        own.set("@x", Some("a"), true, &global).unwrap();
        own.set("@x", Some("b"), true, &global).unwrap();
        assert_eq!(own.show("@x").as_deref(), Some("ab"));
        own.set("@x", Some("c"), false, &global).unwrap();
        assert_eq!(own.user().collect::<Vec<_>>(), [("@x", "c")]);
        assert_eq!(
            own.set("@y", None, false, &global).unwrap_err(),
            "empty value"
        );
        own.unset("@x");
        assert_eq!(own.show("@x"), None);
    }

    #[test]
    fn pane_options_are_window_options() {
        for n in PANE_OPTIONS {
            assert!(WindowOptions::NAMES.contains(n), "{n}");
        }
    }

    #[test]
    fn every_name_is_shown_in_tmux_order() {
        let o = crate::defaults().window;
        for n in WindowOptions::NAMES {
            assert!(o.show(n).is_some(), "{n}");
        }
        let mut sorted = WindowOptions::NAMES.to_vec();
        sorted.sort_unstable();
        assert_eq!(WindowOptions::NAMES, sorted);
    }
}
