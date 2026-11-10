//! The kitty keyboard protocol into panes (plan 18 step 6): a program that
//! pushed flags gets keys under every flag, its releases among them; the
//! client's terminal is asked for what the program its keys go to wants,
//! as focus moves; a legacy client's keys get a release made for them, and
//! a console key record's its own; and a release never runs a bind.
//!
//! Unix only, all of it: the kitty harness terminal needs its query to
//! reach it and its keys to reach the client. ConPTY between the harness
//! and a Windows client is not known to pass either (outside Windows
//! Terminal the client asks nothing, reading key records), nor kitty keys
//! from the server to a pane. The encoder's side is
//! unit-tested on every platform (`tmxr-term`'s `encode::kitty::tests`).

#![cfg(unix)]

use super::*;
use tmxr_term::encode::TMXR_KITTY_FLAGS;

/// Opens a window in `session` (as a pane split in its current window
/// where `split`) running the key-log program, which pushes kitty `flags`
/// and logs the bytes it reads; waits until it has pushed them, and
/// returns its log.
fn key_log_pane(t: &Tmxr, session: &str, name: &str, flags: u8, split: bool) -> PathBuf {
    let log = t.dir.path().join(format!("{name}.log"));
    let argv = key_log_program(t, name);
    let env = format!("{KEY_LOG}={}", log.display());
    let raw = format!(r"{KEY_LOG_RAW}=\e[>{flags}u");
    let how = if split { "split-window" } else { "new-window" };
    let mut new = vec![how, "-t", session, "-e", &env, "-e", &raw];
    new.extend(argv.iter().map(String::as_str));
    t.run(&new);
    t.wait_run(&["capture-pane", "-p", "-t", session], "the mode", |o| {
        o.contains("raw-on")
    });
    log
}

/// What the key-log program has read, every read run together, escaped.
fn logged(log: &Path) -> String {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|&l| l != "ready")
        .collect()
}

/// Wait until the key-log program has read exactly `want` (escaped).
fn wait_logged(log: &Path, want: &str) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let got = logged(log);
        if got == want {
            return;
        }
        assert!(
            Instant::now() < deadline && want.starts_with(&got),
            "read {got:?}, not {want:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A program under each flag gets what the kitty terminal sends it as the
/// spec has it: under flag 2 a press and its release, under flag 4 a
/// chord's shifted and base-layout keys, under flag 8 a plain letter (typed
/// as text or sent as an escape code) as an escape code, under flag 16 the
/// text with it. The terminal is asked for each program's flags first.
#[test]
fn a_kitty_pane_gets_each_flags_keys() {
    let t = Tmxr::new("kittyeach");
    t.run(&["new-session", "-d", "-s", "ke"]);
    let s = t.attach(&["attach", "-t", "ke"]);
    s.wait_for("status line", |text| text.contains("ke"));
    s.wait_kitty_flags(TMXR_KITTY_FLAGS);
    for (name, flags, send, want) in [
        (
            "events",
            1 | 2,
            &b"\x1b[97;5u\x1b[97;5:3u"[..],
            r"\x1b[97;5u\x1b[97;5:3u",
        ),
        (
            "alternates",
            1 | 4,
            b"\x1b[1089:1057:99;6u",
            r"\x1b[1089:1057:99;6u",
        ),
        ("allkeys", 8, b"\x1b[97ub", r"\x1b[97u\x1b[98u"),
        ("text", 8 | 16, b"\x1b[97:65;2;65u", r"\x1b[97;2;65u"),
    ] {
        let log = key_log_pane(&t, "ke", name, flags, false);
        s.wait_kitty_flags(TMXR_KITTY_FLAGS | flags);
        s.send(send);
        wait_logged(&log, want);
    }
}

/// The client's terminal is asked for the flags of the program its keys go
/// to, and for tmxr's own where tmxr reads them: as focus moves between a
/// pane under flags 1, 2 and 8 and a shell that pushed none, and while copy
/// mode or the command prompt has the keys.
#[test]
fn focus_changes_what_the_terminal_is_asked_for() {
    let t = Tmxr::new("kittyfocus");
    t.run(&["new-session", "-d", "-s", "kf"]);
    let s = t.attach(&["attach", "-t", "kf"]);
    s.wait_for("status line", |text| text.contains("kf"));
    s.wait_kitty_flags(TMXR_KITTY_FLAGS);
    key_log_pane(&t, "kf", "elevenpane", 11, true);
    let program = TMXR_KITTY_FLAGS | 11;
    s.wait_kitty_flags(program);
    t.run(&["select-pane", "-t", "kf:0.0"]);
    s.wait_kitty_flags(TMXR_KITTY_FLAGS);
    t.run(&["select-pane", "-t", "kf:0.1"]);
    s.wait_kitty_flags(program);
    t.run(&["copy-mode", "-t", "kf:0.1"]);
    s.wait_kitty_flags(TMXR_KITTY_FLAGS);
    t.run(&["send-keys", "-X", "-t", "kf:0.1", "cancel"]);
    s.wait_kitty_flags(program);
    // C-b : from a terminal under flags 1, 2, 4 and 8, as it sends them.
    s.send(b"\x1b[98;5u\x1b[98;5:3u\x1b[59:58;2u\x1b[59:58;2:3u");
    s.wait_kitty_flags(TMXR_KITTY_FLAGS);
    s.send(b"\x1b[27u");
    s.wait_kitty_flags(program);
}

/// A legacy terminal reports no releases, so a program that asked for
/// them gets one made right after each press: Ctrl+a (`0x01`) and a typed
/// `x`, each released.
#[test]
fn a_legacy_clients_keys_are_released_for_it() {
    let t = Tmxr::new("legacyrelease");
    t.run(&["new-session", "-d", "-s", "lr"]);
    let s = t.attach_legacy(&["attach", "-t", "lr"]);
    s.wait_for("status line", |text| text.contains("lr"));
    let log = key_log_pane(&t, "lr", "releasepane", 1 | 2, false);
    s.send(b"\x01");
    wait_logged(&log, r"\x1b[97;5u\x1b[97;5:3u");
    s.send(b"x");
    wait_logged(&log, r"\x1b[97;5u\x1b[97;5:3ux\x1b[120;1:3u");
}

/// A Windows console's key records (here as Windows Terminal's
/// win32-input-mode writes them, `CSI Vk;Sc;Uc;Kd;Cs;Rc _`, which the
/// client's reader decodes on every platform) come with their releases
/// whatever was asked, so a program that asked for releases gets those,
/// not one made at once: two keys held together are released as the
/// terminal released them.
#[test]
fn a_console_keys_releases_are_its_own() {
    let t = Tmxr::new("consolerelease");
    t.run(&["new-session", "-d", "-s", "cr"]);
    let s = t.attach_legacy(&["attach", "-t", "cr"]);
    s.wait_for("status line", |text| text.contains("cr"));
    let log = key_log_pane(&t, "cr", "consolepane", 1 | 2, false);
    let record = |vk: u8, unit: char, down: bool| {
        format!("\x1b[{vk};0;{};{};0;1_", u32::from(unit), u8::from(down))
    };
    let keys = [
        record(b'A', 'a', true),
        record(b'B', 'b', true),
        record(b'A', 'a', false),
        record(b'B', 'b', false),
    ];
    s.send(keys.concat().as_bytes());
    wait_logged(&log, r"ab\x1b[97;1:3u\x1b[98;1:3u");
}

/// With the terminal reporting releases (asked for flag 2 by the program
/// in front), a bind runs once a press, its release going nowhere: F5's
/// presses each open one window and the program sees none of F5; a held
/// prefix takes the next press, not the prefix's own release. A repeat
/// runs the bind as a press does.
#[test]
fn binds_run_once_a_press_with_releases_on() {
    let t = Tmxr::new("bindrelease");
    t.run(&["new-session", "-d", "-s", "br"]);
    t.run(&["bind", "-n", "F5", "new-window", "-d"]);
    t.run(&["bind", "c", "new-window", "-d"]);
    let s = t.attach(&["attach", "-t", "br"]);
    s.wait_for("status line", |text| text.contains("br"));
    let log = key_log_pane(&t, "br", "bindpane", 1 | 2, false);
    s.wait_kitty_flags(TMXR_KITTY_FLAGS | 2);
    // Press, release; press, repeat, release.
    s.send(b"\x1b[15~\x1b[15;1:3~");
    s.send(b"\x1b[15~\x1b[15;1:2~\x1b[15;1:3~");
    // C-b, its release, then c and its release.
    s.send(b"\x1b[98;5u\x1b[98;5:3uc\x1b[99;1:3u");
    // Then a key for the program: once it has it, every key before it has
    // been handled, and it is all the program saw.
    s.wait_kitty_flags(TMXR_KITTY_FLAGS | 2);
    s.send(b"\x1b[97;5u\x1b[97;5:3u");
    wait_logged(&log, r"\x1b[97;5u\x1b[97;5:3u");
    let windows = t.run(&["list-windows", "-t", "br", "-F", "#{window_index}"]);
    assert_eq!(windows.lines().count(), 6, "{windows}");
}

/// A key a program saw pressed is released for it when the terminal stops
/// reporting releases: as focus moves to a window whose program asked for
/// none, and as the client detaches. A second press of a key still held
/// (its release lost) releases it first.
#[test]
fn held_keys_are_released_when_releases_stop() {
    let t = Tmxr::new("heldrelease");
    t.run(&["new-session", "-d", "-s", "hr"]);
    let mut s = t.attach(&["attach", "-t", "hr"]);
    s.wait_for("status line", |text| text.contains("hr"));
    let log = key_log_pane(&t, "hr", "heldpane", 1 | 2, false);
    s.wait_kitty_flags(TMXR_KITTY_FLAGS | 2);
    // Ctrl+a pressed twice with no release between.
    s.send(b"\x1b[97;5u\x1b[97;5u");
    let twice = r"\x1b[97;5u\x1b[97;5:3u\x1b[97;5u";
    wait_logged(&log, twice);
    // Focus moves to the shell: the held Ctrl+a is released.
    t.run(&["select-window", "-t", "hr:0"]);
    let moved = format!(r"{twice}\x1b[97;5:3u");
    wait_logged(&log, &moved);
    s.wait_kitty_flags(TMXR_KITTY_FLAGS);
    // Back, Ctrl+x pressed and held, then the client goes.
    t.run(&["select-window", "-t", "hr:1"]);
    s.wait_kitty_flags(TMXR_KITTY_FLAGS | 2);
    s.send(b"\x1b[120;5u");
    let pressed = format!(r"{moved}\x1b[120;5u");
    wait_logged(&log, &pressed);
    t.run(&["detach-client", "-s", "hr"]);
    s.wait_exit();
    wait_logged(&log, &format!(r"{pressed}\x1b[120;5:3u"));
}

/// A bind on a key only the kitty protocol reports fires from a kitty
/// terminal, however the key was named: Super by kitty's name, tmxr's and
/// hjkl's (`super+k`, `Super-l`, `<D-J>`, Super+Shift+j), Super+Up in its
/// legacy form, kitty's own Meta, F13, the keypad's Enter apart from Enter,
/// and a media key. Modifier bit 8 is Super only as the client's reader
/// knows the flags it pushed are on; else it is xterm's Meta, `M-`.
#[test]
fn a_bind_on_a_kitty_only_key_fires_from_a_kitty_terminal() {
    let t = Tmxr::new("kittyonly");
    t.run(&["new-session", "-d", "-s", "ko"]);
    for (key, name) in [
        ("super+k", "super-k"),
        ("Super-l", "super-l"),
        ("<D-J>", "super-shift-j"),
        ("Super-Up", "super-up"),
        ("M-Up", "alt-up"),
        ("Meta-x", "meta-x"),
        ("M-x", "alt-x"),
        ("F13", "f13"),
        ("Enter", "enter"),
        ("kp_enter", "kp-enter"),
        ("MediaPlay", "media-play"),
    ] {
        t.run(&["bind", "-n", key, "rename-window", name]);
    }
    let s = t.attach(&["attach", "-t", "ko"]);
    s.wait_for("status line", |text| text.contains("ko"));
    s.wait_kitty_flags(TMXR_KITTY_FLAGS);
    for (key, want) in [
        (&b"\x1b[107;9u"[..], "super-k"),
        (b"\x1b[108;9u", "super-l"),
        (b"\x1b[106:74;10u", "super-shift-j"),
        (b"\x1b[1;9A", "super-up"),
        (b"\x1b[1;3A", "alt-up"),
        (b"\x1b[120;33u", "meta-x"),
        (b"\x1b[120;3u", "alt-x"),
        (b"\x1b[1;9A", "super-up"),
        (b"\x1b[57376u", "f13"),
        (b"\x1b[57414u", "kp-enter"),
        (b"\r", "enter"),
        (b"\x1b[57414u", "kp-enter"),
        (b"\x1b[57428u", "media-play"),
    ] {
        s.send(key);
        t.wait_run(
            &["display-message", "-p", "-t", "ko", "#{window_name}"],
            &format!("window named {want}"),
            |o| o.trim() == want,
        );
    }
}
