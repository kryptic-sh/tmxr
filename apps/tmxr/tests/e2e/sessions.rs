use super::*;

#[test]
fn attach_type_split_navigate_detach_reattach() {
    let t = Tmxr::new("flow");
    let s = t.attach(&["new", "-s", "e2e"]);

    // The status line names the session.
    s.wait_for("status line", |text| text.contains("e2e"));

    // Typing reaches the shell in the pane.
    s.send(b"echo tmxr-e2e-marker\r");
    s.wait_for("echoed marker", |text| {
        text.matches("tmxr-e2e-marker").count() >= 2
    });

    // prefix % splits left/right: two panes, the new one active.
    s.send(PREFIX);
    s.send(b"%");
    t.wait_run(&["list-panes", "-t", "e2e"], "two panes", |o| {
        o.lines().count() == 2
    });
    s.wait_for("vertical border", |text| text.contains('│'));
    let active = |t: &Tmxr| t.run(&["display-message", "-p", "-t", "e2e", "#{pane_index}"]);
    assert_eq!(active(&t).trim(), "1");

    // prefix h focuses the left pane (vim-style selection from the config).
    s.send(PREFIX);
    s.send(b"h");
    t.wait_run(
        &["display-message", "-p", "-t", "e2e", "#{pane_index}"],
        "left pane",
        |o| o.trim() == "0",
    );

    // C-l in a plain shell moves right (navigator: no editor in front).
    s.send(b"\x0c");
    t.wait_run(
        &["display-message", "-p", "-t", "e2e", "#{pane_index}"],
        "right pane",
        |o| o.trim() == "1",
    );

    // prefix d detaches; the session keeps running.
    let mut s = s;
    s.send(PREFIX);
    s.send(b"d");
    s.wait_exit();
    assert!(
        s.raw_text().contains("detached (from session e2e)"),
        "{}",
        s.raw_text()
    );
    let ls = t.run(&["ls"]);
    assert!(ls.contains("e2e: 1 windows"), "{ls}");

    // Reattach: the pane still shows what was typed before detaching.
    let s = t.attach(&["attach", "-t", "e2e"]);
    s.wait_for("preserved output", |text| text.contains("tmxr-e2e-marker"));
}

#[test]
fn command_clients_start_and_stop_a_server() {
    let t = Tmxr::new("cmd");
    assert!(t.run(&["ls"]).is_empty(), "no server yet");
    t.run(&["new-session", "-d", "-s", "bg"]);
    t.run(&["new-window", "-d", "-t", "bg", "-n", "second"]);
    let windows = t.wait_run(&["list-windows", "-t", "bg"], "two windows", |o| {
        o.lines().count() == 2
    });
    assert!(windows.contains("second"), "{windows}");
    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);
}

#[test]
fn session_names_may_hold_dots() {
    let t = Tmxr::new("dots");
    // Pane %0 is elsewhere, so a target that went wrong cannot pass for it.
    t.run(&["new-session", "-d", "-s", "first", "-n", "elsewhere"]);
    t.run(&["new-session", "-d", "-s", "my.app", "-n", "zero"]);
    t.run(&["new-window", "-d", "-t", "my.app", "-n", "one"]);
    t.run(&["split-window", "-d", "-t", "my.app:1"]);
    let name = |target: &str| {
        let out = t.output(&["display", "-p", "-t", target, "#S #W #P"]);
        assert!(out.status.success(), "{target}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    // Exactly a session's name, without ':', is that session; after the
    // ':', the window and pane parts are as ever.
    assert_eq!(name("my.app"), "my.app zero 0");
    assert_eq!(name("=my.app"), "my.app zero 0");
    assert_eq!(name("my.app:"), "my.app zero 0");
    assert_eq!(name("my.app:1"), "my.app one 0");
    assert_eq!(name("my.app:1.1"), "my.app one 1");
    assert_eq!(name("my.app:one.1"), "my.app one 1");
    assert!(t.output(&["has-session", "-t", "my.app"]).status.success());
    // A dotted word naming no session is still window.pane.
    // (my.app is the current session, as the newest.)
    assert_eq!(name("one.1"), "my.app one 1");
    // A target naming nothing is an error, not some other pane.
    let none = t.output(&["display", "-p", "-t", "no.such", "#S"]);
    assert!(!none.status.success(), "{none:?}");
    assert!(
        String::from_utf8_lossy(&none.stderr).contains("can't find"),
        "{none:?}"
    );

    t.run(&["rename-session", "-t", "my.app", "v1.2.3"]);
    assert!(t.run(&["ls", "-F", "#S"]).lines().any(|l| l == "v1.2.3"));
    let bad = t.output(&["rename-session", "-t", "v1.2.3", "a:b"]);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("bad session name: a:b"));
}

#[test]
fn a_bare_tmxr_opens_its_directorys_session() {
    let t = Tmxr::new("bare");
    let proj = t.dir.path().join("my.proj");
    std::fs::create_dir(&proj).unwrap();
    let mut s = t.attach_in(&proj, &[]);
    s.wait_for("status line", |text| text.contains("my.proj"));
    t.wait_prompt("my.proj");
    s.send(b"echo bare-marker\r");
    s.wait_for("echoed marker", |text| {
        text.matches("bare-marker").count() >= 2
    });
    let path = t.run(&["display", "-p", "-t", "my.proj", "#{pane_current_path}"]);
    assert!(path.trim().ends_with("my.proj"), "{path}");
    s.send(PREFIX);
    s.send(b"d");
    s.wait_exit();

    // Again from there: the same session, attached, not a second one.
    let s = t.attach_in(&proj, &[]);
    s.wait_for("the session's output", |text| text.contains("bare-marker"));
    assert_eq!(t.run(&["ls", "-F", "#S"]).trim(), "my.proj");
    // `new-session -A` attaches to it too; without -A the name is taken.
    let s_a = t.attach(&["new-session", "-A", "-s", "my.proj"]);
    s_a.wait_for("the session's output", |text| text.contains("bare-marker"));
    assert_eq!(t.run(&["ls", "-F", "#S"]).trim(), "my.proj");
    let again = t.output(&["new-session", "-d", "-s", "my.proj"]);
    assert!(
        String::from_utf8_lossy(&again.stderr).contains("duplicate session: my.proj"),
        "{again:?}"
    );

    // Another directory gets its own session.
    let other = t.dir.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let s2 = t.attach_in(&other, &[]);
    s2.wait_for("status line", |text| text.contains("other"));
    t.wait_run(&["ls", "-F", "#S"], "two sessions", |o| {
        o.lines().collect::<Vec<_>>() == ["my.proj", "other"]
    });
    drop((s, s_a));
}

/// The owner's `t` script: a session per directory with three windows,
/// window 0 in front, made by a bare `tmxr` and a `[hooks]` config entry.
#[test]
fn attaching_takes_tmuxs_c_d_e_and_f() {
    let t = Tmxr::new("attachflags");
    let work = t.dir.path().join("work.dir");
    std::fs::create_dir(&work).unwrap();
    t.run(&["new-session", "-d", "-s", "at"]);
    let sock = || {
        t.run(&["show-environment", "-t", "at", "SSH_AUTH_SOCK"])
            .trim()
            .to_owned()
    };

    // Attaching takes the client's update-environment variables, as tmux's.
    let mut a = t.attach_env(
        &["attach", "-t", "at"],
        &[("SSH_AUTH_SOCK", Some("first-sock"))],
    );
    a.wait_for("status line", |text| text.contains("at"));
    assert_eq!(sock(), "SSH_AUTH_SOCK=first-sock");
    // -E leaves them; -d detaches the session's other clients; -c moves
    // the session's directory, where its new windows start.
    let work_arg = work.display().to_string();
    let mut b = t.attach_env(
        &["attach", "-dE", "-c", &work_arg, "-t", "at"],
        &[("SSH_AUTH_SOCK", Some("second-sock"))],
    );
    a.wait_exit();
    b.wait_for("status line", |text| text.contains("at"));
    assert_eq!(sock(), "SSH_AUTH_SOCK=first-sock");
    t.run(&["new-window", "-d", "-t", "at:1"]);
    t.wait_run(
        &[
            "display-message",
            "-p",
            "-t",
            "at:1",
            "#{pane_current_path}",
        ],
        "the session's new directory",
        |o| o.trim().ends_with("work.dir"),
    );

    // new -A -D as attach -d, and -f read-only; a client without a
    // variable marks it removed.
    let c = t.attach_env(
        &["new", "-A", "-D", "-s", "at", "-f", "read-only"],
        &[("SSH_AUTH_SOCK", None)],
    );
    b.wait_exit();
    c.wait_for("status line", |text| text.contains("at"));
    assert_eq!(sock(), "-SSH_AUTH_SOCK");
    t.wait_prompt("at:0");
    c.send(b"echo typed-read-only\r");
    std::thread::sleep(Duration::from_secs(1));
    let screen = t.run(&["capture-pane", "-p", "-t", "at:0"]);
    assert!(!screen.contains("typed-read-only"), "{screen}");
    // Client flags tmxr does not have are refused by name.
    let bad = t.output(&["attach", "-f", "no-output", "-t", "at"]);
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("unsupported client flag: no-output"),
        "{bad:?}"
    );

    // switch-client gives the session it goes to the switching client's
    // variables too, unless -E.
    t.run(&["new-session", "-d", "-s", "sw"]);
    t.run(&["set-environment", "-t", "sw", "SSH_AUTH_SOCK", "old-sock"]);
    let sw_sock = || {
        t.run(&["show-environment", "-t", "sw", "SSH_AUTH_SOCK"])
            .trim()
            .to_owned()
    };
    let d = t.attach_env(
        &["attach", "-t", "at"],
        &[("SSH_AUTH_SOCK", Some("third-sock"))],
    );
    d.wait_for("status line", |text| text.contains("at"));
    d.send(PREFIX);
    d.send(b":");
    d.wait_for("the prompt", |text| {
        text.lines().any(|l| l.starts_with(':'))
    });
    d.send(b"switch-client -E -t sw\r");
    t.wait_run(
        &["list-clients", "-F", "#{client_session}"],
        "the switch",
        |o| o.lines().any(|l| l == "sw"),
    );
    assert_eq!(sw_sock(), "SSH_AUTH_SOCK=old-sock");
    d.send(PREFIX);
    d.send(b":");
    d.wait_for("the prompt", |text| {
        text.lines().any(|l| l.starts_with(':'))
    });
    d.send(b"switch-client -t at\r");
    t.wait_run(&["list-clients", "-F", "#{client_session}"], "back", |o| {
        !o.lines().any(|l| l == "sw")
    });
    d.send(PREFIX);
    d.send(b":");
    d.wait_for("the prompt", |text| {
        text.lines().any(|l| l.starts_with(':'))
    });
    d.send(b"switch-client -t sw\r");
    t.wait_run(
        &["show-environment", "-t", "sw", "SSH_AUTH_SOCK"],
        "sw takes the client's variable",
        |o| o.trim() == "SSH_AUTH_SOCK=third-sock",
    );
}

/// `detach-client -P` hangs up the client's parent: here a shell that runs
/// the client and then would print a word. A plain detach lets it.
#[cfg(unix)]
#[test]
fn detach_client_p_hangs_up_the_clients_parent() {
    let t = Tmxr::new("hangup");
    t.run(&["new-session", "-d", "-s", "hu"]);
    let words = t.args(&["attach", "-t", "hu"]);
    let line = format!(
        "{} {}; echo survived; sleep 5",
        env!("CARGO_BIN_EXE_tmxr"),
        words.join(" ")
    );
    let shell = |t: &Tmxr| t.spawn_at(t.dir.path(), "/bin/sh", &["-c".into(), line.clone()], &[]);

    let s = shell(&t);
    s.wait_for("status line", |text| text.contains("hu"));
    t.run(&["detach-client", "-s", "hu"]);
    s.wait_for("the shell after the client", |text| {
        text.contains("survived")
    });
    drop(s);

    let mut s = shell(&t);
    s.wait_for("status line", |text| text.contains("hu"));
    t.run(&["detach-client", "-P", "-s", "hu"]);
    s.wait_exit();
    assert!(!s.text().contains("survived"), "{}", s.text());
}

/// A Windows client has no parent to send SIGHUP: `-P` is refused and the
/// client stays.
#[cfg(windows)]
#[test]
fn detach_client_p_is_refused_on_windows() {
    let t = Tmxr::new("hangup");
    let s = t.attach(&["new", "-s", "hu"]);
    s.wait_for("status line", |text| text.contains("hu"));
    let out = t.output(&["detach-client", "-P", "-s", "hu"]);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("SIGHUP"),
        "{out:?}"
    );
    assert_eq!(
        t.run(&["list-clients", "-F", "#{client_session}"]).trim(),
        "hu"
    );
    let out = t.output(&["attach-session", "-x", "-t", "hu"]);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("SIGHUP"),
        "{out:?}"
    );
}

#[test]
fn a_config_that_cannot_be_used_is_shown_to_the_first_client() {
    let t = Tmxr::new("badcfg");
    std::fs::write(
        &t.config,
        "mouse = \"maybe\"
",
    )
    .unwrap();
    let s = t.attach(&["new", "-s", "bc"]);
    s.wait_for("the error", |text| {
        text.contains("config error, using defaults")
    });
    assert!(t.run(&["show-messages"]).contains("config error"));
    // Once: a second client is not told again.
    let second = t.attach(&["attach", "-t", "bc"]);
    second.wait_for("the status line", |text| text.contains("bc"));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!second.text().contains("config error"), "{}", second.text());
}

#[test]
fn a_sessions_own_update_environment_decides_what_attaching_takes() {
    let t = Tmxr::new("envown");
    t.run(&["new-session", "-d", "-E", "-s", "own"]);
    t.run(&["set", "-g", "update-environment", "E2E_GLOBAL"]);
    // The session's own array starts empty, as tmux's, so item 1 is all
    // it has.
    t.run(&["set", "-t", "own", "update-environment[1]", "E2E_MINE"]);
    assert_eq!(
        t.run(&["show", "-t", "own", "update-environment"]).trim(),
        "update-environment[1] E2E_MINE"
    );
    let s = t.attach_env(
        &["attach", "-t", "own"],
        &[("E2E_MINE", Some("m")), ("E2E_GLOBAL", Some("g"))],
    );
    s.wait_for("status line", |text| text.contains("own"));
    let env = t.run(&["show-environment", "-t", "own"]);
    let lines: Vec<&str> = env.lines().map(str::trim).collect();
    assert!(lines.contains(&"E2E_MINE=m"), "{env}");
    assert!(!env.contains("E2E_GLOBAL"), "{env}");

    // Errors, as tmux's.
    let err = |args: &[&str]| {
        let out = t.output(args);
        assert!(!out.status.success(), "{args:?}");
        String::from_utf8_lossy(&out.stderr).trim().to_owned()
    };
    assert!(err(&["set", "-g", "status-left[1]", "x"]).contains("not an array: status-left[1]"));
    assert!(err(&["set", "-g", "@u[1]", "x"]).contains("not an array: @u[1]"));
    assert!(
        err(&["set", "-o", "-t", "own", "update-environment[1]", "x"])
            .contains("already set: update-environment[1]")
    );
}

#[test]
fn destroy_unattached_and_the_exit_options_follow_tmux() {
    let t = Tmxr::new("lifecycle");
    t.run(&["new-session", "-d", "-s", "keep"]);
    t.run(&["new-session", "-d", "-s", "goes"]);
    t.run(&["set", "-t", "goes", "destroy-unattached", "on"]);
    // Made with -d, it stays until a client comes or goes, as tmux's.
    assert!(t.output(&["has-session", "-t", "goes"]).status.success());
    let mut s = t.attach(&["attach", "-t", "keep"]);
    s.wait_for("the status line", |text| text.contains("keep"));
    t.wait_run(&["list-sessions", "-F", "#S"], "goes gone", |o| {
        o.trim() == "keep"
    });

    // exit-empty off: the server outlives its last session.
    t.run(&["set", "-s", "exit-empty", "off"]);
    s.send(PREFIX);
    s.send(b"d");
    s.wait_exit();
    t.run(&["kill-session", "-t", "keep"]);
    std::thread::sleep(Duration::from_millis(300));
    let ls = t.output(&["list-sessions"]);
    assert!(ls.status.success(), "the server went: {ls:?}");

    // exit-unattached on: it goes once no client is attached, sessions or
    // not.
    // A session first: exit-empty on with none would end the server at once.
    t.run(&["new-session", "-d", "-s", "left"]);
    t.run(&["set", "-s", "exit-empty", "on"]);
    let mut s = t.attach(&["attach", "-t", "left"]);
    s.wait_for("the status line", |text| text.contains("left"));
    t.run(&["set", "-s", "exit-unattached", "on"]);
    s.send(PREFIX);
    s.send(b"d");
    s.wait_exit();
    let deadline = Instant::now() + TIMEOUT;
    while t.output(&["has-session", "-t", "left"]).status.success() {
        assert!(Instant::now() < deadline, "the server stayed");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn input_and_replies_larger_than_a_frame_go_whole() {
    let t = Tmxr::new("bigreply");
    t.run(&["new-session", "-d", "-s", "br"]);
    // Larger than a frame may be (`MAX_FRAME`, 16 MiB), each way.
    let size = 17 << 20;
    let text = "x".repeat(size);
    let load = t.output_with_stdin(&["load-buffer", "-b", "big", "-"], text.as_bytes());
    assert!(load.status.success(), "{load:?}");
    let out = t.output(&["show-buffer", "-b", "big"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout.len(), size);
    assert!(out.stdout.iter().all(|&b| b == b'x'));
    assert!(t.output(&["has-session", "-t", "br"]).status.success());
}

#[test]
fn a_client_that_fell_behind_is_repainted_without_another_event() {
    let t = Tmxr::new("fellbehind");
    t.run(&["new-session", "-d", "-s", "fb"]);
    // No status-line timer to repaint it by chance.
    t.run(&["set", "-g", "status-interval", "0"]);
    let s = t.attach(&["attach", "-t", "fb"]);
    s.wait_for("status line", |text| text.contains("fb"));
    // A terminal that stops reading while a pane prints a lot: the
    // server drops frames for it once its queue is full. On Windows this
    // burst overfills the queue (without the repaint the test fails there);
    // on Linux it passes either way (2026-10-18, WSL), so the queue
    // evidently did not fill there.
    s.pause(true);
    #[cfg(unix)]
    let burst = [
        "/bin/sh",
        "-c",
        "i=0; while [ $i -lt 20000 ]; do echo \"line $i\"; i=$((i+1)); done; echo done-marker; sleep 100",
    ];
    #[cfg(windows)]
    let burst = [
        "powershell.exe",
        "-NoProfile",
        "-Command",
        "1..20000 | % { \"line $_\" }; 'done-marker'; Start-Sleep 100",
    ];
    let mut respawn = vec!["respawn-pane", "-k", "-t", "fb"];
    respawn.extend(burst);
    t.run(&respawn);
    t.wait_run(
        &["capture-pane", "-p", "-t", "fb"],
        "the burst's end",
        |o| o.contains("done-marker"),
    );
    std::thread::sleep(Duration::from_millis(500));
    // Reading again, it catches up to the end with nothing else happening.
    s.pause(false);
    s.wait_for("the end on screen", |text| text.contains("done-marker"));
}

#[test]
fn a_program_that_reads_nothing_does_not_stall_the_server() {
    let t = Tmxr::new("nostall");
    // A program that never reads its input, on a raw terminal: input
    // backs up instead of being echoed and dropped.
    #[cfg(unix)]
    let idle = ["/bin/sh", "-c", "stty raw -echo; sleep 100"];
    #[cfg(windows)]
    let idle = [
        "powershell.exe",
        "-NoProfile",
        "-Command",
        "Start-Sleep 100",
    ];
    let mut new = vec!["new-session", "-d", "-s", "ns"];
    new.extend(idle);
    t.run(&new);
    // Far more than a terminal's input buffer holds.
    let file = t.dir.path().join("big.txt");
    std::fs::write(&file, "x".repeat(2 * 1024 * 1024)).unwrap();
    t.run(&["load-buffer", "-b", "big", file.to_str().unwrap()]);
    // Each command from a thread, so a stalled server fails the test
    // rather than hanging it (it still hangs at the end, in kill-server).
    let in_thread = |rest: &[&str]| {
        let (tx, rx) = std::sync::mpsc::channel();
        let args = t.args(rest);
        let env = t.env();
        std::thread::spawn(move || {
            let out = tmxr_term::child::command(env!("CARGO_BIN_EXE_tmxr"))
                .args(args)
                .envs(env)
                .output();
            let _ = tx.send(out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()));
        });
        rx
    };
    let paste = in_thread(&["paste-buffer", "-b", "big", "-t", "ns"]);
    let answer = in_thread(&["display-message", "-p", "-t", "ns", "#S"])
        .recv_timeout(Duration::from_secs(10))
        .expect("the server answered no command after the paste")
        .unwrap();
    assert_eq!(answer.trim(), "ns");
    assert!(
        paste.recv_timeout(Duration::from_secs(10)).is_ok(),
        "paste-buffer never finished"
    );
}

#[test]
fn session_activity_moves_on_with_attaching_and_keys() {
    let t = Tmxr::new("sessactivity");
    t.run(&["new-session", "-d", "-s", "act"]);
    let time = |f: &str| -> u64 {
        t.run(&["display-message", "-p", "-t", "act", f])
            .trim()
            .parse()
            .unwrap()
    };
    let created = time("#{session_created}");
    assert_eq!(time("#{session_activity}"), created);
    let never = t.run(&[
        "display-message",
        "-p",
        "-t",
        "act",
        "[#{session_last_attached}]",
    ]);
    assert_eq!(never.trim(), "[]");
    // The times are whole seconds: wait past one before each step.
    std::thread::sleep(Duration::from_millis(1100));
    let s = t.attach(&["attach", "-t", "act"]);
    s.wait_for("attached", |text| text.contains("act"));
    let attached = time("#{session_activity}");
    assert!(attached > created, "{attached} after {created}");
    // Attaching also sets when it was last attached.
    assert_eq!(time("#{session_last_attached}"), attached);
    std::thread::sleep(Duration::from_millis(1100));
    s.send(b"x");
    t.wait_run(
        &["display-message", "-p", "-t", "act", "#{session_activity}"],
        "a key's activity",
        |o| o.trim().parse::<u64>().is_ok_and(|n| n > attached),
    );
}

#[test]
fn client_variables_tell_its_times_and_last_session() {
    let t = Tmxr::new("clientvars");
    t.run(&["new-session", "-d", "-s", "first"]);
    t.run(&["new-session", "-d", "-s", "second"]);
    let s = t.attach(&["attach", "-t", "first"]);
    s.wait_for("attached", |text| text.contains("first"));
    let client = |f: &str| t.run(&["list-clients", "-F", f]).trim().to_owned();
    assert_eq!(client("[#{client_last_session}]"), "[]");
    t.run(&["switch-client", "-t", "second"]);
    t.wait_run(
        &["list-clients", "-F", "#{client_last_session}"],
        "the last session",
        |o| o.trim() == "first",
    );
    // Seconds since the epoch, the activity no earlier than the connection.
    let times =
        client("#{e|>|:#{client_created},1700000000} #{e|>=:#{client_activity},#{client_created}}");
    assert_eq!(times, "1 1");
    assert_eq!(
        client("#{client_pid}"),
        s.child.process_id().unwrap().to_string()
    );
    // Its terminal's device, as tmux's ttyname; a Windows console has none.
    #[cfg(unix)]
    assert_eq!(
        Some(std::path::PathBuf::from(client("#{client_tty}"))),
        s.master.tty_name()
    );
    #[cfg(windows)]
    assert_eq!(client("[#{client_tty}]"), "[]");
    // The client is in "second" now, showing its window, and no longer in
    // "first".
    let name = client("#{client_name}");
    let lists = |target: &str| {
        let f =
            "[#{session_attached_list}] #{window_active_clients} [#{window_active_clients_list}]";
        t.run(&["display-message", "-p", "-t", target, f])
            .trim()
            .to_owned()
    };
    assert_eq!(lists("second"), format!("[{name}] 1 [{name}]"));
    assert_eq!(lists("first"), "[] 0 []");
    // The client runs as the user the server does.
    let server_user = t.run(&["display-message", "-p", "#{uid} #{user}"]);
    assert!(!server_user.trim().ends_with(' '), "{server_user:?}");
    assert_eq!(client("#{client_uid} #{client_user}"), server_user.trim());
    // In a loop the client's session is the current one, and the client loop
    // takes in the attached client.
    assert_eq!(
        client("#{S:#{session_name} ,[#{session_name}] }"),
        "first [second]"
    );
    let all = client("#{L:#{client_name}/}");
    assert!(all.split('/').any(|n| n == name), "{all:?} has {name}");
}

#[test]
fn switch_client_takes_a_window_and_pane_r_and_z() {
    let t = Tmxr::new("switchwp");
    t.run(&["new-session", "-d", "-s", "here"]);
    t.run(&["new-session", "-d", "-s", "other"]);
    t.run(&["new-window", "-d", "-t", "other:1"]);
    t.run(&["split-window", "-d", "-t", "other:1"]);
    t.run(&["split-window", "-d", "-t", "other:1"]);
    let s = t.attach(&["attach", "-t", "here"]);
    s.wait_for("attached", |text| text.contains("here"));
    let show = |f: &str| {
        t.run(&["display-message", "-p", "-t", "other", f])
            .trim()
            .to_owned()
    };
    // A window and pane in -t become the session's current ones.
    t.run(&["switch-client", "-t", "other:1.1"]);
    t.wait_run(
        &["list-clients", "-F", "#{session_name}"],
        "the switch",
        |o| o.trim() == "other",
    );
    assert_eq!(show("#{window_index}.#{pane_index}"), "1.1");
    // -Z: a zoomed window stays zoomed on the pane picked.
    t.run(&["resize-pane", "-Z", "-t", "other:1.1"]);
    t.run(&["switch-client", "-Z", "-t", "other:1.2"]);
    assert_eq!(show("#{pane_index} #{window_zoomed_flag}"), "2 1");
    t.run(&["switch-client", "-t", "other:1.0"]);
    assert_eq!(show("#{pane_index} #{window_zoomed_flag}"), "0 0");
    // -r turns the client read-only, and back.
    let flags = || {
        t.run(&["list-clients", "-F", "#{client_readonly}"])
            .trim()
            .to_owned()
    };
    t.run(&["switch-client", "-r"]);
    assert_eq!(flags(), "1");
    t.run(&["switch-client", "-r"]);
    assert_eq!(flags(), "0");
    // attach-session takes a window and pane in -t the same way.
    t.run(&["new-session", "-d", "-s", "third"]);
    t.run(&["new-window", "-d", "-t", "third:1"]);
    t.run(&["split-window", "-d", "-t", "third:1"]);
    let third = t.attach(&["attach", "-t", "third:1.1"]);
    third.wait_for("attached to third", |text| text.contains("third"));
    let at = t.run(&[
        "display-message",
        "-p",
        "-t",
        "third",
        "#{window_index}.#{pane_index}",
    ]);
    assert_eq!(at.trim(), "1.1");
}

#[test]
fn switch_client_c_names_the_client_to_move() {
    let t = Tmxr::new("switchc");
    t.run(&["new-session", "-d", "-s", "one"]);
    t.run(&["new-session", "-d", "-s", "two"]);
    t.run(&["new-session", "-d", "-s", "three"]);
    let a = t.attach(&["attach", "-t", "one"]);
    a.wait_for("first client", |text| text.contains("one"));
    let b = t.attach(&["attach", "-t", "two"]);
    b.wait_for("second client", |text| text.contains("two"));
    let clients = || t.run(&["list-clients", "-F", "#{client_name} #{session_name}"]);
    let id_on = |session: &str| {
        clients()
            .lines()
            .find(|l| l.ends_with(&format!(" {session}")))
            .and_then(|l| l.split(' ').next())
            .map_or_else(
                || panic!("no client on {session}: {}", clients()),
                str::to_owned,
            )
    };
    let first = id_on("one");
    let second = id_on("two");
    t.run(&["switch-client", "-c", &first, "-t", "three"]);
    t.wait_run(
        &["list-clients", "-F", "#{client_name} #{session_name}"],
        "the move",
        |o| o.lines().any(|l| l == format!("{first} three")),
    );
    // The other client stayed where it was.
    assert_eq!(id_on("two"), second);
    assert!(
        !t.output(&["switch-client", "-c", "999", "-t", "one"])
            .status
            .success()
    );
}

#[test]
fn clients_are_listed_and_detached_by_id() {
    let t = Tmxr::new("clients");
    t.run(&["new-session", "-d", "-s", "cl"]);
    let first = t.attach(&["attach", "-t", "cl"]);
    first.wait_for("first attached", |text| text.contains("cl"));
    let second = t.attach(&["attach", "-t", "cl"]);
    second.wait_for("second attached", |text| text.contains("cl"));
    let ids = |t: &Tmxr| -> Vec<String> {
        t.run(&["list-clients"])
            .lines()
            .filter_map(|l| l.split(':').next().map(str::to_owned))
            .collect()
    };
    let both = t.wait_run(&["list-clients"], "two clients", |o| o.lines().count() == 2);
    assert!(both.lines().all(|l| l.contains(": cl [")), "{both}");

    // detach-client -t detaches exactly that client.
    let before = ids(&t);
    t.run(&["detach-client", "-t", &before[0]]);
    t.wait_run(&["list-clients"], "one client left", |o| {
        o.lines().count() == 1
    });
    assert_eq!(ids(&t), [before[1].clone()]);
    assert!(!t.output(&["detach-client", "-t", "999"]).status.success());

    // prefix D lists the clients; Enter detaches the chosen one.
    // Ids grow, so the first client attached has the smaller one.
    let num = |id: &String| id.parse::<u32>().unwrap();
    let (detached, remaining) = if num(&before[0]) < num(&before[1]) {
        (first, second)
    } else {
        (second, first)
    };
    drop(detached);
    remaining.send(PREFIX);
    remaining.send(b"D");
    remaining.wait_for("client picker", |text| text.contains("clients 1/1"));
    remaining.send(b"\r");
    t.wait_run(&["list-clients"], "no clients", str::is_empty);
}

#[cfg(unix)]
#[test]
fn suspend_client_stops_and_fg_resumes() {
    let t = Tmxr::new("suspend");
    // tmxr under an interactive shell, so it has job control to stop in.
    let sh = t.spawn("/bin/sh", &["-i".to_owned()]);
    let line: Vec<String> = std::iter::once(env!("CARGO_BIN_EXE_tmxr").to_owned())
        .chain(t.args(&["new", "-s", "sus"]))
        .collect();
    sh.send(format!("{}\r", line.join(" ")).as_bytes());
    // The typed command line holds "sus" too: attached means tmxr has taken
    // the screen, so the line is gone and the status line shows the session.
    sh.wait_for("attached", |text| {
        text.contains("sus") && !text.contains("new -s sus")
    });
    sh.send(PREFIX);
    sh.send(b"\x1a");
    sh.wait_for("stopped job", |text| {
        text.to_lowercase().contains("stopped")
    });
    sh.send(b"fg\r");
    sh.wait_for("redrawn after fg", |text| {
        text.lines().last().is_some_and(|l| l.contains("sus"))
    });
}

#[cfg(windows)]
#[test]
fn suspend_client_says_windows_cannot() {
    let t = Tmxr::new("suspend");
    let s = t.attach(&["new", "-s", "sus"]);
    s.wait_for("status line", |text| text.contains("sus"));
    s.send(PREFIX);
    s.send(b"\x1a");
    s.wait_for("error", |text| text.contains("no job control"));
}

#[test]
fn detach_stays_responsive_under_flood_output() {
    // A pane printing changing lines as fast as it can.
    #[cfg(unix)]
    let flood = ["/bin/sh", "-c", "seq 1 999999999"];
    #[cfg(windows)]
    let flood = [
        "cmd.exe",
        "/d",
        "/c",
        "for /l %i in (0,1,99999999) do @echo %i yyyyyyyyyyyyyyyyyyyyyyyy",
    ];
    let t = Tmxr::new("flood");
    let mut args = vec!["new", "-s", "fl"];
    args.extend(flood);
    let mut s = t.attach(&args);
    std::thread::sleep(Duration::from_secs(2));
    let start = Instant::now();
    s.send(PREFIX);
    s.send(b"d");
    s.wait_exit();
    let took = start.elapsed();
    assert!(
        took < Duration::from_secs(5),
        "detach took {took:?} under flood"
    );
    eprintln!("detach under flood: {took:?}");
}

/// A pipe the starter of `tmxr new -d` left inheritable (a script reading
/// its output to the end) reaches end-of-file when the client exits, though
/// the server it started keeps running.
#[test]
fn the_server_does_not_keep_its_starters_pipes_open() {
    use std::io::Read;
    let t = Tmxr::new("inherit");
    let (mut reader, writer) = std::io::pipe().unwrap();
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: clears close-on-exec on a descriptor this test owns.
        assert_eq!(
            unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_SETFD, 0) },
            0
        );
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
        // SAFETY: sets the inherit flag on a handle this test owns.
        let ok = unsafe {
            SetHandleInformation(
                writer.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        };
        assert_ne!(ok, 0);
    }
    let started = t.output(&["new-session", "-d", "-s", "ih"]);
    assert!(started.status.success(), "{started:?}");
    drop(writer);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut rest = Vec::new();
        let _ = reader.read_to_end(&mut rest);
        let _ = tx.send(());
    });
    assert!(
        rx.recv_timeout(Duration::from_secs(10)).is_ok(),
        "the server still holds its starter's pipe"
    );
    assert!(
        t.run(&["ls"]).contains("ih:"),
        "the server is still running"
    );
}

#[test]
fn lock_hands_the_keys_to_the_lock_command() {
    let t = Tmxr::new("lock");
    let s = t.attach(&["new", "-s", "lk"]);
    s.wait_for("status line", |text| text.contains("lk"));
    // Prints a marker, then waits for a line from the terminal, after a
    // pause: the keys arrive while nothing but the client could read them,
    // which is when a client still reading would take them.
    let lock = if cfg!(windows) {
        "echo LOCKED-NOW& ping -n 3 127.0.0.1 >nul& set /p x="
    } else {
        "echo LOCKED-NOW; sleep 2; read x"
    };
    t.run(&["set-option", "-g", "lock-command", lock]);
    t.run(&["lock-session", "-t", "lk"]);
    s.wait_for("the lock command", |text| text.contains("LOCKED-NOW"));
    // The lock command gets the line, so it exits and the client is back.
    s.send(b"secret\r");
    s.wait_for("the client back", |text| {
        text.contains("lk") && !text.contains("LOCKED-NOW")
    });
    let pane = t.run(&["capture-pane", "-p", "-t", "lk"]);
    assert!(
        !pane.contains("secret"),
        "the pane got the lock's keys:\n{pane}"
    );

    // A lock command that fails says so, and the client is back.
    t.run(&["set-option", "-g", "lock-command", "exit 3"]);
    t.run(&["lock-client"]);
    t.wait_run(&["show-messages"], "the failure", |o| o.contains("exit 3"));
    t.run(&["set-option", "-g", "lock-command", ""]);
    assert!(!t.output(&["lock-server"]).status.success());
}

#[test]
fn kill_session_c_clears_alerts_and_unbind_q_is_quiet() {
    let t = Tmxr::new("killc");
    t.run(&["new-session", "-d", "-s", "kc"]);
    t.run(&["new-window", "-d", "-t", "kc:1"]);
    t.run(&["set-window-option", "-g", "monitor-activity", "on"]);
    t.wait_prompt("kc:1");
    t.run(&["send-keys", "-t", "kc:1", "echo busy", "Enter"]);
    let activity = |t: &Tmxr| {
        t.run(&[
            "display-message",
            "-p",
            "-t",
            "kc:1",
            "#{window_activity_flag}",
        ])
        .trim()
        .to_owned()
    };
    t.wait_run(
        &[
            "display-message",
            "-p",
            "-t",
            "kc:1",
            "#{window_activity_flag}",
        ],
        "activity",
        |o| o.trim() == "1",
    );
    // -C clears the session's alert flags and leaves it be.
    t.run(&["set-window-option", "-g", "monitor-activity", "off"]);
    t.run(&["kill-session", "-C", "-t", "kc"]);
    assert_eq!(activity(&t), "0");
    assert!(t.output(&["has-session", "-t", "kc"]).status.success());

    // unbind-key -q: an unknown key is no error.
    assert!(!t.output(&["unbind-key", "NoSuchKey"]).status.success());
    assert!(
        t.output(&["unbind-key", "-q", "NoSuchKey"])
            .status
            .success()
    );
    // set-buffer with neither data nor -n says so.
    let out = t.output(&["set-buffer"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no data specified"));
}

#[test]
fn detach_client_e_runs_a_command_in_the_clients_place() {
    let t = Tmxr::new("detexec");
    let mut s = t.attach(&["new", "-s", "de"]);
    s.wait_for("status line", |text| text.contains("de"));
    let name = t.run(&["list-clients", "-F", "#{client_name}"]);
    // An empty command does nothing, as tmux's.
    t.run(&["detach-client", "-E", "", "-t", name.trim()]);
    assert_eq!(t.run(&["list-clients"]).lines().count(), 1);
    // The command runs in the client's terminal, with no detach message.
    t.run(&["detach-client", "-E", "echo zqexec", "-t", name.trim()]);
    s.wait_for("the command's output", |text| text.contains("zqexec"));
    s.wait_exit();
    assert!(!s.text().contains("[detached"), "{}", s.text());
    assert_eq!(t.run(&["list-clients"]).trim(), "");
}

#[test]
fn read_only_clients_look_but_do_not_touch() {
    let t = Tmxr::new("readonly");
    t.run(&["new-session", "-d", "-s", "ro"]);
    let s = t.attach(&["attach", "-r", "-t", "ro"]);
    s.wait_for("status line", |text| text.contains("ro"));
    // Typing reaches no pane.
    s.send(b"echo ro-typed\r");
    std::thread::sleep(Duration::from_millis(500));
    let pane = t.run(&["capture-pane", "-p", "-t", "ro"]);
    assert!(!pane.contains("ro-typed"), "{pane}");
    // A bind that changes something is refused, and says so.
    s.send(PREFIX);
    s.send(b"c");
    s.wait_for("the refusal", |text| text.contains("read-only"));
    assert_eq!(t.run(&["list-windows", "-t", "ro"]).lines().count(), 1);
    // Detaching still works.
    s.send(PREFIX);
    s.send(b"d");
    s.wait_for("detached", |text| text.contains("[detached"));
}

#[test]
fn server_access_lists_who_may_connect() {
    let t = Tmxr::new("access");
    let me = std::env::var(if cfg!(windows) { "USERNAME" } else { "USER" }).ok();
    let other = if cfg!(windows) { "Guest" } else { "nobody" };
    // The default socket-access keeps everyone else out: -a says why.
    t.run(&["new-session", "-d", "-s", "own"]);
    let refused = t.output(&["server-access", "-a", other]);
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("socket-access"),
        "{refused:?}"
    );
    assert!(
        !t.output(&["set-option", "-g", "socket-access", "users"])
            .status
            .success()
    );
    t.run(&["kill-server"]);

    // socket-access users, on a socket outside tmxr's private directory.
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(&t.config, format!("socket-access = \"users\"\n{config}")).unwrap();
    let sock = if cfg!(windows) {
        format!(r"\\.\pipe\tmxr-e2e-access-{}", std::process::id())
    } else {
        t.dir
            .path()
            .join("shared")
            .join("tmxr.sock")
            .display()
            .to_string()
    };
    let at = |rest: &[&str]| t.output_at(&sock, rest);
    let started = at(&["new-session", "-d", "-s", "sa"]);
    assert!(started.status.success(), "{started:?}");
    let list = || String::from_utf8_lossy(&at(&["server-access", "-l"]).stdout).into_owned();
    assert_eq!(list(), "");
    if let Some(me) = &me {
        let own = at(&["server-access", "-a", me]);
        assert!(
            String::from_utf8_lossy(&own.stderr).contains("owns the server"),
            "{own:?}"
        );
    }
    assert!(at(&["server-access", "-a", other]).status.success());
    assert_eq!(list().trim(), format!("{other} (W)"));
    assert!(at(&["server-access", "-r", other]).status.success());
    assert_eq!(list().trim(), format!("{other} (R)"));
    assert!(at(&["server-access", "-w", other]).status.success());
    assert_eq!(list().trim(), format!("{other} (W)"));
    assert!(at(&["server-access", "-d", other]).status.success());
    assert_eq!(list(), "");
    assert!(!at(&["server-access", "-d", other]).status.success());
    assert!(
        !at(&["server-access", "-a", "no-such-user-tmxr"])
            .status
            .success()
    );
    at(&["kill-server"]);

    // A label's socket is in tmxr's private directory on Unix: -a says so.
    if cfg!(unix) {
        t.run(&["new-session", "-d", "-s", "lbl"]);
        let private = t.output(&["server-access", "-a", other]);
        assert!(
            String::from_utf8_lossy(&private.stderr).contains("-S"),
            "{private:?}"
        );
    }
}

#[test]
fn lock_after_time_locks_an_idle_client() {
    let t = Tmxr::new("lockidle");
    let s = t.attach(&["new", "-s", "li"]);
    s.wait_for("status line", |text| text.contains("li"));
    let lock = if cfg!(windows) {
        "echo IDLE-LOCKED& set /p x="
    } else {
        "echo IDLE-LOCKED; read x"
    };
    t.run(&["set-option", "-g", "lock-command", lock]);
    t.run(&["set-option", "-g", "lock-after-time", "1"]);
    // Nothing typed: locked by itself.
    s.wait_for("the idle lock", |text| text.contains("IDLE-LOCKED"));
    s.send(b"unlock\r");
    s.wait_for("the client back", |text| {
        text.contains("li") && !text.contains("IDLE-LOCKED")
    });
    // Unlocking counts as activity: not locked again at once.
    std::thread::sleep(Duration::from_millis(500));
    assert!(!s.text().contains("IDLE-LOCKED"), "locked again at once");
    t.run(&["set-option", "-g", "lock-after-time", "0"]);
}

#[test]
fn refresh_client_pans_a_window_larger_than_the_client() {
    let t = Tmxr::new("pan");
    let s = t.attach(&["new", "-s", "pa"]);
    s.wait_for("status line", |text| text.contains("pa"));
    s.wait_mouse(true);
    t.run(&["resize-window", "-t", "pa", "-x", "150", "-y", "40"]);
    t.run(&["split-window", "-h", "-t", "pa"]);
    t.run(&["select-pane", "-t", "pa:0.0"]);
    t.wait_prompt("pa:0.0");
    let left: u16 = t
        .run(&["display-message", "-p", "-t", "pa:0.0", "#{pane_width}"])
        .trim()
        .parse()
        .unwrap();
    // Where the border between the panes shows on the client's top row.
    let border = |s: &Screen| s.row_cells(0).iter().position(|c| c.0 == "│");
    let border_at = |s: &Screen, want: u16, what: &str| {
        s.wait_for(what, |_| border(s) == Some(usize::from(want)));
    };
    // The left pane's cursor is in the first screenful: no offset.
    border_at(&s, left, "the window's left edge in view");
    t.run(&["refresh-client", "-R", "10"]);
    border_at(&s, left - 10, "panned right by 10");
    // No further than the window's edge: 150 - 100.
    t.run(&["refresh-client", "-R", "1000"]);
    border_at(&s, left - 50, "panned to the right edge");
    t.run(&["refresh-client", "-L", "20"]);
    border_at(&s, left - 30, "panned back by 20");
    // A click lands on the window cell under it: the right pane.
    s.click(0, left - 30 + 5, 3);
    t.wait_run(
        &["display-message", "-p", "-t", "pa", "#{pane_index}"],
        "the right pane",
        |o| o.trim() == "1",
    );
    // A box placed at the mouse lands by the click on screen, not at its
    // window cell: centred on column 40, its bottom row just below 10.
    let hold = if cfg!(windows) {
        "'echo PANPOP& ping -n 30 127.0.0.1 >NUL'"
    } else {
        "'echo PANPOP; sleep 30'"
    };
    let popup = format!("display-popup -x M -y M -w 10 -h 4 {hold}");
    t.run(&["bind-key", "-n", "MouseDown3Pane", &popup]);
    s.click(2, 40, 10);
    s.wait_for("the popup", |text| text.contains("PANPOP"));
    let corner = (0..ROWS).find_map(|r| {
        let col = s.row_cells(r).iter().position(|(c, _, _)| c == "┌")?;
        Some((col, r))
    });
    assert_eq!(corner, Some((35, 10)), "popup at the mouse while panned");
    t.run(&["display-popup", "-C"]);
    s.wait_for("closed", |text| !text.contains("PANPOP"));
    // Dragging the border three cells right widens the left pane by three.
    let at = left - 30;
    s.mouse(0, at, 5, false);
    s.mouse(32, at + 1, 5, false);
    s.mouse(32, at + 3, 5, false);
    s.mouse(0, at + 3, 5, true);
    t.wait_run(
        &["display-message", "-p", "-t", "pa:0.0", "#{pane_width}"],
        "the border dragged",
        |o| o.trim() == (left + 3).to_string(),
    );
    // -c follows the cursor again: back to the left pane's, at offset 0.
    t.run(&["select-pane", "-t", "pa:0.0"]);
    t.run(&["refresh-client", "-c"]);
    // The left pane, three wider since the drag.
    border_at(&s, left + 3, "following the cursor again");
}

#[test]
fn an_ignore_size_client_sizes_windows_only_alone() {
    let t = Tmxr::new("ignoresize");
    t.run(&["new-session", "-d", "-s", "is"]);
    let size = |t: &Tmxr| {
        t.run(&[
            "display-message",
            "-p",
            "-t",
            "is",
            "#{window_width}x#{window_height}",
        ])
        .trim()
        .to_owned()
    };
    let full = format!("{COLS}x{}", ROWS - 1);
    let mut a = t.attach(&["attach", "-t", "is"]);
    a.wait_for("the status line", |text| text.contains("is"));
    t.wait_run(&["list-clients"], "the first client", |o| {
        o.lines().count() == 1
    });
    assert_eq!(size(&t), full);
    let b = t.attach(&["attach", "-f", "ignore-size", "-t", "is"]);
    b.wait_for("the status line", |text| text.contains("is"));
    b.resize(20, 70);
    b.send(b"x");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(size(&t), full, "the ignore-size client sized the window");

    a.send(PREFIX);
    a.send(b"d");
    a.wait_exit();
    b.resize(21, 71);
    t.wait_run(
        &[
            "display-message",
            "-p",
            "-t",
            "is",
            "#{window_width}x#{window_height}",
        ],
        "the window sized for the client left",
        |o| o.trim() == "71x20",
    );
}

#[test]
fn refresh_client_sets_and_clears_client_flags() {
    let t = Tmxr::new("clientflags");
    t.run(&["new-session", "-d", "-s", "cf"]);
    let size = |t: &Tmxr| {
        t.run(&[
            "display-message",
            "-p",
            "-t",
            "cf",
            "#{window_width}x#{window_height}",
        ])
        .trim()
        .to_owned()
    };
    let flags = |t: &Tmxr| {
        t.run(&["list-clients", "-F", "#{client_flags} #{client_readonly}"])
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let a = t.attach(&["attach", "-t", "cf"]);
    a.wait_for("the status line", |text| text.contains("cf"));
    t.wait_run(&["list-clients"], "the first client", |o| {
        o.lines().count() == 1
    });
    let listed = flags(&t);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert!(listed[0].starts_with("attached") && listed[0].ends_with(" 0"));
    // The second client typed last, so the window takes its size and
    // refresh-client from outside means it.
    let b = t.attach(&["attach", "-t", "cf"]);
    b.wait_for("the status line", |text| text.contains("cf"));
    // On Linux under load a resize just after attaching can go unseen by
    // the client (docs/backlog.md, "A resize just after attaching"), so
    // resize again until the server has the size: this test is about the
    // flags, not that.
    let deadline = Instant::now() + TIMEOUT;
    loop {
        b.resize(21, 71);
        b.resize(20, 70);
        b.send(b"x");
        std::thread::sleep(Duration::from_millis(300));
        if size(&t) == "70x19" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the window never took the second client's size"
        );
    }

    // -f ignore-size gives the window back to the first client at once.
    t.run(&["refresh-client", "-f", "ignore-size"]);
    assert_eq!(size(&t), format!("{COLS}x{}", ROWS - 1));
    assert!(
        flags(&t).iter().any(|l| l.contains("ignore-size")),
        "{:?}",
        flags(&t)
    );
    // !ignore-size turns it off, and the window follows that client again.
    t.run(&["refresh-client", "-f", "!ignore-size"]);
    assert_eq!(size(&t), "70x19");
    assert!(!flags(&t).iter().any(|l| l.contains("ignore-size")));

    // read-only shows in both formats, and as in tmux !read-only leaves it.
    t.run(&["refresh-client", "-F", "read-only"]);
    let ro = |t: &Tmxr| {
        flags(t)
            .iter()
            .filter(|l| l.contains("read-only") && l.ends_with(" 1"))
            .count()
    };
    assert_eq!(ro(&t), 1, "{:?}", flags(&t));
    t.run(&["refresh-client", "-f", "!read-only"]);
    assert_eq!(ro(&t), 1, "{:?}", flags(&t));
    let out = t.output(&["refresh-client", "-f", "no-output"]);
    assert!(!out.status.success(), "{out:?}");
    drop(a);
}

#[test]
fn a_grouped_session_takes_no_command_and_lists_its_group() {
    let t = Tmxr::new("groups");
    t.run(&["new-session", "-d", "-s", "g"]);
    // tmux's error: the group's windows are the new session's.
    for extra in [&["-n", "x"][..], &["sleep", "10"][..]] {
        let args = [&["new-session", "-d", "-t", "g"][..], extra].concat();
        let refused = t.output(&args);
        assert!(!refused.status.success(), "{refused:?}");
        assert!(
            String::from_utf8_lossy(&refused.stderr)
                .contains("command or window name given with target"),
            "{refused:?}"
        );
    }
    t.run(&["new-session", "-d", "-t", "g", "-s", "member"]);
    // Windows do not move between two members, which hold the same ones.
    for (args, error) in [
        (
            &["swap-window", "-s", "g:0", "-t", "member:0"][..],
            "can't move window, sessions are grouped",
        ),
        (
            &["move-window", "-s", "g:0", "-t", "member:3"][..],
            "sessions are grouped",
        ),
        (
            &["link-window", "-s", "g:0", "-t", "member:3"][..],
            "sessions are grouped",
        ),
    ] {
        let refused = t.output(args);
        assert!(!refused.status.success(), "{refused:?}");
        assert!(
            String::from_utf8_lossy(&refused.stderr).contains(error),
            "{refused:?}"
        );
    }
    let listed = t.run(&["list-sessions"]);
    assert!(
        listed
            .lines()
            .any(|l| l.starts_with("member: 1 windows (group g)")),
        "{listed}"
    );
}
