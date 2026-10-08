//! tmux key names (`C-b`, `M-h`, `C-\`, `BSpace`, `PPage`, …).

use std::fmt;
use std::str::FromStr;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A key as bound in a key table: a key code plus Ctrl / Alt (Meta) / Shift.
///
/// Built from terminal events with [`Key::from_event`], which normalises the
/// ways different terminals report the same chord, so a bind made for `C-\`
/// matches whether the terminal sent `0x1c` or a kitty `CSI 92;5u`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

const MODS: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SHIFT);

impl Key {
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        Self::normalise(code, mods & MODS)
    }

    pub fn from_event(ev: &KeyEvent) -> Self {
        Self::new(ev.code, ev.modifiers)
    }

    fn normalise(code: KeyCode, mut mods: KeyModifiers) -> Self {
        let code = match code {
            KeyCode::Char(c) => {
                // A character already carries its shift state ('A', '%').
                mods.remove(KeyModifiers::SHIFT);
                let c = if mods.contains(KeyModifiers::CONTROL) {
                    // Legacy terminals deliver C-\ C-] C-^ C-_ as the bytes
                    // 0x1c..0x1f, which crossterm reports as C-4 .. C-7.
                    match c {
                        '4' => '\\',
                        '5' => ']',
                        '6' => '^',
                        '7' => '_',
                        c => c.to_ascii_lowercase(),
                    }
                } else {
                    c
                };
                KeyCode::Char(c)
            }
            KeyCode::BackTab => {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::BackTab
            }
            other => other,
        };
        Self { code, mods }
    }
}

/// Error for an unrecognised key name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown key: {0}")]
pub struct UnknownKey(pub String);

const NAMED: &[(&str, KeyCode)] = &[
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

impl FromStr for Key {
    type Err = UnknownKey;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || UnknownKey(s.to_owned());
        // hjkl's notation (`<C-b>`, `<CR>`, `<M-h>`), accepted alongside
        // tmux's names.
        if let Some(inner) = s.strip_prefix('<').and_then(|r| r.strip_suffix('>'))
            && !inner.is_empty()
        {
            return parse_bracketed(inner).ok_or_else(err);
        }
        let (mods, rest) = split_mods(s);
        if let Some(n) = rest.strip_prefix('F').and_then(|n| n.parse::<u8>().ok())
            && (1..=24).contains(&n)
        {
            return Ok(Self::new(KeyCode::F(n), mods));
        }
        if let Some((_, code)) = NAMED
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(rest))
        {
            return Ok(Self::new(*code, mods));
        }
        let mut chars = rest.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Ok(Self::new(KeyCode::Char(c), mods)),
            _ => Err(err()),
        }
    }
}

/// Strip tmux's `C-` / `M-` / `S-` prefixes from a key name. A lone `-` or
/// a name like `C-` (the key `-` with Ctrl) is handled by requiring something
/// after each prefix.
pub(crate) fn split_mods(s: &str) -> (KeyModifiers, &str) {
    let mut mods = KeyModifiers::NONE;
    let mut rest = s;
    while rest.len() > 2 && rest.as_bytes()[1] == b'-' {
        match rest.as_bytes()[0] {
            b'C' | b'c' => mods |= KeyModifiers::CONTROL,
            b'M' | b'm' => mods |= KeyModifiers::ALT,
            b'S' | b's' => mods |= KeyModifiers::SHIFT,
            _ => break,
        }
        rest = &rest[2..];
    }
    (mods, rest)
}

/// Write `mods` as tmux's `C-` / `M-` / `S-` prefixes.
pub(crate) fn write_mods(f: &mut fmt::Formatter<'_>, mods: KeyModifiers) -> fmt::Result {
    if mods.contains(KeyModifiers::CONTROL) {
        f.write_str("C-")?;
    }
    if mods.contains(KeyModifiers::ALT) {
        f.write_str("M-")?;
    }
    if mods.contains(KeyModifiers::SHIFT) {
        f.write_str("S-")?;
    }
    Ok(())
}

/// The inside of an hjkl `<…>` key: `C-` / `S-` / `A-` / `M-` prefixes
/// (any case), then a special name or one character, as hjkl-keymap reads
/// them.
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
            _ => break,
        }
        rest = tail;
    }
    let code = match rest.to_ascii_lowercase().as_str() {
        "space" => KeyCode::Char(' '),
        "lt" => KeyCode::Char('<'),
        "gt" => KeyCode::Char('>'),
        "cr" | "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "bs" | "backspace" => KeyCode::Backspace,
        "del" | "delete" => KeyCode::Delete,
        "ins" | "insert" => KeyCode::Insert,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        name => match name.strip_prefix('f').map(str::parse::<u8>) {
            Some(Ok(n)) if (1..=24).contains(&n) => KeyCode::F(n),
            _ => {
                let mut chars = rest.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => KeyCode::Char(c),
                    _ => return None,
                }
            }
        },
    };
    Some(Key::new(code, mods))
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_mods(f, self.mods)?;
        match self.code {
            KeyCode::Char(' ') => f.write_str("Space"),
            KeyCode::Char(c) => write!(f, "{c}"),
            KeyCode::F(n) => write!(f, "F{n}"),
            code => {
                let name = NAMED
                    .iter()
                    .find(|(_, c)| *c == code)
                    .map_or("Unknown", |(name, _)| name);
                f.write_str(name)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> Key {
        s.parse().unwrap()
    }

    #[test]
    fn names_parse_and_print_back() {
        for name in [
            "C-b", "M-h", "C-\\", "'", "\"", ";", "%", "x", "C-l", "Enter", "Escape", "BSpace",
            "Space", "PPage", "F5", "C-M-x", "M-Up", "C-Left", "BTab", "Tab", "-", "C--",
        ] {
            assert_eq!(k(name).to_string(), name, "{name}");
        }
        assert_eq!(k("PageUp"), k("PPage"));
        assert_eq!(k("c-b"), k("C-b"));
        assert!("Nope".parse::<Key>().is_err());
        // hjkl's notation names the same keys.
        for (hjkl, tmux) in [
            ("<C-b>", "C-b"),
            ("<c-b>", "C-b"),
            ("<M-h>", "M-h"),
            ("<A-h>", "M-h"),
            ("<CR>", "Enter"),
            ("<Esc>", "Escape"),
            ("<BS>", "BSpace"),
            ("<Space>", "Space"),
            ("<C-Space>", "C-Space"),
            ("<lt>", "<"),
            ("<F5>", "F5"),
            ("<C-M-x>", "C-M-x"),
            ("<M-Up>", "M-Up"),
        ] {
            assert_eq!(k(hjkl), k(tmux), "{hjkl}");
        }
        assert!("<Nope>".parse::<Key>().is_err());
        // A lone < or > is still the key itself.
        assert_eq!(k("<").code, KeyCode::Char('<'));
        assert!("".parse::<Key>().is_err());
    }

    #[test]
    fn terminal_variants_of_one_chord_are_equal() {
        // C-\ as legacy byte 0x1c (crossterm: C-4) and as an explicit char.
        let legacy = Key::from_event(&KeyEvent::new(KeyCode::Char('4'), KeyModifiers::CONTROL));
        assert_eq!(legacy, k("C-\\"));
        // Shifted characters carry their shift.
        let pct = Key::from_event(&KeyEvent::new(KeyCode::Char('%'), KeyModifiers::SHIFT));
        assert_eq!(pct, k("%"));
        let upper = Key::from_event(&KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT));
        assert_eq!(upper, k("R"));
        // Ctrl letters ignore case.
        let ch = Key::from_event(&KeyEvent::new(
            KeyCode::Char('H'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert_eq!(ch, k("C-h"));
        // Super / Hyper never take part in matching.
        let sup = Key::from_event(&KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::ALT | KeyModifiers::SUPER,
        ));
        assert_eq!(sup, k("M-h"));
    }
}
