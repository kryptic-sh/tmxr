//! Programs on pseudo-terminals.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// What to run in a new pane.
#[derive(Debug, Clone, Default)]
pub struct SpawnSpec {
    /// Program and arguments; empty runs [`default_shell`].
    pub argv: Vec<String>,
    /// Working directory; `None` inherits the server's.
    pub cwd: Option<PathBuf>,
    /// Variables set on top of the server's environment.
    pub env: Vec<(String, String)>,
    /// Variables taken out of it (tmux's `set-environment -r`).
    pub env_remove: Vec<String>,
    pub rows: u16,
    pub cols: u16,
}

/// Something that happened to a pane's program, reported from the pane's
/// reader / waiter threads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyEvent {
    Output(Vec<u8>),
    /// The output stream ended. On Unix this follows the program's exit once
    /// every holder of the terminal has closed it; ConPTY only ends the stream
    /// when the pseudo console itself is closed, so do not wait for it there.
    Eof,
    /// The program exited: its exit code, or the signal that ended it,
    /// when either is known.
    Exited {
        code: Option<u32>,
        signal: Option<i32>,
    },
}

/// A running program on a pseudo-terminal. Dropping it closes the terminal
/// (which ends the program's output stream) but does not kill the program;
/// call [`Pty::kill`] for that.
pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    /// Bytes for the program, written by the pane's writer thread: a
    /// program that does not read its input blocks that thread, never the
    /// caller.
    input: Sender<Vec<u8>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pid: Option<u32>,
}

impl Pty {
    /// Start `spec` and deliver its output and exit to `sink`, from
    /// threads owned by this pane (a reader, a writer and a waiter).
    pub fn spawn(spec: &SpawnSpec, sink: Arc<dyn Fn(PtyEvent) + Send + Sync>) -> io::Result<Self> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: spec.rows.max(1),
                cols: spec.cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(io::Error::other)?;

        let argv: Vec<OsString> = if spec.argv.is_empty() {
            vec![default_shell()]
        } else {
            spec.argv.iter().map(OsString::from).collect()
        };
        let mut cmd = CommandBuilder::from_argv(argv);
        // A pane inherits the server's environment, as in tmux. On Windows
        // portable-pty lays the registry's variables over it (a fresh login's
        // PATH, losing what the shell that started tmxr added); setting each
        // of the server's own values again puts them back. On Unix the values
        // are the ones already there.
        for (k, v) in std::env::vars_os() {
            cmd.env(k, v);
        }
        for k in &spec.env_remove {
            cmd.env_remove(k);
        }
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }

        let mut child = pair.slave.spawn_command(cmd).map_err(io::Error::other)?;
        // The child holds its own handle to the terminal; keeping ours open
        // would stop the reader from ever seeing end-of-file on Unix.
        drop(pair.slave);

        let pid = child.process_id();
        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
        let mut writer = pair.master.take_writer().map_err(io::Error::other)?;

        let out = Arc::clone(&sink);
        std::thread::Builder::new()
            .name("tmxr-pty-read".into())
            .spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => out(PtyEvent::Output(buf[..n].to_vec())),
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
                out(PtyEvent::Eof);
            })?;
        // The writer stops at its first error, and the next write says so.
        let (input, queued) = channel::<Vec<u8>>();
        std::thread::Builder::new()
            .name("tmxr-pty-write".into())
            .spawn(move || {
                for bytes in queued {
                    if writer
                        .write_all(&bytes)
                        .and_then(|()| writer.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        std::thread::Builder::new()
            .name("tmxr-pty-wait".into())
            .spawn(move || {
                let (code, signal) = wait_exit(child.as_mut(), pid);
                sink(PtyEvent::Exited { code, signal });
            })?;

        Ok(Self {
            master: pair.master,
            input,
            killer,
            pid,
        })
    }

    /// Queue bytes for the program, in order. An error means the writer
    /// has stopped: a write before this one failed.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.input
            .send(bytes.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the pane's input is closed"))
    }

    pub fn resize(&self, rows: u16, cols: u16) -> io::Result<()> {
        self.master
            .resize(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(io::Error::other)
    }

    /// Process id of the program started in the pane.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// The terminal's foreground process group (Unix only).
    #[cfg(unix)]
    pub fn foreground_pgrp(&self) -> Option<u32> {
        self.master
            .process_group_leader()
            .and_then(|p| u32::try_from(p).ok())
    }

    /// Kill the program started in the pane.
    pub fn kill(&mut self) -> io::Result<()> {
        self.killer.kill()
    }
}

/// The shell new panes run when no command is given: `$SHELL` (falling back to
/// `/bin/sh`) on Unix; on Windows the first of `pwsh.exe`, `powershell.exe` on
/// `PATH`, then `%COMSPEC%`, then `cmd.exe`.
pub fn default_shell() -> OsString {
    #[cfg(unix)]
    {
        std::env::var_os("SHELL")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| OsString::from("/bin/sh"))
    }
    #[cfg(windows)]
    {
        let path = std::env::var_os("PATH").unwrap_or_default();
        for exe in ["pwsh.exe", "powershell.exe"] {
            if let Some(found) = std::env::split_paths(&path)
                .map(|dir| dir.join(exe))
                .find(|p| p.is_file())
            {
                return found.into_os_string();
            }
        }
        std::env::var_os("COMSPEC")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| OsString::from("cmd.exe"))
    }
}

/// Wait for `child` to exit: its exit code, or the signal that ended it.
/// On Unix the child is reaped here with `waitpid`, read as tmux reads
/// `WIFEXITED` / `WIFSIGNALED`, since portable-pty's status keeps only a
/// signal's description; without a pid, its status is all there is.
#[cfg(unix)]
fn wait_exit(child: &mut dyn portable_pty::Child, pid: Option<u32>) -> (Option<u32>, Option<i32>) {
    let Some(pid) = pid.and_then(|p| libc::pid_t::try_from(p).ok()) else {
        return (child.wait().ok().map(|s| s.exit_code()), None);
    };
    let mut status: libc::c_int = 0;
    loop {
        // SAFETY: `status` is a live c_int, the only memory waitpid writes.
        let reaped = unsafe { libc::waitpid(pid, &mut status, 0) };
        if reaped == pid {
            break;
        }
        if reaped == -1 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return (None, None);
    }
    if libc::WIFEXITED(status) {
        (u32::try_from(libc::WEXITSTATUS(status)).ok(), None)
    } else if libc::WIFSIGNALED(status) {
        (None, Some(libc::WTERMSIG(status)))
    } else {
        (None, None)
    }
}

/// Wait for `child` to exit: its exit code. Windows has no signals.
#[cfg(not(unix))]
fn wait_exit(child: &mut dyn portable_pty::Child, _pid: Option<u32>) -> (Option<u32>, Option<i32>) {
    (child.wait().ok().map(|s| s.exit_code()), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// A command that prints `marker` and exits with status 3.
    fn print_and_exit(marker: &str) -> Vec<String> {
        #[cfg(unix)]
        {
            vec![
                "/bin/sh".into(),
                "-c".into(),
                format!("echo {marker}; exit 3"),
            ]
        }
        #[cfg(windows)]
        {
            vec![
                "cmd.exe".into(),
                "/d".into(),
                "/c".into(),
                format!("echo {marker}& exit /b 3"),
            ]
        }
    }

    #[test]
    fn output_and_exit_code_are_reported() {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: Arc<dyn Fn(PtyEvent) + Send + Sync> =
            Arc::new(move |e| drop(tx.lock().unwrap().send(e)));
        let spec = SpawnSpec {
            argv: print_and_exit("tmxr-marker"),
            rows: 24,
            cols: 80,
            ..SpawnSpec::default()
        };
        let mut pty = Pty::spawn(&spec, sink).unwrap();
        assert!(pty.pid().is_some());
        // ConPTY asks for the cursor position at startup and waits for the
        // answer, so output goes through an emulator that replies, exactly as
        // the server does.
        let mut emu = crate::Emulator::new(24, 80, 0);

        let mut output = Vec::new();
        let mut exit = None;
        let deadline = Instant::now() + Duration::from_secs(20);
        while exit.is_none() || !String::from_utf8_lossy(&output).contains("tmxr-marker") {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(PtyEvent::Output(b)) => {
                    let replies = emu.process(&b);
                    if !replies.is_empty() {
                        pty.write(&replies).unwrap();
                    }
                    output.extend(b);
                }
                Ok(PtyEvent::Exited { code, .. }) => exit = Some(code),
                Ok(PtyEvent::Eof) => {}
                Err(_) => panic!(
                    "timed out; exit={exit:?} output={:?}",
                    String::from_utf8_lossy(&output)
                ),
            }
        }
        assert_eq!(exit, Some(Some(3)));
    }
}
