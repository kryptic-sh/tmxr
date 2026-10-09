//! `pipe-pane`: a pane's output copied to a shell command's standard input,
//! as tmux does it (`pipe-pane 'cat >> ~/pane.log'`).

use std::io::Write as _;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{SyncSender, sync_channel};

/// Output chunks waiting for a slow command before more are dropped: the
/// server never waits on a pipe.
const QUEUE_CHUNKS: usize = 1024;

/// A pane's open pipe: the command, and the thread feeding its input.
#[derive(Debug)]
pub struct PanePipe {
    child: Option<Child>,
    tx: Option<SyncSender<Vec<u8>>>,
}

impl PanePipe {
    /// Start `line` with the shell, its input fed from the pane.
    pub fn open(line: &str) -> std::io::Result<Self> {
        let argv = crate::util::shell_command(line);
        let mut child = Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("stdin was piped");
        let (tx, rx) = sync_channel::<Vec<u8>>(QUEUE_CHUNKS);
        std::thread::Builder::new()
            .name("tmxr-pipe-pane".into())
            .spawn(move || {
                // Ends when the pipe closes (the sender goes) or the command
                // stops reading.
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
        Ok(Self {
            child: Some(child),
            tx: Some(tx),
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
    /// `cat`-like command writes what it has and finishes. It is reaped off
    /// the server's thread, so a slow one never holds the server up.
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(mut child) = self.child.take() {
            let reaped = std::thread::Builder::new()
                .name("tmxr-pipe-reap".into())
                .spawn(move || child.wait());
            if let Err(e) = reaped {
                tracing::warn!("pipe-pane: could not reap the command: {e}");
            }
        }
    }
}
