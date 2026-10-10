//! What a key table binds: a keyboard [`Key`] or one of tmux's mouse keys
//! (`MouseDown1Pane`, `WheelUpPane`, `MouseDragEnd1Pane`, …).

use std::fmt;
use std::str::FromStr;

use crossterm::event::KeyModifiers;

use crate::keys::{Key, UnknownKey, split_mods, write_mods};

/// What happened, in tmux's mouse key names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseAction {
    /// A button went down: `MouseDown1…`.
    Down(u8),
    /// A button went up without a drag: `MouseUp1…`.
    Up(u8),
    /// The first motion with a button held: `MouseDrag1…`.
    Drag(u8),
    /// A button released at the end of a drag: `MouseDragEnd1…`.
    DragEnd(u8),
    /// A second press in the same cell soon after the first:
    /// `SecondClick1…`.
    SecondClick(u8),
    /// A second press that no third followed in time: `DoubleClick1…`,
    /// sent once the click time has passed.
    DoubleClick(u8),
    /// A third: `TripleClick1…`.
    TripleClick(u8),
    WheelUp,
    WheelDown,
}

/// Where it happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseLocation {
    Pane,
    Border,
    /// A window in the status line's window list.
    Status,
    /// The status line's `status-left` part.
    StatusLeft,
    /// The status line's `status-right` part.
    StatusRight,
    /// The status line outside both and the window list.
    StatusDefault,
    /// A pane's scrollbar above its slider, on it, and below it.
    ScrollbarUp,
    ScrollbarSlider,
    ScrollbarDown,
}

/// A mouse key, such as `MouseDown1Pane` or `M-WheelUpPane`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MouseKey {
    pub action: MouseAction,
    pub location: MouseLocation,
    pub mods: KeyModifiers,
}

/// Buttons tmux names: 1 left, 2 middle, 3 right.
const BUTTONS: std::ops::RangeInclusive<u8> = 1..=3;

/// Builds a button action from its button number.
type ButtonAction = fn(u8) -> MouseAction;

const ACTIONS: &[(&str, ButtonAction)] = &[
    // Longest first: `MouseDragEnd` starts with `MouseDrag`.
    ("MouseDragEnd", MouseAction::DragEnd),
    ("MouseDrag", MouseAction::Drag),
    ("MouseDown", MouseAction::Down),
    ("MouseUp", MouseAction::Up),
    ("SecondClick", MouseAction::SecondClick),
    ("DoubleClick", MouseAction::DoubleClick),
    ("TripleClick", MouseAction::TripleClick),
];

const LOCATIONS: &[(&str, MouseLocation)] = &[
    ("Pane", MouseLocation::Pane),
    ("Border", MouseLocation::Border),
    ("Status", MouseLocation::Status),
    ("StatusLeft", MouseLocation::StatusLeft),
    ("StatusRight", MouseLocation::StatusRight),
    ("StatusDefault", MouseLocation::StatusDefault),
    ("ScrollbarUp", MouseLocation::ScrollbarUp),
    ("ScrollbarSlider", MouseLocation::ScrollbarSlider),
    ("ScrollbarDown", MouseLocation::ScrollbarDown),
];

impl FromStr for MouseKey {
    type Err = UnknownKey;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || UnknownKey(s.to_owned());
        let (mods, rest) = split_mods(s);
        let (action, place) = if let Some(place) = rest.strip_prefix("WheelUp") {
            (MouseAction::WheelUp, place)
        } else if let Some(place) = rest.strip_prefix("WheelDown") {
            (MouseAction::WheelDown, place)
        } else {
            let (make, tail) = ACTIONS
                .iter()
                .find_map(|(name, make)| rest.strip_prefix(name).map(|t| (make, t)))
                .ok_or_else(err)?;
            let digit = tail.chars().next().ok_or_else(err)?;
            let button = digit
                .to_digit(10)
                .and_then(|d| u8::try_from(d).ok())
                .filter(|b| BUTTONS.contains(b))
                .ok_or_else(err)?;
            (make(button), &tail[1..])
        };
        let location = LOCATIONS
            .iter()
            .find(|(name, _)| *name == place)
            .map(|(_, l)| *l)
            .ok_or_else(err)?;
        Ok(Self {
            action,
            location,
            mods,
        })
    }
}

impl fmt::Display for MouseKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_mods(f, self.mods)?;
        match self.action {
            MouseAction::Down(b) => write!(f, "MouseDown{b}")?,
            MouseAction::Up(b) => write!(f, "MouseUp{b}")?,
            MouseAction::Drag(b) => write!(f, "MouseDrag{b}")?,
            MouseAction::DragEnd(b) => write!(f, "MouseDragEnd{b}")?,
            MouseAction::SecondClick(b) => write!(f, "SecondClick{b}")?,
            MouseAction::DoubleClick(b) => write!(f, "DoubleClick{b}")?,
            MouseAction::TripleClick(b) => write!(f, "TripleClick{b}")?,
            MouseAction::WheelUp => f.write_str("WheelUp")?,
            MouseAction::WheelDown => f.write_str("WheelDown")?,
        }
        let place = LOCATIONS
            .iter()
            .find(|(_, l)| *l == self.location)
            .map_or("Pane", |(name, _)| name);
        f.write_str(place)
    }
}

/// A key a key table can bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindKey {
    Key(Key),
    Mouse(MouseKey),
}

impl From<Key> for BindKey {
    fn from(k: Key) -> Self {
        Self::Key(k)
    }
}

impl From<MouseKey> for BindKey {
    fn from(m: MouseKey) -> Self {
        Self::Mouse(m)
    }
}

impl FromStr for BindKey {
    type Err = UnknownKey;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.parse::<MouseKey>() {
            Ok(m) => Ok(Self::Mouse(m)),
            Err(_) => s.parse::<Key>().map(Self::Key),
        }
    }
}

impl fmt::Display for BindKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(k) => k.fmt(f),
            Self::Mouse(m) => m.fmt(f),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_names_parse_and_print_back() {
        for name in [
            "MouseDown1Pane",
            "MouseUp3Pane",
            "MouseDrag1Border",
            "MouseDragEnd1Pane",
            "MouseDown2Status",
            "DoubleClick1Pane",
            "SecondClick2Pane",
            "TripleClick3Pane",
            "WheelUpPane",
            "WheelDownStatus",
            "MouseDown3StatusLeft",
            "MouseUp1StatusRight",
            "WheelUpStatusDefault",
            "M-MouseDown1Pane",
            "C-S-WheelUpPane",
        ] {
            let key: BindKey = name.parse().unwrap();
            assert!(matches!(key, BindKey::Mouse(_)), "{name}");
            assert_eq!(key.to_string(), name);
        }
        let k: MouseKey = "MouseDragEnd1Pane".parse().unwrap();
        assert_eq!(k.action, MouseAction::DragEnd(1));
        assert_eq!(k.location, MouseLocation::Pane);
    }

    #[test]
    fn bad_mouse_names_are_rejected_and_keys_still_parse() {
        for name in [
            "MouseDown4Pane",
            "MouseDown0Pane",
            "MouseDownPane",
            "MouseDown1",
            "MouseDown1Window",
            "WheelLeftPane",
            "Mouse",
        ] {
            assert!(name.parse::<BindKey>().is_err(), "{name}");
        }
        assert_eq!(
            "C-b".parse::<BindKey>(),
            Ok(BindKey::Key("C-b".parse().unwrap()))
        );
        assert_eq!(
            "M".parse::<BindKey>(),
            Ok(BindKey::Key("M".parse().unwrap()))
        );
    }
}
