//! One multiplexer under the oracle: its server, kept apart from any other
//! (its own socket label and directories), and for cases that attach, a
//! client on a pseudo-terminal whose screen the oracle reads.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::{At, Case, Check, Mouse, Step, split_words};

/// Which multiplexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Tmux,
    Tmxr,
}

/// The session every case runs in.
const SESSION: &str = "o";

/// How long to wait for a server, a client or text to show.
const WAIT: Duration = Duration::from_secs(10);

/// Socket labels in use by this process: Windows pipe names are not kept
/// apart by a directory.
static NEXT_LABEL: AtomicU32 = AtomicU32::new(0);

/// Variables a server or client must not inherit: they would point it at
/// the multiplexer the tests run in.
const UNSET: &[&str] = &["TMUX", "TMXR"];

/// How to run this server's program: its socket and config arguments, and
/// its environment.
struct Invocation {
    program: PathBuf,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

/// A running server, and its client if the case attaches one.
pub struct Server {
    pub side: Side,
    program: PathBuf,
    label: String,
    dir: tempfile::TempDir,
    config: PathBuf,
    client: Option<Client>,
    /// Commands started by `spawn` steps, ended with the server.
    spawned: Vec<std::process::Child>,
}

impl Server {
    /// Start `program` as `side` for `case`: its session, and its client.
    pub fn start(side: Side, program: &Path, case: &Case) -> Result<Self, String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let config = match side {
            Side::Tmux => crate::tmux_conf(),
            Side::Tmxr => {
                let path = dir.path().join("tmxr.toml");
                std::fs::write(
                    &path,
                    "[resurrect]\nrestore-on-start = false\nauto-save-minutes = 0\n",
                )
                .map_err(|e| e.to_string())?;
                path
            }
        };
        let label = format!(
            "oracle-{}-{}",
            std::process::id(),
            NEXT_LABEL.fetch_add(1, Ordering::Relaxed)
        );
        let mut server = Self {
            side,
            program: program.to_owned(),
            label,
            dir,
            config,
            client: None,
            spawned: Vec::new(),
        };
        let (cols, rows) = case.size;
        let (cols, rows) = (cols.to_string(), rows.to_string());
        let mut new = vec!["new-session", "-d", "-s", SESSION, "-x", &cols, "-y", &rows];
        if let Some(program) = &case.program {
            new.extend(["/bin/sh", "-c", program]);
        }
        server.run_args(&new)?;
        if case.attach {
            server.client = Some(Client::attach(&server, case.size)?);
        }
        Ok(server)
    }

    /// The program with this server's socket, config and environment.
    fn base(&self) -> Invocation {
        let args = vec![
            "-L".to_owned(),
            self.label.clone(),
            "-f".to_owned(),
            self.config.display().to_string(),
        ];
        let d = self.dir.path().display().to_string();
        let mut env = vec![
            ("TMUX_TMPDIR".to_owned(), d.clone()),
            ("TMXR_TMPDIR".to_owned(), d.clone()),
            ("XDG_DATA_HOME".to_owned(), d.clone()),
            ("XDG_STATE_HOME".to_owned(), d.clone()),
            ("XDG_CONFIG_HOME".to_owned(), d),
        ];
        if cfg!(unix) {
            env.push(("SHELL".to_owned(), "/bin/sh".to_owned()));
        }
        Invocation {
            program: self.program.clone(),
            args,
            env,
        }
    }

    fn command(&self, words: &[&str]) -> Command {
        let base = self.base();
        let mut cmd = Command::new(base.program);
        cmd.args(base.args).args(words);
        for (k, v) in base.env {
            cmd.env(k, v);
        }
        for k in UNSET {
            cmd.env_remove(k);
        }
        cmd
    }

    fn run_args(&self, words: &[&str]) -> Result<String, String> {
        let out = self.command(words).output().map_err(|e| e.to_string())?;
        let text = |b: &[u8]| String::from_utf8_lossy(b).replace("\r\n", "\n");
        if !out.status.success() {
            return Err(format!("{words:?}: {}", text(&out.stderr).trim_end()));
        }
        Ok(text(&out.stdout).trim_end().to_owned())
    }

    /// Run a command line.
    pub fn run(&self, line: &str) -> Result<String, String> {
        let words = split_words(line);
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        self.run_args(&words)
    }

    fn client(&self) -> Result<&Client, String> {
        self.client
            .as_ref()
            .ok_or_else(|| "the case needs attach = true".into())
    }

    /// Do `step`.
    pub fn step(&mut self, step: &Step) -> Result<(), String> {
        match step {
            Step::Run(line) => self.run(line).map(drop),
            Step::Spawn(line) => {
                let words = split_words(line);
                let words: Vec<&str> = words.iter().map(String::as_str).collect();
                let child = self
                    .command(&words)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .map_err(|e| e.to_string())?;
                self.spawned.push(child);
                std::thread::sleep(Duration::from_millis(300));
                Ok(())
            }
            Step::Keys(keys) => self.client()?.send(keys.as_bytes()),
            Step::Mouse(m) => self.mouse(m),
            Step::Sleep(secs) => {
                std::thread::sleep(Duration::from_secs_f64(*secs));
                Ok(())
            }
            Step::WaitText(text) => {
                let c = self.client()?;
                let deadline = Instant::now() + WAIT;
                while !c.screen().contains(text.as_str()) {
                    if Instant::now() > deadline {
                        return Err(format!("{text:?} never showed:\n{}", c.screen()));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Ok(())
            }
        }
    }

    fn mouse(&self, m: &Mouse) -> Result<(), String> {
        let c = self.client()?;
        let at = c.locate(&m.at)?;
        let b = u32::from(m.button.saturating_sub(1));
        match m.action.as_str() {
            "click" => {
                c.sgr(b, at, true)?;
                c.sgr(b, at, false)
            }
            "down" => c.sgr(b, at, true),
            "up" => c.sgr(b, at, false),
            "wheel-up" => c.sgr(64, at, true),
            "wheel-down" => c.sgr(65, at, true),
            "drag" => {
                let to = c.locate(m.to.as_ref().ok_or("drag needs `to`")?)?;
                c.sgr(b, at, true)?;
                let steps = 4;
                for i in 1..=steps {
                    let lerp = |a: u16, z: u16| {
                        let (a, z) = (i32::from(a), i32::from(z));
                        u16::try_from(a + (z - a) * i / steps).unwrap_or(0)
                    };
                    c.sgr(b + 32, (lerp(at.0, to.0), lerp(at.1, to.1)), true)?;
                }
                c.sgr(b, to, false)
            }
            other => Err(format!("unknown mouse action {other}")),
        }
    }

    /// Read `check`.
    pub fn read(&mut self, check: &Check) -> Result<String, String> {
        match check {
            Check::Format { format, .. } => {
                self.run_args(&["display-message", "-p", "-t", SESSION, format])
            }
            Check::Command { command, .. } => self.run(command),
            Check::Screen {
                screen_contains, ..
            } => Ok(self
                .client()?
                .screen()
                .contains(screen_contains.as_str())
                .to_string()),
            Check::Find { screen_find, .. } => {
                let at = At::Text {
                    text: screen_find.clone(),
                    row: None,
                    dx: 0,
                };
                Ok(self
                    .client()?
                    .locate(&at)
                    .map_or_else(|_| "none".to_owned(), |(c, r)| format!("{c},{r}")))
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.run_args(&["kill-server"]);
        for child in &mut self.spawned {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A client attached on a pseudo-terminal.
pub struct Client {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    parser: Arc<Mutex<vt100::Parser>>,
    _master: Box<dyn MasterPty + Send>,
    _child: Box<dyn portable_pty::Child + Send + Sync>,
    rows: u16,
}

impl Client {
    fn attach(server: &Server, (cols, rows): (u16, u16)) -> Result<Self, String> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;
        let base = server.base();
        let mut cmd = CommandBuilder::new(base.program);
        cmd.args(base.args);
        cmd.args(["attach", "-t", SESSION]);
        for (k, v) in base.env {
            cmd.env(k, v);
        }
        for k in UNSET {
            cmd.env_remove(k);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.cwd(server.dir.path());
        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = Arc::new(Mutex::new(
            pair.master.take_writer().map_err(|e| e.to_string())?,
        ));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
        let feed = Arc::clone(&parser);
        let answer = Arc::clone(&writer);
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                feed.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .process(&buf[..n]);
                // A device-attributes query answered as a terminal would, so
                // neither multiplexer waits on one.
                if buf[..n].windows(3).any(|w| w == b"\x1b[c") {
                    let mut w = answer
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let _ = w.write_all(b"\x1b[?62;22c");
                }
            }
        });
        let client = Self {
            writer,
            parser,
            _master: pair.master,
            _child: child,
            rows,
        };
        // Attached: the status line drawn; and the mouse asked for, which
        // mouse steps need.
        let deadline = Instant::now() + WAIT;
        loop {
            let ready = {
                let p = client.lock();
                p.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None
                    && !p.screen().contents().trim().is_empty()
            };
            if ready {
                break;
            }
            if Instant::now() > deadline {
                return Err(format!("client never ready:\n{}", client.screen()));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(300));
        Ok(client)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, vt100::Parser> {
        self.parser
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn send(&self, bytes: &[u8]) -> Result<(), String> {
        let mut w = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        w.write_all(bytes)
            .and_then(|()| w.flush())
            .map_err(|e| e.to_string())?;
        drop(w);
        std::thread::sleep(Duration::from_millis(80));
        Ok(())
    }

    /// An SGR mouse report: button code `b` at `(col, row)`, pressed or let
    /// go.
    fn sgr(&self, b: u32, (col, row): (u16, u16), press: bool) -> Result<(), String> {
        let end = if press { 'M' } else { 'm' };
        self.send(format!("\x1b[<{b};{};{}{end}", col + 1, row + 1).as_bytes())
    }

    /// The screen's text, row by row.
    pub fn screen(&self) -> String {
        self.lock().screen().contents()
    }

    /// The cell `at` names on the screen now.
    fn locate(&self, at: &At) -> Result<(u16, u16), String> {
        match at {
            At::Cell([col, row]) => Ok((*col, *row)),
            At::Text { text, row, dx } => {
                let p = self.lock();
                let (rows, cols) = p.screen().size();
                let only = row.map(|r| {
                    if r < 0 {
                        u16::try_from(i32::from(self.rows) + r).unwrap_or(0)
                    } else {
                        u16::try_from(r).unwrap_or(0)
                    }
                });
                p.screen()
                    .rows(0, cols)
                    .enumerate()
                    .take(usize::from(rows))
                    .filter(|(r, _)| only.is_none_or(|o| usize::from(o) == *r))
                    .find_map(|(r, line)| {
                        let byte = line.find(text.as_str())?;
                        let col = line[..byte].chars().count();
                        Some((u16::try_from(col).ok()? + dx, u16::try_from(r).ok()?))
                    })
                    .ok_or_else(|| {
                        format!("{text:?} is not on the screen:\n{}", p.screen().contents())
                    })
            }
        }
    }
}
