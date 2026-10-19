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

// tmux 3.6's key codes (`tmux.h`): a character is its code point, a special
// key `KEYC_BASE` plus its place in the special keys' enum, and the
// modifiers bits above both.
const KEYC_BASE: u64 = 0x10_e000;
const KEYC_META: u64 = 1 << 44;
const KEYC_CTRL: u64 = 1 << 45;
const KEYC_SHIFT: u64 = 1 << 46;
/// The special keys before the mouse keys: focus in and out, any, the
/// paste brackets, and the three unclassified mouse events.
const FIRST_MOUSE: u64 = 8;
/// Each mouse event's places, in `KEYC_MOUSE_KEY`'s order.
const MOUSE_PLACES: u64 = 9;
/// The mouse events `tmux.h` lists, `MouseMove` first.
const MOUSE_EVENTS: u64 = 66;
/// The buttons each button event comes in: 1 to 3, then 6 to 11.
const BUTTON_SLOTS: u64 = 9;
/// `KEYC_BSPACE`, the first special key after the mouse keys; F1 to F12,
/// IC, DC, Home, End, NPage, PPage, BTab, Up, Down, Left and Right follow
/// it in that order.
const BSPACE: u64 = FIRST_MOUSE + MOUSE_EVENTS * MOUSE_PLACES;

fn modifier_bits(mods: KeyModifiers) -> u64 {
    let mut bits = 0;
    if mods.contains(KeyModifiers::ALT) {
        bits |= KEYC_META;
    }
    if mods.contains(KeyModifiers::CONTROL) {
        bits |= KEYC_CTRL;
    }
    if mods.contains(KeyModifiers::SHIFT) {
        bits |= KEYC_SHIFT;
    }
    bits
}

impl BindKey {
    /// The key code tmux 3.6 stores this binding under, with its flags
    /// masked off (key_bindings_add): what tmux lists a table's bindings in
    /// the order of (key_bindings_cmp). A key tmux has no code for, such
    /// as F13 and above, comes after every one it has.
    pub fn tmux_code(&self) -> u64 {
        match self {
            Self::Key(k) => k.tmux_code(),
            Self::Mouse(m) => m.tmux_code(),
        }
    }
}

impl MouseKey {
    fn tmux_code(&self) -> u64 {
        let slot = |b: u8| u64::from(b.saturating_sub(1)).min(BUTTON_SLOTS - 1);
        let event = match self.action {
            MouseAction::Down(b) => 1 + slot(b),
            MouseAction::Up(b) => 1 + BUTTON_SLOTS + slot(b),
            MouseAction::Drag(b) => 1 + 2 * BUTTON_SLOTS + slot(b),
            MouseAction::DragEnd(b) => 1 + 3 * BUTTON_SLOTS + slot(b),
            MouseAction::WheelUp => 1 + 4 * BUTTON_SLOTS,
            MouseAction::WheelDown => 2 + 4 * BUTTON_SLOTS,
            MouseAction::SecondClick(b) => 3 + 4 * BUTTON_SLOTS + slot(b),
            MouseAction::DoubleClick(b) => 3 + 5 * BUTTON_SLOTS + slot(b),
            MouseAction::TripleClick(b) => 3 + 6 * BUTTON_SLOTS + slot(b),
        };
        let place = match self.location {
            MouseLocation::Pane => 0,
            MouseLocation::Status => 1,
            MouseLocation::StatusLeft => 2,
            MouseLocation::StatusRight => 3,
            MouseLocation::StatusDefault => 4,
            MouseLocation::ScrollbarUp => 5,
            MouseLocation::ScrollbarSlider => 6,
            MouseLocation::ScrollbarDown => 7,
            MouseLocation::Border => 8,
        };
        (KEYC_BASE + FIRST_MOUSE + event * MOUSE_PLACES + place) | modifier_bits(self.mods)
    }
}

impl Key {
    fn tmux_code(&self) -> u64 {
        use crossterm::event::KeyCode;
        let special = |n: u64| KEYC_BASE + BSPACE + n;
        let code = match self.code {
            KeyCode::Char(c) => u64::from(c),
            KeyCode::Enter => 0x0d,
            KeyCode::Esc => 0x1b,
            KeyCode::Tab => 0x09,
            KeyCode::Backspace => special(0),
            KeyCode::F(n @ 1..=12) => special(u64::from(n)),
            KeyCode::Insert => special(13),
            KeyCode::Delete => special(14),
            KeyCode::Home => special(15),
            KeyCode::End => special(16),
            KeyCode::PageDown => special(17),
            KeyCode::PageUp => special(18),
            KeyCode::BackTab => special(19),
            KeyCode::Up => special(20),
            KeyCode::Down => special(21),
            KeyCode::Left => special(22),
            KeyCode::Right => special(23),
            KeyCode::F(n) => KEYC_META - 0x1000 + u64::from(n),
            _ => KEYC_META - 1,
        };
        code | modifier_bits(self.mods)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_codes_order_keys_as_tmux_lists_them() {
        // tmux 3.6's own copy-mode-vi table as its list-keys printed it
        // (WSL, 2026-10-20), a key from each kind in its order.
        let listed = [
            "Enter",
            "Escape",
            "Space",
            "#",
            "$",
            "%",
            "*",
            ",",
            "/",
            "0",
            ":",
            ";",
            "?",
            "A",
            "^",
            "b",
            "{",
            "}",
            "MouseDown1Pane",
            "MouseDrag1Pane",
            "MouseDragEnd1Pane",
            "WheelUpPane",
            "WheelDownPane",
            "DoubleClick1Pane",
            "TripleClick1Pane",
            "BSpace",
            "Home",
            "End",
            "NPage",
            "PPage",
            "Up",
            "Down",
            "Left",
            "Right",
            "M-x",
            "C-b",
            "C-Up",
            "C-Down",
        ];
        let codes: Vec<u64> = listed
            .iter()
            .map(|k| k.parse::<BindKey>().unwrap().tmux_code())
            .collect();
        for (pair, keys) in codes.windows(2).zip(listed.windows(2)) {
            assert!(pair[0] < pair[1], "{} before {}", keys[0], keys[1]);
        }
        // Its prefix table: F1 between a character and the arrows, M- keys
        // before C- keys, C-M- after both.
        let order = ["a", "F1", "Up", "M-x", "C-b", "C-M-b"];
        let codes: Vec<u64> = order
            .iter()
            .map(|k| k.parse::<BindKey>().unwrap().tmux_code())
            .collect();
        assert!(codes.windows(2).all(|p| p[0] < p[1]), "{codes:?}");
    }

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
