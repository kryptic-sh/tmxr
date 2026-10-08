//! A pane's terminal emulator: `vt100` plus what vt100 hands to its host.

use std::path::PathBuf;

use crate::encode::InputModes;

/// Side-channel state collected while parsing a pane's output.
#[derive(Debug, Default)]
pub struct Hooks {
    title: Option<String>,
    bell: bool,
    /// OSC 52 payloads (`(selection, base64 data)`) waiting to be forwarded.
    clipboard: Vec<(String, String)>,
    /// Working directory announced by the shell (OSC 7 / OSC 9;9).
    cwd: Option<PathBuf>,
    /// Bytes to write back to the program: answers to terminal queries.
    replies: Vec<u8>,
    /// xterm `modifyOtherKeys` level set with `CSI > 4 ; n m`.
    modify_other_keys: u8,
    /// kitty keyboard flags stack (`CSI > f u` pushes, `CSI < n u` pops).
    kitty_flags: Vec<u8>,
}

impl vt100::Callbacks for Hooks {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bell = true;
    }

    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = Some(String::from_utf8_lossy(title).into_owned());
    }

    fn copy_to_clipboard(&mut self, _: &mut vt100::Screen, ty: &[u8], data: &[u8]) {
        self.clipboard.push((
            String::from_utf8_lossy(ty).into_owned(),
            String::from_utf8_lossy(data).into_owned(),
        ));
    }

    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let p = |i: usize| params.get(i).and_then(|v| v.first()).copied();
        match (i1, c) {
            // DA1: identify as a VT220 with ANSI colour, as tmux does.
            (None, 'c') if p(0).unwrap_or(0) == 0 => self.replies.extend(b"\x1b[?62;22c"),
            // DA2: tmux reports terminal type 84.
            (Some(b'>'), 'c') => self.replies.extend(b"\x1b[>84;0;0c"),
            // DSR: status and cursor position (1-based).
            (None, 'n') => match p(0) {
                Some(5) => self.replies.extend(b"\x1b[0n"),
                Some(6) => {
                    let (row, col) = screen.cursor_position();
                    self.replies
                        .extend(format!("\x1b[{};{}R", row + 1, col + 1).into_bytes());
                }
                _ => {}
            },
            (Some(b'>'), 'm') if p(0) == Some(4) => {
                self.modify_other_keys = p(1).unwrap_or(0).min(2) as u8;
            }
            (Some(b'>'), 'u') => self.kitty_flags.push(p(0).unwrap_or(0) as u8),
            (Some(b'<'), 'u') => {
                for _ in 0..p(0).unwrap_or(1).max(1) {
                    self.kitty_flags.pop();
                }
            }
            (Some(b'='), 'u') => {
                let flags = p(0).unwrap_or(0) as u8;
                let top = self.kitty_flags.last().copied().unwrap_or(0);
                let new = match p(1).unwrap_or(1) {
                    2 => top | flags,
                    3 => top & !flags,
                    _ => flags,
                };
                match self.kitty_flags.last_mut() {
                    Some(t) => *t = new,
                    None => self.kitty_flags.push(new),
                }
            }
            // Kitty keyboard query: report the current flags, so programs
            // that probe for the protocol get a truthful answer.
            (Some(b'?'), 'u') => {
                let flags = self.kitty_flags.last().copied().unwrap_or(0);
                self.replies.extend(format!("\x1b[?{flags}u").into_bytes());
            }
            _ => {}
        }
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        match params {
            [b"7", uri, ..] => {
                if let Some(path) = file_uri_path(&String::from_utf8_lossy(uri)) {
                    self.cwd = Some(path);
                }
            }
            [b"9", b"9", path, ..] => {
                let path = String::from_utf8_lossy(path);
                let path = path.trim_matches('"');
                if !path.is_empty() {
                    self.cwd = Some(PathBuf::from(path));
                }
            }
            _ => {}
        }
    }
}

/// One pane's emulator.
pub struct Emulator {
    parser: vt100::Parser<Hooks>,
}

impl Emulator {
    pub fn new(rows: u16, cols: u16, scrollback: usize) -> Self {
        Self {
            parser: vt100::Parser::new_with_callbacks(rows, cols, scrollback, Hooks::default()),
        }
    }

    /// Feed program output. Returns bytes that must be written back to the
    /// program (answers to queries such as cursor-position reports).
    pub fn process(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.parser.process(bytes);
        std::mem::take(&mut self.parser.callbacks_mut().replies)
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn screen_mut(&mut self) -> &mut vt100::Screen {
        self.parser.screen_mut()
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows.max(1), cols.max(1));
    }

    pub fn title(&self) -> Option<&str> {
        self.parser.callbacks().title.as_deref()
    }

    pub fn cwd(&self) -> Option<&std::path::Path> {
        self.parser.callbacks().cwd.as_deref()
    }

    /// Whether the bell rang since the last call; clears the flag.
    pub fn take_bell(&mut self) -> bool {
        std::mem::take(&mut self.parser.callbacks_mut().bell)
    }

    /// OSC 52 copies requested since the last call.
    pub fn take_clipboard(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.parser.callbacks_mut().clipboard)
    }

    /// The modes key encoding depends on.
    pub fn input_modes(&self) -> InputModes {
        let s = self.parser.screen();
        let h = self.parser.callbacks();
        InputModes {
            app_cursor: s.application_cursor(),
            bracketed_paste: s.bracketed_paste(),
            kitty_flags: h.kitty_flags.last().copied().unwrap_or(0),
            modify_other_keys: h.modify_other_keys,
        }
    }
}

/// The path of a `file://host/path` URI, percent-decoded. On Windows a
/// `/C:/x` path loses its leading slash.
fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(h << 4 | l);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    let decoded = String::from_utf8(out).ok()?;
    let b = decoded.as_bytes();
    if cfg!(windows) && b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        return Some(PathBuf::from(&decoded[1..]));
    }
    Some(PathBuf::from(decoded))
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_position_and_device_attribute_queries_are_answered() {
        let mut e = Emulator::new(24, 80, 0);
        assert_eq!(e.process(b"ab\x1b[6n"), b"\x1b[1;3R");
        assert_eq!(e.process(b"\x1b[c"), b"\x1b[?62;22c");
        assert_eq!(e.process(b"\x1b[5n"), b"\x1b[0n");
        assert!(e.process(b"plain text").is_empty());
    }

    #[test]
    fn osc7_sets_the_working_directory() {
        let mut e = Emulator::new(24, 80, 0);
        e.process(b"\x1b]7;file://host/home/u/my%20dir\x07");
        assert_eq!(e.cwd(), Some(std::path::Path::new("/home/u/my dir")));
        e.process(b"\x1b]7;file://host/C:/Users/u\x07");
        let want = if cfg!(windows) {
            "C:/Users/u"
        } else {
            "/C:/Users/u"
        };
        assert_eq!(e.cwd(), Some(std::path::Path::new(want)));
    }

    #[test]
    fn title_bell_and_clipboard_are_captured() {
        let mut e = Emulator::new(24, 80, 0);
        e.process(b"\x1b]2;my title\x07\x07\x1b]52;c;aGk=\x07");
        assert_eq!(e.title(), Some("my title"));
        assert!(e.take_bell());
        assert!(!e.take_bell());
        assert_eq!(e.take_clipboard(), vec![("c".into(), "aGk=".into())]);
    }

    #[test]
    fn extended_key_modes_are_tracked() {
        let mut e = Emulator::new(24, 80, 0);
        assert_eq!(e.input_modes().modify_other_keys, 0);
        e.process(b"\x1b[>4;2m");
        assert_eq!(e.input_modes().modify_other_keys, 2);
        e.process(b"\x1b[>1u");
        assert_eq!(e.input_modes().kitty_flags, 1);
        assert_eq!(e.process(b"\x1b[?u"), b"\x1b[?1u");
        e.process(b"\x1b[<u");
        assert_eq!(e.input_modes().kitty_flags, 0);
        e.process(b"\x1b[?1h");
        assert!(e.input_modes().app_cursor);
    }
}
