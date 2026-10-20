//! The DEC private modes the emulator makes as tmux does rather than as
//! vt100 would, split from a program's output: the alternate screen (47,
//! 1047 and 1049, `Emulator::switch_screen`, dropped for `alternate-screen
//! off`) and the mouse (9, 1000 to 1003, 1005 and 1006,
//! `Emulator::mouse_mode`), and a full reset (`ESC c`), which clears what
//! the emulator keeps of them.

/// The DEC private modes split out.
const SPLIT_MODES: [u32; 10] = [9, 47, 1000, 1001, 1002, 1003, 1005, 1006, 1047, 1049];

/// The longest mode sequence held back waiting for its end; one longer is
/// passed on as it is.
const MAX_SEQUENCE: usize = 64;

/// A stretch of program output, or one mode change or reset in it.
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
}

/// Splits the split modes out of DEC private mode sets and resets
/// (`ESC [ ? … h` / `l`), keeping any other mode the same sequence sets, in
/// order, and `ESC c` out of the text. A sequence split across reads is
/// held until its end arrives.
#[derive(Debug, Default)]
pub struct ModeSplitter {
    pending: Vec<u8>,
}

enum Scan {
    /// Not a DEC private mode set or reset: the ESC is passed on.
    Other,
    /// Cut off by the end of the input.
    Incomplete,
    /// `ESC c`.
    Reset,
    /// One `len` bytes long, its parameters and final byte.
    Mode {
        len: usize,
        params: Vec<u8>,
        last: u8,
    },
}

/// What `bytes`, starting at an ESC, holds.
fn scan(bytes: &[u8]) -> Scan {
    match bytes {
        [_] | [_, b'['] => return Scan::Incomplete,
        [_, b'c', ..] => return Scan::Reset,
        [_, b'[', b'?', ..] => {}
        _ => return Scan::Other,
    }
    for (i, &b) in bytes.iter().enumerate().skip(3) {
        if i >= MAX_SEQUENCE {
            return Scan::Other;
        }
        match b {
            b'0'..=b'9' | b';' => {}
            b'h' | b'l' => {
                return Scan::Mode {
                    len: i + 1,
                    params: bytes[3..i].to_vec(),
                    last: b,
                };
            }
            _ => return Scan::Other,
        }
    }
    Scan::Incomplete
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

impl ModeSplitter {
    /// `bytes`, after anything held from the last call, with each split
    /// mode and reset split out.
    pub fn split(&mut self, bytes: &[u8]) -> Vec<Piece> {
        let mut input = std::mem::take(&mut self.pending);
        input.extend_from_slice(bytes);
        let mut pieces = Vec::new();
        let mut i = 0;
        while i < input.len() {
            let Some(esc) = input[i..].iter().position(|&b| b == 0x1b) else {
                push_text(&mut pieces, &input[i..]);
                break;
            };
            push_text(&mut pieces, &input[i..i + esc]);
            i += esc;
            match scan(&input[i..]) {
                Scan::Other => {
                    push_text(&mut pieces, &[0x1b]);
                    i += 1;
                }
                Scan::Incomplete => {
                    self.pending = input[i..].to_vec();
                    break;
                }
                Scan::Reset => {
                    pieces.push(Piece::Reset);
                    i += 2;
                }
                Scan::Mode { len, params, last } => {
                    let mut kept = Vec::new();
                    for p in params.split(|&b| b == b';') {
                        let mode = std::str::from_utf8(p)
                            .ok()
                            .and_then(|p| p.parse::<u32>().ok())
                            .filter(|n| SPLIT_MODES.contains(n));
                        match mode {
                            Some(mode) => {
                                push_modes(&mut pieces, &mut kept, last);
                                pieces.push(Piece::Mode {
                                    mode,
                                    on: last == b'h',
                                });
                            }
                            None => kept.push(p),
                        }
                    }
                    push_modes(&mut pieces, &mut kept, last);
                    i += len;
                }
            }
        }
        pieces
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
                Piece::Mode { .. } | Piece::Reset => None,
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
}
