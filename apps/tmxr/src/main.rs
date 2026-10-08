//! `tmxr` — a tmux-style terminal multiplexer.
//!
//! The same binary is the client (`tmxr [command]`) and, run as
//! `tmxr -S <socket> __server`, the server a client starts on demand.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use tmxr_client::SERVER_ARG;
use tmxr_proto::socket::Endpoint;

/// Command-line interface, modelled on tmux's:
/// `tmxr [-L label | -S socket] [-f config] [command [flags] [args]]`.
#[derive(Debug, Parser)]
#[command(
    name = "tmxr",
    version,
    about = "A tmux-style terminal multiplexer",
    after_help = "With no command, tmxr starts a new session (like tmux).\n\
                  Commands use tmux's names and flags; `tmxr list-commands` lists them.\n\
                  Separate several commands with a `\\;` argument."
)]
struct Cli {
    /// Server socket label; servers with different labels are independent.
    #[arg(short = 'L', value_name = "LABEL", conflicts_with = "socket")]
    label: Option<String>,

    /// Explicit server socket path (a pipe name on Windows).
    #[arg(short = 'S', value_name = "PATH")]
    socket: Option<PathBuf>,

    /// Config file to use instead of ~/.config/tmxr/config.toml.
    #[arg(short = 'f', value_name = "FILE")]
    config: Option<PathBuf>,

    /// Print shell completions to stdout and exit (for packaging).
    #[arg(long, value_enum, value_name = "SHELL", hide = true)]
    completions: Option<CompletionShell>,

    /// Print the man page (troff) to stdout and exit (for packaging).
    #[arg(long, hide = true)]
    man: bool,

    /// Command and its arguments, in tmux's command language.
    #[arg(
        value_name = "COMMAND",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    command: Vec<String>,
}

/// Shells `--completions` generates for: clap_complete's own five plus
/// nushell, which has its own generator crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    Powershell,
    Elvish,
    Nushell,
}

impl CompletionShell {
    fn generate(self, out: &mut dyn std::io::Write) {
        use clap::CommandFactory;
        use clap_complete::Shell;
        let cmd = &mut Cli::command();
        let shell = match self {
            Self::Bash => Shell::Bash,
            Self::Zsh => Shell::Zsh,
            Self::Fish => Shell::Fish,
            Self::Powershell => Shell::PowerShell,
            Self::Elvish => Shell::Elvish,
            Self::Nushell => {
                clap_complete::generate(clap_complete_nushell::Nushell, cmd, "tmxr", out);
                return;
            }
        };
        clap_complete::generate(shell, cmd, "tmxr", out);
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Some(shell) = cli.completions {
        shell.generate(&mut std::io::stdout());
        return ExitCode::SUCCESS;
    }
    if cli.man {
        use clap::CommandFactory;
        return match clap_mangen::Man::new(Cli::command()).render(&mut std::io::stdout()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("tmxr: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let endpoint = match Endpoint::resolve(cli.socket.as_deref(), cli.label.as_deref()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("tmxr: {e}");
            return ExitCode::FAILURE;
        }
    };
    if cli.command.first().map(String::as_str) == Some(SERVER_ARG) {
        init_server_log(&endpoint);
        return match tmxr_server::run(endpoint, cli.config) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                tracing::error!("server failed: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let opts = tmxr_client::Options {
        endpoint,
        args: cli.command,
        config: cli.config,
    };
    match tmxr_client::run(opts) {
        Ok(status) => ExitCode::from(u8::try_from(status).unwrap_or(1)),
        Err(e) => {
            eprintln!("tmxr: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Server logs go to `<state dir>/tmxr/logs/server-<socket name>.log`, never
/// to a terminal; `TMXR_LOG` sets the filter (default `info`).
fn init_server_log(endpoint: &Endpoint) {
    let Ok(dir) = hjkl_xdg::state_dir("tmxr").map(|d| d.join("logs")) else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let name = endpoint.slug();
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("server-{name}.log")))
    else {
        return;
    };
    let filter = tracing_subscriber::EnvFilter::try_from_env("TMXR_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file))
        .init();
}

#[cfg(test)]
mod cli_tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_completion_shell_names_the_binary() {
        use clap::ValueEnum;
        for shell in CompletionShell::value_variants() {
            let mut out = Vec::new();
            shell.generate(&mut out);
            let text = String::from_utf8(out).unwrap();
            assert!(text.contains("tmxr"), "{shell:?}: {text}");
        }
    }

    #[test]
    fn packaging_flags_are_not_taken_for_a_command() {
        let cli = Cli::parse_from(["tmxr", "--completions", "zsh"]);
        assert_eq!(cli.completions, Some(CompletionShell::Zsh));
        assert!(cli.command.is_empty());
        let cli = Cli::parse_from(["tmxr", "--man"]);
        assert!(cli.man && cli.command.is_empty());
        // After a command, they belong to it.
        let cli = Cli::parse_from(["tmxr", "new", "--man"]);
        assert!(!cli.man);
        assert_eq!(cli.command, ["new", "--man"]);
    }

    #[test]
    fn command_and_its_flags_are_passed_through() {
        let cli = Cli::parse_from(["tmxr", "-L", "work", "split-window", "-h", "-c", "/tmp"]);
        assert_eq!(cli.label.as_deref(), Some("work"));
        assert_eq!(cli.command, ["split-window", "-h", "-c", "/tmp"]);
    }

    #[test]
    fn label_and_socket_conflict() {
        let err = Cli::try_parse_from(["tmxr", "-L", "a", "-S", "/tmp/s"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}
