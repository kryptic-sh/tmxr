//! What every tmux option table is made of: how a value is read by
//! `set-option` and printed by `show-options`, and the macro that makes a
//! table (the global values) with the overrides one object sets over it.

use crate::{BoxLines, ModeKeys, ScrollbarPosition, Scrollbars, WindowSize};

/// An option's value: how `set-option` reads it and `show-options` prints
/// it.
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

/// `value` read for option `name`, now `current`: errors name the option, and
/// `-a` (`append`) adds to a string.
pub fn parse<T: OptionValue>(
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
                        let current = self.$field.as_ref().unwrap_or(&inherited.$field);
                        self.$field = Some($crate::table::parse(name, value, append, current)?);
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
