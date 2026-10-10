//! Small helpers: host name, environment, shell commands, base64, clipboard,
//! foreground-command refresh.

use std::path::PathBuf;

use crate::server::Server;

/// The machine's host name, for `#H` / `#h`.
pub fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: the buffer is valid for `len` bytes; gethostname writes at
        // most that many and we stop at the first NUL (or the end).
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if rc == 0 {
            let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
            return String::from_utf8_lossy(&buf[..end]).into_owned();
        }
        std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".into())
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".into())
    }
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// `TERM` for panes: the configured `default-terminal` when this machine has
/// a terminfo entry for it, else `xterm-256color` (programs fail outright on
/// an unknown `TERM`). Windows has no terminfo, so the value is used as is.
pub fn pane_term(configured: &str) -> String {
    #[cfg(unix)]
    {
        let first = configured.chars().next().unwrap_or('x');
        let hex = format!("{:x}", first as u32);
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(d) = std::env::var_os("TERMINFO") {
            dirs.push(PathBuf::from(d));
        }
        dirs.push(home_dir().join(".terminfo"));
        for d in [
            "/etc/terminfo",
            "/lib/terminfo",
            "/usr/share/terminfo",
            "/usr/lib/terminfo",
            "/usr/local/share/terminfo",
            "/opt/homebrew/share/terminfo",
        ] {
            dirs.push(PathBuf::from(d));
        }
        let found = dirs.iter().any(|d| {
            d.join(first.to_string()).join(configured).exists()
                || d.join(&hex).join(configured).exists()
        });
        if found {
            configured.to_owned()
        } else {
            "xterm-256color".to_owned()
        }
    }
    #[cfg(windows)]
    {
        configured.to_owned()
    }
}

/// argv that runs a shell command line, as tmux runs `new-window 'cmd args'`:
/// with `default-shell` when set, else the shell panes start by default.
pub fn shell_command(line: &str, default_shell: Option<&str>) -> Vec<String> {
    let sh = default_shell.filter(|s| !s.is_empty()).map_or_else(
        || {
            tmxr_term::pty::default_shell()
                .to_string_lossy()
                .into_owned()
        },
        str::to_owned,
    );
    // Each shell's own flag for "run this line": cmd's /c and PowerShell's
    // -Command on Windows, -c for the rest (sh, bash, zsh, fish, nu, ...).
    // Split on both separators: a Windows path is read the same anywhere.
    let stem = sh
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let stem = stem.strip_suffix(".exe").unwrap_or(&stem);
    let mut argv = vec![sh];
    match stem {
        "cmd" => argv.push("/c".into()),
        "pwsh" | "powershell" => argv.extend(["-NoLogo".into(), "-Command".into()]),
        _ => argv.push("-c".into()),
    }
    argv.push(line.to_owned());
    argv
}

/// The display name of the program a window starts with: the first word of
/// `argv`, or with no command the shell a pane starts (`default-shell`, else
/// the system's).
pub fn program_name(argv: &[String], default_shell: Option<&str>) -> String {
    let prog = match (argv.first(), default_shell) {
        (Some(p), _) => p.split_whitespace().next().unwrap_or(p).to_owned(),
        (None, Some(sh)) => sh.to_owned(),
        (None, None) => tmxr_term::pty::default_shell()
            .to_string_lossy()
            .into_owned(),
    };
    process_name(&prog)
}

/// Basename without directory or `.exe`, as tmux shows commands.
pub fn process_name(path: &str) -> String {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let base = base
        .len()
        .checked_sub(4)
        .filter(|&i| base.is_char_boundary(i) && base[i..].eq_ignore_ascii_case(".exe"))
        .map_or(base, |i| &base[..i]);
    base.strip_prefix('-').unwrap_or(base).to_owned()
}

/// Standard base64 (RFC 4648 §4, with padding), for OSC 52.
pub fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Copy to the local desktop clipboard, best effort, off the server thread
/// (a clipboard backend may block talking to the display server). Skipped
/// over SSH, where OSC 52 to the client is the only meaningful target.
pub fn local_clipboard(text: &str) {
    if std::env::var_os("SSH_CONNECTION").is_some() {
        return;
    }
    let text = text.to_owned();
    let _ = std::thread::Builder::new()
        .name("tmxr-clipboard".into())
        .spawn(move || match hjkl_clipboard::Clipboard::new() {
            Ok(cb) => {
                let r = cb.set(
                    hjkl_clipboard::Selection::Clipboard,
                    hjkl_clipboard::MimeType::Text,
                    text.as_bytes(),
                );
                if let Err(e) = r {
                    tracing::debug!(error = %e, "local clipboard copy failed");
                }
            }
            Err(e) => tracing::debug!(error = %e, "no local clipboard"),
        });
}

/// Refresh every pane's foreground command, and auto-named windows' names.
pub fn refresh_commands(srv: &mut Server) {
    let ids: Vec<_> = srv.panes.keys().copied().collect();
    for id in ids {
        let Some(p) = srv.panes.get(&id) else {
            continue;
        };
        if let Some(cmd) = tmxr_term::process::foreground_command(&p.pty) {
            srv.commands.insert(id, cmd);
        }
    }
    let renames: Vec<(u32, String)> = srv
        .windows
        .values()
        .filter(|w| w.auto_name)
        .filter_map(|w| srv.commands.get(&w.active).map(|c| (w.id, c.clone())))
        .collect();
    for (wid, name) in renames {
        if let Some(w) = srv.windows.get_mut(&wid)
            && w.name != name
        {
            w.name = name;
            srv.mark_window_dirty(wid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        // RFC 4648 §10 test vectors.
        for (input, want) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), want, "{input:?}");
        }
    }

    #[test]
    fn shell_commands_use_the_shells_own_flag() {
        let argv = |sh| shell_command("echo hi", Some(sh));
        assert_eq!(argv("/bin/sh"), ["/bin/sh", "-c", "echo hi"]);
        assert_eq!(argv("cmd.exe"), ["cmd.exe", "/c", "echo hi"]);
        assert_eq!(
            argv(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            [
                r"C:\Program Files\PowerShell\7\pwsh.exe",
                "-NoLogo",
                "-Command",
                "echo hi"
            ]
        );
        assert_eq!(
            argv("powershell"),
            ["powershell", "-NoLogo", "-Command", "echo hi"]
        );
        assert_eq!(argv("/usr/bin/fish"), ["/usr/bin/fish", "-c", "echo hi"]);
    }

    #[test]
    fn process_names_are_basenames() {
        assert_eq!(process_name("/usr/bin/fish"), "fish");
        assert_eq!(
            process_name(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            "pwsh"
        );
        assert_eq!(process_name("-zsh"), "zsh");
        assert_eq!(process_name("PING.EXE"), "PING");
        assert_eq!(program_name(&["hjkl src/main.rs".into()], None), "hjkl");
        assert_eq!(program_name(&[], Some("/usr/bin/fish")), "fish");
        assert_eq!(program_name(&["top".into()], Some("cmd.exe")), "top");
    }
}
