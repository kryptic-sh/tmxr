//! A pane's 256-colour palette as a program sets it (OSC 4, reset by OSC
//! 104), as tmux's `colour_palette`: an entry set is drawn in its RGB
//! instead of the outer terminal's colour of that number.

/// The palette's entries set, as RGB; none until one is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Palette(Option<Box<[Option<u32>; 256]>>);

impl Palette {
    /// Entry `n`, where the program set it.
    pub fn get(&self, n: u8) -> Option<u32> {
        self.0.as_ref()?[usize::from(n)]
    }

    fn set(&mut self, n: u8, rgb: Option<u32>) {
        self.0.get_or_insert_with(|| Box::new([None; 256]))[usize::from(n)] = rgb;
    }

    /// Every entry unset.
    pub fn clear(&mut self) {
        self.0 = None;
    }

    /// OSC 4's body (`n;spec;n;spec…`), as tmux's input_osc_4: each spec
    /// sets entry `n`, read as an X11 colour (one it cannot read changes
    /// nothing); a `?` asks for it, answered in `replies` where it is set
    /// ([`answer`], ended by `end`), and otherwise named in `unset` with
    /// where in `replies` its answer belongs, for the outer terminal to be
    /// asked, as tmux does. A number out of range or not followed by `;`
    /// ends the list.
    pub fn osc_4(
        &mut self,
        body: &str,
        end: &str,
        replies: &mut Vec<u8>,
        unset: &mut Vec<(u8, usize)>,
    ) {
        let mut rest = body;
        while !rest.is_empty() {
            let (n, after) = number(rest);
            let Some(after) = after.strip_prefix(';') else {
                return;
            };
            let Some(n) = n.and_then(|n| u8::try_from(n).ok()) else {
                return;
            };
            let (spec, next) = after.split_once(';').unwrap_or((after, ""));
            rest = next;
            if spec == "?" {
                match self.get(n) {
                    Some(rgb) => replies.extend(answer(n, rgb, end)),
                    None => unset.push((n, replies.len())),
                }
            } else if let Some(rgb) = tmxr_command::colour::parse_x11(spec) {
                self.set(n, Some(rgb));
            }
        }
    }

    /// OSC 104's body (`n;n…`), as tmux's input_osc_104: each entry named
    /// is unset. An empty body is handled by the caller, as it resets more
    /// than the palette.
    pub fn osc_104(&mut self, body: &str) {
        let mut rest = body;
        while !rest.is_empty() {
            let (n, after) = number(rest);
            if !(after.is_empty() || after.starts_with(';')) {
                return;
            }
            let Some(n) = n.and_then(|n| u8::try_from(n).ok()) else {
                return;
            };
            self.set(n, None);
            rest = after.strip_prefix(';').unwrap_or(after);
        }
    }
}

/// The answer to a query for entry `n` of colour `rgb`, ended by `end`, as
/// tmux's input_osc_colour_reply writes it: `OSC 4 ; n ; rgb:rrrr/gggg/bbbb`.
pub fn answer(n: u8, rgb: u32, end: &str) -> Vec<u8> {
    let [_, r, g, b] = rgb.to_be_bytes();
    format!("\x1b]4;{n};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}{end}").into_bytes()
}

/// A number at the start of `s` as C's `strtol` reads one (0 where there
/// are no digits; `None` if it does not fit), and what follows it.
fn number(s: &str) -> (Option<i64>, &str) {
    let trimmed = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let sign = usize::from(trimmed.starts_with(['+', '-']));
    let digits = trimmed[sign..]
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    if digits == 0 {
        return (Some(0), s);
    }
    let end = sign + digits;
    (trimmed[..end].parse().ok(), &trimmed[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_4_sets_and_answers_entries_as_tmuxs() {
        let mut p = Palette::default();
        let (mut replies, mut unset) = (Vec::new(), Vec::new());
        p.osc_4(
            "1;#102030;200;red;3;bogus",
            "\x07",
            &mut replies,
            &mut unset,
        );
        assert_eq!(
            (p.get(1), p.get(200), p.get(3)),
            (Some(0x102030), Some(0xff0000), None)
        );
        // Entry 2 is not set: it is named, with where its answer goes.
        p.osc_4("2;?;1;?;5;?", "\x1b\\", &mut replies, &mut unset);
        assert_eq!(replies, b"\x1b]4;1;rgb:1010/2020/3030\x1b\\");
        assert_eq!(unset, [(2, 0), (5, replies.len())]);
        // 256 is out of range: the list ends there.
        p.osc_4("256;#ffffff;2;#ffffff", "\x07", &mut replies, &mut unset);
        assert_eq!(p.get(2), None);
    }

    #[test]
    fn osc_104_unsets_the_entries_named() {
        let mut p = Palette::default();
        p.osc_4(
            "1;red;2;red;3;red",
            "\x07",
            &mut Vec::new(),
            &mut Vec::new(),
        );
        p.osc_104("1;3");
        assert_eq!((p.get(1), p.get(2), p.get(3)), (None, Some(0xff0000), None));
    }
}
