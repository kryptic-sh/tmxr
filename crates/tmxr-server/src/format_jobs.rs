//! `#(command)` in formats, as tmux's format_job_*: each command a client's
//! formats name runs in the background (again at most once a second, or at
//! once when its expanded text changes), and the format shows the last line
//! it printed, or nothing until it has printed one (`<'command' not ready>`
//! after a second).

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::ClientId;
use crate::server::{ChildEnv, Event, Server};

/// Which job: the client whose formats name it (none from a command line),
/// and the command as written.
pub type JobKey = (Option<ClientId>, String);

/// The format jobs, by client and command.
#[derive(Debug, Default)]
pub struct FormatJobs {
    jobs: HashMap<JobKey, Job>,
}

#[derive(Debug, Default)]
struct Job {
    /// The command as last expanded, which a change runs again.
    expanded: String,
    /// What the format shows.
    out: Option<String>,
    running: bool,
    /// The second it last started, as tmux's fj->last.
    last: u64,
    /// It printed a whole line in its latest run.
    updated: bool,
    /// Which run it is on, so a run started since tells its events apart.
    run: u64,
}

/// What a job's run reports to the server.
#[derive(Debug)]
pub struct JobOutput {
    pub key: JobKey,
    pub run: u64,
    /// A whole line printed, or (when `done`) what was left after the last.
    pub text: String,
    pub done: bool,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Server {
    /// What `cmd` (expanded to `expanded`) shows for `client`'s formats,
    /// starting its run when it is due, as tmux's format_job_get.
    pub fn format_job(&self, client: Option<ClientId>, cmd: &str, expanded: &str) -> String {
        let Ok(mut jobs) = self.format_jobs.lock() else {
            return String::new();
        };
        let key = (client, cmd.to_owned());
        let job = jobs.jobs.entry(key.clone()).or_default();
        let t = now();
        let changed = job.expanded != expanded;
        if changed {
            expanded.clone_into(&mut job.expanded);
        }
        if changed || (!job.running && job.last != t) {
            job.running = true;
            job.last = t;
            job.updated = false;
            job.run += 1;
            self.start_format_job(key, job.run, expanded.to_owned());
        } else if job.running && t.saturating_sub(job.last) > 1 && job.out.is_none() {
            job.out = Some(format!("<'{cmd}' not ready>"));
        }
        job.out.clone().unwrap_or_default()
    }

    /// Run `expanded` with the client's session's shell, in its directory,
    /// each whole line it prints and then its end reported as events.
    fn start_format_job(&self, key: JobKey, run: u64, expanded: String) {
        let session = key
            .0
            .and_then(|c| self.clients.get(&c))
            .and_then(|c| c.att.as_ref())
            .map(|a| a.session);
        let shell = session.and_then(|s| self.session_options(s).default_shell.clone());
        let cwd: Option<PathBuf> = session
            .and_then(|s| self.sessions.get(&s))
            .map(|s| s.cwd.clone());
        let env: ChildEnv = self.child_env(session);
        let events = self.events.clone();
        let name = key.1.clone();
        let label = key.1.clone();
        let spawned = std::thread::Builder::new()
            .name("tmxr-format-job".into())
            .spawn(move || {
                let send = |text: String, done: bool| {
                    let output = JobOutput {
                        key: key.clone(),
                        run,
                        text,
                        done,
                    };
                    let _ = events.send(Event::FormatJob(Box::new(output)));
                };
                let argv = crate::util::shell_command(&expanded, shell.as_deref());
                let mut cmd = tmxr_term::child::command(&argv[0]);
                env.apply(&mut cmd);
                cmd.args(&argv[1..])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null());
                if let Some(dir) = cwd.filter(|d| d.is_dir()) {
                    cmd.current_dir(dir);
                }
                let Ok(mut child) = cmd.spawn() else {
                    send(format!("<'{name}' didn't start>"), true);
                    return;
                };
                let mut rest = Vec::new();
                if let Some(stdout) = child.stdout.take() {
                    let mut reader = BufReader::new(stdout);
                    let mut line = Vec::new();
                    loop {
                        line.clear();
                        match reader.read_until(b'\n', &mut line) {
                            Ok(0) | Err(_) => break,
                            Ok(_) if line.ends_with(b"\n") => {
                                let text = String::from_utf8_lossy(&line);
                                let text = text.trim_end_matches(['\n', '\r']);
                                send(text.to_owned(), false);
                            }
                            // The last of it, with no newline after.
                            Ok(_) => rest.extend_from_slice(&line),
                        }
                    }
                }
                // Reaped only: tmux shows no exit status for a job.
                let _ = child.wait();
                send(String::from_utf8_lossy(&rest).into_owned(), true);
            });
        if let Err(e) = spawned {
            let _ = self
                .events
                .send(Event::Log(format!("#({label}) failed: {e}")));
        }
    }

    /// Client `client`'s jobs dropped, as tmux frees them with the client;
    /// a run still going reports to nothing.
    pub fn forget_format_jobs(&self, client: ClientId) {
        if let Ok(mut jobs) = self.format_jobs.lock() {
            jobs.jobs.retain(|(c, _), _| *c != Some(client));
        }
    }

    /// A line or the end from a job's run, as tmux's format_job_update and
    /// format_job_complete: a line becomes what the format shows; at the
    /// end, what was left does too unless it is empty after a line. The
    /// client's status line is drawn again.
    pub fn format_job_output(&mut self, output: JobOutput) {
        let Ok(mut jobs) = self.format_jobs.lock() else {
            return;
        };
        let Some(job) = jobs
            .jobs
            .get_mut(&output.key)
            .filter(|j| j.run == output.run)
        else {
            return;
        };
        if output.done {
            job.running = false;
            if !output.text.is_empty() || !job.updated {
                job.out = Some(output.text);
            }
        } else {
            job.updated = true;
            job.out = Some(output.text);
        }
        drop(jobs);
        match output.key.0 {
            Some(c) => self.mark_client_dirty(c),
            None => self.mark_all_dirty(),
        }
    }
}
