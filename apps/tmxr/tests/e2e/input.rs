use super::*;

#[test]
fn prefix_timeout_gives_a_late_key_to_the_root_table() {
    let t = Tmxr::new("pfxtimeout");
    let s = t.attach(&["new", "-s", "pt"]);
    s.wait_for("status line", |text| text.contains("pt"));
    let windows = |t: &Tmxr| t.run(&["list-windows", "-t", "pt"]).lines().count();
    t.run(&["set", "-s", "prefix-timeout", "200"]);
    assert_eq!(t.run(&["show", "-sv", "prefix-timeout"]).trim(), "200");
    // In time, the prefix table takes the key.
    s.send(PREFIX);
    s.send(b"c");
    t.wait_run(&["list-windows", "-t", "pt"], "a second window", |o| {
        o.lines().count() == 2
    });
    // Too late, `c` goes to the pane, as a key in the root table would.
    s.send(PREFIX);
    std::thread::sleep(Duration::from_millis(600));
    s.send(b"c");
    s.send(b"late-key");
    t.wait_run(
        &["capture-pane", "-p", "-t", "pt"],
        "the late key typed",
        |o| o.contains("clate-key"),
    );
    assert_eq!(windows(&t), 2);
    // switch-client -T starts the wait afresh, as the prefix key does.
    std::thread::sleep(Duration::from_millis(300));
    t.run(&["switch-client", "-T", "prefix"]);
    s.send(b"c");
    t.wait_run(&["list-windows", "-t", "pt"], "a third window", |o| {
        o.lines().count() == 3
    });
}

#[test]
fn prefix2_and_key_table_decide_where_keys_go() {
    let t = Tmxr::new("prefix2");
    let s = t.attach(&["new", "-s", "p2"]);
    s.wait_for("the status line", |text| text.contains("p2"));
    let windows = |t: &Tmxr| t.run(&["list-windows", "-t", "p2"]).lines().count();
    // A second prefix works as the first.
    t.run(&["set", "-g", "prefix2", "C-a"]);
    s.send(b"");
    s.send(b"c");
    t.wait_run(
        &["list-windows", "-t", "p2"],
        "a window from prefix2",
        |o| o.lines().count() == 2,
    );
    assert!(
        !t.output(&["set", "-g", "prefix2", "NoSuchKey"])
            .status
            .success()
    );
    // key-table: keys start in that table; the prefix still works.
    t.run(&["bind-key", "-T", "mine", "y", "new-window"]);
    t.run(&["set", "-g", "key-table", "mine"]);
    s.send(b"y");
    t.wait_run(
        &["list-windows", "-t", "p2"],
        "a window from key-table",
        |o| o.lines().count() == 3,
    );
    s.send(PREFIX);
    s.send(b"c");
    t.wait_run(
        &["list-windows", "-t", "p2"],
        "the prefix still works",
        |o| o.lines().count() == 4,
    );
    assert_eq!(windows(&t), 4);
}

#[test]
fn ctrl_hjkl_prefix_hjkl_and_arrows_move_between_panes() {
    let t = Tmxr::new("move");
    // Keys come as bytes here, not as Windows Terminal's key events.
    let s = t.attach_env(&["new", "-s", "nav"], &[("WT_SESSION", None)]);
    s.wait_for("status line", |text| text.contains("nav"));
    pane_grid(&t);
    // The root table's C-h/j/k/l (navigate-pane, a shell in front).
    round_the_grid(&t, &s, "C-", CTRL_HJKL_BYTES);
    // tmux's kitty keys (`CSI codepoint;5u`); ConPTY does not pass these.
    if cfg!(unix) {
        round_the_grid(
            &t,
            &s,
            "kitty C-",
            [
                b"\x1b[104;5u",
                b"\x1b[106;5u",
                b"\x1b[107;5u",
                b"\x1b[108;5u",
            ],
        );
    }
    round_the_grid(&t, &s, "prefix ", [b"\x02h", b"\x02j", b"\x02k", b"\x02l"]);
    round_the_grid(
        &t,
        &s,
        "prefix arrow ",
        [b"\x02\x1b[D", b"\x02\x1b[B", b"\x02\x1b[A", b"\x02\x1b[C"],
    );
    // copy-mode-vi's C-h/j/k/l leave copy mode for the pane they focus.
    for (key, name, from, want) in [
        (CTRL_HJKL_BYTES[0], "h", "4", "3"),
        (CTRL_HJKL_BYTES[1], "j", "4", "7"),
        (CTRL_HJKL_BYTES[2], "k", "4", "1"),
        (CTRL_HJKL_BYTES[3], "l", "4", "5"),
    ] {
        t.run(&["select-pane", "-t", &format!("nav:0.{from}")]);
        t.run(&["copy-mode", "-t", &format!("nav:0.{from}")]);
        t.wait_run(
            &["display-message", "-p", "-t", "nav", "#{pane_in_mode}"],
            "copy mode",
            |o| o.trim() == "1",
        );
        s.send(key);
        t.wait_run(
            &["display-message", "-p", "-t", "nav", "#{pane_index}"],
            &format!("copy mode C-{name} to pane {want}"),
            |o| o.trim() == want,
        );
    }
}

/// Windows Terminal sends keys as win32-input-mode events, not bytes, and
/// sets `WT_SESSION`: C-j and C-h come as themselves.
#[cfg(windows)]
#[test]
fn ctrl_hjkl_move_between_panes_in_windows_terminal() {
    let t = Tmxr::new("move-wt");
    let s = t.attach_env(&["new", "-s", "nav"], &[("WT_SESSION", Some("e2e"))]);
    s.wait_for("status line", |text| text.contains("nav"));
    pane_grid(&t);
    // `CSI Vk;Sc;Uc;Kd;Cs;Rc _`, down then up, with left Ctrl (8) held.
    let win32 = |vk: u8, uc: u8| format!("\x1b[{vk};0;{uc};1;8;1_\x1b[{vk};0;{uc};0;8;1_");
    let keys = [win32(72, 8), win32(74, 10), win32(75, 11), win32(76, 12)];
    let [h, j, k, l] = keys.each_ref().map(|k| k.as_bytes());
    round_the_grid(&t, &s, "win32 C-", [h, j, k, l]);
    // Its Ctrl+Enter and Ctrl+Backspace are keys of their own here.
    for (key, name, event) in [
        ("C-Enter", "enter", win32(13, 10)),
        ("C-BSpace", "bspace", win32(8, 0x7f)),
    ] {
        t.run(&["bind", "-n", key, &format!("set -g @hit {name}")]);
        s.send(event.as_bytes());
        t.wait_run(&["show", "-gv", "@hit"], key, |o| o.trim() == name);
    }
}

#[test]
fn navigator_passes_keys_to_hjkl_and_moves_otherwise() {
    for (name, passes_through) in [("hjkl", true), ("plainprog", false)] {
        let t = Tmxr::new(&format!("nav-{name}"));
        let log = t.dir.path().join("keys.log");
        let argv = key_log_program(&t, name);
        let env = format!("{KEY_LOG}={}", log.display());
        let mut new = vec!["new-session", "-d", "-s", "nav", "-e", &env];
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
fn a_pane_that_asked_is_told_when_it_gains_and_loses_focus() {
    let t = Tmxr::new("focus");
    let log = t.dir.path().join("keys.log");
    let argv = key_log_program(&t, "focuslog");
    let env = format!("{KEY_LOG}={}", log.display());
    let focus = format!("{KEY_LOG_FOCUS}=1");
    let mut new = vec!["new-session", "-d", "-s", "fo", "-e", &env, "-e", &focus];
    new.extend(argv.iter().map(String::as_str));
    t.run(&new);
    for h in ["pane-focus-in", "pane-focus-out"] {
        t.run(&["set-hook", "-g", h, &format!("set -ga @log '{h} '")]);
    }
    let lines = || std::fs::read_to_string(&log).unwrap_or_default();
    let wait_log = |want: &str, count: usize| {
        let deadline = Instant::now() + TIMEOUT;
        while lines().lines().filter(|l| *l == want).count() < count {
            assert!(Instant::now() < deadline, "no {want} x{count}: {}", lines());
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    wait_log("ready", 1);
    // tmux tells a pane nothing when it turns focus reports on, only when
    // its focus changes: attach once the server has the mode.
    t.wait_run(&["capture-pane", "-p", "-t", "fo"], "the mode", |o| {
        o.contains("focus-on")
    });
    // Shown to a client, the active pane gains focus.
    let s = t.attach(&["attach", "-t", "fo"]);
    s.wait_for("the status line", |text| text.contains("fo"));
    wait_log("FocusGained", 1);
    // Another pane made active takes it; selecting this one gives it back.
    t.run(&["split-window", "-h", "-t", "fo"]);
    wait_log("FocusLost", 1);
    t.run(&["select-pane", "-t", "fo.0"]);
    wait_log("FocusGained", 2);
    t.wait_run(&["show", "-gv", "@log"], "the focus hooks", |o| {
        o.contains("pane-focus-out") && o.contains("pane-focus-in")
    });
    // Selecting the shell beside it takes focus away again.
    t.run(&["select-pane", "-t", "fo.1"]);
    wait_log("FocusLost", 2);
}

#[test]
fn the_navigator_hands_c_hjkl_to_the_program_in_front() {
    let t = Tmxr::new("nav-keys");
    let log = t.dir.path().join("keys.log");
    let argv = key_log_program(&t, "hjkl");
    let env = format!("{KEY_LOG}={}", log.display());
    let mut new = vec!["new-session", "-d", "-s", "nav", "-e", &env];
    new.extend(argv.iter().map(String::as_str));
    t.run(&new);
    // Panes on every side, so a key the program did not get would move.
    pane_grid(&t);
    t.run(&["swap-pane", "-s", "nav.0", "-t", "nav.4"]);
    t.run(&["select-pane", "-t", "nav.4"]);
    t.wait_run(
        &[
            "display-message",
            "-p",
            "-t",
            "nav",
            "#{pane_index} #{pane_current_command}",
        ],
        "the fixture in front, in the middle",
        |o| o.trim() == "4 hjkl",
    );
    let read_log = || std::fs::read_to_string(&log).unwrap_or_default();
    let deadline = Instant::now() + TIMEOUT;
    while !read_log().starts_with("ready") {
        assert!(Instant::now() < deadline, "the fixture never started");
        std::thread::sleep(Duration::from_millis(100));
    }
    let s = t.attach_env(&["attach", "-t", "nav"], &[("WT_SESSION", None)]);
    s.wait_for("status line", |text| text.contains("nav"));
    for key in CTRL_HJKL_BYTES {
        s.send(key);
    }
    while read_log().lines().count() < 5 {
        assert!(
            Instant::now() < deadline,
            "keys logged:
{}",
            read_log()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let keys: Vec<String> = read_log().lines().skip(1).map(str::to_owned).collect();
    assert_eq!(
        keys,
        [
            "Char('h') KeyModifiers(CONTROL)",
            "Char('j') KeyModifiers(CONTROL)",
            "Char('k') KeyModifiers(CONTROL)",
            "Char('l') KeyModifiers(CONTROL)",
        ]
    );
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "nav", "#{pane_index}"])
            .trim(),
        "4",
        "focus moved"
    );
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
    // The columns are padded as tmux's: compare the words.
    let keys = t.run(&["list-keys", "-T", "root"]);
    let words = keys.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        words.contains("MouseDown3Pane rename-window clicked") && !keys.contains("WheelUpPane"),
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
    s.wait_for("unbound key", |text| text.contains("unknown key: F5"));
}

#[test]
fn list_keys_refuses_what_tmux_refuses() {
    let t = Tmxr::new("lskerr");
    t.run(&["new-session", "-d", "-s", "le"]);
    // tmux 3.6's messages, seen on WSL (2026-10-20).
    for (args, want) in [
        (&["list-keys", "-N", "F5"][..], "unknown key: F5"),
        (&["list-keys", "F5"][..], "unknown key: F5"),
        (
            &["list-keys", "-T", "nosuch"][..],
            "table nosuch doesn't exist",
        ),
        (
            &["list-keys", "-N", "bogus-key-name"][..],
            "invalid key: bogus-key-name",
        ),
    ] {
        let out = t.output(args);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && err.contains(want),
            "{args:?}: {err}"
        );
    }
}

#[test]
fn clicks_send_the_keys_tmux_sends() {
    let t = Tmxr::new("clickseq");
    let s = t.attach(&["new", "-s", "cs"]);
    s.wait_for("status line", |text| text.contains("cs"));
    s.wait_mouse(true);
    for (key, mark) in [
        ("MouseDown1Pane", "D"),
        ("MouseUp1Pane", "U"),
        ("SecondClick1Pane", "S"),
        ("DoubleClick1Pane", "2"),
        ("TripleClick1Pane", "3"),
    ] {
        t.run(&[
            "bind-key",
            "-n",
            key,
            &format!("set-option -ga @seq {mark}"),
        ]);
    }
    let seq = |t: &Tmxr| {
        t.run(&["display-message", "-p", "#{@seq}"])
            .trim()
            .to_owned()
    };
    // What tmux 3.6 sends, recorded from a running tmux: the second and third
    // presses replace MouseDown, a fourth starts over, and DoubleClick comes
    // from the click timer only when no third press followed.
    let clicks = |t: &Tmxr, n: usize, want: &str| {
        t.run(&["set-option", "-g", "@seq", ""]);
        for _ in 0..n {
            s.click(0, 2, 0);
        }
        t.wait_run(&["display-message", "-p", "#{@seq}"], want, |o| {
            o.trim().len() >= want.len()
        });
        // Past the click time, for a DoubleClick that should not come.
        std::thread::sleep(Duration::from_secs(1));
        assert_eq!(seq(t), want, "{n} clicks");
    };
    clicks(&t, 4, "DUSU3UDU");
    clicks(&t, 2, "DUSU2");
}

#[test]
fn the_wheel_on_the_status_line_and_a_middle_click_do_what_tmuxs_do() {
    let t = Tmxr::new("tmuxmouse");
    let s = t.attach(&["new", "-s", "tm"]);
    s.wait_for("status line", |text| text.contains("tm"));
    s.wait_mouse(true);
    t.run(&["rename-window", "-t", "tm:0", "wheelfirst"]);
    t.run(&["new-window", "-d", "-t", "tm:1", "-n", "wheelsecond"]);
    s.wait_for("both windows listed", |text| text.contains("wheelsecond"));
    let current = |t: &Tmxr| {
        t.run(&["display-message", "-p", "-t", "tm", "#I"])
            .trim()
            .to_owned()
    };
    // Over a window's name, where tmux's Status keys are.
    let cells = s.status_cells();
    let col = (0..cells.len())
        .find(|&i| {
            cells[i..]
                .iter()
                .take(10)
                .map(|c| c.0.as_str())
                .collect::<String>()
                == "wheelfirst"
        })
        .expect("the first window in the status line");
    let col = u16::try_from(col).unwrap();
    // Wheel down then up: the next window, then back.
    s.mouse(65, col, ROWS - 1, false);
    t.wait_run(
        &["display-message", "-p", "-t", "tm", "#I"],
        "next window",
        |o| o.trim() == "1",
    );
    s.mouse(64, col, ROWS - 1, false);
    t.wait_run(
        &["display-message", "-p", "-t", "tm", "#I"],
        "previous window",
        |o| o.trim() == "0",
    );
    assert_eq!(current(&t), "0");
    // The middle button pastes the latest buffer into the pane clicked.
    t.wait_prompt("tm:0");
    t.run(&["set-buffer", "middle-pasted"]);
    s.click(1, 5, 2);
    s.wait_for("the paste", |text| text.contains("middle-pasted"));
}

/// `escape-time`, as tmux's: an Escape with a key soon after is Meta and
/// that key, and a lone Escape goes once the time is up; at 0, an Escape
/// is a key of its own.
#[test]
fn escape_time_makes_escape_and_a_key_meta() {
    let t = Tmxr::new("esctime");
    let s = t.attach(&["new", "-s", "et"]);
    s.wait_for("status line", |text| text.contains("et"));
    t.run(&["bind", "-n", "M-x", "set", "-g", "@meta", "yes"]);
    t.run(&["bind", "-n", "Escape", "set", "-g", "@esc", "yes"]);
    t.run(&["set", "-s", "escape-time", "300"]);
    s.send(b"\x1b");
    std::thread::sleep(Duration::from_millis(50));
    s.send(b"x");
    t.wait_run(&["show", "-gv", "@meta"], "Escape and x as M-x", |o| {
        o.trim() == "yes"
    });
    assert_eq!(
        t.run(&["show", "-gv", "@esc"]).trim(),
        "",
        "no Escape of its own"
    );
    // Alone, it goes as Escape when the time is up.
    s.send(b"\x1b");
    t.wait_run(&["show", "-gv", "@esc"], "the lone Escape", |o| {
        o.trim() == "yes"
    });
    // At 0 the Escape is a key at once, and x after it is just x.
    t.run(&["set", "-s", "escape-time", "0"]);
    t.run(&["set", "-gu", "@meta"]);
    t.run(&["set", "-gu", "@esc"]);
    s.send(b"\x1b");
    t.wait_run(&["show", "-gv", "@esc"], "the Escape at once", |o| {
        o.trim() == "yes"
    });
    std::thread::sleep(Duration::from_millis(50));
    s.send(b"x");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        t.run(&["show", "-gv", "@meta"]).trim(),
        "",
        "x alone is not M-x"
    );
}

/// assume-paste-time times a key from the mouse event before it too, as
/// tmux's: after a click, the first quick key arms the paste and the next
/// is pasted, where timed from the key before the click both are keys.
#[test]
fn a_key_right_after_a_click_counts_toward_a_paste() {
    let t = Tmxr::new("pasteclick");
    let s = t.attach(&["new", "-s", "pc"]);
    s.wait_for("status line", |text| text.contains("pc"));
    t.run(&["set", "-g", "mouse", "on"]);
    t.run(&["set", "-g", "assume-paste-time", "500"]);
    t.run(&["bind", "-n", "x", "set", "-ga", "@hits", "x"]);
    std::thread::sleep(Duration::from_millis(600));
    s.click(0, 2, 2);
    for _ in 0..2 {
        std::thread::sleep(Duration::from_millis(50));
        s.send(b"x");
    }
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        t.run(&["show", "-gv", "@hits"]).trim(),
        "x",
        "the second was pasted"
    );
}

/// assume-paste-time is timed per session, as tmux's SESSION_PASTING: one
/// client's click counts toward a paste typed by another in the session.
#[test]
fn a_paste_is_timed_across_the_sessions_clients() {
    let t = Tmxr::new("pastesess");
    let a = t.attach(&["new", "-s", "ps"]);
    a.wait_for("status line", |text| text.contains("ps"));
    let b = t.attach(&["attach", "-t", "ps"]);
    b.wait_for("status line", |text| text.contains("ps"));
    t.run(&["set", "-g", "mouse", "on"]);
    t.run(&["set", "-g", "assume-paste-time", "500"]);
    t.run(&["bind", "-n", "x", "set", "-ga", "@hits", "x"]);
    std::thread::sleep(Duration::from_millis(600));
    a.click(0, 2, 2);
    for _ in 0..2 {
        std::thread::sleep(Duration::from_millis(50));
        b.send(b"x");
    }
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        t.run(&["show", "-gv", "@hits"]).trim(),
        "x",
        "the second was pasted"
    );
}
