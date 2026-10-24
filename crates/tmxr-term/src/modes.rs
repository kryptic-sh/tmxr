//! The sequences the emulator makes itself rather than leave to vt1k,
//! split from a program's output: a full reset (`ESC c`), which clears the
//! palette the emulator keeps, and the OSCs that set or ask for colours
//! (4, 104, and 10 / 11 / 12 `; ?`), whose answers end as the query did
//! and whose palette entries can be more than vt1k's parser hands on
//! (vte keeps 16 of an OSC's parameters).

/// The longest sequence held back waiting for its end; one longer is
/// passed on as it is.
const MAX_SEQUENCE: usize = 64;

/// A stretch of program output, or one sequence in it the emulator makes
/// itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Text(Vec<u8>),
    /// `ESC c`, the terminal's full reset.
    Reset,
    /// `OSC 10`, `11` or `12` `; ?`: the program asks for the default
    /// foreground, background or cursor colour; `bel` if the query ended in
    /// a BEL (an answer ends as it did).
    ColourQuery {
        which: u8,
        bel: bool,
    },
    /// `OSC 4` (`osc` 4) or `OSC 104` and what follows its number, which
    /// set, ask for or reset palette entries; `bel` as for a colour query.
    Palette {
        osc: u16,
        body: Vec<u8>,
        bel: bool,
    },
}

/// Splits `ESC c` and the colour OSCs out of the text. One split across
/// reads is held until its end arrives.
#[derive(Debug, Default)]
pub struct ModeSplitter {
    pending: Vec<u8>,
}

enum Scan {
    /// Not a sequence split here: its first `len` bytes are passed on.
    Other(usize),
    /// Cut off by the end of the input.
    Incomplete,
    /// `ESC c`.
    Reset,
    /// A colour query `len` bytes long.
    ColourQuery { len: usize, which: u8, bel: bool },
    /// An OSC 4 or 104 `len` bytes long.
    Palette {
        len: usize,
        osc: u16,
        body: Vec<u8>,
        bel: bool,
    },
}

/// After `ESC ]`: an OSC asking for a colour (`10;?`, `11;?` or `12;?`, then
/// a BEL or `ESC \`), the start of one cut off, or `None` for any other.
fn scan_colour_query(rest: &[u8]) -> Option<Scan> {
    for (which, ask) in [(10, b"10;?"), (11, b"11;?"), (12, b"12;?")] {
        let k = rest.len().min(ask.len());
        if rest[..k] != ask[..k] {
            continue;
        }
        if k < ask.len() {
            return Some(Scan::Incomplete);
        }
        let len = 2 + ask.len();
        return match &rest[ask.len()..] {
            [] | [0x1b] => Some(Scan::Incomplete),
            [0x07, ..] => Some(Scan::ColourQuery {
                len: len + 1,
                which,
                bel: true,
            }),
            [0x1b, b'\\', ..] => Some(Scan::ColourQuery {
                len: len + 2,
                which,
                bel: false,
            }),
            _ => None,
        };
    }
    None
}

/// The longest OSC 4 or 104 split out; a longer one is passed on as it is
/// (and changes nothing).
const MAX_PALETTE_OSC: usize = 4096;

/// After `ESC ]`: an OSC 4 or 104 up to its BEL or `ESC \`, the start of
/// one cut off, or `None` for any other OSC (or one another control ends).
fn scan_palette(rest: &[u8]) -> Option<Scan> {
    let (osc, start) = match rest {
        [b'4'] | [b'1'] | [b'1', b'0'] | [b'1', b'0', b'4'] => return Some(Scan::Incomplete),
        [b'4', b';', ..] => (4, 2),
        [b'1', b'0', b'4', b';', ..] => (104, 4),
        [b'1', b'0', b'4', 0x07 | 0x1b, ..] => (104, 3),
        _ => return None,
    };
    for (i, &b) in rest.iter().enumerate().skip(start) {
        let (bel, len) = match (b, rest.get(i + 1)) {
            (0x07, _) => (true, i + 1),
            (0x1b, Some(b'\\')) => (false, i + 2),
            (0x1b, None) => return Some(Scan::Incomplete),
            (0x1b | 0x18 | 0x1a, _) => return None,
            _ => continue,
        };
        return Some(Scan::Palette {
            len: 2 + len,
            osc,
            body: rest[start..i].to_vec(),
            bel,
        });
    }
    (rest.len() < MAX_PALETTE_OSC).then_some(Scan::Incomplete)
}

/// What `bytes`, starting at an ESC, holds.
fn scan(bytes: &[u8]) -> Scan {
    match bytes {
        [_] => Scan::Incomplete,
        [_, b'c', ..] => Scan::Reset,
        // A CSI's parameters hold no ESC; vt1k reads it across reads.
        [_, b'[', ..] => Scan::Other(2),
        [_, b']', rest @ ..] => scan_colour_query(rest)
            .or_else(|| scan_palette(rest))
            .unwrap_or(Scan::Other(2)),
        // A string's body follows, and ends at a BEL or an `ESC \`.
        [_, b'P' | b'_' | b'^' | b'X', ..] => Scan::Other(2),
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

/// Adds `bytes` to the text at the end of `pieces`.
fn push_text(pieces: &mut Vec<Piece>, bytes: &[u8]) {
    if let Some(Piece::Text(text)) = pieces.last_mut() {
        text.extend_from_slice(bytes);
    } else if !bytes.is_empty() {
        pieces.push(Piece::Text(bytes.to_vec()));
    }
}

impl ModeSplitter {
    /// `bytes`, after anything held from the last call, with each reset
    /// and colour OSC split out.
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
                Scan::ColourQuery { len, which, bel } => {
                    pieces.push(Piece::ColourQuery { which, bel });
                    len
                }
                Scan::Palette {
                    len,
                    osc,
                    body,
                    bel,
                } => {
                    pieces.push(Piece::Palette { osc, body, bel });
                    len
                }
            };
            i += len;
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
                Piece::Reset | Piece::ColourQuery { .. } | Piece::Palette { .. } => None,
            })
            .flatten()
            .collect()
    }

    #[test]
    fn palette_oscs_split_out_whole() {
        let mut f = ModeSplitter::default();
        assert_eq!(
            f.split(b"\x1b]4;1;red\x07\x1b]104\x1b\\"),
            vec![
                Piece::Palette {
                    osc: 4,
                    body: b"1;red".to_vec(),
                    bel: true
                },
                Piece::Palette {
                    osc: 104,
                    body: Vec::new(),
                    bel: false
                },
            ]
        );
        // Cut off, it waits for its end.
        assert_eq!(f.split(b"\x1b]10"), vec![]);
        assert_eq!(f.split(b"4;2;3"), vec![]);
        assert_eq!(
            f.split(b"\x07"),
            vec![Piece::Palette {
                osc: 104,
                body: b"2;3".to_vec(),
                bel: true
            }]
        );
        // OSC 1 (a title) and OSC 40 are not palette ones.
        let other = b"\x1b]1;x\x07\x1b]40;y\x07";
        assert_eq!(text(f.split(other)), other);
    }

    #[test]
    fn colour_queries_split_out_with_their_ending() {
        let mut f = ModeSplitter::default();
        assert_eq!(
            f.split(b"a\x1b]11;?\x07b\x1b]10;?\x1b\\"),
            vec![
                Piece::Text(b"a".to_vec()),
                Piece::ColourQuery {
                    which: 11,
                    bel: true
                },
                Piece::Text(b"b".to_vec()),
                Piece::ColourQuery {
                    which: 10,
                    bel: false
                },
            ]
        );
        // Cut off, it waits for its end.
        assert_eq!(f.split(b"\x1b]1"), vec![]);
        assert_eq!(f.split(b"2;?\x1b"), vec![]);
        assert_eq!(
            f.split(b"\\"),
            vec![Piece::ColourQuery {
                which: 12,
                bel: false
            }]
        );
        // Other OSCs, a colour set among them, stay as they are.
        let other = b"\x1b]11;#000000\x07\x1b]10;?x\x07\x1b]0;t\x07";
        assert_eq!(text(f.split(other)), other);
    }

    #[test]
    fn a_reset_splits_out_and_other_sequences_stay() {
        let mut f = ModeSplitter::default();
        assert_eq!(
            f.split(b"a\x1b[?1049h\x1bcb"),
            vec![
                Piece::Text(b"a\x1b[?1049h".to_vec()),
                Piece::Reset,
                Piece::Text(b"b".to_vec()),
            ]
        );
        let other = b"\x1b[?25h\x1b[2J\x1b]0;t\x07\x1b[5 q\x1b[3;5r\x1bP+q\x1b\\";
        assert_eq!(text(f.split(other)), other);
    }

    #[test]
    fn a_sequence_split_across_reads_is_held() {
        let mut f = ModeSplitter::default();
        assert_eq!(text(f.split(b"x\x1b")), b"x");
        assert_eq!(f.split(b"]11;"), vec![]);
        assert_eq!(
            f.split(b"?\x07y"),
            vec![
                Piece::ColourQuery {
                    which: 11,
                    bel: true
                },
                Piece::Text(b"y".to_vec()),
            ]
        );
        // A CSI cut off passes as it comes; vt1k reads it across reads.
        assert_eq!(text(f.split(b"\x1b[")), b"\x1b[");
        assert_eq!(text(f.split(b"2J")), b"2J");
        assert_eq!(text(f.split(b"\x1b]1")), b"");
        assert_eq!(f.flush(), b"\x1b]1");
    }
}
