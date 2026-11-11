//! Keys as the kitty keyboard protocol has them, for a program that pushed
//! flags (<https://sw.kovidgoyal.net/kitty/keyboard-protocol/>):
//!
//! ```text
//! CSI key:shifted:base ; modifiers:event ; text u
//! ```
//!
//! Where the spec's prose is silent, or kitty's own encoder
//! (`kitty/key_encoding.c`, which the programs that push flags are written
//! against) departs from it, this follows kitty's: a key that types text is
//! sent as that text unless flag 8 is on, yet its release is still reported
//! under flag 2 (the spec's prose says it has none); Enter, Tab and
//! Backspace keep their legacy bytes and have no release without flag 8;
//! the lock modifiers are reported on every key that is not sent as text.
//! One case departs from kitty: under flag 2 without flags 1 and 8, kitty
//! sends Escape's release as a second Escape, and this sends nothing.
//!
//! A key carries what the client's terminal reported of it and no more: a
//! legacy terminal reports no releases, no base-layout key and no Super or
//! Hyper, so none is sent. Its shifted key (an upper-case letter it typed)
//! and its text (the character) are known, and are sent.

use ttyr::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use super::{
    KITTY_DISAMBIGUATE, KITTY_REPORT_ALL_KEYS, KITTY_REPORT_ALTERNATE, KITTY_REPORT_EVENTS,
    KITTY_REPORT_TEXT, is_modifier_key, legacy_control,
};

/// The modifier bits of the lock keys, which a key that types text under
/// flag 1 does not report.
const LOCKS: u32 = 64 | 128;

/// The modifier bit for Shift.
const SHIFT: u32 = 1;

/// The flags a program pushed, as the encoder reads them.
#[derive(Clone, Copy)]
struct Flags {
    disambiguate: bool,
    events: bool,
    alternates: bool,
    all_keys: bool,
    /// Flag 16, which the spec defines only with flag 8.
    text: bool,
}

impl Flags {
    fn new(flags: u8) -> Self {
        let all_keys = flags & KITTY_REPORT_ALL_KEYS != 0;
        Self {
            disambiguate: flags & KITTY_DISAMBIGUATE != 0,
            events: flags & KITTY_REPORT_EVENTS != 0,
            alternates: flags & KITTY_REPORT_ALTERNATE != 0,
            all_keys,
            text: all_keys && flags & KITTY_REPORT_TEXT != 0,
        }
    }
}

/// `key` for a program under kitty keyboard `flags`, which hold flag 1, 2
/// or 8.
pub(super) fn encode(key: &KeyEvent, flags: u8) -> Vec<u8> {
    let f = Flags::new(flags);
    if is_modifier_key(key.key) && !f.all_keys {
        return Vec::new();
    }
    // Without flag 2 a repeat is a press, and there is no release.
    let kind = match key.kind {
        KeyEventKind::Release if !f.events => return Vec::new(),
        kind if f.events => kind,
        _ => KeyEventKind::Press,
    };
    let text = text(key);
    if key.key == KeyCode::Null && text.is_none() {
        return Vec::new();
    }
    if !f.all_keys
        && kind != KeyEventKind::Release
        && let Some(text) = &text
    {
        return text.as_bytes().to_vec();
    }
    let mut mods = modifier_bits(key);
    if key.key == KeyCode::BackTab {
        mods |= SHIFT;
    }
    let event = (kind != KeyEventKind::Press).then_some(kind);
    let text = text.filter(|_| f.text);
    if let Some((number, trailer)) = functional(key, f.disambiguate || f.all_keys) {
        // Enter, Tab and Backspace keep their bytes, so a shell a program
        // left in this mode still takes `reset`; Escape keeps its own under
        // flag 2 alone. None of them is released in its legacy form.
        let legacy: Option<&[u8]> = match (number, trailer) {
            (13, 'u') if mods & !LOCKS == 0 && !f.all_keys => Some(b"\r"),
            (9, 'u') if mods & !LOCKS == 0 && !f.all_keys => Some(b"\t"),
            (127, 'u') if mods & !LOCKS == 0 && !f.all_keys => Some(b"\x7f"),
            (27, 'u') if mods == 0 && !f.disambiguate && !f.all_keys => Some(b"\x1b"),
            _ => None,
        };
        if let Some(bytes) = legacy {
            return match kind {
                KeyEventKind::Release => Vec::new(),
                _ => bytes.to_vec(),
            };
        }
        return serialize(number, None, mods, event, text.as_deref(), trailer);
    }
    // A key code is the unshifted key's: tmux's C-A (`send-keys C-A`, the
    // upper-case letter with Ctrl alone) is Ctrl with `a`.
    let code = match key.key {
        KeyCode::Char(c) => u32::from(
            legacy_control(key)
                .or_else(|| tmxr_command::keys::single_lower(c))
                .unwrap_or(c),
        ),
        KeyCode::Null => 0,
        _ => return Vec::new(),
    };
    let shifted = key
        .shifted
        .map(u32::from)
        .filter(|&s| mods & SHIFT != 0 && s != code);
    let base = match key.base_layout {
        Some(KeyCode::Char(c)) => Some(u32::from(c)).filter(|&b| b != code),
        _ => None,
    };
    let alternates =
        (f.alternates && (shifted.is_some() || base.is_some())).then_some((shifted, base));
    if event.is_none() && alternates.is_none() && text.is_none() {
        if mods == 0 && !f.all_keys {
            return char::from_u32(code).map_or_else(Vec::new, |c| c.to_string().into_bytes());
        }
        if !f.disambiguate
            && !f.all_keys
            && let Some(bytes) = legacy_chord(code, key.shifted, base, mods)
        {
            return bytes;
        }
    }
    serialize(code, alternates, mods, event, text.as_deref(), 'u')
}

/// The modifier bit for Alt.
const ALT: u32 = 2;

/// The modifier bit for Ctrl.
const CTRL: u32 = 4;

/// The modifier bits for Alt and Ctrl together.
const ALT_CTRL: u32 = ALT | CTRL;

/// A chord's legacy form, which flag 2 alone keeps where kitty does: on an
/// ASCII key, or a key off ASCII whose base-layout key (`base`) is one, held
/// with Ctrl, Alt or both. `None` where it has none and goes as an escape
/// code.
fn legacy_chord(code: u32, shifted: Option<char>, base: Option<u32>, mods: u32) -> Option<Vec<u8>> {
    if (is_legacy_ascii(code) || shifted.is_some_and(|s| is_legacy_ascii(u32::from(s))))
        && let Some(bytes) = legacy_ascii(code, shifted, mods)
    {
        return Some(bytes);
    }
    let base = base.filter(|&b| is_legacy_ascii(b) && !is_legacy_ascii(code))?;
    matches!(mods, ALT | CTRL | ALT_CTRL).then(|| legacy_ascii(base, None, mods))?
}

/// An ASCII key's chord as kitty's encode_printable_ascii_key_legacy has
/// it: Shift takes the shifted key where it is known (but for a letter
/// with Ctrl), then Alt is an `ESC` before it and Ctrl maps it through the
/// spec's "Legacy ctrl mapping"; Ctrl+Shift and Alt+Shift are kept only on
/// Space. `None` for any other chord.
fn legacy_ascii(code: u32, shifted: Option<char>, mods: u32) -> Option<Vec<u8>> {
    let letter = |c: u32| (u32::from(b'a')..=u32::from(b'z')).contains(&c);
    let mut key = code;
    let mut m = mods;
    if m & SHIFT != 0
        && let Some(s) = shifted.map(u32::from)
        && s != key
        && (m & CTRL == 0 || !letter(key))
    {
        key = s;
        m &= !SHIFT;
    }
    let key = u8::try_from(key).ok().filter(u8::is_ascii)?;
    Some(match m {
        _ if mods == SHIFT => vec![key],
        ALT => vec![0x1b, key],
        CTRL => vec![ctrl_mapped(key)],
        ALT_CTRL => vec![0x1b, ctrl_mapped(key)],
        m if key == b' ' && m == CTRL | SHIFT => vec![ctrl_mapped(key)],
        m if key == b' ' && m == ALT | SHIFT => vec![0x1b, key],
        _ => return None,
    })
}

/// Whether kitty gives key `code` a legacy form (its is_legacy_ascii_key):
/// printable ASCII, Space among it, but for the upper-case letters.
fn is_legacy_ascii(code: u32) -> bool {
    (0x20..0x7f).contains(&code) && !(u32::from(b'A')..=u32::from(b'Z')).contains(&code)
}

/// The byte Ctrl makes of an ASCII key, from the spec's "Legacy ctrl
/// mapping of ASCII keys" table; a key it does not list is left as it is.
fn ctrl_mapped(key: u8) -> u8 {
    match key {
        b' ' | b'2' | b'@' => 0,
        b'3' | b'[' => 27,
        b'4' | b'\\' => 28,
        b'5' | b']' => 29,
        b'6' | b'^' | b'~' => 30,
        b'/' | b'7' | b'_' => 31,
        b'8' | b'?' => 127,
        b'a'..=b'z' => key - b'a' + 1,
        _ => key,
    }
}

/// The text `key` types, without control characters: what the terminal
/// reported, or else the character, of a key held with no modifier but
/// Shift. `None` for a key that types none, and for a chord, which keeps
/// its modifiers whatever text came with it.
fn text(key: &KeyEvent) -> Option<String> {
    let chord = key.modifiers.intersects(
        KeyModifiers::CONTROL
            | KeyModifiers::ALT
            | KeyModifiers::SUPER
            | KeyModifiers::HYPER
            | KeyModifiers::META,
    );
    if chord {
        return None;
    }
    if let Some(text) = &key.text {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        return (!text.is_empty()).then_some(text);
    }
    if !matches!(key.key, KeyCode::Char(_)) {
        return None;
    }
    key.logical_char()
        .filter(|c| !c.is_control())
        .map(String::from)
}

/// The protocol's modifier bits for `key`: Shift 1, Alt 2, Ctrl 4, Super 8,
/// Hyper 16, Meta 32, Caps Lock 64, Num Lock 128.
fn modifier_bits(key: &KeyEvent) -> u32 {
    let mut bits = 0;
    for (bit, modifier) in [
        (1, KeyModifiers::SHIFT),
        (2, KeyModifiers::ALT),
        (4, KeyModifiers::CONTROL),
        (8, KeyModifiers::SUPER),
        (16, KeyModifiers::HYPER),
        (32, KeyModifiers::META),
    ] {
        if key.modifiers.contains(modifier) {
            bits |= bit;
        }
    }
    for (bit, lock) in [
        (64, KeyEventState::CAPS_LOCK),
        (128, KeyEventState::NUM_LOCK),
    ] {
        if key.state.contains(lock) {
            bits |= bit;
        }
    }
    bits
}

/// The number and final byte the spec's "Functional key definitions" table
/// gives a key that is not a character, or a keypad key where `keypad`
/// says the keypad is told apart (flags 1 and 8; without them it is the
/// key it stands for). `None` for a character.
fn functional(key: &KeyEvent, keypad: bool) -> Option<(u32, char)> {
    if keypad && key.state.contains(KeyEventState::KEYPAD) {
        let number = match key.key {
            KeyCode::Char(c @ '0'..='9') => 57399 + c.to_digit(10)?,
            KeyCode::Char('.') => 57409,
            KeyCode::Char('/') => 57410,
            KeyCode::Char('*') => 57411,
            KeyCode::Char('-') => 57412,
            KeyCode::Char('+') => 57413,
            KeyCode::Enter => 57414,
            KeyCode::Char('=') => 57415,
            KeyCode::Char(',') => 57416,
            KeyCode::Left => 57417,
            KeyCode::Right => 57418,
            KeyCode::Up => 57419,
            KeyCode::Down => 57420,
            KeyCode::PageUp => 57421,
            KeyCode::PageDown => 57422,
            KeyCode::Home => 57423,
            KeyCode::End => 57424,
            KeyCode::Insert => 57425,
            KeyCode::Delete => 57426,
            _ => 0,
        };
        if number != 0 {
            return Some((number, 'u'));
        }
    }
    Some(match key.key {
        KeyCode::Esc => (27, 'u'),
        KeyCode::Enter => (13, 'u'),
        KeyCode::Tab | KeyCode::BackTab => (9, 'u'),
        KeyCode::Backspace => (127, 'u'),
        KeyCode::Insert => (2, '~'),
        KeyCode::Delete => (3, '~'),
        KeyCode::Left => (1, 'D'),
        KeyCode::Right => (1, 'C'),
        KeyCode::Up => (1, 'A'),
        KeyCode::Down => (1, 'B'),
        KeyCode::PageUp => (5, '~'),
        KeyCode::PageDown => (6, '~'),
        KeyCode::Home => (1, 'H'),
        KeyCode::End => (1, 'F'),
        KeyCode::KeypadBegin => (1, 'E'),
        KeyCode::F(1) => (1, 'P'),
        KeyCode::F(2) => (1, 'Q'),
        KeyCode::F(3) => (13, '~'),
        KeyCode::F(4) => (1, 'S'),
        KeyCode::F(n @ 5..=12) => ([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)], '~'),
        KeyCode::F(n @ 13..=35) => (57376 + u32::from(n - 13), 'u'),
        KeyCode::CapsLock => (57358, 'u'),
        KeyCode::ScrollLock => (57359, 'u'),
        KeyCode::NumLock => (57360, 'u'),
        KeyCode::PrintScreen => (57361, 'u'),
        KeyCode::Pause => (57362, 'u'),
        KeyCode::Menu => (57363, 'u'),
        KeyCode::Media(m) => (57428 + media_index(m)?, 'u'),
        KeyCode::Modifier(m) => (57441 + modifier_index(m)?, 'u'),
        _ => return None,
    })
}

/// A media key's place in the table, from 57428; `None` for one the
/// table does not have.
fn media_index(key: ttyr::event::MediaKeyCode) -> Option<u32> {
    use ttyr::event::MediaKeyCode as M;
    Some(match key {
        M::Play => 0,
        M::Pause => 1,
        M::PlayPause => 2,
        M::Reverse => 3,
        M::Stop => 4,
        M::FastForward => 5,
        M::Rewind => 6,
        M::TrackNext => 7,
        M::TrackPrevious => 8,
        M::Record => 9,
        M::LowerVolume => 10,
        M::RaiseVolume => 11,
        M::MuteVolume => 12,
        _ => return None,
    })
}

/// A modifier key's place in the table, from 57441; `None` for one the
/// table does not have.
fn modifier_index(key: ttyr::event::ModifierKeyCode) -> Option<u32> {
    use ttyr::event::ModifierKeyCode as M;
    Some(match key {
        M::LeftShift => 0,
        M::LeftControl => 1,
        M::LeftAlt => 2,
        M::LeftSuper => 3,
        M::LeftHyper => 4,
        M::LeftMeta => 5,
        M::RightShift => 6,
        M::RightControl => 7,
        M::RightAlt => 8,
        M::RightSuper => 9,
        M::RightHyper => 10,
        M::RightMeta => 11,
        M::IsoLevel3Shift => 12,
        M::IsoLevel5Shift => 13,
        _ => return None,
    })
}

/// `CSI number:shifted:base ; modifiers:event ; text <trailer>`, each part
/// left out where it says nothing, as the spec has it: the number where it
/// is a letter key's 1 and nothing follows, the modifiers where there are
/// none and no event type, the event type for a press.
fn serialize(
    number: u32,
    alternates: Option<(Option<u32>, Option<u32>)>,
    mods: u32,
    event: Option<KeyEventKind>,
    text: Option<&str>,
    trailer: char,
) -> Vec<u8> {
    let mut out = String::from("\x1b[");
    let second = mods != 0 || event.is_some();
    if number != 1 || alternates.is_some() || second || text.is_some() {
        out += &number.to_string();
    }
    if let Some((shifted, base)) = alternates {
        out.push(':');
        if let Some(shifted) = shifted {
            out += &shifted.to_string();
        }
        if let Some(base) = base {
            out.push(':');
            out += &base.to_string();
        }
    }
    if second || text.is_some() {
        out.push(';');
        if second {
            out += &(mods + 1).to_string();
        }
        match event {
            Some(KeyEventKind::Repeat) => out.push_str(":2"),
            Some(KeyEventKind::Release) => out.push_str(":3"),
            _ => {}
        }
    }
    if let Some(text) = text {
        for (i, c) in text.chars().enumerate() {
            out.push(if i == 0 { ';' } else { ':' });
            out += &u32::from(c).to_string();
        }
    }
    out.push(trailer);
    out.into_bytes()
}

#[cfg(test)]
mod tests;
