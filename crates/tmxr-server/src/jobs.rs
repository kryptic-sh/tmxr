//! `run-shell` jobs. tmux's command queue waits for a `run-shell` not given
//! `-b`: the commands after it in its list run once it finishes, and a
//! command client's reply carries its output. A job is a shell command, or
//! with `-C` a tmux command, started after an optional delay (`-d`).

use std::path::PathBuf;
use std::time::Duration;

use crate::cmds::{Ctx, Outcome};
use crate::model::ClientId;
use crate::server::{Event, Server};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    /// A shell command line, run in `cwd` (the server's own when `None`).
    Shell { line: String, cwd: Option<PathBuf> },
    /// A tmux command line (`run-shell -C`), run on the server thread.
    Command(String),
    /// Only the delay (`run-shell -d` with no command).
    Wait,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub delay: Duration,
    pub work: Work,
    /// The commands after the `run-shell` in its list, run once it is done.
    pub rest: Vec<Vec<String>>,
}

/// Where a finished job's output goes.
pub struct Then {
    pub ctx: Ctx,
    /// A command client waiting for its reply, and the reply so far; `None`
    /// shows the output on `ctx.client`, if any.
    pub reply: Option<(ClientId, Outcome)>,
}

/// A finished job, with its shell command's output.
pub struct Done {
    pub job: Job,
    pub output: String,
    pub then: Then,
}

impl Server {
    /// Start `job` off the server thread; [`Server::job_done`] takes it up
    /// when it finishes.
    pub fn start_job(&self, job: Job, then: Then) {
        let events = self.events.clone();
        let shell = self.cfg.default_shell.clone();
        let started = std::thread::Builder::new()
            .name("tmxr-run-shell".into())
            .spawn(move || {
                std::thread::sleep(job.delay);
                let output = match &job.work {
                    Work::Shell { line, cwd } => {
                        crate::server::shell_text(line, cwd.as_deref(), shell.as_deref())
                    }
                    Work::Command(_) | Work::Wait => String::new(),
                };
                let _ = events.send(Event::JobDone(Box::new(Done { job, output, then })));
            });
        if let Err(e) = started {
            tracing::warn!("run-shell: could not start a thread for the job: {e}");
        }
    }

    /// A job finished: deliver its output, then run the rest of its list.
    pub fn job_done(&mut self, Done { job, output, then }: Done) {
        let done = match &job.work {
            Work::Command(cmd) => crate::cmds::run_string(self, &then.ctx, cmd),
            _ => Outcome {
                stdout: output,
                ..Outcome::default()
            },
        };
        match then.reply {
            Some((id, mut out)) => {
                append(&mut out, done);
                let rest = crate::cmds::run_list(self, &then.ctx, &job.rest);
                append(&mut out, rest);
                self.conclude_command(id, &then.ctx, out);
            }
            None => {
                let rest = crate::cmds::run_detached(self, &then.ctx, &job.rest);
                if let Some(c) = then.ctx.client {
                    for out in [done, rest] {
                        if !out.stdout.is_empty() || !out.stderr.is_empty() {
                            self.report(c, &out);
                        }
                    }
                }
            }
        }
    }
}

/// Add `more`, which ran after `out`, to it: the output follows, and what
/// `more` asks for next (an error status, a job, a wait, an attach) is what
/// happens next.
fn append(out: &mut Outcome, more: Outcome) {
    out.stdout.push_str(&more.stdout);
    out.stderr.push_str(&more.stderr);
    if more.status != 0 {
        out.status = more.status;
    }
    out.job = more.job;
    out.wait = more.wait;
    if more.attach.is_some() {
        out.attach = more.attach;
    }
}
