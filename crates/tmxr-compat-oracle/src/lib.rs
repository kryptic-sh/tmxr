//! Runs the same steps in tmux and in tmxr and diffs what each reports, for
//! tmux-compatibility regression testing (as `hjkl-compat-oracle` does for
//! hjkl against nvim).
//!
//! A case (TOML, `corpus/`) starts a session in each multiplexer, optionally
//! attaches a client on a pseudo-terminal, runs its steps (commands, keys,
//! mouse events at cells or at text on the client's screen), then compares
//! its checks (a format, a command's output, text on the screen) between the
//! two. A check may also carry the value it must have, which is how the
//! corpus still guards tmxr where tmux is not installed (Windows).

mod driver;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;

pub use driver::{Client, Server, Side};

/// A corpus file.
#[derive(Debug, Deserialize)]
pub struct Corpus {
    pub cases: Vec<Case>,
}

/// One comparison.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub name: String,
    /// The client's (or a detached session's) columns and rows.
    #[serde(default = "default_size")]
    pub size: (u16, u16),
    /// The session's program, run by `/bin/sh -c`: Unix only. The default
    /// is the shell.
    #[serde(default)]
    pub program: Option<String>,
    /// Attach a client (needed for keys, the mouse and screen checks).
    #[serde(default)]
    pub attach: bool,
    #[serde(default)]
    pub steps: Vec<Step>,
    pub checks: Vec<Check>,
    /// Needs a Unix shell in its panes: never run against tmxr alone on
    /// Windows.
    #[serde(default)]
    pub unix_only: bool,
}

const fn default_size() -> (u16, u16) {
    (100, 30)
}

/// Something done to both multiplexers.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    /// A command line, split as a shell would.
    Run(String),
    /// A command line left running, for commands that wait (tmux's
    /// `display-popup` and `display-menu` return when they close).
    Spawn(String),
    /// Bytes typed into the client.
    Keys(String),
    /// A mouse event at a place on the client's screen.
    Mouse(Mouse),
    /// Wait this many seconds.
    Sleep(f64),
    /// Wait until the client's screen shows this text.
    WaitText(String),
}

/// A mouse event: `click`, `down`, `up`, `drag` (from `at` to `to`),
/// `wheel-up` or `wheel-down`, with button 1 (left), 2 (middle) or 3
/// (right).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mouse {
    pub action: String,
    #[serde(default = "default_button")]
    pub button: u8,
    pub at: At,
    #[serde(default)]
    pub to: Option<At>,
}

const fn default_button() -> u8 {
    1
}

/// A cell on the client's screen: `[col, row]`, or where `text` first shows
/// (on `row`, if given; -1 is the last row) plus `dx` columns.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum At {
    Cell([u16; 2]),
    Text {
        text: String,
        #[serde(default)]
        row: Option<i32>,
        #[serde(default)]
        dx: u16,
    },
}

/// Something compared between the two.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Check {
    /// `display-message -p` of a format in the session.
    Format {
        format: String,
        #[serde(default)]
        expect: Option<String>,
    },
    /// A command's output.
    Command {
        command: String,
        #[serde(default)]
        expect: Option<String>,
    },
    /// Whether the client's screen shows this text.
    Screen {
        screen_contains: String,
        #[serde(default)]
        expect: Option<bool>,
    },
    /// Where this text first shows on the client's screen: `col,row`, or
    /// `none`.
    Find {
        screen_find: String,
        #[serde(default)]
        expect: Option<String>,
    },
}

impl Check {
    fn describe(&self) -> String {
        match self {
            Self::Format { format, .. } => format!("format {format}"),
            Self::Command { command, .. } => format!("command {command}"),
            Self::Screen {
                screen_contains, ..
            } => format!("screen contains {screen_contains:?}"),
            Self::Find { screen_find, .. } => format!("screen position of {screen_find:?}"),
        }
    }

    fn expected(&self) -> Option<String> {
        match self {
            Self::Format { expect, .. }
            | Self::Command { expect, .. }
            | Self::Find { expect, .. } => expect.clone(),
            Self::Screen { expect, .. } => expect.map(|e| e.to_string()),
        }
    }
}

/// How a case went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Pass,
    /// tmux and tmxr disagree.
    Diverge {
        check: String,
        tmux: String,
        tmxr: String,
    },
    /// A side disagrees with the value the case expects.
    Unexpected {
        check: String,
        side: Side,
        expected: String,
        got: String,
    },
    Skipped(String),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct CaseResult {
    pub name: String,
    pub status: Status,
}

/// Read a corpus file.
pub fn load_corpus(path: &Path) -> Result<Corpus, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The tmux to compare against: `$TMUX_BIN`, if it runs. Never just any
/// tmux on PATH: the corpus is measured against one version (CI pins it),
/// and another would differ for its own reasons.
pub fn tmux_program() -> Option<PathBuf> {
    let program = PathBuf::from(std::env::var_os("TMUX_BIN")?);
    std::process::Command::new(&program)
        .arg("-V")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| program)
}

/// The tmxr under test: `$TMXR_BIN`, else the workspace's debug build.
pub fn tmxr_program() -> PathBuf {
    std::env::var_os("TMXR_BIN").map_or_else(
        || {
            let exe = format!("tmxr{}", std::env::consts::EXE_SUFFIX);
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug")
                .join(exe)
        },
        PathBuf::from,
    )
}

/// The tmux config the tmux side runs with.
pub fn tmux_conf() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("corpus/tmux.conf")
}

/// Run every case of `corpus`: against tmux and tmxr when tmux is given,
/// else tmxr alone against the cases' expected values.
pub fn run_corpus(corpus: &Corpus, tmux: Option<&Path>, tmxr: &Path) -> Vec<CaseResult> {
    corpus
        .cases
        .iter()
        .map(|case| CaseResult {
            name: case.name.clone(),
            status: run_case(case, tmux, tmxr),
        })
        .collect()
}

/// How long a check waits for the two to agree: tmxr and tmux each handle
/// input on their own time.
const SETTLE: Duration = Duration::from_secs(3);

fn run_case(case: &Case, tmux: Option<&Path>, tmxr: &Path) -> Status {
    if tmux.is_none() && (case.unix_only || cfg!(windows) && case.program.is_some()) {
        return Status::Skipped("needs tmux and a Unix shell".into());
    }
    if tmux.is_none() && case.checks.iter().all(|c| c.expected().is_none()) {
        return Status::Skipped("no tmux, and no expected values".into());
    }
    let mut sides = Vec::new();
    if let Some(t) = tmux {
        match Server::start(Side::Tmux, t, case) {
            Ok(s) => sides.push(s),
            Err(e) => return Status::Error(format!("tmux: {e}")),
        }
    }
    match Server::start(Side::Tmxr, tmxr, case) {
        Ok(s) => sides.push(s),
        Err(e) => return Status::Error(format!("tmxr: {e}")),
    }
    for step in &case.steps {
        for s in &mut sides {
            if let Err(e) = s.step(step) {
                return Status::Error(format!("{:?}: {step:?}: {e}", s.side));
            }
        }
    }
    for check in &case.checks {
        if let Some(status) = compare(&mut sides, check) {
            return status;
        }
    }
    Status::Pass
}

/// Read `check` from every side, again until they agree (and match the
/// expected value) or [`SETTLE`] passes.
fn compare(sides: &mut [Server], check: &Check) -> Option<Status> {
    let deadline = Instant::now() + SETTLE;
    loop {
        let values: Vec<Result<String, String>> = sides.iter_mut().map(|s| s.read(check)).collect();
        let outcome = judge(sides, check, &values);
        if outcome.is_none() || Instant::now() >= deadline {
            return outcome;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn judge(sides: &[Server], check: &Check, values: &[Result<String, String>]) -> Option<Status> {
    let name = check.describe();
    let mut got = Vec::new();
    for (s, v) in sides.iter().zip(values) {
        match v {
            Ok(v) => got.push((s.side, v.clone())),
            Err(e) => return Some(Status::Error(format!("{:?}: {name}: {e}", s.side))),
        }
    }
    if let Some(expected) = check.expected() {
        for (side, v) in &got {
            if *v != expected {
                return Some(Status::Unexpected {
                    check: name,
                    side: *side,
                    expected,
                    got: v.clone(),
                });
            }
        }
    }
    if let [(_, tmux), (_, tmxr)] = got.as_slice()
        && tmux != tmxr
    {
        return Some(Status::Diverge {
            check: name,
            tmux: tmux.clone(),
            tmxr: tmxr.clone(),
        });
    }
    None
}

/// Split a command line into words as a shell would: whitespace separates,
/// `'…'` is literal, `"…"` allows `\"` and `\\`.
pub fn split_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    word.push(c);
                }
            }
            '"' => {
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => word.extend(chars.next()),
                        c => word.push(c),
                    }
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_as_a_shell_would() {
        assert_eq!(
            split_words(r#"list-panes -F '#{pane_index} x' -t "a b""#),
            ["list-panes", "-F", "#{pane_index} x", "-t", "a b"]
        );
        assert_eq!(split_words("  "), Vec::<String>::new());
        assert_eq!(split_words(r#"set @x "q\"uote""#), ["set", "@x", "q\"uote"]);
    }
}
