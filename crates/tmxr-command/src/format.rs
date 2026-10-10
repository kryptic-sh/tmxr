//! tmux formats: `#{…}` expansion and `#[…]` style runs.
//!
//! Supported: `#{name}`, `#{?cond,then,else}`, `#{E:name}` (expand the
//! variable's value as a format again), `#{T:name}` (same), `#{=N:x}` /
//! `#{=-N:x}` (keep the first / last N characters; `#{=/N/marker:x}` marks a
//! cut), `#{==:a,b}`, `#{!=:a,b}`, `#{<:a,b}`, `#{>:a,b}`, `#{<=:a,b}`,
//! `#{>=:a,b}` (compared as strings, as tmux's), `#{m:glob,text}`,
//! `#{m/r:regex,text}` (`i` ignores case), `#{q:x}` (shell-quoted) and
//! `#{q/e:x}` (`#` doubled), `#{||:a,b}`, `#{&&:a,b}`, the short aliases
//! `#S #W #I #P #H #h #F #D #T` and `##` for a literal `#`. Commas inside
//! nested `#{…}` do not split.
//!
//! `#[…]` is left in the expanded text; [`styled`] splits text into styled
//! runs afterwards.

/// Supplies variable values (`session_name`, `@my_option`, …).
pub trait Context {
    fn get(&self, name: &str) -> Option<String>;
}

impl<F: Fn(&str) -> Option<String>> Context for F {
    fn get(&self, name: &str) -> Option<String> {
        self(name)
    }
}

/// Nesting limit for `#{E:…}`, so an option that refers to itself cannot
/// recurse forever.
const MAX_DEPTH: usize = 10;

/// Expand a format string.
pub fn expand(fmt: &str, ctx: &dyn Context) -> String {
    expand_depth(fmt, ctx, 0)
}

fn expand_depth(fmt: &str, ctx: &dyn Context, depth: usize) -> String {
    if depth > MAX_DEPTH {
        return String::new();
    }
    let mut out = String::with_capacity(fmt.len());
    let mut rest = fmt;
    while let Some(at) = rest.find('#') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let mut chars = after.chars();
        match chars.next() {
            Some('{') => match matching_brace(&after[1..]) {
                Some(end) => {
                    out.push_str(&expand_item(&after[1..1 + end], ctx, depth));
                    rest = &after[1 + end + 1..];
                    continue;
                }
                None => {
                    out.push_str(&rest[at..]);
                    return out;
                }
            },
            Some('#') => {
                out.push('#');
                rest = &after[1..];
                continue;
            }
            Some(c) if short_alias(c).is_some() => {
                let name = short_alias(c).unwrap_or_default();
                out.push_str(&ctx.get(name).unwrap_or_default());
                rest = &after[c.len_utf8()..];
                continue;
            }
            _ => {
                out.push('#');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn short_alias(c: char) -> Option<&'static str> {
    Some(match c {
        'S' => "session_name",
        'W' => "window_name",
        'I' => "window_index",
        'P' => "pane_index",
        'H' => "host",
        'h' => "host_short",
        'F' => "window_flags",
        'D' => "pane_id",
        'T' => "pane_title",
        _ => return None,
    })
}

/// Index of the `}` closing a `#{` whose body starts at `s[0]`.
fn matching_brace(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'#' if bytes.get(i + 1) == Some(&b'{') => {
                depth += 1;
                i += 2;
                continue;
            }
            b'}' if depth == 0 => return Some(i),
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split on top-level commas (not inside a nested `#{…}`).
fn split_top(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'#' if bytes.get(i + 1) == Some(&b'{') => {
                depth += 1;
                i += 2;
                continue;
            }
            b'}' if depth > 0 => depth -= 1,
            b',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&s[start..]);
    parts
}

fn truthy(s: &str) -> bool {
    !s.is_empty() && s != "0"
}

fn expand_item(body: &str, ctx: &dyn Context, depth: usize) -> String {
    let ex = |s: &str| expand_depth(s, ctx, depth + 1);
    if let Some(cond) = body.strip_prefix('?') {
        let parts = split_top(cond);
        let test = parts.first().copied().unwrap_or_default();
        // The condition is a variable name or a nested format.
        let value = if test.contains("#{") {
            ex(test)
        } else {
            ctx.get(test).unwrap_or_default()
        };
        let pick = if truthy(&value) { 1 } else { 2 };
        return parts.get(pick).map(|p| ex(p)).unwrap_or_default();
    }
    for (op, f) in [
        ("==:", (|a: &str, b: &str| a == b) as fn(&str, &str) -> bool),
        ("!=:", |a, b| a != b),
        ("||:", |a, b| truthy(a) || truthy(b)),
        ("&&:", |a, b| truthy(a) && truthy(b)),
        // tmux compares with strcmp: as strings, not numbers.
        ("<=:", |a, b| a <= b),
        (">=:", |a, b| a >= b),
        ("<:", |a, b| a < b),
        (">:", |a, b| a > b),
    ] {
        if let Some(rest) = body.strip_prefix(op) {
            let parts = split_top(rest);
            let a = ex(parts.first().copied().unwrap_or_default());
            let b = ex(parts.get(1).copied().unwrap_or_default());
            return if f(&a, &b) { "1" } else { "0" }.to_owned();
        }
    }
    if let Some(name) = body.strip_prefix("E:").or_else(|| body.strip_prefix("T:")) {
        let raw = ctx.get(name).unwrap_or_default();
        return ex(&raw);
    }
    if let Some((flags, rest)) = body.strip_prefix('m').and_then(modifier_args) {
        let parts = split_top(rest);
        let pattern = ex(parts.first().copied().unwrap_or_default());
        let text = ex(parts.get(1).copied().unwrap_or_default());
        return if format_match(flags, &pattern, &text) {
            "1"
        } else {
            "0"
        }
        .to_owned();
    }
    if let Some((flags, inner)) = body.strip_prefix('q').and_then(modifier_args) {
        let text = expand_item(inner, ctx, depth + 1);
        return if flags.contains('e') || flags.contains('h') {
            text.replace('#', "##")
        } else {
            quote_shell(&text)
        };
    }
    // `=N:` keeps the first N characters, `=-N:` the last; `=/N/marker:`
    // also puts `marker` where text was cut off.
    if let Some((args, inner)) = body.strip_prefix('=').and_then(|rest| {
        rest.split_once(':')
            .filter(|(n, _)| n.parse::<i64>().is_ok())
            .or_else(|| modifier_args(rest).filter(|(a, _)| !a.is_empty()))
    }) {
        let mut args = args.split('/');
        let Ok(n) = args.next().unwrap_or_default().parse::<i64>() else {
            return String::new();
        };
        let marker = args.next().unwrap_or_default();
        let text = expand_item(inner, ctx, depth + 1);
        let chars: Vec<char> = text.chars().collect();
        let keep = n.unsigned_abs() as usize;
        if chars.len() <= keep {
            return text;
        }
        return if n >= 0 {
            chars.iter().take(keep).collect::<String>() + marker
        } else {
            marker.to_owned() + &chars[chars.len() - keep..].iter().collect::<String>()
        };
    }
    if body.contains("#{") {
        return ex(body);
    }
    ctx.get(body).unwrap_or_default()
}

/// A modifier's `/`-separated arguments and what follows its `:`, for
/// `rest` after the modifier's letter: `:x` gives no arguments, `/ri:x`
/// gives `ri`. `None` when `rest` is not a modifier at all (a variable whose
/// name starts with the letter).
fn modifier_args(rest: &str) -> Option<(&str, &str)> {
    if let Some(after) = rest.strip_prefix(':') {
        return Some(("", after));
    }
    rest.strip_prefix('/')?.split_once(':')
}

/// tmux's `m:`: `pattern` as fnmatch(3) with no flags against all of
/// `text`, or with `r` as an extended regular expression found anywhere in
/// it; `i` ignores case. A pattern that does not compile matches nothing.
fn format_match(flags: &str, pattern: &str, text: &str) -> bool {
    let regex = if flags.contains('r') {
        pattern.to_owned()
    } else {
        glob_regex(pattern)
    };
    regex::RegexBuilder::new(&regex)
        .case_insensitive(flags.contains('i'))
        .build()
        .is_ok_and(|r| r.is_match(text))
}

/// fnmatch(3) with no flags as an anchored regex: `*` any run (`/`
/// included), `?` any character, `[…]` a class (`[!…]` negated, a leading
/// `]` literal), `\x` the character x; anything else literal. An unclosed
/// `[` is a literal `[`.
fn glob_regex(glob: &str) -> String {
    let lit = |c: char| regex::escape(c.encode_utf8(&mut [0; 4]));
    let chars: Vec<char> = glob.chars().collect();
    let mut out = String::from("^");
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            '\\' if i + 1 < chars.len() => {
                i += 1;
                out.push_str(&lit(chars[i]));
            }
            '[' => {
                let mut j = i + 1;
                let negate = chars.get(j) == Some(&'!');
                if negate {
                    j += 1;
                }
                let first = j;
                while j < chars.len() && (chars[j] != ']' || j == first) {
                    j += 1;
                }
                if j >= chars.len() {
                    out.push_str(&lit('['));
                } else {
                    out.push('[');
                    if negate {
                        out.push('^');
                    }
                    for &c in &chars[first..j] {
                        if c == '-' {
                            out.push('-');
                        } else {
                            out.push_str(&lit(c));
                        }
                    }
                    out.push(']');
                    i = j;
                }
            }
            c => out.push_str(&lit(c)),
        }
        i += 1;
    }
    out.push('$');
    out
}

/// tmux's `q:`: a backslash before each character a shell would read
/// specially.
fn quote_shell(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if "|&;<>()$`\\\"'*?[# =%".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A colour in a `#[…]` style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    Default,
    /// 0–255 palette index (`colour123`, `red` = 1, `brightred` = 9).
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Attributes set by a `#[…]` run; `None` leaves the base style alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Colour>,
    pub bg: Option<Colour>,
    pub bold: Option<bool>,
    pub dim: Option<bool>,
    pub italic: Option<bool>,
    pub underscore: Option<bool>,
    pub reverse: Option<bool>,
}

impl Style {
    /// Apply a style string such as `fg=#7aa2f7,bg=default,bold`.
    pub fn apply(&mut self, spec: &str) {
        for item in spec.split([',', ' ']).filter(|s| !s.is_empty()) {
            if let Some(c) = item.strip_prefix("fg=") {
                self.fg = parse_colour(c);
            } else if let Some(c) = item.strip_prefix("bg=") {
                self.bg = parse_colour(c);
            } else if item == "default" || item == "none" {
                *self = Self {
                    fg: Some(Colour::Default),
                    bg: Some(Colour::Default),
                    ..Self::default()
                };
                self.bold = Some(false);
                self.dim = Some(false);
                self.italic = Some(false);
                self.underscore = Some(false);
                self.reverse = Some(false);
            } else {
                let (on, name) = match item.strip_prefix("no") {
                    Some(n) => (false, n),
                    None => (true, item),
                };
                match name {
                    "bold" | "bright" => self.bold = Some(on),
                    "dim" => self.dim = Some(on),
                    "italics" | "italic" => self.italic = Some(on),
                    "underscore" | "underline" => self.underscore = Some(on),
                    "reverse" => self.reverse = Some(on),
                    _ => {}
                }
            }
        }
    }
}

/// Parse a tmux colour: `default`, `#rrggbb`, `colourN` / `colorN`, a basic
/// name (`red`) or a bright one (`brightred`).
pub fn parse_colour(s: &str) -> Option<Colour> {
    const NAMES: [&str; 8] = [
        "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
    ];
    if s == "default" || s == "terminal" {
        return Some(Colour::Default);
    }
    if let Some(hex) = s.strip_prefix('#')
        && hex.len() == 6
        && let Ok(v) = u32::from_str_radix(hex, 16)
    {
        return Some(Colour::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8));
    }
    if let Some(n) = s
        .strip_prefix("colour")
        .or_else(|| s.strip_prefix("color"))
        .and_then(|n| n.parse::<u8>().ok())
    {
        return Some(Colour::Indexed(n));
    }
    if let Some(i) = NAMES.iter().position(|n| *n == s) {
        return Some(Colour::Indexed(i as u8));
    }
    if let Some(i) = s
        .strip_prefix("bright")
        .and_then(|b| NAMES.iter().position(|n| *n == b))
    {
        return Some(Colour::Indexed(i as u8 + 8));
    }
    None
}

/// Split expanded text into `(style, text)` runs. Each `#[…]` updates the
/// running style; text before the first one has the default (empty) style.
pub fn styled(text: &str) -> Vec<(Style, String)> {
    let mut runs = Vec::new();
    let mut style = Style::default();
    let mut rest = text;
    while let Some(at) = rest.find("#[") {
        if at > 0 {
            runs.push((style, rest[..at].to_owned()));
        }
        let Some(end) = rest[at..].find(']') else {
            break;
        };
        style.apply(&rest[at + 2..at + end]);
        rest = &rest[at + end + 1..];
    }
    if !rest.is_empty() {
        runs.push((style, rest.to_owned()));
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(name: &str) -> Option<String> {
        Some(
            match name {
                "session_name" => "main",
                "window_index" => "2",
                "window_name" => "hjkl",
                "pane_current_path" => "/home/u/src",
                "client_prefix" => "1",
                "zero" => "0",
                "@status_session" => "[#S]",
                "@self" => "#{E:@self}",
                "@sp" => "a b;c#d",
                "@w" => "hjkl",
                _ => return None,
            }
            .to_owned(),
        )
    }

    fn x(f: &str) -> String {
        expand(f, &ctx)
    }

    #[test]
    fn comparisons_matches_quoting_and_cut_markers_are_tmuxs() {
        // Each as tmux 3.6's display-message -p printed it.
        for (format, tmux) in [
            (r"#{>:2,10}", r"1"),
            (r"#{<:a,b}", r"1"),
            (r"#{<=:b,b}", r"1"),
            (r"#{>=:a,b}", r"0"),
            (r"#{m:*foo*,xfooy}", r"1"),
            (r"#{m:f?o,foo}", r"1"),
            (r"#{m:[!a]x,bx}", r"1"),
            (r"#{m:[!a]x,ax}", r"0"),
            (r"#{m:foo,FOO}", r"0"),
            (r"#{m/i:FOO,foo}", r"1"),
            (r"#{m:[a-c]z,bz}", r"1"),
            (r"#{m:a\*,a*}", r"1"),
            (r"#{m:a\*,ab}", r"0"),
            (r"#{m/r:(copy|view)-mode,copy-mode}", r"1"),
            (r"#{m/r:^view,copy-mode}", r"0"),
            (r"#{m/ri:COPY,copy-mode}", r"1"),
            (r"#{q:@sp}", r"a\ b\;c\#d"),
            (r"#{q/e:@sp}", r"a b;c##d"),
            (r"#{=/2/...:@w}", r"hj..."),
            (r"#{=/-2/...:@w}", r"...kl"),
            (r"#{=/9/...:@w}", r"hjkl"),
            (r"#{=2:@w}", r"hj"),
        ] {
            assert_eq!(x(format), tmux, "{format}");
        }
    }

    #[test]
    fn variables_and_aliases() {
        assert_eq!(x("#{pane_current_path}"), "/home/u/src");
        assert_eq!(x("#I:#W #S ##"), "2:hjkl main #");
        assert_eq!(x("#{nope}|"), "|");
        assert_eq!(x("50# off"), "50# off");
    }

    #[test]
    fn conditionals_and_comparisons() {
        assert_eq!(x("#{?client_prefix,red,green}"), "red");
        assert_eq!(x("#{?zero,red,green}"), "green");
        assert_eq!(x("#{?nope,a,b}"), "b");
        assert_eq!(x("#{?client_prefix,#{session_name},x}"), "main");
        assert_eq!(x("#{==:#{window_name},hjkl}"), "1");
        assert_eq!(x("#{!=:#{window_name},hjkl}"), "0");
        assert_eq!(x("#{?#{==:#S,main},yes,no}"), "yes");
        assert_eq!(x("#{&&:#{client_prefix},#{zero}}"), "0");
        assert_eq!(x("#{||:#{client_prefix},#{zero}}"), "1");
    }

    #[test]
    fn nested_expansion_trim_and_recursion_guard() {
        assert_eq!(x("#{E:@status_session}"), "[main]");
        assert_eq!(x("#{=3:pane_current_path}"), "/ho");
        assert_eq!(x("#{=-3:pane_current_path}"), "src");
        assert_eq!(x("#{E:@self}"), "");
    }

    #[test]
    fn styles_split_into_runs() {
        let runs = styled("a#[fg=#7aa2f7,bold]b#[bg=colour3,nobold]c");
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0], (Style::default(), "a".into()));
        assert_eq!(runs[1].0.fg, Some(Colour::Rgb(0x7a, 0xa2, 0xf7)));
        assert_eq!(runs[1].0.bold, Some(true));
        assert_eq!(runs[2].0.bg, Some(Colour::Indexed(3)));
        assert_eq!(runs[2].0.bold, Some(false));
        assert_eq!(runs[2].0.fg, Some(Colour::Rgb(0x7a, 0xa2, 0xf7)));
        assert_eq!(parse_colour("brightred"), Some(Colour::Indexed(9)));
        assert_eq!(parse_colour("nope"), None);
    }
}
