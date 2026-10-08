//! Starting a detached server.
//!
//! The server is this same executable run as `tmxr -S <socket> __server`,
//! detached from the client's terminal so closing the terminal does not take
//! the server (and every pane) with it:
//!
//! - Unix: `setsid()` in the child, which leaves the client's session and
//!   controlling terminal.
//! - Windows: `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`, so the server has
//!   no console and is not in the client's console process group.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use tmxr_proto::socket::Endpoint;

/// Hidden subcommand the server runs as.
pub const SERVER_ARG: &str = "__server";

pub fn server(endpoint: &Endpoint, config: Option<&Path>) -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg("-S").arg(endpoint.path());
    if let Some(c) = config {
        cmd.arg("-f").arg(c);
    }
    cmd.arg(SERVER_ARG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut cmd);
    // The server outlives this client; it is never waited for.
    cmd.spawn().map(drop)
}

#[cfg(unix)]
fn detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid is async-signal-safe and touches no memory, which is all
    // a pre_exec hook may do between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(windows)]
fn detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    // CreateProcess hands every inheritable handle to the child, and this
    // client's own stdio handles are inheritable when its parent passed them
    // that way (a pipe from a script capturing `tmxr ...` output). A server
    // holding that pipe keeps the caller waiting for end-of-file forever, so
    // stop them from being inherited; the server gets its own null stdio.
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: GetStdHandle takes no pointers; SetHandleInformation only
        // changes a flag on a handle this process owns (or fails harmlessly
        // on a null/invalid one).
        unsafe {
            let h = GetStdHandle(which);
            if !h.is_null() {
                SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
    // Process creation flags (winbase.h), spelled out rather than imported.
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}
