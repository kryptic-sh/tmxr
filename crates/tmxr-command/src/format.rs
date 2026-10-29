//! tmux formats: `#{…}` expansion and `#[…]` style runs.
//!
//! Supported: `#{name}`, `#{?cond,then,else}`, `#{E:name}` (expand the
//! variable's value as a format again), `#{T:name}` (same), `#{=N:x}` /
//! `#{=-N:x}` (keep the first / last N characters; `#{=/N/marker:x}` marks a
//! cut), `#{==:a,b}`, `#{!=:a,b}`, `#{<:a,b}`, `#{>:a,b}`, `#{<=:a,b}`,
//! `#{>=:a,b}` (compared as strings, as tmux's), `#{m:glob,text}`,
//! `#{m/r:regex,text}` (`i` ignores case), `#{q:x}` (shell-quoted) and
//! `#{q/e:x}` (`#` doubled), `#{N:name}` / `#{N/w:name}` (a window of the
//! session is called that) and `#{N/s:name}` (a session is), `#{C:text}`
//! (the first line of the pane's screen holding text, as a glob; `C/r` a
//! regex, `i` ignoring case), `#{R:text,n}` (text n times), `#{!:x}` /
//! `#{!!:x}` (whether x expands to false / true), the loops `#{S:fmt}`,
//! `#{W:fmt}`, `#{P:fmt}` and `#{L:fmt}` (fmt for each session, window of the
//! session, pane of the window, or client; `S`, `W` and `P` take a second
//! format for the current one), `#{||:a,b}`,
//! `#{&&:a,b}`, the short aliases
//! `#S #W #I #P #H #h #F #D #T` and `##` for a literal `#`. Commas inside
//! nested `#{…}` do not split.
//!
//! `#[…]` is left in the expanded text; [`styled`] splits text into styled
//! runs afterwards.

/// Supplies variable values (`session_name`, `@my_option`, …).
pub trait Context {
    fn get(&self, name: &str) -> Option<String>;

    /// Whether a window of the context's session (`Window`) or any session
    /// (`Session`) is called `name`, for `#{N:…}`; `None` when there is no
    /// session to ask, which expands to nothing, as tmux's.
    fn name_exists(&self, kind: NameKind, name: &str) -> Option<bool>;

    /// The lines on the screen of the context's pane, top to bottom, for
    /// `#{C:…}`; `None` when there is no pane.
    fn pane_lines(&self) -> Option<Vec<String>>;

    /// What a loop goes over, in index order (session id, window index,
    /// pane id, the order clients connected), each as a context of its own;
    /// `None` when there is nothing to loop in (no session for windows, no
    /// window for panes), which expands to nothing.
    fn loop_items(&self, kind: LoopKind) -> Option<Vec<LoopItem<'_>>>;

    /// What shell command `cmd` (as written in `#(…)`, `expanded` its text
    /// expanded) printed last, as tmux's format_job_get; `None` when the
    /// context runs no commands, which expands to nothing.
    fn job(&self, cmd: &str, expanded: &str) -> Option<String> {
        let _ = (cmd, expanded);
        None
    }
}

/// What a format loop goes over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopKind {
    Sessions,
    Windows,
    Panes,
    Clients,
}

/// One session, window, pane or client of a loop.
pub struct LoopItem<'a> {
    /// The variables as they are for it.
    pub ctx: Box<dyn Context + 'a>,
    /// It is the current one: the client's session, the session's window,
    /// the window's active pane.
    pub current: bool,
    /// Its place in index order.
    pub index: u64,
    /// The name `n` sorts by.
    pub name: String,
    /// The activity `t` sorts by, most recent first.
    pub activity: std::time::SystemTime,
}

/// What `#{N:…}` looks a name up among.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKind {
    Window,
    Session,
}

/// A plain lookup function has variables and no sessions to ask.
impl<F: Fn(&str) -> Option<String>> Context for F {
    fn get(&self, name: &str) -> Option<String> {
        self(name)
    }

    fn name_exists(&self, _kind: NameKind, _name: &str) -> Option<bool> {
        None
    }

    fn pane_lines(&self) -> Option<Vec<String>> {
        None
    }

    fn loop_items(&self, _kind: LoopKind) -> Option<Vec<LoopItem<'_>>> {
        None
    }
}

/// A loop item's context with `loop_last_flag` set for it.
struct InLoop<'c> {
    inner: &'c dyn Context,
    last: bool,
}

impl Context for InLoop<'_> {
    fn get(&self, name: &str) -> Option<String> {
        if name == "loop_last_flag" {
            return Some(if self.last { "1" } else { "0" }.to_owned());
        }
        self.inner.get(name)
    }

    fn name_exists(&self, kind: NameKind, name: &str) -> Option<bool> {
        self.inner.name_exists(kind, name)
    }

    fn pane_lines(&self) -> Option<Vec<String>> {
        self.inner.pane_lines()
    }

    fn loop_items(&self, kind: LoopKind) -> Option<Vec<LoopItem<'_>>> {
        self.inner.loop_items(kind)
    }

    fn job(&self, cmd: &str, expanded: &str) -> Option<String> {
        self.inner.job(cmd, expanded)
    }
}

/// tmux's format loops: `fmt` expanded for each item in turn, sorted as
/// `flags` ask (`i` index, `n` name, `t` activity, `r` reversed; panes by
/// id only), the part after the first comma used for the current item
/// instead (not for clients).
fn expand_loop(kind: LoopKind, flags: &str, fmt: &str, ctx: &dyn Context, pass: Pass) -> String {
    let Some(mut items) = ctx.loop_items(kind) else {
        return String::new();
    };
    let reversed = flags.contains('r');
    let by_name = |a: &LoopItem<'_>, b: &LoopItem<'_>| a.name.cmp(&b.name);
    if kind != LoopKind::Panes && !flags.contains('i') && flags.contains('n') {
        items.sort_by(by_name);
    } else if kind != LoopKind::Panes && !flags.contains('i') && flags.contains('t') {
        items.sort_by(|a, b| b.activity.cmp(&a.activity).then_with(|| by_name(a, b)));
    } else {
        items.sort_by_key(|item| item.index);
    }
    if reversed {
        items.reverse();
    }
    let (all, current) = match split_top(fmt).first() {
        Some(first) if kind != LoopKind::Clients && first.len() < fmt.len() => {
            (*first, Some(&fmt[first.len() + 1..]))
        }
        _ => (fmt, None),
    };
    let count = items.len();
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let use_fmt = match current {
                Some(c) if item.current => c,
                _ => all,
            };
            let in_loop = InLoop {
                inner: item.ctx.as_ref(),
                last: i + 1 == count,
            };
            expand_depth(use_fmt, &in_loop, pass.deeper())
        })
        .collect()
}

/// The most times `#{R:…}` repeats its text, as tmux's.
const MAX_REPEAT: usize = 10_000;

/// Nesting limit for `#{E:…}`, so an option that refers to itself cannot
/// recurse forever.
const MAX_DEPTH: usize = 10;

/// `fmt` with its `strftime` sequences (`%H:%M`) as the time now, as tmux
/// fills them in before expanding the status line, the terminal title and
/// `display-message` (`format_expand_time`).
fn time_now(fmt: &str) -> String {
    match crate::localtime::now() {
        Some(now) if fmt.contains('%') => strftime(fmt, &now),
        _ => fmt.to_owned(),
    }
}

/// [`expand`] with time expansion on, as tmux's `format_expand_time`: each
/// format expanded, nested ones and modifiers' arguments too, has its
/// strftime sequences filled in first ([`time_now`]).
pub fn expand_time(fmt: &str, ctx: &dyn Context) -> String {
    expand_depth(
        fmt,
        ctx,
        Pass {
            depth: 0,
            time: true,
            jobs: true,
        },
    )
}

/// How deep an expansion is, and whether it fills in strftime sequences.
#[derive(Debug, Clone, Copy)]
struct Pass {
    depth: usize,
    time: bool,
    /// `#(…)` runs its command; off inside a command and its output, as
    /// tmux's FORMAT_EXPAND_NOJOBS.
    jobs: bool,
}

impl Pass {
    const fn deeper(self) -> Self {
        Self {
            depth: self.depth + 1,
            ..self
        }
    }
}

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// `fmt` with C's `strftime` conversions filled in from `t`, in the C
/// locale: `%a %A %b %B %c %C %d %D %e %F %h %H %I %j %k %l %m %M %n %p %P %r
/// %R %s %S %t %T %u %w %y %Y %%`. Any other `%` sequence is kept as it is.
/// The ISO 8601 week-based year and the days into its week 1 of `t`, as
/// glibc's strftime works them out for `%G` / `%V` (its iso_week_days,
/// transcribed): weeks start on Monday, and week 1 holds the first
/// Thursday.
fn iso_week(t: &crate::localtime::LocalTime) -> (i32, i32) {
    // Days from `yday`'s week-1 Monday; the 7 * 54 keeps `%` nonnegative.
    let days = |yday: i32| yday - (yday - i32::from(t.weekday) + 4 + 7 * 54) % 7 + 3;
    let year_len = |y: i32| 365 + i32::from(crate::localtime::is_leap(y));
    let yday = i32::from(t.yearday);
    let here = days(yday);
    if here < 0 {
        return (t.year - 1, days(yday + year_len(t.year - 1)));
    }
    let next = days(yday - year_len(t.year));
    if next >= 0 {
        (t.year + 1, next)
    } else {
        (t.year, here)
    }
}

pub fn strftime(fmt: &str, t: &crate::localtime::LocalTime) -> String {
    let day = DAYS[usize::from(t.weekday % 7)];
    let month = MONTHS[usize::from(t.month.clamp(1, 12) - 1)];
    let hour12 = match t.hour % 12 {
        0 => 12,
        h => h,
    };
    let mut out = String::new();
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(spec) = chars.next() else {
            out.push('%');
            break;
        };
        let piece = match spec {
            'a' => day[..3].to_owned(),
            'A' => day.to_owned(),
            'b' | 'h' => month[..3].to_owned(),
            'B' => month.to_owned(),
            'c' => format!(
                "{} {} {:>2} {:02}:{:02}:{:02} {}",
                &day[..3],
                &month[..3],
                t.day,
                t.hour,
                t.minute,
                t.second,
                t.year
            ),
            'C' => format!("{:02}", t.year.div_euclid(100)),
            'd' => format!("{:02}", t.day),
            'D' => format!("{:02}/{:02}/{:02}", t.month, t.day, t.year.rem_euclid(100)),
            'e' => format!("{:>2}", t.day),
            'F' => format!("{}-{:02}-{:02}", t.year, t.month, t.day),
            'H' => format!("{:02}", t.hour),
            'I' => format!("{hour12:02}"),
            'j' => format!("{:03}", t.yearday + 1),
            'k' => format!("{:>2}", t.hour),
            'l' => format!("{hour12:>2}"),
            'm' => format!("{:02}", t.month),
            'M' => format!("{:02}", t.minute),
            'n' => "\n".to_owned(),
            'p' => if t.hour < 12 { "AM" } else { "PM" }.to_owned(),
            'P' => if t.hour < 12 { "am" } else { "pm" }.to_owned(),
            'r' => format!(
                "{hour12:02}:{:02}:{:02} {}",
                t.minute,
                t.second,
                if t.hour < 12 { "AM" } else { "PM" }
            ),
            'R' => format!("{:02}:{:02}", t.hour, t.minute),
            's' => t.epoch.to_string(),
            'S' => format!("{:02}", t.second),
            't' => "\t".to_owned(),
            'T' => format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second),
            // Weeks of the year from its first Sunday (`%U`) or Monday
            // (`%W`), week 0 before it, as C's strftime.
            'U' => format!("{:02}", (t.yearday + 7 - u16::from(t.weekday)) / 7),
            'W' => format!(
                "{:02}",
                (t.yearday + 7 - u16::from((t.weekday + 6) % 7)) / 7
            ),
            'V' => format!("{:02}", iso_week(t).1 / 7 + 1),
            'G' => iso_week(t).0.to_string(),
            'g' => format!("{:02}", iso_week(t).0.rem_euclid(100)),
            // The zone's offset from UTC, `+hhmm`.
            'z' => {
                let minutes = t.utc_offset().div_euclid(60);
                let sign = if minutes < 0 { '-' } else { '+' };
                let minutes = minutes.abs();
                format!("{sign}{:02}{:02}", minutes / 60, minutes % 60)
            }
            // The C locale's date and time.
            'x' => format!("{:02}/{:02}/{:02}", t.month, t.day, t.year.rem_euclid(100)),
            'X' => format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second),
            'u' => match t.weekday {
                0 => 7,
                d => d,
            }
            .to_string(),
            'w' => t.weekday.to_string(),
            'y' => format!("{:02}", t.year.rem_euclid(100)),
            'Y' => t.year.to_string(),
            '%' => "%".to_owned(),
            other => format!("%{other}"),
        };
        out.push_str(&piece);
    }
    out
}

/// tmux's `format_pretty_time`: a time as short as its age allows (`%H:%M`
/// within a day, `%a%d` within the month or 28 days, `%d%b` within a year,
/// else `%b%y`).
fn pretty_time(t: &crate::localtime::LocalTime, now: &crate::localtime::LocalTime) -> String {
    let age = (now.epoch - t.epoch).max(0);
    let fmt = if age < 24 * 3600 {
        "%H:%M"
    } else if (t.year == now.year && t.month == now.month) || age < 28 * 24 * 3600 {
        "%a%d"
    } else if (t.year == now.year && t.month < now.month)
        || (t.year == now.year - 1 && t.month > now.month)
    {
        "%d%b"
    } else {
        "%h%y"
    };
    strftime(fmt, t)
}

/// tmux's `format_strip`: `#` escapes taken out of a modifier's argument,
/// except inside a nested `#{…}`.
fn strip_escapes(s: &str) -> String {
    let mut out = String::new();
    let mut brackets = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        let next = chars.peek().copied();
        if c == '#' && next == Some('{') {
            brackets += 1;
        }
        if c == '#' && next.is_some_and(|n| ",#{}:".contains(n)) {
            if brackets != 0 {
                out.push(c);
            }
            continue;
        }
        if c == '}' {
            brackets = brackets.saturating_sub(1);
        }
        out.push(c);
    }
    out
}

/// Expand a format string.
pub fn expand(fmt: &str, ctx: &dyn Context) -> String {
    expand_depth(
        fmt,
        ctx,
        Pass {
            depth: 0,
            time: false,
            jobs: true,
        },
    )
}

/// `#(command)` at the start of `after` (just past the `#`): the command's
/// text and what follows its `)`, found by counting parentheses as tmux's
/// format_expand1 does; `None` without a closing one.
fn job_text(after: &str) -> Option<(&str, &str)> {
    let body = after.strip_prefix('(')?;
    let mut depth = 1;
    for (i, c) in body.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&body[..i], &body[i + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

fn expand_depth(fmt: &str, ctx: &dyn Context, pass: Pass) -> String {
    if pass.depth > MAX_DEPTH {
        return String::new();
    }
    let timed;
    let fmt = if pass.time {
        timed = time_now(fmt);
        timed.as_str()
    } else {
        fmt
    };
    let mut out = String::with_capacity(fmt.len());
    let mut rest = fmt;
    while let Some(at) = rest.find('#') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let mut chars = after.chars();
        match chars.next() {
            Some('{') => match matching_brace(&after[1..]) {
                Some(end) => {
                    // A syntax error ends the expansion there, as tmux's.
                    let Some(value) = expand_item(&after[1..1 + end], ctx, pass) else {
                        return out;
                    };
                    out.push_str(&value);
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
            Some('(') if job_text(after).is_some() => {
                let (cmd, tail) = job_text(after).unwrap_or_default();
                if pass.jobs {
                    // The command and what it printed are formats with no
                    // jobs and no time in them, as tmux's.
                    let plain = Pass {
                        time: false,
                        jobs: false,
                        ..pass.deeper()
                    };
                    let expanded = expand_depth(cmd, ctx, plain);
                    if let Some(printed) = ctx.job(cmd, &expanded) {
                        out.push_str(&expand_depth(&printed, ctx, plain));
                    }
                }
                rest = tail;
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

/// Where `s` first reaches one of `end` outside a `#{…}`, as tmux's
/// format_skip: a `#` before any of `,#{}:` escapes it (though `#{` still
/// opens a format, which its `}` closes).
fn format_skip(s: &str, end: &[u8]) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'#' if bytes.get(i + 1).is_some_and(|b| b",#{}:".contains(b)) => {
                if bytes[i + 1] == b'{' {
                    depth += 1;
                }
                i += 2;
                continue;
            }
            b'}' if depth > 0 => depth -= 1,
            b if depth == 0 && end.contains(&b) => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Index of the `}` closing a `#{` whose body starts at `s[0]`.
fn matching_brace(s: &str) -> Option<usize> {
    format_skip(s, b"}")
}

/// Split on top-level commas (not inside a nested `#{…}`, nor escaped).
fn split_top(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = s;
    while let Some(at) = format_skip(rest, b",") {
        parts.push(&rest[..at]);
        rest = &rest[at + 1..];
    }
    parts.push(rest);
    parts
}

fn truthy(s: &str) -> bool {
    !s.is_empty() && s != "0"
}

/// `s` cut at its first comma outside `#{…}`, as tmux's format_choose: the
/// right side is all the rest. `None` without one, which tmux takes as a
/// syntax error.
fn split_first(s: &str) -> Option<(&str, &str)> {
    let first = split_top(s).into_iter().next()?;
    Some((first, s.get(first.len() + 1..)?))
}

/// One `#{…}` item's value, or `None` for a syntax error, which ends the
/// expansion holding the item as tmux's format_replace failing does: a
/// comparison, `m:` or `R:` without its comma.
fn expand_item(body: &str, ctx: &dyn Context, pass: Pass) -> Option<String> {
    let ex = |s: &str| expand_depth(s, ctx, pass.deeper());
    let flag = |b: bool| if b { "1" } else { "0" }.to_owned();
    // ||: and &&: over any number of parts, as tmux's format_bool_op_n.
    for (op, and) in [("||:", false), ("&&:", true)] {
        if let Some(rest) = body.strip_prefix(op) {
            let mut parts = split_top(rest).into_iter().map(|p| truthy(&ex(p)));
            return Some(flag(if and {
                parts.all(|t| t)
            } else {
                parts.any(|t| t)
            }));
        }
    }
    for (op, f) in [
        ("==:", (|a: &str, b: &str| a == b) as fn(&str, &str) -> bool),
        ("!=:", |a, b| a != b),
        // tmux compares with strcmp: as strings, not numbers.
        ("<=:", |a, b| a <= b),
        (">=:", |a, b| a >= b),
        ("<:", |a, b| a < b),
        (">:", |a, b| a > b),
    ] {
        if let Some(rest) = body.strip_prefix(op) {
            let (a, b) = split_first(rest)?;
            return Some(flag(f(&ex(a), &ex(b))));
        }
    }
    // R: the text repeated the given number of times, from 1 to tmux's
    // limit; any other count gives nothing.
    if let Some(rest) = body.strip_prefix("R:") {
        let (text, count) = split_first(rest)?;
        return Some(
            ex(count)
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=MAX_REPEAT).contains(n))
                .map(|n| ex(text).repeat(n))
                .unwrap_or_default(),
        );
    }
    if let Some((mods, inner)) = value_modifiers(body)
        && mods.iter().any(|m| matches!(m, Mod::Match(_)))
        && split_first(inner).is_none()
    {
        return None;
    }
    Some(item_value(body, ctx, pass))
}

/// The value of an item that cannot be a syntax error.
fn item_value(body: &str, ctx: &dyn Context, pass: Pass) -> String {
    let ex = |s: &str| expand_depth(s, ctx, pass.deeper());
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
    // !: and !!: whether the text expands to something false or true; it is
    // expanded as a format, not looked up as a variable, as tmux's.
    if let Some(text) = body.strip_prefix("!!:") {
        return if truthy(&ex(text)) { "1" } else { "0" }.to_owned();
    }
    if let Some(text) = body.strip_prefix("!:") {
        return if truthy(&ex(text)) { "0" } else { "1" }.to_owned();
    }
    for (letter, kind) in [
        ('S', LoopKind::Sessions),
        ('W', LoopKind::Windows),
        ('P', LoopKind::Panes),
        ('L', LoopKind::Clients),
    ] {
        if let Some((flags, fmt)) = body.strip_prefix(letter).and_then(modifier_args) {
            return expand_loop(kind, flags, fmt, ctx, pass);
        }
    }
    // t: a time in seconds as ctime writes it; t/p as short as its age
    // allows; t/f/FORMAT by strftime; nothing when the value is not a time,
    // as tmux's.
    if let Some((args, name)) =
        body.strip_prefix('t')
            .and_then(|rest| match rest.strip_prefix(':') {
                Some(name) => Some((Vec::new(), name)),
                None => expression_args(rest),
            })
    {
        let value = if name.contains("#{") {
            ex(name)
        } else {
            ctx.get(name).unwrap_or_default()
        };
        let Some(t) = value
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .and_then(crate::localtime::at)
        else {
            return String::new();
        };
        return match args.as_slice() {
            [flags, ..] if flags.contains('p') => {
                crate::localtime::now().map_or_else(String::new, |now| pretty_time(&t, &now))
            }
            // The format is expanded first, as tmux's modifier arguments
            // are, so under time expansion its own %s are filled in too.
            [flags, format, ..] if flags.contains('f') => strftime(&strip_escapes(&ex(format)), &t),
            _ => strftime("%a %b %e %H:%M:%S %Y", &t),
        };
    }
    if let Some(text) = body.strip_prefix("a:") {
        // `a`: the character with that code, printable ASCII only.
        return ex(text)
            .parse::<u8>()
            .ok()
            .filter(|c| (32..=126).contains(c))
            .map(|c| char::from(c).to_string())
            .unwrap_or_default();
    }
    if let Some(text) = body.strip_prefix("c:") {
        // `c`: the colour it names as six hex digits, as tmux's.
        return crate::colour::rgb(&ex(text)).map_or_else(String::new, |c| format!("{c:06x}"));
    }
    if let Some((args, rest)) = body.strip_prefix('e').and_then(expression_args) {
        let parts = split_top(rest);
        let left = ex(parts.first().copied().unwrap_or_default());
        let right = ex(parts.get(1).copied().unwrap_or_default());
        return expression(&args, &left, &right).unwrap_or_default();
    }
    // N: whether a window (no flag, or w) or a session (s) has the name the
    // rest expands to; with neither flag, an ordinary lookup, as tmux's.
    if let Some((flags, inner)) = body.strip_prefix('N').and_then(modifier_args) {
        let kind = if flags.is_empty() || flags.contains('w') {
            NameKind::Window
        } else if flags.contains('s') {
            NameKind::Session
        } else {
            return ctx.get(inner).unwrap_or_default();
        };
        return ctx
            .name_exists(kind, &ex(inner))
            .map_or_else(String::new, |found| {
                if found { "1" } else { "0" }.to_owned()
            });
    }
    // C: the number (from 1) of the first screen line holding the text, its
    // trailing spaces cut; 0 for none or no pane, as tmux's
    // window_pane_search.
    if let Some((flags, inner)) = body.strip_prefix('C').and_then(modifier_args) {
        let term = ex(inner);
        let pattern = if flags.contains('r') {
            term
        } else {
            format!("*{term}*")
        };
        let found = ctx.pane_lines().and_then(|lines| {
            lines.iter().position(|line| {
                let line = line.trim_end_matches(|c: char| c.is_ascii_whitespace());
                format_match(flags, &pattern, line)
            })
        });
        return found.map_or(0, |i| i + 1).to_string();
    }
    if let Some((mods, inner)) = value_modifiers(body) {
        return apply_modifiers(&mods, inner, ctx, pass);
    }
    if body.contains("#{") {
        return ex(body);
    }
    ctx.get(body).unwrap_or_default()
}

/// `e`'s arguments and what follows their `:`: `|+|:` or `|/|f|4:` (any
/// punctuation for `|`; the last argument ends at its `|` or at the `:`),
/// as tmux's `format_build_modifiers` splits them.
fn expression_args(rest: &str) -> Option<(Vec<&str>, &str)> {
    let sep = rest.chars().next().filter(char::is_ascii_punctuation)?;
    let mut args = Vec::new();
    let mut s = &rest[1..];
    loop {
        let end = s.find([sep, ':'])?;
        args.push(&s[..end]);
        // Both are ASCII, one byte.
        let after = &s[end + 1..];
        if s.as_bytes()[end] == b':' {
            return Some((args, after));
        }
        if let Some(inner) = after.strip_prefix(':') {
            return Some((args, inner));
        }
        s = after;
    }
}

/// `#{e|op|flags|precision:left,right}`, as tmux's
/// `format_replace_expression`: arithmetic (`+ - * / %` or `m`) or a
/// comparison (`== != < > <= >=`, 1 or 0) on numbers, whole unless the
/// flags hold `f`, printed with `precision` decimals (2 with `f`, else 0).
/// `None` for anything tmux would not expand.
fn expression(args: &[&str], left: &str, right: &str) -> Option<String> {
    let (&op, rest) = args.split_first()?;
    if rest.len() > 2 {
        return None;
    }
    let float = rest.first().is_some_and(|f| f.contains('f'));
    // tmux's is a C int given to `%.*f`, where a negative one means the
    // default, 6.
    let precision = match rest.get(1) {
        Some(p) => usize::try_from(p.parse::<i32>().ok()?).unwrap_or(6),
        None if float => 2,
        None => 0,
    };
    let number = |s: &str| {
        let n = s.parse::<f64>().ok()?;
        Some(if float { n } else { n.trunc() })
    };
    let (l, r) = (number(left)?, number(right)?);
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    let result = match op {
        "+" => l + r,
        "-" => l - r,
        "*" => l * r,
        "/" => l / r,
        "%" | "m" => l % r,
        "==" => flag((l - r).abs() < 1e-9),
        "!=" => flag((l - r).abs() > 1e-9),
        ">" => flag(l > r),
        "<" => flag(l < r),
        ">=" => flag(l >= r),
        "<=" => flag(l <= r),
        _ => return None,
    };
    let result = if float { result } else { result.trunc() };
    Some(fixed(result, precision))
}

/// The most decimals a double's exact expansion needs: past them every
/// digit is 0.
const EXACT_DECIMALS: usize = 1100;

/// `n` with `precision` decimals, as C's `%.*f`. Rust's formatter refuses a
/// precision past `u16::MAX`, so the exact digits are written and the rest
/// are the zeros they would have been.
fn fixed(n: f64, precision: usize) -> String {
    let exact = precision.min(EXACT_DECIMALS);
    let mut s = format!("{n:.exact$}");
    if n.is_finite() {
        s.extend(std::iter::repeat_n('0', precision - exact));
    }
    s
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
    /// `w`: the width on screen, as tmux's `format_width`.
    Width,
    /// `s/pattern/replacement/flags`: each match of a regular expression
    /// replaced (`i` ignores case).
    Sub {
        pattern: &'a str,
        replacement: &'a str,
        flags: &'a str,
    },
    /// `=N` keeps the first N characters, `=-N` the last; `=/N/marker`
    /// also puts `marker` where text was cut off. N may be a format, as in
    /// tmux's `=/#{status-left-length}`, so it is read when applied.
    Limit(&'a str, &'a str),
    /// `pN` pads to N characters with spaces after, `p-N` before.
    Pad(&'a str),
    /// `E`: the value expanded again as a format.
    Expand,
    /// `T`: the same with time expansion on.
    ExpandTime,
    /// `q`: a variable's value shell-quoted; `q/e` (or `q/h`) with `#`
    /// doubled instead.
    Quote(&'a str),
    /// `m/flags:pattern,text`: 1 when the pattern matches the text.
    Match(&'a str),
}

/// Where a modifier's argument ends: at the first `;` or `:` outside a
/// `#{…}` in it.
fn arg_end(s: &str) -> Option<usize> {
    format_skip(s, b";:")
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
            'b' | 'd' | 'l' | 'n' | 'w' | 'E' | 'T' => {
                let m = match letter {
                    'b' => Mod::Basename,
                    'd' => Mod::Dirname,
                    'l' => Mod::Literal,
                    'n' => Mod::Length,
                    'w' => Mod::Width,
                    'E' => Mod::Expand,
                    _ => Mod::ExpandTime,
                };
                (m, after)
            }
            'q' | 'm' => {
                // Optional `/flags` before the `;` or `:`.
                let (flags, tail) = match after.strip_prefix('/') {
                    Some(f) => {
                        let end = f.find([';', ':'])?;
                        (&f[..end], &f[end..])
                    }
                    None => ("", after),
                };
                let m = if letter == 'q' {
                    Mod::Quote(flags)
                } else {
                    Mod::Match(flags)
                };
                (m, tail)
            }
            's' => {
                let sep = after.chars().next().filter(|c| !c.is_alphanumeric())?;
                let (pattern, rest) = after[sep.len_utf8()..].split_once(sep)?;
                // The replacement ends at the next separator, or at a `;`
                // or `:` with no closing one, as tmux's
                // format_build_modifiers reads `s/a/b`.
                let end = rest.find([sep, ';', ':'])?;
                let replacement = &rest[..end];
                let (flags, tail) = match rest[end..].strip_prefix(sep) {
                    Some(tail) => {
                        let end = tail.find([';', ':'])?;
                        (&tail[..end], &tail[end..])
                    }
                    None => ("", &rest[end..]),
                };
                let m = Mod::Sub {
                    pattern,
                    replacement,
                    flags,
                };
                (m, tail)
            }
            'p' | '=' => {
                let (args, tail) = match after.chars().next() {
                    Some(c) if c == '-' || c.is_ascii_digit() => {
                        let end = arg_end(after)?;
                        (&after[..end], &after[end..])
                    }
                    // `#` starts a format, so tmux takes no `=#…`.
                    Some(sep) if letter == '=' && !sep.is_alphanumeric() && !"=#".contains(sep) => {
                        let inner = &after[sep.len_utf8()..];
                        let end = arg_end(inner)?;
                        (inner[..end].trim_end_matches(sep), &inner[end..])
                    }
                    _ => return None,
                };
                let mut args = args.split(['/', '|']);
                let n = args.next()?;
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
/// value (the text itself for `l`, a match's result for `m`, a variable read
/// directly with its path part taken and then quoted), expanded again for
/// `E` or `T`, then substitutions, the length limit, padding, and the length.
fn apply_modifiers(mods: &[Mod<'_>], inner: &str, ctx: &dyn Context, pass: Pass) -> String {
    let ex = |s: &str| expand_depth(s, ctx, pass.deeper());
    let matching = mods.iter().find_map(|m| match m {
        Mod::Match(flags) => Some(*flags),
        _ => None,
    });
    let mut value = if mods.contains(&Mod::Literal) {
        inner.to_owned()
    } else if let Some(flags) = matching {
        // expand_item has refused one without its comma.
        let (pattern, text) = split_first(inner).unwrap_or((inner, ""));
        if format_match(flags, &ex(pattern), &ex(text)) {
            "1"
        } else {
            "0"
        }
        .to_owned()
    } else if inner.contains("#{") {
        ex(inner)
    } else {
        let found = ctx.get(inner).unwrap_or_default();
        let path = std::path::Path::new(&found);
        let part = |p: Option<&std::ffi::OsStr>| p.map(|p| p.to_string_lossy().into_owned());
        let found = if mods.contains(&Mod::Basename) {
            part(path.file_name()).unwrap_or(found)
        } else if mods.contains(&Mod::Dirname) {
            part(path.parent().map(std::path::Path::as_os_str))
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| ".".to_owned())
        } else {
            found
        };
        // Quoting applies to a variable read directly only, as tmux's
        // format_find: `#{q:#{x}}` is not quoted.
        match mods.iter().find_map(|m| match m {
            Mod::Quote(flags) => Some(*flags),
            _ => None,
        }) {
            Some(flags) if flags.contains('e') || flags.contains('h') => found.replace('#', "##"),
            Some(_) => quote_shell(&found),
            None => found,
        }
    };
    if mods.contains(&Mod::Expand) {
        value = ex(&value);
    } else if mods.contains(&Mod::ExpandTime) {
        let timed = Pass {
            time: true,
            ..pass.deeper()
        };
        value = expand_depth(&value, ctx, timed);
    }
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
    // A count that is no number, or past a C int as tmux's `strtonum`
    // reads it, does nothing.
    let count = |n: &str| ex(n).trim().parse::<i32>().ok().map(i64::from);
    for m in mods {
        if let Mod::Limit(n, marker) = m
            && let Some(n) = count(n)
        {
            value = limit(&value, n, &ex(marker));
        }
    }
    for m in mods {
        if let Mod::Pad(n) = m
            && let Some(n) = count(n)
        {
            let missing = usize::try_from(n.unsigned_abs())
                .unwrap_or(usize::MAX)
                .saturating_sub(value.chars().count());
            let spaces = " ".repeat(missing);
            value = if n >= 0 {
                value + &spaces
            } else {
                spaces + &value
            };
        }
    }
    if mods.contains(&Mod::Length) {
        value = value.len().to_string();
    }
    if mods.contains(&Mod::Width) {
        value = display_width(&value).to_string();
    }
    value
}

/// Columns `text` takes when drawn, as tmux's `format_width`: styles take
/// none, `##` is one `#`, and a control character none. Where a `#[` has no
/// `]` this counts the text before it; tmux's gives 0 for the whole.
fn display_width(text: &str) -> usize {
    use unicode_width::UnicodeWidthChar as _;
    draw_pieces(text)
        .iter()
        .filter_map(|p| match p {
            Piece::Text(t) => Some(t.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>()),
            Piece::Style(_) => None,
        })
        .sum()
}

/// tmux's `s/pattern/replacement/`: every match of the extended regular
/// expression replaced, `\1` and the like naming its groups; `i` in `flags`
/// ignores case. A pattern that does not compile leaves the text alone.
fn substitute(text: &str, pattern: &str, replacement: &str, flags: &str) -> String {
    let Some(re) = compiled(pattern, flags.contains('i')) else {
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

/// `=N`: the first N columns of `text` (the last with N negative), with
/// `marker` where text was cut off. A `#[…]` style takes no room, as
/// tmux's format_trim_left / format_trim_right count it, and 0 keeps it
/// all, as tmux's.
fn limit(text: &str, n: i64, marker: &str) -> String {
    if n == 0 {
        return text.to_owned();
    }
    let keep = usize::try_from(n.unsigned_abs()).unwrap_or(usize::MAX);
    let cut = if n >= 0 {
        trim_left(text, keep)
    } else {
        trim_right(text, keep)
    };
    if cut == text {
        cut
    } else if n >= 0 {
        cut + marker
    } else {
        marker.to_owned() + &cut
    }
}

/// `text` in styles (`#[…]`, which take no room) and characters.
fn style_pieces(text: &str) -> Vec<(&str, bool)> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("#[")
            && let Some(end) = rest.find(']')
        {
            pieces.push((&rest[..=end], true));
            rest = &rest[end + 1..];
            continue;
        }
        pieces.push((&rest[..c.len_utf8()], false));
        rest = &rest[c.len_utf8()..];
    }
    pieces
}

/// The columns a piece of [`style_pieces`] takes: none for a style.
fn piece_width((piece, style): (&str, bool)) -> usize {
    use unicode_width::UnicodeWidthStr as _;
    if style { 0 } else { piece.width() }
}

/// The first `keep` columns of `text` and the styles among them, as tmux's
/// format_trim_left: a wide character that would run past them is left
/// out, and it stops once they are taken.
fn trim_left(text: &str, keep: usize) -> String {
    let mut out = String::new();
    let mut taken = 0;
    for piece in style_pieces(text) {
        if taken >= keep {
            break;
        }
        let width = piece_width(piece);
        if taken + width <= keep {
            out.push_str(piece.0);
        }
        taken += width;
    }
    out
}

/// The last `keep` columns of `text` and every style, as tmux's
/// format_trim_right: a wide character that starts before them is left
/// out.
fn trim_right(text: &str, keep: usize) -> String {
    let pieces = style_pieces(text);
    let total: usize = pieces.iter().copied().map(piece_width).sum();
    let skip = total.saturating_sub(keep);
    let mut seen = 0;
    let mut out = String::new();
    for piece in pieces {
        if piece.1 || seen >= skip {
            out.push_str(piece.0);
        }
        seen += piece_width(piece);
    }
    out
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
    compiled(&regex, flags.contains('i')).is_some_and(|r| r.is_match(text))
}

/// The most patterns [`compiled`] keeps before starting afresh.
const COMPILED_LIMIT: usize = 256;

/// `pattern` compiled, case-insensitive with `icase`; `None` when it does
/// not compile. Kept for the next expansion that asks, as a status line
/// expands the same patterns on every draw and compiling dwarfs matching.
fn compiled(pattern: &str, icase: bool) -> Option<regex::Regex> {
    type Cache = std::collections::HashMap<(String, bool), Option<regex::Regex>>;
    thread_local! {
        static CACHE: std::cell::RefCell<Cache> = std::cell::RefCell::default();
    }
    CACHE.with_borrow_mut(|cache| {
        if let Some(re) = cache.get(&(pattern.to_owned(), icase)) {
            return re.clone();
        }
        if cache.len() >= COMPILED_LIMIT {
            cache.clear();
        }
        let re = regex::RegexBuilder::new(pattern)
            .case_insensitive(icase)
            .build()
            .ok();
        cache.insert((pattern.to_owned(), icase), re.clone());
        re
    })
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

/// A colour as tmux's colour_tostring names it: `default`, the eight names
/// and their `bright` forms for 0 to 15, `colourN` above, `#rrggbb`. (tmux
/// also says `colour1` for one given that way; tmxr keeps no difference.)
pub fn colour_name(c: Colour) -> String {
    const NAMES: [&str; 8] = [
        "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
    ];
    match c {
        Colour::Default => "default".to_owned(),
        Colour::Indexed(i @ 0..=7) => NAMES[usize::from(i)].to_owned(),
        Colour::Indexed(i @ 8..=15) => format!("bright{}", NAMES[usize::from(i - 8)]),
        Colour::Indexed(i) => format!("colour{i}"),
        Colour::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
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
    pub blink: Option<bool>,
    pub hidden: Option<bool>,
    pub strikethrough: Option<bool>,
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
                self.blink = Some(false);
                self.hidden = Some(false);
                self.strikethrough = Some(false);
            } else {
                let (on, name) = match item.strip_prefix("no") {
                    Some(n) => (false, n),
                    None => (true, item),
                };
                match name {
                    "bold" | "bright" => self.bold = Some(on),
                    "dim" => self.dim = Some(on),
                    "italics" | "italic" => self.italic = Some(on),
                    // tmux's other underlines are drawn as the plain one.
                    "underscore" | "underline" | "double-underscore" | "curly-underscore"
                    | "dotted-underscore" | "dashed-underscore" => self.underscore = Some(on),
                    "reverse" => self.reverse = Some(on),
                    "blink" => self.blink = Some(on),
                    "hidden" => self.hidden = Some(on),
                    "strikethrough" => self.strikethrough = Some(on),
                    _ => {}
                }
            }
        }
    }
}

/// Parse a tmux colour: `default`, `#rrggbb`, `colourN` / `colorN`, a basic
/// name (`red`) or a bright one (`brightred`), or another name tmux knows
/// (`grey50`, `orchid`: [`crate::colour::named`]).
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
    crate::colour::named(s).map(|v| Colour::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// A piece of expanded text as tmux's `format_draw` reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece<'a> {
    /// Text drawn as it is.
    Text(String),
    /// What is inside a `#[…]`.
    Style(&'a str),
}

/// `text` in pieces as tmux's `format_draw` reads it: a run of `#` draws as
/// half of them, rounded up (`##` is one `#`); before a `[`, an even run is
/// half of them and a literal `[`, an odd one half the rest and then a
/// style. A `#[` with no `]` ends the text there, as `format_draw` stops.
pub fn draw_pieces(text: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('#') {
        literal.push_str(&rest[..at]);
        let hashes = rest[at..].bytes().take_while(|&b| b == b'#').count();
        let after = &rest[at + hashes..];
        let Some(style) = after.strip_prefix('[') else {
            literal.extend(std::iter::repeat_n('#', hashes.div_ceil(2)));
            rest = after;
            continue;
        };
        literal.extend(std::iter::repeat_n('#', hashes / 2));
        if hashes % 2 == 0 {
            literal.push('[');
            rest = style;
            continue;
        }
        let Some(end) = style.find(']') else {
            rest = "";
            break;
        };
        if !literal.is_empty() {
            pieces.push(Piece::Text(std::mem::take(&mut literal)));
        }
        pieces.push(Piece::Style(&style[..end]));
        rest = &style[end + 1..];
    }
    literal.push_str(rest);
    if !literal.is_empty() {
        pieces.push(Piece::Text(literal));
    }
    pieces
}

/// Split expanded text into `(style, text)` runs, read as
/// [`draw_pieces`]: each `#[…]` updates the running style; text before the
/// first one has the default (empty) style.
pub fn styled(text: &str) -> Vec<(Style, String)> {
    let mut runs = Vec::new();
    let mut style = Style::default();
    for piece in draw_pieces(text) {
        match piece {
            Piece::Text(text) => runs.push((style, text)),
            Piece::Style(spec) => style.apply(spec),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_patterns_are_kept_apart_by_case() {
        // The same pattern asked for with and without `i`, either way round.
        assert!(format_match("ri", "ABC", "abc"));
        assert!(!format_match("r", "ABC", "abc"));
        assert!(format_match("ri", "ABC", "abc"));
        assert!(!format_match("r", "[", "x"), "one that does not compile");
    }

    #[test]
    fn limits_count_no_room_for_styles_as_tmux() {
        assert_eq!(limit("#[fg=red]abcdef", 3, ""), "#[fg=red]abc");
        assert_eq!(limit("ab#[bold]cdef#[default]", 4, ">"), "ab#[bold]cd>");
        assert_eq!(
            limit("#[fg=red]abcdef#[default]", -2, ""),
            "#[fg=red]ef#[default]"
        );
        assert_eq!(limit("#[fg=red]ab", 5, ">"), "#[fg=red]ab");
    }

    #[test]
    fn strftime_gives_week_numbers_and_the_c_locales_date_as_glibc() {
        // Expected values from glibc's strftime (Python's, on Linux).
        let at = |year, month, day, weekday, yearday| crate::localtime::LocalTime {
            year,
            month,
            day,
            hour: 7,
            minute: 8,
            second: 9,
            weekday,
            yearday,
            epoch: 0,
        };
        let f = "%U %W %V %G %g %x";
        assert_eq!(
            strftime(f, &at(2021, 1, 1, 5, 0)),
            "00 00 53 2020 20 01/01/21"
        );
        assert_eq!(
            strftime(f, &at(2024, 12, 30, 1, 364)),
            "52 53 01 2025 25 12/30/24"
        );
        assert_eq!(
            strftime(f, &at(2026, 10, 20, 2, 292)),
            "42 42 43 2026 26 10/20/26"
        );
        assert_eq!(
            strftime(f, &at(2027, 1, 3, 0, 2)),
            "01 00 53 2026 26 01/03/27"
        );
        assert_eq!(
            strftime(f, &at(2020, 12, 31, 4, 365)),
            "52 52 53 2020 20 12/31/20"
        );
        assert_eq!(
            strftime(f, &at(2026, 1, 4, 0, 3)),
            "01 00 01 2026 26 01/04/26"
        );
        // A year starting on a Sunday, then on a Monday: week 1 at once.
        assert_eq!(
            strftime(f, &at(2023, 1, 1, 0, 0)),
            "01 00 52 2022 22 01/01/23"
        );
        assert_eq!(
            strftime(f, &at(2024, 1, 1, 1, 0)),
            "00 01 01 2024 24 01/01/24"
        );
        assert_eq!(strftime("%X", &at(2026, 1, 4, 0, 3)), "07:08:09");
        // The offset is the local clock against the moment: 1970-01-01
        // 00:00 UTC seen as 05:30, and 2026-10-20 12:00 UTC as 07:00.
        let local =
            |year, month, day, hour, minute, weekday, yearday, epoch| crate::localtime::LocalTime {
                hour,
                minute,
                second: 0,
                epoch,
                ..at(year, month, day, weekday, yearday)
            };
        assert_eq!(strftime("%z", &local(1970, 1, 1, 5, 30, 4, 0, 0)), "+0530");
        assert_eq!(
            strftime("%z", &local(2026, 10, 20, 7, 0, 2, 292, 1_792_497_600)),
            "-0500"
        );
        assert_eq!(
            strftime("%z", &local(1969, 12, 31, 19, 0, 3, 364, 0)),
            "-0500"
        );
        // After a leap day.
        assert_eq!(
            strftime("%z", &local(2024, 3, 1, 1, 0, 5, 60, 1_709_251_200)),
            "+0100"
        );
    }

    #[test]
    fn styles_take_the_colour_names_tmux_knows() {
        assert_eq!(parse_colour("orchid"), Some(Colour::Rgb(0xda, 0x70, 0xd6)));
        // round(2.55 * 50) is 127 in double precision, as tmux computes it.
        assert_eq!(parse_colour("grey50"), Some(Colour::Rgb(0x7f, 0x7f, 0x7f)));
        assert_eq!(parse_colour("red"), Some(Colour::Indexed(1)));
        assert_eq!(parse_colour("nonsense"), None);
    }

    #[test]
    fn w_gives_the_width_on_screen() {
        assert_eq!(x("#{w:@u}"), "11");
        assert_eq!(x("#{n:@u}"), "13", "bytes, against columns");
        assert_eq!(x("#{w:#{l:a中b}}"), "4");
        assert_eq!(x("#{w:#{l:#[fg=red]ab##}}"), "3", "a style takes none");
        assert_eq!(x("#{w:@e}"), "0");
    }

    #[test]
    fn draw_pieces_read_hashes_as_tmuxs_format_draw() {
        let text = |t: &str| Piece::Text(t.to_owned());
        assert_eq!(draw_pieces("a##b#c###"), [text("a#b#c##")]);
        // Before `[`: even, half and a literal `[`; odd, half and a style.
        assert_eq!(draw_pieces("x##[y"), [text("x#[y")]);
        assert_eq!(
            draw_pieces("x###[fg=red]y"),
            [text("x#"), Piece::Style("fg=red"), text("y")]
        );
        assert_eq!(draw_pieces("#[bold]"), [Piece::Style("bold")]);
        // A style with no `]` ends the text.
        assert_eq!(draw_pieces("a#[bold"), [text("a")]);
    }

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
                "@wide" => "日本語",
                _ => return None,
            }
            .to_owned(),
        )
    }

    fn x(f: &str) -> String {
        expand(f, &ctx)
    }

    /// Each as tmux 3.6's display-message -p printed it (2026-10-29).
    #[test]
    fn escapes_open_substitutions_and_cuts_by_width_are_tmuxs() {
        assert_eq!(x("#{==:a#,b,a#,b}"), "1", "#, is no comma");
        assert_eq!(x("#{s/a/o:session_name}"), "moin", "no closing /");
        assert_eq!(x("#{s/h/H;s/k/K:@w}"), "HjKl");
        assert_eq!(x("#{s/h/H/:@w}"), "Hjkl", "with it");
        assert_eq!(x("#{=3:@wide}"), "日", "columns, not characters");
        assert_eq!(x("#{=-3:@wide}"), "語");
        assert_eq!(x("#{=4:@wide}"), "日本");
        assert_eq!(x("#{=0:@w}"), "hjkl", "0 cuts nothing");
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
            (r"#{R:ab,3}", r"ababab"),
            (r"#{R:x,#{e|+|:1,1}}", r"xx"),
            (r"#{R:x,0}", r""),
            (r"#{R:x,10001}", r""),
            (r"#{R:x,y}", r""),
            (r"#{R:x}", r""),
            (r"#{R:a,b,2}", r""),
            (r"#{R:,3}", r""),
            (r"#{!:window_active}", r"0"),
            (r"#{!:0}", r"1"),
            (r"#{!:}", r"1"),
            (r"#{!!:abc}", r"1"),
            (r"#{!!:0}", r"0"),
        ] {
            assert_eq!(x(format), tmux, "{format}");
        }
    }

    #[test]
    fn n_without_sessions_to_ask_is_empty() {
        assert_eq!(x("[#{N:main}][#{N/s:main}]"), "[][]");
        // With neither w nor s, an ordinary lookup.
        assert_eq!(x("#{N/x:session_name}"), "main");
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
        assert_eq!(
            x("#{&&:1,1,#{zero}} #{||:0,0,1}"),
            "0 1",
            "any number of parts"
        );
        assert_eq!(x("#{==:a,a,a}"), "0", "the right side is all the rest");
        // A syntax error ends the expansion holding it, as tmux's.
        assert_eq!(x("A#{==:a}B"), "A");
        assert_eq!(x("A#{m:a}B#{R:x}"), "A");
        assert_eq!(x("A#{R:x}B"), "A");
        assert_eq!(
            x("A#{?#{<:a},x,y}B"),
            "AyB",
            "an inner one ends only its own"
        );
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
    fn expressions_and_characters_are_tmuxs() {
        assert_eq!(x("#{e|+|:1,2}"), "3");
        assert_eq!(x("#{e|-|:1,5}"), "-4");
        assert_eq!(x("#{e|*|:3,4}"), "12");
        assert_eq!(x("#{e|/|:7,2}"), "3", "whole numbers");
        assert_eq!(x("#{e|/|:-7,2}"), "-3", "towards zero");
        assert_eq!(x("#{e|/|f|:7,2}"), "3.50");
        assert_eq!(x("#{e|/|f|4:7,2}"), "3.5000");
        assert_eq!(x("#{e|%|:7,3}"), "1");
        assert_eq!(x("#{e|m|:7,3}"), "1");
        assert_eq!(x("#{e|>|:10,9}"), "1", "as numbers, not strings");
        assert_eq!(x("#{e|==|:2.9,2}"), "1", "whole: 2.9 is 2");
        assert_eq!(x("#{e|<=|f|:2.9,2}"), "0.00");
        assert_eq!(x("#{e|+|:#{n:session_name},1}"), "5");
        assert_eq!(x("#{e|+|:x,1}"), "", "not a number");
        assert_eq!(x("#{e|^|:1,1}"), "", "no such operator");
        assert_eq!(x("#{a:65}"), "A");
        assert_eq!(x("#{a:#{e|+|:96,1}}"), "a");
        assert_eq!(x("#{c:colour110}"), "87afd7");
        assert_eq!(
            x("#{c:#{e|+|:200,10}}"),
            "",
            "a number past 97 names no colour"
        );
        assert_eq!(x("#{c:nonsense}"), "");
        assert_eq!(x("#{a:10}"), "", "not printable");
        assert_eq!(x("#{e|+|f|-1:1,2}"), "3.000000", "C's default");
        let long = x("#{e|+|f|70000:1,2}");
        assert_eq!(long.len(), 70002, "past what Rust's formatter takes");
        assert!(long.starts_with("3.000") && long.ends_with("000"));
    }

    /// Past what Rust formats itself, the digits are still exact.
    #[test]
    fn a_long_precision_keeps_the_exact_digits() {
        let third = fixed(1.0 / 3.0, 1200);
        assert_eq!(&third[..20], "0.333333333333333314");
        assert_eq!(third.len(), 1202);
        assert_eq!(third, format!("{:.1200}", 1.0 / 3.0));
        assert_eq!(fixed(f64::INFINITY, 70000), "inf");
    }

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
        assert_eq!(x("#{p99999999999:@e}|"), "|", "past a C int: none");
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

    /// A context whose jobs print their expanded command, bracketed, and
    /// whose variables are `v` and `out`.
    struct Jobs;

    impl Context for Jobs {
        fn get(&self, name: &str) -> Option<String> {
            match name {
                "v" => Some("V".into()),
                "out" => Some("#(nested)".into()),
                _ => None,
            }
        }

        fn name_exists(&self, _kind: NameKind, _name: &str) -> Option<bool> {
            None
        }

        fn pane_lines(&self) -> Option<Vec<String>> {
            None
        }

        fn loop_items(&self, _kind: LoopKind) -> Option<Vec<LoopItem<'_>>> {
            None
        }

        fn job(&self, cmd: &str, expanded: &str) -> Option<String> {
            Some(if cmd == "show" {
                "#{v}#(again)".into()
            } else {
                format!("[{expanded}]")
            })
        }
    }

    #[test]
    fn jobs_run_their_expanded_command_as_tmux() {
        let x = |f: &str| expand(f, &Jobs);
        assert_eq!(x("a#(echo #{v})b"), "a[echo V]b");
        // Parentheses are counted to find the end.
        assert_eq!(x("x#(echo a(b)c)y"), "x[echo a(b)c]y");
        // What it printed is a format, with no jobs in it.
        assert_eq!(x("<#(show)>"), "<V>");
        // A job's command runs no jobs in it.
        assert_eq!(x("#(echo x#(inner)y)"), "[echo xy]");
        // A variable's value goes into the command as it is.
        assert_eq!(
            x("#(echo #{out})"),
            "[echo ]",
            "printed, its #() runs nothing"
        );
        // A context that runs none gives nothing.
        assert_eq!(expand("a#(echo hi)b", &ctx), "ab");
    }

    #[test]
    fn styles_take_tmuxs_other_attributes() {
        let mut st = Style::default();
        st.apply("strikethrough,blink hidden,curly-underscore");
        assert_eq!(
            (st.strikethrough, st.blink, st.hidden, st.underscore),
            (Some(true), Some(true), Some(true), Some(true))
        );
        st.apply("nostrikethrough");
        assert_eq!(st.strikethrough, Some(false));
        st.apply("blink");
        st.apply("default");
        assert_eq!(st.blink, Some(false), "default takes them all off");
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
