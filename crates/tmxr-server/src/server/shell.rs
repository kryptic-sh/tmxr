//! Shell commands run off the state thread (`run-shell`, `if-shell`).

use std::path::Path;

use super::{Event, Server};
use crate::cmds::Ctx;

/// `run-shell`'s output: what `line` printed (standard output, then error),
/// or a line saying it failed, as tmux reports a command that printed
/// nothing. Run on the calling thread, in `cwd` when given.
pub fn shell_text(line: &str, cwd: Option<&Path>, shell: Option<&str>) -> String {
    match shell_output(line, cwd, shell) {
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

/// `if-shell`: run `line` in the background, then run `then` if it exited
/// successfully, else `otherwise`, with `ctx` as the command context.
pub fn if_shell(srv: &Server, ctx: Ctx, line: String, then: String, otherwise: Option<String>) {
    let events = srv.events.clone();
    let shell = srv.cfg.default_shell.clone();
    let _ = std::thread::Builder::new()
        .name("tmxr-if-shell".into())
        .spawn(move || {
            let ok = shell_output(&line, None, shell.as_deref()).is_ok_and(|o| o.status.success());
            if let Some(cmd) = if ok { Some(then) } else { otherwise } {
                let _ = events.send(Event::Run { ctx, cmd });
            }
        });
}

/// `copy-pipe`: run `line` with the shell, `input` on its standard input,
/// off the state thread. A failure goes to the message log.
pub fn pipe_to_shell(srv: &Server, line: String, input: String) {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let events = srv.events.clone();
    let shell = srv.cfg.default_shell.clone();
    let _ = std::thread::Builder::new()
        .name("tmxr-copy-pipe".into())
        .spawn(move || {
            let argv = crate::util::shell_command(&line, shell.as_deref());
            let run = Command::new(&argv[0])
                .args(&argv[1..])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    if let Some(mut stdin) = child.stdin.take() {
                        stdin.write_all(input.as_bytes())?;
                    }
                    child.wait_with_output()
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
) -> std::io::Result<std::process::Output> {
    let argv = crate::util::shell_command(line, shell);
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]).stdin(std::process::Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.output()
}
