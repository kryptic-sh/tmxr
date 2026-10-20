use super::*;

#[test]
fn remain_on_exit_failed_monitor_bell_and_rename_format() {
    let t = Tmxr::new("winopts");
    t.run(&["new-session", "-d", "-s", "wo"]);
    t.run(&["set", "-g", "renumber-windows", "off"]);
    let show = |target: &str, f: &str| {
        t.run(&["display-message", "-p", "-t", target, f])
            .trim()
            .to_owned()
    };
    // automatic-rename-format is what a window is renamed to.
    t.run(&[
        "set",
        "-wg",
        "automatic-rename-format",
        "ar-#{window_index}",
    ]);
    t.wait_run(
        &["display-message", "-p", "-t", "wo:0", "#W"],
        "the window renamed",
        |o| o.trim() == "ar-0",
    );
    // remain-on-exit failed keeps only a pane whose program failed.
    t.run(&["set", "-g", "remain-on-exit", "failed"]);
    #[cfg(unix)]
    let (ok, fail) = (["/bin/sh", "-c", "exit 0"], ["/bin/sh", "-c", "exit 3"]);
    #[cfg(windows)]
    let (ok, fail) = (["cmd", "/c", "exit 0"], ["cmd", "/c", "exit 3"]);
    let mut new = vec!["new-window", "-d", "-t", "wo:1"];
    new.extend(ok);
    t.run(&new);
    let mut new = vec!["new-window", "-d", "-t", "wo:2"];
    new.extend(fail);
    t.run(&new);
    t.wait_run(
        &["list-windows", "-t", "wo", "-F", "#I #{pane_dead}"],
        "only the failed pane kept",
        |o| o == "0 0\n2 1\n",
    );
    // monitor-bell off: a bell flags nothing; on, it flags the window.
    #[cfg(unix)]
    let ring = ["/bin/sh", "-c", r"printf '\007rung'; sleep 30"];
    #[cfg(windows)]
    let ring = [
        "powershell.exe",
        "-NoProfile",
        "-Command",
        "[Console]::Out.Write([string][char]7 + 'rung'); Start-Sleep 30",
    ];
    t.run(&["set", "-g", "remain-on-exit", "off"]);
    t.run(&["set", "-wg", "monitor-bell", "off"]);
    let mut new = vec!["new-window", "-d", "-t", "wo:5"];
    new.extend(ring);
    t.run(&new);
    t.wait_run(&["capture-pane", "-p", "-t", "wo:5"], "rung", |o| {
        o.contains("rung")
    });
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(show("wo:5", "#{window_bell_flag}"), "0");
    t.run(&["set", "-wg", "monitor-bell", "on"]);
    let mut again = vec!["new-window", "-d", "-t", "wo:6"];
    again.extend(ring);
    t.run(&again);
    t.wait_run(
        &["display-message", "-p", "-t", "wo:6", "#{window_bell_flag}"],
        "the bell flag",
        |o| o.trim() == "1",
    );
}

#[test]
fn default_command_is_what_a_new_pane_runs() {
    let t = Tmxr::new("defcmd");
    t.run(&["new-session", "-d", "-s", "dc"]);
    let (marked, other) = if cfg!(windows) {
        (
            "echo dc-marker& ping -n 100 127.0.0.1 >NUL",
            "echo given-cmd& ping -n 100 127.0.0.1 >NUL",
        )
    } else {
        (
            "echo dc-marker; exec sleep 100",
            "echo given-cmd; exec sleep 100",
        )
    };
    t.run(&["set", "-g", "default-command", marked]);
    let shows = |target: &str, text: &'static str| {
        t.wait_run(&["capture-pane", "-p", "-t", target], text, |o| {
            o.lines().any(|l| l.trim() == text)
        });
    };
    // A new window and a split run it; a command given wins.
    t.run(&["new-window", "-t", "dc:1"]);
    shows("dc:1", "dc-marker");
    t.run(&["split-window", "-t", "dc:1"]);
    shows("dc:1.1", "dc-marker");
    t.run(&["new-window", "-t", "dc:2", other]);
    shows("dc:2", "given-cmd");
}

#[test]
fn panes_join_and_swap_across_windows() {
    let t = Tmxr::new("joinp");
    let pane_id = |t: &Tmxr, target: &str| {
        t.run(&["display-message", "-p", "-t", target, "#{pane_id}"])
            .trim()
            .to_owned()
    };
    let count = |t: &Tmxr, args: &[&str]| t.run(args).lines().count();
    t.run(&["new-session", "-d", "-s", "p", "-n", "w1"]);
    t.run(&["split-window", "-d", "-t", "p:w1"]);
    t.run(&["new-window", "-d", "-t", "p", "-n", "w2"]);
    let joined = pane_id(&t, "p:w2");
    assert!(joined.starts_with('%'), "{joined:?}");

    // Joining a window's only pane elsewhere closes that window.
    t.run(&["join-pane", "-d", "-h", "-s", "p:w2", "-t", "p:w1.0"]);
    assert_eq!(count(&t, &["list-windows", "-t", "p"]), 1);
    assert_eq!(count(&t, &["list-panes", "-t", "p:w1"]), 3);
    assert!(
        t.run(&["list-panes", "-t", "p:w1"]).contains(&joined),
        "{joined} not in w1"
    );

    // Swap a pane with one in another window.
    t.run(&["new-window", "-d", "-t", "p", "-n", "w3"]);
    let (lone, other) = (pane_id(&t, "p:w3"), pane_id(&t, "p:w1.1"));
    assert!(lone.starts_with('%') && other.starts_with('%') && lone != other);
    t.run(&["swap-pane", "-d", "-s", "p:w3", "-t", "p:w1.1"]);
    assert_eq!(pane_id(&t, "p:w3"), other);
    assert_eq!(pane_id(&t, "p:w1.1"), lone);
}

#[test]
fn respawn_pane_restarts_the_program_in_place() {
    let t = Tmxr::new("respawn");
    t.run(&["new-session", "-d", "-s", "r"]);
    t.run(&["split-window", "-d", "-t", "r"]);
    let id = t.run(&["display-message", "-p", "-t", "r.0", "#{pane_id}"]);
    assert!(id.starts_with('%'), "{id:?}");
    t.wait_prompt("r.0");
    t.run(&["send-keys", "-t", "r.0", "echo before-respawn", "Enter"]);
    t.wait_run(&["capture-pane", "-p", "-t", "r.0"], "marker", |o| {
        o.matches("before-respawn").count() >= 2
    });

    // Without -k the running program is left alone.
    t.run(&["respawn-pane", "-t", "r.0"]);
    assert!(
        t.run(&["capture-pane", "-p", "-t", "r.0"])
            .contains("before-respawn")
    );

    // With -k a fresh program starts in the same pane: same id, clean screen.
    t.run(&["respawn-pane", "-k", "-t", "r.0"]);
    t.wait_run(&["capture-pane", "-p", "-t", "r.0"], "fresh screen", |o| {
        !o.contains("before-respawn")
    });
    // The old program's exit arrives after the respawn and must not close
    // the new one.
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "r.0", "#{pane_id}"]),
        id
    );
    assert_eq!(t.run(&["list-panes", "-t", "r"]).lines().count(), 2);
}

#[test]
fn rotate_window_moves_panes_up_by_default_like_tmux() {
    let t = Tmxr::new("rotate");
    t.run(&["new-session", "-d", "-s", "r"]);
    t.run(&["split-window", "-d", "-t", "r"]);
    t.run(&["split-window", "-d", "-t", "r"]);
    let order = |t: &Tmxr| -> Vec<String> {
        t.run(&["list-panes", "-t", "r"])
            .lines()
            .filter_map(|l| l.split_whitespace().find(|w| w.starts_with('%')))
            .map(str::to_owned)
            .collect()
    };
    let before = order(&t);
    assert_eq!(before.len(), 3, "{before:?}");
    t.run(&["rotate-window", "-t", "r"]);
    assert_eq!(order(&t), [before[1].as_str(), &before[2], &before[0]]);
    t.run(&["rotate-window", "-D", "-t", "r"]);
    assert_eq!(order(&t), before);
}

#[test]
fn passthrough_is_forwarded_where_the_platform_allows() {
    let t = Tmxr::new("passthrough");
    // A program that wraps an OSC in tmux's passthrough DCS (every ESC inside
    // doubled), prints a marker after it, then waits.
    #[cfg(unix)]
    let program = [
        "/bin/sh",
        "-c",
        r"printf 'abc\033Ptmux;\033\033]1337;tmxr-pt-marker\007\033\\after-dcs'; sleep 30",
    ];
    #[cfg(windows)]
    let program = [
        "powershell.exe",
        "-NoProfile",
        "-Command",
        r"$e=[char]27; [Console]::Out.Write('abc'+$e+'Ptmux;'+$e+$e+']1337;tmxr-pt-marker'+[char]7+$e+'\'+'after-dcs'); Start-Sleep 30",
    ];
    let mut args = vec!["new", "-s", "pt"];
    args.extend(program);
    let s = t.attach(&args);
    // On every platform, output after the DCS still reaches the pane.
    s.wait_for("text after the DCS", |text| text.contains("after-dcs"));
    assert!(!s.text().contains("tmux;"), "{}", s.text());

    let forwarded = s.raw_text().contains("\x1b]1337;tmxr-pt-marker\x07");
    // On Windows passthrough needs Microsoft's ConPTY beside tmxr.exe (the
    // release ships it; CI's conpty job adds it): with it the payload is
    // forwarded, without it tmxr leaves passthrough alone and says so.
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_tmxr"));
    let bundled = bin.with_file_name("conpty.dll").is_file()
        && bin.with_file_name("OpenConsole.exe").is_file();
    if cfg!(windows) && !bundled {
        assert!(!forwarded);
        let messages = t.run(&["show-messages"]);
        assert!(messages.contains("allow-passthrough"), "{messages}");
    } else {
        assert!(forwarded, "payload not forwarded: {:?}", s.raw_text());
        // From the cell it was printed at, after "abc": row 1, column 4.
        assert!(
            s.raw_text()
                .contains("\x1b[1;4H\x1b]1337;tmxr-pt-marker\x07"),
            "not placed at the pane's cursor: {:?}",
            s.raw_text()
        );
    }
}

#[test]
fn select_layout_spread_evens_out_a_row_of_panes() {
    let t = Tmxr::new("spread");
    t.run(&["new-session", "-d", "-s", "sp", "-x", "100", "-y", "30"]);
    t.run(&["split-window", "-h", "-t", "sp"]);
    t.run(&["split-window", "-h", "-t", "sp"]);
    let widths = |t: &Tmxr| -> Vec<u16> {
        t.run(&["list-panes", "-t", "sp"])
            .lines()
            .filter_map(|l| l.split(['[', 'x']).nth(1)?.parse().ok())
            .collect()
    };
    let before = widths(&t);
    assert_eq!(before.len(), 3, "{before:?}");
    assert!(before[0] > before[2] + 10, "uneven first: {before:?}");
    t.run(&["select-layout", "-E", "-t", "sp"]);
    let after = widths(&t);
    let (min, max) = (after.iter().min().unwrap(), after.iter().max().unwrap());
    assert!(max - min <= 1, "{after:?}");
}

#[test]
fn marked_pane_is_the_default_source_of_join_pane() {
    let t = Tmxr::new("mark");
    t.run(&["new-session", "-d", "-s", "mk", "-n", "one"]);
    t.run(&["split-window", "-d", "-t", "mk:one"]);
    t.run(&["new-window", "-d", "-t", "mk", "-n", "two"]);
    let marked = t.run(&["display-message", "-p", "-t", "mk:one.1", "#{pane_id}"]);
    let marked = marked.trim();
    assert!(marked.starts_with('%'), "{marked:?}");

    // Mark a pane: its window shows the M flag.
    t.run(&["select-pane", "-m", "-t", "mk:one.1"]);
    let windows = t.run(&["list-windows", "-t", "mk"]);
    assert!(
        windows.lines().any(|l| l.starts_with("0: one*M (")),
        "{windows}"
    );
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "mk:one.1", "#{pane_marked}"])
            .trim(),
        "1"
    );

    // join-pane without -s takes the marked pane.
    let out = t.output(&["join-pane", "-d", "-t", "mk:two"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(t.run(&["list-panes", "-t", "mk:two"]).contains(marked));
    assert_eq!(t.run(&["list-panes", "-t", "mk:one"]).lines().count(), 1);

    // -M clears it; then join-pane has no source.
    t.run(&["select-pane", "-M"]);
    assert!(!t.run(&["list-windows", "-t", "mk"]).contains('M'));
    assert!(!t.output(&["join-pane", "-t", "mk:one"]).status.success());
}

#[test]
fn remain_on_exit_keeps_a_dead_pane_until_respawned() {
    let t = Tmxr::new("remain");
    #[cfg(unix)]
    let (exits, lasts): (&[&str], &[&str]) = (&["/bin/sh", "-c", "exit 3"], &["/bin/sleep", "30"]);
    #[cfg(windows)]
    let (exits, lasts): (&[&str], &[&str]) = (
        &["cmd.exe", "/d", "/c", "exit 3"],
        &["ping.exe", "-n", "30", "127.0.0.1"],
    );
    t.run(&["new-session", "-d", "-s", "rm"]);
    t.run(&["set-option", "-g", "remain-on-exit", "on"]);
    let mut split = vec!["split-window", "-d", "-t", "rm"];
    split.extend(exits);
    t.run(&split);
    let dead = |t: &Tmxr| {
        t.run(&[
            "display-message",
            "-p",
            "-t",
            "rm.1",
            "#{pane_dead}:#{pane_dead_status}",
        ])
        .trim()
        .to_owned()
    };
    t.wait_run(
        &["display-message", "-p", "-t", "rm.1", "#{pane_dead}"],
        "pane dead",
        |o| o.trim() == "1",
    );
    assert_eq!(dead(&t), "1:3");
    assert_eq!(t.run(&["list-panes", "-t", "rm"]).lines().count(), 2);

    // A dead pane respawns without -k.
    let mut respawn = vec!["respawn-pane", "-t", "rm.1"];
    respawn.extend(lasts);
    let out = t.output(&respawn);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(dead(&t), "0:");
}

#[test]
fn pane_current_path_follows_the_shells_cd() {
    let t = Tmxr::new("cwd");
    let sub = t.dir.path().join("moved-here");
    std::fs::create_dir(&sub).unwrap();
    let s = t.attach(&["new", "-s", "cw"]);
    s.wait_for("status line", |text| text.contains("cw"));
    // cmd reports no directory itself, so on Windows this is read from the
    // shell's process (its PEB); on Unix from /proc or proc_pidinfo.
    let cd = if cfg!(windows) { "cd /d" } else { "cd" };
    s.send(format!("{cd} \"{}\"\r", sub.display()).as_bytes());
    let want = sub.canonicalize().unwrap();
    let in_sub = |what: &str| {
        t.wait_run(
            &["display-message", "-p", "-t", "cw", "#{pane_current_path}"],
            what,
            |o| std::path::Path::new(o.trim()).canonicalize().ok() == Some(want.clone()),
        );
    };
    in_sub("the new directory");
    // The tmux config's binds open new panes and windows there: prefix %
    // splits, prefix c makes a window, each started in the pane's directory.
    s.send(PREFIX);
    s.send(b"%");
    t.wait_run(&["list-panes", "-t", "cw"], "the split", |o| {
        o.lines().count() == 2
    });
    in_sub("the split pane in the directory");
    s.send(PREFIX);
    s.send(b"c");
    t.wait_run(
        &["display-message", "-p", "-t", "cw", "#{window_index}"],
        "the new window",
        |o| o.trim() == "1",
    );
    in_sub("the new window in the directory");
}

#[test]
fn panes_inherit_the_servers_path() {
    let t = Tmxr::new("path");
    let extra = t.dir.path().join("extra-bin");
    std::fs::create_dir(&extra).unwrap();
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(extra.clone()).chain(std::env::split_paths(&inherited)),
    )
    .unwrap();
    // The server starts with the extended PATH, as from a shell that set it.
    let started = tmxr_term::child::command(env!("CARGO_BIN_EXE_tmxr"))
        .args(t.args(&["new-session", "-d", "-s", "pa"]))
        .envs(t.env())
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(started.status.success(), "{started:?}");
    // Only the start of PATH, so it fits on the screen.
    let echo = if cfg!(windows) {
        "echo %PATH:~0,200%"
    } else {
        "echo ${PATH%%:*}"
    };
    t.wait_prompt("pa");
    t.run(&["send-keys", "-t", "pa", echo, "Enter"]);
    let want = extra.display().to_string();
    t.wait_run(
        &["capture-pane", "-p", "-t", "pa"],
        "the pane's PATH",
        |o| o.lines().any(|l| l.starts_with(&want)),
    );
}

#[test]
fn select_pane_z_keeps_a_zoomed_window_zoomed() {
    let t = Tmxr::new("selz");
    t.run(&["new-session", "-d", "-s", "z"]);
    t.run(&["split-window", "-h", "-t", "z"]);
    let state = |t: &Tmxr| {
        t.run(&[
            "display-message",
            "-p",
            "-t",
            "z",
            "#{pane_index} #{window_zoomed_flag}",
        ])
        .trim()
        .to_owned()
    };
    t.run(&["resize-pane", "-Z", "-t", "z"]);
    assert_eq!(state(&t), "1 1");
    t.run(&["select-pane", "-Z", "-L", "-t", "z"]);
    assert_eq!(state(&t), "0 1", "moved and still zoomed");
    // Without -Z, selecting another pane unzooms, as in tmux.
    t.run(&["select-pane", "-R", "-t", "z"]);
    assert_eq!(state(&t), "1 0");
}

#[test]
fn tmux_commands_move_pane_previous_layout_show_window_options_start_server() {
    let t = Tmxr::new("aliases");
    // start: a server with no sessions, which `ls` reaches.
    let started = t.output(&["start"]);
    assert!(started.status.success(), "{started:?}");
    let ls = t.output(&["ls"]);
    assert!(ls.status.success(), "no server after start: {ls:?}");

    t.run(&["new-session", "-d", "-s", "m"]);
    t.run(&["new-window", "-d", "-t", "m"]);
    // movep: window 1's pane joins window 0, and window 1 goes.
    t.run(&["movep", "-s", "m:1.0", "-t", "m:0"]);
    assert_eq!(t.run(&["list-panes", "-t", "m:0"]).lines().count(), 2);
    assert_eq!(t.run(&["list-windows", "-t", "m"]).lines().count(), 1);

    // prevl undoes nextl: side by side, then stacked, then side by side.
    let geometry = |t: &Tmxr| t.run(&["list-panes", "-t", "m:0"]);
    t.run(&["select-layout", "-t", "m:0", "even-horizontal"]);
    let before = geometry(&t);
    t.run(&["nextl", "-t", "m:0"]);
    assert_ne!(geometry(&t), before, "nextl changed nothing");
    t.run(&["prevl", "-t", "m:0"]);
    assert_eq!(geometry(&t), before);

    let showw = t.run(&["showw", "-g"]);
    assert!(showw.contains("mode-keys"), "{showw}");
}

#[test]
fn pipe_pane_copies_output_to_a_command() {
    let t = Tmxr::new("pipepane");
    let log = t.dir.path().join("pane.log");
    #[cfg(unix)]
    let command = format!("cat >> '{}'", log.display());
    #[cfg(windows)]
    let command = format!(
        "$input | ForEach-Object {{ Add-Content -LiteralPath '{}' -Value $_ }}",
        log.display()
    );
    t.run(&["new-session", "-d", "-s", "pp"]);
    // The command is PowerShell, and shell commands run with default-shell.
    #[cfg(windows)]
    t.run(&["set-option", "-g", "default-shell", "powershell.exe"]);
    let piped = |t: &Tmxr| t.run(&["display-message", "-p", "-t", "pp", "#{pane_pipe}"]);
    t.run(&["pipep", "-t", "pp", &command]);
    assert_eq!(piped(&t).trim(), "1");
    t.wait_prompt("pp");
    t.run(&["send-keys", "-t", "pp", "echo @piped-text", "Enter"]);
    t.wait_run(&["capture-pane", "-p", "-t", "pp"], "the echo", |o| {
        o.lines().any(|l| l.trim_end() == "@piped-text")
    });
    // No command closes the pipe; the command then finishes its input.
    t.run(&["pipep", "-t", "pp"]);
    assert_eq!(piped(&t).trim(), "0");
    let deadline = Instant::now() + TIMEOUT;
    while !std::fs::read_to_string(&log).is_ok_and(|s| s.contains("@piped-text")) {
        assert!(
            Instant::now() < deadline,
            "the output never reached the log"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // -o toggles: it opens a pipe when none is open, and closes an open one.
    t.run(&["pipep", "-o", "-t", "pp", &command]);
    assert_eq!(piped(&t).trim(), "1");
    t.run(&["pipep", "-o", "-t", "pp", &command]);
    assert_eq!(piped(&t).trim(), "0");
}

#[test]
fn pipe_pane_input_types_the_commands_output() {
    let t = Tmxr::new("pipein");
    t.run(&["new-session", "-d", "-s", "pi"]);
    // The pipe's command prints a command line; the pane's shell runs it.
    t.run(&["pipe-pane", "-I", "-t", "pi", "echo echo from-the-pipe"]);
    t.wait_run(
        &["capture-pane", "-p", "-t", "pi"],
        "the typed command's output",
        |o| o.lines().any(|l| l.trim_end() == "from-the-pipe"),
    );
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "pi", "#{pane_pipe}"])
            .trim(),
        "1"
    );
}

#[test]
fn splits_size_panes_as_tmux_does() {
    let t = Tmxr::new("splitsize");
    // Each as tmux 3.6 split the same 100x30 window.
    let cases: &[(&[&str], &str, &str)] = &[
        (&["-h"], "#{pane_left}+#{pane_width}", "0+50 51+49"),
        (&["-v"], "#{pane_top}+#{pane_height}", "0+15 16+14"),
        (&["-h", "-l", "30"], "#{pane_width}", "69 30"),
        (&["-h", "-b", "-l", "30"], "#{pane_width}", "30 69"),
    ];
    for (i, (flags, format, tmux)) in cases.iter().enumerate() {
        let name = format!("s{i}");
        t.run(&["new-session", "-d", "-s", &name, "-x", "100", "-y", "30"]);
        let mut split = vec!["split-window", "-t", name.as_str()];
        split.extend_from_slice(flags);
        t.run(&split);
        let got = t.run(&["list-panes", "-t", &name, "-F", format]);
        assert_eq!(
            got.split_whitespace().collect::<Vec<_>>().join(" "),
            *tmux,
            "{flags:?}"
        );
    }
}

#[test]
fn scrollbars_size_panes_as_tmux_does() {
    let t = Tmxr::new("sbsize");
    t.run(&["new-session", "-d", "-s", "s", "-x", "100", "-y", "30"]);
    t.run(&["set-option", "-g", "pane-scrollbars", "on"]);
    t.run(&["split-window", "-h", "-t", "s"]);
    let panes = |t: &Tmxr| {
        t.run(&["list-panes", "-t", "s", "-F", "#{pane_left}+#{pane_width}"])
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    // Each as tmux 3.6 laid out the same window.
    assert_eq!(panes(&t), "0+49 51+48", "on, right");
    t.run(&["set-option", "-g", "pane-scrollbars-position", "left"]);
    assert_eq!(panes(&t), "1+49 52+48", "left");
    t.run(&[
        "set-option",
        "-g",
        "pane-scrollbars-style",
        "bg=black,fg=white,width=2,pad=1",
    ]);
    assert_eq!(panes(&t), "3+47 54+46", "width 2, pad 1");
    t.run(&["set-option", "-g", "pane-scrollbars", "modal"]);
    assert_eq!(panes(&t), "0+50 51+49", "modal, no mode");
    t.run(&["copy-mode", "-t", "s:0.0"]);
    assert_eq!(panes(&t), "3+47 51+49", "modal, the left pane in copy mode");
    t.run(&["send-keys", "-t", "s:0.0", "-X", "cancel"]);
    assert_eq!(panes(&t), "0+50 51+49", "modal, copy mode left");
}

#[test]
fn a_scrollbar_draws_and_scrolls_with_the_mouse() {
    let t = Tmxr::new("sbmouse");
    let s = t.attach(&["new", "-s", "sb"]);
    s.wait_for("status line", |text| text.contains("sb"));
    s.wait_mouse(true);
    t.wait_prompt("sb");
    let fill = if cfg!(windows) {
        "for /l %i in (1,1,200) do @echo filler%i"
    } else {
        "seq -f filler%g 200"
    };
    t.run(&["send-keys", "-t", "sb", fill, "Enter"]);
    s.wait_for("the history", |text| text.contains("filler200"));
    t.run(&["set-option", "-g", "pane-scrollbars", "on"]);
    // The bar is the last column; its slider has the style's colours
    // swapped (fg=white,bg=black), down at the bottom of the history.
    let bar = COLS - 1;
    let slider_rows = |s: &Screen| -> Vec<u16> {
        (0..ROWS - 1)
            .filter(|&r| s.row_cells(r)[usize::from(bar)].2 == "Idx(7)")
            .collect()
    };
    s.wait_for("the bar", |_| !slider_rows(&s).is_empty());
    let slider = slider_rows(&s);
    assert!(slider[0] > 0, "the slider is not at the top: {slider:?}");
    assert_eq!(
        *slider.last().unwrap(),
        ROWS - 2,
        "the slider ends at the bottom"
    );
    assert_eq!(
        s.row_cells(0)[usize::from(bar)].2,
        "Idx(0)",
        "the bar above it"
    );
    // Above the slider: a page up, into copy mode.
    s.click(0, bar, 0);
    t.wait_run(
        &["display-message", "-p", "-t", "sb", "#{pane_in_mode}"],
        "copy mode",
        |o| o.trim() == "1",
    );
    // The slider dragged to the top: the start of the history.
    t.run(&[
        "bind-key",
        "-n",
        "MouseDragEnd1ScrollbarSlider",
        "set-option",
        "-g",
        "@seen",
        "slider",
    ]);
    let top = slider_rows(&s)[0];
    s.mouse(0, bar, top, false);
    s.mouse(32, bar, 0, false);
    s.mouse(0, bar, 0, true);
    s.wait_for("the top of the history", |text| {
        text.lines().any(|l| l.trim_end() == "filler1")
    });
    // Letting go ends the drag with tmux's key, on the slider.
    t.wait_run(&["show-options", "-gv", "@seen"], "the drag's end", |o| {
        o.trim() == "slider"
    });
}

#[test]
fn allow_set_title_off_keeps_programs_from_titling_panes() {
    let t = Tmxr::new("settitle");
    t.run(&["new-session", "-d", "-s", "st"]);
    // A program that titles its pane once the option has changed.
    #[cfg(unix)]
    let titler = [
        "/bin/sh",
        "-c",
        r"sleep 1; printf '\033]2;prog-title\007ready'; sleep 30",
    ];
    #[cfg(windows)]
    let titler = [
        "powershell.exe",
        "-NoProfile",
        "-Command",
        "Start-Sleep 1; [Console]::Out.Write([string][char]27 + ']2;prog-title' + [char]7 + 'ready'); Start-Sleep 30",
    ];
    let title = |t: &Tmxr, w: &str| {
        t.run(&["display-message", "-p", "-t", w, "#{pane_title}"])
            .trim()
            .to_owned()
    };
    // Off before the program starts, so its title cannot come first.
    t.run(&["set", "-g", "allow-set-title", "off"]);
    let mut new = vec!["new-window", "-d", "-t", "st:1"];
    new.extend(titler);
    t.run(&new);
    t.wait_run(
        &["capture-pane", "-p", "-t", "st:1"],
        "the program ran",
        |o| o.contains("ready"),
    );
    assert_ne!(title(&t, "st:1"), "prog-title", "off");
    // select-pane -T still titles it.
    t.run(&["select-pane", "-t", "st:1", "-T", "by-hand"]);
    assert_eq!(title(&t, "st:1"), "by-hand");
    // On, as by default, the program's title is taken.
    t.run(&["set", "-g", "allow-set-title", "on"]);
    let mut new = vec!["new-window", "-d", "-t", "st:2"];
    new.extend(titler);
    t.run(&new);
    t.wait_run(
        &["display-message", "-p", "-t", "st:2", "#{pane_title}"],
        "the program's title",
        |o| o.trim() == "prog-title",
    );
}

#[test]
fn split_window_i_shows_its_input_in_an_empty_pane() {
    let t = Tmxr::new("emptypane");
    t.run(&["new-session", "-d", "-s", "e"]);
    let out = t.output_with_stdin(&["split-window", "-I", "-t", "e"], b"first\nsecond\n");
    assert!(out.status.success(), "{out:?}");
    // Each newline starts a line, as in tmux's empty pane.
    t.wait_run(
        &["capture-pane", "-p", "-t", "e.1"],
        "the input shown",
        |o| o.starts_with("first\nsecond\n"),
    );
    // display-message -I shows more in it.
    let more = t.output_with_stdin(&["display-message", "-I", "-t", "e.1"], b"third\n");
    assert!(more.status.success(), "{more:?}");
    t.wait_run(
        &["capture-pane", "-p", "-t", "e.1"],
        "more input shown",
        |o| o.starts_with("first\nsecond\nthird\n"),
    );
    // Not in a pane running a program.
    let refused = t.output_with_stdin(&["display-message", "-I", "-t", "e.0"], b"x");
    assert!(!refused.status.success(), "{refused:?}");
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("pane is not empty"),
        "{refused:?}"
    );
    // No program: no pid, dead, and the cursor hidden, as tmux's.
    let vars = t.run(&[
        "display",
        "-p",
        "-t",
        "e.1",
        "#{pane_pid} #{pane_dead} #{cursor_flag}",
    ]);
    assert_eq!(vars.trim(), "0 1 0");
    // Typing goes nowhere.
    t.run(&["send-keys", "-t", "e.1", "typed", "Enter"]);
    std::thread::sleep(Duration::from_millis(200));
    let shown = t.run(&["capture-pane", "-p", "-t", "e.1"]);
    assert!(!shown.contains("typed"), "{shown}");
}

/// `#{pane_tty}` is the pane's terminal as the program in it sees it, as
/// tmux's; a ConPTY has no device name, so on Windows it is empty.
#[test]
fn pane_tty_is_the_terminal_the_program_sees() {
    let t = Tmxr::new("panetty");
    #[cfg(unix)]
    {
        t.run(&[
            "new-session",
            "-d",
            "-s",
            "pt",
            "echo TTY=$(tty); exec sleep 100",
        ]);
        let shown = t.wait_run(&["capture-pane", "-p", "-t", "pt"], "the tty", |o| {
            o.contains("TTY=/")
        });
        let tty = shown
            .lines()
            .find_map(|l| l.trim_end().strip_prefix("TTY="))
            .unwrap()
            .to_owned();
        let pane_tty = t.run(&["display-message", "-p", "-t", "pt", "#{pane_tty}"]);
        assert_eq!(pane_tty.trim(), tty);
    }
    #[cfg(windows)]
    {
        t.run(&["new-session", "-d", "-s", "pt"]);
        let pane_tty = t.run(&["display-message", "-p", "-t", "pt", "[#{pane_tty}]"]);
        assert_eq!(pane_tty.trim(), "[]");
    }
}
