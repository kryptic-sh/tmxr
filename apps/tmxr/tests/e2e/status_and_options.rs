use super::*;

#[test]
fn import_tmux_conf_prints_a_config_that_loads() {
    let t = Tmxr::new("import");
    let conf = t.dir.path().join("tmux.conf");
    std::fs::write(
        &conf,
        "set -g status-left 'IMPORTED '\nbind -n M-x kill-pane\nrun '~/tpm'\n",
    )
    .unwrap();
    let out = tmxr_term::child::command(env!("CARGO_BIN_EXE_tmxr"))
        .arg("--import-tmux-conf")
        .arg(&conf)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("#   run '~/tpm': run-shell is run"), "{text}");
    // The printed file is a config tmxr runs with.
    std::fs::write(&t.config, &text).unwrap();
    t.run(&["new-session", "-d", "-s", "im"]);
    assert_eq!(
        t.run(&["show-options", "-gv", "status-left"]),
        "IMPORTED \n"
    );
    assert!(
        t.run(&["list-keys", "-T", "root", "M-x"])
            .contains("kill-pane")
    );
}

#[test]
fn update_environment_entries_are_patterns() {
    let t = Tmxr::new("envglob");
    // Top-level keys go before the file's first table.
    let config = std::fs::read_to_string(&t.config).unwrap();
    let config = format!("update-environment = [\"E2E_GLOB_*\", \"E2E_NONE\"]\n{config}");
    std::fs::write(&t.config, config).unwrap();
    t.run(&["new-session", "-d", "-s", "eg"]);
    // A new session marks what its client lacks removed, as tmux's; -E
    // takes nothing.
    let env = t.run(&["show-environment", "-t", "eg"]);
    let lines: Vec<&str> = env.lines().map(str::trim).collect();
    for want in ["-E2E_GLOB_*", "-E2E_NONE"] {
        assert!(lines.contains(&want), "{want} missing: {env}");
    }
    t.run(&["new-session", "-d", "-E", "-s", "bare"]);
    let bare = t.run(&["show-environment", "-t", "bare"]);
    assert!(!bare.contains("E2E_"), "{bare}");
    let s = t.attach_env(
        &["attach", "-t", "eg"],
        &[("E2E_GLOB_ONE", Some("1")), ("E2E_GLOB_TWO", Some("2"))],
    );
    s.wait_for("status line", |text| text.contains("eg"));
    let env = t.run(&["show-environment", "-t", "eg"]);
    let lines: Vec<&str> = env.lines().map(str::trim).collect();
    for want in ["E2E_GLOB_ONE=1", "E2E_GLOB_TWO=2", "-E2E_NONE"] {
        assert!(lines.contains(&want), "{want} missing: {env}");
    }
    // A session made by a key's command takes that client's variables.
    s.send(PREFIX);
    s.send(b":new-session -d -s fromkey\r");
    t.wait_run(
        &["show-environment", "-t", "fromkey"],
        "its variables",
        |o| o.lines().any(|l| l.trim() == "E2E_GLOB_ONE=1"),
    );
}

#[test]
fn state_changes_fire_tmuxs_notification_hooks() {
    let t = Tmxr::new("notify");
    t.run(&["new-session", "-d", "-s", "o", "-n", "w0"]);
    let hooks = [
        "session-renamed",
        "window-renamed",
        "session-window-changed",
        "window-pane-changed",
        "window-linked",
        "window-unlinked",
        "pane-mode-changed",
        "window-layout-changed",
        "window-resized",
        "pane-died",
        "pane-title-changed",
    ];
    for h in hooks {
        t.run(&["set-hook", "-g", h, &format!("set -ga @log '{h} '")]);
    }
    // Each step fires the hook tmux 3.6 fires for it (checked 2026-10-18).
    let step = |args: &[&str], want: &str| {
        t.run(&["set", "-g", "@log", ""]);
        t.run(args);
        t.wait_run(&["show", "-gv", "@log"], want, |o| {
            o.split_whitespace().any(|h| h == want)
        });
    };
    step(&["rename-session", "-t", "o", "p"], "session-renamed");
    step(&["rename-window", "-t", "p:0", "x"], "window-renamed");
    step(
        &["new-window", "-d", "-t", "p:1", "-n", "w1"],
        "window-linked",
    );
    step(&["select-window", "-t", "p:1"], "session-window-changed");
    step(
        &["split-window", "-d", "-t", "p:1"],
        "window-layout-changed",
    );
    step(&["select-pane", "-t", "p:1.1"], "window-pane-changed");
    step(&["copy-mode", "-t", "p:1.1"], "pane-mode-changed");
    step(
        &["send-keys", "-t", "p:1.1", "-X", "cancel"],
        "pane-mode-changed",
    );
    // tmxr renumbers windows by default, so from here on by name.
    step(&["kill-window", "-t", "p:0"], "window-unlinked");
    step(
        &["resize-window", "-t", "p:w1", "-x", "50"],
        "window-resized",
    );
    // A pane kept by remain-on-exit dies; it does not exit.
    t.run(&["set", "-g", "remain-on-exit", "on"]);
    let exit3 = if cfg!(windows) {
        "cmd /c exit 3"
    } else {
        "exit 3"
    };
    step(&["respawn-pane", "-k", "-t", "p:w1.0", exit3], "pane-died");
    step(
        &["select-pane", "-t", "p:w1.1", "-T", "byhand"],
        "pane-title-changed",
    );
    // As tmux keeps them: the window and pane hooks with the global window
    // options, the rest with the global session options.
    let window = t.run(&["show", "-wgH"]);
    let session = t.run(&["show", "-gH"]);
    for h in hooks {
        let listed = |l: &str| l.starts_with(&format!("{h}["));
        let in_window = h.starts_with("pane-") || (h.starts_with("window-") && !h.contains("link"));
        assert_eq!(window.lines().any(listed), in_window, "{h} with -wg");
        assert_eq!(session.lines().any(listed), !in_window, "{h} with -g");
    }
}

#[test]
fn a_session_created_hook_from_the_config_adds_windows() {
    let t = Tmxr::new("hookcfg");
    let mut config = std::fs::read_to_string(&t.config).unwrap();
    config.push_str(
        "[hooks]
session-created = [\"new-window -d\", \"new-window -d\"]
",
    );
    std::fs::write(&t.config, config).unwrap();
    let proj = t.dir.path().join("proj");
    std::fs::create_dir(&proj).unwrap();
    let mut s = t.attach_in(&proj, &[]);
    s.wait_for("status line", |text| text.contains("proj"));
    let windows = |t: &Tmxr| {
        t.run(&[
            "list-windows",
            "-t",
            "proj",
            "-F",
            "#{window_index}#{?window_active,*,}",
        ])
    };
    t.wait_run(
        &["list-windows", "-t", "proj", "-F", "#{window_index}"],
        "three windows",
        |o| o.lines().count() == 3,
    );
    assert_eq!(
        windows(&t).split_whitespace().collect::<Vec<_>>(),
        ["0*", "1", "2"]
    );
    t.wait_prompt("proj:2");
    let path = t.run(&["display", "-p", "-t", "proj:2", "#{pane_current_path}"]);
    assert!(path.trim().ends_with("proj"), "{path}");
    assert_eq!(
        t.run(&["show-hooks", "-g"]).trim(),
        "session-created[0] new-window -d
session-created[1] new-window -d"
    );

    // Attaching again runs no hook: still three windows.
    s.send(PREFIX);
    s.send(b"d");
    s.wait_exit();
    let s = t.attach_in(&proj, &[]);
    s.wait_for("status line", |text| text.contains("proj"));
    assert_eq!(windows(&t).lines().count(), 3);
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
fn window_entries_take_their_styles_and_separators() {
    let t = Tmxr::new("wstyle");
    let s = t.attach(&["new", "-s", "ws", "-n", "a"]);
    s.wait_for("the status line", |text| text.contains("ws"));
    t.run(&["new-window", "-t", "ws", "-n", "b"]);
    t.run(&["new-window", "-t", "ws", "-n", "c"]);
    for (opt, value) in [
        ("status-left", ""),
        ("status-right", ""),
        ("status-style", "bg=black"),
    ] {
        t.run(&["set", "-g", opt, value]);
    }
    // The separator last: the list's text shows only once every style
    // before it is in.
    for (opt, value) in [
        ("window-status-style", "bg=red"),
        ("window-status-current-style", "bg=green"),
        ("window-status-last-style", "bg=blue"),
        ("window-status-format", "<#I>"),
        ("window-status-current-format", "<#I>"),
        ("window-status-separator", "|"),
    ] {
        t.run(&["set", "-wg", opt, value]);
    }
    s.wait_for("the window list", |text| text.contains("<0>|<1>|<2>"));
    assert_eq!(
        t.run(&["list-windows", "-t", "ws", "-F", "#I#{window_last_flag}"]),
        "00\n11\n20\n"
    );
    // Window 2 is current, 1 the last; the separators keep the line's.
    let row = ROWS - 1;
    let line = s.text().lines().last().unwrap_or_default().to_owned();
    let bg = |c: &str| {
        let col = line.find(c).unwrap() as u16;
        // vt100's colour by its debug form: the tests do not depend on it.
        let emu = s.emu.lock().unwrap();
        format!("{:?}", emu.screen().cell(row, col).unwrap().bgcolor())
    };
    assert_eq!(bg("0"), "Idx(1)", "window-status-style");
    assert_eq!(bg("1"), "Idx(4)", "window-status-last-style");
    assert_eq!(bg("2"), "Idx(2)", "window-status-current-style");
    assert_eq!(bg("|"), "Idx(0)", "the status line's own");
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
fn alerts_follow_the_action_and_visual_options() {
    let t = Tmxr::new("alerts");
    // Rings, then says so where capture-pane can see it.
    #[cfg(unix)]
    let ring = ["/bin/sh", "-c", r"printf '\007rung'; sleep 30"];
    #[cfg(windows)]
    let ring = [
        "powershell.exe",
        "-NoProfile",
        "-Command",
        "[Console]::Out.Write([string][char]7 + 'rung'); Start-Sleep 30",
    ];
    t.run(&["new-session", "-d", "-s", "va"]);
    t.run(&["new-window", "-d", "-t", "va:1"]);
    let s = t.attach(&["attach", "-t", "va:0"]);
    s.wait_for("status line", |text| text.contains("va"));
    t.run(&["set-hook", "-g", "alert-bell", "set-option -ga @rang x"]);
    let rang = |t: &Tmxr| {
        t.run(&["display-message", "-p", "#{@rang}"])
            .trim()
            .to_owned()
    };
    let flags = |t: &Tmxr, w: &str| {
        t.run(&["display-message", "-p", "-t", w, "#{window_flags}"])
            .trim()
            .to_owned()
    };
    let ring_in = |t: &Tmxr, w: &str| {
        let mut cmd = vec!["respawn-window", "-k", "-t", w];
        cmd.extend(ring);
        t.run(&cmd);
    };
    // A bare BEL reaching the client's terminal, outside any OSC string.
    let bells = |s: &Screen| {
        let raw = s.raw_text();
        let mut n = 0;
        let mut in_osc = false;
        let mut prev = '\0';
        for c in raw.chars() {
            match c {
                ']' if prev == '\x1b' => in_osc = true,
                '\x07' if in_osc => in_osc = false,
                '\\' if prev == '\x1b' => in_osc = false,
                '\x07' => n += 1,
                _ => {}
            }
            prev = c;
        }
        n
    };

    // visual-bell on: a message, not the terminal's bell.
    t.run(&["set-option", "-g", "visual-bell", "on"]);
    let before = bells(&s);
    ring_in(&t, "va:1");
    s.wait_for("the message", |text| text.contains("Bell in window 1"));
    t.wait_run(&["display-message", "-p", "#{@rang}"], "the hook", |o| {
        o.trim() == "x"
    });
    assert!(flags(&t, "va:1").contains('!'), "{}", flags(&t, "va:1"));
    assert_eq!(bells(&s), before, "visual-bell on rang the terminal");

    // both, in the window in sight: message and bell, and no flag.
    t.run(&["set-option", "-g", "visual-bell", "both"]);
    let before = bells(&s);
    ring_in(&t, "va:0");
    s.wait_for("the message", |text| {
        text.contains("Bell in current window")
    });
    s.wait_for("the terminal's bell", |_| bells(&s) > before);
    assert!(!flags(&t, "va:0").contains('!'), "{}", flags(&t, "va:0"));

    // bell-action other: a bell in sight does nothing at all.
    t.run(&["set-option", "-g", "bell-action", "other"]);
    let before = (bells(&s), rang(&t));
    ring_in(&t, "va:0");
    t.wait_run(&["capture-pane", "-p", "-t", "va:0"], "the ring", |o| {
        o.contains("rung")
    });
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!((bells(&s), rang(&t)), before, "bell-action other acted");
    assert_eq!(
        t.run(&["show-options", "-gv", "bell-action"]).trim(),
        "other"
    );
    let bad = t.output(&["set-option", "-g", "visual-bell", "loud"]);
    assert!(!bad.status.success(), "{bad:?}");
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
fn clock_mode_style_twelve_shows_am_or_pm() {
    let t = Tmxr::new("clock12");
    let s = t.attach(&["new", "-s", "c12"]);
    s.wait_for("status line", |text| text.contains("c12"));
    // Panes too narrow for big digits show the time as text.
    t.run(&["split-window", "-h", "-t", "c12"]);
    t.run(&["split-window", "-h", "-t", "c12"]);
    // `H:MM AM` or `HH:MM PM`: a digit, then `:`, two digits, a blank, `AM`
    // or `PM`.
    let twelve_hour = |text: &str| {
        let chars: Vec<char> = text.chars().collect();
        chars.windows(7).any(|w| {
            w[0].is_ascii_digit()
                && w[1] == ':'
                && w[2].is_ascii_digit()
                && w[3].is_ascii_digit()
                && w[4] == ' '
                && (w[5] == 'A' || w[5] == 'P')
                && w[6] == 'M'
        })
    };
    t.run(&["clock-mode", "-t", "c12"]);
    s.wait_for("the 24-hour clock", |text| {
        text.chars().filter(|c| *c == ':').count() >= 1 && !twelve_hour(text)
    });
    t.run(&["set", "-wg", "clock-mode-style", "12"]);
    s.wait_for("the 12-hour clock", twelve_hour);
}

#[test]
fn source_file_q_and_a_missing_file_leave_the_config_alone() {
    let t = Tmxr::new("sourcefile");
    t.run(&["new-session", "-d", "-s", "sf"]);
    let missing = t.dir.path().join("no-such.toml");
    let missing = missing.to_str().unwrap();
    // -q: a missing file is no error.
    assert!(t.output(&["source-file", "-q", missing]).status.success());
    // Without -q it is, and the config to reload stays the old one.
    assert!(!t.output(&["source-file", missing]).status.success());
    let reload = t.output(&["source-file"]);
    assert!(
        reload.status.success(),
        "{}",
        String::from_utf8_lossy(&reload.stderr)
    );
    let config_files = || t.run(&["display-message", "-p", "#{config_files}"]);
    assert_eq!(config_files().trim(), t.config.display().to_string());
    // A file that does load becomes the config's file.
    let other = t.dir.path().join("other.toml");
    std::fs::write(&other, "").unwrap();
    t.run(&["source-file", other.to_str().unwrap()]);
    assert_eq!(config_files().trim(), other.display().to_string());
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

/// Unix only: it needs a shell that can kill itself with a signal.
#[cfg(unix)]
#[test]
fn run_shell_reports_how_a_command_failed_and_exits_with_its_code() {
    let t = Tmxr::new("runshell");
    t.run(&["new-session", "-d", "-s", "rs"]);
    // As tmux 3.6 printed them.
    let failed = t.output(&["run-shell", "echo out; exit 3"]);
    assert_eq!(failed.status.code(), Some(3));
    assert_eq!(
        String::from_utf8_lossy(&failed.stdout),
        "out\n'echo out; exit 3' returned 3\n"
    );
    let killed = t.output(&["run-shell", "kill -9 $$"]);
    assert_eq!(killed.status.code(), Some(128 + 9));
    assert_eq!(
        String::from_utf8_lossy(&killed.stdout),
        "'kill -9 $$' terminated by signal 9\n"
    );
    let quiet = t.output(&["run-shell", "echo err >&2; exit 3"]);
    assert_eq!(
        String::from_utf8_lossy(&quiet.stdout),
        "'echo err >&2; exit 3' returned 3\n"
    );
}

#[test]
fn command_error_runs_once_for_each_command_that_fails() {
    let t = Tmxr::new("cmderror");
    t.run(&["new-session", "-d", "-s", "ce"]);
    t.run(&["set", "-g", "@n", "0"]);
    t.run(&[
        "set-hook",
        "-g",
        "command-error",
        "set -gF @n '#{e|+|:#{@n},1}'",
    ]);
    let n = || t.run(&["show", "-gv", "@n"]).trim().to_owned();
    // A command that fails as it runs, each time; one that does not parse
    // does not run at all, as in tmux.
    assert!(!t.output(&["select-window", "-t", "ce:9"]).status.success());
    assert_eq!(n(), "1");
    assert!(!t.output(&["select-window", "-t", "ce:9"]).status.success());
    assert_eq!(n(), "2");
    assert!(!t.output(&["no-such-command"]).status.success());
    assert_eq!(n(), "2");
    // A hook that fails itself does not fire it again.
    t.run(&["set-hook", "-g", "command-error", "select-window -t ce:9"]);
    assert!(!t.output(&["select-window", "-t", "ce:9"]).status.success());
    assert_eq!(t.run(&["display-message", "-p", "alive"]).trim(), "alive");
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
    // show-options -H adds the hooks to a list, -A the inherited ones marked
    // `*`; the global hooks split as tmux keeps them, the pane hooks with the
    // window options.
    t.run(&["set-hook", "-g", "pane-exited", "display-message gone"]);
    let pane = t.run(&["show-options", "-pAH", "-t", "hk:7.0"]);
    assert!(
        pane.contains("after-select-pane[0] set-option -g @who pane\n")
            && pane.contains("pane-exited[0]* display-message gone\n")
            && !pane.contains("@who window"),
        "{pane}"
    );
    assert!(
        !t.run(&["show-options", "-p", "-t", "hk:7.0"])
            .contains("after-")
    );
    let global = t.run(&["show-options", "-gH"]);
    assert!(
        global.contains("after-select-pane[0] set-option -g @who global\n")
            && !global.contains("pane-exited"),
        "{global}"
    );
    let global_window = t.run(&["show-options", "-wgH"]);
    assert!(
        global_window.contains("pane-exited[0] display-message gone\n")
            && !global_window.contains("after-select-pane"),
        "{global_window}"
    );
    t.run(&["set-hook", "-gu", "pane-exited"]);
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
fn window_and_pane_options_override_the_global_ones() {
    let t = Tmxr::new("scopes");
    t.run(&["new-session", "-d", "-s", "sc"]);
    t.run(&["new-window", "-d", "-t", "sc:1"]);
    t.run(&["split-window", "-d", "-t", "sc:1"]);
    t.run(&["split-window", "-d", "-t", "sc:1"]);
    let show = |args: &[&str]| t.run(&[&["show-options"], args].concat()).trim().to_owned();
    let err = |args: &[&str]| {
        let out = t.output(args);
        assert!(!out.status.success(), "{args:?} succeeded");
        String::from_utf8_lossy(&out.stderr).trim().to_owned()
    };

    // A window option without -g is the target window's own, as tmux's.
    t.run(&["set-option", "-t", "sc:1", "mode-keys", "emacs"]);
    assert_eq!(show(&["-wv", "-t", "sc:1", "mode-keys"]), "emacs");
    assert_eq!(show(&["-gv", "mode-keys"]), "vi");
    // Another window has none of its own: -A shows what it inherits.
    assert_eq!(show(&["-w", "-t", "sc:0", "mode-keys"]), "");
    assert_eq!(show(&["-wA", "-t", "sc:0", "mode-keys"]), "mode-keys* vi");
    assert_eq!(show(&["-w", "-t", "sc:1"]), "mode-keys emacs");
    // And it takes effect there only.
    t.run(&["set-window-option", "-t", "sc:1", "pane-base-index", "1"]);
    let index = |target: &str| {
        t.run(&["display-message", "-p", "-t", target, "#{pane_index}"])
            .trim()
            .to_owned()
    };
    assert_eq!(index("sc:1.1"), "1");
    assert_eq!(index("sc:0.0"), "0");

    // A pane's own, for the options a pane may hold.
    t.run(&["set-option", "-p", "-t", "sc:1.1", "remain-on-exit", "on"]);
    assert_eq!(show(&["-pv", "-t", "sc:1.1", "remain-on-exit"]), "on");
    assert_eq!(show(&["-pv", "-t", "sc:1.2", "remain-on-exit"]), "");
    assert_eq!(show(&["-p", "-t", "sc:1.1"]), "remain-on-exit on");
    // -U takes the window's panes' own values too.
    t.run(&["set-option", "-wU", "-t", "sc:1", "remain-on-exit"]);
    assert_eq!(show(&["-pv", "-t", "sc:1.1", "remain-on-exit"]), "");
    // -u gives a window's option back to the global one, and a global one
    // goes back to its default.
    t.run(&["set-option", "-wu", "-t", "sc:1", "mode-keys"]);
    assert_eq!(show(&["-wv", "-t", "sc:1", "mode-keys"]), "");
    t.run(&["set-option", "-g", "main-pane-width", "10"]);
    t.run(&["set-option", "-gu", "main-pane-width"]);
    assert_eq!(show(&["-gv", "main-pane-width"]), "80");

    // User options: the pane's, else its window's, else the global one.
    t.run(&["set-option", "@x", "global"]);
    t.run(&["set-option", "-w", "-t", "sc:1", "@x", "window"]);
    t.run(&["set-option", "-p", "-t", "sc:1.2", "@x", "pane"]);
    let x = |target: &str| {
        t.run(&["display-message", "-p", "-t", target, "#{@x}"])
            .trim()
            .to_owned()
    };
    assert_eq!(x("sc:1.1"), "window");
    assert_eq!(x("sc:1.2"), "pane");
    assert_eq!(x("sc:0.0"), "global");
    // Shown quoted as tmux's, to be read back.
    t.run(&["set-option", "-w", "-t", "sc:0", "@q", "a b"]);
    assert_eq!(show(&["-w", "-t", "sc:0", "@q"]), r#"@q "a b""#);
    assert_eq!(
        err(&["show-options", "-w", "-t", "sc:0", "@x"]),
        "invalid option: @x"
    );
    assert_eq!(
        err(&["set-option", "-o", "-w", "-t", "sc:1", "@x", "again"]),
        "already set: @x"
    );
    assert_eq!(err(&["show-options", "nosuch"]), "invalid option: nosuch");

    // synchronize-panes: what is typed in a synchronized pane goes to its
    // siblings that synchronize too.
    for p in ["sc:1.1", "sc:1.2", "sc:1.3"] {
        t.wait_prompt(p);
    }
    t.run(&["set-option", "-t", "sc:1", "synchronize-panes", "on"]);
    t.run(&[
        "set-option",
        "-p",
        "-t",
        "sc:1.3",
        "synchronize-panes",
        "off",
    ]);
    t.run(&["send-keys", "-t", "sc:1.1", "echo sync-mark", "Enter"]);
    t.wait_run(&["capture-pane", "-p", "-t", "sc:1.2"], "the copy", |o| {
        o.lines().filter(|l| l.contains("sync-mark")).count() == 2
    });
    std::thread::sleep(Duration::from_millis(500));
    let left_out = t.run(&["capture-pane", "-p", "-t", "sc:1.3"]);
    assert!(!left_out.contains("sync-"), "{left_out}");

    // A sibling in copy mode, or hidden by a zoom, gets none of it either,
    // as tmux's.
    let marks = |pane: &str, mark: &str| {
        t.run(&["capture-pane", "-p", "-t", pane])
            .lines()
            .filter(|l| l.contains(mark))
            .count()
    };
    t.run(&["copy-mode", "-t", "sc:1.2"]);
    t.run(&["send-keys", "-t", "sc:1.1", "echo mode-mark", "Enter"]);
    t.wait_run(&["capture-pane", "-p", "-t", "sc:1.1"], "the typing", |o| {
        o.lines().filter(|l| l.contains("mode-mark")).count() == 2
    });
    t.run(&["send-keys", "-t", "sc:1.2", "-X", "cancel"]);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(marks("sc:1.2", "mode-mark"), 0, "copy mode got the input");
    t.run(&["clock-mode", "-t", "sc:1.2"]);
    t.run(&["send-keys", "-t", "sc:1.1", "echo clock-mark", "Enter"]);
    t.wait_run(&["capture-pane", "-p", "-t", "sc:1.1"], "the typing", |o| {
        o.lines().filter(|l| l.contains("clock-mark")).count() == 2
    });
    t.run(&["copy-mode", "-q", "-t", "sc:1.2"]);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(marks("sc:1.2", "clock-mark"), 0, "clock mode got the input");
    t.run(&["resize-pane", "-Z", "-t", "sc:1.1"]);
    t.run(&["send-keys", "-t", "sc:1.1", "echo zoom-mark", "Enter"]);
    t.wait_run(&["capture-pane", "-p", "-t", "sc:1.1"], "the typing", |o| {
        o.lines().filter(|l| l.contains("zoom-mark")).count() == 2
    });
    t.run(&["resize-pane", "-Z", "-t", "sc:1.1"]);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        marks("sc:1.2", "zoom-mark"),
        0,
        "a hidden pane got the input"
    );
}

#[test]
fn session_options_are_each_sessions_own() {
    let t = Tmxr::new("sessopts");
    t.run(&["new-session", "-d", "-s", "one"]);
    let show = |args: &[&str]| t.run(&[&["show-options"], args].concat()).trim().to_owned();
    let err = |args: &[&str]| {
        let out = t.output(args);
        assert!(!out.status.success(), "{args:?} succeeded");
        String::from_utf8_lossy(&out.stderr).trim().to_owned()
    };

    // Without -g a session option is the target session's own, as tmux's.
    t.run(&["set-option", "-t", "one", "status-left", "ONE-LEFT"]);
    t.run(&["set-option", "-t", "one", "prefix", "C-a"]);
    t.run(&["set-option", "-t", "one", "@who", "first"]);
    let a = t.attach(&["new", "-s", "two"]);
    a.wait_for("status line", |text| text.contains("two"));
    assert_eq!(show(&["-v", "-t", "one", "status-left"]), "ONE-LEFT");
    assert_eq!(show(&["-v", "-t", "two", "status-left"]), "");
    assert_eq!(
        show(&["-t", "one"]),
        "@who first
prefix C-a
status-left ONE-LEFT"
    );
    assert_eq!(show(&["-A", "-t", "two", "prefix"]), "prefix* C-b");
    assert_eq!(show(&["-gv", "prefix"]), "C-b");
    let who = |target: &str| {
        t.run(&["display-message", "-p", "-t", target, "#{@who}"])
            .trim()
            .to_owned()
    };
    assert_eq!(who("one"), "first");
    assert_eq!(who("two"), "");

    // Each session's status line, prefix and window numbering are its own.
    let b = t.attach(&["attach", "-t", "one"]);
    b.wait_for("one's status line", |text| text.contains("ONE-LEFT"));
    assert!(!a.text().contains("ONE-LEFT"), "{}", a.text());
    b.send(b"c");
    t.wait_run(&["list-windows", "-t", "one", "-F", "#I"], "C-a c", |o| {
        o.lines().count() == 2
    });
    a.send(b"c");
    a.send(PREFIX);
    a.send(b"c");
    t.wait_run(&["list-windows", "-t", "two", "-F", "#I"], "C-b c", |o| {
        o.lines().count() == 2
    });
    t.run(&["set-option", "-t", "two", "base-index", "5"]);
    t.run(&["new-window", "-d", "-t", "two"]);
    assert_eq!(
        t.run(&["list-windows", "-t", "two", "-F", "#I"]).trim(),
        "0
1
5"
    );
    assert_eq!(
        t.run(&["list-windows", "-t", "one", "-F", "#I"]).trim(),
        "0
1"
    );

    // -u gives the session the global value back; -o refuses one it sets.
    assert_eq!(
        err(&["set-option", "-o", "-t", "one", "prefix", "C-x"]),
        "already set: prefix"
    );
    t.run(&["set-option", "-o", "-t", "two", "prefix", "C-x"]);
    t.run(&["set-option", "-u", "-t", "one", "status-left"]);
    b.wait_for("one's own status-left gone", |text| {
        !text.contains("ONE-LEFT")
    });
    assert_eq!(show(&["-t", "one", "status-left"]), "");
    // A prefix must be a key; server options are listed with -s.
    assert!(err(&["set-option", "-g", "prefix", "nokey"]).contains("nokey"));
    assert!(show(&["-s"]).lines().any(|l| l == "escape-time 0"));
    assert!(!show(&["-s"]).contains("status-left"));
    // -q: an unknown option, or an unset user option, is no error.
    for quiet in [
        &["set-option", "-q", "no-such-option", "x"][..],
        &["show-options", "-q", "no-such-option"],
        &["show-options", "-q", "@unset"],
    ] {
        let out = t.output(quiet);
        assert!(
            out.status.success() && out.stdout.is_empty(),
            "{quiet:?}: {out:?}"
        );
    }
    assert_eq!(
        err(&["set-option", "no-such-option", "x"]),
        "invalid option: no-such-option"
    );
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
    // 16: the ANSI colours only.
    t.run(&["set-option", "-g", "rgb-colour", "16"]);
    s.wait_for_cells("the 16 colours", status_in_16_colours);
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
    drop(s);
    // A TERM without 256 colours gets the 16, as tmux reads one.
    let sixteen = [
        ("COLORTERM", None),
        ("WT_SESSION", None),
        ("TERM", Some("xterm")),
    ];
    let s = t.attach_env(&["attach", "-t", "ra"], &sixteen);
    s.wait_for("status line", |text| text.contains("ra"));
    s.wait_for_cells("the 16 colours", status_in_16_colours);
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
fn set_option_f_expands_the_value_for_the_current_pane() {
    let t = Tmxr::new("setf");
    t.run(&["new-session", "-d", "-s", "sf"]);
    t.run(&[
        "set-option",
        "-gF",
        "@expanded",
        "#{session_name}-#{window_index}",
    ]);
    assert_eq!(t.run(&["show-options", "-gv", "@expanded"]).trim(), "sf-0");
    // Without -F the value is kept as written.
    t.run(&["set-option", "-g", "@kept", "plain"]);
    assert_eq!(t.run(&["show-options", "-gv", "@kept"]).trim(), "plain");
}

/// Each client's status line redraws on its own session's `status-interval`,
/// none at 0, as tmux's: a change only the timer shows (the pane's command,
/// with no output) reaches the client whose session ticks, not the other.
#[test]
fn each_session_redraws_its_status_line_at_its_own_interval() {
    #[cfg(unix)]
    let (quiet, program) = ("sleep 30", "sleep");
    #[cfg(windows)]
    let (quiet, program) = ("ping -n 30 127.0.0.1 >NUL", "ping");
    let shows = |text: &str| {
        text.lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .is_some_and(|l| l.to_ascii_lowercase().contains(&format!("cmd={program}")))
    };
    let t = Tmxr::new("statustimer");
    let mut screens = Vec::new();
    for (name, interval) in [("ticks", "1"), ("still", "0")] {
        t.run(&["new-session", "-d", "-s", name]);
        t.run(&["set-option", "-t", name, "status-interval", interval]);
        t.run(&[
            "set-option",
            "-t",
            name,
            "status-right",
            "cmd=#{pane_current_command}",
        ]);
        t.run(&["set-option", "-w", "-t", name, "automatic-rename", "off"]);
        t.wait_settled(name);
        let s = t.attach(&["attach", "-t", name]);
        s.wait_for("the status line", |text| text.contains("cmd="));
        screens.push(s);
    }
    // The command cache refreshes each second: let it hold the shells, so
    // the echo of the typing below redraws with the shell's name and only a
    // later redraw can show the program's.
    std::thread::sleep(Duration::from_millis(1500));
    for name in ["ticks", "still"] {
        t.run(&["send-keys", "-t", name, quiet, "Enter"]);
    }
    screens[0].wait_for("the command in the ticking status line", shows);
    t.wait_run(
        &[
            "display-message",
            "-p",
            "-t",
            "still",
            "#{pane_current_command}",
        ],
        "the command seen in the other session",
        |o| o.trim().eq_ignore_ascii_case(program),
    );
    std::thread::sleep(Duration::from_secs(3));
    let text = screens[1].text();
    assert!(
        !shows(&text),
        "a status-interval 0 status line redrew:\n{text}"
    );
}

/// `attach -f ignore-size`: the client's size sizes no window while a client
/// without the flag is attached, as tmux's; alone, it does.
#[test]
fn cursor_style_and_programs_shape_the_clients_cursor() {
    let t = Tmxr::new("cursorstyle");
    let s = t.attach(&["new", "-s", "cs"]);
    s.wait_for("status line", |text| text.contains("cs"));
    // No program has asked: the option's shape (underline, 4).
    t.run(&["set", "-g", "cursor-style", "underline"]);
    s.wait_for("the option's shape", |_| s.raw_text().contains("\x1b[4 q"));
    // A program's own shape (bar, 6) wins over the option.
    #[cfg(unix)]
    let program = ["/bin/sh", "-c", r"printf '\033[6 qbar-asked'; sleep 30"];
    #[cfg(windows)]
    let program = [
        "powershell.exe",
        "-NoProfile",
        "-Command",
        "[Console]::Out.Write([string][char]27 + '[6 qbar-asked'); Start-Sleep 30",
    ];
    let mut args = vec!["new-window", "-t", "cs"];
    args.extend(program);
    t.run(&args);
    s.wait_for("the program's shape", |_| s.raw_text().contains("\x1b[6 q"));
    // The shape can reach the terminal a frame before the program's text.
    s.wait_for("the program's text", |text| text.contains("bar-asked"));
    // Back on the first window, the option's shape again.
    let before = s.raw_text().matches("\x1b[4 q").count();
    t.run(&["select-window", "-t", "cs:0"]);
    s.wait_for("the option's shape again", |_| {
        s.raw_text().matches("\x1b[4 q").count() > before
    });
}

#[test]
fn the_terminal_title_is_the_programs_else_session_and_window() {
    let t = Tmxr::new("title");
    let s = t.attach(&["new", "-s", "tt", "-n", "main"]);
    s.wait_for("the status line", |text| text.contains("tt"));
    let wait_title = |want: &str| {
        let deadline = Instant::now() + TIMEOUT;
        while s.title() != want {
            assert!(
                Instant::now() < deadline,
                "title {:?}, not {want:?}",
                s.title()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    // A pane whose program set no title has the host name for its title,
    // so the session and window names show.
    let host = t.run(&["display-message", "-p", "#{host}"]);
    t.run(&["select-pane", "-t", "tt", "-T", host.trim()]);
    wait_title("tt:main");
    // A program's title wins; control characters never reach the terminal.
    t.run(&["select-pane", "-t", "tt", "-T", "my [31mprog!"]);
    wait_title("my [31mprog!");
    // set-titles off leaves the terminal's title alone.
    t.run(&["set", "-g", "set-titles", "off"]);
    t.run(&["select-pane", "-t", "tt", "-T", "other"]);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(s.title(), "my [31mprog!");
    t.run(&["set", "-g", "set-titles", "on"]);
    wait_title("other");
}
