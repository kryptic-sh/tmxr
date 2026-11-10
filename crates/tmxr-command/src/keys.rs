//! Keys as key tables bind them: the [`Key`] type, how a terminal's key
//! event becomes one, and back. How a key is named is [`names`].

mod names;

use std::fmt;

use ttyr::event::{KeyCode, KeyEncoding, KeyEvent, KeyEventState, KeyModifiers};

/// A key as bound in a key table: a key code, the modifiers held with it
/// (tmux's Ctrl, Alt and Shift, and the kitty protocol's Super, Hyper and
/// Meta) and whether it is on the keypad.
///
/// Built from terminal events with [`Key::from_event`], which normalises the
/// ways different terminals report the same chord, so a bind made for `C-\`
/// matches whether the terminal sent `0x1c` or a kitty `CSI 92;5u`.
///
/// Shift is carried by the character a key types (`A`, `%`, `M-A`), except
/// on a character held with Ctrl: there it is kept, so C-S-h and C-S-1 are
/// keys of their own, as in tmux 3.6. tmux's C-H (Ctrl with an upper-case
/// letter and no Shift) is a third key beside C-h and C-S-h, which
/// `send-keys` sends as tmux does. `S-Tab` is apart from `BTab`, as in
/// tmux, which a terminal sends for Shift+Tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub mods: KeyModifiers,
    /// The key is on the keypad (tmux's `KP` keys): only a terminal under
    /// the kitty protocol reports one apart from its main key.
    pub keypad: bool,
}

const MODS: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SHIFT)
    .union(KeyModifiers::SUPER)
    .union(KeyModifiers::HYPER)
    .union(KeyModifiers::META);

impl Key {
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        Self::normalise(code, mods & MODS, false)
    }

    /// This key on the keypad, where the keypad has it (`KP1`, `KPEnter`);
    /// any other key is unchanged.
    #[must_use]
    pub fn on_keypad(self) -> Self {
        Self::normalise(self.code, self.mods, true)
    }

    /// The main key a keypad key stands for (`1` for `KP1`), which a key
    /// table falls back to when it binds no keypad key: a bind on `1` fires
    /// from the keypad's 1 too, as it does from a terminal that does not
    /// tell the two apart. `None` for a key not on the keypad.
    pub fn main_key(self) -> Option<Self> {
        self.keypad.then_some(Self {
            keypad: false,
            ..self
        })
    }

    /// The key a terminal's event names, as tmux reads it (see [`Typed`]).
    /// A legacy terminal's C0 bytes for C-\ C-] C-^ C-_ and C-Space, which
    /// ttyr reads as Ctrl with 4 to 7 (and Windows reports as Ctrl with 2
    /// or `@`), are the keys tmux names them, as tmux's tty_keys_next; from
    /// a terminal that reports keys apart, Ctrl+4 is C-4.
    pub fn from_event(ev: &KeyEvent) -> Self {
        let typed = Typed::of(ev);
        let code = match typed.key {
            KeyCode::Char(c)
                if ev.encoding == KeyEncoding::Legacy
                    && typed.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                KeyCode::Char(match c {
                    '2' | '@' => ' ',
                    '4' => '\\',
                    '5' => ']',
                    '6' => '^',
                    '7' => '_',
                    c => c,
                })
            }
            key => key,
        };
        let keypad = ev.state.contains(KeyEventState::KEYPAD);
        Self::normalise(code, typed.modifiers & MODS, keypad)
    }

    /// This key as a terminal reports it, in ttyr's meaning: an upper-case
    /// letter is its lower-case key with Shift, the letter its shifted key,
    /// as ttyr's decoder reads the letter's byte; but tmux's C-A, Ctrl with
    /// the upper-case letter and no Shift, is that letter's key. For keys
    /// tmxr makes itself (`send-keys`, `send-prefix`).
    pub fn to_event(self) -> KeyEvent {
        let ctrl = self.mods.contains(KeyModifiers::CONTROL);
        let shift = self.mods.contains(KeyModifiers::SHIFT);
        let state = if self.keypad {
            KeyEventState::KEYPAD
        } else {
            KeyEventState::NONE
        };
        let event = match self.code {
            KeyCode::Char(c) => match single_lower(c) {
                Some(lower) if shift || !ctrl => {
                    KeyEvent::new(KeyCode::Char(lower), self.mods | KeyModifiers::SHIFT)
                        .with_shifted(Some(c))
                }
                _ if shift => KeyEvent::new(self.code, self.mods).with_shifted(single_upper(c)),
                _ => KeyEvent::new(self.code, self.mods),
            },
            code => KeyEvent::new(code, self.mods),
        };
        event.with_state(state)
    }

    fn normalise(code: KeyCode, mut mods: KeyModifiers, keypad: bool) -> Self {
        let shift = mods.contains(KeyModifiers::SHIFT);
        let code = match code {
            KeyCode::Char(c) if mods.contains(KeyModifiers::CONTROL) => {
                // Ctrl keeps Shift apart (C-S-h, C-S-1), and an upper-case
                // letter with both is the same key (C-S-H). With no Shift,
                // the upper-case letter is tmux's C-H.
                match single_lower(c) {
                    Some(lower) if shift => KeyCode::Char(lower),
                    _ => KeyCode::Char(c),
                }
            }
            KeyCode::Char(c) => {
                // Anything else carries Shift in its character ('A', '%'),
                // so Shift written beside one is dropped. That is the key
                // tmux's send-keys sends a legacy pane for M-S-a (M-a); at
                // modifyOtherKeys level 2 tmux keeps the Shift
                // (`CSI 27;4;104~` for M-S-h), which tmxr does not.
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::Char(c)
            }
            KeyCode::BackTab => {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::BackTab
            }
            other => other,
        };
        Self {
            code,
            mods,
            keypad: keypad && on_the_keypad(code),
        }
    }
}

/// Whether the keypad has a key that is `code` off it: the digits and the
/// operators, Enter, and (with Num Lock off) the cursor and editing keys,
/// as the kitty protocol's `KP_*` keys.
fn on_the_keypad(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::Char('0'..='9' | '.' | '/' | '*' | '-' | '+' | '=' | ',')
            | KeyCode::Enter
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Insert
            | KeyCode::Delete
    )
}

/// A key event as tmux reads a key: the character typed, with Shift only
/// where the character does not already carry it (`A`, `!` and `M-A`
/// rather than ttyr's unshifted key with Shift), except a letter with
/// Ctrl, which stays the letter's key with Shift as reported (C-S-h, which
/// a pane under modifyOtherKeys tells from C-h). Any other key is as
/// reported. What tmxr's own key handling (binds, copy mode, the prompt,
/// menus, pickers) reads; a pane's program gets the event itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Typed {
    pub key: KeyCode,
    pub modifiers: KeyModifiers,
}

impl Typed {
    pub fn of(ev: &KeyEvent) -> Self {
        let letter_chord = ev.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(ev.key, KeyCode::Char(c) if c.is_lowercase());
        match ev.logical_char() {
            Some(c) if !letter_chord => Self {
                key: KeyCode::Char(c),
                modifiers: ev.logical_modifiers(),
            },
            _ => Self {
                key: ev.key,
                modifiers: ev.modifiers,
            },
        }
    }
}

/// A key event as tmux reads its Meta, given whether the decoder that read
/// it took the kitty keyboard protocol to be on (`kitty`). Off, Meta is
/// xterm's (modifier bit 8, in `CSI 1 ; 9 A` and `CSI 27 ; 9 ; … ~`), which
/// tmux reads as `M-`, the same as Alt: so it is Alt here, and a bind on
/// `M-h` fires and a pane is sent `M-h`. On, Meta is the kitty protocol's
/// own, kept as `Meta-`. Which it was cannot be told from the event: kitty
/// sends its Meta+Up in the same legacy form xterm does.
pub fn xterm_meta_as_alt(mut ev: KeyEvent, kitty: bool) -> KeyEvent {
    if !kitty && ev.modifiers.contains(KeyModifiers::META) {
        ev.modifiers.remove(KeyModifiers::META);
        ev.modifiers.insert(KeyModifiers::ALT);
    }
    ev
}

/// `c`'s lower case, where `c` is upper case and its lower case is one
/// other character, as ttyr's decoder takes a letter's key.
pub fn single_lower(c: char) -> Option<char> {
    if !c.is_uppercase() {
        return None;
    }
    let mut lower = c.to_lowercase();
    let l = lower.next()?;
    (lower.next().is_none() && l != c).then_some(l)
}

/// `c`'s upper case, where `c` is lower case and its upper case is one
/// other character.
pub(crate) fn single_upper(c: char) -> Option<char> {
    if !c.is_lowercase() {
        return None;
    }
    let mut upper = c.to_uppercase();
    let u = upper.next()?;
    (upper.next().is_none() && u != c).then_some(u)
}

/// Error for an unrecognised key name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown key: {0}")]
pub struct UnknownKey(pub String);

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

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> Key {
        s.parse().unwrap()
    }

    /// A terminal asked for kitty flag 8 (and 16, and the lock keys it
    /// then reports) sends what it typed as escape codes; through ttyr's
    /// decoder each is the key its legacy bytes are, so binds and key
    /// tables see no difference; a keypad key falls back to its main key.
    #[test]
    fn keys_reported_as_escape_codes_are_the_keys_typed() {
        let decode = |bytes: &[u8]| {
            let mut decoder = ttyr::event::Decoder::new();
            decoder.feed(bytes);
            match decoder.next_event().or_else(|| decoder.flush_event()) {
                Some(ttyr::event::Event::Key(ev)) => Key::from_event(&ev),
                other => panic!("{bytes:?}: {other:?}"),
            }
        };
        for (legacy, escaped) in [
            (
                &b"a"[..],
                &[&b"\x1b[97u"[..], b"\x1b[97;;97u", b"\x1b[97;129u"][..],
            ),
            (
                b"A",
                &[b"\x1b[97:65;2u", b"\x1b[97:65;2;65u", b"\x1b[97;65u"],
            ),
            (b"!", &[b"\x1b[49:33;2u", b"\x1b[49:33;2;33u"]),
            (b" ", &[b"\x1b[32u", b"\x1b[32;;32u"]),
            ("é".as_bytes(), &[b"\x1b[233u", b"\x1b[233;;233u"]),
            (b"\r", &[b"\x1b[13u"]),
            (b"\t", &[b"\x1b[9u"]),
            (b"\x7f", &[b"\x1b[127u"]),
            (b"\x01", &[b"\x1b[97;5u"]),
        ] {
            let want = decode(legacy);
            for bytes in escaped {
                assert_eq!(decode(bytes), want, "{bytes:?}");
            }
        }
        // The keypad's 1 is its own key, with the 1 a key table falls back
        // to behind it.
        let kp1 = decode(b"\x1b[57400;129;49u");
        assert!(kp1.keypad);
        assert_eq!(kp1.main_key(), Some(decode(b"1")));
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
        // tmux's caret: ^a is C-a, and ^ alone the key.
        assert_eq!(k("^a"), k("C-a"));
        assert_eq!(k("^M-b"), k("C-M-b"));
        assert_eq!(k("^").code, KeyCode::Char('^'));
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
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        let legacy = |c| Key::from_event(&ctrl(c).with_encoding(KeyEncoding::Legacy));
        // C-\ C-] C-^ C-_ as the legacy bytes 0x1c..0x1f (ttyr: Ctrl with 4
        // to 7), and C-Space as 0x00, which Windows reports as Ctrl with 2
        // or @: the keys tmux names those bytes.
        for (c, name) in [
            ('4', "C-\\"),
            ('5', "C-]"),
            ('6', "C-^"),
            ('7', "C-_"),
            ('2', "C-Space"),
            ('@', "C-Space"),
        ] {
            assert_eq!(legacy(c), k(name), "{c}");
        }
        // From a terminal that reports keys apart (kitty's CSI 52;5u),
        // Ctrl+4 is C-4, as tmux 3.6 keeps the names apart.
        for c in ['4', '5', '6', '7', '2', '@'] {
            assert_eq!(
                Key::from_event(&ctrl(c)),
                Key::new(KeyCode::Char(c), KeyModifiers::CONTROL)
            );
        }
        assert_ne!(k("C-4"), k("C-\\"));
        assert_ne!(k("C-@"), k("C-Space"));
        assert_ne!(k("C-2"), k("C-Space"));
        // Shifted characters carry their shift: a legacy `%`, a legacy
        // `R` (ttyr's key `r` with Shift, `R` shifted), and kitty's Shift+5
        // reporting alternate keys (`5` with Shift, `%` shifted).
        let ev = |c, m| KeyEvent::new(KeyCode::Char(c), m);
        let pct = Key::from_event(&ev('%', KeyModifiers::NONE));
        assert_eq!(pct, k("%"));
        let upper = Key::from_event(&ev('r', KeyModifiers::SHIFT).with_shifted(Some('R')));
        assert_eq!(upper, k("R"));
        let five = Key::from_event(&ev('5', KeyModifiers::SHIFT).with_shifted(Some('%')));
        assert_eq!(five, k("%"));
        // A Ctrl letter keeps Shift apart: Ctrl+Shift+h from kitty
        // (`CSI 104;6u`) or xterm (`CSI 27;6;72~`, ttyr's `h` with Shift
        // and `H` shifted) is C-S-h, not C-h; and the legacy byte 0x08 is
        // C-h.
        let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        let ch = Key::from_event(&ev('h', ctrl_shift).with_shifted(Some('H')));
        assert_eq!(ch, k("C-S-h"));
        assert_eq!(Key::from_event(&ev('h', ctrl_shift)), k("C-S-h"));
        assert_ne!(ch, k("C-h"));
        assert_eq!(legacy('h'), k("C-h"));
        // Alt with a shifted letter, from a legacy `ESC A` or kitty's
        // `CSI 97:65;4u` alike, is M-A and not M-a; without the shifted key
        // (kitty's `CSI 97;4u`) too.
        let alt_shift = KeyModifiers::ALT | KeyModifiers::SHIFT;
        let meta_upper = Key::from_event(&ev('a', alt_shift).with_shifted(Some('A')));
        assert_eq!(meta_upper, k("M-A"));
        assert_ne!(meta_upper, k("M-a"));
        assert_eq!(Key::from_event(&ev('a', alt_shift)), k("M-A"));
        // Alt+Shift+1 reporting alternate keys is M-!, as tmux names it.
        let bang = Key::from_event(&ev('1', alt_shift).with_shifted(Some('!')));
        assert_eq!(bang, k("M-!"));
        // Super, Hyper and Meta are part of the key; the lock keys are not.
        let sup = Key::from_event(&ev('h', KeyModifiers::ALT | KeyModifiers::SUPER));
        assert_eq!(sup, k("M-Super-h"));
        assert_ne!(sup, k("M-h"));
        let hyper_meta = Key::from_event(&ev('x', KeyModifiers::HYPER | KeyModifiers::META));
        assert_eq!(hyper_meta, k("Hyper-Meta-x"));
        let caps =
            Key::from_event(&ev('h', KeyModifiers::CONTROL).with_state(KeyEventState::CAPS_LOCK));
        assert_eq!(caps, k("C-h"));
        // The keypad's keys are their own, from kitty's `CSI 57400u` and a
        // legacy terminal's `ESC O M` alike, with their main key behind them.
        let kp1 = Key::from_event(&ev('1', KeyModifiers::NONE).with_state(KeyEventState::KEYPAD));
        assert_eq!(kp1, k("KP1"));
        assert_ne!(kp1, k("1"));
        assert_eq!(kp1.main_key(), Some(k("1")));
        assert_eq!(k("1").main_key(), None);
        let mut decoder = ttyr::event::Decoder::new();
        decoder.feed(b"\x1bOM");
        let Some(ttyr::event::Event::Key(kp_enter)) = decoder.flush_event() else {
            panic!("not a key");
        };
        assert_eq!(Key::from_event(&kp_enter), k("KPEnter"));
    }

    /// xterm's Meta (modifier bit 8) is `M-`, as tmux reads it, from a
    /// decoder outside the kitty protocol; under it, bit 8 is Super and
    /// kitty's own Meta (32) stays `Meta-`, in its `CSI u` and legacy forms.
    #[test]
    fn xterm_meta_is_alt_and_kittys_is_meta() {
        let read = |bytes: &[u8], kitty: bool| {
            let mut decoder = ttyr::event::Decoder::new();
            decoder.set_kitty_keyboard(kitty);
            decoder.feed(bytes);
            match decoder.next_event().or_else(|| decoder.flush_event()) {
                Some(ttyr::event::Event::Key(ev)) => Key::from_event(&xterm_meta_as_alt(ev, kitty)),
                other => panic!("{bytes:?}: {other:?}"),
            }
        };
        for (bytes, name) in [
            (&b"\x1b[27;9;120~"[..], "M-x"),
            (b"\x1b[27;13;120~", "C-M-x"),
            (b"\x1b[1;9A", "M-Up"),
            (b"\x1b[15;9~", "M-F5"),
            (b"\x1b[120;9u", "M-x"),
        ] {
            assert_eq!(read(bytes, false), k(name), "{bytes:?}");
        }
        for (bytes, name) in [
            (&b"\x1b[120;33u"[..], "Meta-x"),
            (b"\x1b[1;33A", "Meta-Up"),
            (b"\x1b[120;9u", "Super-x"),
            (b"\x1b[1;9A", "Super-Up"),
        ] {
            assert_eq!(read(bytes, true), k(name), "{bytes:?}");
        }
    }

    #[test]
    fn a_key_made_by_tmxr_is_what_a_terminal_reports() {
        // An upper-case letter as ttyr's decoder reads `ESC A`.
        let mut decoder = ttyr::event::Decoder::new();
        decoder.feed(b"\x1bA");
        let Some(ttyr::event::Event::Key(alt_a)) = decoder.flush_event() else {
            panic!("not a key");
        };
        let made = k("M-A").to_event();
        assert_eq!(
            (made.key, made.modifiers, made.shifted),
            (alt_a.key, alt_a.modifiers, alt_a.shifted)
        );
        // tmux's C-A is the upper-case letter's key with Ctrl alone, which
        // the encoder sends as tmux does (0x01, or 65 extended); C-S-a is
        // the letter's key with Shift.
        let made = k("C-A").to_event();
        assert_eq!(
            (made.key, made.modifiers),
            (KeyCode::Char('A'), KeyModifiers::CONTROL)
        );
        let made = k("C-S-a").to_event();
        assert_eq!(
            (made.key, made.modifiers, made.shifted),
            (
                KeyCode::Char('a'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                Some('A')
            )
        );
        // And back again, as every other key.
        for name in [
            "M-A",
            "C-b",
            "%",
            "Enter",
            "M-Up",
            "C-\\",
            "É",
            "C-A",
            "C-S-a",
            "C-4",
            "C-@",
            "KP1",
            "C-KPEnter",
            "Super-k",
            "C-S-Super-h",
            "Meta-x",
            "F35",
            "MediaPlay",
            "LeftShift",
        ] {
            assert_eq!(Key::from_event(&k(name).to_event()), k(name), "{name}");
        }
    }

    #[test]
    fn typed_keeps_shift_on_a_ctrl_letter_only() {
        let ev = |c, m| KeyEvent::new(KeyCode::Char(c), m);
        let typed = |e: &KeyEvent| {
            let t = Typed::of(e);
            (t.key, t.modifiers)
        };
        let (c, s, a) = (
            KeyModifiers::CONTROL,
            KeyModifiers::SHIFT,
            KeyModifiers::ALT,
        );
        assert_eq!(
            typed(&ev('h', c | s).with_shifted(Some('H'))),
            (KeyCode::Char('h'), c | s)
        );
        assert_eq!(
            typed(&ev('a', a | s).with_shifted(Some('A'))),
            (KeyCode::Char('A'), a)
        );
        assert_eq!(
            typed(&ev('1', c | s).with_shifted(Some('!'))),
            (KeyCode::Char('!'), c)
        );
        assert_eq!(typed(&ev('a', s)), (KeyCode::Char('A'), KeyModifiers::NONE));
        let up = KeyEvent::new(KeyCode::Up, s);
        assert_eq!(typed(&up), (KeyCode::Up, s));
    }
}
