//! Copies to the local desktop's clipboard, beside the OSC 52 every tmxr
//! copy sends its clients.
//!
//! The desktop is the one the copying client sits at: its display variables
//! come from that client's environment, not the server's, which may be from
//! a session long gone (a server outlives a logout) or have none at all (a
//! server started from a TTY). A client over SSH gets OSC 52 only, its
//! desktop being elsewhere.
//!
//! On Linux and the BSDs a Wayland or X11 selection lives only as long as
//! the process that set it serves it, so each copy runs `tmxr __clipboard`
//! with the client's display, which sets the selection from its standard
//! input and serves it until the next copy replaces it (the server stops
//! the previous helper) or the server is gone. On macOS and Windows the
//! clipboard keeps a copy by itself, so it is set from the server.

/// The argument that makes `tmxr` the clipboard helper.
pub const CLIPBOARD_ARG: &str = "__clipboard";

/// Variables naming the desktop a copy goes to: taken from the copying
/// client, and cleared from the helper when the client has none.
#[cfg(all(unix, not(target_os = "macos")))]
const DISPLAY_VARS: [&str; 4] = [
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "XAUTHORITY",
    "XDG_RUNTIME_DIR",
];

/// Variables an SSH session sets, as hjkl-clipboard reads them: a client
/// with one is remote, and the helper never inherits them (hjkl-clipboard
/// would send OSC 52 instead of setting the desktop's clipboard).
const SSH_VARS: [&str; 3] = ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"];

/// How often the helper checks that the server that started it is still
/// there, where the kernel cannot tell it (not Linux).
#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
const PARENT_CHECK: std::time::Duration = std::time::Duration::from_secs(1);

/// The server's side: the helper serving the last copy.
#[derive(Debug, Default)]
pub struct Local {
    #[cfg(all(unix, not(target_os = "macos")))]
    helper: Option<std::process::Child>,
}

/// Whether the environment `var` reads is an SSH session's.
fn remote(var: impl Fn(&str) -> Option<String>) -> bool {
    SSH_VARS.iter().any(|name| var(name).is_some())
}

impl Local {
    /// Put `text` on the desktop clipboard of the client whose environment
    /// is `env`, or the server's own without one; best effort, as the desktop
    /// may be gone.
    pub fn copy(&mut self, text: &str, env: Option<&[(String, String)]>) {
        let var = |name: &str| match env {
            Some(env) => env.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()),
            None => std::env::var(name).ok(),
        };
        if remote(var) {
            return;
        }
        self.set(text, var);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn set(&mut self, text: &str, var: impl Fn(&str) -> Option<String>) {
        use std::io::Write as _;
        use std::process::Stdio;
        if let Some(mut old) = self.helper.take() {
            // Its selection is about to be replaced; a helper that already
            // exited is only reaped.
            let _ = old.kill();
            if let Err(e) = old.wait() {
                tracing::debug!(error = %e, "clipboard helper not reaped");
            }
        }
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(e) => {
                tracing::debug!(error = %e, "no clipboard helper");
                return;
            }
        };
        let mut cmd = tmxr_term::child::command(exe);
        cmd.arg(CLIPBOARD_ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for name in SSH_VARS {
            cmd.env_remove(name);
        }
        for name in DISPLAY_VARS {
            match var(name) {
                Some(value) => cmd.env(name, value),
                None => cmd.env_remove(name),
            };
        }
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                tracing::debug!(error = %e, "clipboard helper did not start");
                return;
            }
        };
        // The pipe holds the text until the helper reads it; a write that
        // fails means it is already gone.
        if let Some(mut stdin) = child.stdin.take()
            && let Err(e) = stdin.write_all(text.as_bytes())
        {
            tracing::debug!(error = %e, "clipboard helper took no text");
        }
        self.helper = Some(child);
    }

    /// macOS and Windows keep a copy once it is made, so the server sets it
    /// itself, off its own thread (a backend may block).
    #[cfg(any(windows, target_os = "macos"))]
    fn set(&mut self, text: &str, _var: impl Fn(&str) -> Option<String>) {
        let text = text.to_owned();
        let _ = std::thread::Builder::new()
            .name("tmxr-clipboard".into())
            .spawn(move || set_clipboard(text.as_bytes()));
    }
}

/// Set the desktop's clipboard to `text`, logging why when it cannot be.
fn set_clipboard(text: &[u8]) -> bool {
    let cb = match hjkl_clipboard::Clipboard::new() {
        Ok(cb) => cb,
        Err(e) => {
            tracing::debug!(error = %e, "no local clipboard");
            return false;
        }
    };
    match cb.set(
        hjkl_clipboard::Selection::Clipboard,
        hjkl_clipboard::MimeType::Text,
        text,
    ) {
        Ok(()) => true,
        Err(e) => {
            tracing::debug!(error = %e, "local clipboard copy failed");
            false
        }
    }
}

/// `tmxr __clipboard`: set the clipboard to standard input and serve it
/// until the server that started this is gone (the server stops it sooner,
/// when the next copy replaces it). Fails when there is no clipboard to set.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn helper() -> bool {
    use std::io::Read as _;
    let server = std::os::unix::process::parent_id();
    let mut text = Vec::new();
    if std::io::stdin().read_to_end(&mut text).is_err() || !set_clipboard(&text) {
        return false;
    }
    wait_for_server(server);
    true
}

/// Return once the server that started the helper is gone. Linux ends the
/// helper with SIGTERM when it is (`PR_SET_PDEATHSIG`; the server starts
/// helpers from its main thread, whose end is what the kernel watches), so
/// it sleeps until then without waking.
#[cfg(target_os = "linux")]
fn wait_for_server(server: u32) {
    // SAFETY: `prctl` with `PR_SET_PDEATHSIG` and a signal number only sets
    // this process's death signal; SIGTERM's default action ends it.
    if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) } != 0 {
        tracing::debug!(
            error = %std::io::Error::last_os_error(),
            "clipboard helper cannot follow the server"
        );
        return;
    }
    // The server may have gone before the death signal was set.
    while std::os::unix::process::parent_id() == server {
        std::thread::park();
    }
}

/// Return once the server that started the helper is gone, checking each
/// [`PARENT_CHECK`].
#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn wait_for_server(server: u32) {
    while std::os::unix::process::parent_id() == server {
        std::thread::sleep(PARENT_CHECK);
    }
}

/// macOS and Windows copy from the server itself; the helper is refused.
#[cfg(any(windows, target_os = "macos"))]
pub fn helper() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_over_ssh_is_remote() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        assert!(remote(env(&[("SSH_CONNECTION", "1.2.3.4 5 6.7.8.9 22")])));
        assert!(remote(env(&[("SSH_TTY", "/dev/pts/3")])));
        assert!(!remote(env(&[("WAYLAND_DISPLAY", "wayland-1")])));
    }
}
