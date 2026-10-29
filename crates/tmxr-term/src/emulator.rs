//! A pane's terminal emulator: `vt1k` plus what vt1k hands to its host.

use std::num::NonZeroU16;
use std::path::PathBuf;

use crate::dcs::DcsSplitter;
use crate::encode::{ExtendedKeys, InputModes};
use crate::modes::{ModeSplitter, Piece};

/// Most kitty keyboard flag sets a pane keeps. The protocol asks a terminal
/// to cap the stack, dropping the oldest entry on a push to a full one, so a
/// program cannot grow it without end; the size is the terminal's choice.
const KITTY_STACK_LIMIT: usize = 16;

/// `screen`'s rows and columns, which vt1k keeps nonzero, as numbers.
pub fn screen_size(screen: &vt1k::Screen) -> (u16, u16) {
    let (rows, cols) = screen.size();
    (rows.get(), cols.get())
}

/// A screen dimension as vt1k takes it: at least 1, as tmux's
/// `PANE_MINIMUM`.
fn dimension(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n).unwrap_or(NonZeroU16::MIN)
}

/// Side-channel state collected while parsing a pane's output.
#[derive(Debug, Default)]
pub struct Hooks {
    /// Rows the main screen's history has lost off its top: with the
    /// history's length and a screen row, a line's number for good, which
    /// the shell's prompt marks are kept by.
    dropped: usize,
    /// Lines the shell marked as a prompt's start (OSC 133 A) and an
    /// output's (C), by number, as tmux's `GRID_LINE_START_PROMPT` /
    /// `_OUTPUT` line flags.
    prompts: std::collections::BTreeSet<usize>,
    outputs: std::collections::BTreeSet<usize>,
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
    /// The `extended-keys` option as output arrives: `off` ignores a
    /// `CSI > 4 ; n m`, as tmux's input_csi_dispatch_sgr_other.
    extended_keys: ExtendedKeys,
    /// kitty keyboard flags stack (`CSI > f u` pushes, `CSI < n u` pops).
    kitty_flags: Vec<u8>,
    /// The queries the server answers, from the last [`Emulator::process`],
    /// each with where in `replies` its answer goes.
    queries: Vec<(usize, Query)>,
    /// The cursor's colour as RGB (OSC 12; OSC 112 resets it), which a
    /// full reset keeps, as tmux's.
    cursor_colour: Option<u32>,
    /// The default foreground and background the program set as RGB (OSC
    /// 10 / 11; OSC 110 / 111 or a full reset clear them), as tmux's pane
    /// palette's `fg` / `bg`.
    palette_fg: Option<u32>,
    palette_bg: Option<u32>,
    /// Where the shell's input starts, from its OSC 133;B prompt mark:
    /// (lines of history then, screen row, column).
    input_mark: Option<(usize, u16, u16)>,
}

impl vt1k::Callbacks for Hooks {
    fn audible_bell(&mut self, _: &mut vt1k::Screen) {
        self.bell = true;
    }

    fn on_scroll(&mut self, _: vt1k::capture::RowContents<'_>, alternate_screen: bool) {
        if !alternate_screen {
            self.dropped += 1;
        }
    }

    fn set_window_title(&mut self, _: &mut vt1k::Screen, title: &[u8]) {
        self.title = Some(String::from_utf8_lossy(title).into_owned());
    }

    fn copy_to_clipboard(&mut self, _: &mut vt1k::Screen, ty: &[u8], data: &[u8]) {
        self.clipboard.push((
            String::from_utf8_lossy(ty).into_owned(),
            String::from_utf8_lossy(data).into_owned(),
        ));
    }

    fn unhandled_csi(
        &mut self,
        screen: &mut vt1k::Screen,
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
            (Some(b'?'), 'n') if p(0) == Some(996) => {
                self.queries.push((self.replies.len(), Query::Theme));
            }
            (None, 'n') => match p(0) {
                Some(5) => self.replies.extend(b"\x1b[0n"),
                Some(6) => {
                    let (row, col) = screen.cursor_position();
                    self.replies
                        .extend(format!("\x1b[{};{}R", row + 1, col + 1).into_bytes());
                }
                _ => {}
            },
            (Some(b'>'), 'm') if p(0) == Some(4) && self.extended_keys != ExtendedKeys::Off => {
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

    fn unhandled_osc(&mut self, screen: &mut vt1k::Screen, params: &[&[u8]]) {
        match params {
            // Shell integration (FinalTerm / OSC 133): B ends the prompt, so
            // the input starts at the cursor; A (a new prompt), C (the
            // command runs) and D (it finished) end that input.
            [b"133", b"B", ..] => {
                let (row, col) = screen.cursor_position();
                self.input_mark = Some((screen.scrollback_len(), row, col));
            }
            [b"133", mark @ (b"A" | b"C" | b"D"), ..] => {
                self.input_mark = None;
                // The cursor's line starts a prompt (A) or an output (C).
                if !screen.alternate_screen() && *mark != b"D" {
                    let row = usize::from(screen.cursor_position().0);
                    let line = self.dropped + screen.scrollback_len() + row;
                    let marks = if *mark == b"A" {
                        &mut self.prompts
                    } else {
                        &mut self.outputs
                    };
                    marks.insert(line);
                }
            }
            // A title holding `;` arrives split, which vt1k leaves alone:
            // all of it after the first `;` is the title, as xterm reads it.
            [b"0" | b"2", rest @ ..] if rest.len() > 1 => {
                self.title = Some(String::from_utf8_lossy(&rest.join(&b';')).into_owned());
            }
            // The cursor's colour, as tmux's input_osc_12: one it cannot
            // read changes nothing. Queries (`?`) are not answered.
            [b"12", spec @ ..] if !spec.is_empty() => {
                let spec = String::from_utf8_lossy(&spec.join(&b';')).into_owned();
                if let Some(rgb) = tmxr_command::colour::parse_x11(&spec) {
                    self.cursor_colour = Some(rgb);
                }
            }
            [b"112"] | [b"112", b""] => self.cursor_colour = None,
            // The default colours, as tmux's input_osc_10 / _11; a query
            // never gets here (`modes::Piece::ColourQuery`).
            [which @ (b"10" | b"11"), spec @ ..] if !spec.is_empty() => {
                let spec = String::from_utf8_lossy(&spec.join(&b';')).into_owned();
                if let Some(rgb) = tmxr_command::colour::parse_x11(&spec) {
                    match *which {
                        b"10" => self.palette_fg = Some(rgb),
                        _ => self.palette_bg = Some(rgb),
                    }
                }
            }
            [b"110"] | [b"110", b""] => self.palette_fg = None,
            [b"111"] | [b"111", b""] => self.palette_bg = None,
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
/// the output goes to vt1k unsplit, as it would without passthrough.
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
/// modes, focus reports, the cursor's blinking and bracketed paste off, the
/// numeric keypad, the cursor shown and not very visible, wrapping on, and
/// the screen cleared with the cursor home.
const RESET: &[u8] = b"\x18\x1b[0m\x1b[r\x1b[?1l\x1b[?6l\x1b[4l\x1b[?1000l\x1b[?1005l\x1b[?1006l\x1b[?1004l\x1b[?12l\x1b[?2004l\x1b>\x1b[?25h\x1b[34h\x1b[?7h\x1b[H\x1b[2J";

/// A query a program made that the server answers ([`Emulator::take_queries`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Query {
    /// `OSC 10`, `11` or `12` `; ?` (`which`): the default foreground,
    /// background or the cursor's colour; `bel` where it ended in a BEL, as
    /// its answer must.
    Colour { which: u8, bel: bool },
    /// `CSI ? 996 n`: the color scheme.
    Theme,
    /// `OSC 4 ; index ; ?` for an entry the program has not set: the outer
    /// terminal has it.
    Palette { index: u8, bel: bool },
}

/// One pane's emulator.
pub struct Emulator {
    parser: vt1k::Parser<Hooks>,
    dcs: DcsSplitter,
    /// tmux passthrough payloads (`ESC P tmux; … ESC \`) waiting to be
    /// forwarded to the outer terminal.
    passthrough: Vec<(Vec<u8>, (u16, u16))>,
    /// The last window name a program asked for (`ESC k … ESC \`) since
    /// the last [`Emulator::take_rename`].
    rename: Option<Vec<u8>>,
    /// [`passthrough_supported`], read once.
    split: bool,
    /// The full resets and colour OSCs, split from the program's output.
    modes: ModeSplitter,
    /// The 256 colours' entries the program set (OSC 4).
    palette: crate::palette::Palette,
    /// The history limit, for a fresh parser.
    scrollback: usize,
    /// Lines of history above the screen, counted whenever they can change
    /// ([`Emulator::count_history`]): a scrollbar reads it every frame.
    history: usize,
    /// When each line of history went into it, in seconds since the epoch,
    /// as tmux's grid line time; `times_from` is the first one's number (as
    /// [`Hooks::dropped`] counts lines). 0 for a line a rewrap made. Lines
    /// the screen took back from the history keep theirs past its end, as
    /// tmux's grid lines carry their time.
    times: std::collections::VecDeque<u64>,
    times_from: usize,
    /// The number of the line after the history's last when it was last
    /// counted: lines from there are new to it.
    history_end: usize,
}

impl Emulator {
    pub fn new(rows: u16, cols: u16, scrollback: usize) -> Self {
        let mut parser = vt1k::Parser::new_with_callbacks(
            dimension(rows),
            dimension(cols),
            scrollback,
            Hooks::default(),
        );
        // A width change rewraps the lines, as tmux's reflow.
        parser.screen_mut().set_reflow(true);
        Self {
            parser,
            dcs: DcsSplitter::default(),
            passthrough: Vec::new(),
            rename: None,
            split: passthrough_supported(),
            modes: ModeSplitter::default(),
            palette: crate::palette::Palette::default(),
            scrollback,
            history: 0,
            times: std::collections::VecDeque::new(),
            times_from: 0,
            history_end: 0,
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

    /// `resized` says a resize moved the lines, as tmux's screen_resize_y:
    /// it stamps none it pushes into the history, and the lines it takes
    /// back onto the screen keep their times. Otherwise lines the history
    /// lost are gone for good.
    fn count_history(&mut self, resized: bool) {
        self.history = self.parser.screen().scrollback_len();
        self.stamp_history(resized);
    }

    /// Lines output scrolled into the history take the time now, as tmux's
    /// grid_scroll_history stamps them; lines it lost take theirs away.
    fn stamp_history(&mut self, resized: bool) {
        let dropped = self.parser.callbacks().dropped;
        while self.times_from < dropped && self.times.pop_front().is_some() {
            self.times_from += 1;
        }
        if self.times.is_empty() {
            self.times_from = dropped;
        }
        let end = dropped + self.history;
        if end < self.history_end && !resized {
            self.times.truncate(end.saturating_sub(self.times_from));
        }
        let fresh = self.history_end.max(self.times_from)..end;
        self.history_end = end;
        if fresh.is_empty() {
            return;
        }
        if self.times.len() < end - self.times_from {
            self.times.resize(end - self.times_from, 0);
        }
        if resized {
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        for line in fresh {
            self.times[line - self.times_from] = now;
        }
    }

    /// When line `line` of the history (the oldest 0) went into it, in
    /// seconds since the epoch (tmux's `#{top_line_time}` for a view
    /// starting there); 0 for one a rewrap made, and for a line of the
    /// screen unless the screen took it back from the history.
    pub fn line_time(&self, line: usize) -> u64 {
        let number = self.parser.callbacks().dropped + line;
        number
            .checked_sub(self.times_from)
            .and_then(|i| self.times.get(i).copied())
            .unwrap_or(0)
    }

    /// Drop the history above the screen (tmux's `clear-history`), keeping
    /// the screen, cursor and modes.
    pub fn clear_history(&mut self) {
        let lost = self.parser.screen().scrollback_len();
        self.parser.callbacks_mut().dropped += lost;
        self.parser.screen_mut().clear_scrollback();
        self.count_history(false);
    }

    /// The lines of the history and screen, the oldest 0, that the shell
    /// marked as starting a prompt (OSC 133 A), or with `output` an
    /// output (C): what copy mode's `next-prompt` / `previous-prompt` move
    /// between. Marks on lines the history has lost are dropped here.
    pub fn prompt_lines(&mut self, output: bool) -> Vec<usize> {
        let screen = self.parser.screen();
        let lines = screen.scrollback_len() + usize::from(screen_size(screen).0);
        let hooks = self.parser.callbacks_mut();
        let dropped = hooks.dropped;
        for marks in [&mut hooks.prompts, &mut hooks.outputs] {
            *marks = marks.split_off(&dropped);
        }
        let marks = if output {
            &hooks.outputs
        } else {
            &hooks.prompts
        };
        marks
            .range(dropped..dropped + lines)
            .map(|l| l - dropped)
            .collect()
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
        self.parser.screen_mut().reset_tab_stops();
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
        hooks.kitty_flags.clear();
        self.reset_keys();
        self.count_history(false);
    }

    /// The `extended-keys` option, for the output that follows: `off`
    /// ignores a program's request for extended keys, as tmux's.
    pub fn set_extended_keys(&mut self, option: ExtendedKeys) {
        self.parser.callbacks_mut().extended_keys = option;
    }

    /// The extended-keys level a new or reset screen starts at, as tmux's
    /// screen_write_reset: 1 under `extended-keys always`, else none.
    pub fn reset_keys(&mut self) {
        let hooks = self.parser.callbacks_mut();
        hooks.modify_other_keys = u8::from(hooks.extended_keys == ExtendedKeys::Always);
    }

    /// Whether the program may switch to the alternate screen (tmux's
    /// `alternate-screen`).
    pub fn set_alternate_screen(&mut self, allowed: bool) {
        self.parser.screen_mut().set_alternate_allowed(allowed);
    }

    /// Parse program text, the colours it sets or asks for kept here.
    fn feed(&mut self, text: &[u8]) {
        for piece in self.modes.split(text) {
            match piece {
                Piece::Text(text) => self.parser.process(&text),
                Piece::Reset => {
                    // tmux's full reset clears the program's colours; its
                    // cursor colour is kept.
                    let hooks = self.parser.callbacks_mut();
                    (hooks.palette_fg, hooks.palette_bg) = (None, None);
                    self.palette.clear();
                    // The new screen's lines are none the history had.
                    let history = self.history_end.saturating_sub(self.times_from);
                    self.times.truncate(history);
                    self.parser.process(b"\x1bc");
                    // Keys go plain again, as tmux's screen_write_reset
                    // and the kitty protocol's full reset have them.
                    self.parser.callbacks_mut().kitty_flags.clear();
                    self.reset_keys();
                }
                Piece::ColourQuery { which, bel } => {
                    let hooks = self.parser.callbacks_mut();
                    hooks
                        .queries
                        .push((hooks.replies.len(), Query::Colour { which, bel }));
                }
                Piece::Palette { osc, body, bel } => self.palette_osc(osc, &body, bel),
            }
        }
    }

    /// The columns with a tab stop, counted from 0 (`#{pane_tabs}`).
    pub fn tab_stops(&self) -> impl Iterator<Item = u16> + '_ {
        self.parser.screen().tab_stops()
    }

    /// The scroll region's top and bottom rows, counted from 0
    /// (`#{scroll_region_upper}`, `#{scroll_region_lower}`).
    pub fn scroll_region(&self) -> (u16, u16) {
        self.parser.screen().scroll_region()
    }

    /// Whether the program set the UTF-8 mouse encoding (1005), set or not
    /// beside SGR, as tmux's MODE_MOUSE_UTF8.
    pub fn mouse_utf8(&self) -> bool {
        self.parser.screen().mouse_utf8()
    }

    /// Feed program output. Returns bytes that must be written back to the
    /// program (answers to queries such as cursor-position reports).
    pub fn process(&mut self, bytes: &[u8]) -> Vec<u8> {
        // Their places are in the replies the last call returned.
        self.parser.callbacks_mut().queries.clear();
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
        // Leaving the alternate screen reflows a normal screen resized
        // under it.
        self.follow_reflow();
        self.count_history(false);
        std::mem::take(&mut self.parser.callbacks_mut().replies)
    }

    pub fn screen(&self) -> &vt1k::Screen {
        self.parser.screen()
    }

    /// For moving the scrollback view; [`Emulator::resize`] resizes, so
    /// the history stays counted.
    pub fn screen_mut(&mut self) -> &mut vt1k::Screen {
        self.parser.screen_mut()
    }

    /// Resize the screen, as tmux 3.6's `screen_resize_y` on the main
    /// screen: fewer rows take the rows below the cursor first, then push
    /// rows off the top into the history; more rows take rows back from the
    /// history first, then come in blank at the bottom. (tmux takes back only
    /// rows scrolled since the screen was last cleared, vt1k any it has.)
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser
            .screen_mut()
            .set_size(dimension(rows), dimension(cols));
        self.follow_reflow();
        self.count_history(true);
    }

    /// After a reflow, the line numbers the emulator keeps follow their
    /// lines: each prompt mark goes to its line's new number, those whose
    /// line fell off the history go, and the input mark goes (its column
    /// is not kept through a rewrap), as tmux's line flags move with their
    /// lines.
    fn follow_reflow(&mut self) {
        let Some(reflow) = self.parser.screen_mut().take_reflow() else {
            return;
        };
        let hooks = self.parser.callbacks_mut();
        let before = hooks.dropped;
        let after = before + reflow.dropped;
        let follow = |marks: &std::collections::BTreeSet<usize>| {
            marks
                .iter()
                .filter_map(|&mark| {
                    let line = (*reflow.lines.get(mark.checked_sub(before)?)?)?;
                    Some(after + line)
                })
                .collect()
        };
        hooks.prompts = follow(&hooks.prompts);
        hooks.outputs = follow(&hooks.outputs);
        hooks.dropped = after;
        hooks.input_mark = None;
        // A line's time goes with its first row, as tmux's grid_reflow moves
        // it: rows joined onto it keep its time, rows split off it have none.
        let history = self.parser.screen().scrollback_len();
        let mut times = std::collections::VecDeque::from(vec![0; history]);
        let mut last = None;
        for (i, &time) in self.times.iter().enumerate() {
            let line = (self.times_from + i)
                .checked_sub(before)
                .and_then(|old| reflow.lines.get(old).copied().flatten());
            if line != last
                && let Some(line) = line
            {
                if times.len() <= line {
                    times.resize(line + 1, 0);
                }
                times[line] = time;
            }
            last = line;
        }
        self.times = times;
        self.times_from = after;
        self.history_end = after + history;
    }

    /// Whether the program asked to be told of focus changes (mode 1004),
    /// as `CSI I` and `CSI O`.
    pub fn focus_reports(&self) -> bool {
        self.parser.screen().focus_reports()
    }

    /// Insert mode and autowrap as the program set them (`#{insert_flag}`,
    /// `#{wrap_flag}`).
    pub fn insert_and_wrap(&self) -> (bool, bool) {
        let screen = self.parser.screen();
        (screen.insert_mode(), screen.autowrap())
    }

    /// OSC 4 or 104 with `body`, as tmux's input_osc_4 / _104. An OSC 104
    /// with nothing after it unsets every entry and leaves the OSC 10 / 11
    /// colours, as tmux 3.6 shows it: its colour_palette_clear resets those
    /// too, but nothing marks the pane's style changed, so it goes on
    /// drawing and showing them.
    fn palette_osc(&mut self, osc: u16, body: &[u8], bel: bool) {
        let body = String::from_utf8_lossy(body);
        match osc {
            4 => {
                let end = if bel { "\x07" } else { "\x1b\\" };
                let hooks = self.parser.callbacks_mut();
                let mut unset = Vec::new();
                self.palette
                    .osc_4(&body, end, &mut hooks.replies, &mut unset);
                hooks.queries.extend(
                    unset
                        .into_iter()
                        .map(|(index, at)| (at, Query::Palette { index, bel })),
                );
            }
            _ if body.is_empty() => self.palette.clear(),
            _ => self.palette.osc_104(&body),
        }
    }

    /// The palette entries the program set (OSC 4).
    pub fn palette(&self) -> &crate::palette::Palette {
        &self.palette
    }

    /// The queries the program made in the last [`Emulator::process`] that
    /// the server answers, as their answers depend on options and clients,
    /// in order, each with where in the replies that call returned its
    /// answer goes: before the bytes from there on.
    pub fn take_queries(&mut self) -> Vec<(usize, Query)> {
        std::mem::take(&mut self.parser.callbacks_mut().queries)
    }

    /// Where the cursor was (row, column) when the program last switched to
    /// the alternate screen with `CSI ? 1049 h` (`#{alternate_saved_x}`,
    /// `#{alternate_saved_y}`), if it has.
    pub fn alternate_saved(&self) -> Option<(u16, u16)> {
        self.parser.screen().alternate_saved()
    }

    /// The cursor's colour the program set, as RGB (`#{cursor_colour}`).
    pub fn cursor_colour(&self) -> Option<u32> {
        self.parser.callbacks().cursor_colour
    }

    /// The default foreground and background the program set (OSC 10 /
    /// 11), as RGB.
    pub fn palette_colours(&self) -> (Option<u32>, Option<u32>) {
        let hooks = self.parser.callbacks();
        (hooks.palette_fg, hooks.palette_bg)
    }

    /// Whether the program made the cursor very visible
    /// (`#{cursor_very_visible}`).
    pub fn cursor_very_visible(&self) -> bool {
        self.parser.screen().cursor_very_visible()
    }

    /// Whether origin mode is on (`#{origin_flag}`).
    pub fn origin_mode(&self) -> bool {
        self.parser.screen().origin_mode()
    }

    /// Whether the cursor blinks, as the program last set it
    /// (`#{cursor_blinking}`).
    pub fn cursor_blinking(&self) -> bool {
        self.parser.screen().cursor_blinking()
    }

    /// The cursor shape the program asked for, as DECSCUSR's `Ps`: 0 (the
    /// terminal's own) until it asks.
    pub fn cursor_style(&self) -> u8 {
        self.parser.screen().cursor_style().decscusr()
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

    /// Each line's text, the oldest of the history first.
    fn lines(emu: &Emulator) -> Vec<String> {
        let screen = emu.screen();
        (0..screen.scrollback_len() + 3)
            .map(|i| {
                screen
                    .line_cells(i)
                    .unwrap()
                    .iter()
                    .map(vt1k::Cell::contents)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn prompt_marks_follow_their_lines_as_the_history_drops_some() {
        let mut emu = Emulator::new(3, 20, 5);
        for i in 0..10 {
            emu.process(format!("\x1b]133;A\x07p{i}\r\n\x1b]133;C\x07o{i}\r\n").as_bytes());
        }
        let text = lines(&emu);
        for (output, first) in [(false, 'p'), (true, 'o')] {
            let marked = emu.prompt_lines(output);
            let expected: Vec<usize> = (0..text.len())
                .filter(|&i| text[i].starts_with(first))
                .collect();
            assert_eq!(marked, expected, "{first}: {text:?}");
        }
        assert!(text.iter().any(|l| l == "p9"), "{text:?}");
        // Clearing the history keeps the marks on the screen where they are.
        emu.clear_history();
        let text = lines(&emu);
        let marked = emu.prompt_lines(false);
        assert!(
            marked.iter().all(|&i| text[i].starts_with('p')),
            "{marked:?} {text:?}"
        );
    }

    #[test]
    fn cursor_colour_and_very_visible_are_kept_as_tmuxs() {
        let mut e = Emulator::new(6, 20, 0);
        assert_eq!((e.cursor_colour(), e.cursor_very_visible()), (None, false));
        e.process(b"\x1b]12;rgb:12/34/56\x07\x1b[4;34l");
        assert_eq!(e.cursor_colour(), Some(0x123456));
        assert!(e.cursor_very_visible());
        e.process(b"\x1b]12;bogus\x07\x1b[34h");
        assert_eq!(e.cursor_colour(), Some(0x123456), "an unread colour");
        assert!(!e.cursor_very_visible());
        e.process(b"\x1b[34l\x1bc");
        assert_eq!(e.cursor_colour(), Some(0x123456), "a reset keeps it");
        assert!(!e.cursor_very_visible(), "a reset ends it");
        e.process(b"\x1b]112\x07");
        assert_eq!(e.cursor_colour(), None);
    }

    #[test]
    fn default_colours_set_by_the_program_are_kept() {
        let mut e = Emulator::new(6, 20, 0);
        e.process(b"\x1b]10;#102030\x07\x1b]11;rgb:aa/bb/cc\x1b\\\x1b]11;bogus\x07");
        assert_eq!(e.palette_colours(), (Some(0x102030), Some(0xaabbcc)));
        e.process(b"\x1b]110\x07");
        assert_eq!(e.palette_colours(), (None, Some(0xaabbcc)));
        e.process(b"\x1bc");
        assert_eq!(
            e.palette_colours(),
            (None, None),
            "a full reset clears them"
        );
    }

    #[test]
    fn history_lines_keep_when_they_went_in() {
        let mut e = Emulator::new(2, 10, 100);
        e.process(b"0123456789\r\nb\r\nc");
        assert_eq!(e.history_size(), 1);
        let went_in = e.line_time(0);
        assert!(went_in > 0);
        assert_eq!(e.line_time(1), 0, "a line of the screen");
        // Narrowed, the line rewraps into two rows of history: its first
        // keeps the time, the row the rewrap made has none, as tmux's.
        e.resize(2, 5);
        assert_eq!(e.history_size(), 2);
        assert_eq!((e.line_time(0), e.line_time(1)), (went_in, 0));
        // Widened again, the rows join and the line keeps its time.
        e.resize(2, 10);
        assert_eq!(e.history_size(), 1);
        assert_eq!(e.line_time(0), went_in);
        // Lines the history loses past its limit take their times away.
        let mut e = Emulator::new(2, 10, 3);
        for _ in 0..10 {
            e.process(b"x\r\n");
        }
        assert_eq!(e.history_size(), 3);
        assert!((0..3).all(|l| e.line_time(l) > 0));
        assert_eq!(e.line_time(3), 0);
    }

    #[test]
    fn a_shrink_stamps_no_line_it_pushes_into_the_history() {
        let mut e = Emulator::new(3, 10, 100);
        e.process(b"a\r\nb\r\nc");
        e.resize(2, 10);
        assert_eq!(e.history_size(), 1);
        assert_eq!(e.line_time(0), 0, "as tmux's screen_resize_y");
        // Taken back and pushed again, a stamped line keeps its time.
        e.process(b"\r\nd");
        let went_in = e.line_time(1);
        assert!(went_in > 0);
        e.resize(3, 10);
        e.resize(2, 10);
        assert_eq!((e.line_time(0), e.line_time(1)), (0, went_in));
    }

    #[test]
    fn a_line_the_screen_takes_back_keeps_its_time() {
        let mut e = Emulator::new(2, 10, 100);
        e.process(b"a\r\nb\r\nc\r\nd");
        assert_eq!(e.history_size(), 2);
        let went_in = e.line_time(1);
        assert!(went_in > 0);
        // Taller, the screen takes `b` back from the history; it keeps its
        // time, as tmux's grid line does.
        e.resize(3, 10);
        assert_eq!(e.history_size(), 1);
        assert_eq!((e.line_time(1), e.line_time(2)), (went_in, 0));
        // Through a rewrap too.
        e.resize(3, 5);
        assert_eq!(e.line_time(1), went_in);
        // Back in the history it is stamped again, and it was not stamped
        // twice before: the history's newest line is still `b`.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        e.process(b"\r\ne");
        assert_eq!(e.history_size(), 2);
        assert!(e.line_time(1) > went_in);
        assert_eq!(e.line_time(2), 0);
        // A full reset's new screen has none of the lines it took back.
        let mut e = Emulator::new(2, 10, 100);
        e.process(b"a\r\nb\r\nc");
        e.resize(3, 10);
        assert!(e.line_time(0) > 0);
        e.process(b"\x1bc");
        assert_eq!((0..3).map(|l| e.line_time(l)).max(), Some(0));
        // Nor of the history it drops.
        e.process(b"a\r\nb\r\nc\r\nd\r\ne");
        assert_eq!(e.history_size(), 2);
        e.process(b"\x1bc");
        assert_eq!((0..3).map(|l| e.line_time(l)).max(), Some(0));
    }

    #[test]
    fn a_prompt_mark_follows_its_line_through_a_reflow() {
        let mut e = Emulator::new(3, 10, 10);
        e.process(b"0123456789abcdefghij\r\n\x1b]133;A\x07$ ");
        assert_eq!(e.prompt_lines(false), [2]);
        // Wider, the wrapped line takes one row: the prompt is on the next.
        e.resize(3, 20);
        assert_eq!(e.prompt_lines(false), [1]);
        assert_eq!(e.screen().contents(), "0123456789abcdefghij\n$ ");
    }

    #[test]
    fn the_cursor_1049_saved_is_kept_as_tmuxs() {
        let mut e = Emulator::new(6, 20, 0);
        assert_eq!(e.alternate_saved(), None);
        e.process(b"ab\r\ncde\x1b[?1049h\x1b[4;7Hx");
        assert_eq!(e.alternate_saved(), Some((1, 3)));
        // Not moved by a switch while on the alternate screen, nor by 47 or
        // 1047, and kept after the switch back.
        e.process(b"\x1b[?1049h\x1b[?1049l\x1b[H\x1b[?47h\x1b[?47l\x1b[?1047h");
        assert_eq!(e.alternate_saved(), Some((1, 3)));
        e.process(b"\x1b[?1047l\x1bc");
        assert_eq!(e.alternate_saved(), Some((1, 3)), "a full reset keeps it");
    }

    #[test]
    fn origin_mode_is_kept_and_still_moves_the_cursor() {
        let mut e = Emulator::new(5, 10, 0);
        assert!(!e.origin_mode());
        // A scroll region from row 2, origin mode on: row 1 is its top.
        e.process(b"\x1b[2;4r\x1b[?6h\x1b[1;1H");
        assert!(e.origin_mode());
        assert_eq!(e.screen().cursor_position(), (1, 0), "vt1k still has it");
        e.process(b"\x1b[?6l");
        assert!(!e.origin_mode());
        e.process(b"\x1b[?6h\x1bc");
        assert!(!e.origin_mode(), "a full reset clears it");
    }

    #[test]
    fn insert_and_wrap_modes_are_kept() {
        let mut e = Emulator::new(4, 20, 0);
        assert_eq!(e.insert_and_wrap(), (false, true));
        e.process(b"\x1b[4h\x1b[?7l");
        assert_eq!(e.insert_and_wrap(), (true, false));
        // With another mode in the same sequence.
        e.process(b"\x1b[?1004;7h\x1b[4l");
        assert_eq!(e.insert_and_wrap(), (false, true));
        assert!(e.focus_reports());
        e.process(b"\x1b[4h\x1b[?7l");
        e.reset();
        assert_eq!(e.insert_and_wrap(), (false, true));
    }

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
        // 1047, which vt1k ignores, switches and clears as 47 does; going
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
    fn growing_takes_back_the_rows_a_shrink_pushed_as_tmux_does() {
        let row = |e: &Emulator, r: u16| e.screen().contents_between(r, 0, r, 20);
        let mut e = Emulator::new(4, 20, 100);
        e.process(b"one\r\ntwo\r\nthree\r\n$ ");
        e.resize(2, 20);
        assert_eq!(e.history_size(), 2);
        e.resize(4, 20);
        assert_eq!(e.history_size(), 0);
        assert_eq!(
            (0..4).map(|r| row(&e, r)).collect::<Vec<_>>(),
            ["one", "two", "three", "$ "]
        );
        assert_eq!(e.screen().cursor_position(), (3, 2));
    }

    #[test]
    fn one_row_wraps_without_a_panic() {
        // vt100 0.16 panicked here, taking the server with it.
        let mut e = Emulator::new(1, 5, 100);
        e.process(b"abcdefgh");
        assert_eq!(e.screen().contents(), "fgh");
        assert_eq!(e.history_size(), 1);
    }

    #[test]
    fn csis_vt1k_lacks_draw_as_tmuxs() {
        // Each row as tmux 3.8 drew the same output in a 20-column pane.
        let mut e = Emulator::new(8, 20, 0);
        e.process(b"\x1b[1;18Hx\x1b[9b");
        e.process(b"\x1b[2;1H12345678901234567890\x1b[ZC");
        e.process(b"\x1b[3;1Hab\x1b[5Zq");
        e.process(b"\x1b[4;3fhvp\x1b[f@");
        e.process(b"\x1b[5;1Hbq\x1b[6`Y");
        e.process(b"\x1b[6;1Hsave\x1b[s\x1b[8;1Hmid\x1b[uR");
        let row = |r: u16| e.screen().contents_between(r, 0, r, 20);
        assert_eq!(row(0), "@                xxx");
        assert_eq!(row(1), "1234567890123456C890");
        assert_eq!(row(2), "qb");
        assert_eq!(row(3), "  hvp");
        assert_eq!(row(4), "bq   Y");
        assert_eq!(row(5), "saveR");
        assert_eq!(row(7), "mid");
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
        // Under `extended-keys off` a request is ignored as it arrives, and
        // a reset under `always` starts at level 1, as tmux's.
        e.set_extended_keys(ExtendedKeys::Off);
        e.process(b"\x1b[>4;1m");
        assert_eq!(e.input_modes().modify_other_keys, 2);
        e.reset_keys();
        assert_eq!(e.input_modes().modify_other_keys, 0);
        e.set_extended_keys(ExtendedKeys::Always);
        e.reset_keys();
        assert_eq!(e.input_modes().modify_other_keys, 1);
        e.set_extended_keys(ExtendedKeys::On);
        e.process(b"\x1b[>4;2m");
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
        // A full reset turns both kinds of extended keys off.
        e.process(b"\x1b[>1u\x1b[>4;2m\x1bc");
        assert_eq!(e.input_modes().kitty_flags, 0, "ESC c");
        assert_eq!(e.input_modes().modify_other_keys, 0, "ESC c");
        e.process(b"\x1b[?1h");
        assert!(e.input_modes().app_cursor);
        // Focus reports, in a sequence with a mode vt1k keeps.
        assert!(!e.focus_reports());
        e.process(b"\x1b[?25;1004h");
        assert!(e.focus_reports());
        e.process(b"\x1b[?1004l");
        assert!(!e.focus_reports());
    }

    /// The queries the server answers are named in order, each with where
    /// its answer falls among the replies the emulator gives itself.
    #[test]
    fn queries_for_the_server_keep_their_place_among_the_replies() {
        let mut e = Emulator::new(4, 20, 0);
        let replies = e.process(b"\x1b]11;?\x07\x1b[c\x1b[?996n\x1b]12;?\x1b\\");
        let da1 = b"\x1b[?62;22c".len();
        assert_eq!(replies.len(), da1);
        assert_eq!(
            e.take_queries(),
            [
                (
                    0,
                    Query::Colour {
                        which: 11,
                        bel: true
                    }
                ),
                (da1, Query::Theme),
                (
                    da1,
                    Query::Colour {
                        which: 12,
                        bel: false
                    }
                ),
            ]
        );
        assert_eq!(e.take_queries(), []);
    }

    /// A palette entry asked for and not set is named with where its
    /// answer falls among the other replies, for the outer terminal.
    #[test]
    fn an_unset_palette_entry_asked_for_is_named_in_place() {
        let mut e = Emulator::new(4, 20, 0);
        let replies = e.process(b"\x1b]4;1;#102030\x07\x1b[5n\x1b]4;1;?;3;?\x07\x1b[c");
        let status = b"\x1b[0n".len();
        let answer = b"\x1b]4;1;rgb:1010/2020/3030\x07".len();
        assert_eq!(&replies[status + answer..], b"\x1b[?62;22c");
        assert_eq!(
            e.take_queries(),
            [(
                status + answer,
                Query::Palette {
                    index: 3,
                    bel: true
                }
            )]
        );
        // The next output's places are in its own replies.
        e.process(b"\x1b[c\x1b]4;3;?\x1b\\");
        assert_eq!(
            e.take_queries(),
            [(
                b"\x1b[?62;22c".len(),
                Query::Palette {
                    index: 3,
                    bel: false
                }
            )]
        );
    }
}
