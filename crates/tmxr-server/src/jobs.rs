//! `run-shell` jobs. tmux's command queue waits for a `run-shell` not given
//! `-b`: the commands after it in its list run once it finishes, and a
//! command client's reply carries its output. A job is a shell command, or
//! with `-C` a tmux command, started after an optional delay (`-d`).

use std::path::PathBuf;
use std::time::Duration;

use crate::cmds::{Ctx, Outcome};
use crate::model::{ClientId, PaneId, SessionId};
use crate::server::{Event, Server};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    /// A shell command line, run in `cwd` (the server's own when `None`);
    /// its error output is shown with its output when `stderr` (`-E`).
    Shell {
        line: String,
        cwd: Option<PathBuf>,
        stderr: bool,
        /// Where its output is shown, if not to whoever ran it.
        show_in: Option<ShowIn>,
    },
    /// A tmux command line (`run-shell -C`), run on the server thread.
    Command(String),
    /// `if-shell`: run `line`, then `then` if it succeeded, else
    /// `otherwise`, on the server thread.
    If {
        line: String,
        then: String,
        otherwise: Option<String>,
    },
    /// Only the delay (`run-shell -d` with no command).
    Wait,
    /// `wait-for`: a signal on a channel, or its lock.
    Channel(crate::waits::Wait),
}

/// A pane whose view mode a `run-shell`'s output is written to, as tmux's
/// cmd_run_shell_print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShowIn {
    /// The one `-t` named; if it is gone, as [`ShowIn::Client`].
    Pane(PaneId),
    /// `-b`'s: the client's active pane, or with none the current
    /// session's (tmux's cmd_find_from_nothing).
    Client(Option<ClientId>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub delay: Duration,
    pub work: Work,
    /// The session whose environment a shell command runs in (`TMXR`
    /// names it), as tmux runs a job for its target session.
    pub session: Option<SessionId>,
    /// The commands after the `run-shell` in its list, run once it is done.
    pub rest: Vec<tmxr_command::Line>,
}

/// Where a finished job's output goes.
pub struct Then {
    pub ctx: Ctx,
    /// A command client waiting for its reply, and the reply so far; `None`
    /// shows the output on `ctx.client`, if any.
    pub reply: Option<(ClientId, Outcome)>,
}

/// A finished job, with its shell command's output, whether it succeeded,
/// and its return code (a command client's exit status, as tmux's).
pub struct Done {
    pub job: Job,
    pub output: String,
    pub ok: bool,
    pub status: i32,
    pub then: Then,
}

impl Server {
    /// Start `job` off the server thread, or for `wait-for` queue it on its
    /// channel; [`Server::job_done`] takes it up when it finishes.
    pub fn start_job(&mut self, job: Job, then: Then) {
        // The command that made the job, for an error starting it.
        let name = match &job.work {
            Work::Channel(wait) => {
                let wait = wait.clone();
                let done = Done {
                    job,
                    output: String::new(),
                    ok: true,
                    status: 0,
                    then,
                };
                self.waits.enqueue(done, &wait);
                return;
            }
            Work::If { .. } => "if-shell",
            Work::Shell { .. } | Work::Command(_) | Work::Wait => "run-shell",
        };
        let events = self.events.clone();
        let shell = self
            .pane_session_options(then.ctx.pane)
            .default_shell
            .clone();
        let env = self.child_env(job.session);
        // The job is handed over once its thread has started, so a thread
        // that cannot start leaves it here to fail.
        let (hand, take) = std::sync::mpsc::channel::<(Job, Then)>();
        let started = std::thread::Builder::new()
            .name("tmxr-run-shell".into())
            .spawn(move || {
                let Ok((job, then)) = take.recv() else {
                    return;
                };
                std::thread::sleep(job.delay);
                let (output, ok, status) = match &job.work {
                    Work::Shell {
                        line, cwd, stderr, ..
                    } => {
                        let (output, status) = crate::server::shell_text(
                            line,
                            cwd.as_deref(),
                            shell.as_deref(),
                            &env,
                            *stderr,
                        );
                        (output, true, status)
                    }
                    Work::If { line, .. } => (
                        String::new(),
                        crate::server::shell_succeeds(line, shell.as_deref(), &env),
                        0,
                    ),
                    Work::Command(_) | Work::Wait | Work::Channel(_) => (String::new(), true, 0),
                };
                let done = Done {
                    job,
                    output,
                    ok,
                    status,
                    then,
                };
                let _ = events.send(Event::JobDone(Box::new(done)));
            });
        let failed = match started {
            Ok(_) => hand.send((job, then)).err().map(|e| {
                let (_, then) = e.0;
                (then, "the job's thread has gone".to_owned())
            }),
            Err(e) => Some((then, e.to_string())),
        };
        if let Some((then, e)) = failed {
            let msg = format!("{name}: could not start the job: {e}");
            tracing::warn!("{msg}");
            self.job_failed(then, msg);
        }
    }

    /// A job could not be started: its list stops there with `msg`, as at a
    /// command's error, replied to the client waiting on it or shown on
    /// `ctx.client`.
    fn job_failed(&mut self, Then { ctx, reply }: Then, msg: String) {
        let err = Outcome::error(msg);
        match reply {
            Some((id, mut out)) => {
                append(&mut out, err);
                self.conclude_command(id, &ctx, out);
            }
            None => {
                if let Some(c) = ctx.client {
                    self.report(c, &err);
                }
            }
        }
    }

    /// A job finished: deliver its output, then run the rest of its list.
    pub fn job_done(
        &mut self,
        Done {
            job,
            output,
            ok,
            status,
            then,
        }: Done,
    ) {
        let ctx = &then.ctx;
        let mut done = match &job.work {
            Work::Command(cmd) => crate::cmds::run_line(self, ctx, cmd),
            Work::If {
                then: yes,
                otherwise,
                ..
            } => match if ok { Some(yes) } else { otherwise.as_ref() } {
                Some(cmd) => crate::cmds::run_line(self, ctx, cmd),
                None => Outcome::default(),
            },
            Work::Shell {
                show_in: Some(to), ..
            } => {
                if let Some(pane) = self.show_in_pane(*to, ctx) {
                    crate::copy::add_to_view(self, pane, &output);
                }
                Outcome {
                    status,
                    ..Outcome::default()
                }
            }
            Work::Shell { .. } | Work::Wait | Work::Channel(_) => Outcome {
                stdout: output,
                status,
                ..Outcome::default()
            },
        };
        // A run-shell in what ran just now holds the rest of the list too.
        let rest = match done.job.as_mut() {
            Some(inner) => {
                inner.rest.extend(job.rest);
                Vec::new()
            }
            None => job.rest,
        };
        match then.reply {
            Some((id, mut out)) => {
                append(&mut out, done);
                let rest = crate::cmds::run_list(self, ctx, &rest);
                append(&mut out, rest);
                self.conclude_command(id, ctx, out);
            }
            None => {
                if let Some(inner) = done.job.take() {
                    self.start_job(
                        inner,
                        Then {
                            ctx: ctx.clone(),
                            reply: None,
                        },
                    );
                }
                let rest = crate::cmds::run_detached(self, ctx, &rest);
                if let Some(c) = ctx.client {
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

impl Server {
    /// The pane `to` names, if any is left.
    fn show_in_pane(&self, to: ShowIn, ctx: &Ctx) -> Option<PaneId> {
        if let ShowIn::Pane(p) = to
            && self.panes.contains_key(&p)
        {
            return Some(p);
        }
        let client = match to {
            ShowIn::Client(c) => c,
            ShowIn::Pane(_) => ctx.client,
        };
        let session = client
            .and_then(|c| self.clients.get(&c)?.att.as_ref())
            .map(|a| a.session)
            .or_else(|| crate::target::current_session(self, ctx))?;
        self.active_pane_of_session(session)
    }
}

/// Add `more`, which ran after `out`, to it: the output follows, and what
/// `more` asks for next (an error status, a job, an attach) is what happens
/// next.
fn append(out: &mut Outcome, more: Outcome) {
    out.stdout.push_str(&more.stdout);
    out.stderr.push_str(&more.stderr);
    if more.status != 0 {
        out.status = more.status;
    }
    if more.job.is_some() {
        out.job = more.job;
    }
    if more.attach.is_some() {
        out.attach = more.attach;
    }
}

#[cfg(test)]
mod tests {
    use tmxr_proto::ServerMsg;

    use super::*;

    #[test]
    fn a_job_that_cannot_start_fails_its_waiting_client() {
        let (mut srv, rx) = crate::server::testing::with_client(None);
        let earlier = Outcome {
            stdout: "x".into(),
            ..Outcome::default()
        };
        let then = Then {
            ctx: Ctx::default(),
            reply: Some((1, earlier)),
        };
        srv.job_failed(then, "if-shell: could not start the job: no thread".into());
        let Ok(ServerMsg::CommandResult {
            status,
            stdout,
            stderr,
        }) = rx.try_recv()
        else {
            panic!("the waiting client got no reply");
        };
        assert_eq!(status, 1);
        assert_eq!(stdout, "x", "what ran before the job is kept");
        assert_eq!(stderr, "if-shell: could not start the job: no thread");
    }
}
