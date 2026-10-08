//! tmux formats: `#{…}` expansion and `#[…]` style runs.
//!
//! Supported: `#{name}`, `#{?cond,then,else}`, `#{E:name}` (expand the
//! variable's value as a format again), `#{T:name}` (same), `#{=N:x}` /
//! `#{=-N:x}` (keep the first / last N characters), `#{==:a,b}`, `#{!=:a,b}`,
//! `#{||:a,b}`, `#{&&:a,b}`, the short aliases `#S #W #I #P #H #h #F #D #T`
//! and `##` for a literal `#`. Commas inside nested `#{…}` do not split.
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
    if let Some(rest) = body.strip_prefix('=')
        && let Some((n, inner)) = rest.split_once(':')
        && let Ok(n) = n.parse::<i64>()
    {
        let text = expand_item(inner, ctx, depth + 1);
        let chars: Vec<char> = text.chars().collect();
        let keep = n.unsigned_abs() as usize;
        return if n >= 0 {
            chars.iter().take(keep).collect()
        } else {
            chars[chars.len().saturating_sub(keep)..].iter().collect()
        };
    }
    if body.contains("#{") {
        return ex(body);
    }
    ctx.get(body).unwrap_or_default()
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
                _ => return None,
            }
            .to_owned(),
        )
    }

    fn x(f: &str) -> String {
        expand(f, &ctx)
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
