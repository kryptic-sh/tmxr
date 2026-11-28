//! The pane holder proof of concept (`docs/plan/19-pane-holders.md`, phase
//! 1): the real `tmxr` binary run as `tmxr __holder <ENDPOINT>`, with these
//! tests as the server that loses it and connects again.

#[path = "../src/holder/proto.rs"]
mod proto;

use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use interprocess::local_socket::Stream;
use proto::{FromHolder, ToHolder};
use tmxr_proto::codec::{read_msg, write_msg};
use tmxr_proto::socket::Endpoint;

/// How long any one wait in a test may take before it fails.
const WAIT: Duration = Duration::from_secs(30);
/// How long a whole test may take: past it, the [`Watchdog`] kills the
/// holder, which ends any read the test is blocked in, and the test fails.
const TEST_LIMIT: Duration = Duration::from_secs(120);
/// How long the shell waits before printing its late line, in seconds.
const LATE_DELAY_SECS: u64 = 1;
/// How long the test stays disconnected: five times [`LATE_DELAY_SECS`], so
/// the late line is written while nothing is connected.
const DISCONNECTED: Duration = Duration::from_secs(LATE_DELAY_SECS * 5);

/// The variable the shell's markers are made from: a command line names it,
/// its output has the value, so a command echoed back never matches.
const MARK_VAR: &str = "TMXR_POC_MARK";
/// What the shell's prompt starts with (`PS1` on Unix, `PROMPT` for cmd),
/// so the test can wait for it before typing: an Enter sent to cmd before
/// its first read can be held by ConPTY (docs/backlog.md).
const PROMPT: &str = "tmxr-ready>";

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

/// A command line printing the marker with `suffix` [`LATE_DELAY_SECS`]
/// from now.
#[cfg(unix)]
fn print_mark_later(suffix: &str) -> Vec<u8> {
    format!("sleep {LATE_DELAY_SECS}; echo \"${MARK_VAR}-{suffix}\"\r").into_bytes()
}

/// `ping -n N` waits N - 1 seconds; `timeout` needs console input cmd may
/// not have.
#[cfg(windows)]
fn print_mark_later(suffix: &str) -> Vec<u8> {
    let pings = LATE_DELAY_SECS + 1;
    format!("ping -n {pings} 127.0.0.1 >nul & echo %{MARK_VAR}%-{suffix}\r").into_bytes()
}

/// A unique endpoint: a socket in `dir` on Unix, a pipe named by this
/// process and a count on Windows.
fn endpoint(dir: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        dir.join("holder")
    }
    #[cfg(windows)]
    {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let _ = dir;
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        PathBuf::from(format!(
            r"\\.\pipe\tmxr-test-holder-{}-{n}",
            std::process::id()
        ))
    }
}

/// A marker unique to this test.
fn mark() -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("tmxr-poc-{}-{n}", std::process::id())
}

/// A started holder. Kills it if the test runs past [`TEST_LIMIT`], and
/// when the test ends, so a failure leaves nothing running.
struct Watchdog {
    _dir: tempfile::TempDir,
    endpoint: Endpoint,
    holder: Arc<Mutex<Child>>,
    done: Sender<()>,
}

impl Watchdog {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = endpoint(dir.path());
        let endpoint = Endpoint::resolve(Some(&path), None).unwrap();
        let holder = tmxr_term::child::command(env!("CARGO_BIN_EXE_tmxr"))
            .arg("__holder")
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let holder = Arc::new(Mutex::new(holder));
        let (done, finished) = channel::<()>();
        let watched = Arc::clone(&holder);
        std::thread::spawn(move || {
            if finished.recv_timeout(TEST_LIMIT).is_err() {
                eprintln!("timed out: killing the holder");
                let _ = watched.lock().unwrap().kill();
            }
        });
        Self {
            _dir: dir,
            endpoint,
            holder,
            done,
        }
    }

    /// Wait for the holder to exit by itself.
    fn wait_exit(&self) -> std::process::ExitStatus {
        let deadline = Instant::now() + WAIT;
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
    fn open(holder: &Watchdog) -> Self {
        let deadline = Instant::now() + WAIT;
        let stream = loop {
            match holder.endpoint.connect() {
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

    /// Start `argv` with the marker variable set to `mark` and the prompt
    /// set to [`PROMPT`].
    fn spawn(&mut self, argv: Vec<String>, mark: &str) {
        self.send(&ToHolder::Spawn {
            argv,
            cwd: None,
            env: vec![
                (MARK_VAR.into(), mark.into()),
                ("PS1".into(), format!("{PROMPT} ")),
                ("PROMPT".into(), PROMPT.replace('>', "$G")),
            ],
            rows: 24,
            cols: 80,
        });
    }

    /// The next message. A hung holder is killed by the [`Watchdog`], which
    /// ends this read.
    fn recv(&mut self) -> FromHolder {
        match read_msg(&mut self.stream)
            .unwrap()
            .expect("the holder hung up")
        {
            FromHolder::Error(e) => panic!("the holder failed: {e}"),
            msg => msg,
        }
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

    /// Keep an output message, checking it follows on from the last; any
    /// other message is returned.
    fn keep(&mut self, msg: FromHolder) -> Option<FromHolder> {
        match msg {
            FromHolder::Output { number, bytes } => {
                assert_eq!(
                    number,
                    self.start + self.output.len() as u64,
                    "output out of sequence"
                );
                self.output.extend(bytes);
                None
            }
            other => Some(other),
        }
    }

    /// Read output until `until` holds of it.
    fn read_until(&mut self, what: &str, until: impl Fn(&[u8]) -> bool) {
        while !until(&self.output) {
            let msg = self.recv();
            if let Some(other) = self.keep(msg) {
                panic!(
                    "expected output, waiting for {what}, got {other:?}; so far: {:?}",
                    String::from_utf8_lossy(&self.output)
                );
            }
        }
    }

    fn read_until_text(&mut self, text: &str) {
        self.read_until(text, |out| find(out, text).is_some());
    }

    /// Read output until the program's exit: its code and signal.
    fn read_exit(&mut self) -> (Option<u32>, Option<i32>) {
        loop {
            let msg = self.recv();
            match self.keep(msg) {
                None => {}
                Some(FromHolder::Exited { code, signal }) => return (code, signal),
                Some(other) => panic!("expected output or Exited, got {other:?}"),
            }
        }
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

/// Close the holder on `conn`, then check it exits and `pid` has gone.
fn close(holder: &Watchdog, conn: &mut Conn, pid: u32) {
    conn.send(&ToHolder::Close);
    let status = holder.wait_exit();
    assert!(status.success(), "the holder exited with {status}");
    let deadline = Instant::now() + WAIT;
    while running(pid) {
        assert!(Instant::now() < deadline, "the program {pid} still runs");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_holder_keeps_its_program_across_connections() {
    let holder = Watchdog::start();
    let mark = mark();
    let one = format!("{mark}-one");
    let late = format!("{mark}-late");
    let two = format!("{mark}-two");

    // The first server: starts the program, sees its output, and goes.
    let mut first = Conn::open(&holder);
    first.spawn(shell(), &mark);
    let (pid, ring_start, next) = first.hello();
    assert_eq!((ring_start, next), (0, 0));
    first.take_from(0);
    first.read_until_text(PROMPT);
    first.send(&ToHolder::Input(print_mark("one")));
    first.read_until_text(&one);
    first.send(&ToHolder::Input(print_mark_later("late")));
    let seen = first.output.clone();
    drop(first);

    // Nothing connected while the late line is printed.
    std::thread::sleep(DISCONNECTED);

    // The second server: the same program, all its output from the start.
    let mut second = Conn::open(&holder);
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
    second.send(&ToHolder::Resize {
        rows: 30,
        cols: 100,
    });
    second.send(&ToHolder::Input(print_mark("two")));
    second.read_until_text(&two);
    assert!(find(&second.output, &late) < find(&second.output, &two));
    // The holder answered ConPTY's startup query and took it out.
    #[cfg(windows)]
    assert!(
        find(&second.output, "\x1b[6n").is_none(),
        "the startup query reached the server: {:?}",
        String::from_utf8_lossy(&second.output)
    );

    close(&holder, &mut second, pid);
}

/// A command that prints the marker with `-first` and `-tail`, `delay`
/// seconds from now, then exits with status 3.
#[cfg(unix)]
fn print_and_exit(delay: u64) -> Vec<String> {
    vec![
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        format!("sleep {delay}; echo \"${MARK_VAR}-first\"; echo \"${MARK_VAR}-tail\"; exit 3"),
    ]
}

#[cfg(windows)]
fn print_and_exit(delay: u64) -> Vec<String> {
    let pings = delay + 1;
    vec![
        "cmd.exe".to_owned(),
        "/d".to_owned(),
        "/c".to_owned(),
        format!(
            "ping -n {pings} 127.0.0.1 >nul & echo %{MARK_VAR}%-first& echo %{MARK_VAR}%-tail& exit /b 3"
        ),
    ]
}

/// Run [`print_and_exit`] and take its output, either while it runs or
/// only once it has gone: its exit must be reported after all its output,
/// and the holder must then exit.
fn exit_is_reported_after_the_last_output(while_running: bool) {
    let holder = Watchdog::start();
    let mark = mark();
    let mut conn = Conn::open(&holder);
    conn.spawn(
        print_and_exit(if while_running { LATE_DELAY_SECS } else { 0 }),
        &mark,
    );
    let (pid, _, _) = conn.hello();
    if !while_running {
        let deadline = Instant::now() + WAIT;
        while running(pid) {
            assert!(Instant::now() < deadline, "the program {pid} still runs");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    conn.take_from(0);
    let (code, signal) = conn.read_exit();
    let text = String::from_utf8_lossy(&conn.output).into_owned();
    assert!(
        text.contains(&format!("{mark}-first")) && text.contains(&format!("{mark}-tail")),
        "the output before Exited is not all there: {text:?}"
    );
    assert_eq!((code, signal), (Some(3), None));
    let status = holder.wait_exit();
    assert!(status.success(), "the holder exited with {status}");
}

#[test]
fn an_exit_is_reported_after_the_last_output() {
    exit_is_reported_after_the_last_output(true);
}

#[test]
fn an_exit_before_any_connection_takes_output_is_reported_after_it() {
    exit_is_reported_after_the_last_output(false);
}

/// `Close` ends a program that ignores the hangup it is sent first.
#[cfg(unix)]
#[test]
fn close_kills_a_program_that_ignores_hangups() {
    let holder = Watchdog::start();
    let mark = mark();
    let argv = vec![
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        format!("trap '' HUP; echo \"${MARK_VAR}-ready\"; while :; do sleep 1; done"),
    ];
    let mut conn = Conn::open(&holder);
    conn.spawn(argv, &mark);
    let (pid, _, _) = conn.hello();
    conn.take_from(0);
    conn.read_until_text(&format!("{mark}-ready"));
    close(&holder, &mut conn, pid);
}
