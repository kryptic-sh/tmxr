//! The pane holder proof of concept (`docs/plan/19-pane-holders.md`, phase
//! 1): the real `tmxr` binary run as `tmxr __holder <ENDPOINT>`, with this
//! test as the server that loses it and connects again.

use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use interprocess::local_socket::Stream;
use serde::{Deserialize, Serialize};
use tmxr_proto::codec::{read_msg, write_msg};
use tmxr_proto::socket::Endpoint;

/// Longest the whole test may take: past it, the holder is killed, which
/// ends any read the test is blocked in, and the test fails.
const TIMEOUT: Duration = Duration::from_secs(60);
/// How long the shell waits before printing its late line (the commands
/// below spell it out), and how long the test stays disconnected: well past
/// it, so the line is written while nothing is connected.
const DISCONNECTED: Duration = Duration::from_secs(5);

// The holder's protocol (`apps/tmxr/src/holder.rs`), copied: the binary
// exports nothing to link against. postcard encodes a variant by its index,
// so these must list the same variants in the same order; a mismatch fails
// this test's decoding.

#[derive(Debug, Serialize)]
enum ToHolder {
    Spawn {
        argv: Vec<String>,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        rows: u16,
        cols: u16,
    },
    Input(Vec<u8>),
    #[expect(dead_code, reason = "listed for the variants' order")]
    Resize {
        rows: u16,
        cols: u16,
    },
    TakeFrom(u64),
    Close,
}

#[derive(Debug, Deserialize)]
enum FromHolder {
    Hello {
        pid: Option<u32>,
        ring_start: u64,
        next: u64,
    },
    Output {
        number: u64,
        bytes: Vec<u8>,
    },
    Exited {
        #[expect(dead_code, reason = "read only to be printed")]
        code: Option<u32>,
        #[expect(dead_code, reason = "read only to be printed")]
        signal: Option<i32>,
    },
}

/// The variable the shell's markers are made from: a command line names it,
/// its output has the value, so a command echoed back never matches.
const MARK_VAR: &str = "TMXR_POC_MARK";

#[cfg(unix)]
fn shell() -> Vec<String> {
    vec!["/bin/sh".into()]
}

#[cfg(windows)]
fn shell() -> Vec<String> {
    vec!["cmd.exe".into(), "/d".into()]
}

/// A command line printing the marker with `suffix`.
#[cfg(unix)]
fn print_mark(suffix: &str) -> Vec<u8> {
    format!("echo \"${MARK_VAR}-{suffix}\"\r").into_bytes()
}

#[cfg(windows)]
fn print_mark(suffix: &str) -> Vec<u8> {
    format!("echo %{MARK_VAR}%-{suffix}\r").into_bytes()
}

/// A command line printing the marker with `suffix` a second from now.
#[cfg(unix)]
fn print_mark_later(suffix: &str) -> Vec<u8> {
    format!("sleep 1; echo \"${MARK_VAR}-{suffix}\"\r").into_bytes()
}

/// `ping -n 2` waits a second; `timeout` needs console input cmd may not
/// have.
#[cfg(windows)]
fn print_mark_later(suffix: &str) -> Vec<u8> {
    format!("ping -n 2 127.0.0.1 >nul & echo %{MARK_VAR}%-{suffix}\r").into_bytes()
}

/// A unique endpoint: a socket in `dir` on Unix, a pipe named by this
/// process on Windows (nextest runs each test in a process of its own).
fn endpoint(dir: &std::path::Path) -> PathBuf {
    #[cfg(unix)]
    {
        dir.join("holder")
    }
    #[cfg(windows)]
    {
        let _ = dir;
        PathBuf::from(format!(r"\\.\pipe\tmxr-test-holder-{}", std::process::id()))
    }
}

/// Kills the holder if the test runs past [`TIMEOUT`], and when the test
/// ends, so a failure leaves nothing running.
struct Watchdog {
    holder: Arc<Mutex<Child>>,
    done: Sender<()>,
}

impl Watchdog {
    fn new(holder: Child) -> Self {
        let holder = Arc::new(Mutex::new(holder));
        let (done, finished) = channel::<()>();
        let watched = Arc::clone(&holder);
        std::thread::spawn(move || {
            if finished.recv_timeout(TIMEOUT).is_err() {
                eprintln!("timed out: killing the holder");
                let _ = watched.lock().unwrap().kill();
            }
        });
        Self { holder, done }
    }

    /// Wait for the holder to exit by itself.
    fn wait_exit(&self) -> std::process::ExitStatus {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.holder.lock().unwrap().try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "the holder did not exit");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        let _ = self.done.send(());
        let mut holder = self.holder.lock().unwrap_or_else(|e| e.into_inner());
        let _ = holder.kill();
        let _ = holder.wait();
    }
}

/// A connection to the holder.
struct Conn {
    stream: Stream,
    /// Output received, and the number its first byte has.
    start: u64,
    output: Vec<u8>,
}

impl Conn {
    /// Connect, retrying while the holder starts listening.
    fn open(endpoint: &Endpoint) -> Self {
        let deadline = Instant::now() + TIMEOUT;
        let stream = loop {
            match endpoint.connect() {
                Ok(s) => break s,
                Err(e) => {
                    assert!(Instant::now() < deadline, "could not connect: {e}");
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        };
        Self {
            stream,
            start: 0,
            output: Vec::new(),
        }
    }

    fn send(&mut self, msg: &ToHolder) {
        write_msg(&mut self.stream, msg).unwrap();
    }

    /// The next message. A hung holder is killed by the [`Watchdog`], which
    /// ends this read.
    fn recv(&mut self) -> FromHolder {
        read_msg(&mut self.stream)
            .unwrap()
            .expect("the holder hung up")
    }

    /// The holder's greeting: (pid, oldest output kept, next output number).
    fn hello(&mut self) -> (u32, u64, u64) {
        match self.recv() {
            FromHolder::Hello {
                pid,
                ring_start,
                next,
            } => (pid.expect("a pid"), ring_start, next),
            other => panic!("expected Hello, got {other:?}"),
        }
    }

    /// Take output from `number`, which must be the first one sent.
    fn take_from(&mut self, number: u64) {
        self.send(&ToHolder::TakeFrom(number));
        self.start = number;
    }

    /// Read output until `until` holds of it, checking every piece follows
    /// on from the last.
    fn read_until(&mut self, what: &str, until: impl Fn(&[u8]) -> bool) {
        while !until(&self.output) {
            match self.recv() {
                FromHolder::Output { number, bytes } => {
                    assert_eq!(
                        number,
                        self.start + self.output.len() as u64,
                        "output out of sequence"
                    );
                    self.output.extend(bytes);
                }
                other => panic!(
                    "expected output, waiting for {what}, got {other:?}; so far: {:?}",
                    String::from_utf8_lossy(&self.output)
                ),
            }
        }
    }

    fn read_until_text(&mut self, text: &str) {
        self.read_until(text, |out| find(out, text).is_some());
    }
}

fn find(haystack: &[u8], needle: &str) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle.as_bytes())
}

/// Whether process `pid` is still running.
#[cfg(unix)]
fn running(pid: u32) -> bool {
    let pid = libc::pid_t::try_from(pid).unwrap();
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Whether process `pid` is still running: it can be opened and has no exit
/// code yet.
#[cfg(windows)]
fn running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: OpenProcess takes no pointers; the handle is checked and
    // closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return false;
    }
    let mut code = 0u32;
    // SAFETY: `handle` is a live process handle and `code` a live u32, the
    // only memory the call writes.
    let ok = unsafe { GetExitCodeProcess(handle, &mut code) } != 0;
    // SAFETY: `handle` is owned here and closed exactly once.
    unsafe { CloseHandle(handle) };
    ok && i32::try_from(code).is_ok_and(|c| c == STILL_ACTIVE)
}

#[test]
fn a_holder_keeps_its_program_across_connections() {
    let dir = tempfile::tempdir().unwrap();
    let path = endpoint(dir.path());
    let ep = Endpoint::resolve(Some(&path), None).unwrap();
    let holder = tmxr_term::child::command(env!("CARGO_BIN_EXE_tmxr"))
        .arg("__holder")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let watchdog = Watchdog::new(holder);
    let mark = format!("tmxr-poc-{}", std::process::id());
    let one = format!("{mark}-one");
    let late = format!("{mark}-late");
    let two = format!("{mark}-two");

    // The first server: starts the program, sees its output, and goes.
    let mut first = Conn::open(&ep);
    first.send(&ToHolder::Spawn {
        argv: shell(),
        cwd: None,
        env: vec![(MARK_VAR.into(), mark.clone())],
        rows: 24,
        cols: 80,
    });
    let (pid, ring_start, next) = first.hello();
    assert_eq!((ring_start, next), (0, 0));
    first.take_from(0);
    first.send(&ToHolder::Input(print_mark("one")));
    first.read_until_text(&one);
    first.send(&ToHolder::Input(print_mark_later("late")));
    let seen = first.output.clone();
    drop(first);

    // Nothing connected while the late line is printed.
    std::thread::sleep(DISCONNECTED);

    // The second server: the same program, all its output from the start.
    let mut second = Conn::open(&ep);
    let (again, ring_start, next) = second.hello();
    assert_eq!(again, pid, "a different program");
    assert_eq!(ring_start, 0);
    assert!(next >= seen.len() as u64, "{next} < {}", seen.len());
    second.take_from(0);
    let kept = usize::try_from(next).unwrap();
    second.read_until("the ring", |out| out.len() >= kept);
    assert_eq!(
        &second.output[..seen.len()],
        &seen[..],
        "the replay differs from what the first connection saw"
    );
    let replay = String::from_utf8_lossy(&second.output[..kept]).into_owned();
    let (at_one, at_late) = (replay.find(&one), replay.find(&late));
    assert!(
        at_one.is_some() && at_late.is_some() && at_one < at_late,
        "the ring lacks {one} then {late}: {replay:?}"
    );
    second.send(&ToHolder::Input(print_mark("two")));
    second.read_until_text(&two);
    assert!(find(&second.output, &late) < find(&second.output, &two));

    // Closing ends the program and the holder.
    second.send(&ToHolder::Close);
    let status = watchdog.wait_exit();
    assert!(status.success(), "the holder exited with {status}");
    let deadline = Instant::now() + TIMEOUT;
    while running(pid) {
        assert!(Instant::now() < deadline, "the program {pid} still runs");
        std::thread::sleep(Duration::from_millis(50));
    }
}
