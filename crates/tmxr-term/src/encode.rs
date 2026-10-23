//! Key and paste events → bytes for the program in a pane.
//!
//! Legacy encodings follow xterm's ctlseqs ("PC-Style Function Keys",
//! "Alt and Meta Keys"). Keys with no legacy form (`C-;`, `C-Enter`,
//! `C-S-h`, …) use the `CSI <codepoint> ; <modifiers> u` form when the
//! program asked for extended keys (kitty flags or modifyOtherKeys), or when
//! the server option `extended-keys always` forces them.

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use vt1k::{MouseProtocolEncoding, MouseProtocolMode};

/// The pane state key encoding depends on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputModes {
    /// DECCKM: cursor keys send `SS3` instead of `CSI`.
    pub app_cursor: bool,
    /// Wrap pastes in `CSI 200~` / `CSI 201~`.
    pub bracketed_paste: bool,
    /// Current kitty keyboard flags (bit 0: disambiguate escape codes).
    pub kitty_flags: u8,
    /// xterm modifyOtherKeys level.
    pub modify_other_keys: u8,
}

/// The `extended-keys` option, as tmux's: whether a program's request for
/// extended keys (xterm modifyOtherKeys) is honoured, and whether keys are
/// extended without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExtendedKeys {
    /// Requests are ignored.
    Off,
    /// Keys are extended when the program asks.
    #[default]
    On,
    /// Keys are always extended.
    Always,
}

impl ExtendedKeys {
    /// The option's value; anything but `on` and `always` is `off`, as
    /// tmux's default.
    pub fn from_option(value: &str) -> Self {
        match value {
            "on" => Self::On,
            "always" => Self::Always,
            _ => Self::Off,
        }
    }
}

impl InputModes {
    /// The modifyOtherKeys level keys go out at: the pane's (which a
    /// request made under `extended-keys off` never set), at least 1 with
    /// `always`.
    pub fn key_level(self, option: ExtendedKeys) -> u8 {
        match option {
            ExtendedKeys::Always => self.modify_other_keys.max(1),
            ExtendedKeys::Off | ExtendedKeys::On => self.modify_other_keys,
        }
    }

    /// Whether keys go extended: the kitty protocol's flag (which tmux has
    /// no counterpart for, so the option leaves it alone) or a key level.
    fn extended(self, option: ExtendedKeys) -> bool {
        self.kitty_flags & 1 != 0 || self.key_level(option) > 0
    }
}

/// xterm modifier parameter: 1 + (shift | alt << 1 | ctrl << 2).
fn modifier_param(m: KeyModifiers) -> u8 {
    1 + u8::from(m.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(m.contains(KeyModifiers::ALT))
        + 4 * u8::from(m.contains(KeyModifiers::CONTROL))
}

fn csi_u(codepoint: u32, m: KeyModifiers) -> Vec<u8> {
    match modifier_param(m) {
        1 => format!("\x1b[{codepoint}u").into_bytes(),
        p => format!("\x1b[{codepoint};{p}u").into_bytes(),
    }
}

/// The C0 control byte for `C-<c>`, where one exists.
fn ctrl_byte(c: char) -> Option<u8> {
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        'A'..='Z' => c as u8 - b'A' + 1,
        '@' | ' ' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '/' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

/// Encode a key for a pane under the `extended-keys` option. Release
/// events and keys with no encoding produce nothing.
pub fn encode_key(key: &KeyEvent, modes: InputModes, option: ExtendedKeys) -> Vec<u8> {
    if key.kind == KeyEventKind::Release {
        return Vec::new();
    }
    let m = key.modifiers & (KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL);
    let alt = m.contains(KeyModifiers::ALT);
    let ctrl = m.contains(KeyModifiers::CONTROL);
    let extended = modes.extended(option);
    let kitty = modes.kitty_flags & 1 != 0;

    let with_alt = |mut bytes: Vec<u8>| {
        if alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };
    let cursor = |letter: char| -> Vec<u8> {
        match modifier_param(m) {
            1 if modes.app_cursor => format!("\x1bO{letter}").into_bytes(),
            1 => format!("\x1b[{letter}").into_bytes(),
            p => format!("\x1b[1;{p}{letter}").into_bytes(),
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        match modifier_param(m) {
            1 => format!("\x1b[{n}~").into_bytes(),
            p => format!("\x1b[{n};{p}~").into_bytes(),
        }
    };
    let ss3 = |letter: char| -> Vec<u8> {
        match modifier_param(m) {
            1 => format!("\x1bO{letter}").into_bytes(),
            p => format!("\x1b[1;{p}{letter}").into_bytes(),
        }
    };

    match key.code {
        KeyCode::Char(c) => {
            if kitty && (ctrl || alt) {
                // kitty reports the unshifted key with the shift modifier.
                return csi_u(u32::from(c.to_ascii_lowercase()), m);
            }
            if ctrl {
                let shifted_letter = c.is_ascii_alphabetic() && m.contains(KeyModifiers::SHIFT);
                return match ctrl_byte(c) {
                    Some(_) if shifted_letter && extended => {
                        csi_u(u32::from(c.to_ascii_lowercase()), m)
                    }
                    Some(b) => with_alt(vec![b]),
                    None if extended => csi_u(u32::from(c), m),
                    None => Vec::new(),
                };
            }
            let mut buf = [0u8; 4];
            with_alt(c.encode_utf8(&mut buf).as_bytes().to_vec())
        }
        KeyCode::Enter if (ctrl || m.contains(KeyModifiers::SHIFT)) && extended => csi_u(13, m),
        KeyCode::Enter => with_alt(b"\r".to_vec()),
        KeyCode::Tab if ctrl && extended => csi_u(9, m),
        KeyCode::Tab if m.contains(KeyModifiers::SHIFT) => b"\x1b[Z".to_vec(),
        KeyCode::Tab => with_alt(b"\t".to_vec()),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace if ctrl => with_alt(vec![0x08]),
        KeyCode::Backspace => with_alt(vec![0x7f]),
        KeyCode::Esc if kitty => csi_u(27, m),
        KeyCode::Esc => with_alt(vec![0x1b]),
        KeyCode::Up => cursor('A'),
        KeyCode::Down => cursor('B'),
        KeyCode::Right => cursor('C'),
        KeyCode::Left => cursor('D'),
        KeyCode::Home => cursor('H'),
        KeyCode::End => cursor('F'),
        KeyCode::Insert => tilde(2),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::F(1) => ss3('P'),
        KeyCode::F(2) => ss3('Q'),
        KeyCode::F(3) => ss3('R'),
        KeyCode::F(4) => ss3('S'),
        KeyCode::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]),
        _ => Vec::new(),
    }
}

/// A Windows pane's program reads keys through ConPTY, which takes the bytes
/// for C-h and C-j (0x08, 0x0A, after an ESC with Alt) as Ctrl+Backspace and
/// Ctrl+Enter. For those two keys, given the `bytes` [`encode_key`] made,
/// this is their win32-input-mode form instead (`CSI Vk;Sc;Uc;Kd;Cs;Rc _`, a
/// press and a release), which ConPTY reads as the exact key; `None` for
/// every other key, whose bytes ConPTY reads right.
pub fn conpty_key(key: &KeyEvent, bytes: &[u8]) -> Option<Vec<u8>> {
    // The modifier state bits of a KEY_EVENT_RECORD.
    const LEFT_ALT: u8 = 0x02;
    const LEFT_CTRL: u8 = 0x08;
    const SHIFT: u8 = 0x10;
    let KeyCode::Char(c) = key.code else {
        return None;
    };
    let c = c.to_ascii_lowercase();
    let control = match bytes {
        [b] | [0x1b, b] if matches!(b, 0x08 | 0x0a) => *b,
        _ => return None,
    };
    if !key.modifiers.contains(KeyModifiers::CONTROL) || !matches!(c, 'h' | 'j') {
        return None;
    }
    let mut state = LEFT_CTRL;
    if key.modifiers.contains(KeyModifiers::ALT) {
        state |= LEFT_ALT;
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        state |= SHIFT;
    }
    // A letter's virtual-key code is its capital's ASCII code.
    let vk = c.to_ascii_uppercase() as u8;
    Some(format!("[{vk};0;{control};1;{state};1_[{vk};0;{control};0;{state};1_").into_bytes())
}

/// Encode pasted text for a pane, bracketed if the program asked for it.
/// Bracket markers inside the text are removed so a paste cannot end the
/// bracket early and have the rest run as typed input.
pub fn encode_paste(text: &str, modes: InputModes) -> Vec<u8> {
    if !modes.bracketed_paste {
        return text.as_bytes().to_vec();
    }
    let clean = text.replace("\x1b[200~", "").replace("\x1b[201~", "");
    let mut out = b"\x1b[200~".to_vec();
    out.extend_from_slice(clean.as_bytes());
    out.extend_from_slice(b"\x1b[201~");
    out
}

/// Encode a mouse event for a pane at pane-relative (`col`, `row`), in the
/// protocol the program enabled (xterm ctlseqs "Mouse Tracking"). Events the
/// mode does not report produce nothing.
pub fn encode_mouse(
    ev: &MouseEvent,
    col: u16,
    row: u16,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Vec<u8> {
    let button = |b: MouseButton| match b {
        MouseButton::Left => 0u32,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let (code, release, reported) = match ev.kind {
        MouseEventKind::Down(b) => (button(b), false, mode != MouseProtocolMode::None),
        MouseEventKind::Up(b) => (
            button(b),
            true,
            !matches!(mode, MouseProtocolMode::None | MouseProtocolMode::Press),
        ),
        MouseEventKind::Drag(b) => (
            button(b) + 32,
            false,
            matches!(
                mode,
                MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion
            ),
        ),
        MouseEventKind::Moved => (3 + 32, false, mode == MouseProtocolMode::AnyMotion),
        MouseEventKind::ScrollUp => (64, false, mode != MouseProtocolMode::None),
        MouseEventKind::ScrollDown => (65, false, mode != MouseProtocolMode::None),
        MouseEventKind::ScrollLeft => (66, false, mode != MouseProtocolMode::None),
        MouseEventKind::ScrollRight => (67, false, mode != MouseProtocolMode::None),
    };
    if !reported {
        return Vec::new();
    }
    let m = ev.modifiers;
    let code = code
        + 4 * u32::from(m.contains(KeyModifiers::SHIFT))
        + 8 * u32::from(m.contains(KeyModifiers::ALT))
        + 16 * u32::from(m.contains(KeyModifiers::CONTROL));
    let (x, y) = (u32::from(col) + 1, u32::from(row) + 1);
    match encoding {
        MouseProtocolEncoding::Sgr => {
            format!("\x1b[<{code};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes()
        }
        MouseProtocolEncoding::Default | MouseProtocolEncoding::Utf8 => {
            // The legacy forms cannot say which button was released.
            let code = if release { 3 + (code & !3) } else { code };
            let mut out = b"\x1b[M".to_vec();
            for v in [code + 32, x + 32, y + 32] {
                match encoding {
                    MouseProtocolEncoding::Utf8 => {
                        let Some(c) = char::from_u32(v) else {
                            return Vec::new();
                        };
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                    }
                    _ => match u8::try_from(v) {
                        Ok(b) => out.push(b),
                        // Out of range for the one-byte form: not reportable.
                        Err(_) => return Vec::new(),
                    },
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conpty_takes_c_h_and_c_j_as_win32_input() {
        let ctrl = |c, m| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL | m);
        let win32 = |key: KeyEvent| {
            let bytes = encode_key(&key, InputModes::default(), ExtendedKeys::On);
            conpty_key(&key, &bytes).map(|b| String::from_utf8(b).unwrap())
        };
        assert_eq!(
            win32(ctrl('j', KeyModifiers::NONE)).as_deref(),
            Some("[74;0;10;1;8;1_[74;0;10;0;8;1_")
        );
        assert_eq!(
            win32(ctrl('h', KeyModifiers::ALT)).as_deref(),
            Some("[72;0;8;1;10;1_[72;0;8;0;10;1_")
        );
        // ConPTY reads the rest right, Ctrl+Backspace's 0x08 among them.
        assert_eq!(win32(ctrl('k', KeyModifiers::NONE)), None);
        assert_eq!(win32(ctrl('l', KeyModifiers::NONE)), None);
        assert_eq!(
            win32(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(
            win32(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
            None
        );
        // A program that asked for extended keys gets those, not this.
        let kitty = InputModes {
            kitty_flags: 1,
            ..InputModes::default()
        };
        let key = ctrl('j', KeyModifiers::NONE);
        assert_eq!(
            conpty_key(&key, &encode_key(&key, kitty, ExtendedKeys::On)),
            None
        );
    }

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const C: KeyModifiers = KeyModifiers::CONTROL;
    const A: KeyModifiers = KeyModifiers::ALT;
    const S: KeyModifiers = KeyModifiers::SHIFT;

    fn enc(code: KeyCode, m: KeyModifiers) -> Vec<u8> {
        encode_key(
            &KeyEvent::new(code, m),
            InputModes::default(),
            ExtendedKeys::On,
        )
    }

    #[test]
    fn legacy_table() {
        let cases: &[(KeyCode, KeyModifiers, &[u8])] = &[
            (KeyCode::Char('a'), NONE, b"a"),
            (KeyCode::Char('A'), S, b"A"),
            (KeyCode::Char('é'), NONE, "é".as_bytes()),
            (KeyCode::Char('h'), C, b"\x08"),
            (KeyCode::Char('l'), C, b"\x0c"),
            (KeyCode::Char('\\'), C, b"\x1c"),
            (KeyCode::Char(' '), C, b"\x00"),
            (KeyCode::Char('h'), A, b"\x1bh"),
            (KeyCode::Char('x'), C | A, b"\x1b\x18"),
            (KeyCode::Enter, NONE, b"\r"),
            (KeyCode::Tab, NONE, b"\t"),
            (KeyCode::BackTab, S, b"\x1b[Z"),
            (KeyCode::Backspace, NONE, b"\x7f"),
            (KeyCode::Esc, NONE, b"\x1b"),
            (KeyCode::Up, NONE, b"\x1b[A"),
            (KeyCode::Left, C, b"\x1b[1;5D"),
            (KeyCode::Right, S | A, b"\x1b[1;4C"),
            (KeyCode::Home, NONE, b"\x1b[H"),
            (KeyCode::End, NONE, b"\x1b[F"),
            (KeyCode::Delete, NONE, b"\x1b[3~"),
            (KeyCode::PageUp, C, b"\x1b[5;5~"),
            (KeyCode::F(1), NONE, b"\x1bOP"),
            (KeyCode::F(4), S, b"\x1b[1;2S"),
            (KeyCode::F(5), NONE, b"\x1b[15~"),
            (KeyCode::F(12), NONE, b"\x1b[24~"),
        ];
        for (code, m, want) in cases {
            assert_eq!(enc(*code, *m), *want, "{code:?} {m:?}");
        }
    }

    #[test]
    fn application_cursor_mode_uses_ss3() {
        let modes = InputModes {
            app_cursor: true,
            ..InputModes::default()
        };
        let up = KeyEvent::new(KeyCode::Up, NONE);
        assert_eq!(encode_key(&up, modes, ExtendedKeys::On), b"\x1bOA");
        let c_up = KeyEvent::new(KeyCode::Up, C);
        assert_eq!(encode_key(&c_up, modes, ExtendedKeys::On), b"\x1b[1;5A");
    }

    #[test]
    fn keys_without_legacy_form_need_extended_mode() {
        let c_semi = KeyEvent::new(KeyCode::Char(';'), C);
        assert!(encode_key(&c_semi, InputModes::default(), ExtendedKeys::On).is_empty());
        assert_eq!(
            encode_key(&c_semi, InputModes::default(), ExtendedKeys::Always),
            b"\x1b[59;5u"
        );
        let mok = InputModes {
            modify_other_keys: 2,
            ..InputModes::default()
        };
        assert_eq!(encode_key(&c_semi, mok, ExtendedKeys::On), b"\x1b[59;5u");
        let c_enter = KeyEvent::new(KeyCode::Enter, C);
        assert_eq!(encode_key(&c_enter, mok, ExtendedKeys::On), b"\x1b[13;5u");
        // A level the pane holds stands whatever the option is now, as
        // tmux's; `always` extends keys without one.
        assert_eq!(mok.key_level(ExtendedKeys::Off), 2);
        assert_eq!(InputModes::default().key_level(ExtendedKeys::Always), 1);
        let cs_h = KeyEvent::new(KeyCode::Char('H'), C | S);
        assert_eq!(
            encode_key(&cs_h, InputModes::default(), ExtendedKeys::On),
            b"\x08"
        );
        assert_eq!(encode_key(&cs_h, mok, ExtendedKeys::On), b"\x1b[104;6u");
    }

    #[test]
    fn kitty_disambiguates_ctrl_alt_and_escape() {
        let kitty = InputModes {
            kitty_flags: 1,
            ..InputModes::default()
        };
        let k = |code, m| encode_key(&KeyEvent::new(code, m), kitty, ExtendedKeys::On);
        assert_eq!(k(KeyCode::Char('h'), C), b"\x1b[104;5u");
        assert_eq!(k(KeyCode::Char('h'), A), b"\x1b[104;3u");
        assert_eq!(k(KeyCode::Esc, NONE), b"\x1b[27u");
        assert_eq!(k(KeyCode::Char('h'), NONE), b"h");
    }

    #[test]
    fn releases_are_ignored() {
        let mut ev = KeyEvent::new(KeyCode::Char('a'), NONE);
        ev.kind = KeyEventKind::Release;
        assert!(encode_key(&ev, InputModes::default(), ExtendedKeys::On).is_empty());
    }

    #[test]
    fn paste_is_bracketed_and_cannot_escape_the_bracket() {
        let plain = encode_paste("a\x1b[201~b", InputModes::default());
        assert_eq!(plain, b"a\x1b[201~b");
        let modes = InputModes {
            bracketed_paste: true,
            ..InputModes::default()
        };
        assert_eq!(encode_paste("a\x1b[201~b", modes), b"\x1b[200~ab\x1b[201~");
    }

    #[test]
    fn mouse_encodings() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let ev = |kind| MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: NONE,
        };
        let down = ev(MouseEventKind::Down(MouseButton::Left));
        let up = ev(MouseEventKind::Up(MouseButton::Left));
        let sgr = MouseProtocolEncoding::Sgr;
        let pr = MouseProtocolMode::PressRelease;
        assert_eq!(encode_mouse(&down, 4, 2, pr, sgr), b"\x1b[<0;5;3M");
        assert_eq!(encode_mouse(&up, 4, 2, pr, sgr), b"\x1b[<0;5;3m");
        assert_eq!(
            encode_mouse(&down, 4, 2, pr, MouseProtocolEncoding::Default),
            b"\x1b[M %#"
        );
        assert_eq!(
            encode_mouse(&up, 4, 2, pr, MouseProtocolEncoding::Default),
            b"\x1b[M#%#"
        );
        // Press-only mode drops releases; no mode drops everything.
        assert!(encode_mouse(&up, 4, 2, MouseProtocolMode::Press, sgr).is_empty());
        assert!(encode_mouse(&down, 4, 2, MouseProtocolMode::None, sgr).is_empty());
        // Drag needs button-motion mode.
        let drag = ev(MouseEventKind::Drag(MouseButton::Left));
        assert!(encode_mouse(&drag, 1, 1, pr, sgr).is_empty());
        assert_eq!(
            encode_mouse(&drag, 1, 1, MouseProtocolMode::ButtonMotion, sgr),
            b"\x1b[<32;2;2M"
        );
        let wheel = MouseEvent {
            modifiers: C,
            ..ev(MouseEventKind::ScrollUp)
        };
        assert_eq!(encode_mouse(&wheel, 0, 0, pr, sgr), b"\x1b[<80;1;1M");
        // Beyond column 222 the one-byte form cannot encode the position.
        assert!(encode_mouse(&down, 300, 0, pr, MouseProtocolEncoding::Default).is_empty());
    }
}
