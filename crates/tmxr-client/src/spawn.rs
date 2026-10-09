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
//!
//! The server must not keep anything the client inherited open: a pipe a
//! script reads to end-of-file would stay open as long as the server runs,
//! and the script would wait that long. On Unix the server closes inherited
//! descriptors as it starts (`tmxr_server::close_inherited_fds`); on Windows it
//! is created inheriting no handles at all.

use std::ffi::OsString;
use std::io;
use std::path::Path;

use tmxr_proto::socket::Endpoint;

/// Hidden subcommand the server runs as.
pub const SERVER_ARG: &str = "__server";

pub fn server(endpoint: &Endpoint, config: Option<&Path>) -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut args: Vec<OsString> = vec!["-S".into(), endpoint.path().into()];
    if let Some(c) = config {
        args.extend(["-f".into(), c.into()]);
    }
    args.push(SERVER_ARG.into());
    // The server outlives this client; it is never waited for.
    spawn_detached(&exe, &args)
}

#[cfg(unix)]
fn spawn_detached(exe: &Path, args: &[OsString]) -> io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
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
    cmd.spawn().map(drop)
}

/// `CreateProcessW` with `bInheritHandles = FALSE`. `std::process::Command`
/// always lets the child inherit every inheritable handle, including ones
/// this client merely passed through from its own parent, and has no stable
/// way to name the few it should (`raw_attribute` is unstable).
#[cfg(windows)]
fn spawn_detached(exe: &Path, args: &[OsString]) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        CREATE_NEW_PROCESS_GROUP, CreateProcessW, DETACHED_PROCESS, PROCESS_INFORMATION,
        STARTF_USESTDHANDLES, STARTUPINFOW,
    };
    let program: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut line = Vec::new();
    // The program name is parsed without backslash escapes; a path holds no
    // quote, so quoting it whole is enough.
    line.push(u16::from(b'"'));
    line.extend(exe.as_os_str().encode_wide());
    line.push(u16::from(b'"'));
    for a in args {
        line.push(u16::from(b' '));
        quote_arg(&a.encode_wide().collect::<Vec<_>>(), &mut line);
    }
    line.push(0);
    // SAFETY: STARTUPINFOW is integers, pointers and handles, for which zero
    // is valid (none, null).
    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    // Null standard handles rather than the client's console's.
    si.dwFlags = STARTF_USESTDHANDLES;
    // SAFETY: as above.
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `program` and `line` are NUL-terminated and live through the
    // call (CreateProcessW may write into `line`); `si` and `pi` are live
    // structs of the sizes it expects; null environment and directory mean
    // this process's own.
    let ok = unsafe {
        CreateProcessW(
            program.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP,
            std::ptr::null(),
            std::ptr::null(),
            &si,
            &mut pi,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both handles came from CreateProcessW and are closed once.
    unsafe {
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
    }
    Ok(())
}

/// Append `arg` to a Windows command line so the C runtime's parser (and
/// `CommandLineToArgvW`) reads it back unchanged: Microsoft's documented
/// rules, where backslashes are literal unless they precede a quote, `2n`
/// backslashes before a quote are `n` and the quote ends the argument, and
/// `2n + 1` are `n` and a literal quote.
#[cfg(windows)]
fn quote_arg(arg: &[u16], out: &mut Vec<u16>) {
    let (quote, backslash) = (u16::from(b'"'), u16::from(b'\\'));
    let plain = !arg.is_empty()
        && !arg
            .iter()
            .any(|&c| c == quote || c == u16::from(b' ') || c == u16::from(b'\t'));
    if plain {
        out.extend_from_slice(arg);
        return;
    }
    out.push(quote);
    let mut backslashes = 0;
    for &c in arg {
        if c == backslash {
            backslashes += 1;
        } else {
            if c == quote {
                // Double the run before it, and escape the quote itself.
                out.extend(std::iter::repeat_n(backslash, backslashes + 1));
            }
            backslashes = 0;
        }
        out.push(c);
    }
    // A run just before the closing quote must be doubled too.
    out.extend(std::iter::repeat_n(backslash, backslashes));
    out.push(quote);
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// What Windows' own parser makes of a command line.
    fn parse(line: &[u16]) -> Vec<String> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::UI::Shell::CommandLineToArgvW;
        let mut z = line.to_vec();
        z.push(0);
        let mut n = 0;
        // SAFETY: `z` is NUL-terminated; the result is freed below.
        let argv = unsafe { CommandLineToArgvW(z.as_ptr(), &mut n) };
        assert!(!argv.is_null());
        let out = (0..usize::try_from(n).unwrap())
            .map(|i| {
                // SAFETY: CommandLineToArgvW returned `n` NUL-terminated
                // strings, alive until LocalFree.
                unsafe {
                    let p = *argv.add(i);
                    let len = (0..).take_while(|&j| *p.add(j) != 0).count();
                    String::from_utf16(std::slice::from_raw_parts(p, len)).unwrap()
                }
            })
            .collect();
        // SAFETY: freed once, as CommandLineToArgvW documents.
        unsafe { LocalFree(argv.cast()) };
        out
    }

    #[test]
    fn quoted_arguments_read_back_unchanged() {
        let args = [
            "plain",
            "",
            "two words",
            r"C:\Program Files\tmxr\",
            r"\\.\pipe\tmxr-me-default",
            r#"say "hi""#,
            r#"back\"slash"#,
            r"trailing\\",
            "tab\there",
        ];
        let mut line: Vec<u16> = "prog".encode_utf16().collect();
        for a in args {
            line.push(u16::from(b' '));
            quote_arg(&a.encode_utf16().collect::<Vec<_>>(), &mut line);
        }
        let parsed = parse(&line);
        assert_eq!(parsed[1..], args.map(str::to_owned));
    }
}
