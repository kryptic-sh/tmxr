use super::*;

#[test]
fn respawn_window_finds_its_window_by_index() {
    let t = Tmxr::new("respawnw");
    t.run(&["new-session", "-d", "-s", "rw"]);
    // Index 7, id 1: they differ, which the lookup once mixed up.
    t.run(&["new-window", "-d", "-t", "rw:7"]);
    let id = t
        .run(&["display-message", "-p", "-t", "rw:7", "#{window_id}"])
        .trim()
        .to_owned();
    let out = t.output(&["respawn-window", "-k", "-t", "rw:7"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "rw:7", "#{window_id}"])
            .trim(),
        id
    );
    assert_eq!(t.run(&["list-windows", "-t", "rw"]).lines().count(), 2);
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
        !t.output(&["set-window-option", "-t", "rw", "window-size", "biggest"])
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
        t.run(&["show-options", "-gv", "monitor-activity"]).trim(),
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
    // No client is looking at the current window, so it is flagged too, as
    // tmux 3.6 does; `alerts_follow_the_action_and_visual_options` covers a
    // window in sight.
    t.wait_prompt("al:0");
    t.run(&["send-keys", "-t", "al:0", "echo here", "Enter"]);
    t.wait_run(
        &["display-message", "-p", "-t", "al:0", "#{window_flags}"],
        "activity in the unseen current window",
        |o| o.contains('#'),
    );
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

/// `#{pane_current_command}` names a program as soon as it is in front, as
/// tmux reads it on each expansion, not at the next refresh of the commands
/// tmxr keeps for automatic renames. Unix only: Windows reads the kept copy, a
/// lookup there snapshotting every process.
#[cfg(unix)]
#[test]
fn the_current_command_is_read_when_asked() {
    let t = Tmxr::new("curcmd");
    t.run(&["new-session", "-d", "-s", "cc"]);
    t.wait_prompt("cc");
    // A refresh keeps the shell's name, then the next is put far off.
    t.run(&["set-option", "-g", "status-interval", "1"]);
    std::thread::sleep(Duration::from_millis(1500));
    t.run(&["set-option", "-g", "status-interval", "60"]);
    t.run(&["send-keys", "-t", "cc", "sleep 30", "Enter"]);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let name = t.run(&[
            "display-message",
            "-p",
            "-t",
            "cc",
            "#{pane_current_command}",
        ]);
        if name.trim() == "sleep" {
            break;
        }
        assert!(Instant::now() < deadline, "still named {name:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `session_bell_flag`, `session_activity_flag` and `session_silence_flag`
/// are set when any of the session's windows has the alert, whichever window
/// it is (tmux 3.6 looks at one window only), and not in the session showing
/// the window, which is not alerted.
#[cfg(unix)]
#[test]
fn session_alert_flags_look_at_every_window() {
    let t = Tmxr::new("sessionflags");
    t.run(&[
        "new-session",
        "-d",
        "-s",
        "o",
        r#"read x; printf "\a"; exec sleep 1000"#,
    ]);
    let s = t.attach(&["attach", "-t", "o"]);
    s.wait_for("the status line", |text| text.contains("o"));
    t.run(&["set-option", "-g", "monitor-activity", "on"]);
    t.run(&["new-session", "-d", "-s", "p"]);
    t.run(&["new-window", "-d", "-t", "p:3"]);
    t.run(&["link-window", "-d", "-s", "o:0", "-t", "p:5"]);
    t.run(&["send-keys", "-t", "o:0", "Enter"]);
    let flags = "#{session_bell_flag}#{session_activity_flag}#{session_silence_flag}";
    t.wait_run(
        &["display-message", "-p", "-t", "p", flags],
        "p's flags",
        |o| o.trim() == "110",
    );
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "o", flags]).trim(),
        "000"
    );
}

#[test]
fn allow_rename_lets_programs_name_their_window() {
    // The name string is found by the splitter passthrough uses, which needs
    // Microsoft's ConPTY beside tmxr.exe on Windows (see the passthrough
    // test); without it there is nothing to check.
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_tmxr"));
    if cfg!(windows) && !bin.with_file_name("conpty.dll").is_file() {
        return;
    }
    let t = Tmxr::new("allowrename");
    t.run(&["new-session", "-d", "-s", "ar", "-n", "start"]);
    // A program that names its window, then prints `mark`.
    #[cfg(unix)]
    let namer = |name: &str, mark: &str| {
        let script = format!(r"printf '\033k{name}\033\\{mark}'; sleep 30");
        vec!["/bin/sh".to_owned(), "-c".to_owned(), script]
    };
    #[cfg(windows)]
    let namer = |name: &str, mark: &str| {
        let script = format!(
            "$e=[char]27; [Console]::Out.Write($e+'k{name}'+$e+'\\'+'{mark}'); Start-Sleep 30"
        );
        vec![
            "powershell.exe".to_owned(),
            "-NoProfile".to_owned(),
            "-Command".to_owned(),
            script,
        ]
    };
    let name = |t: &Tmxr| {
        t.run(&["display-message", "-p", "-t", "ar:0", "#{window_name}"])
            .trim()
            .to_owned()
    };
    let respawn = |t: &Tmxr, n: &str, mark: &str| {
        let mut args = vec![
            "respawn-pane".to_owned(),
            "-k".to_owned(),
            "-t".to_owned(),
            "ar:0".to_owned(),
        ];
        args.extend(namer(n, mark));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        t.run(&args);
        t.wait_run(&["capture-pane", "-p", "-t", "ar:0"], mark, |o| {
            o.contains(mark)
        });
    };
    // Off, tmux's default: the name stays, and the string is not printed.
    respawn(&t, "by-prog", "mark-off");
    assert_eq!(name(&t), "start");
    let screen = t.run(&["capture-pane", "-p", "-t", "ar:0"]);
    assert!(!screen.contains("by-prog"), "{screen}");
    // On: the program names the window, and automatic renaming stops.
    t.run(&["set", "-g", "allow-rename", "on"]);
    respawn(&t, "by-prog", "mark-on");
    assert_eq!(name(&t), "by-prog");
    let auto = t.run(&["show", "-wv", "-t", "ar:0", "automatic-rename"]);
    assert_eq!(auto.trim(), "off");
    // An empty name gives automatic-rename back to the global value.
    t.run(&["set", "-wg", "automatic-rename", "off"]);
    respawn(&t, "", "mark-empty");
    assert_eq!(name(&t), "");
}

#[test]
fn new_windows_panes_and_sessions_print_themselves_with_p() {
    let t = Tmxr::new("printnew");
    t.run(&["new-session", "-d", "-s", "p", "-x", "80", "-y", "24"]);
    // tmux 3.6 prints the same for each (checked against it).
    let run = |args: &[&str]| t.run(args).trim_end().to_owned();
    assert_eq!(run(&["new-window", "-d", "-P", "-t", "p:1"]), "p:1.0");
    assert_eq!(run(&["split-window", "-d", "-P", "-t", "p:1"]), "p:1.1");
    // -d: the new pane, not the active one.
    assert_eq!(
        run(&[
            "split-window",
            "-d",
            "-P",
            "-F",
            "#{window_panes} #{pane_active}",
            "-t",
            "p:1.1"
        ]),
        "3 0"
    );
    assert_eq!(run(&["new-session", "-d", "-P", "-s", "q"]), "q:");
    assert_eq!(
        run(&[
            "new-session",
            "-d",
            "-P",
            "-F",
            "#{session_name}/#{window_index}",
            "-s",
            "r"
        ]),
        "r/0"
    );
    // break-pane prints the window the pane went to.
    let id = run(&["display-message", "-p", "-t", "p:1.2", "#{pane_id}"]);
    assert_eq!(
        run(&[
            "break-pane",
            "-d",
            "-P",
            "-F",
            "#{pane_id} #{window_panes}",
            "-s",
            "p:1.2"
        ]),
        format!("{id} 1")
    );
    // Without -P, nothing.
    assert_eq!(run(&["new-window", "-d", "-t", "p"]), "");
}

#[test]
fn window_size_largest_and_smallest_take_each_way() {
    let t = Tmxr::new("wsize");
    t.run(&["new-session", "-d", "-s", "ws"]);
    let size = |t: &Tmxr| {
        t.run(&[
            "display-message",
            "-p",
            "-t",
            "ws",
            "#{window_width}x#{window_height}",
        ])
        .trim()
        .to_owned()
    };
    let a = t.attach(&["attach", "-t", "ws"]);
    a.wait_for("the first client", |text| text.contains("ws"));
    let b = t.attach(&["attach", "-t", "ws"]);
    b.wait_for("the second client", |text| text.contains("ws"));
    // Narrower but taller than the first, so largest and smallest each
    // take a different client's width and height. A resize just after
    // attaching can go unseen (docs/backlog.md), so again until it lands.
    let deadline = Instant::now() + TIMEOUT;
    loop {
        b.resize(ROWS + 1, 61);
        b.resize(ROWS + 6, 60);
        std::thread::sleep(Duration::from_millis(300));
        let clients = t.run(&["list-clients", "-F", "#{client_width}x#{client_height}"]);
        if clients.lines().any(|l| l == format!("60x{}", ROWS + 6)) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the resize never landed: {clients}"
        );
    }
    t.run(&["set", "-wg", "window-size", "largest"]);
    assert_eq!(size(&t), format!("{COLS}x{}", ROWS + 5));
    t.run(&["set", "-wg", "window-size", "smallest"]);
    assert_eq!(size(&t), format!("60x{}", ROWS - 1));
    drop(a);
}

#[test]
fn a_linked_window_is_sized_by_every_client_or_with_aggressive_resize_its_viewers() {
    let t = Tmxr::new("aggr");
    t.run(&["new-session", "-d", "-s", "aa"]);
    t.run(&["new-session", "-d", "-s", "bb"]);
    // aa:0 linked into bb, which shows its own window.
    t.run(&["link-window", "-s", "aa:0", "-t", "bb:5"]);
    t.run(&["select-window", "-t", "bb:0"]);
    t.run(&["set", "-wg", "window-size", "smallest"]);
    let a = t.attach(&["attach", "-t", "aa"]);
    a.wait_for("the first client", |text| text.contains("aa"));
    let b = t.attach(&["attach", "-t", "bb"]);
    b.wait_for("the second client", |text| text.contains("bb"));
    // The second client narrower: as in window_size_largest_and_smallest.
    let deadline = Instant::now() + TIMEOUT;
    loop {
        b.resize(ROWS + 1, 61);
        b.resize(ROWS, 60);
        std::thread::sleep(Duration::from_millis(300));
        let clients = t.run(&["list-clients", "-F", "#{client_width}"]);
        if clients.lines().any(|l| l == "60") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the resize never landed: {clients}"
        );
    }
    let width = |t: &Tmxr| {
        t.run(&["display-message", "-p", "-t", "aa:0", "#{window_width}"])
            .trim()
            .to_owned()
    };
    // Both sessions hold it, so both clients size it, as tmux's.
    t.wait_run(
        &["display-message", "-p", "-t", "aa:0", "#{window_width}"],
        "both clients' smallest",
        |o| o.trim() == "60",
    );
    // aggressive-resize: only a client showing it counts.
    t.run(&["set", "-w", "-t", "aa:0", "aggressive-resize", "on"]);
    t.run(&["refresh-client", "-t", "aa"]);
    b.resize(ROWS, 59);
    t.wait_run(
        &["display-message", "-p", "-t", "aa:0", "#{window_width}"],
        "the viewer's width",
        |o| o.trim() == COLS.to_string(),
    );
    assert_eq!(width(&t), COLS.to_string());
    drop(a);
}

#[test]
fn the_smallest_client_in_use_sizes_the_window() {
    let t = Tmxr::new("inuse");
    t.run(&["new-session", "-d", "-s", "iu"]);
    let a = t.attach(&["attach", "-t", "iu"]);
    a.wait_for("the first client", |text| text.contains("iu"));
    let b = t.attach(&["attach", "-t", "iu"]);
    b.wait_for("the second client", |text| text.contains("iu"));
    // The second client narrower, as in window_size_largest_and_smallest.
    let deadline = Instant::now() + TIMEOUT;
    loop {
        b.resize(ROWS + 1, 61);
        b.resize(ROWS, 60);
        std::thread::sleep(Duration::from_millis(300));
        let clients = t.run(&["list-clients", "-F", "#{client_width}"]);
        if clients.lines().any(|l| l == "60") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the resize never landed: {clients}"
        );
    }
    let width = |w: &str| {
        let want = w.to_owned();
        t.wait_run(
            &["display-message", "-p", "-t", "iu", "#{window_width}"],
            &format!("width {want}"),
            move |o| o.trim() == want,
        );
    };
    // smallest by default: both in use, the narrower one sizes it.
    width("60");
    // Out of focus, the narrower one stops counting; back, it counts again.
    b.send(b"\x1b[O");
    width(&COLS.to_string());
    b.send(b"\x1b[I");
    width("60");
    // Away: no input for client-away-time, while the other keeps typing.
    t.run(&["set", "-s", "client-away-time", "1"]);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        a.send(b"x");
        std::thread::sleep(Duration::from_millis(300));
        let w = t.run(&["display-message", "-p", "-t", "iu", "#{window_width}"]);
        if w.trim() == COLS.to_string() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the away client still sizes it: {w}"
        );
    }
    // Typing again brings it back.
    b.send(b"y");
    width("60");
}

#[test]
fn an_equals_sign_asks_for_the_exact_name() {
    let t = Tmxr::new("exact");
    t.run(&["new-session", "-d", "-s", "foobar"]);
    t.run(&["new-window", "-d", "-t", "foobar", "-n", "main"]);
    let fails = |args: &[&str], why: &str| {
        let out = t.output(args);
        assert!(!out.status.success(), "{args:?} succeeded");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(why), "{args:?}: {err}");
    };
    // Without `=` a prefix finds the session or window; with it, not.
    assert!(t.output(&["has-session", "-t", "foo"]).status.success());
    fails(&["has-session", "-t", "=foo"], "can't find session: foo");
    assert!(
        t.output(&["select-window", "-t", "foobar:mai"])
            .status
            .success()
    );
    fails(
        &["select-window", "-t", "foobar:=mai"],
        "can't find window: mai",
    );
}
