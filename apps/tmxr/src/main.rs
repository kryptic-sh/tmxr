//! `tmxr` — a tmux-style terminal multiplexer.
//!
//! Scaffold: the CLI surface is parsed, but no server or client exists yet, so
//! every command reports that and exits non-zero. See `docs/plan/` for the
//! design and `docs/plan/15-milestones.md` for the build order.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

/// Command-line interface, modelled on tmux's:
/// `tmxr [-L label | -S socket] [-f config] [command [flags] [args]]`.
#[derive(Debug, Parser)]
#[command(
    name = "tmxr",
    version,
    about = "A tmux-style terminal multiplexer",
    after_help = "With no command, tmxr starts a new session (like tmux)."
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

    /// Command and its arguments, in tmux's command language.
    #[arg(
        value_name = "COMMAND",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    command: Vec<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let command = if cli.command.is_empty() {
        "new-session".to_owned()
    } else {
        cli.command.join(" ")
    };
    eprintln!("tmxr: {command}: not implemented yet (the server lands in milestone M1)");
    ExitCode::FAILURE
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
