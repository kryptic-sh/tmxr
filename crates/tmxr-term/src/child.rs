//! A [`Command`] for a child process that opens no console window.
//!
//! On Windows a console program started by a process with no console of its
//! own is given a new console, and that console has a window. The tmxr
//! server is such a process (it runs detached), so every `run-shell`,
//! `if-shell`, `copy-pipe` and `pipe-pane` command flashed a window up; a
//! test harness or an agent's shell can be one too. A process that has a
//! console, with a window or without, passes it on and nothing appears.
//!
//! [`command`] is [`Command::new`] that marks the child `CREATE_NO_WINDOW`
//! when this process has no console, and leaves it alone when it has one, so
//! a client run in a terminal still gives its children that terminal.
//! Piped and captured output, and exit codes, are unaffected either way.
//! Other platforms never open a window for a child, so there it is exactly
//! [`Command::new`].
//!
//! The workspace's `clippy.toml` disallows [`Command::new`], so a new spawn
//! cannot forget this. Checked on Windows by this module's test and the
//! `run-shell` e2e test; Linux and macOS take the plain path.

use std::ffi::OsStr;
use std::process::Command;

/// [`Command::new`], opening no console window for the child on Windows.
#[must_use]
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place the plain constructor is wrapped"
    )]
    let mut command = Command::new(program);
    windowless(&mut command);
    command
}

/// Marks `command`'s child `CREATE_NO_WINDOW` if this process has no console.
#[cfg(windows)]
fn windowless(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
    if !has_console() {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

#[cfg(not(windows))]
const fn windowless(_: &mut Command) {}

/// Whether this process is attached to a console, with a window or without:
/// a child of a windowless console shares it and shows nothing, and must keep
/// it, or output written to the inherited console handles is lost.
#[cfg(windows)]
pub fn has_console() -> bool {
    use windows_sys::Win32::System::Console::GetConsoleProcessList;
    let mut list = [0_u32; 1];
    // SAFETY: a one-element buffer and its true length; the call writes at
    // most that many ids, and returns 0 when there is no console.
    unsafe { GetConsoleProcessList(list.as_mut_ptr(), 1) != 0 }
}

#[cfg(test)]
mod tests {
    use super::command;

    /// The child still runs, and its exit code and captured output come back.
    #[test]
    fn a_child_runs_and_its_output_is_captured() {
        let output = command(env!("CARGO"))
            .arg("--version")
            .output()
            .expect("cargo runs");
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("cargo "));
    }
}
