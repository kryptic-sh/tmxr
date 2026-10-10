//! `pipe-pane`: a pane's output copied to a shell command's standard input
//! (`-O`, the default), and with `-I` the command's output typed into the
//! pane, as tmux does it (`pipe-pane 'cat >> ~/pane.log'`).

use std::io::{Read as _, Write as _};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Sender, SyncSender, sync_channel};

use crate::model::PaneId;
use crate::server::Event;

/// Output chunks waiting for a slow command before more are dropped: the
/// server never waits on a pipe.
const QUEUE_CHUNKS: usize = 1024;

/// What a pipe carries: the pane's output to the command (`-O`), the
/// command's output into the pane (`-I`), or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Direction {
    pub output: bool,
    pub input: bool,
}

/// A pane's open pipe: the command, and the thread feeding its input.
#[derive(Debug)]
pub struct PanePipe {
    /// Which pipe this is, so input from one since closed is dropped.
    pub id: u64,
    child: Option<Child>,
    tx: Option<SyncSender<Vec<u8>>>,
    /// The command types into the pane, and may never read its input to its
    /// end: closing the pipe kills it.
    input: bool,
}

impl PanePipe {
    /// Start `line` with the shell (`default-shell`, when set): its input
    /// fed from the pane with `-O`, its output sent to `pane` as
    /// [`Event::PipeInput`] with `-I`.
    pub fn open(
        id: u64,
        line: &str,
        default_shell: Option<&str>,
        dir: Direction,
        pane: PaneId,
        events: Sender<Event>,
    ) -> std::io::Result<Self> {
        let argv = crate::util::shell_command(line, default_shell);
        let piped = |on: bool| if on { Stdio::piped() } else { Stdio::null() };
        let mut child = Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(piped(dir.output))
            .stdout(piped(dir.input))
            .stderr(Stdio::null())
            .spawn()?;
        let tx = match child.stdin.take() {
            Some(mut stdin) => {
                let (tx, rx) = sync_channel::<Vec<u8>>(QUEUE_CHUNKS);
                std::thread::Builder::new()
                    .name("tmxr-pipe-pane".into())
                    .spawn(move || {
                        // Ends when the pipe closes (the sender goes) or the
                        // command stops reading.
                        for chunk in rx {
                            if stdin
                                .write_all(&chunk)
                                .and_then(|()| stdin.flush())
                                .is_err()
                            {
                                break;
                            }
                        }
                    })?;
                Some(tx)
            }
            None => None,
        };
        if let Some(mut stdout) = child.stdout.take() {
            std::thread::Builder::new()
                .name("tmxr-pipe-pane-in".into())
                .spawn(move || {
                    let mut buf = [0u8; 8192];
                    // Ends when the command closes its output or exits.
                    while let Ok(n) = stdout.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        let ev = Event::PipeInput {
                            pane,
                            pipe: id,
                            bytes: buf[..n].to_vec(),
                        };
                        if events.send(ev).is_err() {
                            break;
                        }
                    }
                })?;
        }
        Ok(Self {
            id,
            child: Some(child),
            tx,
            input: dir.input,
        })
    }

    /// Hand the command a chunk of the pane's output, dropping it if the
    /// command has fallen too far behind or has stopped.
    pub fn send(&self, bytes: &[u8]) {
        if let Some(tx) = &self.tx {
            // Full or stopped: the chunk is dropped, by design (QUEUE_CHUNKS).
            let _ = tx.try_send(bytes.to_vec());
        }
    }
}

impl Drop for PanePipe {
    /// Closing the pipe ends the command's input, as tmux does: a
    /// `cat`-like command writes what it has and finishes. One typing into
    /// the pane (`-I`) is killed, as it may never finish. It is reaped off
    /// the server's thread, so a slow one never holds the server up.
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(mut child) = self.child.take() {
            if self.input
                && let Err(e) = child.kill()
            {
                tracing::debug!("pipe-pane: could not stop the command: {e}");
            }
            let reaped = std::thread::Builder::new()
                .name("tmxr-pipe-reap".into())
                .spawn(move || child.wait());
            if let Err(e) = reaped {
                tracing::warn!("pipe-pane: could not reap the command: {e}");
            }
        }
    }
}
