//! The xterm keyboard tier (plan 18 step 5): a terminal tmux gives the
//! `extkeys` feature, by its XTVERSION name or by `terminal-features`, is
//! asked for xterm's modifyOtherKeys under `extended-keys`, and the
//! `CSI 27 ; mods ; key ~` keys it then sends reach binds and panes.
//!
//! Unix only, all of it: the Windows client reads the console's key
//! records through crossterm, asks the console no XTVERSION, and tells the
//! server it cannot read these keys, so it is never asked for the mode
//! (the server's side of that is unit-tested on every platform:
//! `terminal_features_give_a_terminal_by_term_the_xterm_tier`); ConPTY
//! does not pass `CSI 27 ; … ~` keys either.

#![cfg(unix)]

use super::*;
use tmxr_term::encode::TMXR_KITTY_FLAGS;

/// How many times the terminal was asked to turn modifyOtherKeys on and
/// off.
fn modify_other_keys(s: &Screen) -> (usize, usize) {
    let raw = s.raw_text();
    (
        raw.matches(MODIFY_OTHER_KEYS_ON).count(),
        raw.matches(MODIFY_OTHER_KEYS_OFF).count(),
    )
}

/// Wait for a frame drawn after everything run so far, so the client was
/// told whatever the server tells it for it.
fn drawn(t: &Tmxr, s: &Screen, what: &str) {
    t.run(&["display-message", "-d", "0", what]);
    s.wait_for(what, |text| text.contains(what));
}

/// A terminal that answers XTVERSION as xterm does is asked for
/// modifyOtherKeys level 2 at attach, as tmux asks it, off when
/// `extended-keys` goes `off` and on when it comes back, and off on
/// detach, on the alternate screen before it is left. It is never asked
/// for kitty flags.
#[test]
fn an_xterm_terminal_gets_modify_other_keys_under_extended_keys() {
    let t = Tmxr::new("xtermmok");
    t.run(&["new-session", "-d", "-s", "xm"]);
    let mut s = t.attach_xterm(&["attach", "-t", "xm"], false);
    s.wait_for("status line", |text| text.contains("xm"));
    s.wait_for_raw("modifyOtherKeys on", |_| modify_other_keys(&s) == (1, 0));
    t.run(&["set", "-s", "extended-keys", "off"]);
    s.wait_for_raw("modifyOtherKeys off", |_| modify_other_keys(&s) == (1, 1));
    t.run(&["set", "-s", "extended-keys", "on"]);
    s.wait_for_raw("modifyOtherKeys on again", |_| {
        modify_other_keys(&s) == (2, 1)
    });
    // `always` asks the terminal no differently: the counts once it has
    // gone, with everything it was sent, show no more than the detach.
    t.run(&["set", "-s", "extended-keys", "always"]);
    t.run(&["detach-client", "-s", "xm"]);
    s.wait_exit();
    assert_eq!(modify_other_keys(&s), (2, 2));
    assert_eq!(s.kitty_requests(), Vec::<u8>::new());
    let raw = s.raw_text();
    let off = raw.rfind(MODIFY_OTHER_KEYS_OFF).unwrap();
    let left = raw.rfind("\x1b[?1049l").unwrap();
    assert!(off < left, "turned off after leaving the alternate screen");
}

/// A terminal that answers neither the kitty query nor XTVERSION with a
/// name tmux knows (this one answers as tmxr) is asked for neither
/// protocol, whatever `extended-keys` says, nor told to turn either off.
#[test]
fn a_legacy_terminal_gets_neither_protocol() {
    let t = Tmxr::new("legacymok");
    t.run(&["new-session", "-d", "-s", "lm"]);
    let mut s = t.attach_legacy(&["attach", "-t", "lm"]);
    s.wait_for("status line", |text| text.contains("lm"));
    assert!(
        s.raw_text().contains("\x1b[>q"),
        "the client asked the terminal's name"
    );
    for setting in ["off", "on", "always"] {
        t.run(&["set", "-s", "extended-keys", setting]);
        drawn(&t, &s, &format!("drawn-{setting}"));
    }
    t.run(&["detach-client", "-s", "lm"]);
    s.wait_exit();
    assert_eq!(modify_other_keys(&s), (0, 0));
    assert_eq!(s.kitty_requests(), Vec::<u8>::new());
}

/// A terminal that answers both the kitty query and XTVERSION as xterm is
/// a kitty terminal: it gets kitty flags and no modifyOtherKeys, as the
/// kitty tier wins; under `kitty-keys off` it falls to the xterm tier. As
/// it moves between them, one mode is turned off before the other comes
/// on, so it never has both.
#[test]
fn a_terminal_with_both_protocols_gets_kitty_flags() {
    let t = Tmxr::new("bothmok");
    t.run(&["new-session", "-d", "-s", "bm"]);
    let mut s = t.attach_xterm(&["attach", "-t", "bm"], true);
    s.wait_for("status line", |text| text.contains("bm"));
    s.wait_kitty_flags(TMXR_KITTY_FLAGS);
    t.run(&["set", "-s", "kitty-keys", "off"]);
    s.wait_kitty_flags(0);
    s.wait_for_raw("modifyOtherKeys on", |_| modify_other_keys(&s) == (1, 0));
    t.run(&["set", "-s", "kitty-keys", "on"]);
    s.wait_kitty_flags(TMXR_KITTY_FLAGS);
    s.wait_for_raw("modifyOtherKeys off", |_| modify_other_keys(&s) == (1, 1));
    t.run(&["detach-client", "-s", "bm"]);
    s.wait_exit();
    // Once only, with everything it was sent: never as a kitty terminal.
    assert_eq!(modify_other_keys(&s), (1, 1));
    let raw = s.raw_text();
    let on = raw.find(MODIFY_OTHER_KEYS_ON).unwrap();
    let off = raw.rfind(MODIFY_OTHER_KEYS_OFF).unwrap();
    let push = format!("\x1b[>{TMXR_KITTY_FLAGS}u");
    let none = "\x1b[>0u";
    assert!(
        raw[..on].rfind(none) > raw[..on].rfind(&push),
        "flags off first: {raw:?}"
    );
    assert!(
        raw[off..].contains(&push),
        "flags back after it was off: {raw:?}"
    );
}

/// `terminal-features` gives `extkeys` by `TERM` to a terminal whose name
/// does not: one that answers no XTVERSION as tmux knows it is asked for
/// modifyOtherKeys once an entry matches its `TERM`, set before it attaches
/// or while it is attached, and no more once none does.
#[test]
fn terminal_features_give_extkeys_by_term() {
    let t = Tmxr::new("featmok");
    t.run(&["new-session", "-d", "-s", "fm"]);
    t.run(&["set", "-as", "terminal-features", ",foo*:title:extkeys"]);
    // A terminal without the kitty protocol, answering XTVERSION as tmxr.
    let attach = |term: &str| {
        t.terminal(
            t.dir.path(),
            env!("CARGO_BIN_EXE_tmxr"),
            &t.args(&["attach", "-t", "fm"]),
            &[("TERM", Some(term))],
            Answers {
                kitty: false,
                ..Answers::default()
            },
        )
    };
    let mut s = attach("foo-256color");
    s.wait_for("status line", |text| text.contains("fm"));
    s.wait_for_raw("modifyOtherKeys on", |_| modify_other_keys(&s) == (1, 0));
    t.run(&["set", "-su", "terminal-features"]);
    s.wait_for_raw("modifyOtherKeys off", |_| modify_other_keys(&s) == (1, 1));
    t.run(&["detach-client", "-s", "fm"]);
    s.wait_exit();
    // A TERM no entry matches, until one does: on once, then off as it
    // detaches, with nothing before.
    let mut s = attach("bar-256color");
    s.wait_for("status line", |text| text.contains("fm"));
    t.run(&["set", "-s", "terminal-features[9]", "bar*:extkeys"]);
    s.wait_for_raw("modifyOtherKeys on", |_| modify_other_keys(&s) == (1, 0));
    t.run(&["detach-client", "-s", "fm"]);
    s.wait_exit();
    assert_eq!(modify_other_keys(&s), (1, 1));
}

/// The keys an xterm-tier terminal sends under modifyOtherKeys
/// (`CSI 27 ; mods ; key ~`) reach binds with their modifiers: Ctrl+i is
/// C-i, not Tab; Alt+a is M-a with no bare Escape in front; Ctrl+Shift+h
/// is C-S-h, a bind of its own apart from C-h's (which the legacy byte
/// 0x08 still fires), as tmux 3.6 keeps them.
#[test]
fn xterm_keys_reach_binds_with_their_modifiers() {
    let t = Tmxr::new("xtermbinds");
    t.run(&["new-session", "-d", "-s", "xb"]);
    t.run(&["bind", "-n", "C-i", "rename-window", "ctrl-i"]);
    t.run(&["bind", "-n", "Tab", "rename-window", "tab"]);
    t.run(&["bind", "-n", "M-a", "rename-window", "alt-a"]);
    t.run(&["bind", "-n", "C-h", "rename-window", "ctrl-h"]);
    t.run(&["bind", "-n", "C-S-h", "rename-window", "ctrl-shift-h"]);
    let s = t.attach_xterm(&["attach", "-t", "xb"], false);
    s.wait_for("status line", |text| text.contains("xb"));
    s.wait_for_raw("modifyOtherKeys on", |_| modify_other_keys(&s) == (1, 0));
    let name = |want: &str| {
        t.wait_run(
            &["display-message", "-p", "-t", "xb", "#{window_name}"],
            &format!("window named {want}"),
            |o| o.trim() == want,
        );
    };
    for (key, want) in [
        (&b"\x1b[27;5;105~"[..], "ctrl-i"),
        (b"\t", "tab"),
        (b"\x1b[27;3;97~", "alt-a"),
        (b"\x1b[27;6;72~", "ctrl-shift-h"),
        (b"\x08", "ctrl-h"),
        (b"\x1b[27;6;72~", "ctrl-shift-h"),
        (b"\x1b[27;5;104~", "ctrl-h"),
        (b"\x1b[27;5;105~", "ctrl-i"),
    ] {
        s.send(key);
        name(want);
    }
}

/// The same keys reach a pane's program that asked for modifyOtherKeys
/// level 2 as the server's encoder sends them under each `extended-keys`
/// setting: their legacy bytes under `off`, which ignores the program's
/// request, and xterm's form under `on` and `always`, Ctrl+Shift+h apart
/// from Ctrl+h and Ctrl+i from Tab. (Ctrl+Shift+h goes out with the
/// unshifted `h`, 104, as from a kitty terminal; tmux sends the `H` xterm
/// reported, 72.)
#[test]
fn xterm_keys_reach_a_pane_under_each_extended_keys_setting() {
    let t = Tmxr::new("xtermpane");
    t.run(&["new-session", "-d", "-s", "xp"]);
    // To binds it is C-h, which the root table would take.
    t.run(&["unbind", "-n", "C-h"]);
    let s = t.attach_xterm(&["attach", "-t", "xp"], false);
    s.wait_for("status line", |text| text.contains("xp"));
    // Ctrl+Shift+h, then Ctrl+h as a legacy terminal sends it; Ctrl+i,
    // then Tab; Alt+a.
    let keys: [&[u8]; 5] = [
        b"\x1b[27;6;72~",
        b"\x08",
        b"\x1b[27;5;105~",
        b"\t",
        b"\x1b[27;3;97~",
    ];
    for (setting, want) in [
        ("off", [r"\x08", r"\x08", r"\t", r"\t", r"\x1ba"]),
        (
            "on",
            [
                r"\x1b[27;6;104~",
                r"\x1b[27;5;104~",
                r"\x1b[27;5;105~",
                r"\t",
                r"\x1b[27;3;97~",
            ],
        ),
        (
            "always",
            [
                r"\x1b[27;6;104~",
                r"\x1b[27;5;104~",
                r"\x1b[27;5;105~",
                r"\t",
                r"\x1b[27;3;97~",
            ],
        ),
    ] {
        t.run(&["set", "-s", "extended-keys", setting]);
        let name = format!("mok{setting}");
        let log = t.dir.path().join(format!("{name}.log"));
        let argv = key_log_program(&t, &name);
        let env = format!("{KEY_LOG}={}", log.display());
        let raw = format!(r"{KEY_LOG_RAW}=\e[>4;2m");
        let mut new = vec!["new-window", "-t", "xp", "-e", &env, "-e", &raw];
        new.extend(argv.iter().map(String::as_str));
        t.run(&new);
        t.wait_run(&["capture-pane", "-p", "-t", "xp"], "the mode", |o| {
            o.contains("raw-on")
        });
        // One key at a time, each read before the next is sent.
        for (i, key) in keys.iter().enumerate() {
            s.send(key);
            let deadline = Instant::now() + TIMEOUT;
            loop {
                let got = std::fs::read_to_string(&log).unwrap_or_default();
                let read: Vec<&str> = got.lines().filter(|&l| l != "ready").collect();
                if read == want[..=i] {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "{setting}: not {:?} in {got:?}",
                    &want[..=i]
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// `list-keys -T t` run by a command client inside `session`'s pane, as
/// the shell there prints it.
fn list_keys_in_pane(t: &Tmxr, session: &str) -> String {
    let out = t.dir.path().join(format!("{session}.keys"));
    let cmd = format!(
        "'{}' list-keys -T t > '{}.part' && mv '{}.part' '{}'",
        env!("CARGO_BIN_EXE_tmxr"),
        out.display(),
        out.display(),
        out.display()
    );
    t.run(&["send-keys", "-t", session, &cmd, "Enter"]);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Ok(listed) = std::fs::read_to_string(&out) {
            return listed;
        }
        assert!(Instant::now() < deadline, "no list-keys from {session}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `list-keys` marks each bind the terminal of the attached client that
/// runs it (here from a bind) cannot send: for a legacy client, C-S-h needs
/// extended keys and Super-k kitty keys; for a kitty client, neither. A
/// command client prints no marks, as tmux prints none, whether it runs in
/// a pane either client shows or outside tmxr. Every line stays a
/// `bind-key` command.
#[test]
fn list_keys_marks_the_binds_a_clients_terminal_cannot_send() {
    let t = Tmxr::new("listmarks");
    t.run(&["new-session", "-d", "-s", "lg"]);
    t.run(&["new-session", "-d", "-s", "kt"]);
    t.run(&["bind", "-T", "t", "x", "display", "plain"]);
    t.run(&["bind", "-T", "t", "C-S-h", "display", "ctrl-shift-h"]);
    t.run(&["bind", "-T", "t", "super+k", "display", "super-k"]);
    t.run(&["bind", "-n", "F5", "list-keys", "-T", "t"]);
    let legacy = t.attach_legacy(&["attach", "-t", "lg"]);
    legacy.wait_for("status line", |text| text.contains("lg"));
    let kitty = t.attach(&["attach", "-t", "kt"]);
    kitty.wait_for("status line", |text| text.contains("kt"));
    kitty.wait_kitty_flags(TMXR_KITTY_FLAGS);
    let unmarked = "\
bind-key -T t x       display-message plain
bind-key -T t C-S-h   display-message ctrl-shift-h
bind-key -T t Super-k display-message super-k
";
    assert_eq!(list_keys_in_pane(&t, "lg"), unmarked);
    assert_eq!(list_keys_in_pane(&t, "kt"), unmarked);
    assert_eq!(t.run(&["list-keys", "-T", "t"]), unmarked);
    // F5, as both terminals send it.
    legacy.send(b"\x1b[15~");
    legacy.wait_for("marked binds", |text| {
        text.contains("display-message ctrl-shift-h # needs extended keys")
            && text.contains("display-message super-k # needs kitty keys")
    });
    kitty.send(b"\x1b[15~");
    kitty.wait_for("the binds", |text| text.contains("display-message super-k"));
    assert!(!kitty.text().contains("# needs"), "{}", kitty.text());
}
