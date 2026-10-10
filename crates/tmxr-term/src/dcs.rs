//! Take DCS strings (`ESC P … ESC \`) out of a pane's output before `vt100`
//! sees it.
//!
//! vt100 has no DCS hook, and its parser ends a DCS at the first `ESC`. tmux's
//! passthrough (`ESC P tmux; …`, with every `ESC` inside doubled) would then
//! leak its payload onto the screen as text. The splitter runs in front of the
//! parser: ordinary bytes pass through, DCS strings are taken out whole even
//! when they span reads, and `tmux;` payloads come back with their `ESC`s
//! undoubled, ready to forward to the outer terminal.

/// Longest DCS string kept; a longer one (or one that never ends) is dropped
/// rather than buffered without bound. Large enough for inline images.
const MAX_DCS: usize = 32 * 1024 * 1024;

/// Prefix of tmux's passthrough DCS.
const PASSTHROUGH: &[u8] = b"tmux;";

const ESC: u8 = 0x1b;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    /// Saw `ESC` outside a DCS.
    Esc,
    /// Inside a DCS string.
    Dcs,
    /// Saw `ESC` inside a DCS string.
    DcsEsc,
    /// Inside a DCS string that outgrew [`MAX_DCS`]; skipped to its end.
    Skip,
    SkipEsc,
}

/// One read, split.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Split {
    /// Everything but DCS strings, for the terminal parser.
    pub text: Vec<u8>,
    /// Payloads of completed `ESC P tmux; … ESC \` strings, each with how
    /// much of `text` came before it: where in the output it was printed.
    pub passthrough: Vec<(usize, Vec<u8>)>,
}

#[derive(Debug, Default)]
pub struct DcsSplitter {
    state: State,
    dcs: Vec<u8>,
}

impl DcsSplitter {
    pub fn split(&mut self, bytes: &[u8]) -> Split {
        let mut out = Split {
            text: Vec::with_capacity(bytes.len()),
            passthrough: Vec::new(),
        };
        for &b in bytes {
            self.state = match (self.state, b) {
                (State::Ground, ESC) => State::Esc,
                (State::Ground, _) => {
                    out.text.push(b);
                    State::Ground
                }
                (State::Esc, b'P') => {
                    self.dcs.clear();
                    State::Dcs
                }
                (State::Esc, ESC) => {
                    out.text.push(ESC);
                    State::Esc
                }
                (State::Esc, _) => {
                    out.text.extend([ESC, b]);
                    State::Ground
                }
                (State::Dcs, ESC) => State::DcsEsc,
                (State::Dcs, _) => self.keep(&[b]),
                // ESC \ (ST) ends the string.
                (State::DcsEsc, b'\\') => {
                    if let Some(payload) = self.dcs.strip_prefix(PASSTHROUGH) {
                        out.passthrough.push((out.text.len(), payload.to_vec()));
                    }
                    self.dcs.clear();
                    State::Ground
                }
                // A doubled ESC is one ESC of the payload.
                (State::DcsEsc, ESC) => self.keep(&[ESC]),
                // Any other ESC ends the string unfinished, as terminal
                // parsers do, and starts a sequence of its own; a passthrough
                // never contains one, since its ESCs are all doubled.
                (State::DcsEsc, b'P') => {
                    self.dcs.clear();
                    State::Dcs
                }
                (State::DcsEsc, _) => {
                    self.dcs.clear();
                    out.text.extend([ESC, b]);
                    State::Ground
                }
                (State::Skip, ESC) => State::SkipEsc,
                (State::Skip, _) => State::Skip,
                (State::SkipEsc, b'\\') => State::Ground,
                (State::SkipEsc, _) => State::Skip,
            };
        }
        out
    }

    /// Append to the DCS being collected, or give up on an oversized one.
    fn keep(&mut self, bytes: &[u8]) -> State {
        if self.dcs.len() + bytes.len() > MAX_DCS {
            self.dcs = Vec::new();
            return State::Skip;
        }
        self.dcs.extend_from_slice(bytes);
        State::Dcs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split_all(chunks: &[&[u8]]) -> Split {
        let mut s = DcsSplitter::default();
        let mut all = Split::default();
        for c in chunks {
            let part = s.split(c);
            let before = all.text.len();
            all.text.extend(part.text);
            all.passthrough
                .extend(part.passthrough.into_iter().map(|(at, p)| (before + at, p)));
        }
        all
    }

    #[test]
    fn ordinary_output_and_other_escapes_pass_through() {
        let bytes = b"hi \x1b[31mred\x1b[0m \x1b]0;title\x07 \x1b\x1b[A";
        let out = split_all(&[bytes]);
        assert_eq!(out.text, bytes);
        assert!(out.passthrough.is_empty());
    }

    #[test]
    fn tmux_passthrough_is_taken_out_with_escapes_undoubled() {
        let out = split_all(&[b"a\x1bPtmux;\x1b\x1b]52;c;aGk=\x07\x1b\\b"]);
        assert_eq!(out.text, b"ab");
        // After the "a" it followed.
        assert_eq!(out.passthrough, vec![(1, b"\x1b]52;c;aGk=\x07".to_vec())]);
    }

    #[test]
    fn a_dcs_split_across_reads_is_reassembled() {
        let whole: &[u8] = b"x\x1bPtmux;\x1b\x1b_Gf=100;AAAA\x1b\x1b\\\x1b\\y";
        let bytes: Vec<&[u8]> = whole.chunks(1).collect();
        let out = split_all(&bytes);
        assert_eq!(out, split_all(&[whole]));
        assert_eq!(out.text, b"xy");
        assert_eq!(
            out.passthrough,
            vec![(1, b"\x1b_Gf=100;AAAA\x1b\\".to_vec())]
        );
    }

    #[test]
    fn a_lone_escape_ends_an_unterminated_dcs() {
        let out = split_all(&[b"\x1bPtmux;half\x1b[31mred"]);
        assert_eq!(out.text, b"\x1b[31mred");
        assert!(out.passthrough.is_empty());
    }

    #[test]
    fn other_dcs_strings_are_dropped() {
        let out = split_all(&[b"a\x1bP$qm\x1b\\b"]);
        assert_eq!(out.text, b"ab");
        assert!(out.passthrough.is_empty());
    }

    #[test]
    fn an_oversized_dcs_is_skipped_to_its_end() {
        let mut s = DcsSplitter::default();
        s.split(b"\x1bPtmux;");
        let chunk = vec![b'x'; 1024 * 1024];
        for _ in 0..(MAX_DCS / chunk.len() + 1) {
            assert!(s.split(&chunk).text.is_empty());
        }
        assert!(s.dcs.capacity() < MAX_DCS, "buffer released");
        let out = s.split(b"\x1b\\after");
        assert_eq!(out.text, b"after");
        assert!(out.passthrough.is_empty());
    }
}
