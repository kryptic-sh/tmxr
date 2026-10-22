//! Command execution. Each command family lives in its own module; `run_one`
//! hands a parsed command to each in turn.

mod access;
mod binds;
mod buffer;
mod env;
mod hook;
mod options;
mod pane;
mod prompt;
mod scoped;
mod session;
mod source;
mod wait;
mod window;

pub use binds::bind_line;
pub use options::option_lines;
pub(crate) use options::{SERVER_OPTIONS, server_option};
pub use tmxr_command::join_args;

use std::path::PathBuf;

use crossterm::event::KeyEvent;
use tmxr_command::format::expand;
use tmxr_command::{Args, Parsed};

use crate::layout::Dir;
use crate::model::{ClientId, PaneId, SessionId};
use crate::server::Server;
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
    /// `run-shell`, `wait-for`: the rest of the list, and the reply, wait
    /// for this job.
    pub job: Option<crate::jobs::Job>,
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
    match tokenize(line) {
        Ok(cmds) => run_detached(srv, ctx, &cmds),
        Err(e) => Outcome::error(e),
    }
}

/// Run a command line as part of the list that runs it (an `if-shell`
/// branch): a `run-shell` job in it is left in the outcome, for the outer
/// list to wait on.
pub fn run_line(srv: &mut Server, ctx: &Ctx, line: &str) -> Outcome {
    match tokenize(line) {
        Ok(cmds) => run_list(srv, ctx, &cmds),
        Err(e) => Outcome::error(e),
    }
}

fn tokenize(line: &str) -> Result<Vec<tmxr_command::Line>, String> {
    let env = |k: &str| std::env::var(k).ok();
    tmxr_command::tokenize_lines(line, &env).map_err(|e| e.to_string())
}

/// Run commands with no command client waiting on them: a `run-shell` job
/// goes on by itself, its output shown on `ctx.client`.
pub fn run_detached(srv: &mut Server, ctx: &Ctx, cmds: &[tmxr_command::Line]) -> Outcome {
    let mut out = run_list(srv, ctx, cmds);
    if let Some(job) = out.job.take() {
        srv.start_job(
            job,
            crate::jobs::Then {
                ctx: ctx.clone(),
                reply: None,
            },
        );
    }
    out
}

/// Run an argv from the command line, separated into commands as tmux's
/// cmd_parse_from_arguments ([`tmxr_command::split_arguments`]).
pub fn run_argv_list(srv: &mut Server, ctx: &Ctx, argv: &[String]) -> Outcome {
    let cmds: Vec<tmxr_command::Line> = tmxr_command::split_arguments(argv)
        .into_iter()
        .map(Into::into)
        .collect();
    run_list(srv, ctx, &cmds)
}

/// Run commands in order, stopping at the first error.
pub fn run_list(srv: &mut Server, ctx: &Ctx, cmds: &[tmxr_command::Line]) -> Outcome {
    let mut out = Outcome::default();
    for (i, line) in cmds.iter().enumerate() {
        let parsed = match tmxr_command::parse_line(line) {
            Ok(p) => p,
            Err(e) => {
                out.status = 1;
                out.stderr.push_str(&e.to_string());
                return out;
            }
        };
        let ran = run_one(srv, ctx, &parsed, &mut out);
        // What one member of a session group changed reaches the others
        // before the next command, as tmux's.
        srv.sync_groups();
        if let Err(e) = ran {
            out.status = 1;
            out.stderr.push_str(&e);
            // A command that failed as it ran (not one that did not parse)
            // fires command-error, as tmux's cmdq_fire_command.
            let scope = hook_scope(srv, ctx, &parsed);
            srv.queue_hook("command-error", ctx.clone(), scope);
            return out;
        }
        let scope = hook_scope(srv, ctx, &parsed);
        srv.queue_hook(&format!("after-{}", parsed.name()), ctx.clone(), scope);
        // run-shell, wait-for: the rest of the list waits for the job.
        if let Some(job) = out.job.as_mut() {
            // After what an inner list (an if-shell branch) left waiting.
            job.rest.extend_from_slice(&cmds[i + 1..]);
            return out;
        }
    }
    out
}

/// Where a command acted, for its hooks, by its -t target: its pane, window
/// or session, as far as the target names one.
fn hook_scope(srv: &Server, ctx: &Ctx, parsed: &Parsed) -> crate::hooks::HookScope {
    use crate::hooks::HookScope;
    parsed.args.value('t').map_or_else(Default::default, |t| {
        crate::target::pane(srv, ctx, Some(t))
            .map(|(s, w, p)| HookScope {
                session: Some(s),
                window: Some(w),
                pane: Some(p),
            })
            .or_else(|_| {
                crate::target::window(srv, ctx, Some(t)).map(|(s, _, w)| HookScope {
                    session: Some(s),
                    window: Some(w),
                    pane: None,
                })
            })
            .or_else(|_| {
                crate::target::session(srv, ctx, Some(t)).map(|s| HookScope {
                    session: Some(s),
                    ..HookScope::default()
                })
            })
            .unwrap_or_default()
    })
}

/// A window a command made (`new-window`, `break-pane`) has the command's
/// target client as its latest, quietly, as tmux's spawn_window and
/// break-pane: the attached client that ran it or, from a shell, the best
/// one ([`display_client`]).
fn claim_window(srv: &mut Server, ctx: &Ctx, wid: crate::model::WindowId) {
    if let Some(c) = display_client(srv, ctx)
        && let Some(w) = srv.windows.get_mut(&wid)
    {
        w.latest = Some(c);
    }
}

fn attached_client(srv: &Server, ctx: &Ctx) -> Option<ClientId> {
    ctx.client
        .filter(|c| srv.clients.get(c).is_some_and(|c| c.att.is_some()))
}

/// The client to show something on: the caller when it is attached, else
/// (a command client) the attached client typed into most recently, as tmux
/// picks its "best" client.
fn display_client(srv: &Server, ctx: &Ctx) -> Option<ClientId> {
    attached_client(srv, ctx).or_else(|| {
        srv.clients
            .values()
            .filter_map(|c| c.att.as_ref().map(|a| (c.id, a.last_input)))
            .max_by_key(|(_, t)| *t)
            .map(|(id, _)| id)
    })
}

/// The attached client `-c` names, else the best one ([`display_client`]).
fn target_client(srv: &Server, ctx: &Ctx, a: &Args) -> Result<ClientId, String> {
    match a.value('c') {
        Some(name) => session::target_client(srv, name),
        None => display_client(srv, ctx).ok_or_else(|| "no current client".to_owned()),
    }
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
    let (cols, rows) = from_client.unwrap_or(crate::server::DEFAULT_CLIENT_SIZE);
    let status = ctx.client.map_or_else(
        || srv.cfg.session.status.rows(),
        |c| srv.client_status_rows(c),
    );
    (cols.max(2), rows.saturating_sub(status).max(2))
}

/// `s` expanded for `pane` (else the command's) and the command's client,
/// from a shell the best one, as tmux's format_single_from_target takes
/// the target client.
pub(crate) fn expand_for(srv: &Server, ctx: &Ctx, pane: Option<PaneId>, s: &str) -> String {
    let client = display_client(srv, ctx);
    expand_with(srv, ctx, &Vars::for_pane(srv, pane.or(ctx.pane), client), s)
}

/// `s` expanded with `vars`: [`expand_for`] for variables already chosen.
fn expand_with(srv: &Server, ctx: &Ctx, vars: &Vars<'_>, s: &str) -> String {
    expand_by(srv, ctx, vars, s, expand)
}

/// [`expand_with`] by `expander` ([`expand`], or `expand_time` for
/// `display-message`).
fn expand_by(
    srv: &Server,
    ctx: &Ctx,
    vars: &Vars<'_>,
    s: &str,
    expander: fn(&str, &dyn tmxr_command::format::Context) -> String,
) -> String {
    match &ctx.mouse {
        // A mouse bind's commands also see what was under the mouse.
        Some(mouse) => expander(s, &|name: &str| {
            crate::vars::mouse_var(srv, ctx.client, mouse, name)
                .or_else(|| tmxr_command::format::Context::get(vars, name))
        }),
        None => expander(s, vars),
    }
}

/// tmux's template for what `-P` prints of a new window or pane.
const NEW_PANE_TEMPLATE: &str = "#{session_name}:#{window_index}.#{pane_index}";

/// `-P`: print the new session, window or pane by `-F`, else by `default`,
/// as tmux's `new-window`, `split-window`, `break-pane` and `new-session`.
/// Without a pane, the active one of the session's current window.
fn print_new(
    srv: &Server,
    ctx: &Ctx,
    a: &Args,
    out: &mut Outcome,
    session: SessionId,
    pane: Option<PaneId>,
    default: &str,
) {
    if !a.has('P') {
        return;
    }
    let pane = pane.or_else(|| {
        let window = srv.sessions.get(&session)?.current_window()?;
        srv.windows.get(&window).map(|w| w.active)
    });
    let vars = Vars {
        srv,
        session: Some(session),
        window: pane.and_then(|p| srv.panes.get(&p)).map(|p| p.window),
        pane,
        client: ctx.client,
        kind: crate::vars::Kind::of(Some(session), None, pane),
    };
    out.stdout.push_str(&expand_with(
        srv,
        ctx,
        &vars,
        a.value('F').unwrap_or(default),
    ));
    out.stdout.push('\n');
}

/// One item of a list command: its `default` line, or `-F`'s format
/// expanded with the item's variables; `None` when `-f`'s filter expands to
/// nothing or `0` for it, as tmux's filters.
/// The command client's standard input, which it sent ahead for a command
/// that reads it ([`tmxr_command::reads_stdin`]).
fn take_stdin(srv: &mut Server, ctx: &Ctx) -> Option<Vec<u8>> {
    srv.clients.get_mut(&ctx.client?)?.stdin.take()
}

/// Whether an expanded format counts as true, as tmux's format_true: it
/// is not empty and not `0`.
pub fn format_true(value: &str) -> bool {
    !value.is_empty() && value != "0"
}

fn list_item(
    a: &Args,
    vars: &dyn tmxr_command::format::Context,
    default: impl FnOnce() -> String,
) -> Option<String> {
    if let Some(filter) = a.value('f')
        && !format_true(&expand(filter, vars))
    {
        return None;
    }
    Some(a.value('F').map_or_else(default, |f| expand(f, vars)))
}

fn cwd_arg(srv: &Server, ctx: &Ctx, pane: Option<PaneId>, a: &Args) -> Option<PathBuf> {
    a.value('c')
        .map(|c| PathBuf::from(expand_for(srv, ctx, pane, c)))
}

/// The `-e NAME=VALUE` flags, in order.
fn env_flags(a: &Args) -> Result<Vec<(String, String)>, String> {
    a.values('e')
        .map(|e| {
            e.split_once('=')
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .ok_or_else(|| format!("-e {e}: not NAME=VALUE"))
        })
        .collect()
}

/// What `update-environment` (`patterns`) takes from a client's environment
/// `env`, as tmux's `environ_update` reads it: each entry is an `fnmatch`
/// pattern and every variable it matches is taken; an entry that matches
/// none names a variable the client does not have (`None`).
fn updated_env(
    patterns: &tmxr_config::Array,
    env: &[(String, String)],
) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for pattern in patterns.values() {
        let re = crate::util::glob_regex(pattern);
        let before = out.len();
        out.extend(
            env.iter()
                .filter(|(k, _)| re.is_match(k))
                .map(|(k, v)| (k.clone(), Some(v.clone()))),
        );
        if out.len() == before {
            out.push((pattern.to_owned(), None));
        }
    }
    out
}

/// A new session's environment from its creating client, as tmux's
/// `environ_update` makes it: the variables the global `update-environment`
/// takes, one
/// the client lacks marked removed. Empty with no client (a command from
/// the config), as tmux's. The client's own environment, from when it
/// connected, so a bind's command reads its client's.
fn session_env(srv: &Server, ctx: &Ctx) -> crate::model::Environment {
    let mut env = crate::model::Environment::default();
    let client = ctx.client.and_then(|c| srv.clients.get(&c));
    if let Some(hello) = client.and_then(|c| c.hello.as_ref()) {
        for (name, value) in updated_env(&srv.cfg.session.update_environment, &hello.env) {
            env.set(&name, value);
        }
    }
    env
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
    <bool as tmxr_config::OptionValue>::parse(value, &current)
}

fn run_one(srv: &mut Server, ctx: &Ctx, p: &Parsed, out: &mut Outcome) -> Res {
    // The command's name for its formats; a command it runs in turn has
    // its own while it runs.
    let outer = srv.running.replace(p.name());
    let ran = dispatch(srv, ctx, p, out);
    srv.running = outer;
    ran
}

fn dispatch(srv: &mut Server, ctx: &Ctx, p: &Parsed, out: &mut Outcome) -> Res {
    // A read-only client runs only what looks; a hook's commands are the
    // owner's, whoever's event fired it.
    if !srv.in_hook
        && !access::READ_ONLY.contains(&p.name())
        && ctx
            .client
            .and_then(|c| srv.clients.get(&c))
            .is_some_and(crate::server::Client::read_only)
    {
        return Err(format!("{}: the client is read-only", p.name()));
    }
    for family in [
        access::run,
        session::run,
        window::run,
        pane::run,
        binds::run,
        buffer::run,
        env::run,
        hook::run,
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
