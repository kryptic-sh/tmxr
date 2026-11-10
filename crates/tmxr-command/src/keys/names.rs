//! Key names, in each spelling tmxr reads:
//!
//! - tmux's (`C-b`, `M-h`, `C-\`, `BSpace`, `PPage`, `KPEnter`), extended
//!   to the keys and modifiers the kitty protocol reports: `Super-`,
//!   `Hyper-` and `Meta-` prefixes, F13 up to [`MAX_F`], the keypad keys tmux has no
//!   name for (`KPLeft`, `KP=`), and the kitty spec's other keys in
//!   CamelCase (`CapsLock`, `MediaPlay`, `LeftShift`);
//! - kitty's, as its docs write them: `+`-joined modifiers and its key
//!   names (`ctrl+shift+h`, `super+k`, `kp_enter`, `f13`);
//! - hjkl's `<…>` form, which follows neovim (`<C-S-h>`, `<D-a>` for
//!   Super, `<T-a>` for Meta, `<kEnter>`, `<F13>`).
//!
//! A key prints in tmux's name where tmux has one, else in the extended
//! tmux name: one spelling per key, whichever it was bound with.
//! `send-keys` takes only the names tmux itself knows as keys
//! ([`Key::from_tmux_name`]) and types any other word, as tmux does.

use std::fmt;
use std::str::FromStr;

use ttyr::event::{KeyCode, KeyModifiers, MediaKeyCode, ModifierKeyCode};

use super::{Key, UnknownKey, write_mods};

/// The highest function key the kitty protocol reports.
const MAX_F: u8 = 35;

/// The highest function key tmux 3.6 names (its key_string_table).
const TMUX_MAX_F: u8 = 12;

/// The keypad's characters tmux 3.6 names (`KP/` … `KP.`); its only other
/// keypad key is `KPEnter`.
const TMUX_KEYPAD: &str = "/*-789+456123.0";

/// Keys tmux 3.6 names by a word (its key_string_table), matched in any
/// case as its key_string_search_table. A key prints as its first name here
/// or in [`EXTENDED_WORDS`].
const TMUX_WORDS: &[(&str, KeyCode)] = &[
    ("Enter", KeyCode::Enter),
    ("Escape", KeyCode::Esc),
    ("Space", KeyCode::Char(' ')),
    ("Tab", KeyCode::Tab),
    ("BTab", KeyCode::BackTab),
    ("BSpace", KeyCode::Backspace),
    ("Up", KeyCode::Up),
    ("Down", KeyCode::Down),
    ("Left", KeyCode::Left),
    ("Right", KeyCode::Right),
    ("Home", KeyCode::Home),
    ("End", KeyCode::End),
    ("PPage", KeyCode::PageUp),
    ("PageUp", KeyCode::PageUp),
    ("PgUp", KeyCode::PageUp),
    ("NPage", KeyCode::PageDown),
    ("PageDown", KeyCode::PageDown),
    ("PgDn", KeyCode::PageDown),
    ("IC", KeyCode::Insert),
    ("Insert", KeyCode::Insert),
    ("DC", KeyCode::Delete),
    ("Delete", KeyCode::Delete),
];

/// tmxr's words for keys tmux has no name for: the kitty spec's names in
/// CamelCase.
const EXTENDED_WORDS: &[(&str, KeyCode)] = &[
    ("CapsLock", KeyCode::CapsLock),
    ("ScrollLock", KeyCode::ScrollLock),
    ("NumLock", KeyCode::NumLock),
    ("PrintScreen", KeyCode::PrintScreen),
    ("Pause", KeyCode::Pause),
    ("Menu", KeyCode::Menu),
    ("KPBegin", KeyCode::KeypadBegin),
    ("MediaPlay", KeyCode::Media(MediaKeyCode::Play)),
    ("MediaPause", KeyCode::Media(MediaKeyCode::Pause)),
    ("MediaPlayPause", KeyCode::Media(MediaKeyCode::PlayPause)),
    ("MediaReverse", KeyCode::Media(MediaKeyCode::Reverse)),
    ("MediaStop", KeyCode::Media(MediaKeyCode::Stop)),
    (
        "MediaFastForward",
        KeyCode::Media(MediaKeyCode::FastForward),
    ),
    ("MediaRewind", KeyCode::Media(MediaKeyCode::Rewind)),
    ("MediaTrackNext", KeyCode::Media(MediaKeyCode::TrackNext)),
    (
        "MediaTrackPrevious",
        KeyCode::Media(MediaKeyCode::TrackPrevious),
    ),
    ("MediaRecord", KeyCode::Media(MediaKeyCode::Record)),
    ("LowerVolume", KeyCode::Media(MediaKeyCode::LowerVolume)),
    ("RaiseVolume", KeyCode::Media(MediaKeyCode::RaiseVolume)),
    ("MuteVolume", KeyCode::Media(MediaKeyCode::MuteVolume)),
    ("LeftShift", KeyCode::Modifier(ModifierKeyCode::LeftShift)),
    (
        "LeftControl",
        KeyCode::Modifier(ModifierKeyCode::LeftControl),
    ),
    ("LeftAlt", KeyCode::Modifier(ModifierKeyCode::LeftAlt)),
    ("LeftSuper", KeyCode::Modifier(ModifierKeyCode::LeftSuper)),
    ("LeftHyper", KeyCode::Modifier(ModifierKeyCode::LeftHyper)),
    ("LeftMeta", KeyCode::Modifier(ModifierKeyCode::LeftMeta)),
    ("RightShift", KeyCode::Modifier(ModifierKeyCode::RightShift)),
    (
        "RightControl",
        KeyCode::Modifier(ModifierKeyCode::RightControl),
    ),
    ("RightAlt", KeyCode::Modifier(ModifierKeyCode::RightAlt)),
    ("RightSuper", KeyCode::Modifier(ModifierKeyCode::RightSuper)),
    ("RightHyper", KeyCode::Modifier(ModifierKeyCode::RightHyper)),
    ("RightMeta", KeyCode::Modifier(ModifierKeyCode::RightMeta)),
    (
        "IsoLevel3Shift",
        KeyCode::Modifier(ModifierKeyCode::IsoLevel3Shift),
    ),
    (
        "IsoLevel5Shift",
        KeyCode::Modifier(ModifierKeyCode::IsoLevel5Shift),
    ),
];

/// The prefixes tmxr adds to tmux's `C-` / `M-` / `S-`, matched in any
/// case, in the order they print after tmux's.
const EXTENDED_MODS: &[(&str, KeyModifiers)] = &[
    ("Super-", KeyModifiers::SUPER),
    ("Hyper-", KeyModifiers::HYPER),
    ("Meta-", KeyModifiers::META),
];

/// The kitty spec's functional key names (its "Functional key codes"
/// table), lower-cased as kitty's docs write them, but for `f1`… and
/// `kp_0`…, which [`kitty_numbered`] reads; with the keypad flag where the
/// key is the keypad's. kitty's `space` and `plus` (the `+` that cannot
/// follow a `+`) are names too.
const KITTY: &[(&str, KeyCode, bool)] = &[
    ("escape", KeyCode::Esc, false),
    ("enter", KeyCode::Enter, false),
    ("tab", KeyCode::Tab, false),
    ("backspace", KeyCode::Backspace, false),
    ("insert", KeyCode::Insert, false),
    ("delete", KeyCode::Delete, false),
    ("left", KeyCode::Left, false),
    ("right", KeyCode::Right, false),
    ("up", KeyCode::Up, false),
    ("down", KeyCode::Down, false),
    ("page_up", KeyCode::PageUp, false),
    ("page_down", KeyCode::PageDown, false),
    ("home", KeyCode::Home, false),
    ("end", KeyCode::End, false),
    ("caps_lock", KeyCode::CapsLock, false),
    ("scroll_lock", KeyCode::ScrollLock, false),
    ("num_lock", KeyCode::NumLock, false),
    ("print_screen", KeyCode::PrintScreen, false),
    ("pause", KeyCode::Pause, false),
    ("menu", KeyCode::Menu, false),
    ("kp_decimal", KeyCode::Char('.'), true),
    ("kp_divide", KeyCode::Char('/'), true),
    ("kp_multiply", KeyCode::Char('*'), true),
    ("kp_subtract", KeyCode::Char('-'), true),
    ("kp_add", KeyCode::Char('+'), true),
    ("kp_enter", KeyCode::Enter, true),
    ("kp_equal", KeyCode::Char('='), true),
    ("kp_separator", KeyCode::Char(','), true),
    ("kp_left", KeyCode::Left, true),
    ("kp_right", KeyCode::Right, true),
    ("kp_up", KeyCode::Up, true),
    ("kp_down", KeyCode::Down, true),
    ("kp_page_up", KeyCode::PageUp, true),
    ("kp_page_down", KeyCode::PageDown, true),
    ("kp_home", KeyCode::Home, true),
    ("kp_end", KeyCode::End, true),
    ("kp_insert", KeyCode::Insert, true),
    ("kp_delete", KeyCode::Delete, true),
    ("kp_begin", KeyCode::KeypadBegin, false),
    ("media_play", KeyCode::Media(MediaKeyCode::Play), false),
    ("media_pause", KeyCode::Media(MediaKeyCode::Pause), false),
    (
        "media_play_pause",
        KeyCode::Media(MediaKeyCode::PlayPause),
        false,
    ),
    (
        "media_reverse",
        KeyCode::Media(MediaKeyCode::Reverse),
        false,
    ),
    ("media_rewind", KeyCode::Media(MediaKeyCode::Rewind), false),
    ("media_stop", KeyCode::Media(MediaKeyCode::Stop), false),
    (
        "media_fast_forward",
        KeyCode::Media(MediaKeyCode::FastForward),
        false,
    ),
    (
        "media_track_next",
        KeyCode::Media(MediaKeyCode::TrackNext),
        false,
    ),
    (
        "media_track_previous",
        KeyCode::Media(MediaKeyCode::TrackPrevious),
        false,
    ),
    ("media_record", KeyCode::Media(MediaKeyCode::Record), false),
    (
        "lower_volume",
        KeyCode::Media(MediaKeyCode::LowerVolume),
        false,
    ),
    (
        "raise_volume",
        KeyCode::Media(MediaKeyCode::RaiseVolume),
        false,
    ),
    (
        "mute_volume",
        KeyCode::Media(MediaKeyCode::MuteVolume),
        false,
    ),
    (
        "left_shift",
        KeyCode::Modifier(ModifierKeyCode::LeftShift),
        false,
    ),
    (
        "left_control",
        KeyCode::Modifier(ModifierKeyCode::LeftControl),
        false,
    ),
    (
        "left_alt",
        KeyCode::Modifier(ModifierKeyCode::LeftAlt),
        false,
    ),
    (
        "left_super",
        KeyCode::Modifier(ModifierKeyCode::LeftSuper),
        false,
    ),
    (
        "left_hyper",
        KeyCode::Modifier(ModifierKeyCode::LeftHyper),
        false,
    ),
    (
        "left_meta",
        KeyCode::Modifier(ModifierKeyCode::LeftMeta),
        false,
    ),
    (
        "right_shift",
        KeyCode::Modifier(ModifierKeyCode::RightShift),
        false,
    ),
    (
        "right_control",
        KeyCode::Modifier(ModifierKeyCode::RightControl),
        false,
    ),
    (
        "right_alt",
        KeyCode::Modifier(ModifierKeyCode::RightAlt),
        false,
    ),
    (
        "right_super",
        KeyCode::Modifier(ModifierKeyCode::RightSuper),
        false,
    ),
    (
        "right_hyper",
        KeyCode::Modifier(ModifierKeyCode::RightHyper),
        false,
    ),
    (
        "right_meta",
        KeyCode::Modifier(ModifierKeyCode::RightMeta),
        false,
    ),
    (
        "iso_level3_shift",
        KeyCode::Modifier(ModifierKeyCode::IsoLevel3Shift),
        false,
    ),
    (
        "iso_level5_shift",
        KeyCode::Modifier(ModifierKeyCode::IsoLevel5Shift),
        false,
    ),
    ("space", KeyCode::Char(' '), false),
    ("plus", KeyCode::Char('+'), false),
];

/// kitty's modifier names, as its docs list them.
const KITTY_MODS: &[(&str, KeyModifiers)] = &[
    ("ctrl", KeyModifiers::CONTROL),
    ("shift", KeyModifiers::SHIFT),
    ("alt", KeyModifiers::ALT),
    ("opt", KeyModifiers::ALT),
    ("option", KeyModifiers::ALT),
    ("super", KeyModifiers::SUPER),
    ("cmd", KeyModifiers::SUPER),
    ("command", KeyModifiers::SUPER),
    ("hyper", KeyModifiers::HYPER),
    ("meta", KeyModifiers::META),
];

/// neovim's key names (`:help key-notation`) for the `<…>` form, matched
/// in any case, with the keypad flag where the key is the keypad's; `F1`…
/// and `k0`… are read by [`bracketed_numbered`].
const NEOVIM: &[(&str, KeyCode, bool)] = &[
    ("space", KeyCode::Char(' '), false),
    ("lt", KeyCode::Char('<'), false),
    ("gt", KeyCode::Char('>'), false),
    ("bar", KeyCode::Char('|'), false),
    ("bslash", KeyCode::Char('\\'), false),
    ("cr", KeyCode::Enter, false),
    ("enter", KeyCode::Enter, false),
    ("return", KeyCode::Enter, false),
    ("esc", KeyCode::Esc, false),
    ("escape", KeyCode::Esc, false),
    ("tab", KeyCode::Tab, false),
    ("bs", KeyCode::Backspace, false),
    ("backspace", KeyCode::Backspace, false),
    ("del", KeyCode::Delete, false),
    ("delete", KeyCode::Delete, false),
    ("ins", KeyCode::Insert, false),
    ("insert", KeyCode::Insert, false),
    ("up", KeyCode::Up, false),
    ("down", KeyCode::Down, false),
    ("left", KeyCode::Left, false),
    ("right", KeyCode::Right, false),
    ("home", KeyCode::Home, false),
    ("end", KeyCode::End, false),
    ("pageup", KeyCode::PageUp, false),
    ("pagedown", KeyCode::PageDown, false),
    ("kplus", KeyCode::Char('+'), true),
    ("kminus", KeyCode::Char('-'), true),
    ("kmultiply", KeyCode::Char('*'), true),
    ("kdivide", KeyCode::Char('/'), true),
    ("kpoint", KeyCode::Char('.'), true),
    ("kcomma", KeyCode::Char(','), true),
    ("kequal", KeyCode::Char('='), true),
    ("kenter", KeyCode::Enter, true),
    ("kup", KeyCode::Up, true),
    ("kdown", KeyCode::Down, true),
    ("kleft", KeyCode::Left, true),
    ("kright", KeyCode::Right, true),
    ("khome", KeyCode::Home, true),
    ("kend", KeyCode::End, true),
    ("kpageup", KeyCode::PageUp, true),
    ("kpagedown", KeyCode::PageDown, true),
    ("kinsert", KeyCode::Insert, true),
    ("kdel", KeyCode::Delete, true),
    ("korigin", KeyCode::KeypadBegin, false),
];

impl FromStr for Key {
    type Err = UnknownKey;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let key = match s.strip_prefix('<').and_then(|r| r.strip_suffix('>')) {
            Some(inner) if !inner.is_empty() => parse_bracketed(inner),
            _ => parse_kitty(s).or_else(|| parse_tmux(s, false)),
        };
        key.ok_or_else(|| UnknownKey(s.to_owned()))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_mods(f, self.mods)?;
        for (prefix, m) in EXTENDED_MODS {
            if self.mods.contains(*m) {
                f.write_str(prefix)?;
            }
        }
        if self.keypad {
            f.write_str("KP")?;
        }
        match self.code {
            KeyCode::Char(' ') => f.write_str("Space"),
            KeyCode::Char(c) => write!(f, "{c}"),
            KeyCode::F(n) => write!(f, "F{n}"),
            code => {
                let name = words()
                    .find(|(_, c)| *c == code)
                    .map_or("Unknown", |(name, _)| name);
                f.write_str(name)
            }
        }
    }
}

impl Key {
    /// The key `s` names where tmux 3.6 knows the name itself: `^`, its
    /// `C-` / `M-` / `S-` prefixes, then one character, F1 to F12, a word
    /// of [`TMUX_WORDS`] or one of its `KP` keys. `send-keys` types any
    /// other word as text, as tmux's does, so a kitty, hjkl or extended name
    /// (`ctrl+c`, `<C-b>`, `Pause`, `F13`) is typed.
    pub fn from_tmux_name(s: &str) -> Option<Self> {
        parse_tmux(s, true)
    }
}

/// A key in tmux's form: `^`, then `C-` / `M-` / `S-` and (unless
/// `tmux_only`) tmxr's `Super-` / `Hyper-` / `Meta-` prefixes, then a key's
/// name, of tmux's own alone where `tmux_only`.
fn parse_tmux(s: &str, tmux_only: bool) -> Option<Key> {
    // `^a` is `C-a`, its one character lower-cased, as tmux's
    // key_string_lookup_string; `^` alone is the key itself.
    let (caret, named) = match s.strip_prefix('^') {
        Some(rest) if !rest.is_empty() => (KeyModifiers::CONTROL, rest),
        _ => (KeyModifiers::NONE, s),
    };
    let mut chars = named.chars();
    if let (false, Some(c), None) = (caret.is_empty(), chars.next(), chars.next()) {
        return Some(Key::new(KeyCode::Char(c.to_ascii_lowercase()), caret));
    }
    let (mods, rest) = if tmux_only {
        super::split_mods(named)
    } else {
        split_tmux_mods(named)
    };
    let (code, keypad) = if tmux_only {
        tmux_word(rest)?
    } else {
        key_word(rest)?
    };
    Some(with_keypad(Key::new(code, mods | caret), keypad))
}

/// A key's name with no modifiers as tmux 3.6 knows it: one character, F1
/// to F12, a word of [`TMUX_WORDS`], or one of its `KP` keys.
fn tmux_word(name: &str) -> Option<(KeyCode, bool)> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some((KeyCode::Char(c), false));
    }
    if let Some(n) = strip_prefix_any_case(name, "F")
        .and_then(function_key)
        .filter(|&n| n <= TMUX_MAX_F)
    {
        return Some((KeyCode::F(n), false));
    }
    if let Some((_, code)) = TMUX_WORDS
        .iter()
        .find(|(w, _)| w.eq_ignore_ascii_case(name))
    {
        return Some((*code, false));
    }
    let main = strip_prefix_any_case(name, "KP")?;
    if main.eq_ignore_ascii_case("Enter") {
        return Some((KeyCode::Enter, true));
    }
    let mut chars = main.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if TMUX_KEYPAD.contains(c) => Some((KeyCode::Char(c), true)),
        _ => None,
    }
}

/// Strip tmux's prefixes and tmxr's extended ones. Each needs something
/// after it, so `C--` is Ctrl with `-`.
fn split_tmux_mods(s: &str) -> (KeyModifiers, &str) {
    let mut mods = KeyModifiers::NONE;
    let mut rest = s;
    loop {
        let extended = EXTENDED_MODS.iter().find_map(|(prefix, m)| {
            strip_prefix_any_case(rest, prefix)
                .filter(|tail| !tail.is_empty())
                .map(|tail| (*m, tail))
        });
        if let Some((m, tail)) = extended {
            mods |= m;
            rest = tail;
            continue;
        }
        let (m, tail) = super::split_mods(rest);
        if m.is_empty() {
            return (mods, rest);
        }
        mods |= m;
        rest = tail;
    }
}

/// A key's name with no modifiers, in any of tmux's, tmxr's or kitty's
/// spellings: one character, `F1` to [`MAX_F`], a word ([`words`]), `KP` and a
/// keypad key's main name (tmux's `KP1`, `KPEnter`, and tmxr's `KPLeft`),
/// or a kitty name. Says whether the key is the keypad's.
fn key_word(name: &str) -> Option<(KeyCode, bool)> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some((KeyCode::Char(c), false));
    }
    if let Some(n) = strip_prefix_any_case(name, "F").and_then(function_key) {
        return Some((KeyCode::F(n), false));
    }
    if let Some(code) = word(name) {
        return Some((code, false));
    }
    let keypad = strip_prefix_any_case(name, "KP").and_then(|main| {
        let mut chars = main.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Some(KeyCode::Char(c)),
            _ => word(main),
        }
    });
    match keypad {
        Some(code) if super::on_the_keypad(code) => Some((code, true)),
        _ => kitty_word(name),
    }
}

/// Every key named by a word, tmux's first.
fn words() -> impl Iterator<Item = &'static (&'static str, KeyCode)> {
    TMUX_WORDS.iter().chain(EXTENDED_WORDS)
}

/// A key [`words`] names.
fn word(name: &str) -> Option<KeyCode> {
    words()
        .find(|(w, _)| w.eq_ignore_ascii_case(name))
        .map(|(_, code)| *code)
}

/// A key kitty names: one of [`KITTY`], `f1` up to [`MAX_F`], or `kp_0` to
/// `kp_9`.
fn kitty_word(name: &str) -> Option<(KeyCode, bool)> {
    let lower = name.to_ascii_lowercase();
    if let Some((_, code, keypad)) = KITTY.iter().find(|(n, _, _)| *n == lower) {
        return Some((*code, *keypad));
    }
    kitty_numbered(&lower)
}

/// kitty's `f1` up to [`MAX_F`] and `kp_0` to `kp_9`, lower-cased.
fn kitty_numbered(lower: &str) -> Option<(KeyCode, bool)> {
    if let Some(n) = lower.strip_prefix('f').and_then(function_key) {
        return Some((KeyCode::F(n), false));
    }
    keypad_digit(lower.strip_prefix("kp_")?)
}

/// A function key's number, 1 to [`MAX_F`].
fn function_key(digits: &str) -> Option<u8> {
    digits
        .parse::<u8>()
        .ok()
        .filter(|n| (1..=MAX_F).contains(n))
}

/// The keypad's digit `digit` (one decimal digit).
fn keypad_digit(digit: &str) -> Option<(KeyCode, bool)> {
    let mut chars = digit.chars();
    match (chars.next(), chars.next()) {
        (Some(c @ '0'..='9'), None) => Some((KeyCode::Char(c), true)),
        _ => None,
    }
}

/// A key in kitty's form, `+`-joined modifiers before a key's name
/// (`ctrl+shift+h`); `None` where `s` is not one, so `C-+` and `M-+` stay
/// tmux's.
fn parse_kitty(s: &str) -> Option<Key> {
    let (modifiers, name) = s.rsplit_once('+')?;
    if modifiers.is_empty() || name.is_empty() {
        return None;
    }
    let mut mods = KeyModifiers::NONE;
    for m in modifiers.split('+') {
        mods |= KITTY_MODS
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(m))?
            .1;
    }
    let (code, keypad) = key_word(name)?;
    Some(with_keypad(shifted(code, mods), keypad))
}

/// The inside of an hjkl `<…>` key, after neovim, as hjkl's plan sets the
/// notation for its phase 6 (hjkl-keymap reads only `C-` / `S-` / `A-` /
/// `M-`, F1 to F12 and the main keys' names today): those prefixes and
/// neovim's `D-` (Super) and `T-` (Meta), in any case and order, then a
/// neovim key name or one character; tmux's and tmxr's names (`KPEnter`,
/// `MediaPlay`) name the keys neovim does not.
fn parse_bracketed(inner: &str) -> Option<Key> {
    let mut mods = KeyModifiers::NONE;
    let mut rest = inner;
    while let Some((m, tail)) = rest.split_once('-')
        && !tail.is_empty()
    {
        match m.to_ascii_uppercase().as_str() {
            "C" => mods |= KeyModifiers::CONTROL,
            "S" => mods |= KeyModifiers::SHIFT,
            "A" | "M" => mods |= KeyModifiers::ALT,
            "D" => mods |= KeyModifiers::SUPER,
            "T" => mods |= KeyModifiers::META,
            _ => break,
        }
        rest = tail;
    }
    let lower = rest.to_ascii_lowercase();
    let (code, keypad) = match NEOVIM.iter().find(|(n, _, _)| *n == lower) {
        Some((_, code, keypad)) => (*code, *keypad),
        None => bracketed_numbered(&lower).or_else(|| {
            let mut chars = rest.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some((KeyCode::Char(c), false)),
                _ => key_word(rest).filter(|_| !rest.contains('_')),
            }
        })?,
    };
    Some(with_keypad(shifted(code, mods), keypad))
}

/// The key `code` with `mods` as a terminal sends the chord kitty's and
/// hjkl's names mean: Shift with a letter and no Ctrl is the upper-case
/// letter (`alt+shift+h` and `<M-S-a>` are `M-H` and `M-A`), and Shift with
/// Tab is BTab. tmux's own `M-S-a` and `S-Tab` stay the keys tmux makes of
/// them, apart from these.
fn shifted(code: KeyCode, mods: KeyModifiers) -> Key {
    let shift = mods.contains(KeyModifiers::SHIFT);
    let code = match code {
        KeyCode::Char(c) if shift && !mods.contains(KeyModifiers::CONTROL) => {
            KeyCode::Char(super::single_upper(c).unwrap_or(c))
        }
        KeyCode::Tab if shift => KeyCode::BackTab,
        code => code,
    };
    Key::new(code, mods)
}

/// neovim's `F1` up to [`MAX_F`] and `k0` to `k9`, lower-cased.
fn bracketed_numbered(lower: &str) -> Option<(KeyCode, bool)> {
    if let Some(n) = lower.strip_prefix('f').and_then(function_key) {
        return Some((KeyCode::F(n), false));
    }
    keypad_digit(lower.strip_prefix('k')?)
}

fn with_keypad(key: Key, keypad: bool) -> Key {
    if keypad { key.on_keypad() } else { key }
}

/// `s` without `prefix`, matched in any ASCII case.
fn strip_prefix_any_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

#[cfg(test)]
mod tests;
