//! Colours by name as tmux 3.6's `colour.c` reads them, for the `c:` format
//! modifier.

/// The 16 colours tmux's colour_256toRGB starts its table with.
const BASIC: [u32; 16] = [
    0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xc0c0c0, 0x808080,
    0xff0000, 0x00ff00, 0xffff00, 0x0000ff, 0xff00ff, 0x00ffff, 0xffffff,
];

/// The levels of each channel in the 6×6×6 cube that follows them.
const CUBE: [u32; 6] = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];

/// The first of the 24 greys that end the table, and the step between them.
const GREY_START: u32 = 0x08;
const GREY_STEP: u32 = 0x0a;

/// The colour names tmux takes for 0 to 7, and with `bright` for 90 to 97.
const NAMES: [&str; 8] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
];

/// What colour `n` of the 256 is in RGB: tmux's table, which is xterm's
/// 16 colours, cube and grey ramp.
fn indexed(n: u8) -> u32 {
    match n {
        0..=15 => BASIC[usize::from(n)],
        16..=231 => {
            let i = u32::from(n - 16);
            let level = |c: u32| CUBE[c as usize];
            level(i / 36) << 16 | level(i / 6 % 6) << 8 | level(i % 6)
        }
        232..=255 => {
            let v = GREY_START + GREY_STEP * u32::from(n - 232);
            v << 16 | v << 8 | v
        }
    }
}

/// `s` with ASCII case ignored starts with `prefix`; what follows it.
fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

/// The RGB value of the colour `s` names, as tmux's colour_fromstring then
/// colour_force_rgb: `#rrggbb`, `colourN` / `colorN`, the eight names (or
/// `0` to `7`) and their `bright` forms (or `90` to `97`), and `greyN` /
/// `grayN` from 0 to 100, and X11's colour names ([`named`]). `default`,
/// `terminal` and anything else give none.
pub fn rgb(s: &str) -> Option<u32> {
    if let Some(hex) = s.strip_prefix('#') {
        return (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| u32::from_str_radix(hex, 16).ok())
            .flatten();
    }
    if let Some(n) =
        strip_prefix_ignore_case(s, "colour").or_else(|| strip_prefix_ignore_case(s, "color"))
    {
        return n.parse::<u8>().ok().map(indexed);
    }
    for (i, name) in (0u8..).zip(NAMES) {
        if s.eq_ignore_ascii_case(name) || s == i.to_string() {
            return Some(indexed(i));
        }
        let bright =
            strip_prefix_ignore_case(s, "bright").is_some_and(|n| n.eq_ignore_ascii_case(name));
        if bright || s == (90 + i).to_string() {
            return Some(indexed(8 + i));
        }
    }
    named(s)
}

/// The RGB value of a colour name, as tmux's colour_byname: `grey` / `gray`
/// alone or with a percentage from 0 to 100 (`grey50`), else one of X11's
/// names (`orchid`, `NavyBlue`), either ignoring case. A name starting with
/// grey or gray is only ever read as one of those, as in tmux.
pub fn named(s: &str) -> Option<u32> {
    if let Some(grey) =
        strip_prefix_ignore_case(s, "grey").or_else(|| strip_prefix_ignore_case(s, "gray"))
    {
        if grey.is_empty() {
            return Some(0xbebebe);
        }
        let percent = grey.parse::<u8>().ok().filter(|p| *p <= 100)?;
        // tmux's round(2.55 * c), in the same double precision.
        let v = (2.55 * f64::from(percent)).round() as u32;
        return Some(v << 16 | v << 8 | v);
    }
    crate::x11_colours::X11_COLOURS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(s))
        .map(|&(_, v)| v)
}

/// The RGB value of an X11 colour specification, as tmux's
/// colour_parseX11 reads one (OSC 12 and the other OSC colours):
/// `rgb:rr/gg/bb`, `#rrggbb`, `r,g,b`, `rgb:rrrr/gggg/bbbb`, `#rrrrggggbbbb`,
/// `cmyk:c/m/y/k` or `cmy:c/m/y` (each from 0 to 1), else a name as
/// [`named`] reads it, spaces around it ignored. The first five take their
/// numbers as tmux's `sscanf` does: each hex field up to its width, and
/// anything after the last ignored.
pub fn parse_x11(p: &str) -> Option<u32> {
    let join = |r: u32, g: u32, b: u32| (r & 0xff) << 16 | (g & 0xff) << 8 | (b & 0xff);
    let short = match p.len() {
        12 => p
            .strip_prefix("rgb:")
            .and_then(|f| hex_fields(f, Some('/'), 2)),
        7 => p.strip_prefix('#').and_then(|f| hex_fields(f, None, 2)),
        _ => None,
    };
    if let Some([r, g, b]) = short.or_else(|| decimal_fields(p)) {
        return Some(join(r, g, b));
    }
    let long = match p.len() {
        18 => p
            .strip_prefix("rgb:")
            .and_then(|f| hex_fields(f, Some('/'), 4)),
        13 => p.strip_prefix('#').and_then(|f| hex_fields(f, None, 4)),
        _ => None,
    };
    if let Some([r, g, b]) = long {
        return Some(join(r >> 8, g >> 8, b >> 8));
    }
    let cmyk = p
        .strip_prefix("cmyk:")
        .and_then(float_fields::<4>)
        .or_else(|| {
            let [c, m, y] = float_fields::<3>(p.strip_prefix("cmy:")?)?;
            Some([c, m, y, 0.0])
        });
    if let Some([c, m, y, k]) = cmyk {
        if [c, m, y, k].iter().all(|v| (0.0..=1.0).contains(v)) {
            // C's double to unsigned char conversion: truncated.
            let channel = |v: f64| ((1.0 - v) * (1.0 - k) * 255.0) as u32;
            return Some(join(channel(c), channel(m), channel(y)));
        }
        return None;
    }
    named(p.trim_matches(' '))
}

/// Three hex fields of up to `width` digits each (at least one), as
/// `sscanf`'s `%0Nx`, with `sep` between them; what follows is ignored.
fn hex_fields(mut s: &str, sep: Option<char>, width: usize) -> Option<[u32; 3]> {
    let mut out = [0; 3];
    for (i, field) in out.iter_mut().enumerate() {
        if i > 0
            && let Some(sep) = sep
        {
            s = s.strip_prefix(sep)?;
        }
        let digits = s
            .bytes()
            .take(width)
            .take_while(u8::is_ascii_hexdigit)
            .count();
        *field = u32::from_str_radix(s.get(..digits).filter(|d| !d.is_empty())?, 16).ok()?;
        s = &s[digits..];
    }
    Some(out)
}

/// `sscanf`'s `%d,%d,%d`: three decimal numbers, each after optional spaces
/// and a sign, with commas between; what follows is ignored. Each is kept
/// as its low byte, as tmux passes them on as `u_char`.
fn decimal_fields(mut s: &str) -> Option<[u32; 3]> {
    let mut out = [0; 3];
    for (i, field) in out.iter_mut().enumerate() {
        if i > 0 {
            s = s.strip_prefix(',')?;
        }
        s = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
        let sign = usize::from(s.starts_with(['+', '-']));
        let digits = s[sign..].bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let n: i64 = s[..sign + digits].parse().unwrap_or(i64::MAX);
        *field = (n & 0xff) as u32;
        s = &s[sign + digits..];
    }
    Some(out)
}

/// `N` numbers split by `/`, as `sscanf`'s `%lf` reads each (the longest
/// start of it that is a number); what follows the last is ignored.
fn float_fields<const N: usize>(s: &str) -> Option<[f64; N]> {
    let mut out = [0.0; N];
    let mut rest = s;
    for (i, field) in out.iter_mut().enumerate() {
        if i > 0 {
            rest = rest.strip_prefix('/')?;
        }
        let end = (1..=rest.len())
            .rev()
            .find(|&n| rest.is_char_boundary(n) && rest[..n].trim_start().parse::<f64>().is_ok())?;
        *field = rest[..end].trim_start().parse().ok()?;
        rest = &rest[end..];
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x11_specifications_read_as_tmuxs() {
        // Each from tmux 3.6, given as OSC 12 and read back as
        // #{cursor_colour} (2026-10-22).
        assert_eq!(parse_x11("red"), Some(0xff0000));
        assert_eq!(parse_x11("rgb:12/34/56"), Some(0x123456));
        assert_eq!(parse_x11("#abcdef"), Some(0xabcdef));
        assert_eq!(parse_x11("rgb:1234/5678/9abc"), Some(0x12569a));
        assert_eq!(parse_x11("Navy Blue"), Some(0x000080));
        assert_eq!(parse_x11("1,2,3"), Some(0x010203));
        assert_eq!(parse_x11("cmy:0/1/0"), Some(0xff00ff));
        assert_eq!(parse_x11("bogus"), None);
        assert_eq!(parse_x11("256,0,-1"), Some(0x0000ff));
        assert_eq!(parse_x11(" red "), Some(0xff0000));
        assert_eq!(parse_x11("grey50"), Some(0x7f7f7f));
        assert_eq!(parse_x11("cmyk:0/0/0/0.5"), Some(0x7f7f7f));
        assert_eq!(parse_x11("rgb:1/2/3"), None);
        assert_eq!(parse_x11("rgb:a/bb/ccc"), Some(0x0abbcc));
        assert_eq!(parse_x11("#12345"), None);
        assert_eq!(parse_x11("2,3"), None);
        assert_eq!(parse_x11("cmyk:2/0/0/0"), None);
        assert_eq!(parse_x11("#abcdefabcdef"), Some(0xabefcd));
        assert_eq!(parse_x11(" 1, 2,3x"), Some(0x010203));
        assert_eq!(parse_x11("cmy:0.5/0.25/1e0"), Some(0x7fbf00));
        assert_eq!(parse_x11("RED"), Some(0xff0000));
        assert_eq!(parse_x11("DarkSlateGrey"), Some(0x2f4f4f));
    }

    #[test]
    fn colours_read_as_tmuxs_table() {
        // Values from tmux 3.6's colour_256toRGB table.
        assert_eq!(rgb("colour0"), Some(0x000000));
        assert_eq!(rgb("colour7"), Some(0xc0c0c0));
        assert_eq!(rgb("colour16"), Some(0x000000));
        assert_eq!(rgb("colour17"), Some(0x00005f));
        assert_eq!(rgb("colour110"), Some(0x87afd7));
        assert_eq!(rgb("colour231"), Some(0xffffff));
        assert_eq!(rgb("colour232"), Some(0x080808));
        assert_eq!(rgb("COLOR255"), Some(0xeeeeee));
        assert_eq!(rgb("colour256"), None);
        // X11's names, as tmux 3.6's colour_byname, ignoring case.
        assert_eq!(rgb("orchid"), Some(0xda70d6));
        assert_eq!(rgb("NAVYBLUE"), Some(0x000080));
        assert_eq!(rgb("navy blue"), Some(0x000080), "tmux's table has both");
        assert_eq!(rgb("not a colour"), None);
        assert_eq!(rgb("greyish"), None, "a grey name is only a grey");
        assert_eq!(rgb("red"), Some(0x800000));
        assert_eq!(rgb("1"), Some(0x800000));
        assert_eq!(rgb("BrightRed"), Some(0xff0000));
        assert_eq!(rgb("91"), Some(0xff0000));
        assert_eq!(rgb("#ABCdef"), Some(0xabcdef));
        assert_eq!(rgb("#abcde"), None);
        assert_eq!(rgb("grey"), Some(0xbebebe));
        // 2.55 is a little under in double precision, as in tmux.
        assert_eq!(rgb("gray50"), Some(0x7f7f7f));
        assert_eq!(rgb("grey101"), None);
        assert_eq!(rgb("default"), None);
        assert_eq!(rgb("terminal"), None);
        assert_eq!(rgb("8"), None);
    }
}
