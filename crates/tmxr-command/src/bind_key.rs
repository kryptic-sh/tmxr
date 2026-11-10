//! What a key table binds: a keyboard [`Key`] or one of tmux's mouse keys
//! (`MouseDown1Pane`, `WheelUpPane`, `MouseDragEnd1Pane`, …).

use std::fmt;
use std::str::FromStr;

use ttyr::event::KeyModifiers;

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
    /// Motion with no button held. tmux has no name for it, so nothing
    /// binds it: it always goes to the pane's program.
    Move,
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

/// Buttons tmux names and a terminal can report: 1 left, 2 middle, 3
/// right, and 6 and 7, the wheel left and right. tmux also names 8 to 11,
/// buttons crossterm cannot read.
const BUTTONS: [u8; 5] = [1, 2, 3, 6, 7];

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
            MouseAction::Move => f.write_str("MouseMove")?,
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
/// tmxr's own modifiers (Super, Hyper, Meta), above every bit of a tmux
/// key code, so a key with one lists after every key tmux has.
const SUPER_BIT: u64 = 1 << 56;
const HYPER_BIT: u64 = 1 << 57;
const META_BIT: u64 = 1 << 58;
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
    for (m, bit) in [
        (KeyModifiers::SUPER, SUPER_BIT),
        (KeyModifiers::HYPER, HYPER_BIT),
        (KeyModifiers::META, META_BIT),
    ] {
        if mods.contains(m) {
            bits |= bit;
        }
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
        // Buttons 1 to 3, then 6 on: tmux has no 4 or 5.
        let slot = |b: u8| u64::from(if b > 3 { b - 3 } else { b.saturating_sub(1) });
        let event = match self.action {
            MouseAction::Move => 0,
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
        use ttyr::event::KeyCode;
        let special = |n: u64| KEYC_BASE + BSPACE + n;
        // Keys tmux has no code for come after every key it has: F13 and
        // up, then tmxr's keypad keys, then its other keys, each in the
        // order kitty's spec lists them.
        let beyond = |n: u64| KEYC_META - 0x1000 + n;
        if self.keypad {
            let code = match self.code {
                KeyCode::Char(c) => match "/*-789+456123".find(c) {
                    Some(i) => special(KP_SLASH + i as u64),
                    None => match c {
                        '0' => special(KP_SLASH + 14),
                        '.' => special(KP_SLASH + 15),
                        '=' => beyond(0x100),
                        _ => beyond(0x101),
                    },
                },
                KeyCode::Enter => special(KP_SLASH + 13),
                code => beyond(0x110 + kitty_order(code)),
            };
            return code | modifier_bits(self.mods);
        }
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
            KeyCode::F(n) => beyond(u64::from(n)),
            code => beyond(0x200 + kitty_order(code)),
        };
        code | modifier_bits(self.mods)
    }
}

/// `KEYC_KP_SLASH`, the first keypad key, after the arrows: `KP/ KP* KP-
/// KP7 KP8 KP9 KP+ KP4 KP5 KP6 KP1 KP2 KP3 KPEnter KP0 KP.` in that order.
const KP_SLASH: u64 = 24;

/// Where kitty's functional key table lists a key tmux has no code for, to
/// order those keys among themselves.
fn kitty_order(code: ttyr::event::KeyCode) -> u64 {
    use ttyr::event::KeyCode;
    match code {
        KeyCode::Left => 0,
        KeyCode::Right => 1,
        KeyCode::Up => 2,
        KeyCode::Down => 3,
        KeyCode::PageUp => 4,
        KeyCode::PageDown => 5,
        KeyCode::Home => 6,
        KeyCode::End => 7,
        KeyCode::Insert => 8,
        KeyCode::Delete => 9,
        KeyCode::CapsLock => 10,
        KeyCode::ScrollLock => 11,
        KeyCode::NumLock => 12,
        KeyCode::PrintScreen => 13,
        KeyCode::Pause => 14,
        KeyCode::Menu => 15,
        KeyCode::KeypadBegin => 16,
        KeyCode::Media(m) => 0x20 + m as u64,
        KeyCode::Modifier(m) => 0x40 + m as u64,
        _ => 0x7f,
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
        // The keypad after the arrows, in tmux.h's order; tmxr's own keys
        // after every key tmux has but those with modifiers, and its own
        // modifiers after tmux's.
        let order = [
            "Right",
            "KP/",
            "KP*",
            "KP-",
            "KP7",
            "KP8",
            "KP9",
            "KP+",
            "KP4",
            "KP5",
            "KP6",
            "KP1",
            "KP2",
            "KP3",
            "KPEnter",
            "KP0",
            "KP.",
            "F13",
            "F35",
            "KP=",
            "KP,",
            "KPLeft",
            "KPDC",
            "CapsLock",
            "Menu",
            "KPBegin",
            "MediaPlay",
            "MuteVolume",
            "LeftShift",
            "IsoLevel5Shift",
            "M-a",
            "C-a",
            "C-S-a",
            "Super-a",
            "Hyper-a",
            "Meta-a",
        ];
        let codes: Vec<u64> = order
            .iter()
            .map(|k| k.parse::<BindKey>().unwrap().tmux_code())
            .collect();
        for (pair, keys) in codes.windows(2).zip(order.windows(2)) {
            assert!(pair[0] < pair[1], "{} before {}", keys[0], keys[1]);
        }
        // tmux.h has no buttons 4 and 5: MOUSEDOWN6 follows MOUSEDOWN3.
        let code = |k: &str| k.parse::<BindKey>().unwrap().tmux_code();
        assert_eq!(
            code("MouseDown6Pane") - code("MouseDown1Pane"),
            3 * MOUSE_PLACES
        );
        assert_eq!(
            code("MouseDown7Pane") - code("MouseDown1Pane"),
            4 * MOUSE_PLACES
        );
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
            "MouseDown6Pane",
            "SecondClick7Pane",
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
            "MouseDown5Pane",
            "MouseDown0Pane",
            "MouseDownPane",
            "MouseDown1",
            "MouseDown1Window",
            // tmux 3.8 refuses it too: motion is not a key a table binds.
            "MouseMovePane",
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
