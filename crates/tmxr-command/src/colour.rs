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
/// `grayN` from 0 to 100. `default`, `terminal` and anything else give
/// none; so do X11's colour names, which tmux takes and tmxr does not.
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
    let grey =
        strip_prefix_ignore_case(s, "grey").or_else(|| strip_prefix_ignore_case(s, "gray"))?;
    if grey.is_empty() {
        return Some(0xbebebe);
    }
    let percent = grey.parse::<u8>().ok().filter(|p| *p <= 100)?;
    // tmux's round(2.55 * c), in the same double precision.
    let v = (2.55 * f64::from(percent)).round() as u32;
    Some(v << 16 | v << 8 | v)
}

#[cfg(test)]
mod tests {
    use super::*;

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
