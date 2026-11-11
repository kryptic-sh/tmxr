//! Key and paste events → bytes for the program in a pane.
//!
//! Legacy encodings follow xterm's ctlseqs ("PC-Style Function Keys",
//! "Alt and Meta Keys"). A program that asked for xterm's modifyOtherKeys
//! (or that `extended-keys always` puts at level 1) gets the keys tmux's
//! input_key sends extended in the `extended-keys-format` option's form:
//! at level 1 the modified keys with no legacy form (`C-;`, `C-Enter`), at
//! level 2 every modified key. A program that pushed kitty keyboard flags
//! gets keys as the kitty keyboard protocol has them ([`kitty`]).

use tmxr_command::Typed;
use ttyr::event::{
    KeyCode, KeyEncoding, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use vt1k::{MouseProtocolEncoding, MouseProtocolMode};

mod kitty;

/// The kitty keyboard flag for disambiguated escape codes.
pub const KITTY_DISAMBIGUATE: u8 = 1;

/// The kitty keyboard flag for reporting repeats and releases beside
/// presses.
pub const KITTY_REPORT_EVENTS: u8 = 2;

/// The kitty keyboard flag for reporting a key's shifted and base-layout
/// keys beside it.
pub const KITTY_REPORT_ALTERNATE: u8 = 4;

/// The kitty keyboard flag for reporting every key as an escape code, text
/// keys, Enter, Tab, Backspace and the modifier keys among them.
pub const KITTY_REPORT_ALL_KEYS: u8 = 8;

/// The kitty keyboard flag for reporting the text a key produces inside
/// its escape code; defined only with [`KITTY_REPORT_ALL_KEYS`].
pub const KITTY_REPORT_TEXT: u8 = 16;

/// The kitty keyboard flags tmxr asks a client's terminal for on its own
/// behalf, whatever the focused program wants: disambiguated keys tell C-h
/// from Backspace and Escape from Alt, and alternate keys keep Shift on a
/// chord (M-A, not M-a).
pub const TMXR_KITTY_FLAGS: u8 = KITTY_DISAMBIGUATE | KITTY_REPORT_ALTERNATE;

/// The kitty keyboard flags [`encode_key`] acts on: every flag the
/// protocol has. A program asking which flags are on is told only these,
/// and a client's terminal is asked for no more of a program's flags than
/// these.
pub const KITTY_FLAGS_ENCODED: u8 = KITTY_DISAMBIGUATE
    | KITTY_REPORT_EVENTS
    | KITTY_REPORT_ALTERNATE
    | KITTY_REPORT_ALL_KEYS
    | KITTY_REPORT_TEXT;

/// The pane state key encoding depends on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputModes {
    /// DECCKM: cursor keys send `SS3` instead of `CSI`.
    pub app_cursor: bool,
    /// Wrap pastes in `CSI 200~` / `CSI 201~`.
    pub bracketed_paste: bool,
    /// Current kitty keyboard flags, as the program pushed them, those
    /// [`encode_key`] does not act on included.
    pub kitty_flags: u8,
    /// xterm modifyOtherKeys level, as tmux holds it: what the program
    /// asked for, or 1 where `extended-keys always` gave it one.
    pub modify_other_keys: u8,
    /// The modifyOtherKeys level the program asked for itself (`CSI > 4 ;
    /// 1 m` or `2 m`), without the level `always` gives.
    pub asked_keys: u8,
    /// Whether the pane's pseudo-terminal is a ConPTY ([`crate::pty::CONPTY`]).
    pub conpty: bool,
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

/// The `extended-keys-format` option, as tmux's: the form a key goes out in
/// when it goes extended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExtendedKeysFormat {
    /// `CSI <codepoint> ; <modifiers> u`.
    CsiU,
    /// xterm's `CSI 27 ; <modifiers> ; <codepoint> ~`, tmux's default.
    #[default]
    Xterm,
}

impl ExtendedKeysFormat {
    /// The option's value; anything but `csi-u` is `xterm`, as tmux's
    /// default.
    pub fn from_option(value: &str) -> Self {
        match value {
            "csi-u" => Self::CsiU,
            _ => Self::Xterm,
        }
    }
}

impl InputModes {
    /// The modifyOtherKeys level keys go out at: the pane's (which a
    /// request made under `extended-keys off` never set), at least 1 with
    /// `always`. A ConPTY pane is at the level its program asked for and
    /// no other, `always` acting as `on`: conhost's input parser does not
    /// read xterm's extended form, and takes the legacy bytes (0x08 for
    /// C-BSpace among them) for the keys they are. Both sides are
    /// unit-tested on every platform; that ConPTY reads the legacy bytes is
    /// from commit 9faac49, not checked here, where only Linux runs.
    pub fn key_level(self, option: ExtendedKeys) -> u8 {
        if self.conpty {
            return self.asked_keys;
        }
        match option {
            ExtendedKeys::Always => self.modify_other_keys.max(1),
            ExtendedKeys::Off | ExtendedKeys::On => self.modify_other_keys,
        }
    }
}

/// xterm modifier parameter: 1 + (shift | alt << 1 | ctrl << 2).
fn modifier_param(m: KeyModifiers) -> u8 {
    1 + u8::from(m.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(m.contains(KeyModifiers::ALT))
        + 4 * u8::from(m.contains(KeyModifiers::CONTROL))
}

/// A modified key in the extended form `format` names, as tmux's
/// input_key_extended; an unmodified one has none.
fn extended(codepoint: u32, m: KeyModifiers, format: ExtendedKeysFormat) -> Vec<u8> {
    match (modifier_param(m), format) {
        (1, _) => Vec::new(),
        (p, ExtendedKeysFormat::Xterm) => format!("\x1b[27;{p};{codepoint}~").into_bytes(),
        (p, ExtendedKeysFormat::CsiU) => format!("\x1b[{codepoint};{p}u").into_bytes(),
    }
}

/// The key tmux's input_key would have for a key with no entry in its
/// table of special keys (a character, Enter, Tab, Escape or a modified
/// Backspace), read as tmux reads a key ([`Typed`], with the legacy
/// control bytes as [`legacy_control`] names them): its codepoint and
/// modifiers. A control letter keeps its case, as tmux keeps C-a (a
/// legacy terminal's 0x01, which ttyr reads as the lower-case letter) apart
/// from C-A (`send-keys C-A`, 65); without Ctrl an upper-case letter, or a
/// printable character with Shift alone, drops Shift, as tmux's
/// tty_keys_extended_key.
fn tmux_key(code: KeyCode, mut m: KeyModifiers) -> Option<(u32, KeyModifiers)> {
    let c = match code {
        KeyCode::Char(c) if m.contains(KeyModifiers::CONTROL) => c,
        KeyCode::Char(c) => {
            let printable = c > ' ' && c != '\x7f';
            if c.is_uppercase() || (printable && m == KeyModifiers::SHIFT) {
                m.remove(KeyModifiers::SHIFT);
            }
            c
        }
        KeyCode::Enter => '\r',
        KeyCode::Tab => '\t',
        KeyCode::Esc => '\x1b',
        KeyCode::Backspace => '\x7f',
        _ => return None,
    };
    Some((u32::from(c), m))
}

/// The key a legacy terminal's control byte 0x1c to 0x1f is to tmux, C-\
/// to C-_, which ttyr's decoder reads as Ctrl+4 to Ctrl+7, the keys that
/// send those bytes too; `None` for any other key, a Ctrl+4 the kitty
/// protocol or xterm's extended keys report among them.
fn legacy_control(key: &KeyEvent) -> Option<char> {
    if key.encoding != KeyEncoding::Legacy || !key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }
    match key.key {
        KeyCode::Char('4') => Some('\\'),
        KeyCode::Char('5') => Some(']'),
        KeyCode::Char('6') => Some('^'),
        KeyCode::Char('7') => Some('_'),
        _ => None,
    }
}

/// Whether `key` is a modifier or lock key itself, which only a terminal
/// under kitty flag 8 reports, and only a program under it is sent: a key
/// held with others, never one typed on its own.
pub fn is_modifier_key(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::Modifier(_) | KeyCode::CapsLock | KeyCode::NumLock | KeyCode::ScrollLock
    )
}

/// Whether tmux's modifyOtherKeys level 1 sends a key as at level 0, as
/// its input_key_mode1: a key with Meta and not Ctrl, and Ctrl with a key
/// xterm's table gives a control byte; the rest go extended.
fn level_one_is_legacy(codepoint: u32, m: KeyModifiers) -> bool {
    if m & (KeyModifiers::CONTROL | KeyModifiers::ALT) == KeyModifiers::ALT {
        return true;
    }
    m.contains(KeyModifiers::CONTROL)
        && char::from_u32(codepoint).is_some_and(|c| matches!(c, ' ' | '/' | '2'..='8' | '@'..='~'))
}

/// What a legacy pane is sent for `C-<c>`, transcribed from tmux 3.6's
/// input_key_vt10x: a few keys remapped by its `standard_map` (some to
/// printable characters, as `C-!` to `1`), `3` to `7` to 0x1b..0x1f, `@` to
/// `~` to their C0 byte (the 0x40 and case bits gone), and a non-ASCII
/// character as itself, Ctrl lost, as tmux has no way to report it; `None`
/// for any other key, which tmux sends nothing for.
fn ctrl_bytes(c: char) -> Option<Vec<u8>> {
    const FROM: &str = "1!9(0)=+;:'\",<.>/-8? 2";
    const TO: &[u8] = b"119900=+;;'',,..\x1f\x1f\x7f\x7f\0\0";
    if !c.is_ascii() {
        let mut buf = [0u8; 4];
        return Some(c.encode_utf8(&mut buf).as_bytes().to_vec());
    }
    let byte = if let Some(i) = FROM.find(c) {
        TO[i]
    } else if ('3'..='7').contains(&c) {
        c as u8 - 0x18
    } else if ('@'..='~').contains(&c) {
        c as u8 & 0x1f
    } else {
        return None;
    };
    Some(vec![byte])
}

/// Encode a key for a pane under the `extended-keys` and
/// `extended-keys-format` options. A program that pushed kitty flags gets
/// the kitty protocol's encoding ([`kitty::encode`]); every other gets a
/// press or repeat as a legacy terminal or tmux sends it, and no release.
/// Keys with no encoding produce nothing.
pub fn encode_key(
    key: &KeyEvent,
    modes: InputModes,
    option: ExtendedKeys,
    format: ExtendedKeysFormat,
) -> Vec<u8> {
    let flags = modes.kitty_flags & KITTY_FLAGS_ENCODED;
    // Flags 4 and 16 alone change no key's form: alternate keys and text
    // ride only on keys another flag makes escape codes.
    if flags & (KITTY_DISAMBIGUATE | KITTY_REPORT_EVENTS | KITTY_REPORT_ALL_KEYS) != 0 {
        return kitty::encode(key, flags);
    }
    if key.kind == KeyEventKind::Release {
        return Vec::new();
    }
    legacy_key(key, modes, modes.key_level(option), format)
}

/// A key as a legacy terminal sends it, or extended at modifyOtherKeys
/// `level` in `format`, as tmux's input_key.
fn legacy_key(key: &KeyEvent, modes: InputModes, level: u8, format: ExtendedKeysFormat) -> Vec<u8> {
    const MODS: KeyModifiers = KeyModifiers::SHIFT
        .union(KeyModifiers::ALT)
        .union(KeyModifiers::CONTROL);
    let typed = Typed::of(key);
    let code = legacy_control(key).map_or(typed.key, KeyCode::Char);
    let m = typed.modifiers & MODS;
    let alt = m.contains(KeyModifiers::ALT);
    let ctrl = m.contains(KeyModifiers::CONTROL);

    // A key tmux's table has no entry for goes extended at modifyOtherKeys
    // level 2 if it is modified, and at level 1 if xterm's legacy form
    // cannot carry it, as tmux's input_key; level 2 sends BTab as S-Tab.
    if level > 0 {
        let (code, mods) = match code {
            KeyCode::BackTab if level >= 2 => (KeyCode::Tab, m | KeyModifiers::SHIFT),
            code => (code, m),
        };
        if let Some((codepoint, mods)) = tmux_key(code, mods)
            && !mods.is_empty()
            && (level >= 2 || !level_one_is_legacy(codepoint, mods))
        {
            return extended(codepoint, mods, format);
        }
    }

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

    match code {
        KeyCode::Char(c) if ctrl => ctrl_bytes(c).map_or_else(Vec::new, with_alt),
        KeyCode::Char(c) => {
            let mut buf = [0u8; 4];
            with_alt(c.encode_utf8(&mut buf).as_bytes().to_vec())
        }
        // The keypad's Enter is a newline, as tmux's table has it for a
        // pane not in application keypad mode (which tmxr does not track).
        KeyCode::Enter if key.state.contains(KeyEventState::KEYPAD) && m.is_empty() => {
            b"\n".to_vec()
        }
        KeyCode::Enter => with_alt(b"\r".to_vec()),
        // tmux's S-Tab (`send-keys S-Tab`) is Tab here, Shift lost, as
        // its table has it; Shift+Tab from a terminal is BTab.
        KeyCode::Tab => with_alt(b"\t".to_vec()),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace if ctrl => with_alt(vec![0x08]),
        KeyCode::Backspace => with_alt(vec![0x7f]),
        KeyCode::Esc => with_alt(vec![0x1b]),
        KeyCode::Up => cursor('A'),
        KeyCode::Down => cursor('B'),
        KeyCode::Right => cursor('C'),
        KeyCode::Left => cursor('D'),
        // As tmux's table, the `tmux-256color` terminfo's khome and kend
        // whatever the cursor key mode; only a modified one is `CSI 1;m H`.
        KeyCode::Home if modifier_param(m) == 1 => tilde(1),
        KeyCode::End if modifier_param(m) == 1 => tilde(4),
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
    let KeyCode::Char(c) = key.key else {
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
    // xterm's buttons 8 and 9 (back and forward) are not encoded: the
    // client's reader does not report them yet.
    let button = |b: MouseButton| match b {
        MouseButton::Left => Some(0u32),
        MouseButton::Middle => Some(1),
        MouseButton::Right => Some(2),
        _ => None,
    };
    let (code, release, reported) = match ev.kind {
        MouseEventKind::Down(b) => (button(b), false, mode != MouseProtocolMode::None),
        MouseEventKind::Up(b) => (
            button(b),
            true,
            !matches!(mode, MouseProtocolMode::None | MouseProtocolMode::Press),
        ),
        MouseEventKind::Drag(b) => (
            button(b).map(|b| b + 32),
            false,
            matches!(
                mode,
                MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion
            ),
        ),
        MouseEventKind::Moved => (Some(3 + 32), false, mode == MouseProtocolMode::AnyMotion),
        MouseEventKind::ScrollUp => (Some(64), false, mode != MouseProtocolMode::None),
        MouseEventKind::ScrollDown => (Some(65), false, mode != MouseProtocolMode::None),
        MouseEventKind::ScrollLeft => (Some(66), false, mode != MouseProtocolMode::None),
        MouseEventKind::ScrollRight => (Some(67), false, mode != MouseProtocolMode::None),
        _ => (None, false, false),
    };
    let Some(code) = code.filter(|_| reported) else {
        return Vec::new();
    };
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
                    // A position past the one-byte form's last byte is
                    // clamped to it, as tmux's input_key_get_mouse does; no
                    // button code reaches it.
                    _ => out.push(u8::try_from(v).unwrap_or(u8::MAX)),
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
            let bytes = encode_key(&key, InputModes::default(), ExtendedKeys::On, XTERM);
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
            conpty_key(&key, &encode_key(&key, kitty, ExtendedKeys::On, XTERM)),
            None
        );
    }

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const C: KeyModifiers = KeyModifiers::CONTROL;
    const A: KeyModifiers = KeyModifiers::ALT;
    const S: KeyModifiers = KeyModifiers::SHIFT;
    const XTERM: ExtendedKeysFormat = ExtendedKeysFormat::Xterm;

    /// Character `c` typed with `m`, as a terminal reports it: an
    /// upper-case letter as its key with Shift, the letter its shifted key.
    fn typed(c: char, m: KeyModifiers) -> KeyEvent {
        let shift = match tmxr_command::keys::single_lower(c) {
            Some(_) => S,
            None => KeyModifiers::NONE,
        };
        tmxr_command::Key::new(KeyCode::Char(c), m | shift).to_event()
    }

    fn enc(code: KeyCode, m: KeyModifiers) -> Vec<u8> {
        encode_key(
            &KeyEvent::new(code, m),
            InputModes::default(),
            ExtendedKeys::On,
            XTERM,
        )
    }

    #[test]
    fn legacy_table() {
        let cases: &[(KeyCode, KeyModifiers, &[u8])] = &[
            (KeyCode::Char('a'), NONE, b"a"),
            (KeyCode::Char('a'), S, b"A"),
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
            (KeyCode::Home, NONE, b"\x1b[1~"),
            (KeyCode::End, NONE, b"\x1b[4~"),
            (KeyCode::Home, C, b"\x1b[1;5H"),
            (KeyCode::End, S, b"\x1b[1;2F"),
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
        assert_eq!(encode_key(&up, modes, ExtendedKeys::On, XTERM), b"\x1bOA");
        let c_up = KeyEvent::new(KeyCode::Up, C);
        assert_eq!(
            encode_key(&c_up, modes, ExtendedKeys::On, XTERM),
            b"\x1b[1;5A"
        );
        let home = KeyEvent::new(KeyCode::Home, NONE);
        assert_eq!(
            encode_key(&home, modes, ExtendedKeys::On, XTERM),
            b"\x1b[1~",
            "not SS3"
        );
    }

    #[test]
    fn keys_without_legacy_form_need_extended_mode() {
        // tmux's input_key_vt10x has no byte for C-#, and maps C-; to `;`
        // (tmux 3.6 sent `;`, 2026-11-10).
        let c_hash = KeyEvent::new(KeyCode::Char('#'), C);
        assert!(encode_key(&c_hash, InputModes::default(), ExtendedKeys::On, XTERM).is_empty());
        assert_eq!(
            encode_key(&c_hash, InputModes::default(), ExtendedKeys::Always, XTERM),
            b"\x1b[27;5;35~"
        );
        let c_semi = KeyEvent::new(KeyCode::Char(';'), C);
        assert_eq!(
            encode_key(&c_semi, InputModes::default(), ExtendedKeys::On, XTERM),
            b";"
        );
        assert_eq!(
            encode_key(&c_semi, InputModes::default(), ExtendedKeys::Always, XTERM),
            b"\x1b[27;5;59~"
        );
        let level = |n| InputModes {
            modify_other_keys: n,
            ..InputModes::default()
        };
        // A level the pane holds stands whatever the option is now, as
        // tmux's; `always` extends keys without one.
        assert_eq!(level(2).key_level(ExtendedKeys::Off), 2);
        assert_eq!(InputModes::default().key_level(ExtendedKeys::Always), 1);
    }

    /// tmux 3.6's bytes for a key at modifyOtherKeys level 1 and 2, under
    /// each `extended-keys-format` (input_key, input_key_mode1 and
    /// input_key_extended).
    #[test]
    fn extended_keys_are_tmuxs_at_each_level_and_format() {
        const CSI_U: ExtendedKeysFormat = ExtendedKeysFormat::CsiU;
        let level = |n| InputModes {
            modify_other_keys: n,
            ..InputModes::default()
        };
        // A key's bytes at level 1, at level 2, and at level 2 under csi-u.
        let check_key = |key: KeyEvent, one: &str, two: &str, two_u: &str| {
            let enc = |n, format| encode_key(&key, level(n), ExtendedKeys::On, format);
            assert_eq!(enc(1, XTERM), one.as_bytes(), "{key:?} level 1");
            assert_eq!(enc(2, XTERM), two.as_bytes(), "{key:?} level 2");
            assert_eq!(enc(2, CSI_U), two_u.as_bytes(), "{key:?} csi-u");
        };
        let check = |code, m, one: &str, two: &str, two_u: &str| {
            check_key(KeyEvent::new(code, m), one, two, two_u);
        };
        check(
            KeyCode::Char(';'),
            C,
            "\x1b[27;5;59~",
            "\x1b[27;5;59~",
            "\x1b[59;5u",
        );
        check(
            KeyCode::Enter,
            C,
            "\x1b[27;5;13~",
            "\x1b[27;5;13~",
            "\x1b[13;5u",
        );
        check(
            KeyCode::Enter,
            S,
            "\x1b[27;2;13~",
            "\x1b[27;2;13~",
            "\x1b[13;2u",
        );
        check(KeyCode::Enter, A, "\x1b\r", "\x1b[27;3;13~", "\x1b[13;3u");
        check(KeyCode::Tab, C, "\x1b[27;5;9~", "\x1b[27;5;9~", "\x1b[9;5u");
        check(KeyCode::BackTab, S, "\x1b[Z", "\x1b[27;2;9~", "\x1b[9;2u");
        check(
            KeyCode::Esc,
            C,
            "\x1b[27;5;27~",
            "\x1b[27;5;27~",
            "\x1b[27;5u",
        );
        check(KeyCode::Esc, A, "\x1b\x1b", "\x1b[27;3;27~", "\x1b[27;3u");
        check(
            KeyCode::Backspace,
            C,
            "\x1b[27;5;127~",
            "\x1b[27;5;127~",
            "\x1b[127;5u",
        );
        check(
            KeyCode::Backspace,
            A,
            "\x1b\x7f",
            "\x1b[27;3;127~",
            "\x1b[127;3u",
        );
        check(KeyCode::Char('a'), C, "\x01", "\x1b[27;5;97~", "\x1b[97;5u");
        check(
            KeyCode::Char('a'),
            A,
            "\x1ba",
            "\x1b[27;3;97~",
            "\x1b[97;3u",
        );
        check(
            KeyCode::Char('x'),
            C | A,
            "\x1b\x18",
            "\x1b[27;7;120~",
            "\x1b[120;7u",
        );
        // The shift an upper-case letter carries is no modifier, nor is
        // Shift alone on a printable character; on Space it is.
        check_key(typed('A', NONE), "A", "A", "A");
        // Shift+a from a kitty terminal that reports no shifted key.
        check(KeyCode::Char('a'), S, "A", "A", "A");
        check(KeyCode::Char('%'), S, "%", "%", "%");
        check(
            KeyCode::Char(' '),
            S,
            "\x1b[27;2;32~",
            "\x1b[27;2;32~",
            "\x1b[32;2u",
        );
        check_key(typed('A', A), "\x1bA", "\x1b[27;3;65~", "\x1b[65;3u");
        check(
            KeyCode::Char('h'),
            C | S,
            "\x08",
            "\x1b[27;6;104~",
            "\x1b[104;6u",
        );
        check(KeyCode::Char(' '), C, "\0", "\x1b[27;5;32~", "\x1b[32;5u");
        // C-2 is C-2, NUL's byte in the legacy form only.
        check(KeyCode::Char('2'), C, "\0", "\x1b[27;5;50~", "\x1b[50;5u");
        // A legacy terminal's C-\\ (0x1c) arrives as C-4, and is C-\\ to
        // tmux; a real Ctrl+4, which only the kitty protocol or xterm's
        // extended keys tell apart, is C-4.
        let legacy_c4 = KeyEvent::new(KeyCode::Char('4'), C).with_encoding(KeyEncoding::Legacy);
        check_key(legacy_c4, "\x1c", "\x1b[27;5;92~", "\x1b[92;5u");
        check(KeyCode::Char('4'), C, "\x1c", "\x1b[27;5;52~", "\x1b[52;5u");
        let xterm_c7 = KeyEvent::new(KeyCode::Char('7'), C).with_encoding(KeyEncoding::Xterm);
        check_key(xterm_c7, "\x1f", "\x1b[27;5;55~", "\x1b[55;5u");
        check(
            KeyCode::Char('{'),
            C,
            "\x1b",
            "\x1b[27;5;123~",
            "\x1b[123;5u",
        );
        check(
            KeyCode::Char('1'),
            C,
            "\x1b[27;5;49~",
            "\x1b[27;5;49~",
            "\x1b[49;5u",
        );
        check(
            KeyCode::Char('?'),
            C,
            "\x1b[27;5;63~",
            "\x1b[27;5;63~",
            "\x1b[63;5u",
        );
        check(
            KeyCode::Char('é'),
            C,
            "\x1b[27;5;233~",
            "\x1b[27;5;233~",
            "\x1b[233;5u",
        );
        // Unmodified keys and tmux's table of special keys are as ever.
        check(KeyCode::Char('a'), NONE, "a", "a", "a");
        check(KeyCode::Enter, NONE, "\r", "\r", "\r");
        check(KeyCode::Backspace, NONE, "\x7f", "\x7f", "\x7f");
        check(KeyCode::Up, C, "\x1b[1;5A", "\x1b[1;5A", "\x1b[1;5A");
        check(KeyCode::F(5), A, "\x1b[15;3~", "\x1b[15;3~", "\x1b[15;3~");
        // csi-u at level 1 is the same choice of keys in the other form.
        let c_enter = KeyEvent::new(KeyCode::Enter, C);
        assert_eq!(
            encode_key(&c_enter, level(1), ExtendedKeys::On, CSI_U),
            b"\x1b[13;5u"
        );
        // `always` puts a pane that asked for nothing at level 1, not 2.
        let c_a = KeyEvent::new(KeyCode::Char('a'), C);
        assert_eq!(
            encode_key(&c_a, InputModes::default(), ExtendedKeys::Always, XTERM),
            b"\x01"
        );
    }

    /// A ConPTY pane is at the level its program asked for, `always` or
    /// not; another pane gets level 1 from `always`.
    #[test]
    fn a_conpty_pane_takes_only_the_level_its_program_asked_for() {
        let modes = |held, asked, conpty| InputModes {
            modify_other_keys: held,
            asked_keys: asked,
            conpty,
            ..InputModes::default()
        };
        assert_eq!(modes(1, 0, false).key_level(ExtendedKeys::Always), 1);
        assert_eq!(modes(0, 0, false).key_level(ExtendedKeys::Always), 1);
        assert_eq!(modes(1, 0, true).key_level(ExtendedKeys::Always), 0);
        assert_eq!(modes(2, 2, true).key_level(ExtendedKeys::Always), 2);
        assert_eq!(modes(1, 1, true).key_level(ExtendedKeys::On), 1);
        // So a shell under ConPTY gets C-BSpace as the 0x08 it reads.
        let c_bs = KeyEvent::new(KeyCode::Backspace, C);
        let enc = |m| encode_key(&c_bs, m, ExtendedKeys::Always, XTERM);
        assert_eq!(enc(modes(1, 0, true)), b"\x08");
        assert_eq!(enc(modes(1, 0, false)), b"\x1b[27;5;127~");
        assert_eq!(enc(modes(1, 1, true)), b"\x1b[27;5;127~");
    }

    #[test]
    fn kitty_disambiguates_ctrl_alt_and_escape() {
        let kitty = InputModes {
            kitty_flags: 1,
            ..InputModes::default()
        };
        let k = |code, m| encode_key(&KeyEvent::new(code, m), kitty, ExtendedKeys::On, XTERM);
        assert_eq!(k(KeyCode::Char('h'), C), b"\x1b[104;5u");
        assert_eq!(k(KeyCode::Char('h'), A), b"\x1b[104;3u");
        assert_eq!(k(KeyCode::Esc, NONE), b"\x1b[27u");
        assert_eq!(k(KeyCode::Char('h'), NONE), b"h");
        assert_eq!(k(KeyCode::Home, NONE), b"\x1b[H", "kitty's, not tmux's");
    }

    #[test]
    fn kitty_reports_an_uppercase_letter_as_its_key_with_shift() {
        let kitty = InputModes {
            kitty_flags: 1,
            ..InputModes::default()
        };
        let k = |key: KeyEvent| encode_key(&key, kitty, ExtendedKeys::On, XTERM);
        let legacy =
            |key: KeyEvent| encode_key(&key, InputModes::default(), ExtendedKeys::On, XTERM);
        // M-A, as a terminal reports it (or send-keys M-A), and as a kitty
        // terminal without alternate keys does, with no shifted key.
        assert_eq!(k(typed('A', A)), b"\x1b[97;4u");
        assert_eq!(k(KeyEvent::new(KeyCode::Char('a'), A | S)), b"\x1b[97;4u");
        let ctrl_shift_h = KeyEvent::new(KeyCode::Char('h'), C | S).with_shifted(Some('H'));
        assert_eq!(k(ctrl_shift_h.clone()), b"\x1b[104;6u");
        // A legacy pane gets the character after an ESC, and C-S-h as C-h.
        assert_eq!(legacy(typed('A', A)), b"\x1bA");
        assert_eq!(legacy(ctrl_shift_h), b"\x08");
        // Beyond ASCII, a letter with one lowercase form is that key.
        assert_eq!(k(typed('É', A)), b"\x1b[233;4u");
        assert_eq!(k(typed('É', C)), b"\x1b[233;6u");
        assert_eq!(k(typed('С', A)), b"\x1b[1089;4u");
        assert_eq!(k(typed('с', A)), b"\x1b[1089;3u");
        // İ lowercases to two characters, so it is sent as itself.
        assert_eq!(k(typed('İ', A)), b"\x1b[304;3u");
        // Legacy and modifyOtherKeys panes get the uppercase letter, as a
        // legacy terminal sends it.
        assert_eq!(legacy(typed('É', A)), "\x1bÉ".as_bytes());
        let mok2 = InputModes {
            modify_other_keys: 2,
            ..InputModes::default()
        };
        assert_eq!(
            encode_key(&typed('É', A), mok2, ExtendedKeys::On, XTERM),
            b"\x1b[27;3;201~"
        );
    }

    /// tmux's C-A (`send-keys C-A`: the upper-case letter's key with Ctrl
    /// alone) is 0x01 to a legacy pane as C-a is, and 65 under
    /// modifyOtherKeys, where C-a is 97 and C-S-a is 97 with Shift, as
    /// tmux 3.6 sends them.
    #[test]
    fn ctrl_with_an_upper_case_letter_keeps_its_case_extended() {
        let key = |name: &str| name.parse::<tmxr_command::Key>().unwrap().to_event();
        let mok2 = InputModes {
            modify_other_keys: 2,
            ..InputModes::default()
        };
        for (name, legacy, extended) in [
            ("C-A", &b"\x01"[..], &b"\x1b[27;5;65~"[..]),
            ("C-a", b"\x01", b"\x1b[27;5;97~"),
            ("C-S-a", b"\x01", b"\x1b[27;6;97~"),
        ] {
            let ev = key(name);
            let at = |modes| encode_key(&ev, modes, ExtendedKeys::On, XTERM);
            assert_eq!(at(InputModes::default()), legacy, "{name}");
            assert_eq!(at(mok2), extended, "{name}");
        }
        // A kitty pane has no key for it but Ctrl with `a`.
        let kitty = InputModes {
            kitty_flags: KITTY_DISAMBIGUATE,
            ..InputModes::default()
        };
        let ev = key("C-A");
        assert_eq!(
            encode_key(&ev, kitty, ExtendedKeys::On, XTERM),
            b"\x1b[97;5u"
        );
    }

    /// What tmux 3.6's send-keys sent a `cat -v` pane for these names, at
    /// level 0 and at modifyOtherKeys level 2 (scratchpad build,
    /// 2026-11-10): its input_key_vt10x Ctrl map (C-! is 1, C-é is é),
    /// KPEnter as a newline, S-Tab as Tab, and Shift kept with Ctrl on a
    /// digit.
    #[test]
    fn send_keys_names_reach_a_pane_as_tmuxs() {
        let key = |name: &str| name.parse::<tmxr_command::Key>().unwrap().to_event();
        let mok2 = InputModes {
            modify_other_keys: 2,
            ..InputModes::default()
        };
        for (name, legacy, extended) in [
            ("C-S-1", "1", "\x1b[27;6;49~"),
            ("C-1", "1", "\x1b[27;5;49~"),
            ("C-!", "1", "\x1b[27;5;33~"),
            ("C-é", "é", "\x1b[27;5;233~"),
            ("KPEnter", "\n", "\n"),
            ("KP1", "1", "1"),
            ("KP/", "/", "/"),
            ("S-Tab", "\t", "\x1b[27;2;9~"),
            ("C-S-Tab", "\t", "\x1b[27;6;9~"),
            ("BTab", "\x1b[Z", "\x1b[27;2;9~"),
        ] {
            let ev = key(name);
            let at = |modes| String::from_utf8(encode_key(&ev, modes, ExtendedKeys::On, XTERM));
            assert_eq!(at(InputModes::default()).unwrap(), legacy, "{name}");
            assert_eq!(at(mok2).unwrap(), extended, "{name}");
        }
        // The rest of tmux's standard_map, and 3 to 7.
        for (c, byte) in [
            ('9', b'9'),
            ('(', b'9'),
            ('0', b'0'),
            (')', b'0'),
            ('=', b'='),
            ('+', b'+'),
            (';', b';'),
            (':', b';'),
            ('\'', b'\''),
            ('"', b'\''),
            (',', b','),
            ('<', b','),
            ('.', b'.'),
            ('>', b'.'),
            ('/', 0x1f),
            ('-', 0x1f),
            ('8', 0x7f),
            ('?', 0x7f),
            (' ', 0),
            ('2', 0),
            ('3', 0x1b),
            ('7', 0x1f),
            ('@', 0),
            ('a', 1),
            ('~', 0x1e),
        ] {
            assert_eq!(ctrl_bytes(c), Some(vec![byte]), "{c}");
        }
        assert_eq!(ctrl_bytes('#'), None);
    }

    #[test]
    fn kitty_sends_a_shifted_symbol_as_its_key_with_shift() {
        // As the kitty protocol reports Alt+Shift+1 under alternate keys
        // (`CSI 49:33;4u`): the unshifted key, Shift, and the shifted `!`.
        let kitty = InputModes {
            kitty_flags: 1,
            ..InputModes::default()
        };
        let shifted =
            |key, shifted, m| KeyEvent::new(KeyCode::Char(key), m).with_shifted(Some(shifted));
        let k = |key: &KeyEvent| encode_key(key, kitty, ExtendedKeys::On, XTERM);
        assert_eq!(k(&shifted('1', '!', A | S)), b"\x1b[49;4u");
        assert_eq!(k(&shifted('1', '!', C | S)), b"\x1b[49;6u");
        assert_eq!(k(&shifted(';', ':', A | S)), b"\x1b[59;4u");
        // A legacy terminal's `ESC !` names no key but the character.
        assert_eq!(k(&typed('!', A)), b"\x1b[33;3u");
        // Every other pane gets the character typed, as tmux names it (M-!).
        let legacy = InputModes::default();
        let mok2 = InputModes {
            modify_other_keys: 2,
            ..InputModes::default()
        };
        let alt_bang = shifted('1', '!', A | S);
        assert_eq!(
            encode_key(&alt_bang, legacy, ExtendedKeys::On, XTERM),
            b"\x1b!"
        );
        assert_eq!(
            encode_key(&alt_bang, mok2, ExtendedKeys::On, XTERM),
            b"\x1b[27;3;33~"
        );
        assert_eq!(
            encode_key(&typed('!', A), mok2, ExtendedKeys::On, XTERM),
            b"\x1b[27;3;33~"
        );
    }

    #[test]
    fn releases_are_ignored() {
        let mut ev = KeyEvent::new(KeyCode::Char('a'), NONE);
        ev.kind = KeyEventKind::Release;
        assert!(encode_key(&ev, InputModes::default(), ExtendedKeys::On, XTERM).is_empty());
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
        // Past column and row 222 the one-byte form clamps the position to
        // 255, as tmux's input_key_get_mouse (MOUSE_PARAM_MAX).
        let one_byte = |col, row| encode_mouse(&down, col, row, pr, MouseProtocolEncoding::Default);
        assert_eq!(one_byte(222, 0), b"\x1b[M \xff!");
        assert_eq!(one_byte(223, 0), b"\x1b[M \xff!");
        assert_eq!(one_byte(300, 400), b"\x1b[M \xff\xff");
        assert_eq!(one_byte(221, 223), b"\x1b[M \xfe\xff");
    }
}
