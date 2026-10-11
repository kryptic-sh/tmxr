//! tmux's window options: one typed table, read by `set-option` and printed
//! by `show-options` under tmux's names.

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{ScrollbarPosition, Scrollbars};

/// The window options a pane may also set for itself (`set -p`), as tmux's
/// option table scopes them.
pub const PANE_OPTIONS: &[&str] = &[
    "allow-passthrough",
    "pane-scrollbars-style",
    "remain-on-exit",
    "synchronize-panes",
];

/// A window option's value: how `set-option` reads it and `show-options`
/// prints it.
pub trait OptionValue: Clone {
    /// Whether `set-option -a` appends to it (tmux's string options).
    const APPENDS: bool = false;

    /// `value` for an option now `current`; `None` is no value given.
    fn parse(value: Option<&str>, current: &Self) -> Result<Self, String>;

    fn show(&self) -> String;
}

impl OptionValue for bool {
    /// tmux's flags: no value toggles.
    fn parse(value: Option<&str>, current: &Self) -> Result<Self, String> {
        match value {
            None | Some("toggle") => Ok(!current),
            Some("on" | "1" | "yes" | "true") => Ok(true),
            Some("off" | "0" | "no" | "false") => Ok(false),
            Some(v) => Err(format!("bad value: {v}")),
        }
    }

    fn show(&self) -> String {
        if *self { "on" } else { "off" }.to_owned()
    }
}

impl OptionValue for u32 {
    fn parse(value: Option<&str>, _: &Self) -> Result<Self, String> {
        let v = value.ok_or("needs a value")?;
        v.parse().map_err(|_| format!("bad number {v}"))
    }

    fn show(&self) -> String {
        self.to_string()
    }
}

impl OptionValue for u64 {
    fn parse(value: Option<&str>, _: &Self) -> Result<Self, String> {
        let v = value.ok_or("needs a value")?;
        v.parse().map_err(|_| format!("bad number {v}"))
    }

    fn show(&self) -> String {
        self.to_string()
    }
}

impl OptionValue for String {
    const APPENDS: bool = true;

    fn parse(value: Option<&str>, _: &Self) -> Result<Self, String> {
        value
            .map(str::to_owned)
            .ok_or_else(|| "needs a value".into())
    }

    fn show(&self) -> String {
        self.clone()
    }
}

/// tmux's choice options: one of a few names.
macro_rules! choice {
    ($($ty:ty),*) => {$(
        impl OptionValue for $ty {
            fn parse(value: Option<&str>, _: &Self) -> Result<Self, String> {
                let v = value.ok_or("needs a value")?;
                Self::parse(v).ok_or_else(|| format!("unknown value: {v} ({})", Self::NAMES))
            }

            fn show(&self) -> String {
                self.as_str().to_owned()
            }
        }
    )*};
}

choice!(
    Scrollbars,
    ScrollbarPosition,
    ModeKeys,
    WindowSize,
    BoxLines
);

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
    Latest,
    Manual,
}

impl WindowSize {
    pub const NAMES: &str = "latest or manual";

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "latest" => Some(Self::Latest),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Latest => "latest",
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

/// `value` read for option `name`, now `current`: errors name the option, and
/// `-a` (`append`) adds to a string.
fn parse<T: OptionValue>(
    name: &str,
    value: Option<&str>,
    append: bool,
    current: &T,
) -> Result<T, String> {
    let appended;
    let value = match value {
        Some(v) if append && T::APPENDS => {
            appended = format!("{}{v}", current.show());
            Some(appended.as_str())
        }
        v => v,
    };
    T::parse(value, current).map_err(|e| format!("{name}: {e}"))
}

macro_rules! window_options {
    ($($(#[$doc:meta])* $name:literal => $field:ident: $ty:ty,)*) => {
        /// The window options every window has unless it (or its pane) sets
        /// its own: tmux's global window options (`set -wg`).
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct WindowOptions {
            $($(#[$doc])* pub $field: $ty,)*
        }

        impl WindowOptions {
            /// Every window option by tmux name, in tmux's order.
            pub const NAMES: &[&str] = &[$($name),*];

            /// `name`'s value as `show-options` prints it; `None` for a name
            /// that is not a window option.
            pub fn show(&self, name: &str) -> Option<String> {
                match name {
                    $($name => Some(self.$field.show()),)*
                    _ => None,
                }
            }

            /// Set `name` from `set-option`'s `value`; `append` is `-a`.
            pub fn set(&mut self, name: &str, value: Option<&str>, append: bool) -> Result<(), String> {
                match name {
                    $($name => self.$field = parse(name, value, append, &self.$field)?,)*
                    _ => return Err(format!("invalid option: {name}")),
                }
                Ok(())
            }
        }

        /// The options one window or pane sets itself (`set -w`, `set -p`),
        /// over what it inherits, and its own `@name` user options.
        #[derive(Debug, Clone, Default, PartialEq, Eq)]
        pub struct OwnOptions {
            $(pub $field: Option<$ty>,)*
            user: BTreeMap<String, String>,
        }

        impl OwnOptions {
            /// Set `name` here from `set-option`'s `value`. A flag toggles and
            /// `-a` (`append`) appends from `inherited`, the value without
            /// this one, as tmux's; a user option appends to its own value
            /// only.
            pub fn set(
                &mut self,
                name: &str,
                value: Option<&str>,
                append: bool,
                inherited: &WindowOptions,
            ) -> Result<(), String> {
                if name.starts_with('@') {
                    let value = value.ok_or("empty value")?;
                    let own = self.user.entry(name.to_owned()).or_default();
                    if !append {
                        own.clear();
                    }
                    own.push_str(value);
                    return Ok(());
                }
                match name {
                    $($name => {
                        let current = self.$field.as_ref().unwrap_or(&inherited.$field);
                        self.$field = Some(parse(name, value, append, current)?);
                    })*
                    _ => return Err(format!("invalid option: {name}")),
                }
                Ok(())
            }

            /// Forget `name` here (`set -u`), so the inherited value shows.
            pub fn unset(&mut self, name: &str) {
                match name {
                    $($name => self.$field = None,)*
                    _ => {
                        self.user.remove(name);
                    }
                }
            }

            /// `name`'s own value as `show-options` prints it; `None` when
            /// it is inherited.
            pub fn show(&self, name: &str) -> Option<String> {
                match name {
                    $($name => self.$field.as_ref().map(OptionValue::show),)*
                    _ => self.user.get(name).cloned(),
                }
            }

            /// The user options set here, by name.
            pub fn user(&self) -> impl Iterator<Item = (&str, &str)> {
                self.user.iter().map(|(k, v)| (k.as_str(), v.as_str()))
            }

            /// `base` with the options set here over it; borrowed still when
            /// none are.
            pub fn over<'a>(&self, base: Cow<'a, WindowOptions>) -> Cow<'a, WindowOptions> {
                let mut base = base;
                $(if let Some(v) = &self.$field {
                    base.to_mut().$field = v.clone();
                })*
                base
            }
        }
    };
}

window_options! {
    /// Forward programs' `ESC P tmux; … ESC \` passthrough to the outer
    /// terminal (inline images and the like).
    "allow-passthrough" => allow_passthrough: bool,
    /// Follow the active pane's command until the window is renamed.
    "automatic-rename" => automatic_rename: bool,
    /// The colour of clock mode's digits.
    "clock-mode-colour" => clock_mode_colour: String,
    /// Copy mode search matches under the cursor, and the others.
    "copy-mode-current-match-style" => copy_mode_current_match_style: String,
    "copy-mode-match-style" => copy_mode_match_style: String,
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
    /// Flag a window quiet for this many seconds; 0 is off.
    "monitor-silence" => monitor_silence: u64,
    "other-pane-height" => other_pane_height: String,
    "other-pane-width" => other_pane_width: String,
    "pane-active-border-style" => pane_active_border_style: String,
    "pane-base-index" => pane_base_index: u32,
    "pane-border-style" => pane_border_style: String,
    /// When panes show a scrollbar, on which side, and in what style (which
    /// also takes `width=N` and `pad=N`).
    "pane-scrollbars" => pane_scrollbars: Scrollbars,
    "pane-scrollbars-position" => pane_scrollbars_position: ScrollbarPosition,
    "pane-scrollbars-style" => pane_scrollbars_style: String,
    /// Popups' looks, for a popup whose command gives none.
    "popup-border-lines" => popup_border_lines: BoxLines,
    "popup-border-style" => popup_border_style: String,
    "popup-style" => popup_style: String,
    /// Keep a pane whose program exited, shown as dead, until it is
    /// respawned or killed.
    "remain-on-exit" => remain_on_exit: bool,
    /// Send what is typed in a pane to all of its window's panes.
    "synchronize-panes" => synchronize_panes: bool,
    "window-size" => window_size: WindowSize,
    "window-status-current-format" => window_status_current_format: String,
    "window-status-format" => window_status_format: String,
}

#[cfg(test)]
mod tests {
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
            "remain-on-exit: bad value: x"
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
        assert_eq!(o.remain_on_exit, !global.remain_on_exit);
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
