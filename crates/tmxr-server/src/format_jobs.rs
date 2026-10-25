//! `#(command)` in formats, as tmux's format_job_*: each command a client's
//! formats name runs in the background (again at most once a second, or at
//! once when its expanded text changes, the run before killed), and the
//! format shows the last line it printed, or nothing until it has printed
//! one (`<'command' not ready>` after a second). A job no format has
//! started for an hour is dropped, as tmux's format_job_tidy.

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader};
use std::process::{Child, ChildStdout};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::ClientId;
use crate::server::{Event, Server};

/// Which job: the client whose formats name it (none from a command line),
/// and the command as written.
pub type JobKey = (Option<ClientId>, String);

/// The format jobs, by client and command.
#[derive(Debug, Default)]
pub struct FormatJobs {
    jobs: HashMap<JobKey, Job>,
}

/// How long a job may go without starting before it is dropped, in
/// seconds, as tmux's format_job_tidy.
const UNUSED_SECS: u64 = 3600;

impl FormatJobs {
    /// The jobs not started for [`UNUSED_SECS`] by `now` dropped, their runs
    /// stopped; one started after `now` (the clock went back) is kept.
    fn tidy(&mut self, now: u64) {
        self.jobs.retain(|_, job| {
            let keep = job.last > now || now - job.last < UNUSED_SECS;
            if !keep {
                job.stop();
            }
            keep
        });
    }
}

#[derive(Debug, Default)]
struct Job {
    /// The command as last expanded, which a change runs again.
    expanded: String,
    /// What the format shows.
    out: Option<String>,
    /// The run going, if one is.
    child: Option<Child>,
    /// The second it last started, as tmux's fj->last.
    last: u64,
    /// It printed a whole line in its latest run.
    updated: bool,
    /// Which run it is on, so a run started since tells its events apart.
    run: u64,
}

impl Job {
    /// The run going, if any, stopped as tmux's job_free stops it: sent
    /// SIGTERM on Unix (TerminateProcess on Windows), and reaped by a
    /// thread of its own, so one that ignores the signal does not hold up
    /// the server (it runs on, as under tmux).
    fn stop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        #[cfg(unix)]
        // SAFETY: `kill` takes any pid and signal; this pid is our own
        // child's, not yet reaped (the thread below reaps it), so it names
        // no other process.
        if let Ok(pid) = libc::pid_t::try_from(child.id()) {
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
        #[cfg(windows)]
        let _ = child.kill();
        let reaped = std::thread::Builder::new()
            .name("tmxr-format-job-reap".into())
            .spawn(move || child.wait());
        if let Err(e) = reaped {
            tracing::warn!("format job not reaped: {e}");
        }
    }
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
            job.stop();
        }
        if changed || (job.child.is_none() && job.last != t) {
            job.last = t;
            job.updated = false;
            job.run += 1;
            match self.spawn_format_job(&key, expanded) {
                Ok((child, stdout)) => {
                    job.child = Some(child);
                    self.read_format_job(key, job.run, stdout);
                }
                Err(_) => job.out = Some(format!("<'{cmd}' didn't start>")),
            }
        } else if job.child.is_some() && t.saturating_sub(job.last) > 1 && job.out.is_none() {
            job.out = Some(format!("<'{cmd}' not ready>"));
        }
        job.out.clone().unwrap_or_default()
    }

    /// `expanded` started with the client's session's shell, in its
    /// directory and environment, its output piped.
    fn spawn_format_job(
        &self,
        key: &JobKey,
        expanded: &str,
    ) -> std::io::Result<(Child, ChildStdout)> {
        let session = key
            .0
            .and_then(|c| self.clients.get(&c))
            .and_then(|c| c.att.as_ref())
            .map(|a| a.session);
        let shell = session.and_then(|s| self.session_options(s).default_shell.clone());
        let argv = crate::util::shell_command(expanded, shell.as_deref());
        let mut cmd = tmxr_term::child::command(&argv[0]);
        self.child_env(session).apply(&mut cmd);
        cmd.args(&argv[1..])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        if let Some(dir) = session
            .and_then(|s| self.sessions.get(&s))
            .map(|s| &s.cwd)
            .filter(|d| d.is_dir())
        {
            cmd.current_dir(dir);
        }
        let mut child = cmd.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("no output pipe"))?;
        Ok((child, stdout))
    }

    /// Each whole line the run prints, then its end with what was left,
    /// reported as events from a thread of its own.
    fn read_format_job(&self, key: JobKey, run: u64, stdout: ChildStdout) {
        let events = self.events.clone();
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
                let mut reader = BufReader::new(stdout);
                let mut rest = Vec::new();
                let mut line = Vec::new();
                loop {
                    line.clear();
                    match reader.read_until(b'\n', &mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if line.ends_with(b"\n") => {
                            let text = String::from_utf8_lossy(&line);
                            send(text.trim_end_matches(['\n', '\r']).to_owned(), false);
                        }
                        // The last of it, with no newline after.
                        Ok(_) => rest.extend_from_slice(&line),
                    }
                }
                send(String::from_utf8_lossy(&rest).into_owned(), true);
            });
        if let Err(e) = spawned {
            let _ = self
                .events
                .send(Event::Log(format!("#({label}) failed: {e}")));
        }
    }

    /// The jobs unused for an hour dropped, as tmux's format_tidy_jobs.
    pub fn tidy_format_jobs(&self) {
        if let Ok(mut jobs) = self.format_jobs.lock() {
            jobs.tidy(now());
        }
    }

    /// Client `client`'s jobs dropped, their runs stopped, as tmux frees
    /// them with the client.
    pub fn forget_format_jobs(&self, client: ClientId) {
        if let Ok(mut jobs) = self.format_jobs.lock() {
            jobs.jobs.retain(|(c, _), job| {
                let keep = *c != Some(client);
                if !keep {
                    job.stop();
                }
                keep
            });
        }
    }

    /// Every job dropped, its run stopped, as tmux frees each client's jobs
    /// when the server exits.
    pub fn stop_format_jobs(&self) {
        if let Ok(mut jobs) = self.format_jobs.lock() {
            for (_, mut job) in jobs.jobs.drain() {
                job.stop();
            }
        }
    }

    /// A line or the end from a job's run, as tmux's format_job_update and
    /// format_job_complete: a line becomes what the format shows; at the
    /// end, what was left does too unless it is empty after a line, and the
    /// run is reaped. The client's status line is drawn again.
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
            job.stop();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A run stopped is sent SIGTERM, as tmux's job_free does, and one
    /// that ignores it does not keep `stop` waiting.
    #[cfg(unix)]
    #[test]
    fn a_stopped_run_is_sent_sigterm_and_not_waited_for() {
        let dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("term");
        let run = |script: &str| Job {
            child: Some(
                tmxr_term::child::command("/bin/sh")
                    .arg("-c")
                    .arg(script)
                    .spawn()
                    .unwrap(),
            ),
            ..Job::default()
        };
        let mut job = run(&format!(
            "trap 'echo term > {}; exit' TERM; while :; do sleep 0.05; done",
            mark.display()
        ));
        // Give the shell time to set its trap.
        std::thread::sleep(std::time::Duration::from_millis(300));
        job.stop();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !mark.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(mark.exists(), "the run never saw SIGTERM");
        let mut stubborn = run("trap '' TERM; sleep 3");
        std::thread::sleep(std::time::Duration::from_millis(300));
        let started = std::time::Instant::now();
        stubborn.stop();
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn tidy_drops_the_jobs_unused_for_an_hour() {
        let mut jobs = FormatJobs::default();
        for (cmd, last) in [("old", 100), ("recent", 100 + UNUSED_SECS), ("ahead", 9999)] {
            let job = Job {
                last,
                ..Job::default()
            };
            jobs.jobs.insert((None, cmd.to_owned()), job);
        }
        jobs.tidy(100 + UNUSED_SECS);
        let mut left: Vec<&str> = jobs.jobs.keys().map(|(_, c)| c.as_str()).collect();
        left.sort_unstable();
        assert_eq!(left, ["ahead", "recent"]);
    }
}
