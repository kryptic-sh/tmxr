//! The tmxr client.
//!
//! Sends one command to the server — starting the server first when the
//! command needs one — and either prints the reply (command clients) or, when
//! the server attaches it, turns the terminal over to the server: input events
//! go up, output bytes come down, until the server detaches it.
//! See `docs/plan/02-architecture.md`.

mod spawn;
mod terminal;

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use interprocess::local_socket::Stream;
use interprocess::local_socket::traits::Stream as _;
use tmxr_proto::socket::{ENV_TMXR, Endpoint};
use tmxr_proto::{
    ClientMsg, Hello, PROTOCOL_VERSION, ServerMsg, TerminalInfo, read_msg, write_msg,
};

pub use spawn::SERVER_ARG;

/// How long to wait for a freshly spawned server to accept connections.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(10);

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
        Ok("new-session" | "resurrect-restore")
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
    if std::env::var_os(ENV_TMXR).is_some_and(|v| !v.is_empty()) && would_attach(&opts.args) {
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
    let deadline = Instant::now() + SPAWN_TIMEOUT;
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

fn hello() -> Hello {
    let terminal = (io::stdin().is_terminal() && io::stdout().is_terminal())
        .then(|| crossterm::terminal::size().ok())
        .flatten()
        .map(|(cols, rows)| TerminalInfo {
            cols,
            rows,
            term: std::env::var("TERM").unwrap_or_default(),
        });
    Hello {
        protocol: PROTOCOL_VERSION,
        version: env!("CARGO_PKG_VERSION").into(),
        cwd: std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default(),
        env: std::env::vars().collect(),
        terminal,
    }
}

fn session(stream: Stream, args: Vec<String>) -> io::Result<i32> {
    let (mut recv, mut send) = stream.split();
    let to_io = |e: tmxr_proto::ProtoError| io::Error::other(e.to_string());
    write_msg(&mut send, &ClientMsg::Hello(hello())).map_err(to_io)?;
    write_msg(&mut send, &ClientMsg::Command(args)).map_err(to_io)?;

    let mut send = Some(send);
    let mut term: Option<terminal::Guard> = None;
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
                term = Some(terminal::Guard::enter()?);
                if let Some(s) = send.take() {
                    terminal::spawn_input(s);
                }
            }
            ServerMsg::Output(bytes) => {
                stdout.write_all(&bytes)?;
                stdout.flush()?;
            }
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
            ServerMsg::Detached { reason } => {
                drop(term.take());
                println!("[{reason}]");
                return Ok(0);
            }
        }
    }
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
