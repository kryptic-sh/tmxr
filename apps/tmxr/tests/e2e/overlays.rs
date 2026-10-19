use super::*;

#[test]
fn session_picker_switches_by_enter_filter_and_jk() {
    let t = Tmxr::new("picker");
    t.run(&["new-session", "-d", "-s", "alpha"]);
    let s = t.attach(&["new", "-s", "bravo"]);
    s.wait_for("status line", |text| text.contains("bravo"));
    let attached = |ls: &str, name: &str| {
        ls.lines()
            .any(|l| l.starts_with(&format!("{name}:")) && l.contains("(attached)"))
    };

    // prefix s opens the picker on the previous session; Enter switches.
    s.send(PREFIX);
    s.send(b"s");
    s.wait_for("picker", |text| text.contains("sessions 2/2"));
    s.send(b"\r");
    let ls = t.wait_run(&["ls"], "alpha attached", |o| attached(o, "alpha"));
    assert!(!attached(&ls, "bravo"), "{ls}");

    // `i` starts typing, which filters: "char" leaves one row, Enter
    // switches to it.
    t.run(&["new-session", "-d", "-s", "charlie"]);
    s.send(PREFIX);
    s.send(b"s");
    s.wait_for("picker", |text| text.contains("sessions 3/3"));
    s.send(b"ichar");
    s.wait_for("filtered", |text| text.contains("sessions 1/3"));
    s.send(b"\r");
    t.wait_run(&["ls"], "charlie attached", |o| attached(o, "charlie"));

    // The picker opens in normal mode, where j and k move: charlie
    // (current), alpha (where the picker opens), then bravo.
    s.send(PREFIX);
    s.send(b"s");
    s.wait_for("picker", |text| text.contains("sessions 3/3"));
    s.send(b"j");
    std::thread::sleep(Duration::from_millis(200));
    s.send(b"\r");
    t.wait_run(&["ls"], "bravo attached", |o| attached(o, "bravo"));
    // Now bravo (current), charlie (where it opens), alpha: k comes back
    // from alpha to charlie.
    s.send(PREFIX);
    s.send(b"s");
    s.wait_for("picker", |text| text.contains("sessions 3/3"));
    s.send(b"jk");
    std::thread::sleep(Duration::from_millis(200));
    s.send(b"\r");
    t.wait_run(&["ls"], "charlie attached", |o| attached(o, "charlie"));
}

#[test]
fn display_panes_selects_by_number_until_it_times_out() {
    let t = Tmxr::new("displayp");
    let s = t.attach(&["new", "-s", "dp"]);
    s.wait_for("status line", |text| text.contains("dp"));
    t.run(&["split-window", "-h", "-t", "dp"]);
    let active = |t: &Tmxr| t.run(&["display-message", "-p", "-t", "dp", "#{pane_index}"]);
    t.wait_run(
        &["display-message", "-p", "-t", "dp", "#{pane_index}"],
        "new pane",
        |o| o.trim() == "1",
    );

    // prefix q numbers the panes; 0 picks the left one.
    s.send(PREFIX);
    s.send(b"q");
    std::thread::sleep(Duration::from_millis(200));
    s.send(b"0");
    t.wait_run(
        &["display-message", "-p", "-t", "dp", "#{pane_index}"],
        "pane 0",
        |o| o.trim() == "0",
    );

    // After display-panes-time the numbers are gone: a digit is just typed.
    s.send(PREFIX);
    s.send(b"q");
    std::thread::sleep(Duration::from_millis(1600));
    s.send(b"1");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(active(&t).trim(), "0");
}

#[test]
fn display_panes_runs_its_template_and_n_passes_keys_on() {
    let t = Tmxr::new("displayp2");
    let s = t.attach(&["new", "-s", "dq"]);
    s.wait_for("status line", |text| text.contains("dq"));
    t.run(&["split-window", "-h", "-t", "dq"]);
    let pane1 = t
        .run(&["display-message", "-p", "-t", "dq", "#{pane_id}"])
        .trim()
        .to_owned();
    t.run(&["select-pane", "-t", "dq.0"]);

    // -d 0 waits for a key however long; the key runs the template with the
    // pane's id for %%.
    t.run(&["bind", "x", "display-panes", "-d", "0", "set -g @picked %%"]);
    s.send(PREFIX);
    s.send(b"x");
    std::thread::sleep(Duration::from_millis(1600));
    s.send(b"1");
    t.wait_run(&["show", "-gv", "@picked"], "the template ran", |o| {
        o.trim() == pane1
    });

    // -N: the numbers show, and keys go on to the pane as typed.
    t.run(&["bind", "y", "display-panes", "-N", "-d", "5000"]);
    s.send(PREFIX);
    s.send(b"y");
    std::thread::sleep(Duration::from_millis(200));
    s.send(b"echo passthrough-7\r");
    t.wait_run(
        &["capture-pane", "-p", "-t", "dq.0"],
        "the command ran",
        |o| o.lines().any(|l| l.trim() == "passthrough-7"),
    );
}

#[test]
fn status_off_gives_its_row_to_the_window() {
    let t = Tmxr::new("statusoff");
    let s = t.attach(&["new", "-s", "zqx"]);
    s.wait_for("status line", |text| text.contains("zqx"));
    let window = || {
        t.run(&["display-message", "-p", "-t", "zqx", "#{window_height}"])
            .trim()
            .to_owned()
    };
    let client: u16 = t
        .run(&["list-clients", "-F", "#{client_height}"])
        .trim()
        .parse()
        .unwrap();
    assert_eq!(
        window(),
        (client - 1).to_string(),
        "the status line takes a row"
    );
    assert_eq!(t.run(&["show", "-gv", "status"]).trim(), "on");
    // Off: the window takes the whole client, and the line is gone.
    t.run(&["set", "-g", "status", "off"]);
    let all = client.to_string();
    t.wait_run(
        &["display-message", "-p", "-t", "zqx", "#{window_height}"],
        "the full height",
        |o| o.trim() == all,
    );
    s.wait_for("no status line", |text| !text.contains("zqx"));
    // A message still shows, over the window's last row, as tmux's.
    let name = t.run(&["list-clients", "-F", "#{client_name}"]);
    t.run(&["display-message", "-c", name.trim(), "qzmsg"]);
    s.wait_for_cells("the message on the last row", |s| {
        s.status_cells()
            .iter()
            .map(|(c, _, _)| c.as_str())
            .collect::<String>()
            .contains("qzmsg")
    });
    // On again: the row goes back to the line.
    t.run(&["set", "-g", "status", "on"]);
    let less = (client - 1).to_string();
    t.wait_run(
        &["display-message", "-p", "-t", "zqx", "#{window_height}"],
        "the status row back",
        |o| o.trim() == less,
    );
    s.wait_for("the status line back", |text| text.contains("zqx"));
}

#[test]
fn status_fg_and_bg_colour_the_status_line_over_its_style() {
    let t = Tmxr::new("statusfgbg");
    let s = t.attach(&["new", "-s", "fgb"]);
    s.wait_for("status line", |text| text.contains("fgb"));
    assert_eq!(t.run(&["show", "-gv", "status-bg"]).trim(), "default");
    assert_eq!(t.run(&["show", "-gv", "status-fg"]).trim(), "default");
    let coloured = |s: &Screen, fg: &str, bg: &str| {
        let cells = s.status_cells();
        cells.iter().any(|(_, f, _)| f == fg) && cells.iter().any(|(_, _, b)| b == bg)
    };
    assert!(!coloured(&s, "Idx(2)", "Idx(1)"), "{:?}", s.status_cells());
    // Not default: over status-style's colours, where the line's own style
    // shows (the theme's window entries set theirs).
    t.run(&["set", "-g", "status-bg", "red"]);
    t.run(&["set", "-g", "status-fg", "green"]);
    s.wait_for_cells("a green on red status line", |s| {
        coloured(s, "Idx(2)", "Idx(1)")
    });
    // Back to default: status-style's again.
    t.run(&["set", "-g", "status-bg", "default"]);
    s.wait_for_cells("the style's background back", |s| {
        s.status_cells().iter().all(|(_, _, bg)| bg != "Idx(1)")
    });
}

#[test]
fn two_panes_colour_half_their_border_with_colour_indicators() {
    let t = Tmxr::new("halves");
    let s = t.attach(&["new", "-s", "hv"]);
    s.wait_for("status line", |text| text.contains("hv"));
    t.run(&["split-window", "-h", "-t", "hv"]);
    t.run(&["set", "-g", "pane-border-style", "fg=red"]);
    t.run(&["set", "-g", "pane-active-border-style", "fg=green"]);
    // The border column between the panes; the new, right one is active.
    let col = usize::from(COLS / 2);
    let fg = |s: &Screen, row: u16| s.row_cells(row)[col].1.clone();
    // tmux's default, colour: only the half nearer the active pane.
    s.wait_for_cells("the lower half green", |s| {
        fg(s, 3) == "Idx(1)" && fg(s, ROWS - 4) == "Idx(2)"
    });
    // off: the active pane's whole border.
    t.run(&["set", "-g", "pane-border-indicators", "off"]);
    s.wait_for_cells("all of it green", |s| {
        fg(s, 3) == "Idx(2)" && fg(s, ROWS - 4) == "Idx(2)"
    });
}

#[test]
fn window_style_colours_panes_and_window_active_style_the_active_one() {
    let t = Tmxr::new("winstyle");
    let s = t.attach(&["new", "-s", "ws"]);
    s.wait_for("status line", |text| text.contains("ws"));
    t.run(&["split-window", "-h", "-t", "ws"]);
    t.run(&["set", "-g", "window-style", "bg=red"]);
    t.run(&["set", "-g", "window-active-style", "bg=green"]);
    // The new pane, on the right, is active.
    let bg = |s: &Screen, col: usize| s.row_cells(5)[col].2.clone();
    s.wait_for_cells("the panes' colours", |s| {
        bg(s, 2) == "Idx(1)" && bg(s, COLS as usize - 3) == "Idx(2)"
    });
    // Unset, the active pane takes window-style's too.
    t.run(&["set", "-g", "window-active-style", "default"]);
    s.wait_for_cells("both red", |s| {
        bg(s, 2) == "Idx(1)" && bg(s, COLS as usize - 3) == "Idx(1)"
    });
}

// Unix only: ConPTY asks its own terminal for focus reports when it starts
// and keeps a `?1004l` to itself (seen on Windows 11), so a Windows client's
// requests never show.
#[cfg(unix)]
#[test]
fn focus_events_asks_the_terminal_for_focus_reports() {
    let t = Tmxr::new("focusev");
    let s = t.attach(&["new", "-s", "fev"]);
    s.wait_for("status line", |text| text.contains("fev"));
    let asked = |what: &str, seq: &str| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !s.raw_text().contains(seq) {
            assert!(Instant::now() < deadline, "{what}: no {seq:?}");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    // On by default (tmux-sensible's): the client's terminal is asked.
    assert_eq!(t.run(&["show", "-sv", "focus-events"]).trim(), "on");
    asked("on", "\x1b[?1004h");
    // Off: told to stop, as tmux asks for none.
    t.run(&["set", "-s", "focus-events", "off"]);
    asked("off", "\x1b[?1004l");
}

#[test]
fn command_prompt_asks_each_prompt_and_fills_the_template() {
    let t = Tmxr::new("cprompts");
    let s = t.attach(&["new", "-s", "cp"]);
    s.wait_for("status line", |text| text.contains("cp"));
    let r = || t.run(&["show", "-gv", "@r"]).trim().to_owned();
    // Two prompts, their inputs given; Enter answers each in turn.
    t.run(&[
        "command-prompt",
        "-p",
        "first,second",
        "-I",
        "x,y",
        "set -g @r '%1|%2'",
    ]);
    s.wait_for("the first prompt", |text| text.contains("first x"));
    s.send(b"\r");
    s.wait_for("the second prompt", |text| text.contains("second y"));
    s.send(b"\r");
    t.wait_run(&["show", "-gv", "@r"], "both answers", |o| {
        o.trim() == "x|y"
    });
    // No -p: the template is the prompt, in brackets.
    t.run(&["command-prompt", "set -g @r '%%'"]);
    s.wait_for("the template prompt", |text| {
        text.contains("(set -g @r '%%')")
    });
    s.send(b"z\r");
    t.wait_run(&["show", "-gv", "@r"], "the answer", |o| o.trim() == "z");
    // -l: one prompt, its commas kept.
    t.run(&[
        "command-prompt",
        "-l",
        "-p",
        "a,b",
        "-I",
        "x,y",
        "set -g @r '%1'",
    ]);
    s.wait_for("the one prompt", |text| text.contains("a,b x,y"));
    s.send(b"\r");
    t.wait_run(&["show", "-gv", "@r"], "the literal answer", |o| {
        o.trim() == "x,y"
    });
    assert_eq!(r(), "x,y");
    // -F: the template is expanded as a format first.
    t.run(&[
        "command-prompt",
        "-F",
        "-p",
        "f",
        "set -g @r '#{session_name}-%%'",
    ]);
    s.wait_for("the -F prompt", |text| text.contains("f "));
    s.send(b"q\r");
    t.wait_run(&["show", "-gv", "@r"], "the expanded template", |o| {
        o.trim() == "cp-q"
    });
}

#[test]
fn choose_tree_t_starts_from_its_target() {
    let t = Tmxr::new("choosett");
    let s = t.attach(&["new", "-s", "aa"]);
    s.wait_for("status line", |text| text.contains("aa"));
    t.run(&["new-session", "-d", "-s", "bb"]);
    t.run(&["new-window", "-d", "-t", "bb:1", "-n", "second"]);
    // The client's session is current without -t; -t's with it.
    t.run(&["choose-tree", "-t", "bb"]);
    s.wait_for("bb current", |text| text.contains("* bb:"));
    s.send(b"\x1b");
    s.wait_for("picker closed", |text| !text.contains("* bb:"));
    t.run(&["choose-tree", "-w", "-t", "bb:1"]);
    s.wait_for("bb:1 current", |text| text.contains("* bb:1 second"));
    s.send(b"\x1b");
    t.run(&["choose-tree"]);
    s.wait_for("aa current", |text| text.contains("* aa:"));
}

#[test]
fn client_active_runs_when_another_client_takes_the_window() {
    let t = Tmxr::new("cactive");
    t.run(&["new-session", "-d", "-s", "ca", "-x", "80", "-y", "24"]);
    t.run(&[
        "set-hook",
        "-g",
        "client-active",
        "set -gaF @act '#{client_name} '",
    ]);
    let act = || t.run(&["show", "-gv", "@act"]).trim().to_owned();
    let a = t.attach(&["attach", "-t", "ca"]);
    a.wait_for("first client", |text| text.contains("ca"));
    let b = t.attach(&["attach", "-t", "ca"]);
    b.wait_for("second client", |text| text.contains("ca"));
    let names = t.run(&["list-clients", "-F", "#{client_name}"]);
    let names: Vec<&str> = names.lines().collect();
    let (na, nb) = (names[0], names[1]);
    // Attaching makes a client its window's latest without the hook, as
    // tmux 3.6 showed; a key from the other client runs it.
    assert_eq!(act(), "");
    a.send(b"a");
    t.wait_run(&["show", "-gv", "@act"], "the first", |o| o.trim() == na);
    b.send(b"b");
    let both = format!("{na} {nb}");
    t.wait_run(&["show", "-gv", "@act"], "the second", |o| o.trim() == both);
    // Still the latest: no hook.
    b.send(b"c");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(act(), both);
    // The latest going hands the window to the other.
    t.run(&["detach-client", "-t", nb]);
    let handed = format!("{na} {nb} {na}");
    t.wait_run(&["show", "-gv", "@act"], "the hand-back", |o| {
        o.trim() == handed
    });
}

#[test]
fn display_message_n_stays_up_through_keys() {
    let t = Tmxr::new("dmsgn");
    let s = t.attach(&["new", "-s", "dm"]);
    s.wait_for("status line", |text| text.contains("dm"));
    // From a shell the message prints; -c shows it on the client.
    let client = t
        .run(&["list-clients", "-F", "#{client_name}"])
        .trim()
        .to_owned();
    // -N: a key leaves the message up until its time is out.
    t.run(&[
        "display-message",
        "-c",
        &client,
        "-N",
        "-d",
        "20000",
        "held-message",
    ]);
    s.wait_for("the message", |text| text.contains("held-message"));
    s.send(b"x");
    std::thread::sleep(Duration::from_millis(500));
    assert!(s.text().contains("held-message"), "{}", s.text());
    // Without it, the next key takes it down.
    t.run(&[
        "display-message",
        "-c",
        &client,
        "-d",
        "20000",
        "brief-message",
    ]);
    s.wait_for("the second message", |text| text.contains("brief-message"));
    s.send(b"y");
    s.wait_for("the message gone", |text| !text.contains("brief-message"));
}

#[test]
fn send_keys_k_types_into_the_client_and_its_binds() {
    let t = Tmxr::new("sendk");
    let s = t.attach(&["new", "-s", "sk"]);
    s.wait_for("status line", |text| text.contains("sk"));
    let opt = |name: &str| t.run(&["show", "-gv", name]).trim().to_owned();
    // A root bind, and a prefix bind after the prefix, as typed keys reach.
    t.run(&["bind", "-n", "F9", "set -g @root yes"]);
    t.run(&["bind", "z", "set -g @prefixed yes"]);
    t.run(&["send-keys", "-K", "F9"]);
    t.wait_run(&["show", "-gv", "@root"], "the root bind", |o| {
        o.trim() == "yes"
    });
    t.run(&["send-keys", "-K", "C-b", "z"]);
    t.wait_run(&["show", "-gv", "@prefixed"], "the prefix bind", |o| {
        o.trim() == "yes"
    });
    assert_eq!(opt("@root"), "yes");
    assert!(
        !t.output(&["send-keys", "-K", "no-such-key"])
            .status
            .success()
    );
}

#[test]
fn choose_buffer_pastes_and_find_window_switches() {
    let t = Tmxr::new("chooseb");
    let s = t.attach(&["new", "-s", "cb", "-n", "logs"]);
    s.wait_for("status line", |text| text.contains("logs"));
    t.run(&["new-window", "-d", "-t", "cb", "-n", "editor"]);
    t.run(&["set-buffer", "-b", "alpha", "first-buffer-text"]);
    t.run(&["set-buffer", "-b", "beta", "second-buffer-text"]);

    // prefix = lists the buffers; typing (after `i`) filters, Enter pastes
    // into the pane.
    s.send(PREFIX);
    s.send(b"=");
    s.wait_for("buffer picker", |text| text.contains("buffers 2/2"));
    s.send(b"ialpha");
    s.wait_for("filtered", |text| text.contains("buffers 1/2"));
    s.send(b"\r");
    s.wait_for("pasted buffer", |text| {
        !text.contains("buffers 1/2") && text.contains("first-buffer-text")
    });
    assert!(!s.text().contains("second-buffer-text"));

    // prefix f asks for text and lists the windows it matches.
    s.send(PREFIX);
    s.send(b"f");
    s.wait_for("find prompt", |text| text.contains("(find-window)"));
    s.send(b"editor\r");
    s.wait_for("the matching window", |text| text.contains("windows 1/1"));
    s.send(b"\r");
    t.wait_run(
        &["display-message", "-p", "-t", "cb", "#{window_name}"],
        "editor selected",
        |o| o.trim() == "editor",
    );
}

#[test]
fn session_picker_kills_renames_and_creates_sessions() {
    let t = Tmxr::new("pickact");
    t.run(&["new-session", "-d", "-s", "first"]);
    let s = t.attach(&["new", "-s", "home"]);
    s.wait_for("status line", |text| text.contains("home"));
    let names = |t: &Tmxr| -> Vec<String> {
        t.run(&["ls"])
            .lines()
            .filter_map(|l| l.split(':').next())
            .map(str::to_owned)
            .collect()
    };
    let open_picker = |s: &Screen| {
        s.send(PREFIX);
        s.send(b"s");
        s.wait_for("picker", |text| text.contains("sessions "));
    };

    // C-r renames the highlighted session (the previous one, "first").
    open_picker(&s);
    s.send(b"\x12");
    s.wait_for("rename prompt", |text| text.contains("(rename-session)"));
    s.send(b"\x15renamed\r");
    t.wait_run(&["ls"], "renamed session", |o| o.contains("renamed:"));

    // C-x asks, then kills it.
    open_picker(&s);
    s.send(b"\x18");
    s.wait_for("confirm", |text| text.contains("kill-session renamed?"));
    s.send(b"y");
    t.wait_run(&["ls"], "session killed", |o| !o.contains("renamed:"));

    // Enter on a name nothing matches creates it and switches to it.
    open_picker(&s);
    s.send(b"ibrand-new\r");
    t.wait_run(&["ls"], "new session attached", |o| {
        o.lines()
            .any(|l| l.starts_with("brand-new:") && l.contains("(attached)"))
    });
    let mut all = names(&t);
    all.sort();
    assert_eq!(all, ["brand-new", "home"]);
}

#[test]
fn session_picker_previews_the_highlighted_session() {
    let t = Tmxr::new("preview");
    t.run(&["new-session", "-d", "-s", "alpha"]);
    t.wait_prompt("alpha");
    t.run(&["send-keys", "-t", "alpha", "echo preview-marker", "Enter"]);
    t.wait_run(
        &["capture-pane", "-p", "-t", "alpha"],
        "marker in alpha",
        |o| o.matches("preview-marker").count() >= 2,
    );
    let s = t.attach(&["new", "-s", "bravo"]);
    s.wait_for("status line", |text| text.contains("bravo"));
    assert!(!s.text().contains("preview-marker"));

    // The picker opens on alpha: its pane shows under the list.
    s.send(PREFIX);
    s.send(b"s");
    s.wait_for("alpha previewed", |text| {
        text.contains("sessions 2/2") && text.contains("preview-marker")
    });
    // Up moves to bravo, which never printed the marker.
    s.send(b"\x1b[A");
    s.wait_for("bravo previewed", |text| {
        text.contains("sessions 2/2") && !text.contains("preview-marker")
    });
}

#[test]
fn command_prompt_from_a_script_opens_on_the_client() {
    let t = Tmxr::new("cmdprompt");
    let s = t.attach(&["new", "-s", "cp"]);
    s.wait_for("status line", |text| text.contains("cp"));
    // From a command client: the attached client gets the prompt, at once.
    t.run(&["command-prompt", "-p", "name?", "rename-window '%%'"]);
    s.wait_for("the prompt", |text| text.contains("name? "));
    s.send(b"typed\r");
    t.wait_run(
        &["display-message", "-p", "-t", "cp", "#W"],
        "the rename",
        |o| o.trim() == "typed",
    );
    // -t names the client.
    let id = t.run(&["list-clients", "-F", "#{client_name}"]);
    t.run(&[
        "command-prompt",
        "-t",
        id.trim(),
        "-p",
        "again?",
        "rename-window '%%'",
    ]);
    s.wait_for("the -t prompt", |text| text.contains("again? "));
    assert!(
        !t.output(&["command-prompt", "-t", "999", "x"])
            .status
            .success()
    );
}

#[test]
fn confirm_before_c_and_y_choose_what_confirms() {
    let t = Tmxr::new("confirmc");
    let s = t.attach(&["new", "-s", "cf"]);
    s.wait_for("status line", |text| text.contains("cf"));
    let windows = |t: &Tmxr| t.run(&["list-windows", "-t", "cf"]).lines().count();
    // -c x: the prompt says so, `y` does nothing and `x` confirms.
    t.run(&["confirm-before", "-c", "x", "new-window -d"]);
    s.wait_for("the prompt", |text| {
        text.contains("Confirm 'new-window'? (x/n)")
    });
    s.send(b"y");
    s.wait_for("the prompt gone", |text| !text.contains("Confirm"));
    assert_eq!(windows(&t), 1);
    t.run(&["confirm-before", "-c", "x", "new-window -d"]);
    s.wait_for("the prompt again", |text| text.contains("(x/n)"));
    s.send(b"x");
    t.wait_run(&["list-windows", "-t", "cf"], "a second window", |o| {
        o.lines().count() == 2
    });
    // -y: Enter confirms.
    t.run(&["confirm-before", "-y", "new-window -d"]);
    s.wait_for("the -y prompt", |text| text.contains("(y/n)"));
    s.send(b"\r");
    t.wait_run(&["list-windows", "-t", "cf"], "a third window", |o| {
        o.lines().count() == 3
    });
    // A confirm key is one printable character.
    assert!(
        !t.output(&["confirm-before", "-c", "xx", "new-window"])
            .status
            .success()
    );
}

#[test]
fn choose_templates_run_with_the_chosen_target() {
    let t = Tmxr::new("choosetpl");
    let s = t.attach(&["new", "-s", "ct"]);
    s.wait_for("status line", |text| text.contains("ct"));
    t.run(&["set-buffer", "-b", "mybuf", "hello"]);
    let option = |name: &str| t.run(&["show", "-gv", name]).trim().to_owned();
    // A buffer's target is its name.
    t.run(&["choose-buffer", "set -g @b '%%'"]);
    s.wait_for("the buffer picker", |text| text.contains("mybuf"));
    s.send(b"\r");
    t.wait_run(&["show", "-gv", "@b"], "the template run", |o| {
        o.trim() == "mybuf"
    });
    // A session's, as tmux's window_tree_get_target: `=name:`.
    t.run(&["choose-tree", "set -g @t '%%'"]);
    s.wait_for("the session picker", |text| text.contains("sessions"));
    s.send(b"\r");
    t.wait_run(&["show", "-gv", "@t"], "the template run", |o| {
        o.trim() == "=ct:"
    });
    // A window's: `=name:index.`.
    t.run(&["choose-tree", "-w", "set -g @w '%%'"]);
    s.wait_for("the window picker", |text| text.contains("windows"));
    s.send(b"\r");
    t.wait_run(&["show", "-gv", "@w"], "the template run", |o| {
        o.trim() == "=ct:0."
    });
    assert_eq!(option("@b"), "mybuf");
}

#[test]
fn prompt_history_is_kept_recalled_and_cleared() {
    let t = Tmxr::new("phist");
    let s = t.attach(&["new", "-s", "ph"]);
    s.wait_for("status line", |text| text.contains("ph"));
    let name = |t: &Tmxr| {
        t.run(&["display-message", "-p", "-t", "ph", "#W"])
            .trim()
            .to_owned()
    };
    for entry in ["rename-window first", "rename-window second"] {
        s.send(PREFIX);
        s.send(b":");
        s.wait_for("command prompt", |text| {
            text.lines().last().is_some_and(|l| l.starts_with(':'))
        });
        s.send(format!("{entry}\r").as_bytes());
        let want = entry.rsplit(' ').next().unwrap();
        t.wait_run(
            &["display-message", "-p", "-t", "ph", "#W"],
            "renamed",
            |o| o.trim() == want,
        );
    }
    let shown = t.run(&["showphist", "-T", "command"]);
    assert!(
        shown.contains("1: rename-window first") && shown.contains("2: rename-window second"),
        "{shown}"
    );
    // Up twice recalls the older entry; Enter runs it again.
    s.send(PREFIX);
    s.send(b":");
    s.wait_for("command prompt", |text| {
        text.lines().last().is_some_and(|l| l.starts_with(':'))
    });
    s.send(b"\x1b[A");
    s.send(b"\x1b[A");
    s.wait_for("recalled", |text| text.contains(":rename-window first"));
    s.send(b"\r");
    t.wait_run(
        &["display-message", "-p", "-t", "ph", "#W"],
        "renamed back",
        |o| o.trim() == "first",
    );
    assert_eq!(name(&t), "first");
    t.run(&["clearphist"]);
    assert_eq!(t.run(&["showphist"]).trim(), "");
}

#[test]
fn history_file_is_read_at_start_and_written_at_exit() {
    let t = Tmxr::new("histfile");
    let file = t.dir.path().join("history");
    // tmux's format: `type:entry`; a type it does not know is a command.
    std::fs::write(
        &file,
        "search:foo\ncommand:display -p hi\nno type here\nwindow-target:2\n",
    )
    .unwrap();
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        config.replace(
            "[resurrect]",
            &format!("history-file = {file:?}\n[resurrect]"),
        ),
    )
    .unwrap();
    t.run(&["new-session", "-d", "-s", "hf"]);
    let shown = t.run(&["show-prompt-history", "-T", "command"]);
    assert!(
        shown.contains("1: display -p hi") && shown.contains("2: no type here"),
        "{shown}"
    );
    assert!(
        t.run(&["show-prompt-history", "-T", "search"])
            .contains("1: foo")
    );
    t.run(&["clear-prompt-history", "-T", "window-target"]);
    // At exit, each type in tmux's order.
    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);
    let deadline = Instant::now() + Duration::from_secs(10);
    let want = "command:display -p hi\ncommand:no type here\nsearch:foo\n";
    while std::fs::read_to_string(&file).unwrap() != want {
        assert!(
            Instant::now() < deadline,
            "{:?}",
            std::fs::read_to_string(&file)
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn display_message_d_sets_how_long_and_c_where() {
    let t = Tmxr::new("dispd");
    let s = t.attach(&["new", "-s", "dd"]);
    s.wait_for("the status line", |text| text.contains("dd"));
    let client = t
        .run(&["list-clients", "-F", "#{client_name}"])
        .trim()
        .to_owned();
    t.run(&["set", "-g", "display-time", "200"]);
    // -d 0 keeps the message until a key, past display-time.
    t.run(&["display-message", "-c", &client, "-d", "0", "stays-put"]);
    s.wait_for("the message", |text| text.contains("stays-put"));
    std::thread::sleep(Duration::from_millis(800));
    assert!(s.text().contains("stays-put"), "{}", s.text());
    s.send(b"x");
    s.wait_for("the message gone", |text| !text.contains("stays-put"));
    // -d N lasts N milliseconds, whatever display-time says.
    t.run(&["set", "-g", "display-time", "60000"]);
    t.run(&["display-message", "-c", &client, "-d", "200", "goes-soon"]);
    s.wait_for("the message", |text| text.contains("goes-soon"));
    s.wait_for("it goes", |text| !text.contains("goes-soon"));
    // display-time 0 also lasts until a key, as tmux's.
    t.run(&["set", "-g", "display-time", "0"]);
    t.run(&["display-message", "-c", &client, "zero-time"]);
    s.wait_for("the message", |text| text.contains("zero-time"));
    std::thread::sleep(Duration::from_millis(500));
    assert!(s.text().contains("zero-time"), "{}", s.text());
    assert!(
        !t.output(&["display-message", "-c", "nosuch", "x"])
            .status
            .success()
    );
}

#[test]
fn display_menu_runs_the_item_picked() {
    let t = Tmxr::new("menu");
    let s = t.attach(&["new", "-s", "mn"]);
    s.wait_for("status line", |text| text.contains("mn"));
    let open = |t: &Tmxr| {
        t.run(&[
            "display-menu",
            "-t",
            "mn",
            "-T",
            "Tools",
            "Rename one",
            "o",
            "rename-window one",
            "",
            "Rename two",
            "w",
            "rename-window two",
        ]);
    };
    let name = |want: &'static str| move |o: &str| o.trim() == want;
    let opened = Instant::now();
    open(&t);
    s.wait_for("the menu", |text| {
        text.contains("Tools") && text.contains("Rename two") && text.contains("(w)")
    });
    // Shown at once, not at the next periodic repaint (up to 5 s later).
    assert!(
        opened.elapsed() < Duration::from_secs(3),
        "the menu took {:?} to show",
        opened.elapsed()
    );
    // An item's key runs it.
    s.send(b"w");
    t.wait_run(
        &["display-message", "-p", "-t", "mn", "#W"],
        "renamed by key",
        name("two"),
    );
    s.wait_for("the menu closed", |text| !text.contains("Rename two"));
    // Down skips the separator; Enter runs the selection.
    open(&t);
    s.wait_for("the menu again", |text| text.contains("Rename one"));
    s.send(b"\x1b[B");
    s.send(b"\r");
    t.wait_run(
        &["display-message", "-p", "-t", "mn", "#W"],
        "renamed by Enter",
        name("two"),
    );
    open(&t);
    s.wait_for("the menu a third time", |text| text.contains("Rename one"));
    s.send(b"\r");
    t.wait_run(
        &["display-message", "-p", "-t", "mn", "#W"],
        "the first item",
        name("one"),
    );
}

#[test]
fn display_popup_runs_a_command_over_the_panes() {
    let t = Tmxr::new("popup");
    let s = t.attach(&["new", "-s", "pp"]);
    s.wait_for("status line", |text| text.contains("pp"));
    let read_line = if cfg!(windows) {
        "echo POPUP-UP& set /p x="
    } else {
        "echo POPUP-UP; read x"
    };
    // -E: the keys go to the popup's command, which closes it by exiting.
    t.run(&[
        "display-popup",
        "-E",
        "-w",
        "40",
        "-h",
        "8",
        "-T",
        "Pop",
        read_line,
    ]);
    s.wait_for("the popup", |text| {
        text.contains("POPUP-UP") && text.contains("Pop")
    });
    s.send(b"typed\r");
    s.wait_for("the popup closed", |text| !text.contains("POPUP-UP"));
    let pane = t.run(&["capture-pane", "-p", "-t", "pp"]);
    assert!(
        !pane.contains("typed"),
        "the pane got the popup's keys:\n{pane}"
    );

    // Without -E it stays after its command, until Escape (or C-c), as
    // tmux's; other keys only with -k.
    t.run(&["display-popup", "echo DONE-HERE"]);
    s.wait_for("the finished popup", |text| text.contains("DONE-HERE"));
    std::thread::sleep(Duration::from_millis(300));
    assert!(s.text().contains("DONE-HERE"), "closed by itself");
    s.send(b"q");
    std::thread::sleep(Duration::from_millis(300));
    assert!(s.text().contains("DONE-HERE"), "closed by q without -k");
    s.send(b"\x1b");
    s.wait_for("closed by Escape", |text| !text.contains("DONE-HERE"));
    t.run(&["display-popup", "-k", "echo DONE-ANYKEY"]);
    s.wait_for("the -k popup", |text| text.contains("DONE-ANYKEY"));
    std::thread::sleep(Duration::from_millis(300));
    s.send(b"q");
    s.wait_for("closed by any key", |text| !text.contains("DONE-ANYKEY"));

    // -C closes a running popup and ends its command.
    let marker = t.dir.path().join("late-marker");
    let late = if cfg!(windows) {
        format!(
            "echo RUNNING& ping -n 3 127.0.0.1 >nul& echo x> {}",
            marker.display()
        )
    } else {
        format!("echo RUNNING; sleep 2; touch '{}'", marker.display())
    };
    t.run(&["display-popup", &late]);
    s.wait_for("the running popup", |text| text.contains("RUNNING"));
    t.run(&["display-popup", "-C"]);
    s.wait_for("closed by -C", |text| !text.contains("RUNNING"));
    std::thread::sleep(Duration::from_secs(4));
    assert!(!marker.exists(), "the popup's command outlived it");
}

#[test]
fn customize_mode_t_lists_the_targets_own_options() {
    let t = Tmxr::new("customizet");
    let s = t.attach(&["new", "-s", "ct"]);
    s.wait_for("status line", |text| text.contains("ct"));
    t.run(&["set-option", "-t", "ct", "status-left", "local-left"]);
    let sid = t
        .run(&["display-message", "-p", "-t", "ct", "#{session_id}"])
        .trim()
        .to_owned();
    // -t: the session's own setting, to be set again at its scope.
    t.run(&["customize-mode", "-t", "ct", "-f", "local-left"]);
    let line = format!("option -t {sid} status-left local-left");
    s.wait_for("the session's option", |text| text.contains(&line));
    s.send(b"\r");
    let set = format!(":set-option -t {sid} status-left local-left");
    s.wait_for("its set-option", |text| text.contains(&set));
    s.send(b"\x1b");
    // Flags tmux's customize-mode has no use for here are refused.
    assert!(!t.output(&["customize-mode", "-a"]).status.success());
    assert!(
        !t.output(&["customize-mode", "-F", "#{name}"])
            .status
            .success()
    );
}

#[test]
fn customize_mode_edits_an_option_through_the_prompt() {
    let t = Tmxr::new("customize");
    let s = t.attach(&["new", "-s", "cu"]);
    s.wait_for("status line", |text| text.contains("cu"));
    t.run(&["customize-mode", "-f", "mode-keys"]);
    s.wait_for("the option", |text| text.contains("option mode-keys vi"));
    s.send(b"\r");
    s.wait_for("the prompt", |text| {
        text.contains(":set-option -g mode-keys vi")
    });
    // Replace "vi" with "emacs" and run it.
    s.send(b"\x7f\x7femacs\r");
    t.wait_run(
        &["show-options", "-gv", "mode-keys"],
        "the edited option",
        |o| o.trim() == "emacs",
    );

    // A bind comes back as its bind-key line, formats unexpanded.
    t.run(&["customize-mode", "-f", "pane_current_path"]);
    s.wait_for("a bind", |text| text.contains("bind-key"));
    s.send(b"\r");
    s.wait_for("the bind in the prompt", |text| {
        text.lines()
            .any(|l| l.starts_with(":bind-key") && l.contains("#{pane_current_path}"))
    });
    s.send(b"\x1b");
    s.wait_for("the prompt closed", |text| !text.contains(":bind-key"));
    // prefix C opens it, as tmux's.
    s.send(PREFIX);
    s.send(b"C");
    s.wait_for("customize mode", |text| {
        text.contains("option escape-time 0")
    });
}

#[test]
fn find_window_matches_names_and_pane_contents() {
    let t = Tmxr::new("findw");
    let s = t.attach(&["new", "-s", "fw", "-n", "alpha"]);
    s.wait_for("status line", |text| text.contains("fw"));
    t.run(&["new-window", "-d", "-t", "fw", "-n", "beta"]);
    t.wait_prompt("fw:beta");
    t.run(&["send-keys", "-t", "fw:beta", "echo needle-in-pane", "Enter"]);
    t.wait_run(
        &["capture-pane", "-p", "-t", "fw:beta"],
        "the output",
        |o| o.lines().any(|l| l.trim() == "needle-in-pane"),
    );
    let found = |t: &Tmxr, s: &Screen, args: &[&str], want: &'static str| {
        let mut cmd = vec!["find-window"];
        cmd.extend_from_slice(args);
        // The prompt's own command line must not match: run from outside.
        t.run(&["send-keys", "-t", "fw:alpha", "C-l"]);
        let out = t.output(&cmd);
        assert!(out.status.success(), "{cmd:?}: {out:?}");
        s.wait_for(want, |text| text.contains(want));
        s.send(b"\x1b");
        s.wait_for("the picker closed", |text| !text.contains("windows "));
    };
    found(&t, &s, &["needle-in"], "windows 1/1");
    found(&t, &s, &["-N", "needle-in"], "windows 0/0");
    found(&t, &s, &["ALPHA"], "windows 0/0");
    found(&t, &s, &["-i", "ALPHA"], "windows 1/1");
    found(&t, &s, &["-r", "need.e-in-p"], "windows 1/1");
    assert!(!t.output(&["find-window", "-r", "("]).status.success());
}

#[test]
fn display_popup_stays_inside_a_resized_client() {
    let t = Tmxr::new("popupfit");
    let s = t.attach(&["new", "-s", "pf"]);
    s.wait_for("status line", |text| text.contains("pf"));
    // Kept running through the resize (cmd's pause would end on it).
    let show = if cfg!(windows) {
        "echo POPUP-READY& ping -n 30 127.0.0.1 >NUL"
    } else {
        "echo POPUP-READY; sleep 30"
    };
    // A wide box at the right edge.
    t.run(&[
        "display-popup",
        "-E",
        "-x",
        "40",
        "-w",
        "56",
        "-h",
        "8",
        show,
    ]);
    s.wait_for("the popup", |text| text.contains("POPUP-READY"));
    // The client shrinks to less than the popup's width.
    s.resize(20, 50);
    // Row by row: full-width rows would run together in the screen's text.
    let row_text = |s: &Screen, row: u16| -> String {
        s.row_cells(row).into_iter().map(|(c, _, _)| c).collect()
    };
    let popup_row = |s: &Screen| (0..20).find(|&r| row_text(s, r).contains("POPUP-READY"));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        // The box moved in to the left edge and fits the new width.
        if let Some(r) = popup_row(&s)
            && row_text(&s, r).starts_with("│POPUP-READY")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "popup not moved in:
{}",
            s.text()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn menu_and_popup_take_tmux_style_flags() {
    let t = Tmxr::new("looks");
    let s = t.attach(&["new", "-s", "lk"]);
    s.wait_for("status line", |text| text.contains("lk"));
    let all_cells = |s: &Screen| -> Vec<(String, String, String)> {
        (0..ROWS).flat_map(|r| s.row_cells(r)).collect()
    };
    // -b double, -S the border's colour, -s the items', -H the selected one.
    t.run(&[
        "display-menu",
        "-b",
        "double",
        "-S",
        "fg=green",
        "-s",
        "bg=red",
        "-H",
        "bg=blue",
        "-T",
        "Look",
        "First",
        "f",
        "rename-window first",
        "Second",
        "s",
        "rename-window second",
    ]);
    s.wait_for("the menu", |text| text.contains("Second"));
    let cells = all_cells(&s);
    let corner = cells
        .iter()
        .find(|(c, _, _)| c == "╔")
        .expect("a double border");
    assert_eq!(corner.1, "Idx(2)", "border fg: {corner:?}");
    // An item's first letter, on the row that holds its name.
    let item = |name: &str| -> (String, String, String) {
        (0..ROWS)
            .find_map(|r| {
                let row = s.row_cells(r);
                let text: String = row.iter().map(|(c, _, _)| c.as_str()).collect();
                text.find(name)?;
                row.into_iter()
                    .find(|(c, _, _)| name.starts_with(c.as_str()) && !c.is_empty())
            })
            .unwrap_or_else(|| panic!("{name} not on the screen"))
    };
    let second = item("Second");
    assert_eq!(second.2, "Idx(1)", "item bg: {second:?}");
    let first = item("First");
    assert_eq!(first.2, "Idx(4)", "selected bg: {first:?}");
    s.send(b"\x1b");
    s.wait_for("the menu closed", |text| !text.contains("Second"));

    // A popup's -b rounded, and -b none for no border.
    let hold = if cfg!(windows) {
        "echo IN-THE-BOX& ping -n 30 127.0.0.1 >NUL"
    } else {
        "echo IN-THE-BOX; sleep 30"
    };
    t.run(&["display-popup", "-b", "rounded", hold]);
    s.wait_for("the rounded popup", |text| {
        text.contains("IN-THE-BOX") && text.contains('╭')
    });
    t.run(&["display-popup", "-C"]);
    s.wait_for("closed", |text| !text.contains("IN-THE-BOX"));
    t.run(&["display-popup", "-b", "none", hold]);
    s.wait_for("the bare popup", |text| text.contains("IN-THE-BOX"));
    assert!(
        !s.text().contains(['╭', '┌', '│']),
        "a border was drawn:\n{}",
        s.text()
    );
    t.run(&["display-popup", "-C"]);
    // -b simple: tmux's ASCII lines, the title on the top one.
    t.run(&["display-popup", "-b", "simple", "-T", "Pop", hold]);
    s.wait_for("the simple popup", |text| {
        text.contains("IN-THE-BOX") && text.contains("+-Pop---")
    });
    t.run(&["display-popup", "-C"]);
    let bad = t.output(&["display-popup", "-b", "bogus", "echo"]);
    assert!(!bad.status.success());
    let err = String::from_utf8_lossy(&bad.stderr);
    assert!(
        err.contains("popup-border-lines unknown value: bogus"),
        "{err}"
    );
}

#[test]
fn menu_and_popup_options_set_their_look() {
    let t = Tmxr::new("boxopts");
    let s = t.attach(&["new", "-s", "bo"]);
    s.wait_for("status line", |text| text.contains("bo"));
    t.run(&["set-option", "-g", "menu-border-lines", "simple"]);
    t.run(&["set-option", "-g", "menu-border-style", "fg=green"]);
    t.run(&["set-option", "-g", "menu-style", "bg=red"]);
    t.run(&[
        "display-menu",
        "-T",
        "Opts",
        "Only",
        "o",
        "rename-window only",
        "Next",
        "n",
        "rename-window next",
    ]);
    s.wait_for("the simple menu", |text| text.contains("+-Opts"));
    // The first cell of `text` on the screen, so a capital letter elsewhere
    // (a prompt, a title) is not taken for the menu's.
    let cell_of = |text: &str| {
        let n = text.chars().count();
        (0..ROWS)
            .find_map(|r| {
                s.row_cells(r)
                    .windows(n)
                    .find(|w| w.iter().map(|(c, _, _)| c.as_str()).collect::<String>() == text)
                    .map(|w| w[0].clone())
            })
            .unwrap_or_else(|| panic!("{text} on the screen"))
    };
    let corner = cell_of("+-Opts");
    assert_eq!(corner.1, "Idx(2)", "menu-border-style: {corner:?}");
    // The first item is selected; the second has menu-style.
    let item = cell_of("Next");
    assert_eq!(item.2, "Idx(1)", "menu-style: {item:?}");
    s.send(b"\x1b");
    s.wait_for("the menu closed", |text| !text.contains("+-Opts"));
    // -b still wins over the option.
    t.run(&["set-option", "-g", "popup-border-lines", "double"]);
    let hold = if cfg!(windows) {
        "echo OPTS-BOX& ping -n 30 127.0.0.1 >NUL"
    } else {
        "echo OPTS-BOX; sleep 30"
    };
    t.run(&["display-popup", hold]);
    s.wait_for("the double popup", |text| {
        text.contains("OPTS-BOX") && text.contains('╔')
    });
    t.run(&["display-popup", "-C"]);
    t.run(&["display-popup", "-b", "rounded", hold]);
    s.wait_for("the rounded popup", |text| {
        text.contains("OPTS-BOX") && text.contains('╭')
    });
    t.run(&["display-popup", "-C"]);
    assert_eq!(
        t.run(&["show-options", "-gv", "popup-border-lines"]).trim(),
        "double"
    );
    let bad = t.output(&["set-option", "-g", "menu-border-lines", "bogus"]);
    assert!(!bad.status.success(), "a bad line type was taken");
}

#[test]
fn popups_and_menus_are_placed_where_tmux_places_them() {
    let t = Tmxr::new("popplace");
    let s = t.attach(&["new", "-s", "pl"]);
    s.wait_for("status line", |text| text.contains("pl"));
    // Two panes side by side; the right one, active, is the target.
    t.run(&["split-window", "-h", "-t", "pl"]);
    let hold = if cfg!(windows) {
        "echo PLACED& ping -n 30 127.0.0.1 >NUL"
    } else {
        "echo PLACED; sleep 30"
    };
    // The box's top-left corner.
    let corner = |s: &Screen| -> Option<(u16, u16)> {
        (0..ROWS).find_map(|r| {
            let col = s.row_cells(r).iter().position(|(c, _, _)| c == "┌")?;
            Some((u16::try_from(col).unwrap(), r))
        })
    };
    // Where tmux 3.6 drew each, recorded from a running tmux with the same
    // client size and panes.
    let popups: &[(&str, &str, (u16, u16))] = &[
        ("R", "S", (80, 24)),
        ("#{pane_index}", "3", (1, 0)),
        ("3", "12", (3, 7)),
        ("C", "C", (39, 11)),
        // The target pane's left edge and bottom: tmux's split gives the
        // left pane 50 columns, so the right one starts at 51.
        ("P", "P", (51, 24)),
        ("#{popup_pane_right}", "#{popup_centre_y}", (80, 11)),
        // No mouse event: the variables are empty, which is 0.
        ("M", "M", (0, 0)),
    ];
    for &(x, y, want) in popups {
        t.run(&[
            "display-popup",
            "-x",
            x,
            "-y",
            y,
            "-w",
            "20",
            "-h",
            "5",
            hold,
        ]);
        s.wait_for("the popup, drawn", |text| {
            text.contains("PLACED") && corner(&s).is_some()
        });
        assert_eq!(corner(&s), Some(want), "popup -x {x} -y {y}");
        t.run(&["display-popup", "-C"]);
        s.wait_for("closed", |text| !text.contains("PLACED"));
    }
    let menus: &[(&str, &str, (u16, u16))] = &[
        ("3", "12", (3, 8)),
        ("C", "C", (44, 12)),
        ("R", "S", (89, 25)),
    ];
    for &(x, y, want) in menus {
        t.run(&[
            "display-menu",
            "-T",
            "t",
            "-x",
            x,
            "-y",
            y,
            "aaa",
            "a",
            "",
            "bbb",
            "b",
            "",
        ]);
        s.wait_for("the menu, drawn", |text| {
            text.contains("aaa") && corner(&s).is_some()
        });
        assert_eq!(corner(&s), Some(want), "menu -x {x} -y {y}");
        s.send(b"\x1b");
        s.wait_for("closed", |text| !text.contains("aaa"));
    }
    assert!(
        !t.output(&["display-popup", "-x", "Q", "echo"])
            .status
            .success()
    );
}

#[test]
fn tmuxs_window_menu_works() {
    let t = Tmxr::new("winmenu");
    let s = t.attach(&["new", "-s", "wm", "-n", "first"]);
    s.wait_for("status line", |text| text.contains("wm"));
    t.run(&["new-window", "-d", "-t", "wm:1", "-n", "second"]);
    // tmux's own prefix < bind, in tmxr's defaults.
    let open = |s: &Screen| {
        s.send(PREFIX);
        s.send(b"<");
        s.wait_for("the menu", |text| text.contains("Swap Right"));
    };
    open(&s);
    // The title's #[align=centre] is a style: not shown, and centred over
    // the menu's items.
    let text = s.text();
    assert!(!text.contains("#["), "{text}");
    let title = text
        .lines()
        .find(|l| l.contains("0:first"))
        .expect("the title");
    let item = text.lines().find(|l| l.contains("Swap Right")).unwrap();
    // In characters: the box's lines are wider than a byte.
    let column = |line: &str, byte: usize| line[..byte].chars().count();
    let left = column(item, item.find('│').unwrap());
    let right = column(item, item.rfind('│').unwrap());
    let at = column(title, title.find("0:first").unwrap());
    let (before, after) = (at - left, right - (at + "0:first".len()));
    assert!(
        before.abs_diff(after) <= 2,
        "not centred:
{title}
{item}"
    );
    // Swap Right: this window and the next change places.
    s.send(b"r");
    t.wait_run(
        &[
            "list-windows",
            "-t",
            "wm",
            "-F",
            "#{window_index}:#{window_name}",
        ],
        "swapped",
        |o| {
            o == "0:second
1:first
"
        },
    );
    // Mark: the pane is marked.
    open(&s);
    s.send(b"m");
    t.wait_run(
        &["display-message", "-p", "-t", "wm", "#{pane_marked}"],
        "marked",
        |o| o.trim() == "1",
    );
}

#[test]
fn tmuxs_pane_menu_works_from_the_prefix_and_the_right_button() {
    let t = Tmxr::new("panemenu");
    let s = t.attach(&["new", "-s", "pn"]);
    s.wait_for("status line", |text| text.contains("pn"));
    s.wait_mouse(true);
    t.wait_prompt("pn");
    t.run(&["send-keys", "-t", "pn", "echo rightword", "Enter"]);
    s.wait_for("the output", |text| {
        text.lines().any(|l| l.trim_end() == "rightword")
    });
    // prefix >: copy-mode and mouse items are hidden (no mode, no mouse).
    s.send(PREFIX);
    s.send(b">");
    s.wait_for("the menu", |text| text.contains("Horizontal Split"));
    let text = s.text();
    assert!(!text.contains("Go To Top"), "{text}");
    assert!(!text.contains("Search For"), "{text}");
    s.send(b"\x1b");
    s.wait_for("closed", |text| !text.contains("Horizontal Split"));
    // The right button on a word: the menu offers that word, and Copy
    // puts it in a buffer.
    let text = s.text();
    let (row, line) = text
        .lines()
        .enumerate()
        .find(|(_, l)| l.trim_end() == "rightword")
        .unwrap();
    let col = line.find("rightword").unwrap() + 3;
    // Pressed, not let go: as in tmux, a release on the menu's border
    // (where the mouse is) would close it.
    s.mouse(
        2,
        u16::try_from(col).unwrap(),
        u16::try_from(row).unwrap(),
        false,
    );
    s.wait_for("the menu with the word", |text| {
        text.contains("Copy rightword")
    });
    s.send(b"c");
    t.wait_run(&["show-buffer"], "the word copied", |o| o == "rightword");
    // The button let go, the menu gone.
    s.mouse(
        2,
        u16::try_from(col).unwrap(),
        u16::try_from(row).unwrap(),
        true,
    );
    // Horizontal Split from the menu splits the pane.
    s.send(PREFIX);
    s.send(b">");
    s.wait_for("the menu", |text| text.contains("Horizontal Split"));
    s.send(b"h");
    t.wait_run(
        &["display-message", "-p", "-t", "pn", "#{window_panes}"],
        "split",
        |o| o.trim() == "2",
    );
    // The new right pane is active; a right-click on the left one runs the
    // chosen item against the pane clicked, as tmux's: Mark marks it. The
    // narrower pane rewrapped, so the word is found again.
    s.wait_for("the left pane redrawn", |text| {
        text.lines()
            .any(|l| l.starts_with("rightword") && l.contains('│'))
    });
    let row = s
        .text()
        .lines()
        .position(|l| l.starts_with("rightword") && l.contains('│'))
        .expect("the word in the left pane");
    // Another cell of the word: a second press in the same cell this soon
    // is a double click, which tmux binds to nothing either.
    s.mouse(2, 6, u16::try_from(row).unwrap(), false);
    s.wait_for("the menu again", |text| text.contains("Copy rightword"));
    s.send(b"m");
    s.mouse(2, 6, u16::try_from(row).unwrap(), true);
    t.wait_run(
        &["display-message", "-p", "-t", "pn:0.0", "#{pane_marked}"],
        "the clicked pane marked",
        |o| o.trim() == "1",
    );
}

#[test]
fn tmuxs_status_line_menus_act_on_what_was_clicked() {
    let t = Tmxr::new("statusmenu");
    let s = t.attach(&["new", "-s", "sm", "-n", "clickme"]);
    s.wait_for("status line", |text| text.contains("sm"));
    s.wait_mouse(true);
    t.run(&["set-option", "-g", "status-left", "LEFTPART "]);
    t.run(&["new-window", "-t", "sm:1", "-n", "current"]);
    s.wait_for("both windows listed", |text| {
        text.contains("clickme") && text.contains("LEFTPART")
    });
    let cells = s.status_cells();
    let find = |word: &str| {
        (0..cells.len())
            .find(|&i| {
                cells[i..]
                    .iter()
                    .take(word.chars().count())
                    .map(|c| c.0.as_str())
                    .collect::<String>()
                    == word
            })
            .map_or_else(
                || panic!("{word} in the status line"),
                |c| u16::try_from(c).unwrap(),
            )
    };
    // On a window's name: that window's menu, though another is current;
    // Rename starts from the name of the window clicked.
    // Pressed, not let go: a release outside the menu's items closes it.
    s.mouse(2, find("clickme"), ROWS - 1, false);
    s.wait_for("the window menu", |text| text.contains("0:clickme"));
    s.send(b"n");
    s.wait_for("the prompt", |text| {
        text.contains("clickme") && !text.contains("Swap Right")
    });
    s.send(b"\x1b");
    // The prompt gone, the status line back: a click sent along with the
    // Escape would be read as part of an Alt+Escape.
    s.wait_for("the prompt closed", |text| {
        text.lines().last().is_some_and(|l| l.contains("LEFTPART"))
    });
    s.mouse(2, find("clickme"), ROWS - 1, true);
    // On the left part: the session's menu; New Window adds one.
    s.mouse(2, find("LEFTPART"), ROWS - 1, false);
    s.wait_for("the session menu", |text| text.contains("New Window"));
    s.send(b"w");
    t.wait_run(
        &["display-message", "-p", "-t", "sm", "#{session_windows}"],
        "a new window",
        |o| o.trim() == "3",
    );
}
