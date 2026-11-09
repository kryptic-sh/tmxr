//! tmux's terminal features, as far as tmxr acts on them: whether a
//! client's terminal has `extkeys` (xterm's modifyOtherKeys), told by its
//! name or by the `terminal-features` option (tmux 3.6's `tty-features.c`,
//! `tty-keys.c` and `tty_term_create` in `tty-term.c`).

/// tmux's terminal features by name (`tty_features`). A name not among them
/// ends the entry it is in, as tmux's `tty_add_features` stops there.
const FEATURES: &[&str] = &[
    "256",
    "bpaste",
    "ccolour",
    "clipboard",
    "hyperlinks",
    "cstyle",
    "extkeys",
    "focus",
    "ignorefkeys",
    "margins",
    "mouse",
    "osc7",
    "overline",
    "rectfill",
    "RGB",
    "sixel",
    "strikethrough",
    "sync",
    "title",
    "usstyle",
];

/// The feature tmxr acts on: xterm's modifyOtherKeys.
const EXTKEYS: &str = "extkeys";

/// The terminals tmux knows by name whose features (`tty_default_features`)
/// include `extkeys`: iTerm2, foot, XTerm and mintty; not tmux or
/// rxvt-unicode.
const EXTKEYS_TERMINALS: &[&str] = &["iTerm2", "foot", "XTerm", "mintty"];

/// The terminal tmux names from an XTVERSION answer (`version`), as
/// `tty_keys_extended_device_attributes` reads it.
fn named_by_version(version: &str) -> Option<&'static str> {
    [
        ("iTerm2 ", "iTerm2"),
        ("tmux ", "tmux"),
        ("XTerm(", "XTerm"),
        ("mintty ", "mintty"),
        ("foot(", "foot"),
    ]
    .into_iter()
    .find(|(prefix, _)| version.starts_with(prefix))
    .map(|(_, name)| name)
}

/// The terminal tmux names from the first parameter of a DA2 answer
/// (`secondary`), as `tty_keys_device_attributes2` reads it (its `case 'M'`,
/// `'T'` and `'U'`).
fn named_by_secondary(secondary: u16) -> Option<&'static str> {
    match secondary {
        77 => Some("mintty"),
        84 => Some("tmux"),
        85 => Some("rxvt-unicode"),
        _ => None,
    }
}

/// The features one `terminal-features` entry gives a terminal whose
/// `TERM` is `term`: none unless its first field, a pattern (`::` in it is
/// a `:`), matches `term` as `fnmatch` would; else the `:`-separated names
/// after it, up to the first that is not a feature, as tmux's
/// `tty_term_create` and `tty_add_features` read it.
fn entry_features<'e>(entry: &'e str, term: &str) -> Vec<&'e str> {
    let (pattern, rest) = pattern_field(entry);
    if !crate::util::glob_regex(&pattern).is_match(term) {
        return Vec::new();
    }
    rest.split(':')
        .map_while(|name| {
            FEATURES
                .iter()
                .find(|f| f.eq_ignore_ascii_case(name))
                .map(|_| name)
        })
        .collect()
}

/// An entry's first field, a doubled `:` read as one, and what follows the
/// `:` that ends it, as tmux's `tty_term_override_next`.
fn pattern_field(entry: &str) -> (String, &str) {
    let mut pattern = String::new();
    let mut chars = entry.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != ':' {
            pattern.push(c);
        } else if chars.next_if(|&(_, c)| c == ':').is_some() {
            pattern.push(':');
        } else {
            return (pattern, &entry[i + 1..]);
        }
    }
    (pattern, "")
}

/// Whether a client's terminal has tmux's `extkeys` feature: by the name
/// its XTVERSION answer (`version`) or its DA2 answer (`secondary`) gives
/// it, each adding its terminal's features as tmux's do (one answer does
/// not overrule the other), or by an entry of `terminal-features`
/// (`entries`) matching its `TERM` (`term`).
pub fn extkeys<'a>(
    term: &str,
    version: Option<&str>,
    secondary: Option<u16>,
    entries: impl IntoIterator<Item = &'a str>,
) -> bool {
    let named = [
        version.and_then(named_by_version),
        secondary.and_then(named_by_secondary),
    ];
    named
        .into_iter()
        .flatten()
        .any(|name| EXTKEYS_TERMINALS.contains(&name))
        || entries.into_iter().any(|entry| {
            entry_features(entry, term)
                .iter()
                .any(|f| f.eq_ignore_ascii_case(EXTKEYS))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// tmux's default `terminal-features`.
    const TMUX_DEFAULT: &[&str] = &[
        "xterm*:clipboard:ccolour:cstyle:focus:title",
        "screen*:title",
        "rxvt*:ignorefkeys",
    ];

    fn by_name(version: &str) -> bool {
        extkeys(
            "xterm-256color",
            Some(version),
            None,
            TMUX_DEFAULT.iter().copied(),
        )
    }

    #[test]
    fn xtversion_names_tmux_gives_extkeys_to() {
        for version in ["XTerm(390)", "foot(1.20.2)", "mintty 3.7.0", "iTerm2 3.5.0"] {
            assert!(by_name(version), "{version}");
        }
        // tmux itself, tmxr, and names only like the ones tmux knows.
        for version in [
            "tmux 3.6",
            "tmxr 0.15.0",
            "kitty(0.39.1)",
            "WezTerm 20240203",
            "xterm(390)",
            "XTerm 390",
            "foot 1.20",
            "mintty(3.7.0)",
            "iTerm2(3.5)",
            "",
        ] {
            assert!(!by_name(version), "{version}");
        }
    }

    #[test]
    fn da2_gives_extkeys_to_mintty_alone() {
        let da2 = |secondary| extkeys("xterm", None, Some(secondary), []);
        assert!(da2(77));
        // tmux, rxvt-unicode, xterm's VT420 and others.
        for secondary in [84, 85, 41, 1, 0, 65] {
            assert!(!da2(secondary), "{secondary}");
        }
        // Each answer adds its terminal's features, as tmux's: a DA2 of 77
        // gives mintty's whatever XTVERSION names.
        assert!(extkeys("xterm", Some("tmux 3.6"), Some(77), []));
        assert!(extkeys("xterm", Some("XTerm(390)"), Some(84), []));
        assert!(!extkeys("xterm", Some("tmux 3.6"), Some(84), []));
        assert!(extkeys("xterm", Some("kitty(0.39)"), Some(77), []));
        assert!(!extkeys("xterm", None, None, []));
    }

    #[test]
    fn terminal_features_match_term_as_fnmatch() {
        let entries = ["foo*:extkeys", "bar?:title:EXTKEYS", "[ab]z:extkeys"];
        let has = |term| extkeys(term, None, None, entries);
        assert!(has("foo"));
        assert!(has("foo-256color"));
        assert!(has("bar1"));
        assert!(has("az"));
        assert!(!has("xfoo"));
        assert!(!has("bar"));
        assert!(!has("bar12"));
        assert!(!has("cz"));
        // tmux's defaults give no terminal extkeys by TERM.
        for term in ["xterm-256color", "screen", "rxvt-unicode"] {
            assert!(
                !extkeys(term, None, None, TMUX_DEFAULT.iter().copied()),
                "{term}"
            );
        }
    }

    #[test]
    fn an_unknown_feature_ends_its_entry() {
        assert!(!extkeys("foo", None, None, ["foo:nonsense:extkeys"]));
        assert!(!extkeys("foo", None, None, ["foo:title::extkeys"]));
        assert!(extkeys("foo", None, None, ["foo:title:RGB:extkeys"]));
        // A pattern alone, or nothing at all, gives nothing.
        assert!(!extkeys("foo", None, None, ["foo", "", ":extkeys"]));
    }

    #[test]
    fn a_doubled_colon_is_a_colon_in_the_pattern() {
        assert_eq!(pattern_field("a::b:extkeys"), ("a:b".into(), "extkeys"));
        assert_eq!(pattern_field("a:"), ("a".into(), ""));
        assert_eq!(pattern_field("a"), ("a".into(), ""));
        assert!(extkeys("a:b", None, None, ["a::b:extkeys"]));
        assert!(!extkeys("a", None, None, ["a::b:extkeys"]));
    }
}
