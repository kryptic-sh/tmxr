//! The tmxr client.
//!
//! Sends one command to the server — starting the server first when the
//! command needs one — and either prints the reply (command clients) or, when
//! the server attaches it, turns the terminal over to the server: input events
//! go up, output bytes come down, until the server detaches it.
//! See `docs/plan/02-architecture.md`.

mod spawn;
mod terminal;

use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use interprocess::local_socket::Stream;
use tmxr_proto::socket::{ENV_TMXR, Endpoint};
use tmxr_proto::{
    ClientMsg, Hello, PROTOCOL_VERSION, ServerMsg, TerminalInfo, read_msg, write_msg,
};

pub use spawn::SERVER_ARG;
pub use terminal::PROBE_WAIT;

/// What to run.
pub struct Options {
    pub endpoint: Endpoint,
    /// Command line after the global flags; empty means `new-session`.
    pub args: Vec<String>,
    /// `-f` config path, passed to a server this client starts.
    pub config: Option<PathBuf>,
}

/// Commands that start a server when none is running (tmux's
/// `CMD_STARTSERVER`).
fn starts_server(args: &[String]) -> bool {
    let Some(name) = args.first() else {
        return true;
    };
    matches!(
        tmxr_command::lookup(name).map(|c| c.name),
        Ok("new-session" | "resurrect-restore" | "start-server")
    )
}

/// Whether the command would attach this terminal.
fn would_attach(args: &[String]) -> bool {
    if args.is_empty() {
        return true;
    }
    match tmxr_command::parse(args) {
        // `attach -d` still attaches; its -d detaches the other clients.
        Ok(p) => match p.name() {
            "new-session" => !p.args.has('d'),
            "attach-session" => true,
            _ => false,
        },
        Err(_) => false,
    }
}

/// Run the client; returns the process exit status.
pub fn run(opts: Options) -> io::Result<i32> {
    // Only inside one of this server's own panes, as tmux's
    // server_client_check_nested: a client may attach to another server.
    let nested = std::env::var(ENV_TMXR)
        .ok()
        .and_then(|v| tmxr_proto::socket::parse_tmxr_env(&v))
        .is_some_and(|(path, _, _)| path == opts.endpoint.path());
    if nested && would_attach(&opts.args) {
        eprintln!("sessions should be nested with care, unset ${ENV_TMXR} to force");
        return Ok(1);
    }
    let stream = match opts.endpoint.connect() {
        Ok(s) => s,
        Err(e) if is_no_server(&e) && starts_server(&opts.args) => {
            spawn::server(&opts.endpoint, opts.config.as_deref())?;
            wait_for_server(&opts.endpoint)?
        }
        Err(e) if is_no_server(&e) => {
            eprintln!("no server running on {}", opts.endpoint.path().display());
            return Ok(1);
        }
        Err(e) => return Err(e),
    };
    session(stream, opts.args)
}

fn is_no_server(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

fn wait_for_server(endpoint: &Endpoint) -> io::Result<Stream> {
    let deadline = Instant::now() + tmxr_proto::SPAWN_TIMEOUT;
    let mut delay = Duration::from_millis(10);
    loop {
        match endpoint.connect() {
            Ok(s) => return Ok(s),
            Err(e) if is_no_server(&e) && Instant::now() < deadline => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(200));
            }
            Err(e) => {
                return Err(io::Error::new(
                    e.kind(),
                    format!("server did not start on {}: {e}", endpoint.path().display()),
                ));
            }
        }
    }
}

/// Whether this client runs in a terminal it could attach.
fn in_terminal() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn hello() -> Hello {
    let terminal = in_terminal()
        .then(|| terminal::size().ok())
        .flatten()
        .map(|(cols, rows)| TerminalInfo {
            cols,
            rows,
            term: std::env::var("TERM").unwrap_or_default(),
            job_control: cfg!(unix),
            tty: terminal::tty_name(),
            // Read through ttyr's reader where it reads the terminal's
            // bytes; not by a Windows console handing over key records,
            // which is asked nothing.
            colour_answers: terminal::reads_vt(),
        });
    Hello {
        protocol: PROTOCOL_VERSION,
        version: env!("CARGO_PKG_VERSION").into(),
        cwd: std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default(),
        pid: std::process::id(),
        env: tmxr_term::env::vars(),
        terminal,
    }
}

fn session(stream: Stream, args: Vec<String>) -> io::Result<i32> {
    // Read here and written by this thread and the input thread, as `&Stream`
    // reads and writes.
    let conn = Arc::new(stream);
    let (mut recv, mut send) = (&*conn, &*conn);
    let to_io = |e: tmxr_proto::ProtoError| io::Error::other(e.to_string());
    let hello = hello();
    let size = hello.terminal.as_ref().map(|t| (t.cols, t.rows));
    write_msg(&mut send, &ClientMsg::Hello(hello)).map_err(to_io)?;
    // `load-buffer -` reads this process's standard input: all of it goes
    // ahead of the command, in pieces a frame holds (one, empty, for none).
    if tmxr_command::reads_stdin(&args) {
        let mut data = Vec::new();
        io::stdin().read_to_end(&mut data)?;
        let mut pieces = data.chunks(tmxr_proto::STDIN_PIECE).peekable();
        if pieces.peek().is_none() {
            write_msg(&mut send, &ClientMsg::Stdin(Vec::new())).map_err(to_io)?;
        }
        for piece in pieces {
            write_msg(&mut send, &ClientMsg::Stdin(piece.to_vec())).map_err(to_io)?;
        }
    }
    write_msg(&mut send, &ClientMsg::Command(args)).map_err(to_io)?;

    let mut send = Some(Arc::clone(&conn));
    // Shared with the input thread once attached, to report a resume.
    let mut writer: Option<terminal::Sender> = None;
    let mut input: Option<terminal::Input> = None;
    let mut term: Option<terminal::Guard> = None;
    // The kitty keyboard flags the server last asked for, which the
    // terminal is asked for again after a suspend or a lock; none until it
    // says, which it does only once told the terminal speaks the protocol.
    let mut keyboard_flags: Option<u8> = None;
    let mut stdout = io::stdout().lock();
    loop {
        let Ok(Some(msg)) = read_msg::<_, ServerMsg>(&mut recv) else {
            drop(term.take());
            if send.is_none() {
                eprintln!("[lost server]");
            } else {
                eprintln!("server closed the connection");
            }
            return Ok(1);
        };
        match msg {
            ServerMsg::Hello {
                protocol, version, ..
            } => {
                if protocol != PROTOCOL_VERSION {
                    eprintln!(
                        "server {version} speaks protocol {protocol}, this client {} speaks {PROTOCOL_VERSION}",
                        env!("CARGO_PKG_VERSION")
                    );
                }
            }
            ServerMsg::Attached => {
                if let Some(s) = send.take() {
                    let mut keys = terminal::Keys::open()?;
                    let (guard, mut first) = terminal::Guard::enter(&mut keys)?;
                    // After raw mode, which turns the Windows console's
                    // resize records on; behind what was read meanwhile,
                    // as it is looked at after them.
                    first.extend(keys.resized_since(size));
                    let (fg, bg) = guard.colours();
                    let keyboard = guard.keyboard();
                    term = Some(guard);
                    write_msg(&mut &*s, &ClientMsg::TerminalColours { fg, bg }).map_err(to_io)?;
                    // The server tells a kitty terminal its flags once it
                    // knows; until then the terminal is asked for none.
                    write_msg(&mut &*s, &ClientMsg::Keyboard(keyboard)).map_err(to_io)?;
                    let shared = Arc::new(Mutex::new(s));
                    input = Some(terminal::spawn_input(Arc::clone(&shared), keys, first)?);
                    writer = Some(shared);
                }
            }
            ServerMsg::Output(bytes) => match &term {
                Some(t) => t.write_frame(&mut stdout, &bytes)?,
                None => {
                    stdout.write_all(&bytes)?;
                    stdout.flush()?;
                }
            },
            ServerMsg::CommandResult {
                status,
                stdout: out,
                stderr,
            } => {
                drop(term.take());
                if !out.is_empty() {
                    print!("{out}");
                }
                if !stderr.is_empty() {
                    eprintln!("{}", stderr.trim_end());
                }
                return Ok(status);
            }
            ServerMsg::Detached {
                exec: Some(argv), ..
            } => {
                drop(term.take());
                return terminal::exec(&argv);
            }
            ServerMsg::Detached { reason, hangup, .. } => {
                drop(term.take());
                println!("[{reason}]");
                if hangup {
                    terminal::hang_up_parent()?;
                }
                return Ok(0);
            }
            ServerMsg::Suspend => {
                // Hand the terminal back to the shell while stopped.
                hand_over(
                    &mut term,
                    input.as_ref(),
                    writer.as_ref(),
                    &mut keyboard_flags,
                    || terminal::suspend().map(|()| ClientMsg::Resumed),
                )?;
            }
            ServerMsg::Lock { command } => {
                // The lock command reads the keys (a password), so the
                // input thread must not.
                hand_over(
                    &mut term,
                    input.as_ref(),
                    writer.as_ref(),
                    &mut keyboard_flags,
                    || {
                        Ok(match terminal::run_lock(&command) {
                            Ok(()) => ClientMsg::Resumed,
                            Err(why) => ClientMsg::LockFailed(why),
                        })
                    },
                )?;
            }
            ServerMsg::Mouse(on) => {
                if let Some(t) = &term {
                    t.set_mouse(on)?;
                }
            }
            ServerMsg::FocusEvents(on) => {
                if let Some(t) = &term {
                    t.set_focus_events(on)?;
                }
            }
            ServerMsg::KeyboardFlags(flags) => {
                keyboard_flags = Some(flags);
                // The reader is told whether the protocol is on before its
                // next read, without pausing the input thread.
                if let Some(t) = &mut term
                    && let Some(on) = t.set_keyboard_flags(flags)?
                    && let Some(input) = &input
                {
                    input.set_kitty_keyboard(on)?;
                }
            }
            ServerMsg::ExtendedKeys(on) => {
                // The server asks only a client that said it can
                // (`KeyboardReport::modify_other_keys`); a Windows client
                // reading console key records says not, and refuses the
                // mode if asked anyway, which is reported and leaves it
                // attached. A failed write is an error.
                if let Some(t) = &term
                    && let Err(e) = t.set_extended_keys(on)
                {
                    if e.kind() != io::ErrorKind::Unsupported {
                        return Err(e);
                    }
                    eprintln!("[{e}]");
                }
            }
        }
    }
}

/// Give the terminal to `run` (a suspend, the lock command) and take it
/// back, asking it again what it speaks and for the kitty keyboard `flags`
/// the server last sent ([`terminal::Guard::hand_over`]); then tell the
/// server its colours and what its keyboard speaks where they changed
/// (forgetting `flags` where the keyboard did: the server says them, and
/// modifyOtherKeys, again from what it decides), that it is back with the
/// message `run` returns, and the input read as the terminal answered,
/// after a resize meanwhile that the reader did not see. The input
/// thread is paused throughout, so what is typed meanwhile goes to
/// whatever `run` hands the terminal to, and nothing it reads after
/// overtakes that input. Does nothing with no input thread (not attached).
fn hand_over(
    term: &mut Option<terminal::Guard>,
    input: Option<&terminal::Input>,
    writer: Option<&terminal::Sender>,
    flags: &mut Option<u8>,
    run: impl FnOnce() -> io::Result<ClientMsg>,
) -> io::Result<()> {
    let (Some(guard), Some(input), Some(writer)) = (term.as_mut(), input, writer) else {
        return Ok(());
    };
    let mut paused = input.pause()?;
    let (colours, keyboard) = (guard.colours(), guard.keyboard());
    let size = paused.size().ok();
    let (back, mut early) = guard.hand_over(&mut paused, *flags, run)?;
    // A Windows console makes no resize record with raw mode off, as it is
    // while `run` has it; on Unix the reader saw it, and the server takes
    // the size twice.
    early.extend(paused.resized_since(size));
    // The server keeps the terminal's colours and keyboard from attach:
    // tell it of new ones, as attaching does.
    let (fg, bg) = guard.colours();
    let recoloured = colours != (fg, bg);
    let rekeyed = Some(guard.keyboard()).filter(|now| *now != keyboard);
    if rekeyed.is_some() {
        *flags = None;
    }
    {
        let mut w = &**writer
            .lock()
            .map_err(|_| io::Error::other("input thread panicked"))?;
        let to_io = |e: tmxr_proto::ProtoError| io::Error::other(e.to_string());
        if recoloured {
            write_msg(&mut w, &ClientMsg::TerminalColours { fg, bg }).map_err(to_io)?;
        }
        if let Some(keyboard) = rekeyed {
            write_msg(&mut w, &ClientMsg::Keyboard(keyboard)).map_err(to_io)?;
        }
        write_msg(&mut w, &back?).map_err(to_io)?;
    }
    if early.is_empty() {
        return Ok(());
    }
    terminal::forward(writer, early)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn which_commands_start_or_attach() {
        assert!(starts_server(&[]));
        assert!(starts_server(&v(&["new", "-s", "x"])));
        assert!(!starts_server(&v(&["ls"])));
        assert!(!starts_server(&v(&["attach"])));
        assert!(would_attach(&[]));
        assert!(would_attach(&v(&["attach", "-t", "x"])));
        assert!(would_attach(&v(&["attach", "-d"])));
        assert!(!would_attach(&v(&["new", "-d"])));
        assert!(!would_attach(&v(&["new", "-ds", "bg"])));
        assert!(!would_attach(&v(&["split-window", "-h"])));
    }
}
