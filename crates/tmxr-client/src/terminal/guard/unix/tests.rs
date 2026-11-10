//! The guard against a real terminal: this test binary run again as the
//! fixture below, in a pty that is its standard streams and (as an attached
//! client usually runs) its controlling terminal, or (as under a `setsid`
//! launcher) not; the test plays the terminal.
//!
//! Unix only: tmxr's Windows client still takes the console through
//! crossterm until plan 18 step 8, and crossterm's restore there has no
//! kitty flags to pop.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt as _;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tmxr_term::encode::TMXR_KITTY_FLAGS;

use super::*;

/// Set in the fixture's environment, so it runs only when the test runs it.
const FIXTURE: &str = "TMXR_CLIENT_GUARD_FIXTURE";

/// How long the test waits for the fixture to finish.
const WAIT: Duration = Duration::from_secs(30);

/// The kitty keyboard query, which the test's terminal answers with the
/// flags pushed when it came ([`flags_at`]).
const KITTY_QUERY: &[u8] = b"\x1b[?u";
/// DA1, which the probe asks last, and the test terminal's answer.
const DA1_QUERY: &[u8] = b"\x1b[c";
const DA1_ANSWER: &[u8] = b"\x1b[?62c";
/// The kitty pop `ttyr` writes.
const POP: &str = "\x1b[<1u";
/// modifyOtherKeys level 2 on, and off.
const MOK_ON: &str = "\x1b[>4;2m";
const MOK_OFF: &str = "\x1b[>4m";

/// What the fixture prints once it has written everything, before it
/// waits for a line from the test.
const DONE: &str = "fixture-done";

/// How often the fixture gives the terminal away and takes it back.
const HAND_OVERS: usize = 2;

/// Takes the terminal as an attached client does, asks for the kitty
/// flags as a kitty client is told to and for modifyOtherKeys as an xterm
/// one is (the guard takes both), captures the mouse, gives the
/// terminal away and back [`HAND_OVERS`] times (as locks do, each taking
/// the terminal again), asks for flags 0 and the flags again, gives the
/// terminal away and back once more with no flags to put back (as after
/// the terminal's keyboard changed), asks for the flags again, captures
/// the mouse again, then panics. The reader is told at each step whether
/// the protocol is on. Then waits for the test to have read it all: a pty
/// may drop output its program's side closes on unread.
/// Ask for kitty `flags` as the client does for the server, telling the
/// reader what the guard says is on (the input thread tells it, in the
/// client).
fn ask(guard: &mut Guard, keys: &mut Keys, flags: u8) {
    if let Some(on) = guard.set_keyboard_flags(flags).unwrap() {
        keys.set_kitty_keyboard(on);
    }
}

#[test]
#[ignore = "a fixture a_panic_gives_the_terminal_back_once runs in a pty"]
fn panic_fixture() {
    if std::env::var_os(FIXTURE).is_none() {
        return;
    }
    let panicked = std::panic::catch_unwind(|| {
        let mut keys = Keys::open().unwrap();
        let (mut guard, _) = Guard::enter(&mut keys).unwrap();
        assert!(guard.keyboard().kitty);
        assert!(!keys.kitty_keyboard, "on before any push");
        ask(&mut guard, &mut keys, TMXR_KITTY_FLAGS);
        assert!(keys.kitty_keyboard, "off after the push");
        guard.set_extended_keys(true).unwrap();
        guard.set_mouse(true).unwrap();
        for _ in 0..HAND_OVERS {
            guard
                .hand_over(&mut keys, Some(TMXR_KITTY_FLAGS), || ())
                .unwrap();
            assert!(keys.kitty_keyboard, "off after a hand-over");
        }
        // Flags 0 are the legacy keys: xterm's Meta bit again.
        ask(&mut guard, &mut keys, 0);
        assert!(!keys.kitty_keyboard, "on with flags 0");
        ask(&mut guard, &mut keys, TMXR_KITTY_FLAGS);
        // The probe hears the flags put back, then they are popped.
        guard.hand_over(&mut keys, None, || ()).unwrap();
        assert!(!keys.kitty_keyboard, "on after the pop");
        ask(&mut guard, &mut keys, TMXR_KITTY_FLAGS);
        assert!(keys.kitty_keyboard, "off after the last push");
        guard.set_mouse(true).unwrap();
        panic!("the fixture panics on purpose");
    });
    assert!(panicked.is_err());
    println!("{DONE}");
    std::io::stdin().read_line(&mut String::new()).unwrap();
}

/// A new pty: the terminal's side, and the program's.
fn pty() -> (File, OwnedFd) {
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: `openpty` writes two descriptors into the integers it is
    // given, which outlive the call; the name, attributes and size it
    // takes are optional, and null.
    let opened = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(opened, 0, "openpty: {}", std::io::Error::last_os_error());
    // Kept from programs this process starts: the fixture gets the
    // program's side as its standard streams, which exec keeps open.
    for fd in [master, slave] {
        // SAFETY: `fcntl` only sets a flag on `fd`, which is open.
        let set = unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        assert_eq!(set, 0, "fcntl: {}", std::io::Error::last_os_error());
    }
    // SAFETY: both were just opened and nothing else owns them.
    unsafe { (File::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) }
}

/// Times `needle` occurs in `hay`.
fn count(hay: &[u8], needle: &[u8]) -> usize {
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

/// The kitty flags on top of the terminal's stack after what `written`
/// pushed (`CSI > flags u`) and popped (`CSI < count u`), as it answers
/// the flags query.
fn flags_at(written: &[u8]) -> u32 {
    let mut stack = Vec::new();
    let mut rest = written;
    while let Some(at) = rest.windows(2).position(|w| w == b"\x1b[") {
        rest = &rest[at + 2..];
        let Some((&kind @ (b'>' | b'<'), after)) = rest.split_first() else {
            continue;
        };
        let digits = after.iter().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 || after.get(digits) != Some(&b'u') {
            continue;
        }
        let n: u32 = std::str::from_utf8(&after[..digits])
            .unwrap()
            .parse()
            .unwrap();
        if kind == b'>' {
            stack.push(n);
        } else {
            stack.truncate(stack.len().saturating_sub(n as usize));
        }
    }
    stack.last().copied().unwrap_or(0)
}

/// Where `needle` starts in `hay`, each time.
fn positions(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    hay.windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle)
        .map(|(at, _)| at)
        .collect()
}

/// The kitty pushes in `text` (`CSI > flags u`).
fn pushes(text: &str) -> Vec<usize> {
    text.match_indices("\x1b[>")
        .filter(|&(at, _)| {
            let rest = &text[at + 3..];
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            digits > 0 && rest[digits..].starts_with('u')
        })
        .map(|(at, _)| at)
        .collect()
}

/// What [`panic_fixture`] left: all it wrote, and the pty's local modes
/// once it exited.
struct Run {
    text: String,
    local_modes: libc::tcflag_t,
}

/// Runs [`panic_fixture`] in a pty, its controlling terminal where
/// `controlling`, answering its probes as a kitty terminal, with the flags
/// it has pushed.
fn run_fixture(controlling: bool) -> Run {
    let (master, slave) = pty();
    let mut command = tmxr_term::child::command(std::env::current_exe().unwrap());
    command
        .args([
            "terminal::guard::unix::tests::panic_fixture",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(FIXTURE, "1")
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave));
    // SAFETY: runs in the child between fork and exec, calling only
    // `setsid` and `ioctl`, which are async-signal-safe.
    unsafe {
        command.pre_exec(move || {
            // Its own session, so without a controlling terminal unless the
            // pty (its stdin) is made it.
            if libc::setsid() == -1
                || (controlling && libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1)
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    // The child holds the program's side now: once it exits, reads end.
    drop(command);
    let (send, receive) = mpsc::channel();
    let mut reader = master.try_clone().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0; 4096];
        // Linux ends with EIO once the program's side is closed.
        while let Ok(n @ 1..) = reader.read(&mut buf) {
            if send.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut terminal = master;
    let mut got = Vec::new();
    let (mut kitty, mut da1) = (0, 0);
    let deadline = Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match receive.recv_timeout(left) {
            Ok(bytes) => {
                let done = count(&got, DONE.as_bytes());
                got.extend(bytes);
                if count(&got, DONE.as_bytes()) > done {
                    terminal.write_all(b"\n").unwrap();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = child.kill();
                panic!(
                    "the fixture did not finish: {:?}",
                    String::from_utf8_lossy(&got)
                );
            }
        }
        // Answered in the order asked: the kitty query before DA1.
        let queries = positions(&got, KITTY_QUERY);
        for &at in &queries[kitty..] {
            let answer = format!("\x1b[?{}u", flags_at(&got[..at]));
            terminal.write_all(answer.as_bytes()).unwrap();
        }
        kitty = queries.len();
        for _ in da1..count(&got, DA1_QUERY) {
            terminal.write_all(DA1_ANSWER).unwrap();
        }
        da1 = count(&got, DA1_QUERY);
    }
    child.wait().unwrap();
    // SAFETY: `termios` is plain data that `tcgetattr` fills in, from the
    // pty's side this test holds open.
    let local_modes = unsafe {
        let mut termios = std::mem::zeroed::<libc::termios>();
        assert_eq!(
            libc::tcgetattr(terminal.as_raw_fd(), &mut termios),
            0,
            "tcgetattr: {}",
            std::io::Error::last_os_error()
        );
        termios.c_lflag
    };
    Run {
        text: String::from_utf8_lossy(&got).into_owned(),
        local_modes,
    }
}

#[test]
fn a_panic_gives_the_terminal_back_once() {
    gives_the_terminal_back_once(run_fixture(true));
}

/// As above, from a client with no controlling terminal: its terminal is
/// taken and given back on its standard streams.
#[test]
fn a_panic_gives_a_terminal_that_is_not_the_controlling_one_back_once() {
    gives_the_terminal_back_once(run_fixture(false));
}

/// What [`panic_fixture`] left has the terminal taken and given back once
/// on its panic: raw mode off again on that terminal, and every mode
/// turned off once.
fn gives_the_terminal_back_once(Run { text, local_modes }: Run) {
    // A new pty edits lines and echoes, as raw mode turns off.
    let cooked = libc::ICANON | libc::ECHO;
    assert_eq!(local_modes & cooked, cooked, "{local_modes:#x}");
    assert!(text.contains("the fixture panics on purpose"), "{text:?}");
    assert!(text.contains(DONE), "{text:?}");
    // Pushed on attach, again after each hand-over, for flags 0 and back,
    // and after the hand-over that popped them; popped before each
    // hand-over, in place of each new push, after the last hand-over and
    // once on the panic: never more pops than pushes, which would pop the
    // terminal's own flags.
    let hand_overs = HAND_OVERS + 1;
    let pushed = pushes(&text);
    assert_eq!(pushed.len(), 1 + hand_overs + 3, "{text:?}");
    assert_eq!(text.matches(POP).count(), pushed.len(), "{text:?}");
    // modifyOtherKeys on at attach and after each hand-over, off before
    // each and on the panic.
    assert_eq!(text.matches(MOK_ON).count(), hand_overs + 1, "{text:?}");
    assert_eq!(text.matches(MOK_OFF).count(), hand_overs + 1, "{text:?}");
    // After the last push, the panic gives everything back once.
    let after = &text[pushed[pushed.len() - 1]..];
    for (what, sequence) in [
        ("kitty pop", POP),
        ("modifyOtherKeys off", MOK_OFF),
        ("mouse off", "\x1b[?1000l"),
        ("cursor shape reset", "\x1b[0 q"),
        ("alternate screen left", "\x1b[?1049l"),
    ] {
        assert_eq!(after.matches(sequence).count(), 1, "{what}: {after:?}");
    }
}
