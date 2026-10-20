//! A pane's terminal emulator: `vt100` plus what vt100 hands to its host.

use std::path::PathBuf;

use crate::dcs::DcsSplitter;
use crate::encode::InputModes;
use crate::modes::{ModeSplitter, Piece};

/// Most kitty keyboard flag sets a pane keeps. The protocol asks a terminal
/// to cap the stack, dropping the oldest entry on a push to a full one, so a
/// program cannot grow it without end; the size is the terminal's choice.
const KITTY_STACK_LIMIT: usize = 16;

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
    /// The program asked for focus reports (`CSI ? 1004 h`).
    focus_reports: bool,
    /// The cursor shape the program asked for (`CSI Ps SP q`, DECSCUSR):
    /// 0 for the terminal's own.
    cursor_style: u8,
    /// Where the shell's input starts, from its OSC 133;B prompt mark:
    /// (lines of history then, screen row, column).
    input_mark: Option<(usize, u16, u16)>,
}

/// Lines of history above `screen`, read by moving its view to the top and
/// back.
fn history_lines(screen: &mut vt100::Screen) -> usize {
    let view = screen.scrollback();
    screen.set_scrollback(usize::MAX);
    let lines = screen.scrollback();
    screen.set_scrollback(view);
    lines
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
            // A mode vt100 does not keep: focus reports.
            (Some(b'?'), 'h' | 'l') if params.iter().any(|v| v.first() == Some(&1004)) => {
                self.focus_reports = c == 'h';
            }
            // DECSCUSR: a cursor shape, 0 to 6; others leave it as it was,
            // as tmux's.
            (Some(b' '), 'q') => {
                if let Ok(ps @ 0..=6) = u8::try_from(p(0).unwrap_or(0)) {
                    self.cursor_style = ps;
                }
            }
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
            (Some(b'>'), 'u') => {
                if self.kitty_flags.len() >= KITTY_STACK_LIMIT {
                    self.kitty_flags.remove(0);
                }
                self.kitty_flags.push(p(0).unwrap_or(0) as u8);
            }
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

    fn unhandled_osc(&mut self, screen: &mut vt100::Screen, params: &[&[u8]]) {
        match params {
            // Shell integration (FinalTerm / OSC 133): B ends the prompt, so
            // the input starts at the cursor; A (a new prompt), C (the
            // command runs) and D (it finished) end that input.
            [b"133", b"B", ..] => {
                let (row, col) = screen.cursor_position();
                self.input_mark = Some((history_lines(screen), row, col));
            }
            [b"133", b"A" | b"C" | b"D", ..] => self.input_mark = None,
            // A title holding `;` arrives split, which vt100 leaves alone:
            // all of it after the first `;` is the title, as xterm reads it.
            [b"0" | b"2", rest @ ..] if rest.len() > 1 => {
                self.title = Some(String::from_utf8_lossy(&rest.join(&b';')).into_owned());
            }
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

/// Whether panes' tmux passthrough can be taken out and forwarded.
///
/// Windows' built-in ConPTY forwards a program's DCS but drops its closing
/// `ESC \` (seen on Windows 11 build 26300), so the end of a passthrough
/// cannot be found and the splitter would hide the pane's later output; there
/// the output goes to vt100 unsplit, as it would without passthrough.
/// Microsoft's newer ConPTY keeps the terminator, and portable-pty loads it
/// when its `conpty.dll` (with the `OpenConsole.exe` it starts) sits beside
/// the program, which is how tmxr's Windows release ships it.
pub fn passthrough_supported() -> bool {
    #[cfg(windows)]
    {
        static BUNDLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *BUNDLED.get_or_init(bundled_conpty)
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Whether Microsoft's ConPTY sits beside this program.
#[cfg(windows)]
fn bundled_conpty() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
        .is_some_and(|dir| {
            dir.join("conpty.dll").is_file() && dir.join("OpenConsole.exe").is_file()
        })
}

/// What [`Emulator::reset`] feeds the parser: CAN, then SGR reset, the
/// full scroll region, application cursor keys, origin, insert, the mouse
/// modes and bracketed paste off, the numeric keypad, the cursor shown,
/// wrapping on, and the screen cleared with the cursor home.
const RESET: &[u8] = b"\x18\x1b[0m\x1b[r\x1b[?1l\x1b[?6l\x1b[4l\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?2004l\x1b>\x1b[?25h\x1b[?7h\x1b[H\x1b[2J";

/// One pane's emulator.
pub struct Emulator {
    parser: vt100::Parser<Hooks>,
    dcs: DcsSplitter,
    /// tmux passthrough payloads (`ESC P tmux; … ESC \`) waiting to be
    /// forwarded to the outer terminal.
    passthrough: Vec<(Vec<u8>, (u16, u16))>,
    /// The last window name a program asked for (`ESC k … ESC \`) since
    /// the last [`Emulator::take_rename`].
    rename: Option<Vec<u8>>,
    /// [`passthrough_supported`], read once.
    split: bool,
    /// The program's switches to and from the alternate screen, split
    /// from its output.
    modes: ModeSplitter,
    /// Cleared with `alternate-screen off`: the switches are dropped.
    alt_allowed: bool,
    /// The mouse encodings the program set, apart as tmux keeps them (vt100
    /// keeps one).
    mouse_utf8: bool,
    mouse_sgr: bool,
    /// The history limit, for a fresh parser.
    scrollback: usize,
    /// Lines of history above the screen, counted whenever they can change
    /// ([`Emulator::count_history`]): a scrollbar reads it every frame.
    history: usize,
}

impl Emulator {
    pub fn new(rows: u16, cols: u16, scrollback: usize) -> Self {
        Self {
            parser: vt100::Parser::new_with_callbacks(rows, cols, scrollback, Hooks::default()),
            dcs: DcsSplitter::default(),
            passthrough: Vec::new(),
            rename: None,
            split: passthrough_supported(),
            modes: ModeSplitter::default(),
            alt_allowed: true,
            mouse_utf8: false,
            mouse_sgr: false,
            scrollback,
            history: 0,
        }
    }

    /// Where the shell's input starts on the screen, as its last OSC 133
    /// prompt mark said: `None` without a mark, on the alternate screen, or
    /// once the history is full (lines then scroll away uncounted, and the
    /// mark's row is no longer known).
    pub fn input_start(&self) -> Option<(u16, u16)> {
        let (marked_history, row, col) = self.parser.callbacks().input_mark?;
        let now = self.history_size();
        let screen = self.parser.screen();
        if screen.alternate_screen() || now >= self.scrollback || now < marked_history {
            return None;
        }
        let row = usize::from(row).checked_sub(now - marked_history)?;
        Some((u16::try_from(row).ok()?, col))
    }

    /// Lines of history above the screen (tmux's `#{history_size}`).
    pub fn history_size(&self) -> usize {
        self.history
    }

    /// The most lines of history kept (tmux's `#{history_limit}`).
    pub fn history_limit(&self) -> usize {
        self.scrollback
    }

    fn count_history(&mut self) {
        self.history = history_lines(self.parser.screen_mut());
    }

    /// Drop the history above the screen (tmux's `clear-history`), keeping
    /// the screen, cursor and modes. vt100 has no way to empty its
    /// scrollback, so a fresh parser replays the old screen's state; the
    /// hooks (title, directory, pending output) move over as they are.
    pub fn clear_history(&mut self) {
        let (rows, cols) = self.parser.screen().size();
        let state = self.parser.screen().state_formatted();
        let hooks = std::mem::take(self.parser.callbacks_mut());
        let mut fresh = vt100::Parser::new_with_callbacks(rows, cols, self.scrollback, hooks);
        fresh.process(&state);
        self.parser = fresh;
        self.count_history();
    }

    /// Reset the terminal as tmux's input_reset and screen_write_reset
    /// (`send-keys -R`): any sequence half received is dropped, the cell
    /// attributes, scroll region and modes go back to the defaults (the
    /// cursor shown and wrapping on, nothing else), and the screen is
    /// cleared with the cursor at the top left. The history and the
    /// alternate screen stay, as tmux's.
    pub fn reset(&mut self) {
        self.dcs = DcsSplitter::default();
        self.modes.flush();
        (self.mouse_utf8, self.mouse_sgr) = (false, false);
        // CAN ends whatever sequence is in progress, but vte acts on an OSC
        // it cuts off: what that could change is put back, as tmux's
        // input_clear drops the sequence.
        let hooks = self.parser.callbacks();
        let (title, clips, cwd, mark) = (
            hooks.title.clone(),
            hooks.clipboard.len(),
            hooks.cwd.clone(),
            hooks.input_mark,
        );
        self.parser.process(RESET);
        let hooks = self.parser.callbacks_mut();
        hooks.title = title;
        hooks.clipboard.truncate(clips);
        hooks.cwd = cwd;
        hooks.input_mark = mark;
        hooks.focus_reports = false;
        hooks.modify_other_keys = 0;
        hooks.kitty_flags.clear();
        self.count_history();
    }

    /// Whether the program may switch to the alternate screen (tmux's
    /// `alternate-screen`).
    pub fn set_alternate_screen(&mut self, allowed: bool) {
        self.alt_allowed = allowed;
    }

    /// Parse program text, its alternate screen switches made as tmux's, or
    /// dropped when they are not allowed.
    fn feed(&mut self, text: &[u8]) {
        for piece in self.modes.split(text) {
            match piece {
                Piece::Text(text) => self.parser.process(&text),
                Piece::Mode {
                    mode: 47 | 1047 | 1049,
                    ..
                } if !self.alt_allowed => {}
                Piece::Mode {
                    mode: mode @ (47 | 1047 | 1049),
                    on,
                } => self.switch_screen(mode, on),
                Piece::Mode { mode, on } => self.mouse_mode(mode, on),
                Piece::Reset => {
                    (self.mouse_utf8, self.mouse_sgr) = (false, false);
                    self.parser.process(b"\x1bc");
                }
            }
        }
    }

    /// A mouse mode set or reset as tmux's input_csi_dispatch_sm_private /
    /// _rm_private, not as vt100 would: 9 (X10) is ignored, resetting any of
    /// 1000 to 1003 clears whichever is set (vt100 clears only the one
    /// named), and 1005 / 1006 are kept apart.
    fn mouse_mode(&mut self, mode: u32, on: bool) {
        match (mode, on) {
            (1000 | 1002 | 1003, true) => {
                self.parser.process(format!("\x1b[?{mode}h").as_bytes());
            }
            (1000..=1003, false) => self.parser.process(b"\x1b[?1000l\x1b[?1002l\x1b[?1003l"),
            (1005, on) => {
                self.mouse_utf8 = on;
                self.mouse_encoding();
            }
            (1006, on) => {
                self.mouse_sgr = on;
                self.mouse_encoding();
            }
            // 9, and setting 1001, which tmux takes neither.
            _ => {}
        }
    }

    /// The one encoding vt100 keeps, from the two the program set: SGR
    /// before UTF-8, as tmux's input_key_get_mouse picks.
    fn mouse_encoding(&mut self) {
        self.parser.process(b"\x1b[?1005l\x1b[?1006l");
        if self.mouse_sgr {
            self.parser.process(b"\x1b[?1006h");
        } else if self.mouse_utf8 {
            self.parser.process(b"\x1b[?1005h");
        }
    }

    /// Whether the program set the UTF-8 mouse encoding (1005), set or not
    /// beside SGR, as tmux's MODE_MOUSE_UTF8.
    pub fn mouse_utf8(&self) -> bool {
        self.mouse_utf8
    }

    /// Switch to or from the alternate screen as tmux's screen_alternate_on
    /// / _off, not as vt100 would: on (47, 1047 or 1049, unless already
    /// there) clears it and keeps the cursor where it was, where vt100 homes
    /// it on 1049, keeps the old screen on 47 and ignores 1047; off, 1049
    /// puts back the cursor it saved, and 47 / 1047 keep it where it is.
    fn switch_screen(&mut self, mode: u32, on: bool) {
        let alternate = self.parser.screen().alternate_screen();
        let (row, col) = self.parser.screen().cursor_position();
        let cursor = format!("\x1b[{};{}H", row + 1, col + 1);
        match (on, mode) {
            (true, _) if alternate => {}
            (true, _) => {
                self.parser.process(b"\x1b[?1049h");
                self.parser.process(cursor.as_bytes());
            }
            (false, 1049) => self.parser.process(b"\x1b[?1049l"),
            (false, _) if alternate => {
                self.parser.process(b"\x1b[?47l");
                self.parser.process(cursor.as_bytes());
            }
            (false, _) => {}
        }
    }

    /// Feed program output. Returns bytes that must be written back to the
    /// program (answers to queries such as cursor-position reports).
    pub fn process(&mut self, bytes: &[u8]) -> Vec<u8> {
        if self.split {
            let split = self.dcs.split(bytes);
            // Each payload with the cursor where it was printed: the text
            // before it is parsed first.
            let mut done = 0;
            for (at, payload) in split.passthrough {
                self.feed(&split.text[done..at]);
                done = at;
                let cursor = self.parser.screen().cursor_position();
                self.passthrough.push((payload, cursor));
            }
            self.feed(&split.text[done..]);
            if split.rename.is_some() {
                self.rename = split.rename;
            }
        } else {
            self.feed(bytes);
        }
        self.count_history();
        std::mem::take(&mut self.parser.callbacks_mut().replies)
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// For moving the scrollback view; [`Emulator::resize`] resizes, so
    /// the history stays counted.
    pub fn screen_mut(&mut self) -> &mut vt100::Screen {
        self.parser.screen_mut()
    }

    /// Resize the screen. Fewer rows go as tmux 3.6's `screen_resize_y`
    /// takes them: the rows below the cursor first, then rows off the top,
    /// into the history on the main screen. (vt100 alone cuts rows off the
    /// bottom, the cursor's among them.) More rows come in blank at the
    /// bottom; tmux would first take back the rows a shrink pushed into the
    /// history, which vt100 cannot.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let (old_rows, _) = self.parser.screen().size();
        let (cursor_row, _) = self.parser.screen().cursor_position();
        let below = old_rows.saturating_sub(cursor_row + 1);
        let off_top = old_rows.saturating_sub(rows).saturating_sub(below);
        if off_top > 0 {
            // Scroll them up (SU), the cursor with them (CUU).
            let scroll = format!("\x1b[{off_top}S\x1b[{off_top}A");
            self.parser.process(scroll.as_bytes());
        }
        self.parser.screen_mut().set_size(rows, cols.max(1));
        self.count_history();
    }

    /// Whether the program asked to be told of focus changes (mode 1004),
    /// as `CSI I` and `CSI O`.
    pub fn focus_reports(&self) -> bool {
        self.parser.callbacks().focus_reports
    }

    /// The cursor shape the program asked for, as DECSCUSR's `Ps`: 0 (the
    /// terminal's own) until it asks.
    pub fn cursor_style(&self) -> u8 {
        self.parser.callbacks().cursor_style
    }

    pub fn title(&self) -> Option<&str> {
        self.parser.callbacks().title.as_deref()
    }

    /// Set the title as a program's OSC 0/2 would (`select-pane -T`).
    pub fn set_title(&mut self, title: String) {
        self.parser.callbacks_mut().title = Some(title);
    }

    /// Forget the title, as before any program set one.
    pub fn clear_title(&mut self) {
        self.parser.callbacks_mut().title = None;
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

    /// tmux passthrough payloads received since the last call, each with the
    /// cursor (row, column) it was printed at.
    pub fn take_passthrough(&mut self) -> Vec<(Vec<u8>, (u16, u16))> {
        std::mem::take(&mut self.passthrough)
    }

    /// The window name a program last asked for with `ESC k … ESC \`, if
    /// any since the last call. Only where [`passthrough_supported`]: the
    /// same splitter finds both.
    pub fn take_rename(&mut self) -> Option<Vec<u8>> {
        self.rename.take()
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
    fn reset_clears_the_screen_and_modes_but_keeps_the_history() {
        let mut emu = Emulator::new(3, 20, 100);
        let lines: String = (0..10).map(|i| format!("line{i}\r\n")).collect();
        emu.process(lines.as_bytes());
        let history = emu.history_size();
        assert!(history > 0);
        // Modes on, then an OSC cut off before its end.
        emu.process(b"\x1b[?1h\x1b[?2004h\x1b[?1004h\x1b[?25l\x1b[2;3r\x1b]0;half");
        emu.reset();
        let s = emu.screen();
        assert!(!s.application_cursor() && !s.bracketed_paste() && !s.hide_cursor());
        assert!(!emu.focus_reports());
        assert_eq!(s.cursor_position(), (0, 0));
        assert_eq!(s.contents().trim(), "");
        assert_eq!(emu.history_size(), history);
        // The OSC was dropped: this text is printed, not taken as its title.
        emu.process(b"after");
        assert_eq!(emu.screen().contents().trim(), "after");
        assert_eq!(emu.title(), None);
    }

    #[test]
    fn the_alternate_screen_keeps_the_cursor_as_tmuxs() {
        let rows = |emu: &Emulator| {
            let s = emu.screen();
            (s.alternate_screen(), s.cursor_position(), s.contents())
        };
        // 1049 keeps the cursor going on and puts it back coming off.
        let mut emu = Emulator::new(4, 20, 100);
        emu.process(b"main\r\n\x1b[?1049halt");
        assert_eq!(rows(&emu), (true, (1, 3), "\nalt".to_owned()));
        emu.process(b"\r\nmore\x1b[?1049l");
        assert_eq!(rows(&emu), (false, (1, 0), "main".to_owned()));
        // 1047, which vt100 ignores, switches and clears as 47 does; going
        // off, both keep the cursor where it got to.
        for mode in ["1047", "47"] {
            let mut emu = Emulator::new(4, 20, 100);
            emu.process(format!("main\r\n\x1b[?{mode}hx").as_bytes());
            assert_eq!(rows(&emu), (true, (1, 1), "\nx".to_owned()), "{mode}");
            emu.process(format!("\r\ny\x1b[?{mode}lz").as_bytes());
            assert_eq!(
                rows(&emu),
                (false, (2, 2), "main\n\n z".to_owned()),
                "{mode}"
            );
            // Switched on again, the alternate screen starts clear.
            emu.process(format!("\x1b[?{mode}h").as_bytes());
            assert_eq!(emu.screen().contents(), "", "{mode}");
        }
        // Already there, switching on changes nothing.
        let mut emu = Emulator::new(4, 20, 100);
        emu.process(b"\x1b[?1049hab\x1b[?1049h");
        assert_eq!(rows(&emu), (true, (0, 2), "ab".to_owned()));
    }

    #[test]
    fn clearing_history_keeps_the_screen() {
        let mut emu = Emulator::new(3, 20, 100);
        let lines: String = (0..10).map(|i| format!("line{i}\r\n")).collect();
        emu.process(lines.as_bytes());
        emu.process(b"\x1b[1mbold");
        let history = |emu: &mut Emulator| {
            emu.screen_mut().set_scrollback(usize::MAX);
            let n = emu.screen().scrollback();
            emu.screen_mut().set_scrollback(0);
            n
        };
        assert!(history(&mut emu) > 0);
        assert_eq!(emu.history_size(), history(&mut emu));
        let (screen, cursor) = (emu.screen().contents(), emu.screen().cursor_position());
        emu.clear_history();
        assert_eq!(history(&mut emu), 0);
        assert_eq!(emu.history_size(), 0);
        assert_eq!(emu.screen().contents(), screen);
        assert_eq!(emu.screen().cursor_position(), cursor);
        assert!(emu.screen().cell(2, 0).unwrap().bold(), "attributes kept");
        // New output still scrolls into a history again.
        emu.process(b"\r\nmore\r\nlines\r\n");
        assert!(history(&mut emu) > 0);
    }

    #[test]
    fn cursor_position_and_device_attribute_queries_are_answered() {
        let mut e = Emulator::new(24, 80, 0);
        assert_eq!(e.process(b"ab\x1b[6n"), b"\x1b[1;3R");
        assert_eq!(e.process(b"\x1b[c"), b"\x1b[?62;22c");
        assert_eq!(e.process(b"\x1b[5n"), b"\x1b[0n");
        assert!(e.process(b"plain text").is_empty());
    }

    #[test]
    fn shrinking_keeps_the_cursor_row_as_tmux_does() {
        let row = |e: &Emulator, r: u16| e.screen().contents_between(r, 0, r, 20);
        // The cursor on the last row: the top row goes into the history.
        let mut e = Emulator::new(4, 20, 100);
        e.process(b"one\r\ntwo\r\nthree\r\n$ ");
        e.resize(3, 20);
        assert_eq!(e.history_size(), 1);
        assert_eq!((row(&e, 0), row(&e, 2)), ("two".into(), "$ ".into()));
        assert_eq!(e.screen().cursor_position(), (2, 2));
        // Blank rows under the cursor go first, the history untouched.
        let mut e = Emulator::new(4, 20, 100);
        e.process(b"one\r\n$ ");
        e.resize(2, 20);
        assert_eq!(e.history_size(), 0);
        assert_eq!((row(&e, 0), row(&e, 1)), ("one".into(), "$ ".into()));
        assert_eq!(e.screen().cursor_position(), (1, 2));
    }

    #[test]
    fn passthrough_keeps_the_cursor_it_was_printed_at() {
        let mut e = Emulator::new(4, 20, 0);
        e.split = true;
        e.process(b"ab\r\ncd\x1bPtmux;\x1b\x1b]1;x\x07\x1b\\ef");
        assert_eq!(
            e.take_passthrough(),
            vec![(b"\x1b]1;x\x07".to_vec(), (1, 2))]
        );
        assert_eq!(e.screen().cursor_position(), (1, 4), "the rest parsed too");
    }

    #[test]
    fn prompt_marks_say_where_the_input_starts() {
        let mut e = Emulator::new(3, 20, 100);
        assert_eq!(e.input_start(), None);
        e.process(b"PS> \x1b]133;B\x07echo hi");
        assert_eq!(e.input_start(), Some((0, 4)));
        // The command runs: no input any more.
        e.process(b"\x1b]133;C\x07\r\nhi\r\n");
        assert_eq!(e.input_start(), None);
        // A mark on the last row moves up as the screen scrolls.
        e.process(b"$ \x1b]133;B\x07");
        assert_eq!(e.input_start(), Some((2, 2)));
        e.process(b"a\r\n");
        assert_eq!(e.input_start(), Some((1, 2)));
        // Scrolled off the screen: gone.
        e.process(b"b\r\nc\r\n");
        assert_eq!(e.input_start(), None);
        // A full history no longer counts what scrolls away.
        let mut small = Emulator::new(2, 20, 1);
        small.process(b"x\r\ny\r\nz\r\n$ \x1b]133;B\x07");
        assert_eq!(small.input_start(), None);
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
        e.process(b"\x1b]0;vim; a.rs;\x07");
        assert_eq!(e.title(), Some("vim; a.rs;"), "a title with semicolons");
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
        // A full stack drops its oldest entry: after 20 pushes, popping the
        // limit's worth empties it.
        for flags in 1..=20 {
            e.process(format!("\x1b[>{flags}u").as_bytes());
        }
        e.process(format!("\x1b[<{}u", KITTY_STACK_LIMIT - 1).as_bytes());
        assert_eq!(
            usize::from(e.input_modes().kitty_flags),
            20 - (KITTY_STACK_LIMIT - 1)
        );
        e.process(b"\x1b[<u");
        assert_eq!(e.input_modes().kitty_flags, 0, "the stack holds the limit");
        e.process(b"\x1b[?1h");
        assert!(e.input_modes().app_cursor);
        // Focus reports, in a sequence with a mode vt100 keeps.
        assert!(!e.focus_reports());
        e.process(b"\x1b[?25;1004h");
        assert!(e.focus_reports());
        e.process(b"\x1b[?1004l");
        assert!(!e.focus_reports());
    }
}
