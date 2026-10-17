//! Shell commands run off the state thread (`run-shell`, `if-shell`).

use std::path::Path;
use std::process::Command;

use super::{Event, Server};
use crate::model::PaneId;

/// The environment a child process starts in ([`Server::child_env`]): the
/// variables it gets and the ones it loses.
#[derive(Debug, Clone, Default)]
pub struct ChildEnv {
    pub set: Vec<(String, String)>,
    pub remove: Vec<String>,
}

impl ChildEnv {
    /// Give `cmd`'s child this environment, over the server's own.
    pub fn apply(&self, cmd: &mut Command) {
        for k in &self.remove {
            cmd.env_remove(k);
        }
        cmd.envs(self.set.iter().map(|(k, v)| (k, v)));
    }
}

/// `run-shell`'s output: what `line` printed (standard output, then error),
/// or a line saying it failed, as tmux reports a command that printed
/// nothing. Run on the calling thread, in `cwd` when given, with `env`.
pub fn shell_text(line: &str, cwd: Option<&Path>, shell: Option<&str>, env: &ChildEnv) -> String {
    match shell_output(line, cwd, shell, env) {
        Ok(o) => {
            let mut t = String::from_utf8_lossy(&o.stdout).into_owned();
            t.push_str(&String::from_utf8_lossy(&o.stderr));
            if !o.status.success() && t.is_empty() {
                t = format!("'{line}' returned {}\n", o.status.code().unwrap_or(-1));
            }
            t
        }
        Err(e) => format!("'{line}' failed: {e}\n"),
    }
}

/// `if-shell`'s test: whether `line` ran and exited successfully. Run on
/// the calling thread.
pub fn shell_succeeds(line: &str, shell: Option<&str>, env: &ChildEnv) -> bool {
    shell_output(line, None, shell, env).is_ok_and(|o| o.status.success())
}

/// `copy-pipe`: run `line` with `pane`'s session's shell, `input` on its
/// standard input, off the state thread. A failure goes to the message log.
pub fn pipe_to_shell(srv: &Server, pane: PaneId, line: String, input: String) {
    use std::io::Write as _;
    use std::process::Stdio;
    let events = srv.events.clone();
    let shell = srv.pane_session_options(Some(pane)).default_shell.clone();
    let env = srv.child_env(srv.session_of_pane(Some(pane)));
    let _ = std::thread::Builder::new()
        .name("tmxr-copy-pipe".into())
        .spawn(move || {
            let argv = crate::util::shell_command(&line, shell.as_deref());
            let mut cmd = tmxr_term::child::command(&argv[0]);
            env.apply(&mut cmd);
            let run = cmd
                .args(&argv[1..])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    // The input goes in from its own thread while the error
                    // output is read, so a command that fills its error
                    // output before reading cannot wait on us forever; one
                    // that stops reading early (a broken pipe) chose to.
                    let feeder = child.stdin.take().map(|mut stdin| {
                        std::thread::spawn(move || match stdin.write_all(input.as_bytes()) {
                            Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => Err(e),
                            _ => Ok(()),
                        })
                    });
                    let output = child.wait_with_output()?;
                    if let Some(feeder) = feeder {
                        feeder
                            .join()
                            .map_err(|_| std::io::Error::other("the input thread panicked"))??;
                    }
                    Ok(output)
                });
            let failure = match run {
                Ok(o) if o.status.success() => return,
                Ok(o) => String::from_utf8_lossy(&o.stderr).trim().to_owned(),
                Err(e) => e.to_string(),
            };
            let _ = events.send(Event::Log(format!("copy-pipe '{line}' failed: {failure}")));
        });
}

/// Run a shell command line to completion with no input, in `cwd` when
/// given.
fn shell_output(
    line: &str,
    cwd: Option<&Path>,
    shell: Option<&str>,
    env: &ChildEnv,
) -> std::io::Result<std::process::Output> {
    let argv = crate::util::shell_command(line, shell);
    let mut cmd = tmxr_term::child::command(&argv[0]);
    env.apply(&mut cmd);
    cmd.args(&argv[1..]).stdin(std::process::Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.output()
}
