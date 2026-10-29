//! `source-file` of a tmux command file (any but a `.toml` one), run as
//! tmux's: each statement's commands through the parser and the executor,
//! with `%if` / `%elif` / `%else` / `%endif`, `%hidden` and `NAME=VALUE`
//! handled as the file is read (tmux's `cmd-parse.y`).

use std::path::Path;

use super::{Ctx, expand_for, run_detached};
use crate::server::Server;

/// Run the tmux command file at `path`, returning each error as tmux
/// reports one: `path:line: error` for one in reading it, the command's own
/// for one as it runs. As tmux's, the whole file is read
/// first: `%if`, `%hidden` and `NAME=VALUE` take effect as it is, and a
/// command that does not parse (an unknown one, a bad flag) means none of
/// the file's commands run. A command that fails as it runs stops the rest
/// of its statement (a line, or a `{ … }` block over several), not the file.
pub(super) fn run_command_file(srv: &mut Server, ctx: &Ctx, path: &Path) -> Vec<String> {
    // A file that sources itself stops here, as tmux's
    // `CMD_SOURCE_FILE_DEPTH_LIMIT`, not when the stack runs out.
    if srv.source_depth >= SOURCE_DEPTH_LIMIT {
        return vec!["too many nested files".into()];
    }
    srv.source_depth += 1;
    let errors = read_and_run(srv, ctx, path);
    srv.source_depth -= 1;
    errors
}

/// How deep `source-file` may nest, as tmux's.
const SOURCE_DEPTH_LIMIT: usize = 50;

fn read_and_run(srv: &mut Server, ctx: &Ctx, path: &Path) -> Vec<String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return vec![format!("{}: {e}", path.display())],
    };
    let mut file = CommandFile {
        path,
        conditions: Vec::new(),
        errors: Vec::new(),
        parsed: Vec::new(),
    };
    // A statement still open in a `{` block: where it started, its text, how
    // many blocks are open, and whether it runs (it may be in a false
    // branch, which is skipped as a whole).
    let mut open: Option<(usize, String, usize, bool)> = None;
    for (n, line) in statements(&text) {
        if let Some((start, mut text, depth, runs)) = open.take() {
            text.push('\n');
            text.push_str(&line);
            match depth.checked_add_signed(braces(&line)).filter(|d| *d > 0) {
                Some(depth) => open = Some((start, text, depth, runs)),
                None if runs => file.read(srv, start, &text),
                None => {}
            }
            continue;
        }
        if file.directive(srv, ctx, n, line.trim()) {
            continue;
        }
        let runs = file.active();
        match usize::try_from(braces(&line)).ok().filter(|d| *d > 0) {
            Some(depth) => open = Some((n, line, depth, runs)),
            None if runs => file.read(srv, n, &line),
            None => {}
        }
    }
    if let Some((start, ..)) = open {
        file.error(start, "missing }");
    }
    if !file.conditions.is_empty() {
        file.error(text.lines().count(), "%if without %endif");
    }
    if file.errors.is_empty() {
        for cmds in std::mem::take(&mut file.parsed) {
            let out = run_detached(srv, ctx, &cmds);
            // An error as it runs is the command's own, with no place in
            // the file, as tmux's.
            if out.status != 0 {
                file.errors.push(out.stderr.trim().to_owned());
            }
        }
    }
    file.errors
}

/// One `%if` level: whether its current branch runs, and whether one of its
/// branches has (so a later `%elif` or `%else` does not).
struct Condition {
    runs: bool,
    taken: bool,
    /// Whether the level around it runs at all.
    outer: bool,
}

struct CommandFile<'a> {
    path: &'a Path,
    conditions: Vec<Condition>,
    errors: Vec<String>,
    /// Each statement read, to run once the whole file has parsed.
    parsed: Vec<Vec<tmxr_command::Line>>,
}

impl CommandFile<'_> {
    fn active(&self) -> bool {
        self.conditions.last().is_none_or(|c| c.runs)
    }

    fn error(&mut self, line: usize, e: &str) {
        self.errors
            .push(format!("{}:{line}: {e}", self.path.display()));
    }

    /// Handle `line` if it is a `%` directive, as tmux's parser does at read
    /// time; false for any other line.
    fn directive(&mut self, srv: &mut Server, ctx: &Ctx, n: usize, line: &str) -> bool {
        let (word, rest) = line.split_once([' ', '\t']).unwrap_or((line, ""));
        let rest = rest.trim();
        // A `%` with only digits after it is a word, as tmux's lexer has it.
        if !word.starts_with('%') || word[1..].bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        match word {
            "%if" => {
                let outer = self.active();
                let runs = outer && self.truth(srv, ctx, n, rest);
                self.conditions.push(Condition {
                    runs,
                    taken: runs,
                    outer,
                });
            }
            "%elif" | "%else" => {
                let Some(c) = self.conditions.last() else {
                    self.error(n, &format!("{word} without %if"));
                    return true;
                };
                let (outer, taken) = (c.outer, c.taken);
                let runs = outer && !taken && (word == "%else" || self.truth(srv, ctx, n, rest));
                if let Some(c) = self.conditions.last_mut() {
                    c.runs = runs;
                    c.taken |= runs;
                }
            }
            "%endif" => {
                if self.conditions.pop().is_none() {
                    self.error(n, "%endif without %if");
                }
            }
            "%hidden" => {
                if self.active() {
                    match self.word(srv, n, rest).and_then(|w| assignment(&w)) {
                        Some((name, value)) => srv.global_env.set_hidden(&name, value),
                        None => self.error(n, "%hidden takes NAME=VALUE"),
                    }
                }
            }
            _ => self.error(n, &format!("unknown directive: {word}")),
        }
        true
    }

    /// The single word `text` reads as (quotes taken off, `$NAME` filled in).
    fn word(&mut self, srv: &Server, n: usize, text: &str) -> Option<String> {
        match tmxr_command::tokenize(text, &|k| lookup(srv, k)) {
            Ok(cmds) => cmds.into_iter().flatten().next(),
            Err(e) => {
                self.error(n, &e.to_string());
                None
            }
        }
    }

    /// A condition's format expanded and read as tmux's format_true: true
    /// unless it is empty or `0`.
    fn truth(&mut self, srv: &mut Server, ctx: &Ctx, n: usize, text: &str) -> bool {
        let format = self.word(srv, n, text).unwrap_or_default();
        let value = expand_for(srv, ctx, ctx.pane, &format);
        !value.is_empty() && value != "0"
    }

    /// Read one statement's commands, checking each parses, a `NAME=VALUE`
    /// before a command setting that variable as it is read, as tmux's.
    fn read(&mut self, srv: &mut Server, n: usize, text: &str) {
        let lines = match tmxr_command::tokenize_lines(text, &|k| lookup(srv, k)) {
            Ok(lines) => lines,
            Err(e) => return self.error(n, &e.to_string()),
        };
        let mut cmds = Vec::new();
        for mut line in lines {
            while let Some((name, value)) = line.words.first().and_then(|w| assignment(w)) {
                srv.global_env.set(&name, Some(value));
                line.words.remove(0);
                line.blocks.remove(0);
            }
            if line.words.is_empty() {
                continue;
            }
            match tmxr_command::parse_line(&line) {
                Ok(_) => cmds.push(line),
                Err(e) => self.error(n, &e.to_string()),
            }
        }
        if !cmds.is_empty() {
            self.parsed.push(cmds);
        }
    }
}

/// A variable as a command file reads it: the server's global environment
/// (hidden variables included, as tmux's), else the server's own.
fn lookup(srv: &Server, name: &str) -> Option<String> {
    match srv.global_env.lookup(name) {
        Some(value) => value.map(str::to_owned),
        None => std::env::var(name).ok(),
    }
}

/// `NAME=VALUE`, with a name tmux takes for a variable: a letter or `_`,
/// then letters, digits and `_`.
fn assignment(word: &str) -> Option<(String, String)> {
    let (name, value) = word.split_once('=')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    ((first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_'))
    .then(|| (name.to_owned(), value.to_owned()))
}

/// The file's lines, a line ending in `\` joined with the next as tmux's
/// lexer joins them, each with the number of its first line.
fn statements(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut joined: Option<(usize, String)> = None;
    for (i, line) in text.lines().enumerate() {
        let (start, mut acc) = joined.take().unwrap_or((i + 1, String::new()));
        match line.strip_suffix('\\') {
            Some(head) if !head.ends_with('\\') => {
                acc.push_str(head);
                joined = Some((start, acc));
            }
            _ => {
                acc.push_str(line);
                out.push((start, acc));
            }
        }
    }
    out.extend(joined);
    out
}

/// How many `{` blocks `line` opens, less those it closes: braces outside
/// quotes and comments that stand as words of their own (a `#{format}` is
/// not a block).
fn braces(line: &str) -> isize {
    let mut depth = 0;
    let mut quote = None;
    let mut chars = line.chars().peekable();
    let mut at_word_start = true;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => {
                chars.next();
            }
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '\\') => {
                chars.next();
            }
            (None, '#') if at_word_start && chars.peek() != Some(&'{') => break,
            (None, '{') if at_word_start => depth += 1,
            (None, '}')
                if at_word_start && chars.peek().is_none_or(|n| n.is_whitespace() || *n == ';') =>
            {
                depth -= 1;
            }
            _ => {}
        }
        at_word_start = quote.is_none() && (c.is_whitespace() || c == ';');
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn braces_count_only_blocks() {
        assert_eq!(braces("bind x {"), 1);
        assert_eq!(braces("}"), -1);
        assert_eq!(braces("bind x { a }"), 0);
        assert_eq!(braces("set -g status-left #{session_name}"), 0);
        assert_eq!(braces("set -g x '{' # {"), 0);
        assert_eq!(braces("display \"{\""), 0);
    }

    #[test]
    fn a_line_ending_in_a_backslash_goes_on() {
        assert_eq!(
            statements("a \\\nb\nc\\\\\nd"),
            [
                (1, "a b".to_owned()),
                (3, "c\\\\".to_owned()),
                (4, "d".to_owned())
            ]
        );
    }

    #[test]
    fn assignments_take_tmuxs_names() {
        assert_eq!(assignment("A_1=x=y"), Some(("A_1".into(), "x=y".into())));
        assert_eq!(assignment("1A=x"), None);
        assert_eq!(assignment("set"), None);
        assert_eq!(assignment("a-b=x"), None);
    }
}
