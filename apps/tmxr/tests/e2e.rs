//! End-to-end: the real `tmxr` binary in a real pseudo-terminal (a ConPTY on
//! Windows), driven with keystrokes and checked by what its screen shows.
//!
//! Every test gets its own socket label, socket directory and XDG roots, so
//! nothing touches a running server or the user's config and saved sessions.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tmxr_term::Emulator;

const COLS: u16 = 100;
const ROWS: u16 = 30;
const TIMEOUT: Duration = Duration::from_secs(30);

struct Tmxr {
    dir: tempfile::TempDir,
    label: String,
    config: PathBuf,
}

impl Tmxr {
    fn new(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let label = format!("e2e-{name}-{}", std::process::id());
        let shell = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
        let config = dir.path().join("config.toml");
        // No clipboard: copying in a test must not overwrite the desktop's.
        std::fs::write(
            &config,
            format!("default-shell = {shell:?}\nset-clipboard = \"off\"\n[resurrect]\nrestore-on-start = false\nauto-save-minutes = 0\n"),
        )
        .unwrap();
        Self { dir, label, config }
    }

    fn env(&self) -> Vec<(String, String)> {
        let d = self.dir.path();
        let mut env = vec![
            (
                "TMXR_TMPDIR".to_owned(),
                d.join("sock").display().to_string(),
            ),
            ("TERM".to_owned(), "xterm-256color".to_owned()),
            ("TMXR".to_owned(), String::new()),
        ];
        for (var, sub) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
        ] {
            env.push((var.to_owned(), d.join(sub).display().to_string()));
        }
        env
    }

    fn args<'a>(&'a self, rest: &[&'a str]) -> Vec<String> {
        let mut a = vec![
            "-L".to_owned(),
            self.label.clone(),
            "-f".to_owned(),
            self.config.display().to_string(),
        ];
        a.extend(rest.iter().map(|s| (*s).to_owned()));
        a
    }

    /// Run a command client.
    fn output(&self, rest: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_tmxr"))
            .args(self.args(rest))
            .envs(self.env())
            .output()
            .unwrap()
    }

    /// Run a command client and return its stdout.
    fn run(&self, rest: &[&str]) -> String {
        String::from_utf8_lossy(&self.output(rest).stdout).into_owned()
    }

    /// Wait until a command's output satisfies `pred`.
    fn wait_run(&self, rest: &[&str], what: &str, pred: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let full = self.output(rest);
            let out = String::from_utf8_lossy(&full.stdout).into_owned();
            if pred(&out) {
                return out;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; last output ({}):\n{out}\nstderr:\n{}",
                full.status,
                String::from_utf8_lossy(&full.stderr)
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Start an attaching client in a pseudo-terminal.
    fn attach(&self, rest: &[&str]) -> Screen {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_tmxr"));
        cmd.args(self.args(rest));
        for (k, v) in self.env() {
            cmd.env(k, v);
        }
        cmd.cwd(self.dir.path());
        let child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let emu = Arc::new(Mutex::new(Emulator::new(ROWS, COLS, 0)));
        let raw = Arc::new(Mutex::new(Vec::new()));
        {
            let emu = Arc::clone(&emu);
            let writer = Arc::clone(&writer);
            let raw = Arc::clone(&raw);
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    raw.lock().unwrap().extend_from_slice(&buf[..n]);
                    // Answer terminal queries (ConPTY's cursor-position
                    // probe, crossterm's capability probes) like a terminal.
                    let replies = emu.lock().unwrap().process(&buf[..n]);
                    if !replies.is_empty() {
                        let _ = writer.lock().unwrap().write_all(&replies);
                    }
                }
            });
        }
        Screen {
            _master: pair.master,
            writer,
            emu,
            raw,
            child,
        }
    }
}

impl Drop for Tmxr {
    fn drop(&mut self) {
        let _ = Command::new(env!("CARGO_BIN_EXE_tmxr"))
            .args(self.args(&["kill-server"]))
            .envs(self.env())
            .output();
    }
}

struct Screen {
    _master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    emu: Arc<Mutex<Emulator>>,
    raw: Arc<Mutex<Vec<u8>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl Screen {
    fn text(&self) -> String {
        self.emu.lock().unwrap().screen().contents()
    }

    fn send(&self, bytes: &[u8]) {
        let mut w = self.writer.lock().unwrap();
        w.write_all(bytes).unwrap();
        w.flush().unwrap();
    }

    fn wait_for(&self, what: &str, pred: impl Fn(&str) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let text = self.text();
            if pred(&text) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; screen:\n{text}\nraw tail: {:?}",
                String::from_utf8_lossy(
                    &self
                        .raw
                        .lock()
                        .unwrap()
                        .iter()
                        .rev()
                        .take(400)
                        .rev()
                        .copied()
                        .collect::<Vec<_>>()
                )
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn wait_exit(&mut self) {
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("client did not exit; screen:\n{}", self.text());
    }

    /// The status line's cells: `(contents, fg, bg)`, colours as `Debug`
    /// text (`Rgb(r, g, b)`).
    fn status_cells(&self) -> Vec<(String, String, String)> {
        self.row_cells(ROWS - 1)
    }

    /// One screen row's cells, as [`Screen::status_cells`].
    fn row_cells(&self, row: u16) -> Vec<(String, String, String)> {
        let emu = self.emu.lock().unwrap();
        let screen = emu.screen();
        (0..COLS)
            .filter_map(|col| screen.cell(row, col))
            .map(|c| {
                (
                    c.contents().to_owned(),
                    format!("{:?}", c.fgcolor()),
                    format!("{:?}", c.bgcolor()),
                )
            })
            .collect()
    }

    fn raw_text(&self) -> String {
        String::from_utf8_lossy(&self.raw.lock().unwrap()).into_owned()
    }
}

const PREFIX: &[u8] = b"\x02";

#[test]
fn attach_type_split_navigate_detach_reattach() {
    let t = Tmxr::new("flow");
    let s = t.attach(&["new", "-s", "e2e"]);

    // The status line names the session.
    s.wait_for("status line", |text| text.contains("e2e"));

    // Typing reaches the shell in the pane.
    s.send(b"echo tmxr-e2e-marker\r");
    s.wait_for("echoed marker", |text| {
        text.matches("tmxr-e2e-marker").count() >= 2
    });

    // prefix % splits left/right: two panes, the new one active.
    s.send(PREFIX);
    s.send(b"%");
    t.wait_run(&["list-panes", "-t", "e2e"], "two panes", |o| {
        o.lines().count() == 2
    });
    s.wait_for("vertical border", |text| text.contains('│'));
    let active = |t: &Tmxr| t.run(&["display-message", "-p", "-t", "e2e", "#{pane_index}"]);
    assert_eq!(active(&t).trim(), "1");

    // prefix h focuses the left pane (vim-style selection from the config).
    s.send(PREFIX);
    s.send(b"h");
    t.wait_run(
        &["display-message", "-p", "-t", "e2e", "#{pane_index}"],
        "left pane",
        |o| o.trim() == "0",
    );

    // C-l in a plain shell moves right (navigator: no editor in front).
    s.send(b"\x0c");
    t.wait_run(
        &["display-message", "-p", "-t", "e2e", "#{pane_index}"],
        "right pane",
        |o| o.trim() == "1",
    );

    // prefix d detaches; the session keeps running.
    let mut s = s;
    s.send(PREFIX);
    s.send(b"d");
    s.wait_exit();
    assert!(
        s.raw_text().contains("detached (from session e2e)"),
        "{}",
        s.raw_text()
    );
    let ls = t.run(&["ls"]);
    assert!(ls.contains("e2e: 1 windows"), "{ls}");

    // Reattach: the pane still shows what was typed before detaching.
    let s = t.attach(&["attach", "-t", "e2e"]);
    s.wait_for("preserved output", |text| text.contains("tmxr-e2e-marker"));
}

#[test]
fn command_clients_start_and_stop_a_server() {
    let t = Tmxr::new("cmd");
    assert!(t.run(&["ls"]).is_empty(), "no server yet");
    t.run(&["new-session", "-d", "-s", "bg"]);
    t.run(&["new-window", "-d", "-t", "bg", "-n", "second"]);
    let windows = t.wait_run(&["list-windows", "-t", "bg"], "two windows", |o| {
        o.lines().count() == 2
    });
    assert!(windows.contains("second"), "{windows}");
    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);
}

#[test]
fn session_picker_switches_by_enter_filter_and_jk() {
    let t = Tmxr::new("picker");
    t.run(&["new-session", "-d", "-s", "alpha"]);
    let s = t.attach(&["new", "-s", "bravo"]);
    s.wait_for("status line", |text| text.contains("bravo"));
    let attached = |ls: &str, name: &str| {
        ls.lines()
            .any(|l| l.starts_with(&format!("{name}:")) && l.contains("(attached)"))
    };

    // prefix s opens the picker on the previous session; Enter switches.
    s.send(PREFIX);
    s.send(b"s");
    s.wait_for("picker", |text| text.contains("sessions 2/2"));
    s.send(b"\r");
    let ls = t.wait_run(&["ls"], "alpha attached", |o| attached(o, "alpha"));
    assert!(!attached(&ls, "bravo"), "{ls}");

    // Typing filters: "char" leaves one row, Enter switches to it.
    t.run(&["new-session", "-d", "-s", "charlie"]);
    s.send(PREFIX);
    s.send(b"s");
    s.wait_for("picker", |text| text.contains("sessions 3/3"));
    s.send(b"char");
    s.wait_for("filtered", |text| text.contains("sessions 1/3"));
    s.send(b"\r");
    t.wait_run(&["ls"], "charlie attached", |o| attached(o, "charlie"));

    // Escape to normal mode, j moves down a row: charlie (current), alpha
    // (where the picker opens), then bravo.
    s.send(PREFIX);
    s.send(b"s");
    s.wait_for("picker", |text| text.contains("sessions 3/3"));
    s.send(b"\x1b");
    s.wait_for("normal mode", |text| text.contains("[normal]"));
    s.send(b"j");
    std::thread::sleep(Duration::from_millis(200));
    s.send(b"\r");
    t.wait_run(&["ls"], "bravo attached", |o| attached(o, "bravo"));
}

#[test]
fn resurrect_restores_a_multi_pane_layout() {
    let t = Tmxr::new("resurrect");
    t.run(&["new-session", "-d", "-s", "main"]);
    t.run(&["split-window", "-h", "-t", "main"]);
    t.run(&["split-window", "-v", "-t", "main"]);
    t.run(&["resize-pane", "-L", "-t", "main", "5"]);
    t.wait_run(&["list-panes", "-t", "main"], "three panes", |o| {
        o.lines().count() == 3
    });
    // `0: [38x23] %0 (active)`: pane ids change on restore, so drop them and
    // compare index, size and the active mark.
    let layout = |t: &Tmxr| -> String {
        t.run(&["list-panes", "-t", "main"])
            .lines()
            .map(|l| {
                l.split_whitespace()
                    .filter(|w| !w.starts_with('%'))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let saved = layout(&t);
    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);

    // A new server restores the save before running its first command.
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        config.replace("restore-on-start = false", "restore-on-start = true"),
    )
    .unwrap();
    t.run(&["new-session", "-d", "-s", "other"]);
    let restored = layout(&t);
    assert_eq!(restored.lines().count(), 3, "{restored}");
    assert_eq!(restored, saved);
}

#[test]
fn buffers_save_to_and_load_from_files() {
    let t = Tmxr::new("buffers");
    t.run(&["new-session", "-d", "-s", "b"]);
    let file = t.dir.path().join("buffer.txt");
    let path = file.display().to_string();

    t.run(&["set-buffer", "-b", "one", "hello"]);
    t.run(&["save-buffer", "-b", "one", &path]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello");
    t.run(&["save-buffer", "-a", "-b", "one", &path]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "hellohello");
    assert_eq!(t.run(&["save-buffer", "-b", "one", "-"]), "hello");

    std::fs::write(&file, "from a file\n").unwrap();
    t.run(&["load-buffer", "-b", "two", &path]);
    assert_eq!(t.run(&["show-buffer", "-b", "two"]), "from a file\n");
}

#[test]
fn if_shell_picks_a_command_by_status_or_format() {
    let t = Tmxr::new("ifshell");
    t.run(&["new-session", "-d", "-s", "base"]);
    t.run(&["if-shell", "-F", "1", "new -d -s f-yes", "new -d -s f-no"]);
    t.run(&["if-shell", "-F", "0", "new -d -s z-yes", "new -d -s z-no"]);
    t.run(&["if-shell", "exit 0", "new -d -s s-yes", "new -d -s s-no"]);
    t.run(&["if-shell", "exit 1", "new -d -s e-yes", "new -d -s e-no"]);
    // The shell runs in the background; wait for both chosen sessions.
    let ls = t.wait_run(&["ls"], "if-shell sessions", |o| {
        o.contains("s-yes:") && o.contains("e-no:")
    });
    for want in ["f-yes:", "z-no:", "s-yes:", "e-no:"] {
        assert!(ls.contains(want), "{want} missing:\n{ls}");
    }
    for unwanted in ["f-no:", "z-yes:", "s-no:", "e-yes:"] {
        assert!(!ls.contains(unwanted), "{unwanted} present:\n{ls}");
    }
}

#[test]
fn windows_move_and_swap_between_sessions() {
    let t = Tmxr::new("movew");
    let windows = |t: &Tmxr, s: &str| -> Vec<String> {
        t.run(&["list-windows", "-t", s])
            .lines()
            .map(|l| {
                let (idx, rest) = l.split_once(": ").unwrap_or((l, ""));
                let name = rest.split([' ', '*', '-']).next().unwrap_or("");
                format!("{idx}:{name}")
            })
            .collect()
    };
    t.run(&["new-session", "-d", "-s", "a", "-n", "one"]);
    t.run(&["new-window", "-d", "-t", "a", "-n", "two"]);
    t.run(&["new-session", "-d", "-s", "b", "-n", "bee"]);

    // Within a session: a new index.
    t.run(&["move-window", "-d", "-s", "a:1", "-t", "a:5"]);
    assert_eq!(windows(&t, "a"), ["0:one", "5:two"]);
    // To another session: the first free index there.
    t.run(&["move-window", "-d", "-s", "a:5", "-t", "b"]);
    assert_eq!(windows(&t, "a"), ["0:one"]);
    assert_eq!(windows(&t, "b"), ["0:bee", "1:two"]);
    // Swap across sessions.
    t.run(&["swap-window", "-d", "-s", "a:0", "-t", "b:0"]);
    assert_eq!(windows(&t, "a"), ["0:bee"]);
    assert_eq!(windows(&t, "b"), ["0:one", "1:two"]);
    // Moving a session's last window away ends the session.
    t.run(&["move-window", "-d", "-s", "a:0", "-t", "b:7"]);
    let ls = t.run(&["ls"]);
    assert!(!ls.lines().any(|l| l.starts_with("a:")), "{ls}");
    assert_eq!(windows(&t, "b"), ["0:one", "1:two", "7:bee"]);

    // A bare session name as a destination names the session, not the
    // current one (c is newest, so current for a command client).
    t.run(&["new-session", "-d", "-s", "c", "-n", "sea"]);
    t.run(&["new-window", "-d", "-t", "b", "-n", "late"]);
    assert_eq!(windows(&t, "b"), ["0:one", "1:two", "2:late", "7:bee"]);
    assert_eq!(windows(&t, "c"), ["0:sea"]);
}

#[test]
fn panes_join_and_swap_across_windows() {
    let t = Tmxr::new("joinp");
    let pane_id = |t: &Tmxr, target: &str| {
        t.run(&["display-message", "-p", "-t", target, "#{pane_id}"])
            .trim()
            .to_owned()
    };
    let count = |t: &Tmxr, args: &[&str]| t.run(args).lines().count();
    t.run(&["new-session", "-d", "-s", "p", "-n", "w1"]);
    t.run(&["split-window", "-d", "-t", "p:w1"]);
    t.run(&["new-window", "-d", "-t", "p", "-n", "w2"]);
    let joined = pane_id(&t, "p:w2");
    assert!(joined.starts_with('%'), "{joined:?}");

    // Joining a window's only pane elsewhere closes that window.
    t.run(&["join-pane", "-d", "-h", "-s", "p:w2", "-t", "p:w1.0"]);
    assert_eq!(count(&t, &["list-windows", "-t", "p"]), 1);
    assert_eq!(count(&t, &["list-panes", "-t", "p:w1"]), 3);
    assert!(
        t.run(&["list-panes", "-t", "p:w1"]).contains(&joined),
        "{joined} not in w1"
    );

    // Swap a pane with one in another window.
    t.run(&["new-window", "-d", "-t", "p", "-n", "w3"]);
    let (lone, other) = (pane_id(&t, "p:w3"), pane_id(&t, "p:w1.1"));
    assert!(lone.starts_with('%') && other.starts_with('%') && lone != other);
    t.run(&["swap-pane", "-d", "-s", "p:w3", "-t", "p:w1.1"]);
    assert_eq!(pane_id(&t, "p:w3"), other);
    assert_eq!(pane_id(&t, "p:w1.1"), lone);
}

#[test]
fn respawn_pane_restarts_the_program_in_place() {
    let t = Tmxr::new("respawn");
    t.run(&["new-session", "-d", "-s", "r"]);
    t.run(&["split-window", "-d", "-t", "r"]);
    let id = t.run(&["display-message", "-p", "-t", "r.0", "#{pane_id}"]);
    assert!(id.starts_with('%'), "{id:?}");
    t.run(&["send-keys", "-t", "r.0", "echo before-respawn", "Enter"]);
    t.wait_run(&["capture-pane", "-p", "-t", "r.0"], "marker", |o| {
        o.matches("before-respawn").count() >= 2
    });

    // Without -k the running program is left alone.
    t.run(&["respawn-pane", "-t", "r.0"]);
    assert!(
        t.run(&["capture-pane", "-p", "-t", "r.0"])
            .contains("before-respawn")
    );

    // With -k a fresh program starts in the same pane: same id, clean screen.
    t.run(&["respawn-pane", "-k", "-t", "r.0"]);
    t.wait_run(&["capture-pane", "-p", "-t", "r.0"], "fresh screen", |o| {
        !o.contains("before-respawn")
    });
    // The old program's exit arrives after the respawn and must not close
    // the new one.
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "r.0", "#{pane_id}"]),
        id
    );
    assert_eq!(t.run(&["list-panes", "-t", "r"]).lines().count(), 2);
}

/// argv for a long-running program installed under `name` in the test's
/// directory, so the navigator sees a foreground process with that name.
fn fixture_program(t: &Tmxr, name: &str) -> Vec<String> {
    #[cfg(unix)]
    {
        let path = t.dir.path().join(name);
        std::fs::copy("/bin/sleep", &path).unwrap();
        vec![path.display().to_string(), "60".into()]
    }
    #[cfg(windows)]
    {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let path = t.dir.path().join(format!("{name}.exe"));
        std::fs::copy(format!(r"{root}\System32\PING.EXE"), &path).unwrap();
        vec![
            path.display().to_string(),
            "-n".into(),
            "60".into(),
            "127.0.0.1".into(),
        ]
    }
}

#[test]
fn navigator_passes_keys_to_hjkl_and_moves_otherwise() {
    for (name, passes_through) in [("hjkl", true), ("plainprog", false)] {
        let t = Tmxr::new(&format!("nav-{name}"));
        let argv = fixture_program(&t, name);
        let mut new = vec!["new-session", "-d", "-s", "nav"];
        new.extend(argv.iter().map(String::as_str));
        t.run(&new);
        // A shell to the right; focus stays on the fixture on the left.
        t.run(&["split-window", "-h", "-d", "-t", "nav.0"]);
        t.wait_run(
            &[
                "display-message",
                "-p",
                "-t",
                "nav.0",
                "#{pane_current_command}",
            ],
            "fixture in front",
            |o| o.trim() == name,
        );
        let s = t.attach(&["attach", "-t", "nav"]);
        s.wait_for("status line", |text| text.contains("nav"));
        let active = |t: &Tmxr| t.run(&["display-message", "-p", "-t", "nav", "#{pane_index}"]);
        assert_eq!(active(&t).trim(), "0");

        s.send(b"\x0c");
        if passes_through {
            // C-l went to hjkl; nothing should move.
            std::thread::sleep(Duration::from_secs(1));
            assert_eq!(active(&t).trim(), "0", "{name}: focus moved");
        } else {
            t.wait_run(
                &["display-message", "-p", "-t", "nav", "#{pane_index}"],
                "focus moved right",
                |o| o.trim() == "1",
            );
        }
    }
}

#[test]
fn copy_mode_selection_goes_to_a_buffer_and_pastes() {
    let t = Tmxr::new("copy");
    let s = t.attach(&["new", "-s", "copy"]);
    s.wait_for("status line", |text| text.contains("copy"));
    s.send(b"echo copyme-1234\r");
    s.wait_for("echoed marker", |text| {
        text.matches("copyme-1234").count() >= 2
    });
    let in_mode = |t: &Tmxr| t.run(&["display-message", "-p", "-t", "copy", "#{pane_in_mode}"]);

    // prefix [ enters copy mode; ? searches up; v E y copies one WORD.
    s.send(PREFIX);
    s.send(b"[");
    t.wait_run(
        &["display-message", "-p", "-t", "copy", "#{pane_in_mode}"],
        "copy mode",
        |o| o.trim() == "1",
    );
    s.send(b"?");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    s.send(b"copyme\r");
    s.send(b"vE");
    s.send(b"y");
    t.wait_run(&["show-buffer"], "copied text", |o| o == "copyme-1234");
    assert_eq!(in_mode(&t).trim(), "0", "y leaves copy mode");

    // prefix ] pastes it at the shell prompt.
    s.send(PREFIX);
    s.send(b"]");
    s.wait_for("pasted text", |text| {
        text.matches("copyme-1234").count() >= 3
    });
}

/// The Tokyo Night palette as `Debug` text of a vt100 colour.
fn rgb(hex: u32) -> String {
    format!("Rgb({}, {}, {})", hex >> 16, (hex >> 8) & 0xff, hex & 0xff)
}

#[test]
fn status_line_has_the_catppuccin_layout_in_tokyo_night() {
    const MANTLE: u32 = 0x16_16_1e;
    const SURFACE0: u32 = 0x29_2e_42;
    const SURFACE1: u32 = 0x3b_42_61;
    const OVERLAY2: u32 = 0x73_7a_a2;
    const FG: u32 = 0xc0_ca_f5;
    const MAUVE: u32 = 0xbb_9a_f7;
    const GREEN: u32 = 0x9e_ce_6a;
    const RED: u32 = 0xf7_76_8e;
    const SESSION_ICON: &str = "\u{e795}";
    const HOST_ICON: &str = "\u{f048b}";

    let t = Tmxr::new("status");
    let s = t.attach(&["new", "-s", "look", "-n", "first"]);
    s.wait_for("status line", |text| text.contains("first"));
    t.run(&["new-window", "-t", "look", "-n", "second"]);
    s.wait_for("second window", |text| text.contains("second"));
    let cells = s.status_cells();
    let text: String = cells.iter().map(|c| c.0.as_str()).collect();
    let at = |needle: &str| -> usize {
        let byte = text
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not in {text:?}"));
        text[..byte].chars().count()
    };
    let bg = |col: usize| cells[col].2.clone();
    let fg = |col: usize| cells[col].1.clone();

    // Windows, left: " N " number block, then " name" text block.
    let first = at("first");
    assert_eq!(cells[first - 3].0, "0", "{text:?}");
    assert_eq!(bg(first - 3), rgb(OVERLAY2), "window number block");
    assert_eq!(fg(first - 3), rgb(MANTLE), "number text is crust");
    assert_eq!(bg(first), rgb(SURFACE0), "window text block");
    assert_eq!(fg(first), rgb(FG));
    let second = at("second");
    assert_eq!(cells[second - 3].0, "1", "{text:?}");
    assert_eq!(bg(second - 3), rgb(MAUVE), "current window number block");
    assert_eq!(bg(second), rgb(SURFACE1), "current window text block");
    // The gap between the window list and the modules is the status bg.
    assert_eq!(bg(second + 12), rgb(MANTLE), "status background");

    // Session module, right: █ separator, icon block, " look" text block.
    let icon = at(SESSION_ICON);
    assert_eq!(cells[icon - 1].0, "\u{2588}");
    assert_eq!(
        fg(icon - 1),
        rgb(GREEN),
        "separator takes the module colour"
    );
    assert_eq!(bg(icon), rgb(GREEN), "session icon block");
    let name = at(" look");
    assert_eq!(bg(name + 1), rgb(SURFACE0), "session text block");
    // Host module after it, mauve.
    assert_eq!(bg(at(HOST_ICON)), rgb(MAUVE), "host icon block");

    // The session block turns red while the prefix is pending.
    s.send(PREFIX);
    s.wait_for("prefix pending", |_| {
        let cells = s.status_cells();
        cells.iter().any(|c| c.0 == SESSION_ICON && c.2 == rgb(RED))
    });
}

#[test]
fn mouse_option_turns_the_client_terminal_mouse_on_and_off() {
    let t = Tmxr::new("mouse");
    let s = t.attach(&["new", "-s", "m"]);
    s.wait_for("status line", |text| text.contains('m'));
    let mode = |s: &Screen| format!("{:?}", s.emu.lock().unwrap().screen().mouse_protocol_mode());
    let wait_mode = |s: &Screen, on: bool| {
        let deadline = Instant::now() + TIMEOUT;
        while (mode(s) != "None") != on {
            assert!(
                Instant::now() < deadline,
                "mouse {on} never applied: {}",
                mode(s)
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    // `mouse on` (the default) captures the mouse once attached.
    wait_mode(&s, true);
    t.run(&["set-option", "-g", "mouse", "off"]);
    wait_mode(&s, false);
    t.run(&["set-option", "-g", "mouse", "on"]);
    wait_mode(&s, true);
}

#[test]
fn long_status_right_is_cut_before_the_window_list() {
    // A session name this long makes the right side (session and host
    // modules) wider than status-right-length, like a long hostname does.
    let name = "a-session-name-long-enough-to-crowd-the-status-line";
    let t = Tmxr::new("statuslen");
    let s = t.attach(&["new", "-s", name, "-n", "first"]);
    s.wait_for("status line", |text| text.contains("first"));
    t.run(&["new-window", "-t", name, "-n", "second"]);
    s.wait_for("both windows in full", |_| {
        let text: String = s.status_cells().iter().map(|c| c.0.clone()).collect();
        text.contains("first") && text.contains("second")
    });
    let cells = s.status_cells();
    let right_start = cells.iter().position(|c| c.0 == "\u{2588}").unwrap();
    assert_eq!(
        usize::from(COLS) - right_start,
        40,
        "right side is status-right-length"
    );
}

#[test]
fn display_panes_selects_by_number_until_it_times_out() {
    let t = Tmxr::new("displayp");
    let s = t.attach(&["new", "-s", "dp"]);
    s.wait_for("status line", |text| text.contains("dp"));
    t.run(&["split-window", "-h", "-t", "dp"]);
    let active = |t: &Tmxr| t.run(&["display-message", "-p", "-t", "dp", "#{pane_index}"]);
    t.wait_run(
        &["display-message", "-p", "-t", "dp", "#{pane_index}"],
        "new pane",
        |o| o.trim() == "1",
    );

    // prefix q numbers the panes; 0 picks the left one.
    s.send(PREFIX);
    s.send(b"q");
    std::thread::sleep(Duration::from_millis(200));
    s.send(b"0");
    t.wait_run(
        &["display-message", "-p", "-t", "dp", "#{pane_index}"],
        "pane 0",
        |o| o.trim() == "0",
    );

    // After display-panes-time the numbers are gone: a digit is just typed.
    s.send(PREFIX);
    s.send(b"q");
    std::thread::sleep(Duration::from_millis(1600));
    s.send(b"1");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(active(&t).trim(), "0");
}

#[test]
fn rotate_window_moves_panes_up_by_default_like_tmux() {
    let t = Tmxr::new("rotate");
    t.run(&["new-session", "-d", "-s", "r"]);
    t.run(&["split-window", "-d", "-t", "r"]);
    t.run(&["split-window", "-d", "-t", "r"]);
    let order = |t: &Tmxr| -> Vec<String> {
        t.run(&["list-panes", "-t", "r"])
            .lines()
            .filter_map(|l| l.split_whitespace().find(|w| w.starts_with('%')))
            .map(str::to_owned)
            .collect()
    };
    let before = order(&t);
    assert_eq!(before.len(), 3, "{before:?}");
    t.run(&["rotate-window", "-t", "r"]);
    assert_eq!(order(&t), [before[1].as_str(), &before[2], &before[0]]);
    t.run(&["rotate-window", "-D", "-t", "r"]);
    assert_eq!(order(&t), before);
}

#[test]
fn passthrough_is_forwarded_where_the_platform_allows() {
    let t = Tmxr::new("passthrough");
    // A program that wraps an OSC in tmux's passthrough DCS (every ESC inside
    // doubled), prints a marker after it, then waits.
    #[cfg(unix)]
    let program = [
        "/bin/sh",
        "-c",
        r"printf '\033Ptmux;\033\033]1337;tmxr-pt-marker\007\033\\after-dcs'; sleep 30",
    ];
    #[cfg(windows)]
    let program = [
        "powershell.exe",
        "-NoProfile",
        "-Command",
        r"$e=[char]27; [Console]::Out.Write($e+'Ptmux;'+$e+$e+']1337;tmxr-pt-marker'+[char]7+$e+'\'+'after-dcs'); Start-Sleep 30",
    ];
    let mut args = vec!["new", "-s", "pt"];
    args.extend(program);
    let s = t.attach(&args);
    // On every platform, output after the DCS still reaches the pane.
    s.wait_for("text after the DCS", |text| text.contains("after-dcs"));
    assert!(!s.text().contains("tmux;"), "{}", s.text());

    let forwarded = s.raw_text().contains("\x1b]1337;tmxr-pt-marker\x07");
    if cfg!(windows) {
        // ConPTY drops the DCS terminator, so tmxr leaves passthrough alone
        // there and says so.
        assert!(!forwarded);
        let messages = t.run(&["show-messages"]);
        assert!(messages.contains("allow-passthrough"), "{messages}");
    } else {
        assert!(forwarded, "payload not forwarded: {:?}", s.raw_text());
    }
}

#[test]
fn next_window_with_alert_skips_quiet_windows() {
    let t = Tmxr::new("alert");
    #[cfg(unix)]
    let ring = ["/bin/sh", "-c", r"printf '\007'; sleep 30"];
    #[cfg(windows)]
    let ring = [
        "powershell.exe",
        "-NoProfile",
        "-Command",
        "[Console]::Out.Write([char]7); Start-Sleep 30",
    ];
    t.run(&["new-session", "-d", "-s", "al", "-n", "zero"]);
    t.run(&["new-window", "-d", "-t", "al", "-n", "quiet"]);
    let mut bell = vec!["new-window", "-d", "-t", "al", "-n", "ringing"];
    bell.extend(ring);
    t.run(&bell);
    t.wait_run(&["list-windows", "-t", "al"], "bell flag", |o| {
        o.lines().any(|l| l.contains("ringing!"))
    });
    let current = |t: &Tmxr| t.run(&["display-message", "-p", "-t", "al", "#{window_name}"]);
    t.run(&["next-window", "-a", "-t", "al"]);
    assert_eq!(current(&t).trim(), "ringing");
    // Selecting the window cleared its alert; there is no other.
    let out = t.output(&["next-window", "-a", "-t", "al"]);
    assert!(!out.status.success());
    assert_eq!(current(&t).trim(), "ringing");
}

#[test]
fn select_layout_spread_evens_out_a_row_of_panes() {
    let t = Tmxr::new("spread");
    t.run(&["new-session", "-d", "-s", "sp", "-x", "100", "-y", "30"]);
    t.run(&["split-window", "-h", "-t", "sp"]);
    t.run(&["split-window", "-h", "-t", "sp"]);
    let widths = |t: &Tmxr| -> Vec<u16> {
        t.run(&["list-panes", "-t", "sp"])
            .lines()
            .filter_map(|l| l.split(['[', 'x']).nth(1)?.parse().ok())
            .collect()
    };
    let before = widths(&t);
    assert_eq!(before.len(), 3, "{before:?}");
    assert!(before[0] > before[2] + 10, "uneven first: {before:?}");
    t.run(&["select-layout", "-E", "-t", "sp"]);
    let after = widths(&t);
    let (min, max) = (after.iter().min().unwrap(), after.iter().max().unwrap());
    assert!(max - min <= 1, "{after:?}");
}

#[test]
fn choose_buffer_pastes_and_find_window_switches() {
    let t = Tmxr::new("chooseb");
    let s = t.attach(&["new", "-s", "cb", "-n", "logs"]);
    s.wait_for("status line", |text| text.contains("logs"));
    t.run(&["new-window", "-d", "-t", "cb", "-n", "editor"]);
    t.run(&["set-buffer", "-b", "alpha", "first-buffer-text"]);
    t.run(&["set-buffer", "-b", "beta", "second-buffer-text"]);

    // prefix = lists the buffers; typing filters, Enter pastes into the pane.
    s.send(PREFIX);
    s.send(b"=");
    s.wait_for("buffer picker", |text| text.contains("buffers 2/2"));
    s.send(b"alpha");
    s.wait_for("filtered", |text| text.contains("buffers 1/2"));
    s.send(b"\r");
    s.wait_for("pasted buffer", |text| {
        !text.contains("buffers 1/2") && text.contains("first-buffer-text")
    });
    assert!(!s.text().contains("second-buffer-text"));

    // prefix f asks for text and opens the window list filtered by it.
    s.send(PREFIX);
    s.send(b"f");
    s.wait_for("find prompt", |text| text.contains("(find-window)"));
    s.send(b"editor\r");
    s.wait_for("filtered windows", |text| text.contains("windows 1/2"));
    s.send(b"\r");
    t.wait_run(
        &["display-message", "-p", "-t", "cb", "#{window_name}"],
        "editor selected",
        |o| o.trim() == "editor",
    );
}

#[test]
fn marked_pane_is_the_default_source_of_join_pane() {
    let t = Tmxr::new("mark");
    t.run(&["new-session", "-d", "-s", "mk", "-n", "one"]);
    t.run(&["split-window", "-d", "-t", "mk:one"]);
    t.run(&["new-window", "-d", "-t", "mk", "-n", "two"]);
    let marked = t.run(&["display-message", "-p", "-t", "mk:one.1", "#{pane_id}"]);
    let marked = marked.trim();
    assert!(marked.starts_with('%'), "{marked:?}");

    // Mark a pane: its window shows the M flag.
    t.run(&["select-pane", "-m", "-t", "mk:one.1"]);
    let windows = t.run(&["list-windows", "-t", "mk"]);
    assert!(
        windows.lines().any(|l| l.starts_with("0: one*M (")),
        "{windows}"
    );
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "mk:one.1", "#{pane_marked}"])
            .trim(),
        "1"
    );

    // join-pane without -s takes the marked pane.
    let out = t.output(&["join-pane", "-d", "-t", "mk:two"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(t.run(&["list-panes", "-t", "mk:two"]).contains(marked));
    assert_eq!(t.run(&["list-panes", "-t", "mk:one"]).lines().count(), 1);

    // -M clears it; then join-pane has no source.
    t.run(&["select-pane", "-M"]);
    assert!(!t.run(&["list-windows", "-t", "mk"]).contains('M'));
    assert!(!t.output(&["join-pane", "-t", "mk:one"]).status.success());
}

#[test]
fn session_picker_kills_renames_and_creates_sessions() {
    let t = Tmxr::new("pickact");
    t.run(&["new-session", "-d", "-s", "first"]);
    let s = t.attach(&["new", "-s", "home"]);
    s.wait_for("status line", |text| text.contains("home"));
    let names = |t: &Tmxr| -> Vec<String> {
        t.run(&["ls"])
            .lines()
            .filter_map(|l| l.split(':').next())
            .map(str::to_owned)
            .collect()
    };
    let open_picker = |s: &Screen| {
        s.send(PREFIX);
        s.send(b"s");
        s.wait_for("picker", |text| text.contains("sessions "));
    };

    // C-r renames the highlighted session (the previous one, "first").
    open_picker(&s);
    s.send(b"\x12");
    s.wait_for("rename prompt", |text| text.contains("(rename-session)"));
    s.send(b"\x15renamed\r");
    t.wait_run(&["ls"], "renamed session", |o| o.contains("renamed:"));

    // C-x asks, then kills it.
    open_picker(&s);
    s.send(b"\x18");
    s.wait_for("confirm", |text| text.contains("kill-session renamed?"));
    s.send(b"y");
    t.wait_run(&["ls"], "session killed", |o| !o.contains("renamed:"));

    // Enter on a name nothing matches creates it and switches to it.
    open_picker(&s);
    s.send(b"brand-new\r");
    t.wait_run(&["ls"], "new session attached", |o| {
        o.lines()
            .any(|l| l.starts_with("brand-new:") && l.contains("(attached)"))
    });
    let mut all = names(&t);
    all.sort();
    assert_eq!(all, ["brand-new", "home"]);
}

#[test]
fn copy_mode_counts_and_jumps_by_keyboard() {
    let t = Tmxr::new("jump");
    let s = t.attach(&["new", "-s", "jp"]);
    s.wait_for("status line", |text| text.contains("jp"));
    s.send(b"echo jumpme-a-b-c\r");
    s.wait_for("echoed", |text| text.matches("jumpme-a-b-c").count() >= 2);
    s.send(PREFIX);
    s.send(b"[");
    t.wait_run(
        &["display-message", "-p", "-t", "jp", "#{pane_in_mode}"],
        "copy mode",
        |o| o.trim() == "1",
    );
    s.send(b"?");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    s.send(b"jumpme\r");
    // 2f- lands on the second dash; v E y copies from there to the WORD end.
    s.send(b"2f-");
    s.send(b"vEy");
    t.wait_run(&["show-buffer"], "copied text", |o| o == "-b-c");
}

#[test]
fn copy_mode_highlights_search_matches() {
    const YELLOW: u32 = 0xe0_af_68;
    const MAUVE: u32 = 0xbb_9a_f7;
    let t = Tmxr::new("matches");
    let s = t.attach(&["new", "-s", "hl"]);
    s.wait_for("status line", |text| text.contains("hl"));
    s.send(b"echo zq-hit zq-hit\r");
    s.wait_for("echoed", |text| text.matches("zq-hit").count() >= 4);
    s.send(PREFIX);
    s.send(b"[");
    s.send(b"?");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    s.send(b"zq-hit\r");
    // Every match on screen is highlighted, the one under the cursor apart.
    let backgrounds = |s: &Screen| -> Vec<String> {
        (0..ROWS - 1)
            .flat_map(|row| s.row_cells(row))
            .filter(|c| c.0 == "z")
            .map(|c| c.2)
            .collect()
    };
    s.wait_for("highlighted", |_| {
        let bgs = backgrounds(&s);
        bgs.len() >= 4 && bgs.iter().filter(|b| **b == rgb(MAUVE)).count() == 1
    });
    let bgs = backgrounds(&s);
    assert!(
        bgs.iter().all(|b| *b == rgb(YELLOW) || *b == rgb(MAUVE)),
        "{bgs:?}"
    );
}

#[test]
fn resurrect_skips_unchanged_saves_and_reports_missing_dirs() {
    let t = Tmxr::new("resave");
    let saves = |t: &Tmxr| -> usize {
        let root = t.dir.path().join("data").join("tmxr").join("resurrect");
        std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .flat_map(|server| {
                std::fs::read_dir(server.path())
                    .into_iter()
                    .flatten()
                    .flatten()
            })
            .filter(|f| f.file_name().to_string_lossy().ends_with(".json"))
            .count()
    };
    let gone = t.dir.path().join("soon-gone");
    std::fs::create_dir(&gone).unwrap();
    let gone_str = gone.display().to_string();
    t.run(&["new-session", "-d", "-s", "keep", "-c", &gone_str]);
    t.run(&["resurrect-save"]);
    assert_eq!(saves(&t), 1);
    // Nothing changed: the save on exit writes nothing new.
    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);
    assert_eq!(saves(&t), 1);

    // The session's directory disappears; restoring says so.
    std::fs::remove_dir(&gone).unwrap();
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        config.replace("restore-on-start = false", "restore-on-start = true"),
    )
    .unwrap();
    t.run(&["new-session", "-d", "-s", "other"]);
    let messages = t.run(&["show-messages"]);
    assert!(messages.contains("missing, started in $HOME"), "{messages}");
    assert!(messages.contains("soon-gone"), "{messages}");
}

#[test]
fn session_picker_previews_the_highlighted_session() {
    let t = Tmxr::new("preview");
    t.run(&["new-session", "-d", "-s", "alpha"]);
    t.run(&["send-keys", "-t", "alpha", "echo preview-marker", "Enter"]);
    t.wait_run(
        &["capture-pane", "-p", "-t", "alpha"],
        "marker in alpha",
        |o| o.matches("preview-marker").count() >= 2,
    );
    let s = t.attach(&["new", "-s", "bravo"]);
    s.wait_for("status line", |text| text.contains("bravo"));
    assert!(!s.text().contains("preview-marker"));

    // The picker opens on alpha: its pane shows under the list.
    s.send(PREFIX);
    s.send(b"s");
    s.wait_for("alpha previewed", |text| {
        text.contains("sessions 2/2") && text.contains("preview-marker")
    });
    // Moving to bravo previews bravo, which never printed the marker. Wait
    // for normal mode first: ESC and k sent together read as Alt-k on Unix.
    s.send(b"\x1b");
    s.wait_for("normal mode", |text| text.contains("[normal]"));
    s.send(b"k");
    s.wait_for("bravo previewed", |text| {
        text.contains("sessions 2/2") && !text.contains("preview-marker")
    });
}

#[test]
fn remain_on_exit_keeps_a_dead_pane_until_respawned() {
    let t = Tmxr::new("remain");
    #[cfg(unix)]
    let (exits, lasts): (&[&str], &[&str]) = (&["/bin/sh", "-c", "exit 3"], &["/bin/sleep", "30"]);
    #[cfg(windows)]
    let (exits, lasts): (&[&str], &[&str]) = (
        &["cmd.exe", "/d", "/c", "exit 3"],
        &["ping.exe", "-n", "30", "127.0.0.1"],
    );
    t.run(&["new-session", "-d", "-s", "rm"]);
    t.run(&["set-option", "-g", "remain-on-exit", "on"]);
    let mut split = vec!["split-window", "-d", "-t", "rm"];
    split.extend(exits);
    t.run(&split);
    let dead = |t: &Tmxr| {
        t.run(&[
            "display-message",
            "-p",
            "-t",
            "rm.1",
            "#{pane_dead}:#{pane_dead_status}",
        ])
        .trim()
        .to_owned()
    };
    t.wait_run(
        &["display-message", "-p", "-t", "rm.1", "#{pane_dead}"],
        "pane dead",
        |o| o.trim() == "1",
    );
    assert_eq!(dead(&t), "1:3");
    assert_eq!(t.run(&["list-panes", "-t", "rm"]).lines().count(), 2);

    // A dead pane respawns without -k.
    let mut respawn = vec!["respawn-pane", "-t", "rm.1"];
    respawn.extend(lasts);
    let out = t.output(&respawn);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(dead(&t), "0:");
}

#[test]
fn describe_key_shows_a_binds_note() {
    let t = Tmxr::new("describe");
    let s = t.attach(&["new", "-s", "dk"]);
    s.wait_for("status line", |text| text.contains("dk"));
    let describe = |s: &Screen, key: &[u8]| {
        s.send(PREFIX);
        s.send(b"/");
        s.wait_for("key prompt", |text| text.contains("key "));
        s.send(key);
    };
    describe(&s, b"z");
    s.wait_for("zoom note", |text| text.contains("Zoom pane"));
    // A key press clears the message, so the next prompt shows at once
    // rather than after display-time.
    let start = Instant::now();
    describe(&s, b"\x1b[15~");
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "prompt hidden by the message for {:?}",
        start.elapsed()
    );
    s.wait_for("unbound key", |text| text.contains("F5 is not bound"));
}

#[test]
fn clock_mode_shows_big_digits_until_a_key() {
    const BLUE: u32 = 0x7a_a2_f7;
    let t = Tmxr::new("clock");
    let s = t.attach(&["new", "-s", "ck"]);
    s.wait_for("status line", |text| text.contains("ck"));
    let painted = |s: &Screen| -> usize {
        (0..ROWS - 1)
            .flat_map(|row| s.row_cells(row))
            .filter(|c| c.2 == rgb(BLUE))
            .count()
    };
    assert_eq!(painted(&s), 0);
    s.send(PREFIX);
    s.send(b"t");
    // HH:MM in 5x5 blocks: well over 20 painted cells for any time.
    s.wait_for("clock", |_| painted(&s) > 20);
    s.send(b"q");
    s.wait_for("clock gone", |_| painted(&s) == 0);
}

#[test]
fn clients_are_listed_and_detached_by_id() {
    let t = Tmxr::new("clients");
    t.run(&["new-session", "-d", "-s", "cl"]);
    let first = t.attach(&["attach", "-t", "cl"]);
    first.wait_for("first attached", |text| text.contains("cl"));
    let second = t.attach(&["attach", "-t", "cl"]);
    second.wait_for("second attached", |text| text.contains("cl"));
    let ids = |t: &Tmxr| -> Vec<String> {
        t.run(&["list-clients"])
            .lines()
            .filter_map(|l| l.split(':').next().map(str::to_owned))
            .collect()
    };
    let both = t.wait_run(&["list-clients"], "two clients", |o| o.lines().count() == 2);
    assert!(both.lines().all(|l| l.contains(": cl [")), "{both}");

    // detach-client -t detaches exactly that client.
    let before = ids(&t);
    t.run(&["detach-client", "-t", &before[0]]);
    t.wait_run(&["list-clients"], "one client left", |o| {
        o.lines().count() == 1
    });
    assert_eq!(ids(&t), [before[1].clone()]);
    assert!(!t.output(&["detach-client", "-t", "999"]).status.success());

    // prefix D lists the clients; Enter detaches the chosen one.
    // Ids grow, so the first client attached has the smaller one.
    let num = |id: &String| id.parse::<u32>().unwrap();
    let (detached, remaining) = if num(&before[0]) < num(&before[1]) {
        (first, second)
    } else {
        (second, first)
    };
    drop(detached);
    remaining.send(PREFIX);
    remaining.send(b"D");
    remaining.wait_for("client picker", |text| text.contains("clients 1/1"));
    remaining.send(b"\r");
    t.wait_run(&["list-clients"], "no clients", str::is_empty);
}
