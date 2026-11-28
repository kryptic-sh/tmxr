//! The fuzz targets' bodies (plan 20), one function per target taking the
//! fuzzer's bytes. The `fuzz_targets/` binaries only call them, so they build
//! on stable too, where `tests/replay.rs` runs every seed and regression
//! input through them.
//!
//! Each target checks an invariant where the code has one (a round trip, the
//! same result however the input is split across calls, a bound), not only
//! that nothing panics.

use std::path::PathBuf;

use tmxr_command::format::{Context, LoopItem, LoopKind, NameKind, expand};
use tmxr_command::tokenize::{join_args, tokenize, tokenize_lines};
use tmxr_command::{BindKey, Key};
use tmxr_proto::{ClientMsg, ServerMsg, read_msg};
use tmxr_server::layout;
use tmxr_term::dcs::DcsSplitter;
use tmxr_term::modes::{ModeSplitter, Piece};
use tmxr_term::{Emulator, ExtendedKeys, ExtendedKeysFormat, InputModes};
use ttyr::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use vt1k::{MouseProtocolEncoding, MouseProtocolMode};

/// A target's body: the fuzzer's bytes in.
pub type Target = fn(&[u8]);

/// Every target, by name, as `fuzz_targets/` and the replay test know them.
pub const TARGETS: &[(&str, Target)] = &[
    ("proto_frame", proto_frame),
    ("tmxr_env", tmxr_env),
    ("command_line", command_line),
    ("format_expand", format_expand),
    ("key_names", key_names),
    ("styles", styles),
    ("layout", layout),
    ("config_toml", config_toml),
    ("tmux_conf", tmux_conf),
    ("resurrect", resurrect),
    ("emulator", emulator),
    ("splitters", splitters),
    ("encode", encode),
    ("menu", menu),
];

/// The bytes as text, lossily: the text parsers take `&str`, and a fuzzer's
/// invalid UTF-8 still reaches them as replacement characters.
fn text(data: &[u8]) -> String {
    String::from_utf8_lossy(data).into_owned()
}

/// `data` cut into chunks whose lengths follow `pattern` in turn (empty
/// chunks included), for the targets that check that splitting input across
/// calls changes nothing.
fn chunks<'a>(data: &'a [u8], pattern: &[u8]) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    let mut rest = data;
    let mut i = 0;
    while !rest.is_empty() {
        let len = pattern
            .get(i % pattern.len().max(1))
            .map_or(rest.len(), |&b| usize::from(b % 32));
        let (head, tail) = rest.split_at(len.min(rest.len()));
        out.push(head);
        rest = tail;
        i += 1;
        if out.len() > 4 * data.len() + 8 {
            out.push(rest);
            break;
        }
    }
    out
}

/// No environment, for the tokenizer's `$NAME` and `~`.
fn no_env(_: &str) -> Option<String> {
    None
}

/// The frame codec, both directions: any bytes read as messages or an
/// error, and reading stops (a message or an error each time, never a
/// loop that consumes nothing). The bytes are also read as the body of one
/// well-formed frame, so the message decoder is reached without the
/// fuzzer first having to find a length that matches.
pub fn proto_frame(data: &[u8]) {
    let mut r = data;
    while let Ok(Some(_)) = read_msg::<_, ClientMsg>(&mut r) {}
    let mut r = data;
    while let Ok(Some(_)) = read_msg::<_, ServerMsg>(&mut r) {}
    let Ok(len) = u32::try_from(data.len()) else {
        return;
    };
    let mut frame = len.to_le_bytes().to_vec();
    frame.extend_from_slice(data);
    let _ = read_msg::<_, ClientMsg>(&mut frame.as_slice());
    let _ = read_msg::<_, ServerMsg>(&mut frame.as_slice());
}

/// `$TMXR`: anything parses or is refused; what parses names a path that
/// was in the value.
pub fn tmxr_env(data: &[u8]) {
    let value = text(data);
    if let Some((path, _, _)) = tmxr_proto::socket::parse_tmxr_env(&value) {
        assert!(value.starts_with(&*path.to_string_lossy()));
    }
}

/// The command language: any text tokenizes or fails cleanly, each command
/// parses or fails cleanly, and a command's words, joined back into a line,
/// tokenize to the same words.
pub fn command_line(data: &[u8]) {
    let line = text(data);
    let Ok(lines) = tokenize_lines(&line, &no_env) else {
        return;
    };
    for l in &lines {
        let _ = tmxr_command::table::parse_line(l);
        if l.blocks.iter().any(|&b| b) || l.words.is_empty() {
            continue;
        }
        let joined = join_args(&l.words);
        assert_eq!(
            tokenize(&joined, &no_env).ok(),
            Some(vec![l.words.clone()]),
            "{joined:?}"
        );
    }
}

/// A format context that knows a few names, enough to reach the lookups,
/// as the command crate's property tests use.
struct Vars;

impl Context for Vars {
    fn get(&self, name: &str) -> Option<String> {
        matches!(name, "a" | "b" | "session_name" | "pane_id").then(|| format!("<{name}>"))
    }

    fn name_exists(&self, _: NameKind, name: &str) -> Option<bool> {
        Some(name == "a")
    }

    fn pane_lines(&self) -> Option<Vec<String>> {
        Some(vec!["one".into(), "two".into()])
    }

    fn loop_items(&self, _: LoopKind) -> Option<Vec<LoopItem<'_>>> {
        Some(Vec::new())
    }
}

/// Format expansion: any format expands (a hang shows as libFuzzer's
/// timeout).
pub fn format_expand(data: &[u8]) {
    let fmt = text(data);
    let _ = expand(&fmt, &Vars);
    let _ = tmxr_command::format::expand_time(&fmt, &Vars);
}

/// Key names: what parses prints as a name that parses back to the same
/// key.
pub fn key_names(data: &[u8]) {
    let name = text(data);
    if let Ok(key) = name.parse::<Key>() {
        let printed = key.to_string();
        assert_eq!(
            printed.parse::<Key>().ok(),
            Some(key),
            "{name:?} printed {printed:?}"
        );
    }
    if let Ok(key) = name.parse::<BindKey>() {
        let printed = key.to_string();
        assert_eq!(
            printed.parse::<BindKey>().ok(),
            Some(key),
            "{name:?} printed {printed:?}"
        );
    }
}

/// Colours and `#[style]` runs: anything parses or is refused.
pub fn styles(data: &[u8]) {
    let s = text(data);
    let _ = tmxr_command::format::parse_colour(&s);
    let _ = tmxr_command::colour::parse_x11(&s);
    let _ = tmxr_command::format::styled(&s);
}

/// Layouts, string and JSON: what parses is dumped as JSON that parses
/// back to the same dump, and its panes lie inside the window.
pub fn layout(data: &[u8]) {
    let s = text(data);
    let Ok((tree, (w, h))) = layout::parse(&s) else {
        return;
    };
    let info = |_| layout::PaneInfo {
        active: false,
        last: None,
        index: 0,
    };
    for (_, r) in layout::pane_rects(&tree, w, h) {
        assert!(
            u32::from(r.x) + u32::from(r.w) <= u32::from(w)
                && u32::from(r.y) + u32::from(r.h) <= u32::from(h),
            "{s:?}: {r:?} outside {w}x{h}"
        );
    }
    let dumped = layout::dump(&tree, w, h, &info);
    let (again, size) = layout::parse(&dumped).unwrap_or_else(|e| panic!("{dumped:?}: {e}"));
    assert_eq!(size, (w, h), "{dumped:?}");
    assert_eq!(layout::dump(&again, w, h, &info), dumped);
}

/// A config file: anything loads or is refused.
pub fn config_toml(data: &[u8]) {
    // `load_file` reads a path; one file per fuzzing process.
    let path: PathBuf = std::env::temp_dir().join(format!("tmxr-fuzz-{}.toml", std::process::id()));
    if std::fs::write(&path, data).is_ok() {
        let _ = tmxr_config::load_file(&path);
    }
}

/// A tmux config imported: anything imports.
pub fn tmux_conf(data: &[u8]) {
    let _ = tmxr_config::from_tmux_conf(&text(data), &no_env);
}

/// A resurrect save file: anything reads as a save or is refused.
pub fn resurrect(data: &[u8]) {
    let _ = serde_json::from_slice::<tmxr_server::resurrect::Save>(data);
}

/// What one emulator made of some output, to compare.
#[derive(Debug, PartialEq)]
struct Seen {
    contents: String,
    cells: Vec<Option<vt1k::Cell>>,
    cursor: (u16, u16),
    alternate: bool,
    replies: Vec<u8>,
    queries: usize,
    bell: bool,
    clipboard: Vec<(String, String)>,
    passthrough: Vec<Vec<u8>>,
    rename: Option<Vec<u8>>,
    title: Option<String>,
    modes: InputModes,
}

fn feed_emulator(rows: u16, cols: u16, pieces: &[&[u8]]) -> Seen {
    let mut emu = Emulator::new(rows, cols, 16);
    let mut seen = Seen {
        contents: String::new(),
        cells: Vec::new(),
        cursor: (0, 0),
        alternate: false,
        replies: Vec::new(),
        queries: 0,
        bell: false,
        clipboard: Vec::new(),
        passthrough: Vec::new(),
        rename: None,
        title: None,
        modes: InputModes::default(),
    };
    for piece in pieces {
        seen.replies.extend(emu.process(piece));
        let (r, c) = emu.screen().cursor_position();
        let (sr, sc) = emu.screen().size();
        assert!(
            r < sr.get() && c <= sc.get(),
            "cursor {r},{c} outside {sr}x{sc}"
        );
        seen.queries += emu.take_queries().len();
        seen.bell |= emu.take_bell();
        seen.clipboard.extend(emu.take_clipboard());
        seen.passthrough
            .extend(emu.take_passthrough().into_iter().map(|(p, _)| p));
        if let Some(name) = emu.take_rename() {
            seen.rename = Some(name);
        }
    }
    let screen = emu.screen();
    let (sr, sc) = screen.size();
    seen.contents = screen.contents();
    seen.cells = (0..sr.get())
        .flat_map(|r| (0..sc.get()).map(move |c| (r, c)))
        .map(|(r, c)| screen.cell(r, c).cloned())
        .collect();
    seen.cursor = screen.cursor_position();
    seen.alternate = screen.alternate_screen();
    seen.title = emu.title().map(str::to_owned);
    seen.modes = emu.input_modes();
    seen
}

/// The pane emulator: the cursor stays on the screen, and the output fed
/// in one call or cut across many gives the same screen, modes, replies
/// and side effects (what the splitters carry between calls must lose and
/// add nothing).
pub fn emulator(data: &[u8]) {
    let [rows, cols, p0, p1, p2, p3, rest @ ..] = data else {
        return;
    };
    let rows = u16::from(rows % 40) + 1;
    let cols = u16::from(cols % 120) + 1;
    let whole = feed_emulator(rows, cols, &[rest]);
    let cut = feed_emulator(rows, cols, &chunks(rest, &[*p0, *p1, *p2, *p3]));
    assert_eq!(whole, cut);
}

/// Adjacent text pieces merged, so two splits compare by what they mean.
fn merged(pieces: Vec<Piece>) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for p in pieces {
        match (out.last_mut(), p) {
            (Some(Piece::Text(a)), Piece::Text(b)) => a.extend(b),
            (_, Piece::Text(b)) if b.is_empty() => {}
            (_, p) => out.push(p),
        }
    }
    out
}

/// The two splitters on their own: whole or cut, the same text, strings,
/// pieces and names come out.
pub fn splitters(data: &[u8]) {
    let [p0, p1, p2, rest @ ..] = data else {
        return;
    };
    let cut = chunks(rest, &[*p0, *p1, *p2]);

    let mut whole = DcsSplitter::default();
    let w = whole.split(rest);
    let mut parts = DcsSplitter::default();
    let (mut text, mut strings, mut rename) = (Vec::new(), Vec::new(), None);
    for c in &cut {
        let s = parts.split(c);
        strings.extend(s.strings.into_iter().map(|(at, d)| (at + text.len(), d)));
        text.extend(s.text);
        if s.rename.is_some() {
            rename = s.rename;
        }
    }
    assert_eq!(w.text, text);
    assert_eq!(w.strings, strings);
    assert_eq!(w.rename, rename);

    let mut whole = ModeSplitter::default();
    let w = merged(whole.split(rest));
    let mut parts = ModeSplitter::default();
    let p = merged(cut.iter().flat_map(|c| parts.split(c)).collect());
    assert_eq!(w, p);
}

/// Bytes read in turn as small numbers, for building events.
struct Draw<'a>(&'a [u8]);

impl Draw<'_> {
    fn byte(&mut self) -> u8 {
        let (&b, rest) = self.0.split_first().unwrap_or((&0, &[]));
        self.0 = rest;
        b
    }

    fn bool(&mut self) -> bool {
        self.byte() & 1 == 1
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes([self.byte(), self.byte()])
    }

    fn char(&mut self) -> char {
        let n = u32::from_le_bytes([self.byte(), self.byte(), self.byte() % 0x11, 0]);
        char::from_u32(n).unwrap_or('a')
    }

    fn key_code(&mut self) -> KeyCode {
        match self.byte() % 18 {
            0 => KeyCode::Backspace,
            1 => KeyCode::Enter,
            2 => KeyCode::Left,
            3 => KeyCode::Right,
            4 => KeyCode::Up,
            5 => KeyCode::Down,
            6 => KeyCode::Home,
            7 => KeyCode::End,
            8 => KeyCode::PageUp,
            9 => KeyCode::PageDown,
            10 => KeyCode::Tab,
            11 => KeyCode::BackTab,
            12 => KeyCode::Delete,
            13 => KeyCode::Insert,
            14 => KeyCode::F(self.byte()),
            15 => KeyCode::Null,
            16 => KeyCode::Esc,
            _ => KeyCode::Char(self.char()),
        }
    }

    fn button(&mut self) -> MouseButton {
        match self.byte() % 3 {
            0 => MouseButton::Left,
            1 => MouseButton::Middle,
            _ => MouseButton::Right,
        }
    }
}

/// Key, mouse and paste encoding over any pane modes: nothing panics, and a
/// bracketed paste holds no marker that would end it early, whatever the
/// pasted text (the rest would run as typed input).
pub fn encode(data: &[u8]) {
    let mut d = Draw(data);
    let modes = InputModes {
        app_cursor: d.bool(),
        bracketed_paste: d.bool(),
        kitty_flags: d.byte(),
        modify_other_keys: d.byte() % 3,
        asked_keys: d.byte() % 3,
        conpty: d.bool(),
    };
    let option =
        [ExtendedKeys::Off, ExtendedKeys::On, ExtendedKeys::Always][usize::from(d.byte() % 3)];
    let format = if d.bool() {
        ExtendedKeysFormat::CsiU
    } else {
        ExtendedKeysFormat::Xterm
    };

    let mut key = KeyEvent::new(d.key_code(), KeyModifiers::from_bits_truncate(d.byte()));
    key.kind = [
        KeyEventKind::Press,
        KeyEventKind::Repeat,
        KeyEventKind::Release,
    ][usize::from(d.byte() % 3)];
    if d.bool() {
        key.shifted = Some(d.char());
    }
    let _ = tmxr_term::encode_key(&key, modes, option, format);
    let _ = tmxr_term::conpty_key(&key, d.0);

    let kind = match d.byte() % 8 {
        0 => MouseEventKind::Down(d.button()),
        1 => MouseEventKind::Up(d.button()),
        2 => MouseEventKind::Drag(d.button()),
        3 => MouseEventKind::Moved,
        4 => MouseEventKind::ScrollDown,
        5 => MouseEventKind::ScrollUp,
        6 => MouseEventKind::ScrollLeft,
        _ => MouseEventKind::ScrollRight,
    };
    let ev = MouseEvent {
        kind,
        column: d.u16(),
        row: d.u16(),
        modifiers: KeyModifiers::from_bits_truncate(d.byte()),
    };
    let mode = [
        MouseProtocolMode::None,
        MouseProtocolMode::Press,
        MouseProtocolMode::PressRelease,
        MouseProtocolMode::ButtonMotion,
        MouseProtocolMode::AnyMotion,
    ][usize::from(d.byte() % 5)];
    let encoding = [
        MouseProtocolEncoding::Default,
        MouseProtocolEncoding::Utf8,
        MouseProtocolEncoding::Sgr,
    ][usize::from(d.byte() % 3)];
    let (col, row) = (d.u16(), d.u16());
    let _ = tmxr_term::encode::encode_mouse(&ev, col, row, mode, encoding);

    let pasted = text(d.0);
    let out = tmxr_term::encode_paste(&pasted, modes);
    if modes.bracketed_paste {
        let inner = out
            .strip_prefix(b"\x1b[200~")
            .and_then(|o| o.strip_suffix(b"\x1b[201~"))
            .expect("a bracketed paste starts and ends with its markers");
        assert!(
            !inner
                .windows(6)
                .any(|w| w == b"\x1b[201~" || w == b"\x1b[200~"),
            "{pasted:?} leaves a marker inside the paste"
        );
    }
}

/// `display-menu`'s arguments: any words build a menu or are refused.
pub fn menu(data: &[u8]) {
    let s = text(data);
    let args: Vec<String> = s.split('\0').map(str::to_owned).collect();
    let _ = tmxr_server::menu::Menu::parse("t".into(), &args, 0, &|s| s.to_owned());
}
