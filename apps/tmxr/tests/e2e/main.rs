//! End-to-end: the real `tmxr` binary in a real pseudo-terminal (a ConPTY on
//! Windows), driven with keystrokes and checked by what its screen shows.
//!
//! Every test gets its own socket label, socket directory and XDG roots, so
//! nothing touches a running server or the user's config and saved sessions.
//! The harness is here; the tests are in the modules below, by area.

mod buffers_and_shell;
mod copy_mode;
mod input;
mod overlays;
mod panes;
mod resurrect;
mod sessions;
mod status_and_options;
mod windows;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
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
        // No assume-paste-time: a test types a whole key sequence in one
        // write, which tmux's would take as pasted text.
        std::fs::write(
            &config,
            format!("default-shell = {shell:?}\nset-clipboard = \"off\"\nassume-paste-time = 0\n[resurrect]\nrestore-on-start = false\nauto-save-minutes = 0\n"),
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
            // Over SSH tmxr leaves the desktop's clipboard alone
            // (`util::local_clipboard`): a test that copies must not
            // overwrite it.
            ("SSH_CONNECTION".to_owned(), String::new()),
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
        tmxr_term::child::command(env!("CARGO_BIN_EXE_tmxr"))
            .args(self.args(rest))
            .envs(self.env())
            .output()
            .unwrap()
    }

    /// Run a command client against the server at `socket` (`-S`) in place
    /// of the test's label.
    fn output_at(&self, socket: &str, rest: &[&str]) -> std::process::Output {
        tmxr_term::child::command(env!("CARGO_BIN_EXE_tmxr"))
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

    /// [`Tmxr::attach`] run in directory `cwd`.
    fn attach_in(&self, cwd: &Path, rest: &[&str]) -> Screen {
        self.spawn_at(cwd, env!("CARGO_BIN_EXE_tmxr"), &self.args(rest), &[])
    }

    /// Start `program` in a pseudo-terminal with the test's environment.
    fn spawn(&self, program: &str, args: &[String]) -> Screen {
        self.spawn_env(program, args, &[])
    }

    /// [`Tmxr::spawn`] with variables set or removed, as [`Tmxr::attach_env`].
    fn spawn_env(&self, program: &str, args: &[String], vars: &[(&str, Option<&str>)]) -> Screen {
        self.spawn_at(self.dir.path(), program, args, vars)
    }

    /// [`Tmxr::spawn_env`] in directory `cwd`.
    fn spawn_at(
        &self,
        cwd: &Path,
        program: &str,
        args: &[String],
        vars: &[(&str, Option<&str>)],
    ) -> Screen {
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
        cmd.cwd(cwd);
        let child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let emu = Arc::new(Mutex::new(Emulator::new(ROWS, COLS, 0)));
        let raw = Arc::new(Mutex::new(Vec::new()));
        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let emu = Arc::clone(&emu);
            let writer = Arc::clone(&writer);
            let raw = Arc::clone(&raw);
            let paused = Arc::clone(&paused);
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    while paused.load(std::sync::atomic::Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    let Ok(n) = reader.read(&mut buf) else {
                        break;
                    };
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
            paused,
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
        let _ = tmxr_term::child::command(env!("CARGO_BIN_EXE_tmxr"))
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
    /// While set, the terminal reads nothing, as a stalled one would.
    paused: Arc<std::sync::atomic::AtomicBool>,
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

    /// Stop reading the terminal's output (`true`), or start again.
    fn pause(&self, paused: bool) {
        self.paused
            .store(paused, std::sync::atomic::Ordering::Relaxed);
    }

    /// The title the terminal was last given (OSC 0 / 2).
    fn title(&self) -> String {
        self.emu
            .lock()
            .unwrap()
            .title()
            .unwrap_or_default()
            .to_owned()
    }

    fn send(&self, bytes: &[u8]) {
        let mut w = self.writer.lock().unwrap();
        w.write_all(bytes).unwrap();
        w.flush().unwrap();
    }

    /// Wait until `pred` holds for the screen itself, cells and colours.
    fn wait_for_cells(&self, what: &str, pred: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        while !pred(self) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; status line: {:?}",
                self.status_cells()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
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

/// Set for the copy of this binary [`console_window_probe`] runs as.
#[cfg(windows)]
const CONSOLE_PROBE: &str = "TMXR_E2E_CONSOLE_PROBE";

/// Not a test: run by [`shell_commands_open_no_console_window`] through the
/// server, it prints whether its console has a window.
#[cfg(windows)]
#[test]
#[ignore = "a probe another test runs"]
fn console_window_probe() {
    use windows_sys::Win32::System::Console::GetConsoleWindow;
    if std::env::var_os(CONSOLE_PROBE).is_none() {
        return;
    }
    // SAFETY: takes no arguments and only reads this process's console.
    let window = unsafe { GetConsoleWindow() };
    println!("console-window={}", !window.is_null());
}

/// Where [`key_log_fixture`] logs the keys it reads, when a test starts it.
const KEY_LOG: &str = "TMXR_E2E_KEY_LOG";
/// Set, [`key_log_fixture`] asks for focus reports and logs them too.
const KEY_LOG_FOCUS: &str = "TMXR_E2E_KEY_LOG_FOCUS";

/// This test binary, copied as `name` into the test's directory, made to run
/// [`key_log_fixture`]: a program the navigator sees by that name that
/// records every key it is given, to the file `KEY_LOG` names in its
/// environment.
fn key_log_program(t: &Tmxr, name: &str) -> Vec<String> {
    let me = std::env::current_exe().unwrap();
    let path = t
        .dir
        .path()
        .join(name)
        .with_extension(std::env::consts::EXE_EXTENSION);
    std::fs::copy(me, &path).unwrap();
    vec![
        path.display().to_string(),
        "key_log_fixture".into(),
        "--exact".into(),
        "--ignored".into(),
        "--nocapture".into(),
    ]
}

/// Not a test: the program [`key_log_program`] starts. In raw mode, it logs
/// `ready`, then each key press it reads as `code modifiers`, one a line;
/// with [`KEY_LOG_FOCUS`] set it asks for focus reports and logs each as
/// `FocusGained` or `FocusLost` (and on Unix, no keys).
#[test]
#[ignore = "a fixture other tests run in a pane"]
fn key_log_fixture() {
    use crossterm::event::{EnableFocusChange, Event, KeyEventKind, read};
    let Some(log) = std::env::var_os(KEY_LOG) else {
        return;
    };
    let mut log = std::fs::File::create(log).unwrap();
    crossterm::terminal::enable_raw_mode().unwrap();
    // On Windows the fixture read a focus event it never asked for (in the
    // navigator test), so focus is logged only when the test wants it.
    let focus = std::env::var_os(KEY_LOG_FOCUS).is_some();
    if focus {
        // The marker follows the mode, so a test that sees it on screen
        // knows the server has the mode too.
        crossterm::execute!(std::io::stdout(), EnableFocusChange).unwrap();
        print!("focus-on");
        std::io::stdout().flush().unwrap();
    }
    // On Unix, focus reports are read from stdin as bytes: crossterm's
    // reader lost the first one about half the time on Linux (WSL,
    // 2026-10-18; tmxr had written it), and raw reads lost none in eight
    // runs. On Windows ConPTY turns the reports into console focus events,
    // which only crossterm reads.
    #[cfg(unix)]
    if focus {
        log_focus_bytes(&mut log);
    }
    // Start crossterm's reader before saying so: on Linux, input that came
    // before it existed was lost now and then.
    let _ = crossterm::event::poll(Duration::ZERO);
    writeln!(log, "ready").unwrap();
    loop {
        match read().unwrap() {
            Event::Key(k) if k.kind == KeyEventKind::Press => {
                writeln!(log, "{:?} {:?}", k.code, k.modifiers).unwrap();
            }
            Event::FocusGained if focus => writeln!(log, "FocusGained").unwrap(),
            Event::FocusLost if focus => writeln!(log, "FocusLost").unwrap(),
            _ => {}
        }
    }
}

/// [`key_log_fixture`]'s focus mode on Unix: log `ready`, then each
/// `CSI I` / `CSI O` on stdin as `FocusGained` / `FocusLost`, forever.
#[cfg(unix)]
fn log_focus_bytes(log: &mut std::fs::File) -> ! {
    use std::io::Read as _;
    writeln!(log, "ready").unwrap();
    let mut stdin = std::io::stdin();
    let mut seen = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = stdin.read(&mut buf).unwrap();
        seen.extend_from_slice(&buf[..n]);
        while let Some(i) = seen
            .windows(3)
            .position(|w| w == b"\x1b[I" || w == b"\x1b[O")
        {
            let what = if seen[i + 2] == b'I' {
                "FocusGained"
            } else {
                "FocusLost"
            };
            writeln!(log, "{what}").unwrap();
            seen.drain(..i + 3);
        }
    }
}

/// Make session `nav` a 3×3 grid, panes 0 to 8 in rows from the top left,
/// so each direction from the middle reaches a different pane.
fn pane_grid(t: &Tmxr) {
    for _ in 0..8 {
        t.run(&["split-window", "-t", "nav"]);
        t.run(&["select-layout", "-t", "nav", "tiled"]);
    }
    let mut cells: Vec<(u32, u32, String)> = t
        .run(&[
            "list-panes",
            "-t",
            "nav",
            "-F",
            "#{pane_top} #{pane_left} #{pane_index}",
        ])
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            (
                f[0].parse().unwrap(),
                f[1].parse().unwrap(),
                f[2].to_owned(),
            )
        })
        .collect();
    cells.sort();
    let order: Vec<&str> = cells.iter().map(|c| c.2.as_str()).collect();
    assert_eq!(order, ["0", "1", "2", "3", "4", "5", "6", "7", "8"]);
}

/// Press `keys` (h, j, k and l, as the terminal sends them) in the grid of
/// [`pane_grid`]: each direction out of the middle and back, then off each
/// edge, which wraps to the far side as tmux's does.
fn round_the_grid(t: &Tmxr, s: &Screen, form: &str, keys: [&[u8]; 4]) {
    let [h, j, k, l] = keys;
    t.run(&["select-pane", "-t", "nav:0.4"]);
    for (key, name, want) in [
        (l, "l", "5"),
        (h, "h", "4"),
        (h, "h", "3"),
        (l, "l", "4"),
        (j, "j", "7"),
        (k, "k", "4"),
        (k, "k", "1"),
        (j, "j", "4"),
        // Off an edge: 3 h wraps to 5, 5 l to 3, 1 k to 7, 7 j to 1.
        (h, "h", "3"),
        (h, "h", "5"),
        (l, "l", "3"),
        (l, "l", "4"),
        (k, "k", "1"),
        (k, "k", "7"),
        (j, "j", "1"),
    ] {
        s.send(key);
        t.wait_run(
            &["display-message", "-p", "-t", "nav", "#{pane_index}"],
            &format!("{form}{name} to pane {want}"),
            |o| o.trim() == want,
        );
    }
}

/// C-h / C-j / C-k / C-l as a terminal without kitty keys sends them.
const CTRL_HJKL_BYTES: [&[u8]; 4] = [b"\x08", b"\x0a", b"\x0b", b"\x0c"];

/// The Tokyo Night palette as `Debug` text of a vt100 colour.
fn rgb(hex: u32) -> String {
    format!("Rgb({}, {}, {})", hex >> 16, (hex >> 8) & 0xff, hex & 0xff)
}

/// Whether every colour on `s`'s status line is one of the 16 ANSI ones
/// (or the default), and some cell has one.
fn status_in_16_colours(s: &Screen) -> bool {
    let cells = s.status_cells();
    let in_16 = |c: &str| {
        c == "Default"
            || c.strip_prefix("Idx(")
                .and_then(|n| n.strip_suffix(')'))
                .and_then(|n| n.parse::<u8>().ok())
                .is_some_and(|n| n < 16)
    };
    cells.iter().all(|(_, fg, bg)| in_16(fg) && in_16(bg))
        && cells.iter().any(|(_, _, bg)| bg != "Default")
}
