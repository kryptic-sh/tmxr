//! What every tmux option table is made of: how a value is read by
//! `set-option` and printed by `show-options`, and the macro that makes a
//! table (the global values) with the overrides one object sets over it.

use std::collections::BTreeMap;

use crate::{
    AlertAction, BoxLines, ClockModeStyle, CursorStyle, DestroyUnattached, ModeKeys,
    PaneIndicators, PaneLines, PaneStatus, ScrollbarPosition, Scrollbars, StatusJustify,
    StatusLines, StatusPosition, Visual, WindowSize,
};

/// An option's value: how `set-option` reads it and `show-options` prints
/// it.
pub trait OptionValue: Clone {
    /// Whether it is one of tmux's string options: `set-option -a` appends
    /// to it and `show-options` quotes it.
    const STRING: bool = false;

    /// Whether it is one of tmux's array options, set and shown by index
    /// (`name[3]`).
    const ARRAY: bool = false;

    /// `value` for an option now `current`; `None` is no value given.
    fn parse(value: Option<&str>, current: &Self) -> Result<Self, String>;

    /// `value` added to `self` (`set-option -a`): a string is appended to,
    /// an array gains items, anything else is just set.
    fn append(&self, value: &str) -> Result<Self, String> {
        Self::parse(Some(value), self)
    }

    /// The value an object starts from when it sets this option itself,
    /// `inherited` being the one it had: that one, but an empty array, as
    /// tmux's `options_empty`.
    fn own_start(inherited: &Self) -> Self {
        inherited.clone()
    }

    /// `self` with item `index` set to `value` (`-a` appends to it), or
    /// removed with `None`; an error for an option that is not an array.
    fn set_item(&self, index: u32, value: Option<&str>, append: bool) -> Result<Self, String> {
        let _ = (index, value, append);
        Err("not an array".into())
    }

    /// An array's items by index; `None` for an option that is not one.
    fn items(&self) -> Option<&BTreeMap<u32, String>> {
        None
    }

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

/// tmux's number options.
macro_rules! number {
    ($($ty:ty),*) => {$(
        impl OptionValue for $ty {
            fn parse(value: Option<&str>, _: &Self) -> Result<Self, String> {
                let v = value.ok_or("needs a value")?;
                v.parse().map_err(|_| format!("bad number {v}"))
            }

            fn show(&self) -> String {
                self.to_string()
            }
        }
    )*};
}

number!(u16, u32, u64, usize);

impl OptionValue for String {
    const STRING: bool = true;

    fn parse(value: Option<&str>, _: &Self) -> Result<Self, String> {
        value
            .map(str::to_owned)
            .ok_or_else(|| "needs a value".into())
    }

    fn append(&self, value: &str) -> Result<Self, String> {
        Ok(format!("{self}{value}"))
    }

    fn show(&self) -> String {
        self.clone()
    }
}

/// A string option that may have no value, shown as empty (`default-shell`,
/// whose absence means the platform's shell).
impl OptionValue for Option<String> {
    const STRING: bool = true;

    fn parse(value: Option<&str>, _: &Self) -> Result<Self, String> {
        value
            .map(|v| Some(v.to_owned()))
            .ok_or_else(|| "needs a value".into())
    }

    fn append(&self, value: &str) -> Result<Self, String> {
        Ok(Some(format!("{}{value}", self.show())))
    }

    fn show(&self) -> String {
        self.clone().unwrap_or_default()
    }
}

/// An option tmux types as a colour or a key, kept by its name: unlike a
/// string, `show-options` does not quote it and `-a` does not append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word(pub String);

impl std::ops::Deref for Word {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Word {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl OptionValue for Word {
    fn parse(value: Option<&str>, _: &Self) -> Result<Self, String> {
        value
            .map(|v| Self(v.to_owned()))
            .ok_or_else(|| "needs a value".into())
    }

    fn show(&self) -> String {
        self.0.clone()
    }
}

/// One of tmux's string array options (`update-environment`): items by
/// index, which may leave gaps. Set whole, it takes the words of a string
/// split at spaces and commas, as tmux's `options_array_assign`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Array(BTreeMap<u32, String>);

impl Array {
    /// The items in index order.
    pub fn values(&self) -> impl Iterator<Item = &str> {
        self.0.values().map(String::as_str)
    }

    /// Each word of `s` into the first free index, as tmux's
    /// `options_array_assign` with its default separators.
    fn assign(&mut self, s: &str) {
        for word in s.split([' ', ',']).filter(|w| !w.is_empty()) {
            let free = (0..).find(|i| !self.0.contains_key(i)).unwrap_or(u32::MAX);
            self.0.insert(free, word.to_owned());
        }
    }
}

impl<S: Into<String>> FromIterator<S> for Array {
    fn from_iter<I: IntoIterator<Item = S>>(items: I) -> Self {
        Self((0..).zip(items.into_iter().map(Into::into)).collect())
    }
}

impl OptionValue for Array {
    const STRING: bool = true;
    const ARRAY: bool = true;

    fn parse(value: Option<&str>, _: &Self) -> Result<Self, String> {
        let mut a = Self::default();
        a.assign(value.ok_or("empty value")?);
        Ok(a)
    }

    fn append(&self, value: &str) -> Result<Self, String> {
        let mut a = self.clone();
        a.assign(value);
        Ok(a)
    }

    fn own_start(_: &Self) -> Self {
        Self::default()
    }

    fn set_item(&self, index: u32, value: Option<&str>, append: bool) -> Result<Self, String> {
        let mut a = self.clone();
        match value {
            None => {
                a.0.remove(&index);
            }
            Some(v) => {
                let item = a.0.entry(index).or_default();
                if !append {
                    item.clear();
                }
                item.push_str(v);
            }
        }
        Ok(a)
    }

    fn items(&self) -> Option<&BTreeMap<u32, String>> {
        Some(&self.0)
    }

    /// The items joined by spaces, as tmux's `options_to_string`.
    fn show(&self) -> String {
        self.values().collect::<Vec<_>>().join(" ")
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
    StatusLines,
    StatusJustify,
    StatusPosition,
    ClockModeStyle,
    CursorStyle,
    DestroyUnattached,
    Scrollbars,
    ScrollbarPosition,
    ModeKeys,
    WindowSize,
    BoxLines,
    PaneIndicators,
    PaneLines,
    PaneStatus,
    AlertAction,
    Visual
);

/// `value` read for option `name`, now `current`: errors name the option, and
/// `-a` (`append`) adds to a string or an array.
pub fn parse<T: OptionValue>(
    name: &str,
    value: Option<&str>,
    append: bool,
    current: &T,
) -> Result<T, String> {
    match value {
        Some(v) if append => current.append(v),
        v => T::parse(v, current),
    }
    .map_err(|e| format!("{name}: {e}"))
}

/// `current` with item `index` of option `name` set (`-a` appends to it) or,
/// `None`, removed; errors name the option.
pub fn item<T: OptionValue>(
    name: &str,
    index: u32,
    value: Option<&str>,
    append: bool,
    current: &T,
) -> Result<T, String> {
    current
        .set_item(index, value, append)
        .map_err(|e| format!("{e}: {name}[{index}]"))
}

macro_rules! options_table {
    (
        $(#[$table_doc:meta])* table $table:ident;
        $(#[$own_doc:meta])* own $own:ident;
        $($(#[$doc:meta])* $name:literal => $field:ident: $ty:ty,)*
    ) => {
        $(#[$table_doc])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $table {
            $($(#[$doc])* pub $field: $ty,)*
        }

        impl $table {
            /// Every option in the table by tmux name, in tmux's order.
            pub const NAMES: &[&str] = &[$($name),*];

            /// Whether `name` is in the table as one of tmux's string
            /// options, which `show-options` quotes.
            pub fn is_string(name: &str) -> bool {
                match name {
                    $($name => <$ty as $crate::table::OptionValue>::STRING,)*
                    _ => false,
                }
            }

            /// Whether `name` is in the table as one of tmux's array
            /// options.
            pub fn is_array(name: &str) -> bool {
                match name {
                    $($name => <$ty as $crate::table::OptionValue>::ARRAY,)*
                    _ => false,
                }
            }

            /// Array option `name`'s items by index; `None` for a name
            /// that is not an array in the table.
            pub fn items(&self, name: &str) -> Option<&::std::collections::BTreeMap<u32, String>> {
                match name {
                    $($name => $crate::table::OptionValue::items(&self.$field),)*
                    _ => None,
                }
            }

            /// Set item `index` of array option `name` (`-a` appends to
            /// it), or with `None` remove it.
            pub fn set_item(
                &mut self,
                name: &str,
                index: u32,
                value: Option<&str>,
                append: bool,
            ) -> Result<(), String> {
                match name {
                    $($name => self.$field = $crate::table::item(name, index, value, append, &self.$field)?,)*
                    _ => return Err(format!("invalid option: {name}")),
                }
                Ok(())
            }

            /// `name`'s value as `show-options` prints it; `None` for a name
            /// that is not in the table.
            pub fn show(&self, name: &str) -> Option<String> {
                match name {
                    $($name => Some($crate::table::OptionValue::show(&self.$field)),)*
                    _ => None,
                }
            }

            /// Set `name` from `set-option`'s `value`; `append` is `-a`.
            pub fn set(&mut self, name: &str, value: Option<&str>, append: bool) -> Result<(), String> {
                match name {
                    $($name => self.$field = $crate::table::parse(name, value, append, &self.$field)?,)*
                    _ => return Err(format!("invalid option: {name}")),
                }
                Ok(())
            }
        }

        $(#[$own_doc])*
        #[derive(Debug, Clone, Default, PartialEq, Eq)]
        pub struct $own {
            $(pub $field: Option<$ty>,)*
            user: ::std::collections::BTreeMap<String, String>,
        }

        impl $own {
            /// Set `name` here from `set-option`'s `value`. A flag toggles and
            /// `-a` (`append`) appends from `inherited`, the value without
            /// this one, as tmux's; a user option appends to its own value
            /// only.
            pub fn set(
                &mut self,
                name: &str,
                value: Option<&str>,
                append: bool,
                inherited: &$table,
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
                        let start;
                        let current = match &self.$field {
                            Some(v) => v,
                            None => {
                                start = $crate::table::OptionValue::own_start(&inherited.$field);
                                &start
                            }
                        };
                        self.$field = Some($crate::table::parse(name, value, append, current)?);
                    })*
                    _ => return Err(format!("invalid option: {name}")),
                }
                Ok(())
            }

            /// Set item `index` of array option `name` here (`-a` appends to
            /// it), starting from an empty array when this object has none
            /// of its own, as tmux's `options_empty`.
            pub fn set_item(
                &mut self,
                name: &str,
                index: u32,
                value: &str,
                append: bool,
                inherited: &$table,
            ) -> Result<(), String> {
                match name {
                    $($name => {
                        let start;
                        let current = match &self.$field {
                            Some(v) => v,
                            None => {
                                start = $crate::table::OptionValue::own_start(&inherited.$field);
                                &start
                            }
                        };
                        self.$field = Some($crate::table::item(name, index, Some(value), append, current)?);
                    })*
                    _ => return Err(format!("invalid option: {name}")),
                }
                Ok(())
            }

            /// Remove item `index` of array option `name` here (`set -u
            /// name[index]`); nothing when this object has no value of its
            /// own, as tmux's.
            pub fn unset_item(&mut self, name: &str, index: u32) -> Result<(), String> {
                match name {
                    $($name => if let Some(v) = &self.$field {
                        self.$field = Some($crate::table::item(name, index, None, false, v)?);
                    })*
                    _ => return Err(format!("invalid option: {name}")),
                }
                Ok(())
            }

            /// Array option `name`'s own items by index; `None` when it is
            /// inherited or not an array.
            pub fn items(&self, name: &str) -> Option<&::std::collections::BTreeMap<u32, String>> {
                match name {
                    $($name => self.$field.as_ref().and_then($crate::table::OptionValue::items),)*
                    _ => None,
                }
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
                    $($name => self.$field.as_ref().map($crate::table::OptionValue::show),)*
                    _ => self.user.get(name).cloned(),
                }
            }

            /// The user options set here, by name.
            pub fn user(&self) -> impl Iterator<Item = (&str, &str)> {
                self.user.iter().map(|(k, v)| (k.as_str(), v.as_str()))
            }

            /// `base` with the options set here over it; borrowed still when
            /// none are.
            pub fn over<'a>(
                &self,
                base: ::std::borrow::Cow<'a, $table>,
            ) -> ::std::borrow::Cow<'a, $table> {
                let mut base = base;
                $(if let Some(v) = &self.$field {
                    base.to_mut().$field = v.clone();
                })*
                base
            }
        }
    };
}

pub(crate) use options_table;
