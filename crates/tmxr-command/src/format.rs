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
        // `#{?c1,v1,c2,v2,…,else}`: the value of the first true condition,
        // else the last unpaired part, else nothing, as tmux's.
        let parts = split_top(cond);
        let mut i = 0;
        while i + 1 < parts.len() {
            // A condition is a variable name or a nested format.
            let test = parts[i];
            let value = if test.contains("#{") {
                ex(test)
            } else {
                ctx.get(test).unwrap_or_default()
            };
            if truthy(&value) {
                return ex(parts[i + 1]);
            }
            i += 2;
        }
        return parts.get(i).map(|p| ex(p)).unwrap_or_default();
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
    if let Some((mods, inner)) = value_modifiers(body) {
        return apply_modifiers(&mods, inner, ctx, depth);
    }
    if body.contains("#{") {
        return ex(body);
    }
    ctx.get(body).unwrap_or_default()
}

/// One of tmux's modifiers that change a value, as `format_replace` reads
/// them: `#{b;s/x/y/;=5:name}`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mod<'a> {
    /// `b`: the last part of a path.
    Basename,
    /// `d`: all but the last part of a path.
    Dirname,
    /// `l`: the text itself, not expanded.
    Literal,
    /// `n`: the length in bytes.
    Length,
    /// `s/pattern/replacement/flags`: each match of a regular expression
    /// replaced (`i` ignores case).
    Sub {
        pattern: &'a str,
        replacement: &'a str,
        flags: &'a str,
    },
    /// `=N` keeps the first N characters, `=-N` the last; `=/N/marker`
    /// also puts `marker` where text was cut off.
    Limit(i64, &'a str),
    /// `pN` pads to N characters with spaces after, `p-N` before.
    Pad(i64),
}

/// `body` as a list of value modifiers and what follows their `:`; `None`
/// when it is not one (a plain variable, a condition, a comparison or a
/// modifier this does not read).
fn value_modifiers(body: &str) -> Option<(Vec<Mod<'_>>, &str)> {
    let mut mods = Vec::new();
    let mut rest = body;
    loop {
        let mut chars = rest.chars();
        let letter = chars.next()?;
        let after = chars.as_str();
        // What follows the modifier, up to the `;` or `:` that ends it.
        let (m, tail) = match letter {
            'b' | 'd' | 'l' | 'n' => {
                let m = match letter {
                    'b' => Mod::Basename,
                    'd' => Mod::Dirname,
                    'l' => Mod::Literal,
                    _ => Mod::Length,
                };
                (m, after)
            }
            's' => {
                let sep = after.chars().next().filter(|c| !c.is_alphanumeric())?;
                let mut parts = after[sep.len_utf8()..].splitn(3, sep);
                let pattern = parts.next()?;
                let replacement = parts.next()?;
                let tail = parts.next()?;
                let end = tail.find([';', ':'])?;
                let m = Mod::Sub {
                    pattern,
                    replacement,
                    flags: &tail[..end],
                };
                (m, &tail[end..])
            }
            'p' | '=' => {
                let (args, tail) = match after.chars().next() {
                    Some(c) if c == '-' || c.is_ascii_digit() => {
                        let end = after.find([';', ':'])?;
                        (&after[..end], &after[end..])
                    }
                    Some(sep) if letter == '=' && !sep.is_alphanumeric() && sep != '=' => {
                        let inner = &after[sep.len_utf8()..];
                        let end = inner.find([';', ':'])?;
                        (inner[..end].trim_end_matches(sep), &inner[end..])
                    }
                    _ => return None,
                };
                let mut args = args.split(['/', '|']);
                let n = args.next()?.parse::<i64>().ok()?;
                let m = if letter == 'p' {
                    Mod::Pad(n)
                } else {
                    Mod::Limit(n, args.next().unwrap_or_default())
                };
                (m, tail)
            }
            _ => return None,
        };
        mods.push(m);
        if let Some(inner) = tail.strip_prefix(':') {
            return Some((mods, inner));
        }
        rest = tail.strip_prefix(';')?;
    }
}

/// The value of `inner` with `mods` applied, in the order tmux's
/// `format_replace` applies them whatever order they were written in: the
/// value (a path's parts for a variable read directly, the text itself for
/// `l`), then substitutions, the length limit, padding, and the length.
fn apply_modifiers(mods: &[Mod<'_>], inner: &str, ctx: &dyn Context, depth: usize) -> String {
    let ex = |s: &str| expand_depth(s, ctx, depth + 1);
    let mut value = if mods.contains(&Mod::Literal) {
        inner.to_owned()
    } else if inner.contains("#{") {
        ex(inner)
    } else {
        let found = ctx.get(inner).unwrap_or_default();
        let path = std::path::Path::new(&found);
        let part = |p: Option<&std::ffi::OsStr>| p.map(|p| p.to_string_lossy().into_owned());
        if mods.contains(&Mod::Basename) {
            part(path.file_name()).unwrap_or(found)
        } else if mods.contains(&Mod::Dirname) {
            part(path.parent().map(std::path::Path::as_os_str))
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| ".".to_owned())
        } else {
            found
        }
    };
    for m in mods {
        if let Mod::Sub {
            pattern,
            replacement,
            flags,
        } = m
        {
            value = substitute(&value, &ex(pattern), &ex(replacement), flags);
        }
    }
    for m in mods {
        if let Mod::Limit(n, marker) = m {
            value = limit(&value, *n, &ex(marker));
        }
    }
    for m in mods {
        if let Mod::Pad(n) = m {
            let missing = usize::try_from(n.unsigned_abs())
                .unwrap_or(usize::MAX)
                .saturating_sub(value.chars().count());
            let spaces = " ".repeat(missing);
            value = if *n >= 0 {
                value + &spaces
            } else {
                spaces + &value
            };
        }
    }
    if mods.contains(&Mod::Length) {
        value = value.len().to_string();
    }
    value
}

/// tmux's `s/pattern/replacement/`: every match of the extended regular
/// expression replaced, `\1` and the like naming its groups; `i` in `flags`
/// ignores case. A pattern that does not compile leaves the text alone.
fn substitute(text: &str, pattern: &str, replacement: &str, flags: &str) -> String {
    let Ok(re) = regex::RegexBuilder::new(pattern)
        .case_insensitive(flags.contains('i'))
        .build()
    else {
        return text.to_owned();
    };
    // `\N` as the regex crate writes a group, `$` kept as itself.
    let mut rep = String::new();
    let mut chars = replacement.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek().is_some_and(char::is_ascii_digit) => {
                rep.push_str("${");
                while let Some(d) = chars.next_if(char::is_ascii_digit) {
                    rep.push(d);
                }
                rep.push('}');
            }
            '$' => rep.push_str("$$"),
            c => rep.push(c),
        }
    }
    re.replace_all(text, rep.as_str()).into_owned()
}

/// `=N`: the first N characters of `text` (the last with N negative), with
/// `marker` where text was cut off.
fn limit(text: &str, n: i64, marker: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let keep = usize::try_from(n.unsigned_abs()).unwrap_or(usize::MAX);
    if chars.len() <= keep {
        return text.to_owned();
    }
    if n >= 0 {
        chars.iter().take(keep).collect::<String>() + marker
    } else {
        marker.to_owned() + &chars[chars.len() - keep..].iter().collect::<String>()
    }
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
pub fn glob_regex(glob: &str) -> String {
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
/// A `#[` with no `]` ends the text there, as tmux's `format_draw` stops.
pub fn styled(text: &str) -> Vec<(Style, String)> {
    let mut runs = Vec::new();
    let mut style = Style::default();
    let mut rest = text;
    while let Some(at) = rest.find("#[") {
        if at > 0 {
            runs.push((style, rest[..at].to_owned()));
        }
        let Some(end) = rest[at..].find(']') else {
            return runs;
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
                "@p" => "/home/me/src/proj",
                "@u" => "héllo wörld",
                "@e" => "",
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

    /// Each value is what tmux 3.6 printed for the same format (2026-10-18).
    #[test]
    fn value_modifiers_chain_in_tmuxs_order() {
        assert_eq!(x("#{b:@p}"), "proj");
        assert_eq!(x("#{d:@p}"), "/home/me/src");
        assert_eq!(x("#{s/o/0/:@p}"), "/h0me/me/src/pr0j");
        assert_eq!(x("#{s/O/0/i:@p}"), "/h0me/me/src/pr0j");
        assert_eq!(x(r"#{s/(m)e/\1X/:@p}"), "/homX/mX/src/proj");
        assert_eq!(x("#{l:#{@p}}"), "#{@p}");
        assert_eq!(x("#{n:@p}"), "17");
        assert_eq!(x("#{n:@u}"), "13", "bytes, not characters");
        assert_eq!(x("#{p10:@e}|"), "          |");
        assert_eq!(x("#{p5:session_name}|"), "main |");
        assert_eq!(x("#{p-5:session_name}|"), " main|");
        assert_eq!(x("#{p2:@p}|"), "/home/me/src/proj|", "padding never cuts");
        assert_eq!(x("#{b;s/r/R/:@p}"), "pRoj");
        assert_eq!(x("#{=3;b:@p}"), "pro", "written first, cut last");
        assert_eq!(x("#{b:@nosuch}|"), "|");
        assert_eq!(x("#{?@e,1,@p,2,3}"), "2");
        assert_eq!(x("#{?@e,1,@e,2,3}"), "3");
        assert_eq!(x("#{?@e,1,@e,2}|"), "|");
        // Names that only start like a modifier are variables.
        assert_eq!(x("#{session_name}"), "main");
        assert_eq!(x("#{pane_current_path}"), "/home/u/src");
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
        assert_eq!(
            styled("ab#[fg=red"),
            [(Style::default(), "ab".into())],
            "an unclosed style ends the text"
        );
        assert_eq!(parse_colour("brightred"), Some(Colour::Indexed(9)));
        assert_eq!(parse_colour("nope"), None);
    }
}
