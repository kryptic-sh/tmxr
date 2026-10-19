//! `alternate-screen off`: a program's switches to the alternate screen
//! dropped from its output before the emulator sees them, as tmux ignores
//! them (screen_write_alternateon / _off).

/// The DEC private modes that switch screens: 47, 1047 and 1049.
const ALTERNATE_MODES: [u32; 3] = [47, 1047, 1049];

/// The longest mode sequence held back waiting for its end; one longer is
/// passed on as it is.
const MAX_SEQUENCE: usize = 64;

/// Drops the alternate screen modes from DEC private mode sets and resets
/// (`ESC [ ? … h` / `l`), keeping any other mode the same sequence sets. A
/// sequence split across reads is held until its end arrives.
#[derive(Debug, Default)]
pub struct AltScreenFilter {
    pending: Vec<u8>,
}

enum Scan {
    /// Not a DEC private mode set or reset: the ESC is passed on.
    Other,
    /// Cut off by the end of the input.
    Incomplete,
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

impl AltScreenFilter {
    /// `bytes` without the alternate screen modes, after anything held
    /// from the last call.
    pub fn filter(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut input = std::mem::take(&mut self.pending);
        input.extend_from_slice(bytes);
        let mut out = Vec::with_capacity(input.len());
        let mut i = 0;
        while i < input.len() {
            if input[i] != 0x1b {
                out.push(input[i]);
                i += 1;
                continue;
            }
            match scan(&input[i..]) {
                Scan::Other => {
                    out.push(0x1b);
                    i += 1;
                }
                Scan::Incomplete => {
                    self.pending = input[i..].to_vec();
                    break;
                }
                Scan::Mode { len, params, last } => {
                    let kept: Vec<&[u8]> = params
                        .split(|&b| b == b';')
                        .filter(|p| {
                            let n = std::str::from_utf8(p)
                                .ok()
                                .and_then(|p| p.parse::<u32>().ok());
                            !n.is_some_and(|n| ALTERNATE_MODES.contains(&n))
                        })
                        .collect();
                    if !kept.is_empty() {
                        out.extend_from_slice(b"\x1b[?");
                        out.extend_from_slice(&kept.join(&b';'));
                        out.push(last);
                    }
                    i += len;
                }
            }
        }
        out
    }

    /// Whatever is held waiting for its end, given up: the filter is no
    /// longer wanted.
    pub fn flush(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternate_modes_go_and_others_stay() {
        let mut f = AltScreenFilter::default();
        assert_eq!(f.filter(b"a\x1b[?1049hb"), b"ab");
        assert_eq!(f.filter(b"\x1b[?47l\x1b[?1047h"), b"");
        // Another mode in the same sequence is kept.
        assert_eq!(f.filter(b"\x1b[?1049;25h"), b"\x1b[?25h");
        assert_eq!(f.filter(b"\x1b[?25;1047l"), b"\x1b[?25l");
        // Other sequences pass as they are.
        assert_eq!(
            f.filter(b"\x1b[?25h\x1b[2J\x1b]0;t\x07"),
            b"\x1b[?25h\x1b[2J\x1b]0;t\x07"
        );
        assert_eq!(f.filter(b"\x1b[?1049$p"), b"\x1b[?1049$p");
    }

    #[test]
    fn a_sequence_split_across_reads_is_held() {
        let mut f = AltScreenFilter::default();
        assert_eq!(f.filter(b"x\x1b"), b"x");
        assert_eq!(f.filter(b"[?10"), b"");
        assert_eq!(f.filter(b"49hy"), b"y");
        assert_eq!(f.filter(b"\x1b["), b"");
        assert_eq!(f.filter(b"2J"), b"\x1b[2J");
        assert_eq!(f.filter(b"\x1b[?10"), b"");
        assert_eq!(f.flush(), b"\x1b[?10");
    }
}
