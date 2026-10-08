//! Shell commands run off the state thread (`run-shell`, `if-shell`).

use super::{Event, Server};
use crate::cmds::Ctx;
use crate::model::ClientId;

/// `run-shell`: run `line` with the shell off the server thread and report
/// its output to `client` (unless `background`).
pub fn run_shell(srv: &Server, client: Option<ClientId>, line: String, background: bool) {
    let events = srv.events.clone();
    let _ = std::thread::Builder::new()
        .name("tmxr-run-shell".into())
        .spawn(move || {
            let text = match shell_output(&line) {
                Ok(o) => {
                    let mut t = String::from_utf8_lossy(&o.stdout).into_owned();
                    t.push_str(&String::from_utf8_lossy(&o.stderr));
                    if !o.status.success() && t.is_empty() {
                        t = format!("'{line}' returned {}", o.status.code().unwrap_or(-1));
                    }
                    t
                }
                Err(e) => format!("'{line}' failed: {e}"),
            };
            let client = if background { None } else { client };
            let _ = events.send(Event::Shell {
                client,
                output: text,
            });
        });
}

/// `if-shell`: run `line` in the background, then run `then` if it exited
/// successfully, else `otherwise`, with `ctx` as the command context.
pub fn if_shell(srv: &Server, ctx: Ctx, line: String, then: String, otherwise: Option<String>) {
    let events = srv.events.clone();
    let _ = std::thread::Builder::new()
        .name("tmxr-if-shell".into())
        .spawn(move || {
            let ok = shell_output(&line).is_ok_and(|o| o.status.success());
            if let Some(cmd) = if ok { Some(then) } else { otherwise } {
                let _ = events.send(Event::Run { ctx, cmd });
            }
        });
}

/// Run a shell command line to completion with no input.
fn shell_output(line: &str) -> std::io::Result<std::process::Output> {
    let argv = crate::util::shell_command(line);
    std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .output()
}
