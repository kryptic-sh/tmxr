//! The client's process: the lock command, job control, its tty, exec.

use std::io;

/// Run the lock command in the terminal and wait for it (`lock-client`), as
/// tmux runs `lock-command` with `/bin/sh -c`; an error says why it did not
/// unlock cleanly.
pub fn run_lock(command: &str) -> Result<(), String> {
    #[cfg(unix)]
    let status = tmxr_term::child::command("/bin/sh")
        .arg("-c")
        .arg(command)
        .status();
    #[cfg(windows)]
    let status = {
        use std::os::windows::process::CommandExt as _;
        // Raw, as cmd parses its own command line rather than argv.
        tmxr_term::child::command("cmd")
            .arg("/C")
            .raw_arg(command)
            .status()
    };
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("lock-command {command:?} exited with {s}")),
        Err(e) => Err(format!("lock-command {command:?}: {e}")),
    }
}

/// Stop this process until the shell continues it (`suspend-client`), as
/// `C-z` stops a program; returns once it runs again.
#[cfg(unix)]
pub fn suspend() -> io::Result<()> {
    // SAFETY: `raise` only sends a signal to this process; SIGTSTP's default
    // action stops it until SIGCONT.
    if unsafe { libc::raise(libc::SIGTSTP) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The terminal on standard input, as tmux's client takes
/// `ttyname(STDIN_FILENO)` for `#{client_tty}`; empty when it has none.
#[cfg(unix)]
pub fn tty_name() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: `ttyname_r` writes at most `buf.len()` bytes, its NUL
    // included, into `buf`, which outlives the call.
    let failed = unsafe { libc::ttyname_r(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
    if failed != 0 {
        return String::new();
    }
    std::ffi::CStr::from_bytes_until_nul(&buf)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A Windows console has no device name, so `#{client_tty}` is empty, as
/// tmux's for a client without a tty.
#[cfg(windows)]
pub fn tty_name() -> String {
    String::new()
}

/// Send this process's parent (the shell it ran from) SIGHUP, which usually
/// closes the terminal too (`detach-client -P`).
#[cfg(unix)]
pub fn hang_up_parent() -> io::Result<()> {
    // SAFETY: `kill` only sends a signal; `getppid` cannot fail.
    if unsafe { libc::kill(libc::getppid(), libc::SIGHUP) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Run `argv` in this client's place (`detach-client -E`), its shell as
/// `SHELL`, as tmux's client_exec: the process becomes it, so this returns
/// only when that fails.
#[cfg(unix)]
pub fn exec(argv: &[String]) -> io::Result<i32> {
    use std::os::unix::process::CommandExt as _;
    let (shell, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::other("empty command"))?;
    Err(tmxr_term::child::command(shell)
        .args(args)
        .env("SHELL", shell)
        .exec())
}

/// Run `argv` for this client (`detach-client -E`), its shell as `SHELL`,
/// and exit with its status: Windows cannot replace a running process, as
/// tmux's client_exec does.
#[cfg(windows)]
pub fn exec(argv: &[String]) -> io::Result<i32> {
    let (shell, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::other("empty command"))?;
    let status = tmxr_term::child::command(shell)
        .args(args)
        .env("SHELL", shell)
        .status()?;
    Ok(status.code().unwrap_or(1))
}

/// Windows has no SIGHUP; the server does not ask a client without job
/// control, so reaching this is a protocol error.
#[cfg(windows)]
pub fn hang_up_parent() -> io::Result<()> {
    Err(io::Error::other(
        "detach-client -P is not supported on Windows",
    ))
}

/// Windows has no job control; the server does not ask a client that said
/// so in its hello, so reaching this is a protocol error.
#[cfg(windows)]
pub fn suspend() -> io::Result<()> {
    Err(io::Error::other(
        "suspend-client is not supported on Windows",
    ))
}
