//! Command execution. Each command family lives in its own module; `run_one`
//! hands a parsed command to each in turn.

mod binds;
mod buffer;
mod env;
mod options;
mod pane;
mod prompt;
mod session;
mod wait;
mod window;

use std::path::PathBuf;

use crossterm::event::KeyEvent;
use tmxr_command::format::expand;
use tmxr_command::{Args, Parsed};

use crate::layout::Dir;
use crate::model::{ClientId, PaneId, SessionId};
use crate::server::{STATUS_ROWS, Server};
use crate::vars::Vars;

/// Who is running a command and on whose behalf.
#[derive(Debug, Clone, Default)]
pub struct Ctx {
    /// The client that sent the command (attached or not).
    pub client: Option<ClientId>,
    /// The pane commands default to (the pane a key was pressed in, or the
    /// pane a command client runs inside).
    pub pane: Option<PaneId>,
    /// The key that triggered a bind, for commands that may pass it on.
    pub key: Option<KeyEvent>,
    /// The command client's working directory.
    pub cwd: Option<PathBuf>,
    /// The command client's environment.
    pub env: Vec<(String, String)>,
    /// The mouse event that triggered a bind: the `=` target, and what
    /// `send-keys -M`, `copy-mode -M` and `resize-pane -M` act on.
    pub mouse: Option<crate::mouse::MouseTarget>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
    /// Attach the client to this session when it can.
    pub attach: Option<SessionId>,
    /// `wait-for`: hold the command client's reply until this happens.
    pub wait: Option<crate::waits::Wait>,
}

impl Outcome {
    pub fn error(msg: String) -> Self {
        Self {
            status: 1,
            stderr: msg,
            ..Self::default()
        }
    }
}

type Res = Result<(), String>;

/// Run a command line written in tmux's language (a bind, the prompt).
pub fn run_string(srv: &mut Server, ctx: &Ctx, line: &str) -> Outcome {
    let env = |k: &str| std::env::var(k).ok();
    match tmxr_command::tokenize(line, &env) {
        // Only a command client has a reply to hold back.
        Ok(cmds) => match run_list(srv, ctx, &cmds) {
            out if out.wait.is_some() => {
                Outcome::error("wait-for: only a command client can wait".into())
            }
            out => out,
        },
        Err(e) => Outcome::error(e.to_string()),
    }
}

/// Run an argv from the command line, where a `;` word separates commands.
pub fn run_argv_list(srv: &mut Server, ctx: &Ctx, argv: &[String]) -> Outcome {
    let cmds: Vec<Vec<String>> = argv
        .split(|w| w == ";")
        .filter(|c| !c.is_empty())
        .map(<[String]>::to_vec)
        .collect();
    run_list(srv, ctx, &cmds)
}

/// Run commands in order, stopping at the first error.
pub fn run_list(srv: &mut Server, ctx: &Ctx, cmds: &[Vec<String>]) -> Outcome {
    let mut out = Outcome::default();
    for argv in cmds {
        let parsed = match tmxr_command::parse(argv) {
            Ok(p) => p,
            Err(e) => {
                out.status = 1;
                out.stderr.push_str(&e.to_string());
                return out;
            }
        };
        if let Err(e) = run_one(srv, ctx, &parsed, &mut out) {
            out.status = 1;
            out.stderr.push_str(&e);
            return out;
        }
        if out.wait.is_some() && !std::ptr::eq(argv, cmds.last().expect("in cmds")) {
            out.wait = None;
            out.status = 1;
            out.stderr.push_str("wait-for must end its command list");
            return out;
        }
    }
    out
}

fn attached_client(srv: &Server, ctx: &Ctx) -> Option<ClientId> {
    ctx.client
        .filter(|c| srv.clients.get(c).is_some_and(|c| c.att.is_some()))
}

fn client_size(srv: &Server, ctx: &Ctx) -> (u16, u16) {
    let from_client = ctx.client.and_then(|c| srv.clients.get(&c)).and_then(|c| {
        c.att.as_ref().map(|a| (a.cols, a.rows)).or_else(|| {
            c.hello
                .as_ref()?
                .terminal
                .as_ref()
                .map(|t| (t.cols, t.rows))
        })
    });
    let (cols, rows) = from_client.unwrap_or((80, 24));
    (cols.max(2), rows.saturating_sub(STATUS_ROWS).max(2))
}

fn expand_for(srv: &Server, ctx: &Ctx, pane: Option<PaneId>, s: &str) -> String {
    expand(s, &Vars::for_pane(srv, pane.or(ctx.pane), ctx.client))
}

fn cwd_arg(srv: &Server, ctx: &Ctx, pane: Option<PaneId>, a: &Args) -> Option<PathBuf> {
    a.value('c')
        .map(|c| PathBuf::from(expand_for(srv, ctx, pane, c)))
}

fn session_env(srv: &Server, ctx: &Ctx) -> Vec<(String, String)> {
    let wanted = &srv.cfg.update_environment;
    let from = |k: &str| -> Option<String> {
        ctx.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    };
    wanted
        .iter()
        .filter_map(|k| from(k).map(|v| (k.clone(), v)))
        .collect()
}

fn dir_flag(a: &Args) -> Option<Dir> {
    if a.has('L') {
        Some(Dir::Left)
    } else if a.has('R') {
        Some(Dir::Right)
    } else if a.has('U') {
        Some(Dir::Up)
    } else if a.has('D') {
        Some(Dir::Down)
    } else {
        None
    }
}

fn on_off(value: Option<&str>, current: bool) -> Result<bool, String> {
    match value {
        None | Some("toggle") => Ok(!current),
        Some("on" | "1" | "yes" | "true") => Ok(true),
        Some("off" | "0" | "no" | "false") => Ok(false),
        Some(v) => Err(format!("bad value: {v}")),
    }
}

/// Quote words so they survive another trip through the tokenizer.
pub fn join_args(words: &[String]) -> String {
    words
        .iter()
        .map(|w| {
            if !w.is_empty()
                && w.chars()
                    .all(|c| c.is_alphanumeric() || "-_./:%@#{}=+,^".contains(c))
                && !w.starts_with('#')
            {
                w.clone()
            } else {
                format!("'{}'", w.replace('\'', r"'\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn run_one(srv: &mut Server, ctx: &Ctx, p: &Parsed, out: &mut Outcome) -> Res {
    for family in [
        session::run,
        window::run,
        pane::run,
        binds::run,
        buffer::run,
        env::run,
        options::run,
        prompt::run,
        wait::run,
    ] {
        if family(srv, ctx, p, out)? {
            return Ok(());
        }
    }
    Err(format!("{}: not implemented", p.name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_args_survives_a_round_trip() {
        let words: Vec<String> = ["split-window", "-c", "#{pane_current_path}", "it's", ""]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let line = join_args(&words);
        let back = tmxr_command::tokenize(&line, &|_| None).unwrap();
        assert_eq!(back, vec![words]);
    }
}
