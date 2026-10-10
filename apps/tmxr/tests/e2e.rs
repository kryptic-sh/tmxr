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

    /// Run a command client against the server at `socket` (`-S`) in place
    /// of the test's label.
    fn output_at(&self, socket: &str, rest: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_tmxr"))
            .args(["-S", socket, "-f"])
            .arg(&self.config)
            .args(rest)
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

    /// Wait until `target`'s shell shows its prompt: keys sent to a pane on
    /// Windows before its shell first reads can leave the Enter held in
    /// ConPTY until more input arrives.
    fn wait_prompt(&self, target: &str) {
        self.wait_run(&["capture-pane", "-p", "-t", target], "the prompt", |o| {
            o.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .is_some_and(|l| l.trim_end().ends_with(['>', '$', '#']))
        });
    }

    /// Wait until `target`'s shell has started: its prompt is showing, and
    /// on Windows ConPTY has reported cmd's title, which arrives on its own
    /// time and would otherwise land on what the test does next.
    fn wait_settled(&self, target: &str) {
        self.wait_prompt(target);
        if cfg!(windows) {
            self.wait_run(
                &["display-message", "-p", "-t", target, "#{pane_title}"],
                "the shell's title",
                |o| o.contains("cmd"),
            );
        }
    }

    /// The newest resurrect save's text.
    fn newest_save(&self) -> String {
        let dir = self.dir.path().join("data").join("tmxr").join("resurrect");
        let newest = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .flat_map(|server| std::fs::read_dir(server.path()).unwrap().flatten())
            .map(|f| f.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .max()
            .expect("a save");
        std::fs::read_to_string(newest).unwrap()
    }

    /// Start an attaching client in a pseudo-terminal.
    fn attach(&self, rest: &[&str]) -> Screen {
        self.spawn(env!("CARGO_BIN_EXE_tmxr"), &self.args(rest))
    }

    /// [`Tmxr::attach`] with variables set (`Some`) or removed (`None`) in
    /// the client's environment, over what the test runner has.
    fn attach_env(&self, rest: &[&str], vars: &[(&str, Option<&str>)]) -> Screen {
        self.spawn_env(env!("CARGO_BIN_EXE_tmxr"), &self.args(rest), vars)
    }

    /// Start `program` in a pseudo-terminal with the test's environment.
    fn spawn(&self, program: &str, args: &[String]) -> Screen {
        self.spawn_env(program, args, &[])
    }

    /// [`Tmxr::spawn`] with variables set or removed, as [`Tmxr::attach_env`].
    fn spawn_env(&self, program: &str, args: &[String], vars: &[(&str, Option<&str>)]) -> Screen {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        for (k, v) in self.env() {
            cmd.env(k, v);
        }
        for (k, v) in vars {
            match v {
                Some(v) => cmd.env(k, v),
                None => cmd.env_remove(k),
            }
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
            master: pair.master,
            writer,
            emu,
            raw,
            child,
        }
    }
}

/// The first `args` list in a resurrect save (the test's one pane with any).
fn saved_args(save: &str) -> Vec<String> {
    let Some((_, rest)) = save.split_once("\"args\": [") else {
        return Vec::new();
    };
    let list = rest.split(']').next().unwrap();
    list.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
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
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    emu: Arc<Mutex<Emulator>>,
    raw: Arc<Mutex<Vec<u8>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl Screen {
    /// Resize the terminal, as a user resizing the window.
    fn resize(&self, rows: u16, cols: u16) {
        self.emu.lock().unwrap().resize(rows, cols);
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
    }

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

    /// Wait until the client has (or has not) asked its terminal for the
    /// mouse, so mouse input reaches it.
    fn wait_mouse(&self, on: bool) {
        let mode = || {
            format!(
                "{:?}",
                self.emu.lock().unwrap().screen().mouse_protocol_mode()
            )
        };
        let deadline = Instant::now() + TIMEOUT;
        while (mode() != "None") != on {
            assert!(
                Instant::now() < deadline,
                "mouse {on} never applied: {}",
                mode()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Send an SGR mouse event at a 0-based screen cell: `code` 0/1/2 a
    /// button, +32 a drag, 64/65 the wheel; `release` ends a click.
    fn mouse(&self, code: u8, col: u16, row: u16, release: bool) {
        let end = if release { 'm' } else { 'M' };
        self.send(format!("\x1b[<{code};{};{}{end}", col + 1, row + 1).as_bytes());
    }

    fn click(&self, code: u8, col: u16, row: u16) {
        self.mouse(code, col, row, false);
        self.mouse(code, col, row, true);
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

    // Down moves a row, as in hjkl's pickers: charlie (current), alpha
    // (where the picker opens), then bravo.
    s.send(PREFIX);
    s.send(b"s");
    s.wait_for("picker", |text| text.contains("sessions 3/3"));
    s.send(b"\x1b[B");
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
    // Pane 2 active, pane 0 the last pane.
    t.run(&["select-pane", "-t", "main.0"]);
    t.run(&["select-pane", "-t", "main.2"]);
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
    // The last pane came back too: select-pane -l goes to pane 0.
    t.run(&["select-pane", "-l", "-t", "main"]);
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "main", "#{pane_index}"])
            .trim(),
        "0"
    );
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
    // Each if-shell finished before its command returned.
    let ls = t.run(&["ls"]);
    for want in ["f-yes:", "z-no:", "s-yes:", "e-no:"] {
        assert!(ls.contains(want), "{want} missing:\n{ls}");
    }
    for unwanted in ["f-no:", "z-yes:", "s-no:", "e-yes:"] {
        assert!(!ls.contains(unwanted), "{unwanted} present:\n{ls}");
    }
    // The rest of the list waits for the choice, and a script gets its
    // output, as from a run-shell inside a branch.
    let lines = |out: &str| {
        out.lines()
            .map(str::trim)
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let out = t.run(&[
        "if-shell",
        "exit 1",
        "display -p yes",
        "display -p no",
        ";",
        "display",
        "-p",
        "after",
    ]);
    assert_eq!(lines(&out), ["no", "after"], "{out:?}");
    let out = t.run(&[
        "if-shell",
        "-F",
        "1",
        "run-shell 'echo inner'",
        ";",
        "display",
        "-p",
        "after",
    ]);
    assert_eq!(lines(&out), ["inner", "after"], "{out:?}");
    // A run-shell in the branch a shell test chose holds the rest too.
    let out = t.run(&[
        "if-shell",
        "exit 0",
        "run-shell 'echo deep'",
        ";",
        "display",
        "-p",
        "after",
    ]);
    assert_eq!(lines(&out), ["deep", "after"], "{out:?}");
    // -b: in the background.
    t.run(&["if-shell", "-b", "exit 0", "set-option -g @bg yes"]);
    t.wait_run(
        &["display-message", "-p", "#{@bg}"],
        "the background choice",
        |o| o.trim() == "yes",
    );
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
    t.wait_prompt("r.0");
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
    // `mouse on` (the default) captures the mouse once attached.
    s.wait_mouse(true);
    t.run(&["set-option", "-g", "mouse", "off"]);
    s.wait_mouse(false);
    t.run(&["set-option", "-g", "mouse", "on"]);
    s.wait_mouse(true);
}

#[test]
fn default_mouse_binds_focus_scroll_and_select_windows() {
    let t = Tmxr::new("mousedef");
    let s = t.attach(&["new", "-s", "md", "-n", "first"]);
    s.wait_for("status line", |text| text.contains("first"));
    s.wait_mouse(true);
    let format_is = |f: &str, want: &str, what: &str| {
        t.wait_run(&["display-message", "-p", "-t", "md", f], what, |o| {
            o.trim() == want
        });
    };
    t.run(&["split-window", "-h", "-t", "md"]);
    format_is("#{pane_index}", "1", "the new pane focused");
    // MouseDown1Pane: a click in the left pane focuses it.
    s.click(0, 5, 3);
    format_is("#{pane_index}", "0", "the clicked pane focused");
    // WheelUpPane enters copy mode; WheelDownPane back at the bottom leaves
    // it (copy-mode -e).
    s.mouse(64, 5, 3, false);
    format_is("#{pane_in_mode}", "1", "copy mode from the wheel");
    s.mouse(65, 5, 3, false);
    format_is("#{pane_in_mode}", "0", "copy mode left at the bottom");
    // MouseDown1Status: a click on a window's name selects it.
    t.run(&["new-window", "-t", "md", "-n", "second"]);
    format_is("#{window_index}", "1", "the new window current");
    s.wait_for("both windows listed", |text| text.contains("second"));
    let cells = s.status_cells();
    let col = (0..cells.len())
        .find(|&i| {
            cells[i..]
                .iter()
                .take(5)
                .map(|c| c.0.as_str())
                .collect::<String>()
                == "first"
        })
        .expect("first window in the status line");
    s.click(0, u16::try_from(col).unwrap(), ROWS - 1);
    format_is("#{window_index}", "0", "the clicked window current");
    // The status line's parts have keys of their own.
    t.run(&["set-option", "-g", "status-left", "LEFTPART "]);
    t.run(&["set-option", "-g", "status-right", " RIGHTPART"]);
    t.run(&[
        "bind-key",
        "-n",
        "MouseDown1StatusLeft",
        "set-option -g @hit left",
    ]);
    t.run(&[
        "bind-key",
        "-n",
        "MouseDown1StatusRight",
        "set-option -g @hit right",
    ]);
    t.run(&[
        "bind-key",
        "-n",
        "MouseDown1StatusDefault",
        "set-option -g @hit gap",
    ]);
    s.wait_for("the new status line", |text| text.contains("RIGHTPART"));
    s.click(0, 1, ROWS - 1);
    format_is("#{@hit}", "left", "a click on status-left");
    s.click(0, COLS - 2, ROWS - 1);
    format_is("#{@hit}", "right", "a click on status-right");
    s.click(0, COLS / 2, ROWS - 1);
    format_is("#{@hit}", "gap", "a click between");
}

#[test]
fn dragging_in_a_pane_copies_the_selection() {
    let t = Tmxr::new("mousedrag");
    let s = t.attach(&["new", "-s", "dr"]);
    s.wait_for("status line", |text| text.contains("dr"));
    s.wait_mouse(true);
    s.send(b"echo @drag-me\r");
    s.wait_for("echoed", |text| {
        text.lines().any(|l| l.trim_end() == "@drag-me")
    });
    let row = s
        .text()
        .lines()
        .position(|l| l.trim_end() == "@drag-me")
        .expect("output line");
    let row = u16::try_from(row).unwrap();
    // MouseDrag1Pane starts a selection in copy mode, the drag extends it and
    // MouseDragEnd1Pane copies it and leaves copy mode.
    s.mouse(0, 0, row, false);
    s.mouse(32, 3, row, false);
    s.mouse(32, 7, row, false);
    s.mouse(0, 7, row, true);
    t.wait_run(&["show-buffer"], "the selection copied", |o| {
        o.trim_end() == "@drag-me"
    });
    t.wait_run(
        &["display-message", "-p", "-t", "dr", "#{pane_in_mode}"],
        "copy mode left",
        |o| o.trim() == "0",
    );
}

#[test]
fn mouse_keys_can_be_bound_and_unbound() {
    let t = Tmxr::new("mousebind");
    let s = t.attach(&["new", "-s", "mb"]);
    s.wait_for("status line", |text| text.contains("mb"));
    s.wait_mouse(true);
    t.run(&[
        "bind-key",
        "-n",
        "MouseDown3Pane",
        "rename-window",
        "clicked",
    ]);
    t.run(&["unbind-key", "-n", "WheelUpPane"]);
    let keys = t.run(&["list-keys", "-T", "root"]);
    assert!(
        keys.contains("MouseDown3Pane rename-window clicked") && !keys.contains("WheelUpPane"),
        "{keys}"
    );
    // The unbound wheel goes to the program instead of into copy mode; the
    // right click after it shows both were handled.
    s.mouse(64, 5, 3, false);
    s.click(2, 5, 3);
    t.wait_run(
        &["display-message", "-p", "-t", "mb", "#{window_name}"],
        "the right-click bind run",
        |o| o.trim() == "clicked",
    );
    let mode = t.run(&["display-message", "-p", "-t", "mb", "#{pane_in_mode}"]);
    assert_eq!(mode.trim(), "0", "unbound WheelUpPane entered copy mode");
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
        r"printf 'abc\033Ptmux;\033\033]1337;tmxr-pt-marker\007\033\\after-dcs'; sleep 30",
    ];
    #[cfg(windows)]
    let program = [
        "powershell.exe",
        "-NoProfile",
        "-Command",
        r"$e=[char]27; [Console]::Out.Write('abc'+$e+'Ptmux;'+$e+$e+']1337;tmxr-pt-marker'+[char]7+$e+'\'+'after-dcs'); Start-Sleep 30",
    ];
    let mut args = vec!["new", "-s", "pt"];
    args.extend(program);
    let s = t.attach(&args);
    // On every platform, output after the DCS still reaches the pane.
    s.wait_for("text after the DCS", |text| text.contains("after-dcs"));
    assert!(!s.text().contains("tmux;"), "{}", s.text());

    let forwarded = s.raw_text().contains("\x1b]1337;tmxr-pt-marker\x07");
    // On Windows passthrough needs Microsoft's ConPTY beside tmxr.exe (the
    // release ships it; CI's conpty job adds it): with it the payload is
    // forwarded, without it tmxr leaves passthrough alone and says so.
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_tmxr"));
    let bundled = bin.with_file_name("conpty.dll").is_file()
        && bin.with_file_name("OpenConsole.exe").is_file();
    if cfg!(windows) && !bundled {
        assert!(!forwarded);
        let messages = t.run(&["show-messages"]);
        assert!(messages.contains("allow-passthrough"), "{messages}");
    } else {
        assert!(forwarded, "payload not forwarded: {:?}", s.raw_text());
        // From the cell it was printed at, after "abc": row 1, column 4.
        assert!(
            s.raw_text()
                .contains("\x1b[1;4H\x1b]1337;tmxr-pt-marker\x07"),
            "not placed at the pane's cursor: {:?}",
            s.raw_text()
        );
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

    // prefix f asks for text and lists the windows it matches.
    s.send(PREFIX);
    s.send(b"f");
    s.wait_for("find prompt", |text| text.contains("(find-window)"));
    s.send(b"editor\r");
    s.wait_for("the matching window", |text| text.contains("windows 1/1"));
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
    // The marker starts with @, which no temp-dir name in the prompt holds:
    // counting a plain letter flaked when the random name contained it.
    s.send(b"echo @zq-hit @zq-hit\r");
    s.wait_for("echoed", |text| text.matches("@zq-hit").count() >= 4);
    s.send(PREFIX);
    s.send(b"[");
    s.send(b"?");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    s.send(b"@zq-hit\r");
    // Every match on screen is highlighted, the one under the cursor apart.
    let backgrounds = |s: &Screen| -> Vec<String> {
        (0..ROWS - 1)
            .flat_map(|row| s.row_cells(row))
            .filter(|c| c.0 == "@")
            .map(|c| c.2)
            .collect()
    };
    s.wait_for("highlighted", |_| {
        let bgs = backgrounds(&s);
        bgs.len() >= 4 && bgs.iter().filter(|b| **b == rgb(MAUVE)).count() == 1
    });
    let bgs = backgrounds(&s);
    let where_marker: Vec<(u16, usize)> = (0..ROWS - 1)
        .flat_map(|row| {
            s.row_cells(row)
                .into_iter()
                .enumerate()
                .filter(|(_, c)| c.0 == "@")
                .map(move |(col, _)| (row, col))
        })
        .collect();
    assert!(
        bgs.iter().all(|b| *b == rgb(YELLOW) || *b == rgb(MAUVE)),
        "{bgs:?} at {where_marker:?}\n{}",
        s.text()
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
    // Saves include titles: one arriving late would change the next save.
    t.wait_settled("keep");
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
    t.wait_prompt("alpha");
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
    // Up moves to bravo, which never printed the marker.
    s.send(b"\x1b[A");
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

#[cfg(unix)]
#[test]
fn suspend_client_stops_and_fg_resumes() {
    let t = Tmxr::new("suspend");
    // tmxr under an interactive shell, so it has job control to stop in.
    let sh = t.spawn("/bin/sh", &["-i".to_owned()]);
    let line: Vec<String> = std::iter::once(env!("CARGO_BIN_EXE_tmxr").to_owned())
        .chain(t.args(&["new", "-s", "sus"]))
        .collect();
    sh.send(format!("{}\r", line.join(" ")).as_bytes());
    // The typed command line holds "sus" too: attached means tmxr has taken
    // the screen, so the line is gone and the status line shows the session.
    sh.wait_for("attached", |text| {
        text.contains("sus") && !text.contains("new -s sus")
    });
    sh.send(PREFIX);
    sh.send(b"\x1a");
    sh.wait_for("stopped job", |text| {
        text.to_lowercase().contains("stopped")
    });
    sh.send(b"fg\r");
    sh.wait_for("redrawn after fg", |text| {
        text.lines().last().is_some_and(|l| l.contains("sus"))
    });
}

#[cfg(windows)]
#[test]
fn suspend_client_says_windows_cannot() {
    let t = Tmxr::new("suspend");
    let s = t.attach(&["new", "-s", "sus"]);
    s.wait_for("status line", |text| text.contains("sus"));
    s.send(PREFIX);
    s.send(b"\x1a");
    s.wait_for("error", |text| text.contains("no job control"));
}

#[test]
fn copy_pipe_sends_the_selection_to_copy_command() {
    let t = Tmxr::new("pipe");
    let sink = t.dir.path().join("piped.txt");
    #[cfg(unix)]
    let command = format!("cat > '{}'", sink.display());
    #[cfg(windows)]
    let command = format!("$input | Set-Content -NoNewline -Path '{}'", sink.display());
    let s = t.attach(&["new", "-s", "pp"]);
    s.wait_for("status line", |text| text.contains("pp"));
    t.run(&["set-option", "-g", "copy-command", &command]);
    // The command is PowerShell, and shell commands run with default-shell.
    #[cfg(windows)]
    t.run(&["set-option", "-g", "default-shell", "powershell.exe"]);
    s.send(b"echo pipeme-77\r");
    s.wait_for("echoed", |text| text.matches("pipeme-77").count() >= 2);
    s.send(PREFIX);
    s.send(b"[");
    s.send(b"?");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    s.send(b"pipeme\r");
    s.send(b"vE");
    // copy-pipe-and-cancel with no command uses copy-command.
    std::thread::sleep(Duration::from_millis(200));
    t.run(&["send-keys", "-t", "pp", "-X", "copy-pipe-and-cancel"]);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let got = std::fs::read_to_string(&sink).unwrap_or_default();
        if got.trim_end() == "pipeme-77" {
            break;
        }
        assert!(Instant::now() < deadline, "sink holds {got:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(t.run(&["show-buffer"]), "pipeme-77");
}

#[test]
fn copy_mode_refresh_picks_up_new_output() {
    let t = Tmxr::new("refresh");
    let s = t.attach(&["new", "-s", "rf"]);
    s.wait_for("status line", |text| text.contains("rf"));
    s.send(PREFIX);
    s.send(b"[");
    t.wait_run(
        &["display-message", "-p", "-t", "rf", "#{pane_in_mode}"],
        "copy mode",
        |o| o.trim() == "1",
    );
    // Output arriving meanwhile goes to the live pane, not the snapshot.
    t.run(&["send-keys", "-t", "rf", "echo late-arrival", "Enter"]);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!s.text().contains("late-arrival"), "{}", s.text());
    // r takes a fresh snapshot, still in copy mode.
    s.send(b"r");
    s.wait_for("refreshed", |text| text.contains("late-arrival"));
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "rf", "#{pane_in_mode}"])
            .trim(),
        "1"
    );
}

#[test]
fn move_window_renumbers_and_replaces() {
    let t = Tmxr::new("movewk");
    let windows = |t: &Tmxr| -> Vec<String> {
        t.run(&["list-windows", "-t", "mk"])
            .lines()
            .map(|l| {
                let (idx, rest) = l.split_once(": ").unwrap_or((l, ""));
                let name = rest.split([' ', '*', '-']).next().unwrap_or("");
                format!("{idx}:{name}")
            })
            .collect()
    };
    t.run(&["new-session", "-d", "-s", "mk", "-n", "a"]);
    t.run(&["new-window", "-d", "-t", "mk", "-n", "b"]);
    t.run(&["move-window", "-d", "-s", "mk:1", "-t", "mk:7"]);
    assert_eq!(windows(&t), ["0:a", "7:b"]);
    // -r closes the gaps.
    t.run(&["move-window", "-r", "-t", "mk"]);
    assert_eq!(windows(&t), ["0:a", "1:b"]);
    // Without -k an occupied index is refused; with it, its window goes.
    assert!(
        !t.output(&["move-window", "-d", "-s", "mk:0", "-t", "mk:1"])
            .status
            .success()
    );
    t.run(&["move-window", "-d", "-k", "-s", "mk:0", "-t", "mk:1"]);
    assert_eq!(windows(&t), ["1:a"]);
}

#[test]
fn copy_mode_shows_a_pending_count_and_marked_border() {
    let t = Tmxr::new("indicators");
    let s = t.attach(&["new", "-s", "in"]);
    s.wait_for("status line", |text| text.contains("in"));

    // A count being typed shows until the command that uses it.
    s.send(PREFIX);
    s.send(b"[");
    t.wait_run(
        &["display-message", "-p", "-t", "in", "#{pane_in_mode}"],
        "copy mode",
        |o| o.trim() == "1",
    );
    s.send(b"5");
    s.wait_for("count shown", |text| text.contains("(repeat) 5"));
    s.send(b"k");
    s.wait_for("count used", |text| !text.contains("(repeat)"));
    s.send(b"q");

    // The marked pane's border is drawn reversed.
    let reversed = |s: &Screen| -> usize {
        let emu = s.emu.lock().unwrap();
        let screen = emu.screen();
        (0..ROWS - 1)
            .flat_map(|row| (0..COLS).map(move |col| (row, col)))
            .filter(|(row, col)| screen.cell(*row, *col).is_some_and(|c| c.inverse()))
            .count()
    };
    t.run(&["split-window", "-h", "-t", "in"]);
    s.wait_for("split", |text| text.contains('│'));
    let before = reversed(&s);
    t.run(&["select-pane", "-m", "-t", "in.0"]);
    s.wait_for("marked border", |_| reversed(&s) > before);
    t.run(&["select-pane", "-M"]);
    s.wait_for("mark cleared", |_| reversed(&s) == before);
}

#[cfg(unix)]
#[test]
fn sigterm_saves_sessions_before_the_server_exits() {
    let t = Tmxr::new("sigterm");
    t.run(&["new-session", "-d", "-s", "keepme"]);
    let pid = t.run(&["display-message", "-p", "-t", "keepme", "#{pid}"]);
    let pid = pid.trim();
    assert!(pid.parse::<u32>().is_ok(), "{pid:?}");
    let killed = Command::new("kill").args(["-TERM", pid]).status().unwrap();
    assert!(killed.success());
    t.wait_run(&["ls"], "server gone", str::is_empty);
    let root = t.dir.path().join("data").join("tmxr").join("resurrect");
    let saved: Vec<String> = std::fs::read_dir(&root)
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
        .map(|f| std::fs::read_to_string(f.path()).unwrap_or_default())
        .collect();
    assert!(saved.iter().any(|s| s.contains("keepme")), "{saved:?}");
}

#[test]
fn detach_stays_responsive_under_flood_output() {
    // A pane printing changing lines as fast as it can.
    #[cfg(unix)]
    let flood = ["/bin/sh", "-c", "seq 1 999999999"];
    #[cfg(windows)]
    let flood = [
        "cmd.exe",
        "/d",
        "/c",
        "for /l %i in (0,1,99999999) do @echo %i yyyyyyyyyyyyyyyyyyyyyyyy",
    ];
    let t = Tmxr::new("flood");
    let mut args = vec!["new", "-s", "fl"];
    args.extend(flood);
    let mut s = t.attach(&args);
    std::thread::sleep(Duration::from_secs(2));
    let start = Instant::now();
    s.send(PREFIX);
    s.send(b"d");
    s.wait_exit();
    let took = start.elapsed();
    assert!(
        took < Duration::from_secs(5),
        "detach took {took:?} under flood"
    );
    eprintln!("detach under flood: {took:?}");
}

#[test]
fn emacs_copy_mode_selects_with_its_own_keys() {
    let t = Tmxr::new("emacs");
    let s = t.attach(&["new", "-s", "em"]);
    s.wait_for("status line", |text| text.contains("em"));
    let bad = t.output(&["set-option", "-g", "mode-keys", "bogus"]);
    assert!(
        !bad.status.success() && String::from_utf8_lossy(&bad.stderr).contains("vi or emacs"),
        "{bad:?}"
    );
    t.run(&["set-option", "-g", "mode-keys", "emacs"]);
    s.send(b"echo @emacs-copy\r");
    s.wait_for("echoed", |text| {
        text.lines().any(|l| l.trim_end() == "@emacs-copy")
    });
    s.send(PREFIX);
    s.send(b"[");
    // C-r searches up to the output line's start.
    s.send(b"\x12");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    s.send(b"@emacs-copy\r");
    s.wait_for("search done", |text| !text.contains("(search up)"));
    // C-Space, then M-1 M-0 C-f: ten cells right, and M-w copies.
    s.send(b"\x00");
    s.send(b"\x1b1");
    s.send(b"\x1b0");
    s.send(b"\x06");
    s.send(b"\x1bw");
    t.wait_run(&["show-buffer"], "the selection copied", |o| {
        o.trim_end() == "@emacs-copy"
    });
}

#[test]
fn resurrect_restores_allowlisted_programs_with_their_arguments() {
    // A program with arguments; on Windows the pane's cmd runs it as a child.
    #[cfg(unix)]
    let (program, start, args) = ("sleep", vec!["sleep", "300"], vec!["300"]);
    #[cfg(windows)]
    let (program, start, args) = (
        "PING",
        vec!["cmd.exe", "/d", "/c", "ping -n 300 127.0.0.1 >NUL"],
        vec!["-n", "300", "127.0.0.1"],
    );
    let t = Tmxr::new("resargs");
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        format!("{config}processes = [{program:?}]\nrestore-args = [{program:?}]\n"),
    )
    .unwrap();
    let mut new = vec!["new-session", "-d", "-s", "main"];
    new.extend(&start);
    t.run(&new);
    let running = |t: &Tmxr| {
        t.wait_run(
            &[
                "display-message",
                "-p",
                "-t",
                "main",
                "#{pane_current_command}",
            ],
            "the program running",
            |o| o.trim().eq_ignore_ascii_case(program),
        );
    };
    let saved_args = |t: &Tmxr| saved_args(&t.newest_save());
    running(&t);
    t.run(&["resurrect-save"]);
    assert_eq!(saved_args(&t), args);

    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        config.replace("restore-on-start = false", "restore-on-start = true"),
    )
    .unwrap();
    t.run(&["new-session", "-d", "-s", "other"]);
    // The restored program got its arguments back: a fresh save reads them
    // from the running process again.
    running(&t);
    std::thread::sleep(Duration::from_millis(5));
    t.run(&["resurrect-save"]);
    assert_eq!(saved_args(&t), args);
    t.run(&["kill-server"]);
}

#[test]
fn copy_end_of_line_takes_the_rest_of_the_line() {
    let t = Tmxr::new("eol");
    let s = t.attach(&["new", "-s", "eo"]);
    s.wait_for("status line", |text| text.contains("eo"));
    s.send(b"echo head @eol-copy tail\r");
    s.wait_for("echoed", |text| {
        text.lines().any(|l| l.trim_end() == "head @eol-copy tail")
    });
    s.send(PREFIX);
    s.send(b"[");
    s.send(b"?");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    // The output line is the nearest match above the prompt.
    s.send(b"@eol-copy\r");
    s.wait_for("search done", |text| !text.contains("(search up)"));
    s.send(b"D");
    t.wait_run(&["show-buffer"], "the rest of the line copied", |o| {
        o.trim_end() == "@eol-copy tail"
    });
}

#[test]
fn double_and_triple_clicks_copy_a_word_and_a_line() {
    let t = Tmxr::new("clicks");
    let s = t.attach(&["new", "-s", "ck"]);
    s.wait_for("status line", |text| text.contains("ck"));
    s.wait_mouse(true);
    s.send(b"echo head @clicked tail\r");
    s.wait_for("echoed", |text| {
        text.lines().any(|l| l.trim_end() == "head @clicked tail")
    });
    let row = s
        .text()
        .lines()
        .position(|l| l.trim_end() == "head @clicked tail")
        .expect("output line");
    let row = u16::try_from(row).unwrap();
    // DoubleClick1Pane: the word under the mouse (cols 6..=12, "clicked";
    // the @ is punctuation, its own word).
    s.click(0, 8, row);
    s.click(0, 8, row);
    t.wait_run(&["show-buffer"], "the word copied", |o| {
        o.trim_end() == "clicked"
    });
    // TripleClick1Pane: the whole line.
    s.click(0, 2, row);
    s.click(0, 2, row);
    s.click(0, 2, row);
    t.wait_run(&["show-buffer"], "the line copied", |o| {
        o.trim_end() == "head @clicked tail"
    });

    // The second press is SecondClick1Pane; DoubleClick1Pane comes from the
    // click timer, so a third press in time cancels it.
    let opt = |t: &Tmxr, name: &str| {
        t.run(&["display-message", "-p", &format!("#{{@{name}}}")])
            .trim()
            .to_owned()
    };
    t.run(&[
        "bind-key",
        "-n",
        "SecondClick1Pane",
        "set-option -g @second yes",
    ]);
    t.run(&[
        "bind-key",
        "-n",
        "DoubleClick1Pane",
        "set-option -g @double fired",
    ]);
    t.run(&["set-option", "-g", "@double", "none"]);
    s.click(0, 2, row);
    s.click(0, 2, row);
    s.click(0, 2, row);
    t.wait_run(
        &["display-message", "-p", "#{@second}"],
        "SecondClick",
        |o| o.trim() == "yes",
    );
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(
        opt(&t, "double"),
        "none",
        "a triple click fired DoubleClick"
    );
    // A double click alone does, once the click time has passed.
    std::thread::sleep(Duration::from_millis(400));
    s.click(0, 2, row);
    s.click(0, 2, row);
    t.wait_run(
        &["display-message", "-p", "#{@double}"],
        "DoubleClick",
        |o| o.trim() == "fired",
    );
}

#[test]
fn pane_current_path_follows_the_shells_cd() {
    let t = Tmxr::new("cwd");
    let sub = t.dir.path().join("moved-here");
    std::fs::create_dir(&sub).unwrap();
    let s = t.attach(&["new", "-s", "cw"]);
    s.wait_for("status line", |text| text.contains("cw"));
    // cmd reports no directory itself, so on Windows this is read from the
    // shell's process (its PEB); on Unix from /proc or proc_pidinfo.
    let cd = if cfg!(windows) { "cd /d" } else { "cd" };
    s.send(format!("{cd} \"{}\"\r", sub.display()).as_bytes());
    let want = sub.canonicalize().unwrap();
    let in_sub = |what: &str| {
        t.wait_run(
            &["display-message", "-p", "-t", "cw", "#{pane_current_path}"],
            what,
            |o| std::path::Path::new(o.trim()).canonicalize().ok() == Some(want.clone()),
        );
    };
    in_sub("the new directory");
    // The tmux config's binds open new panes and windows there: prefix %
    // splits, prefix c makes a window, each started in the pane's directory.
    s.send(PREFIX);
    s.send(b"%");
    t.wait_run(&["list-panes", "-t", "cw"], "the split", |o| {
        o.lines().count() == 2
    });
    in_sub("the split pane in the directory");
    s.send(PREFIX);
    s.send(b"c");
    t.wait_run(
        &["display-message", "-p", "-t", "cw", "#{window_index}"],
        "the new window",
        |o| o.trim() == "1",
    );
    in_sub("the new window in the directory");
}

#[test]
fn emacs_search_moves_as_you_type() {
    let t = Tmxr::new("incsearch");
    let s = t.attach(&["new", "-s", "is"]);
    s.wait_for("status line", |text| text.contains("is"));
    t.run(&["set-option", "-g", "mode-keys", "emacs"]);
    s.send(b"echo @inc-search\r");
    s.wait_for("echoed", |text| {
        text.lines().any(|l| l.trim_end() == "@inc-search")
    });
    s.send(PREFIX);
    s.send(b"[");
    s.send(b"\x12");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    // Typed, never entered: Escape leaves the prompt with the cursor where
    // the typing took it.
    s.send(b"@inc-search");
    s.wait_for("typed", |text| text.contains("(search up) @inc-search"));
    s.send(b"\x1b");
    s.wait_for("prompt closed", |text| !text.contains("(search up)"));
    // C-Space, M-1 M-0 C-f, M-w: the eleven cells from the match.
    s.send(b"\x00");
    s.send(b"\x1b1");
    s.send(b"\x1b0");
    s.send(b"\x06");
    s.send(b"\x1bw");
    t.wait_run(&["show-buffer"], "the match copied", |o| {
        o.trim_end() == "@inc-search"
    });
}

#[test]
fn panes_inherit_the_servers_path() {
    let t = Tmxr::new("path");
    let extra = t.dir.path().join("extra-bin");
    std::fs::create_dir(&extra).unwrap();
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(extra.clone()).chain(std::env::split_paths(&inherited)),
    )
    .unwrap();
    // The server starts with the extended PATH, as from a shell that set it.
    let started = Command::new(env!("CARGO_BIN_EXE_tmxr"))
        .args(t.args(&["new-session", "-d", "-s", "pa"]))
        .envs(t.env())
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(started.status.success(), "{started:?}");
    // Only the start of PATH, so it fits on the screen.
    let echo = if cfg!(windows) {
        "echo %PATH:~0,200%"
    } else {
        "echo ${PATH%%:*}"
    };
    t.wait_prompt("pa");
    t.run(&["send-keys", "-t", "pa", echo, "Enter"]);
    let want = extra.display().to_string();
    t.wait_run(
        &["capture-pane", "-p", "-t", "pa"],
        "the pane's PATH",
        |o| o.lines().any(|l| l.starts_with(&want)),
    );
}

#[test]
fn select_pane_z_keeps_a_zoomed_window_zoomed() {
    let t = Tmxr::new("selz");
    t.run(&["new-session", "-d", "-s", "z"]);
    t.run(&["split-window", "-h", "-t", "z"]);
    let state = |t: &Tmxr| {
        t.run(&[
            "display-message",
            "-p",
            "-t",
            "z",
            "#{pane_index} #{window_zoomed_flag}",
        ])
        .trim()
        .to_owned()
    };
    t.run(&["resize-pane", "-Z", "-t", "z"]);
    assert_eq!(state(&t), "1 1");
    t.run(&["select-pane", "-Z", "-L", "-t", "z"]);
    assert_eq!(state(&t), "0 1", "moved and still zoomed");
    // Without -Z, selecting another pane unzooms, as in tmux.
    t.run(&["select-pane", "-R", "-t", "z"]);
    assert_eq!(state(&t), "1 0");
}

/// What Windows sends at logoff or shutdown, sent by hand: the server saves
/// its sessions and exits.
#[cfg(windows)]
#[test]
fn windows_session_end_saves_and_exits() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SMTO_BLOCK, SendMessageTimeoutW, WM_ENDSESSION,
    };
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(Some(0)).collect() };
    let t = Tmxr::new("sessend");
    t.run(&["new-session", "-d", "-s", "kept-at-logoff"]);
    let pid = t
        .run(&["display-message", "-p", "#{pid}"])
        .trim()
        .to_owned();
    let (class, title) = (wide("tmxr-server"), wide(&format!("tmxr-server {pid}")));
    let deadline = Instant::now() + TIMEOUT;
    let hwnd = loop {
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let h = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
        if !h.is_null() {
            break h;
        }
        assert!(Instant::now() < deadline, "no session-end window for {pid}");
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut result = 0usize;
    // ENDSESSION_LOGOFF; the call returns once the server's handler has.
    // SAFETY: `hwnd` is a window handle, `result` a live usize.
    let sent = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_ENDSESSION,
            1,
            0x8000_0000,
            SMTO_BLOCK,
            10_000,
            &mut result,
        )
    };
    assert_ne!(sent, 0, "WM_ENDSESSION was not delivered");
    t.wait_run(&["ls"], "server gone", str::is_empty);
    let root = t.dir.path().join("data").join("tmxr").join("resurrect");
    let saved: Vec<String> = std::fs::read_dir(&root)
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
        .map(|f| std::fs::read_to_string(f.path()).unwrap_or_default())
        .collect();
    assert!(
        saved.iter().any(|s| s.contains("kept-at-logoff")),
        "{saved:?}"
    );
}

/// A pipe the starter of `tmxr new -d` left inheritable (a script reading
/// its output to the end) reaches end-of-file when the client exits, though
/// the server it started keeps running.
#[test]
fn the_server_does_not_keep_its_starters_pipes_open() {
    use std::io::Read;
    let t = Tmxr::new("inherit");
    let (mut reader, writer) = std::io::pipe().unwrap();
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: clears close-on-exec on a descriptor this test owns.
        assert_eq!(
            unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_SETFD, 0) },
            0
        );
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
        // SAFETY: sets the inherit flag on a handle this test owns.
        let ok = unsafe {
            SetHandleInformation(
                writer.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        };
        assert_ne!(ok, 0);
    }
    let started = t.output(&["new-session", "-d", "-s", "ih"]);
    assert!(started.status.success(), "{started:?}");
    drop(writer);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut rest = Vec::new();
        let _ = reader.read_to_end(&mut rest);
        let _ = tx.send(());
    });
    assert!(
        rx.recv_timeout(Duration::from_secs(10)).is_ok(),
        "the server still holds its starter's pipe"
    );
    assert!(
        t.run(&["ls"]).contains("ih:"),
        "the server is still running"
    );
}

#[test]
fn tmux_commands_move_pane_previous_layout_show_window_options_start_server() {
    let t = Tmxr::new("aliases");
    // start: a server with no sessions, which `ls` reaches.
    let started = t.output(&["start"]);
    assert!(started.status.success(), "{started:?}");
    let ls = t.output(&["ls"]);
    assert!(ls.status.success(), "no server after start: {ls:?}");

    t.run(&["new-session", "-d", "-s", "m"]);
    t.run(&["new-window", "-d", "-t", "m"]);
    // movep: window 1's pane joins window 0, and window 1 goes.
    t.run(&["movep", "-s", "m:1.0", "-t", "m:0"]);
    assert_eq!(t.run(&["list-panes", "-t", "m:0"]).lines().count(), 2);
    assert_eq!(t.run(&["list-windows", "-t", "m"]).lines().count(), 1);

    // prevl undoes nextl: side by side, then stacked, then side by side.
    let geometry = |t: &Tmxr| t.run(&["list-panes", "-t", "m:0"]);
    t.run(&["select-layout", "-t", "m:0", "even-horizontal"]);
    let before = geometry(&t);
    t.run(&["nextl", "-t", "m:0"]);
    assert_ne!(geometry(&t), before, "nextl changed nothing");
    t.run(&["prevl", "-t", "m:0"]);
    assert_eq!(geometry(&t), before);

    let showw = t.run(&["showw"]);
    assert!(showw.contains("mode-keys"), "{showw}");
}

#[test]
fn environment_commands_set_show_and_reach_new_panes() {
    let t = Tmxr::new("env");
    t.run(&["new-session", "-d", "-s", "ev"]);
    t.run(&["setenv", "-g", "TMXR_GLOBAL", "from-global"]);
    t.run(&["setenv", "-t", "ev", "TMXR_SESSION", "from-session"]);
    // A variable the server has (the test harness sets it): kept out of new
    // panes.
    t.run(&["setenv", "-r", "-t", "ev", "TMXR_TMPDIR"]);

    let session = t.run(&["showenv", "-t", "ev"]);
    assert!(
        session.lines().any(|l| l == "TMXR_SESSION=from-session")
            && session.lines().any(|l| l == "-TMXR_TMPDIR"),
        "{session}"
    );
    assert_eq!(
        t.run(&["showenv", "-g", "TMXR_GLOBAL"]).trim(),
        "TMXR_GLOBAL=from-global"
    );
    assert_eq!(
        t.run(&["showenv", "-g", "-s", "TMXR_GLOBAL"]).trim(),
        r#"TMXR_GLOBAL="from-global"; export TMXR_GLOBAL;"#
    );
    let unknown = t.output(&["showenv", "-t", "ev", "TMXR_NOPE"]);
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("unknown variable"),
        "{unknown:?}"
    );

    // A new pane gets the global and session values, without the removed one.
    t.run(&["new-window", "-t", "ev"]);
    #[cfg(windows)]
    let (echo, want) = (
        "echo [%TMXR_GLOBAL%][%TMXR_SESSION%][%TMXR_TMPDIR%]",
        "[from-global][from-session][%TMXR_TMPDIR%]",
    );
    #[cfg(unix)]
    let (echo, want) = (
        r#"echo "[$TMXR_GLOBAL][$TMXR_SESSION][$TMXR_TMPDIR]""#,
        "[from-global][from-session][]",
    );
    t.wait_prompt("ev:1");
    t.run(&["send-keys", "-t", "ev:1", echo, "Enter"]);
    t.wait_run(
        &["capture-pane", "-p", "-t", "ev:1"],
        "the pane's view",
        |o| o.lines().any(|l| l.trim_end() == want),
    );

    // -u forgets the session's value.
    t.run(&["setenv", "-u", "-t", "ev", "TMXR_SESSION"]);
    assert!(
        !t.output(&["showenv", "-t", "ev", "TMXR_SESSION"])
            .status
            .success()
    );
}

#[test]
fn clear_history_and_respawn_window() {
    let t = Tmxr::new("clearhist");
    #[cfg(unix)]
    let (lines, done) = ("seq 1 80; echo printed-all", "printed-all");
    #[cfg(windows)]
    let (lines, done) = ("for /l %i in (1,1,80) do @echo %i", "80");
    t.run(&["new-session", "-d", "-s", "ch"]);
    t.wait_prompt("ch");
    t.run(&["send-keys", "-t", "ch", lines, "Enter"]);
    let history = |t: &Tmxr| -> usize {
        t.run(&["display-message", "-p", "-t", "ch", "#{history_size}"])
            .trim()
            .parse()
            .unwrap()
    };
    t.wait_run(
        &["capture-pane", "-p", "-t", "ch"],
        "the lines printed",
        |o| o.lines().any(|l| l.trim_end() == done),
    );
    assert!(history(&t) > 0, "nothing scrolled into history");
    t.run(&["clearhist", "-t", "ch"]);
    assert_eq!(history(&t), 0);

    // respawnw: back to one pane, refused while a program runs, unless -k.
    t.run(&["split-window", "-t", "ch"]);
    let refused = t.output(&["respawnw", "-t", "ch"]);
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("still active"),
        "{refused:?}"
    );
    t.run(&["respawnw", "-k", "-t", "ch"]);
    assert_eq!(t.run(&["list-panes", "-t", "ch"]).lines().count(), 1);
}

#[test]
fn pipe_pane_copies_output_to_a_command() {
    let t = Tmxr::new("pipepane");
    let log = t.dir.path().join("pane.log");
    #[cfg(unix)]
    let command = format!("cat >> '{}'", log.display());
    #[cfg(windows)]
    let command = format!(
        "$input | ForEach-Object {{ Add-Content -LiteralPath '{}' -Value $_ }}",
        log.display()
    );
    t.run(&["new-session", "-d", "-s", "pp"]);
    // The command is PowerShell, and shell commands run with default-shell.
    #[cfg(windows)]
    t.run(&["set-option", "-g", "default-shell", "powershell.exe"]);
    let piped = |t: &Tmxr| t.run(&["display-message", "-p", "-t", "pp", "#{pane_pipe}"]);
    t.run(&["pipep", "-t", "pp", &command]);
    assert_eq!(piped(&t).trim(), "1");
    t.wait_prompt("pp");
    t.run(&["send-keys", "-t", "pp", "echo @piped-text", "Enter"]);
    t.wait_run(&["capture-pane", "-p", "-t", "pp"], "the echo", |o| {
        o.lines().any(|l| l.trim_end() == "@piped-text")
    });
    // No command closes the pipe; the command then finishes its input.
    t.run(&["pipep", "-t", "pp"]);
    assert_eq!(piped(&t).trim(), "0");
    let deadline = Instant::now() + TIMEOUT;
    while !std::fs::read_to_string(&log).is_ok_and(|s| s.contains("@piped-text")) {
        assert!(
            Instant::now() < deadline,
            "the output never reached the log"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // -o toggles: it opens a pipe when none is open, and closes an open one.
    t.run(&["pipep", "-o", "-t", "pp", &command]);
    assert_eq!(piped(&t).trim(), "1");
    t.run(&["pipep", "-o", "-t", "pp", &command]);
    assert_eq!(piped(&t).trim(), "0");
}

#[test]
fn prompt_history_is_kept_recalled_and_cleared() {
    let t = Tmxr::new("phist");
    let s = t.attach(&["new", "-s", "ph"]);
    s.wait_for("status line", |text| text.contains("ph"));
    let name = |t: &Tmxr| {
        t.run(&["display-message", "-p", "-t", "ph", "#W"])
            .trim()
            .to_owned()
    };
    for entry in ["rename-window first", "rename-window second"] {
        s.send(PREFIX);
        s.send(b":");
        s.wait_for("command prompt", |text| {
            text.lines().last().is_some_and(|l| l.starts_with(':'))
        });
        s.send(format!("{entry}\r").as_bytes());
        let want = entry.rsplit(' ').next().unwrap();
        t.wait_run(
            &["display-message", "-p", "-t", "ph", "#W"],
            "renamed",
            |o| o.trim() == want,
        );
    }
    let shown = t.run(&["showphist", "-T", "command"]);
    assert!(
        shown.contains("1: rename-window first") && shown.contains("2: rename-window second"),
        "{shown}"
    );
    // Up twice recalls the older entry; Enter runs it again.
    s.send(PREFIX);
    s.send(b":");
    s.wait_for("command prompt", |text| {
        text.lines().last().is_some_and(|l| l.starts_with(':'))
    });
    s.send(b"\x1b[A");
    s.send(b"\x1b[A");
    s.wait_for("recalled", |text| text.contains(":rename-window first"));
    s.send(b"\r");
    t.wait_run(
        &["display-message", "-p", "-t", "ph", "#W"],
        "renamed back",
        |o| o.trim() == "first",
    );
    assert_eq!(name(&t), "first");
    t.run(&["clearphist"]);
    assert_eq!(t.run(&["showphist"]).trim(), "");
}

#[test]
fn wait_for_blocks_until_signalled_or_unlocked() {
    let t = Tmxr::new("waitfor");
    t.run(&["new-session", "-d", "-s", "wf"]);
    let spawn = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_tmxr"))
            .args(t.args(args))
            .envs(t.env())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };
    let exits_within = |child: &mut std::process::Child, d: Duration| {
        let deadline = Instant::now() + d;
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait().unwrap() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    };

    let mut waiter = spawn(&["wait", "done"]);
    assert!(
        exits_within(&mut waiter, Duration::from_millis(500)).is_none(),
        "returned before the signal"
    );
    t.run(&["wait-for", "-S", "done"]);
    let status = exits_within(&mut waiter, TIMEOUT).expect("not released by the signal");
    assert!(status.success());

    assert!(
        t.output(&["wait-for", "-L", "lk"]).status.success(),
        "a free lock is taken at once"
    );
    let mut second = spawn(&["wait-for", "-L", "lk"]);
    assert!(
        exits_within(&mut second, Duration::from_millis(500)).is_none(),
        "took a held lock"
    );
    t.run(&["wait-for", "-U", "lk"]);
    assert!(
        exits_within(&mut second, TIMEOUT)
            .expect("not handed the lock")
            .success()
    );

    // Mid-list, the rest of the list waits for the signal.
    let mut chained = spawn(&["wait-for", "mid", ";", "set-option", "-g", "@after", "yes"]);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(t.run(&["display-message", "-p", "#{@after}"]).trim(), "");
    t.run(&["wait-for", "-S", "mid"]);
    assert!(
        exits_within(&mut chained, TIMEOUT)
            .expect("not released")
            .success()
    );
    assert_eq!(t.run(&["display-message", "-p", "#{@after}"]).trim(), "yes");

    // A bind waits too, with no command client behind it.
    let s = t.attach(&["attach", "-t", "wf"]);
    s.wait_for("status line", |text| text.contains("wf"));
    t.run(&[
        "bind-key",
        "-n",
        "F8",
        "wait-for bound ; set-option -g @bound yes",
    ]);
    s.send(b"\x1b[19~");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(t.run(&["display-message", "-p", "#{@bound}"]).trim(), "");
    t.run(&["wait-for", "-S", "bound"]);
    t.wait_run(
        &["display-message", "-p", "#{@bound}"],
        "the bind's rest",
        |o| o.trim() == "yes",
    );
}

#[test]
fn hooks_run_after_commands_and_events() {
    let t = Tmxr::new("hooks");
    t.run(&["new-session", "-d", "-s", "hk"]);
    let opt = |t: &Tmxr, name: &str| {
        t.run(&[
            "display-message",
            "-p",
            "-t",
            "hk",
            &format!("#{{@{name}}}"),
        ])
        .trim()
        .to_owned()
    };
    // A global after-<command> hook, then a session one in its place.
    t.run(&[
        "set-hook",
        "-g",
        "after-new-window",
        "set-option -g @who global",
    ]);
    t.run(&["new-window", "-d", "-t", "hk"]);
    assert_eq!(opt(&t, "who"), "global");
    t.run(&[
        "set-hook",
        "-t",
        "hk",
        "after-new-window",
        "set-option -g @who session",
    ]);
    t.run(&["new-window", "-d", "-t", "hk"]);
    assert_eq!(opt(&t, "who"), "session");

    // A named event.
    t.run(&[
        "set-hook",
        "-g",
        "session-created",
        "set-option -g @created yes",
    ]);
    t.run(&["new-session", "-d", "-s", "other"]);
    assert_eq!(opt(&t, "created"), "yes");

    // -a appends; show-hooks lists; -u removes.
    t.run(&[
        "set-hook",
        "-ga",
        "session-created",
        "set-option -g @second yes",
    ]);
    let shown = t.run(&["show-hooks", "-g"]);
    assert!(
        shown.contains("session-created[0] set-option -g @created yes")
            && shown.contains("session-created[1] set-option -g @second yes"),
        "{shown}"
    );
    t.run(&["set-hook", "-gu", "session-created"]);
    assert!(!t.run(&["show-hooks", "-g"]).contains("session-created"));

    assert!(
        !t.output(&["set-hook", "-g", "no-such-hook", "ls"])
            .status
            .success()
    );

    // Window and pane hooks: the most specific one an event has runs.
    t.run(&[
        "set-hook",
        "-g",
        "after-select-pane",
        "set-option -g @who global",
    ]);
    t.run(&["new-window", "-d", "-t", "hk:7"]);
    t.run(&[
        "set-hook",
        "-w",
        "-t",
        "hk:7",
        "after-select-pane",
        "set-option -g @who window",
    ]);
    t.run(&["select-pane", "-t", "hk:7.0"]);
    assert_eq!(opt(&t, "who"), "window");
    t.run(&[
        "set-hook",
        "-p",
        "-t",
        "hk:7.0",
        "after-select-pane",
        "set-option -g @who pane",
    ]);
    t.run(&["select-pane", "-t", "hk:7.0"]);
    assert_eq!(opt(&t, "who"), "pane");
    // Elsewhere, the global one.
    t.run(&["select-pane", "-t", "hk:0.0"]);
    assert_eq!(opt(&t, "who"), "global");
    // A respawned pane keeps its hooks.
    t.run(&["respawn-pane", "-k", "-t", "hk:7.0"]);
    t.run(&["select-pane", "-t", "hk:7.0"]);
    assert_eq!(opt(&t, "who"), "pane");
    let shown = t.run(&["show-hooks", "-w", "-t", "hk:7"]);
    assert!(
        shown.contains("after-select-pane[0] set-option -g @who window"),
        "{shown}"
    );
    t.run(&["set-hook", "-gu", "after-select-pane"]);

    // A hook's own commands fire no hooks: one extra window, not a loop.
    t.run(&[
        "set-hook",
        "-t",
        "hk",
        "after-new-window",
        "new-window -d -t hk",
    ]);
    let before = t.run(&["list-windows", "-t", "hk"]).lines().count();
    t.run(&["new-window", "-d", "-t", "hk"]);
    let after = t.run(&["list-windows", "-t", "hk"]).lines().count();
    assert_eq!(after, before + 2, "the command and its hook's window");
}

#[test]
fn display_menu_runs_the_item_picked() {
    let t = Tmxr::new("menu");
    let s = t.attach(&["new", "-s", "mn"]);
    s.wait_for("status line", |text| text.contains("mn"));
    let open = |t: &Tmxr| {
        t.run(&[
            "display-menu",
            "-t",
            "mn",
            "-T",
            "Tools",
            "Rename one",
            "o",
            "rename-window one",
            "",
            "Rename two",
            "w",
            "rename-window two",
        ]);
    };
    let name = |want: &'static str| move |o: &str| o.trim() == want;
    let opened = Instant::now();
    open(&t);
    s.wait_for("the menu", |text| {
        text.contains("Tools") && text.contains("Rename two") && text.contains("(w)")
    });
    // Shown at once, not at the next periodic repaint (up to 5 s later).
    assert!(
        opened.elapsed() < Duration::from_secs(3),
        "the menu took {:?} to show",
        opened.elapsed()
    );
    // An item's key runs it.
    s.send(b"w");
    t.wait_run(
        &["display-message", "-p", "-t", "mn", "#W"],
        "renamed by key",
        name("two"),
    );
    s.wait_for("the menu closed", |text| !text.contains("Rename two"));
    // Down skips the separator; Enter runs the selection.
    open(&t);
    s.wait_for("the menu again", |text| text.contains("Rename one"));
    s.send(b"\x1b[B");
    s.send(b"\r");
    t.wait_run(
        &["display-message", "-p", "-t", "mn", "#W"],
        "renamed by Enter",
        name("two"),
    );
    open(&t);
    s.wait_for("the menu a third time", |text| text.contains("Rename one"));
    s.send(b"\r");
    t.wait_run(
        &["display-message", "-p", "-t", "mn", "#W"],
        "the first item",
        name("one"),
    );
}

#[test]
fn lock_hands_the_keys_to_the_lock_command() {
    let t = Tmxr::new("lock");
    let s = t.attach(&["new", "-s", "lk"]);
    s.wait_for("status line", |text| text.contains("lk"));
    // Prints a marker, then waits for a line from the terminal, after a
    // pause: the keys arrive while nothing but the client could read them,
    // which is when a client still reading would take them.
    let lock = if cfg!(windows) {
        "echo LOCKED-NOW& ping -n 3 127.0.0.1 >nul& set /p x="
    } else {
        "echo LOCKED-NOW; sleep 2; read x"
    };
    t.run(&["set-option", "-g", "lock-command", lock]);
    t.run(&["lock-session", "-t", "lk"]);
    s.wait_for("the lock command", |text| text.contains("LOCKED-NOW"));
    // The lock command gets the line, so it exits and the client is back.
    s.send(b"secret\r");
    s.wait_for("the client back", |text| {
        text.contains("lk") && !text.contains("LOCKED-NOW")
    });
    let pane = t.run(&["capture-pane", "-p", "-t", "lk"]);
    assert!(
        !pane.contains("secret"),
        "the pane got the lock's keys:\n{pane}"
    );

    // A lock command that fails says so, and the client is back.
    t.run(&["set-option", "-g", "lock-command", "exit 3"]);
    t.run(&["lock-client"]);
    t.wait_run(&["show-messages"], "the failure", |o| o.contains("exit 3"));
    t.run(&["set-option", "-g", "lock-command", ""]);
    assert!(!t.output(&["lock-server"]).status.success());
}

#[test]
fn resize_window_fixes_the_size_until_window_size_latest() {
    let t = Tmxr::new("resizew");
    let s = t.attach(&["new", "-s", "rw"]);
    s.wait_for("status line", |text| text.contains("rw"));
    let size = |t: &Tmxr| {
        t.run(&[
            "display-message",
            "-p",
            "-t",
            "rw",
            "#{window_width}x#{window_height}",
        ])
        .trim()
        .to_owned()
    };
    let auto = size(&t);
    t.run(&["resize-window", "-t", "rw", "-x", "40", "-y", "10"]);
    assert_eq!(size(&t), "40x10");
    t.run(&["resize-window", "-t", "rw", "-R", "5"]);
    t.run(&["resize-window", "-t", "rw", "-U"]);
    assert_eq!(size(&t), "45x9");
    // The pane follows, and the rest of the screen is filled.
    assert_eq!(
        t.run(&[
            "display-message",
            "-p",
            "-t",
            "rw",
            "#{pane_width}x#{pane_height}"
        ])
        .trim(),
        "45x9"
    );
    s.wait_for("the fill", |text| {
        text.lines().nth(3).is_some_and(|l| l.contains("···"))
    });
    // A client resize leaves a manual size alone.
    s.resize(20, 70);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(size(&t), "45x9");
    // -A takes the attached client's size; window-size latest goes back to
    // following it.
    t.run(&["resize-window", "-t", "rw", "-A"]);
    assert_eq!(size(&t), "70x19");
    t.run(&["resize-window", "-t", "rw", "-x", "20"]);
    t.run(&["set-window-option", "-t", "rw", "window-size", "latest"]);
    assert_eq!(size(&t), "70x19");
    assert_ne!(auto, "70x19", "the client resize took effect");
    assert!(
        !t.output(&["set-window-option", "-t", "rw", "window-size", "largest"])
            .status
            .success()
    );
}

#[test]
fn link_window_shows_one_window_in_two_sessions() {
    let t = Tmxr::new("linkw");
    t.run(&["new-session", "-d", "-s", "a"]);
    t.run(&["new-window", "-d", "-t", "a", "-n", "shared"]);
    t.run(&["new-session", "-d", "-s", "b"]);
    let fmt = |t: &Tmxr, target: &str, f: &str| {
        t.run(&["display-message", "-p", "-t", target, f])
            .trim()
            .to_owned()
    };
    // The window names, from list-windows' `0: name* (1 panes) [WxH]`.
    let windows = |t: &Tmxr, s: &str| {
        t.run(&["list-windows", "-t", s])
            .lines()
            .filter_map(|l| l.split_whitespace().nth(1))
            .map(|n| n.trim_end_matches(['*', '-', 'Z', '#', '!']))
            .collect::<Vec<_>>()
            .join(",")
    };

    t.run(&["link-window", "-d", "-s", "a:shared", "-t", "b:"]);
    assert!(windows(&t, "b").contains("shared"), "{}", windows(&t, "b"));
    assert_eq!(
        fmt(&t, "a:shared", "#{window_id}"),
        fmt(&t, "b:shared", "#{window_id}")
    );
    assert_eq!(fmt(&t, "b:shared", "#{window_linked}"), "1");
    // Shown in b, its formats are b's (the status line's #S), not a's.
    let screen = t.attach(&["attach", "-t", "b"]);
    t.run(&["select-window", "-t", "b:shared"]);
    t.run(&["set-option", "-g", "status-left", "[S=#S]"]);
    screen.wait_for("b's name on the status line", |text| text.contains("[S=b]"));
    drop(screen);
    // One window: a rename shows in both.
    t.run(&["rename-window", "-t", "b:shared", "both"]);
    assert!(windows(&t, "a").contains("both"));
    assert!(
        !t.output(&["link-window", "-s", "a:both", "-t", "b:"])
            .status
            .success(),
        "linked twice"
    );

    // Unlinking leaves it in the other session; the last link needs -k.
    t.run(&["unlink-window", "-t", "b:both"]);
    assert!(!windows(&t, "b").contains("both"));
    assert!(windows(&t, "a").contains("both"));
    assert_eq!(fmt(&t, "a:both", "#{window_linked}"), "0");
    assert!(
        !t.output(&["unlink-window", "-t", "a:both"])
            .status
            .success()
    );

    // A killed session's linked window lives on in the other.
    t.run(&["link-window", "-d", "-s", "a:both", "-t", "b:"]);
    t.run(&["kill-session", "-t", "a"]);
    assert!(windows(&t, "b").contains("both"));
    assert_eq!(fmt(&t, "b:both", "#{window_linked}"), "0");

    // -k into a one-window session replaces its window, not the session.
    t.run(&["new-session", "-d", "-s", "c", "-n", "lone"]);
    let lone = fmt(&t, "c:lone", "#{window_index}");
    t.run(&[
        "link-window",
        "-k",
        "-s",
        "b:both",
        "-t",
        &format!("c:{lone}"),
    ]);
    assert_eq!(windows(&t, "c").trim(), "both");
    t.run(&["new-session", "-d", "-s", "d", "-n", "alone"]);
    let alone = fmt(&t, "d:alone", "#{window_index}");
    t.run(&[
        "move-window",
        "-k",
        "-s",
        "c:both",
        "-t",
        &format!("d:{alone}"),
    ]);
    assert_eq!(windows(&t, "d").trim(), "both");
    // unlink-window -k kills the last link.
    t.run(&["new-window", "-d", "-t", "d", "-n", "spare"]);
    t.run(&["unlink-window", "-k", "-t", "d:both"]);
    assert_eq!(windows(&t, "d").trim(), "spare");
}

#[test]
fn display_popup_runs_a_command_over_the_panes() {
    let t = Tmxr::new("popup");
    let s = t.attach(&["new", "-s", "pp"]);
    s.wait_for("status line", |text| text.contains("pp"));
    let read_line = if cfg!(windows) {
        "echo POPUP-UP& set /p x="
    } else {
        "echo POPUP-UP; read x"
    };
    // -E: the keys go to the popup's command, which closes it by exiting.
    t.run(&[
        "display-popup",
        "-E",
        "-w",
        "40",
        "-h",
        "8",
        "-T",
        "Pop",
        read_line,
    ]);
    s.wait_for("the popup", |text| {
        text.contains("POPUP-UP") && text.contains("Pop")
    });
    s.send(b"typed\r");
    s.wait_for("the popup closed", |text| !text.contains("POPUP-UP"));
    let pane = t.run(&["capture-pane", "-p", "-t", "pp"]);
    assert!(
        !pane.contains("typed"),
        "the pane got the popup's keys:\n{pane}"
    );

    // Without -E it stays after its command, until a key.
    t.run(&["display-popup", "echo DONE-HERE"]);
    s.wait_for("the finished popup", |text| text.contains("DONE-HERE"));
    std::thread::sleep(Duration::from_millis(300));
    assert!(s.text().contains("DONE-HERE"), "closed by itself");
    s.send(b"q");
    s.wait_for("closed by a key", |text| !text.contains("DONE-HERE"));

    // -C closes a running popup and ends its command.
    let marker = t.dir.path().join("late-marker");
    let late = if cfg!(windows) {
        format!(
            "echo RUNNING& ping -n 3 127.0.0.1 >nul& echo x> {}",
            marker.display()
        )
    } else {
        format!("echo RUNNING; sleep 2; touch '{}'", marker.display())
    };
    t.run(&["display-popup", &late]);
    s.wait_for("the running popup", |text| text.contains("RUNNING"));
    t.run(&["display-popup", "-C"]);
    s.wait_for("closed by -C", |text| !text.contains("RUNNING"));
    std::thread::sleep(Duration::from_secs(4));
    assert!(!marker.exists(), "the popup's command outlived it");
}

#[test]
fn customize_mode_edits_an_option_through_the_prompt() {
    let t = Tmxr::new("customize");
    let s = t.attach(&["new", "-s", "cu"]);
    s.wait_for("status line", |text| text.contains("cu"));
    t.run(&["customize-mode", "-f", "mode-keys"]);
    s.wait_for("the option", |text| text.contains("option mode-keys vi"));
    s.send(b"\r");
    s.wait_for("the prompt", |text| {
        text.contains(":set-option -g mode-keys vi")
    });
    // Replace "vi" with "emacs" and run it.
    s.send(b"\x7f\x7femacs\r");
    t.wait_run(
        &["show-options", "-v", "mode-keys"],
        "the edited option",
        |o| o.trim() == "emacs",
    );

    // A bind comes back as its bind-key line, formats unexpanded.
    t.run(&["customize-mode", "-f", "pane_current_path"]);
    s.wait_for("a bind", |text| text.contains("bind-key"));
    s.send(b"\r");
    s.wait_for("the bind in the prompt", |text| {
        text.lines()
            .any(|l| l.starts_with(":bind-key") && l.contains("#{pane_current_path}"))
    });
    s.send(b"\x1b");
}

#[test]
fn monitor_activity_and_silence_flag_windows_out_of_sight() {
    let t = Tmxr::new("monitor");
    t.run(&["new-session", "-d", "-s", "al"]);
    t.run(&["new-window", "-d", "-t", "al:1"]);
    t.run(&["new-window", "-d", "-t", "al:2"]);
    let flags = |t: &Tmxr, w: &str| {
        t.run(&[
            "display-message",
            "-p",
            "-t",
            &format!("al:{w}"),
            "#{window_flags}",
        ])
        .trim()
        .to_owned()
    };
    // Off by default: output alone flags nothing.
    t.wait_prompt("al:1");
    t.run(&["send-keys", "-t", "al:1", "echo quiet-one", "Enter"]);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!flags(&t, "1").contains('#'), "{}", flags(&t, "1"));

    t.run(&["set-window-option", "-g", "monitor-activity", "on"]);
    assert_eq!(
        t.run(&["show-options", "-v", "monitor-activity"]).trim(),
        "on"
    );
    t.run(&["send-keys", "-t", "al:1", "echo busy-one", "Enter"]);
    t.wait_run(
        &[
            "display-message",
            "-p",
            "-t",
            "al:1",
            "#{window_activity_flag}",
        ],
        "activity",
        |o| o.trim() == "1",
    );
    assert!(flags(&t, "1").contains('#'));
    // The current window is in sight: no flag.
    t.wait_prompt("al:0");
    t.run(&["send-keys", "-t", "al:0", "echo here", "Enter"]);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!flags(&t, "0").contains('#'), "{}", flags(&t, "0"));
    // next-window -a goes to it, and selecting it clears the flag.
    t.run(&["next-window", "-a", "-t", "al"]);
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "al", "#I"]).trim(),
        "1"
    );
    assert!(!flags(&t, "1").contains('#'));

    // monitor-silence on one window: flagged once quiet that long.
    t.run(&["set-window-option", "-t", "al:2", "monitor-silence", "1"]);
    t.wait_run(
        &["display-message", "-p", "-t", "al:2", "#{window_flags}"],
        "silence",
        |o| o.contains('~'),
    );
    assert!(!flags(&t, "0").contains('~'), "only the window that asked");
}

#[test]
fn find_window_matches_names_and_pane_contents() {
    let t = Tmxr::new("findw");
    let s = t.attach(&["new", "-s", "fw", "-n", "alpha"]);
    s.wait_for("status line", |text| text.contains("fw"));
    t.run(&["new-window", "-d", "-t", "fw", "-n", "beta"]);
    t.wait_prompt("fw:beta");
    t.run(&["send-keys", "-t", "fw:beta", "echo needle-in-pane", "Enter"]);
    t.wait_run(
        &["capture-pane", "-p", "-t", "fw:beta"],
        "the output",
        |o| o.lines().any(|l| l.trim() == "needle-in-pane"),
    );
    let found = |t: &Tmxr, s: &Screen, args: &[&str], want: &'static str| {
        let mut cmd = vec!["find-window"];
        cmd.extend_from_slice(args);
        // The prompt's own command line must not match: run from outside.
        t.run(&["send-keys", "-t", "fw:alpha", "C-l"]);
        let out = t.output(&cmd);
        assert!(out.status.success(), "{cmd:?}: {out:?}");
        s.wait_for(want, |text| text.contains(want));
        s.send(b"\x1b");
        s.wait_for("the picker closed", |text| !text.contains("windows "));
    };
    found(&t, &s, &["needle-in"], "windows 1/1");
    found(&t, &s, &["-N", "needle-in"], "windows 0/0");
    found(&t, &s, &["ALPHA"], "windows 0/0");
    found(&t, &s, &["-i", "ALPHA"], "windows 1/1");
    found(&t, &s, &["-r", "need.e-in-p"], "windows 1/1");
    assert!(!t.output(&["find-window", "-r", "("]).status.success());
}

#[test]
fn rgb_colour_off_sends_the_256_colours() {
    let t = Tmxr::new("rgb");
    let s = t.attach(&["new", "-s", "rg"]);
    s.wait_for("status line", |text| text.contains("rg"));
    let rgb_cells = |s: &Screen| {
        s.status_cells()
            .iter()
            .filter(|(_, fg, bg)| fg.starts_with("Rgb") || bg.starts_with("Rgb"))
            .count()
    };
    assert!(
        rgb_cells(&s) > 0,
        "the theme's status line is 24-bit by default"
    );
    t.run(&["set-option", "-g", "rgb-colour", "off"]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while rgb_cells(&s) > 0 {
        assert!(
            Instant::now() < deadline,
            "still RGB: {:?}",
            s.status_cells()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        s.status_cells()
            .iter()
            .any(|(_, _, bg)| bg.starts_with("Idx")),
        "{:?}",
        s.status_cells()
    );
    assert!(
        !t.output(&["set-option", "-g", "rgb-colour", "maybe"])
            .status
            .success()
    );
}

#[test]
fn rgb_colour_auto_follows_the_client_environment() {
    let t = Tmxr::new("rgbauto");
    t.run(&["new-session", "-d", "-s", "ra"]);
    t.run(&["set-option", "-g", "rgb-colour", "auto"]);
    let rgb_cells = |s: &Screen| {
        s.status_cells()
            .iter()
            .filter(|(_, fg, bg)| fg.starts_with("Rgb") || bg.starts_with("Rgb"))
            .count()
    };
    let plain = [
        ("COLORTERM", None),
        ("WT_SESSION", None),
        ("TERM", Some("xterm-256color")),
    ];
    let s = t.attach_env(&["attach", "-t", "ra"], &plain);
    s.wait_for("status line", |text| text.contains("ra"));
    assert_eq!(rgb_cells(&s), 0, "{:?}", s.status_cells());
    drop(s);
    let true_colour = [("COLORTERM", Some("truecolor")), ("WT_SESSION", None)];
    let s = t.attach_env(&["attach", "-t", "ra"], &true_colour);
    s.wait_for("status line", |text| text.contains("ra"));
    assert!(rgb_cells(&s) > 0, "{:?}", s.status_cells());
}

#[test]
fn resurrect_matches_command_lines_as_tmux_resurrect() {
    // A program with arguments; on Windows the pane's cmd runs it as a child.
    #[cfg(unix)]
    let (program, start) = ("sleep", vec!["sleep", "300"]);
    #[cfg(windows)]
    let (program, start) = (
        "PING",
        vec!["cmd.exe", "/d", "/c", "ping -n 300 127.0.0.1 >NUL"],
    );
    let t = Tmxr::new("resmatch");
    let base = std::fs::read_to_string(&t.config).unwrap();
    let mut new = vec!["new-session", "-d", "-s", "main"];
    new.extend(&start);
    t.run(&new);
    t.wait_run(
        &[
            "display-message",
            "-p",
            "-t",
            "main",
            "#{pane_current_command}",
        ],
        "the program running",
        |o| o.trim().eq_ignore_ascii_case(program),
    );
    let save_with = |t: &Tmxr, processes: &str| {
        std::fs::write(
            &t.config,
            format!(
                "{base}processes = [{processes}]
"
            ),
        )
        .unwrap();
        t.run(&["source-file"]);
        t.run(&["resurrect-save"]);
        t.newest_save()
    };
    // ~: a command line holding the text, restored whole.
    #[cfg(unix)]
    let (anywhere, whole) = ("\"~eep 30\"", vec!["300"]);
    #[cfg(windows)]
    let (anywhere, whole) = ("\"~300 127.0\"", vec!["-n", "300", "127.0.0.1"]);
    assert_eq!(saved_args(&save_with(&t, anywhere)), whole);
    // ->: a command line starting with the match, restored as the command.
    #[cfg(unix)]
    let (swap, restored) = ("\"sleep 300->sleep 'a b'\"", vec!["a b"]);
    #[cfg(windows)]
    let (swap, restored) = ("\"ping -n 300->ping -n 'a b'\"", vec!["-n", "a b"]);
    assert_eq!(saved_args(&save_with(&t, swap)), restored);
    // Neither matches: nothing restored.
    let none = save_with(&t, "\"~not-in-it\", \"zzz->x\"");
    assert!(!none.contains("\"command\": \""), "{none}");
}

#[test]
fn run_shell_holds_its_command_list_and_reply() {
    let t = Tmxr::new("runshell");
    t.run(&["new-session", "-d", "-s", "rs"]);
    // A command client gets the output, and the commands after it wait.
    let out = t.run(&[
        "run-shell",
        "echo from-shell",
        ";",
        "display-message",
        "-p",
        "after",
    ]);
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    assert_eq!(lines, ["from-shell", "after"], "{out:?}");
    // -d delays, here with no command at all: the rest of the list waits.
    let started = Instant::now();
    t.run(&[
        "run-shell",
        "-d",
        "0.5",
        ";",
        "set-option",
        "-g",
        "@after",
        "yes",
    ]);
    assert!(
        started.elapsed() >= Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(t.run(&["display-message", "-p", "#{@after}"]).trim(), "yes");
    // -C runs a tmux command; -c sets the shell's directory.
    t.run(&["run-shell", "-C", "set-option -g @ran yes"]);
    assert_eq!(t.run(&["display-message", "-p", "#{@ran}"]).trim(), "yes");
    let dir = t.dir.path().join("there");
    std::fs::create_dir(&dir).unwrap();
    let pwd = if cfg!(windows) { "cd" } else { "pwd" };
    let shown = t.run(&["run-shell", "-c", dir.to_str().unwrap(), pwd]);
    assert!(shown.trim().ends_with("there"), "{shown:?}");
    // -b does not wait.
    let started = Instant::now();
    t.run(&["run-shell", "-b", "-d", "3", "set-option -g @late yes"]);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    // A failing command says so.
    assert!(t.run(&["run-shell", "exit 3"]).contains("returned 3"));
    // From a bind: the output shows on the client, then the rest runs.
    let s = t.attach(&["attach", "-t", "rs"]);
    s.wait_for("status line", |text| text.contains("rs"));
    t.run(&[
        "bind-key",
        "-n",
        "F7",
        "run-shell 'echo bound-out' ; set-option -g @bound yes",
    ]);
    s.send(b"[18~");
    s.wait_for("the output", |text| text.contains("bound-out"));
    t.wait_run(
        &["display-message", "-p", "#{@bound}"],
        "the rest of the bind",
        |o| o.trim() == "yes",
    );
}

#[test]
fn read_only_clients_look_but_do_not_touch() {
    let t = Tmxr::new("readonly");
    t.run(&["new-session", "-d", "-s", "ro"]);
    let s = t.attach(&["attach", "-r", "-t", "ro"]);
    s.wait_for("status line", |text| text.contains("ro"));
    // Typing reaches no pane.
    s.send(b"echo ro-typed\r");
    std::thread::sleep(Duration::from_millis(500));
    let pane = t.run(&["capture-pane", "-p", "-t", "ro"]);
    assert!(!pane.contains("ro-typed"), "{pane}");
    // A bind that changes something is refused, and says so.
    s.send(PREFIX);
    s.send(b"c");
    s.wait_for("the refusal", |text| text.contains("read-only"));
    assert_eq!(t.run(&["list-windows", "-t", "ro"]).lines().count(), 1);
    // Detaching still works.
    s.send(PREFIX);
    s.send(b"d");
    s.wait_for("detached", |text| text.contains("[detached"));
}

#[test]
fn server_access_lists_who_may_connect() {
    let t = Tmxr::new("access");
    let me = std::env::var(if cfg!(windows) { "USERNAME" } else { "USER" }).ok();
    let other = if cfg!(windows) { "Guest" } else { "nobody" };
    // The default socket-access keeps everyone else out: -a says why.
    t.run(&["new-session", "-d", "-s", "own"]);
    let refused = t.output(&["server-access", "-a", other]);
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("socket-access"),
        "{refused:?}"
    );
    assert!(
        !t.output(&["set-option", "-g", "socket-access", "users"])
            .status
            .success()
    );
    t.run(&["kill-server"]);

    // socket-access users, on a socket outside tmxr's private directory.
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(&t.config, format!("socket-access = \"users\"\n{config}")).unwrap();
    let sock = if cfg!(windows) {
        format!(r"\\.\pipe\tmxr-e2e-access-{}", std::process::id())
    } else {
        t.dir
            .path()
            .join("shared")
            .join("tmxr.sock")
            .display()
            .to_string()
    };
    let at = |rest: &[&str]| t.output_at(&sock, rest);
    let started = at(&["new-session", "-d", "-s", "sa"]);
    assert!(started.status.success(), "{started:?}");
    let list = || String::from_utf8_lossy(&at(&["server-access", "-l"]).stdout).into_owned();
    assert_eq!(list(), "");
    if let Some(me) = &me {
        let own = at(&["server-access", "-a", me]);
        assert!(
            String::from_utf8_lossy(&own.stderr).contains("owns the server"),
            "{own:?}"
        );
    }
    assert!(at(&["server-access", "-a", other]).status.success());
    assert_eq!(list().trim(), format!("{other} (W)"));
    assert!(at(&["server-access", "-r", other]).status.success());
    assert_eq!(list().trim(), format!("{other} (R)"));
    assert!(at(&["server-access", "-w", other]).status.success());
    assert_eq!(list().trim(), format!("{other} (W)"));
    assert!(at(&["server-access", "-d", other]).status.success());
    assert_eq!(list(), "");
    assert!(!at(&["server-access", "-d", other]).status.success());
    assert!(
        !at(&["server-access", "-a", "no-such-user-tmxr"])
            .status
            .success()
    );
    at(&["kill-server"]);

    // A label's socket is in tmxr's private directory on Unix: -a says so.
    if cfg!(unix) {
        t.run(&["new-session", "-d", "-s", "lbl"]);
        let private = t.output(&["server-access", "-a", other]);
        assert!(
            String::from_utf8_lossy(&private.stderr).contains("-S"),
            "{private:?}"
        );
    }
}

#[test]
fn prefix_y_copies_the_shells_command_line() {
    let t = Tmxr::new("yankline");
    // A shell with line editing: dash (Ubuntu's /bin/sh) has none.
    let (new, shell) = if cfg!(windows) {
        (vec!["new", "-s", "yk"], "cmd")
    } else {
        (
            vec!["new", "-s", "yk", "bash --norc --noprofile -i"],
            "bash",
        )
    };
    let s = t.attach(&new);
    s.wait_for("status line", |text| text.contains("yk"));
    t.wait_run(
        &[
            "display-message",
            "-p",
            "-t",
            "yk",
            "#{pane_current_command}",
        ],
        "the shell",
        |o| o.trim().eq_ignore_ascii_case(shell),
    );
    // Typed, not run, with the cursor left in the middle of it.
    s.send(b"echo yank-me-now");
    s.wait_for("the typing", |text| text.contains("echo yank-me-now"));
    for _ in 0..4 {
        s.send(b"\x1b[D");
    }
    s.send(PREFIX);
    s.send(b"y");
    t.wait_run(&["show-buffer"], "the command line copied", |o| {
        o == "echo yank-me-now"
    });
}

#[test]
fn prefix_y_uses_the_shells_prompt_mark() {
    let t = Tmxr::new("yankmark");
    let (new, shell, mark) = if cfg!(windows) {
        (vec!["new", "-s", "ym"], "cmd", r"prompt $P$G$e]133;B$e\")
    } else {
        (
            vec!["new", "-s", "ym", "bash --norc --noprofile -i"],
            "bash",
            r"PS1='$ \[\e]133;B\a\]'",
        )
    };
    let s = t.attach(&new);
    s.wait_for("status line", |text| text.contains("ym"));
    t.wait_run(
        &[
            "display-message",
            "-p",
            "-t",
            "ym",
            "#{pane_current_command}",
        ],
        "the shell",
        |o| o.trim().eq_ignore_ascii_case(shell),
    );
    // The prompt now ends with an OSC 133;B mark.
    s.send(mark.as_bytes());
    s.send(b"\r");
    std::thread::sleep(Duration::from_millis(500));
    s.send(b"echo yank-me-now");
    s.wait_for("the typing", |text| text.contains("echo yank-me-now"));
    for _ in 0..4 {
        s.send(b"\x1b[D");
    }
    s.send(PREFIX);
    s.send(b"y");
    t.wait_run(&["show-buffer"], "the command line copied", |o| {
        o == "echo yank-me-now"
    });
    // With the mark no keys were sent: the cursor is still mid-word.
    s.send(b"Z");
    s.wait_for("Z typed where the cursor was", |text| {
        text.contains("echo yank-meZ-now")
    });
}

#[test]
fn display_popup_stays_inside_a_resized_client() {
    let t = Tmxr::new("popupfit");
    let s = t.attach(&["new", "-s", "pf"]);
    s.wait_for("status line", |text| text.contains("pf"));
    // Kept running through the resize (cmd's pause would end on it).
    let show = if cfg!(windows) {
        "echo POPUP-READY& ping -n 30 127.0.0.1 >NUL"
    } else {
        "echo POPUP-READY; sleep 30"
    };
    // A wide box at the right edge.
    t.run(&[
        "display-popup",
        "-E",
        "-x",
        "40",
        "-w",
        "56",
        "-h",
        "8",
        show,
    ]);
    s.wait_for("the popup", |text| text.contains("POPUP-READY"));
    // The client shrinks to less than the popup's width.
    s.resize(20, 50);
    // Row by row: full-width rows would run together in the screen's text.
    let row_text = |s: &Screen, row: u16| -> String {
        s.row_cells(row).into_iter().map(|(c, _, _)| c).collect()
    };
    let popup_row = |s: &Screen| (0..20).find(|&r| row_text(s, r).contains("POPUP-READY"));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        // The box moved in to the left edge and fits the new width.
        if let Some(r) = popup_row(&s)
            && row_text(&s, r).starts_with("│POPUP-READY")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "popup not moved in:
{}",
            s.text()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn resurrect_restores_a_linked_window_as_one_window() {
    let t = Tmxr::new("reslink");
    t.run(&["new-session", "-d", "-s", "a"]);
    t.run(&["new-window", "-d", "-t", "a:1", "-n", "shared"]);
    t.run(&["new-session", "-d", "-s", "b", "-n", "own"]);
    t.run(&["link-window", "-d", "-s", "a:shared", "-t", "b:5"]);
    // In c the linked window is the first one, which a session is made with.
    t.run(&["new-session", "-d", "-s", "c", "-n", "later"]);
    t.run(&["move-window", "-s", "c:0", "-t", "c:9"]);
    t.run(&["link-window", "-d", "-s", "a:shared", "-t", "c:0"]);
    t.run(&["resurrect-save"]);
    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);

    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        config.replace("restore-on-start = false", "restore-on-start = true"),
    )
    .unwrap();
    t.run(&["new-session", "-d", "-s", "other"]);
    let id = |target: &str| {
        t.run(&["display-message", "-p", "-t", target, "#{window_id}"])
            .trim()
            .to_owned()
    };
    let shared = id("a:shared");
    assert!(!shared.is_empty());
    assert_eq!(id("b:5"), shared, "b's link");
    assert_eq!(id("c:0"), shared, "c's first window");
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "c:0", "#{window_linked}"])
            .trim(),
        "1"
    );
    // c kept its own window, and no copy of the shared one exists.
    let windows = |s: &str| t.run(&["list-windows", "-t", s]).lines().count();
    assert_eq!(windows("c"), 2);
    assert_eq!(windows("b"), 2);
    assert!(!id("c:9").is_empty());
}

#[test]
fn resurrect_keeps_a_manual_window_size() {
    let t = Tmxr::new("ressize");
    t.run(&["new-session", "-d", "-s", "rz"]);
    t.run(&["resize-window", "-t", "rz", "-x", "41", "-y", "11"]);
    t.run(&["resurrect-save"]);
    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        config.replace("restore-on-start = false", "restore-on-start = true"),
    )
    .unwrap();
    t.run(&["new-session", "-d", "-s", "other"]);
    // A client attached would size an automatic window to itself.
    let s = t.attach(&["attach", "-t", "rz"]);
    s.wait_for("status line", |text| text.contains("rz"));
    std::thread::sleep(Duration::from_millis(300));
    let size = t.run(&[
        "display-message",
        "-p",
        "-t",
        "rz",
        "#{window_width}x#{window_height}",
    ]);
    assert_eq!(size.trim(), "41x11");
}

#[test]
fn lock_after_time_locks_an_idle_client() {
    let t = Tmxr::new("lockidle");
    let s = t.attach(&["new", "-s", "li"]);
    s.wait_for("status line", |text| text.contains("li"));
    let lock = if cfg!(windows) {
        "echo IDLE-LOCKED& set /p x="
    } else {
        "echo IDLE-LOCKED; read x"
    };
    t.run(&["set-option", "-g", "lock-command", lock]);
    t.run(&["set-option", "-g", "lock-after-time", "1"]);
    // Nothing typed: locked by itself.
    s.wait_for("the idle lock", |text| text.contains("IDLE-LOCKED"));
    s.send(b"unlock\r");
    s.wait_for("the client back", |text| {
        text.contains("li") && !text.contains("IDLE-LOCKED")
    });
    // Unlocking counts as activity: not locked again at once.
    std::thread::sleep(Duration::from_millis(500));
    assert!(!s.text().contains("IDLE-LOCKED"), "locked again at once");
    t.run(&["set-option", "-g", "lock-after-time", "0"]);
}

#[test]
fn dash_e_sets_variables_for_new_panes_and_sessions() {
    let t = Tmxr::new("envflag");
    // Prints both variables, then stays open.
    let show = if cfg!(windows) {
        "cmd /k echo A=%TMXR_A% B=%TMXR_B%"
    } else {
        "sh -c 'echo A=$TMXR_A B=$TMXR_B; sleep 30'"
    };
    // Each command its own values: the session's (from new-session -e)
    // reach every later pane too, and must not pass for the command's.
    let printed = |t: &Tmxr, target: &str, want: &'static str| {
        t.wait_run(&["capture-pane", "-p", "-t", target], want, |o| {
            o.contains(want)
        });
    };
    let with = |a: &'static str, b: &'static str, cmd: &[&'static str]| {
        let mut args = cmd.to_vec();
        args.extend(["-e", a, "-e", b, show]);
        t.run(&args);
    };
    with("TMXR_A=1", "TMXR_B=2", &["new-session", "-d", "-s", "ev"]);
    printed(&t, "ev", "A=1 B=2");
    let shown = t.run(&["show-environment", "-t", "ev"]);
    assert!(
        shown.contains("TMXR_A=1") && shown.contains("TMXR_B=2"),
        "{shown}"
    );
    with("TMXR_A=3", "TMXR_B=4", &["new-window", "-d", "-t", "ev:5"]);
    printed(&t, "ev:5", "A=3 B=4");
    with(
        "TMXR_A=5",
        "TMXR_B=6",
        &["split-window", "-d", "-t", "ev:5"],
    );
    printed(&t, "ev:5.1", "A=5 B=6");
    with(
        "TMXR_A=7",
        "TMXR_B=8",
        &["respawn-pane", "-k", "-t", "ev:5.1"],
    );
    printed(&t, "ev:5.1", "A=7 B=8");
    assert!(
        !t.output(&["new-window", "-d", "-e", "NOEQUALS"])
            .status
            .success()
    );
}

#[test]
fn pipe_pane_input_types_the_commands_output() {
    let t = Tmxr::new("pipein");
    t.run(&["new-session", "-d", "-s", "pi"]);
    // The pipe's command prints a command line; the pane's shell runs it.
    t.run(&["pipe-pane", "-I", "-t", "pi", "echo echo from-the-pipe"]);
    t.wait_run(
        &["capture-pane", "-p", "-t", "pi"],
        "the typed command's output",
        |o| o.lines().any(|l| l.trim_end() == "from-the-pipe"),
    );
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "pi", "#{pane_pipe}"])
            .trim(),
        "1"
    );
}

#[test]
fn menu_and_popup_take_tmux_style_flags() {
    let t = Tmxr::new("looks");
    let s = t.attach(&["new", "-s", "lk"]);
    s.wait_for("status line", |text| text.contains("lk"));
    let all_cells = |s: &Screen| -> Vec<(String, String, String)> {
        (0..ROWS).flat_map(|r| s.row_cells(r)).collect()
    };
    // -b double, -S the border's colour, -s the items', -H the selected one.
    t.run(&[
        "display-menu",
        "-b",
        "double",
        "-S",
        "fg=green",
        "-s",
        "bg=red",
        "-H",
        "bg=blue",
        "-T",
        "Look",
        "First",
        "f",
        "rename-window first",
        "Second",
        "s",
        "rename-window second",
    ]);
    s.wait_for("the menu", |text| text.contains("Second"));
    let cells = all_cells(&s);
    let corner = cells
        .iter()
        .find(|(c, _, _)| c == "╔")
        .expect("a double border");
    assert_eq!(corner.1, "Idx(2)", "border fg: {corner:?}");
    // An item's first letter, on the row that holds its name.
    let item = |name: &str| -> (String, String, String) {
        (0..ROWS)
            .find_map(|r| {
                let row = s.row_cells(r);
                let text: String = row.iter().map(|(c, _, _)| c.as_str()).collect();
                text.find(name)?;
                row.into_iter()
                    .find(|(c, _, _)| name.starts_with(c.as_str()) && !c.is_empty())
            })
            .unwrap_or_else(|| panic!("{name} not on the screen"))
    };
    let second = item("Second");
    assert_eq!(second.2, "Idx(1)", "item bg: {second:?}");
    let first = item("First");
    assert_eq!(first.2, "Idx(4)", "selected bg: {first:?}");
    s.send(b"\x1b");
    s.wait_for("the menu closed", |text| !text.contains("Second"));

    // A popup's -b rounded, and -b none for no border.
    let hold = if cfg!(windows) {
        "echo IN-THE-BOX& ping -n 30 127.0.0.1 >NUL"
    } else {
        "echo IN-THE-BOX; sleep 30"
    };
    t.run(&["display-popup", "-b", "rounded", hold]);
    s.wait_for("the rounded popup", |text| {
        text.contains("IN-THE-BOX") && text.contains('╭')
    });
    t.run(&["display-popup", "-C"]);
    s.wait_for("closed", |text| !text.contains("IN-THE-BOX"));
    t.run(&["display-popup", "-b", "none", hold]);
    s.wait_for("the bare popup", |text| text.contains("IN-THE-BOX"));
    assert!(
        !s.text().contains(['╭', '┌', '│']),
        "a border was drawn:\n{}",
        s.text()
    );
    t.run(&["display-popup", "-C"]);
    assert!(
        !t.output(&["display-popup", "-b", "simple", "echo"])
            .status
            .success()
    );
}

#[test]
fn display_popup_takes_tmux_position_letters() {
    let t = Tmxr::new("popplace");
    let s = t.attach(&["new", "-s", "pl"]);
    s.wait_for("status line", |text| text.contains("pl"));
    let hold = if cfg!(windows) {
        "echo PLACED& ping -n 30 127.0.0.1 >NUL"
    } else {
        "echo PLACED; sleep 30"
    };
    // A row's cell symbols, blanks as spaces, indexed by column.
    let row = |s: &Screen, row: u16| -> Vec<String> {
        s.row_cells(row)
            .into_iter()
            .map(|(c, _, _)| if c.is_empty() { " ".into() } else { c })
            .collect()
    };
    // R: the right edge; S: the bottom just above the status line.
    t.run(&[
        "display-popup",
        "-x",
        "R",
        "-y",
        "S",
        "-w",
        "20",
        "-h",
        "5",
        hold,
    ]);
    s.wait_for("the popup", |text| text.contains("PLACED"));
    let status = ROWS - 1;
    let bottom = row(&s, status - 1);
    assert_eq!(bottom[usize::from(COLS - 1)], "┘", "{bottom:?}");
    assert_eq!(bottom[usize::from(COLS - 20)], "└", "{bottom:?}");
    t.run(&["display-popup", "-C"]);
    s.wait_for("closed", |text| !text.contains("PLACED"));
    // A number, after its formats are expanded.
    t.run(&[
        "display-popup",
        "-x",
        "#{pane_index}",
        "-y",
        "3",
        "-w",
        "20",
        "-h",
        "5",
        hold,
    ]);
    s.wait_for("the popup", |text| text.contains("PLACED"));
    assert_eq!(row(&s, 3)[0], "┌", "{:?}", row(&s, 3));
    t.run(&["display-popup", "-C"]);
    assert!(
        !t.output(&["display-popup", "-x", "Q", "echo"])
            .status
            .success()
    );
}

#[test]
fn select_pane_titles_a_pane_and_resurrect_keeps_it() {
    let t = Tmxr::new("titles");
    t.run(&["new-session", "-d", "-s", "ti"]);
    t.wait_settled("ti");
    let title = |t: &Tmxr| {
        t.run(&["display-message", "-p", "-t", "ti", "#{pane_title}"])
            .trim()
            .to_owned()
    };
    t.run(&["select-pane", "-t", "ti", "-T", "my-title-#{session_name}"]);
    assert_eq!(title(&t), "my-title-ti");
    t.run(&["resurrect-save"]);
    assert!(
        t.newest_save().contains("\"title\": \"my-title-ti\""),
        "{}",
        t.newest_save()
    );
    // Restored, until the program titles itself; on Windows ConPTY reports
    // cmd's own title as it starts, so only Unix's sh keeps it.
    if cfg!(unix) {
        t.run(&["kill-server"]);
        t.wait_run(&["ls"], "server gone", str::is_empty);
        let config = std::fs::read_to_string(&t.config).unwrap();
        std::fs::write(
            &t.config,
            config.replace("restore-on-start = false", "restore-on-start = true"),
        )
        .unwrap();
        t.run(&["new-session", "-d", "-s", "other"]);
        assert_eq!(title(&t), "my-title-ti");
    }
}
