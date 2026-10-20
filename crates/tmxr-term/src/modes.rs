//! The sequences the emulator makes as tmux does rather than as vt100
//! would, split from a program's output: the DEC private modes for the
//! alternate screen (47, 1047 and 1049, `Emulator::switch_screen`, dropped
//! for `alternate-screen off`), the mouse (9, 1000 to 1003, 1005 and 1006,
//! `Emulator::mouse_mode`) and origin (6, kept for `#{origin_flag}` and
//! passed on), a full reset (`ESC c`), which clears what the emulator keeps
//! of them, and CSIs from tmux's input_csi_table that vt100 lacks or makes
//! differently: SCP and RCP (`s`, `u`) as `ESC 7` and `ESC 8`, REP (`b`),
//! which repeats only a character printed just before, CBT (`Z`), which
//! needs the emulator's cursor, DECSCUSR (`SP q`), whose shape and
//! blinking the emulator keeps itself, and DECSTBM (`r`), IL (`L`) and DL
//! (`M`), as the emulator keeps the scroll region (vt100 keeps one per
//! screen, and its IL and DL do nothing outside it).

/// The DEC private modes split out.
const SPLIT_MODES: [u32; 11] = [6, 9, 47, 1000, 1001, 1002, 1003, 1005, 1006, 1047, 1049];

/// The longest sequence held back waiting for its end; one longer is
/// passed on as it is.
const MAX_SEQUENCE: usize = 64;

/// The most bytes one UTF-8 character takes.
const MAX_CHAR: usize = 4;

/// A stretch of program output, or one sequence in it the emulator makes
/// itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Text(Vec<u8>),
    /// One of the split modes set (`on`) or reset.
    Mode {
        mode: u32,
        on: bool,
    },
    /// `ESC c`, the terminal's full reset.
    Reset,
    /// REP: the character printed just before, `count` more times.
    Repeat {
        ch: char,
        count: u32,
    },
    /// CBT: the cursor back `count` tab stops.
    BackTab(u32),
    /// DECSCUSR: the cursor shape `Ps` asks for.
    CursorStyle(u32),
    /// DECSTBM: the scroll region's top and bottom rows as written,
    /// counted from 1; `None` where one is missing.
    ScrollRegion(Option<u32>, Option<u32>),
    /// IL: `count` blank lines inserted at the cursor.
    InsertLines(u32),
    /// DL: `count` lines deleted at the cursor.
    DeleteLines(u32),
}

/// Splits the split modes out of DEC private mode sets and resets
/// (`ESC [ ? … h` / `l`), keeping any other mode the same sequence sets, in
/// order, `ESC c` out of the text, and the CSIs the emulator makes. A sequence
/// split across reads is held until its end arrives.
#[derive(Debug, Default)]
pub struct ModeSplitter {
    pending: Vec<u8>,
    /// The last bytes of text since the last control sequence, for REP:
    /// tmux repeats only a character with nothing after it but the REP
    /// (input_print's `INPUT_LAST`, which every control clears).
    tail: Vec<u8>,
}

enum Scan {
    /// Not a sequence split here: its first `len` bytes are passed on.
    Other(usize),
    /// Cut off by the end of the input.
    Incomplete,
    /// `ESC c`.
    Reset,
    /// A CSI `len` bytes long: its private marker (`<` to `?`), its
    /// parameters, its intermediate bytes, and its final byte.
    Csi {
        len: usize,
        marker: Option<u8>,
        params: Vec<u8>,
        intermediates: Vec<u8>,
        last: u8,
    },
}

/// What `bytes`, starting at an ESC, holds.
fn scan(bytes: &[u8]) -> Scan {
    match bytes {
        [_] => Scan::Incomplete,
        [_, b'c', ..] => Scan::Reset,
        [_, b'[', ..] => scan_csi(bytes),
        // A string's body follows, and ends at a BEL or an `ESC \`.
        [_, b']' | b'P' | b'_' | b'^' | b'X', ..] => Scan::Other(2),
        // An escape: intermediates, then its final byte.
        _ => {
            for (i, &b) in bytes.iter().enumerate().skip(1) {
                match b {
                    0x20..=0x2f if i < MAX_SEQUENCE => {}
                    0x30..=0x7e => return Scan::Other(i + 1),
                    _ => return Scan::Other(1),
                }
            }
            Scan::Incomplete
        }
    }
}

/// What `bytes`, starting at `ESC [`, holds.
fn scan_csi(bytes: &[u8]) -> Scan {
    let mut i = 2;
    let marker = match bytes.get(i) {
        Some(&m @ b'<'..=b'?') => {
            i += 1;
            Some(m)
        }
        _ => None,
    };
    let start = i;
    while matches!(bytes.get(i), Some(b'0'..=b'9' | b';' | b':')) {
        i += 1;
    }
    let end = i;
    while matches!(bytes.get(i), Some(0x20..=0x2f)) {
        i += 1;
    }
    match bytes.get(i) {
        _ if i >= MAX_SEQUENCE => Scan::Other(1),
        None => Scan::Incomplete,
        Some(&last @ 0x40..=0x7e) => Scan::Csi {
            len: i + 1,
            marker,
            params: bytes[start..end].to_vec(),
            intermediates: bytes[end..i].to_vec(),
            last,
        },
        Some(_) => Scan::Other(1),
    }
}

/// A CSI's first parameter as tmux's input_get reads a count: 1 if it is
/// missing or 0, `None` (the sequence is ignored) if it is not a number.
fn count(params: &[u8]) -> Option<u32> {
    let first = params.split(|&b| b == b';').next().unwrap_or_default();
    if first.is_empty() {
        return Some(1);
    }
    let n: u32 = std::str::from_utf8(first).ok()?.parse().ok()?;
    Some(n.max(1))
}

/// A CSI's first parameter as tmux's input_get reads a value: 0 if it is
/// missing, `None` (the sequence is ignored) if it is not a number.
fn value(params: &[u8]) -> Option<u32> {
    let first = params.split(|&b| b == b';').next().unwrap_or_default();
    if first.is_empty() {
        return Some(0);
    }
    std::str::from_utf8(first).ok()?.parse().ok()
}

/// DECSTBM's two parameters, each `None` if it is missing; `None` (the
/// sequence is ignored) if either is not a number, as tmux's input_get.
fn region(params: &[u8]) -> Option<(Option<u32>, Option<u32>)> {
    let mut parts = params.split(|&b| b == b';');
    let mut next = || match parts.next() {
        None | Some([]) => Some(None),
        Some(p) => std::str::from_utf8(p).ok()?.parse().ok().map(Some),
    };
    Some((next()?, next()?))
}

/// The character `tail` ends with, unless it ends with a control or a
/// partial character.
fn last_char(tail: &[u8]) -> Option<char> {
    (0..tail.len())
        .rev()
        .find_map(|start| std::str::from_utf8(&tail[start..]).ok())
        .and_then(|s| s.chars().next_back())
        .filter(|c| !c.is_control())
}

/// Adds `bytes` to the text at the end of `pieces`.
fn push_text(pieces: &mut Vec<Piece>, bytes: &[u8]) {
    if let Some(Piece::Text(text)) = pieces.last_mut() {
        text.extend_from_slice(bytes);
    } else if !bytes.is_empty() {
        pieces.push(Piece::Text(bytes.to_vec()));
    }
}

/// The modes `kept` as one sequence ending in `last`, if there are any.
fn push_modes(pieces: &mut Vec<Piece>, kept: &mut Vec<&[u8]>, last: u8) {
    if kept.is_empty() {
        return;
    }
    let mut seq = b"\x1b[?".to_vec();
    seq.extend_from_slice(&kept.join(&b';'));
    seq.push(last);
    push_text(pieces, &seq);
    kept.clear();
}

/// The modes of a DEC private mode set or reset (`last`, `h` or `l`), each
/// split mode as its own piece.
fn split_modes(pieces: &mut Vec<Piece>, params: &[u8], last: u8) {
    let mut kept = Vec::new();
    for p in params.split(|&b| b == b';') {
        let mode = std::str::from_utf8(p)
            .ok()
            .and_then(|p| p.parse::<u32>().ok())
            .filter(|n| SPLIT_MODES.contains(n));
        match mode {
            Some(mode) => {
                push_modes(pieces, &mut kept, last);
                pieces.push(Piece::Mode {
                    mode,
                    on: last == b'h',
                });
            }
            None => kept.push(p),
        }
    }
    push_modes(pieces, &mut kept, last);
}

impl ModeSplitter {
    /// `bytes`, after anything held from the last call, with each split
    /// mode, reset and CSI split out.
    pub fn split(&mut self, bytes: &[u8]) -> Vec<Piece> {
        let mut input = std::mem::take(&mut self.pending);
        input.extend_from_slice(bytes);
        let mut pieces = Vec::new();
        let mut i = 0;
        while i < input.len() {
            let Some(esc) = input[i..].iter().position(|&b| b == 0x1b) else {
                self.text(&mut pieces, &input[i..]);
                break;
            };
            self.text(&mut pieces, &input[i..i + esc]);
            i += esc;
            let len = match scan(&input[i..]) {
                Scan::Incomplete => {
                    self.pending = input[i..].to_vec();
                    break;
                }
                Scan::Other(len) => {
                    push_text(&mut pieces, &input[i..i + len]);
                    len
                }
                Scan::Reset => {
                    pieces.push(Piece::Reset);
                    2
                }
                Scan::Csi {
                    len,
                    marker,
                    params,
                    intermediates,
                    last,
                } => {
                    let seq = &input[i..i + len];
                    match (marker, intermediates.as_slice(), last) {
                        (Some(b'?'), [], b'h' | b'l') if !params.contains(&b':') => {
                            split_modes(&mut pieces, &params, last);
                        }
                        (None, [], b's') => push_text(&mut pieces, b"\x1b7"),
                        (None, [], b'u') => push_text(&mut pieces, b"\x1b8"),
                        (None, [], b'b') => {
                            if let (Some(count), Some(ch)) = (count(&params), last_char(&self.tail))
                            {
                                pieces.push(Piece::Repeat { ch, count });
                            }
                        }
                        (None, [], b'Z') => {
                            if let Some(count) = count(&params) {
                                pieces.push(Piece::BackTab(count));
                            }
                        }
                        (None, b" ", b'q') => {
                            if let Some(ps) = value(&params) {
                                pieces.push(Piece::CursorStyle(ps));
                            }
                        }
                        (None, [], b'r') => {
                            if let Some((top, bottom)) = region(&params) {
                                pieces.push(Piece::ScrollRegion(top, bottom));
                            }
                        }
                        (None, [], b'L') => {
                            if let Some(count) = count(&params) {
                                pieces.push(Piece::InsertLines(count));
                            }
                        }
                        (None, [], b'M') => {
                            if let Some(count) = count(&params) {
                                pieces.push(Piece::DeleteLines(count));
                            }
                        }
                        _ => push_text(&mut pieces, seq),
                    }
                    len
                }
            };
            self.tail.clear();
            i += len;
        }
        pieces
    }

    /// Adds text between sequences to `pieces`, keeping its last bytes.
    fn text(&mut self, pieces: &mut Vec<Piece>, bytes: &[u8]) {
        push_text(pieces, bytes);
        self.tail.extend_from_slice(bytes);
        let cut = self.tail.len().saturating_sub(MAX_CHAR);
        self.tail.drain(..cut);
    }

    /// Whatever is held waiting for its end, given up.
    pub fn flush(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of `pieces` with the switches left out.
    fn text(pieces: Vec<Piece>) -> Vec<u8> {
        pieces
            .into_iter()
            .filter_map(|p| match p {
                Piece::Text(t) => Some(t),
                Piece::Mode { .. }
                | Piece::Reset
                | Piece::Repeat { .. }
                | Piece::BackTab(_)
                | Piece::CursorStyle(_)
                | Piece::ScrollRegion(..)
                | Piece::InsertLines(_)
                | Piece::DeleteLines(_) => None,
            })
            .flatten()
            .collect()
    }

    #[test]
    fn alternate_modes_split_out_and_others_stay() {
        let mut f = ModeSplitter::default();
        assert_eq!(
            f.split(b"a\x1b[?1049hb"),
            vec![
                Piece::Text(b"a".to_vec()),
                Piece::Mode {
                    mode: 1049,
                    on: true
                },
                Piece::Text(b"b".to_vec()),
            ]
        );
        assert_eq!(
            f.split(b"\x1b[?47l\x1b[?1047h"),
            vec![
                Piece::Mode {
                    mode: 47,
                    on: false
                },
                Piece::Mode {
                    mode: 1047,
                    on: true
                },
            ]
        );
        // Another mode in the same sequence is kept, in its place.
        assert_eq!(
            f.split(b"\x1b[?1;1049;25h"),
            vec![
                Piece::Text(b"\x1b[?1h".to_vec()),
                Piece::Mode {
                    mode: 1049,
                    on: true
                },
                Piece::Text(b"\x1b[?25h".to_vec()),
            ]
        );
        assert_eq!(text(f.split(b"\x1b[?25;1047l")), b"\x1b[?25l");
        // Other sequences pass as they are.
        assert_eq!(
            text(f.split(b"\x1b[?25h\x1b[2J\x1b]0;t\x07")),
            b"\x1b[?25h\x1b[2J\x1b]0;t\x07"
        );
        assert_eq!(text(f.split(b"\x1b[?1049$p")), b"\x1b[?1049$p");
    }

    #[test]
    fn mouse_modes_and_a_reset_split_out() {
        let mut f = ModeSplitter::default();
        assert_eq!(
            f.split(b"a\x1b[?1002;1006h\x1bcb"),
            vec![
                Piece::Text(b"a".to_vec()),
                Piece::Mode {
                    mode: 1002,
                    on: true
                },
                Piece::Mode {
                    mode: 1006,
                    on: true
                },
                Piece::Reset,
                Piece::Text(b"b".to_vec()),
            ]
        );
        assert_eq!(
            f.split(b"\x1b[?9;25l"),
            vec![
                Piece::Mode { mode: 9, on: false },
                Piece::Text(b"\x1b[?25l".to_vec()),
            ]
        );
    }

    #[test]
    fn a_sequence_split_across_reads_is_held() {
        let mut f = ModeSplitter::default();
        assert_eq!(text(f.split(b"x\x1b")), b"x");
        assert_eq!(f.split(b"[?10"), vec![]);
        assert_eq!(text(f.split(b"49hy")), b"y");
        assert_eq!(text(f.split(b"\x1b[")), b"");
        assert_eq!(text(f.split(b"2J")), b"\x1b[2J");
        assert_eq!(text(f.split(b"\x1b[?10")), b"");
        assert_eq!(f.flush(), b"\x1b[?10");
    }

    #[test]
    fn csis_the_emulator_makes_split_out() {
        let mut f = ModeSplitter::default();
        assert_eq!(text(f.split(b"\x1b[s\x1b[u")), b"\x1b7\x1b8");
        // With a marker or an intermediate they are other sequences, as
        // are the ones vt100 makes as tmux does.
        assert_eq!(
            text(f.split(b"\x1b[?5s\x1b[>1u\x1b[1 u\x1b[3;5f")),
            b"\x1b[?5s\x1b[>1u\x1b[1 u\x1b[3;5f"
        );
        assert_eq!(
            f.split(b"\x1b[5 q\x1b[ q\x1b[1:2 q"),
            vec![Piece::CursorStyle(5), Piece::CursorStyle(0)]
        );
        assert_eq!(f.split(b"\x1b[2Z"), vec![Piece::BackTab(2)]);
        assert_eq!(
            f.split(b"\x1b[0Z\x1b[Z"),
            vec![Piece::BackTab(1), Piece::BackTab(1)]
        );
        // A count that is not a number is ignored, as tmux's input_get.
        assert_eq!(f.split(b"\x1b[1:2Z"), vec![]);
        assert_eq!(
            f.split(b"\x1b[3;5r\x1b[r\x1b[;7r\x1b[2r\x1b[1:2;5r"),
            vec![
                Piece::ScrollRegion(Some(3), Some(5)),
                Piece::ScrollRegion(None, None),
                Piece::ScrollRegion(None, Some(7)),
                Piece::ScrollRegion(Some(2), None),
            ]
        );
        assert_eq!(
            f.split(b"\x1b[L\x1b[3M\x1b[?5r"),
            vec![
                Piece::InsertLines(1),
                Piece::DeleteLines(3),
                Piece::Text(b"\x1b[?5r".to_vec()),
            ]
        );
    }

    #[test]
    fn rep_repeats_only_a_character_just_printed() {
        let mut f = ModeSplitter::default();
        assert_eq!(
            f.split(b"ab\x1b[3b"),
            vec![
                Piece::Text(b"ab".to_vec()),
                Piece::Repeat { ch: 'b', count: 3 }
            ]
        );
        // A character split across reads.
        assert_eq!(text(f.split("x\u{e9}".as_bytes()[..2].as_ref())), b"x\xc3");
        assert_eq!(
            f.split(b"\xa9\x1b[b")[1],
            Piece::Repeat {
                ch: '\u{e9}',
                count: 1
            }
        );
        // Anything between the character and the REP cancels it: a control,
        // another sequence, or the REP before.
        assert_eq!(text(f.split(b"a\r\x1b[2b")), b"a\r");
        assert_eq!(text(f.split(b"a\x1b[1m\x1b[2b")), b"a\x1b[1m");
        assert_eq!(text(f.split(b"a\x1b7\x1b[2b")), b"a\x1b7");
        assert_eq!(f.split(b"\x1b[2b"), vec![]);
        // A REP split across reads is held.
        assert_eq!(text(f.split(b"z\x1b[")), b"z");
        assert_eq!(f.split(b"4b"), vec![Piece::Repeat { ch: 'z', count: 4 }]);
    }
}
