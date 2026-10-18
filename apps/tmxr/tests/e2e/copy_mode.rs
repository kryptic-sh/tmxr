use super::*;

#[test]
fn word_separators_and_wrap_search_reach_copy_mode() {
    let t = Tmxr::new("copyopts");
    #[cfg(unix)]
    let program = ["/bin/sh", "-c", "printf 'a.b/c.d e\n'; sleep 30"];
    #[cfg(windows)]
    let program = ["cmd", "/c", "echo a.b/c.d e& ping -n 30 127.0.0.1 >NUL"];
    let mut new = vec!["new-session", "-d", "-s", "co"];
    new.extend(program);
    t.run(&new);
    t.wait_run(&["capture-pane", "-p", "-t", "co"], "the text", |o| {
        o.contains("a.b/c.d e")
    });
    let show = |f: &str| {
        t.run(&["display-message", "-p", "-t", "co", f])
            .trim()
            .to_owned()
    };
    let x = |cmd: &str| {
        t.run(&["send-keys", "-t", "co", "-X", cmd]);
    };
    t.run(&["set", "-g", "word-separators", "/"]);
    t.run(&["copy-mode", "-t", "co"]);
    x("history-top");
    x("start-of-line");
    assert_eq!(show("#{copy_cursor_word}"), "a.b");
    // `.` is on the line twice. From the second, wrap-search off stays put
    // and on goes round to the first.
    let pos = || show("#{copy_cursor_y},#{copy_cursor_x}");
    let search = |cmd: &str| {
        t.run(&["send-keys", "-t", "co", "-X", cmd, "."]);
    };
    t.run(&["set", "-wg", "wrap-search", "off"]);
    search("search-forward");
    let first = pos();
    search("search-forward");
    let second = pos();
    assert_ne!(first, second);
    search("search-forward");
    assert_eq!(pos(), second, "wrap-search off");
    t.run(&["set", "-wg", "wrap-search", "on"]);
    search("search-forward");
    assert_eq!(pos(), first, "wrap-search on");
}

#[test]
fn copy_mode_selection_goes_to_a_buffer_and_pastes() {
    let t = Tmxr::new("copy");
    let s = t.attach(&["new", "-s", "copy"]);
    s.wait_for("status line", |text| text.contains("copy"));
    s.send(b"echo copyme-1234\r");
    s.wait_for("echoed marker", |text| {
        text.matches("copyme-1234").count() >= 2
    });
    let in_mode = |t: &Tmxr| t.run(&["display-message", "-p", "-t", "copy", "#{pane_in_mode}"]);

    // prefix [ enters copy mode; ? searches up; v E y copies one WORD.
    s.send(PREFIX);
    s.send(b"[");
    t.wait_run(
        &["display-message", "-p", "-t", "copy", "#{pane_in_mode}"],
        "copy mode",
        |o| o.trim() == "1",
    );
    s.send(b"?");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    s.send(b"copyme\r");
    s.send(b"vE");
    s.send(b"y");
    t.wait_run(&["show-buffer"], "copied text", |o| o == "copyme-1234");
    assert_eq!(in_mode(&t).trim(), "0", "y leaves copy mode");

    // prefix ] pastes it at the shell prompt.
    s.send(PREFIX);
    s.send(b"]");
    s.wait_for("pasted text", |text| {
        text.matches("copyme-1234").count() >= 3
    });
}

#[test]
fn dragging_in_a_pane_copies_the_selection() {
    let t = Tmxr::new("mousedrag");
    let s = t.attach(&["new", "-s", "dr"]);
    s.wait_for("status line", |text| text.contains("dr"));
    s.wait_mouse(true);
    s.send(b"echo @drag-me\r");
    s.wait_for("echoed", |text| {
        text.lines().any(|l| l.trim_end() == "@drag-me")
    });
    let row = s
        .text()
        .lines()
        .position(|l| l.trim_end() == "@drag-me")
        .expect("output line");
    let row = u16::try_from(row).unwrap();
    // MouseDrag1Pane starts a selection in copy mode, the drag extends it and
    // MouseDragEnd1Pane copies it and leaves copy mode.
    s.mouse(0, 0, row, false);
    s.mouse(32, 3, row, false);
    s.mouse(32, 7, row, false);
    s.mouse(0, 7, row, true);
    t.wait_run(&["show-buffer"], "the selection copied", |o| {
        o.trim_end() == "@drag-me"
    });
    t.wait_run(
        &["display-message", "-p", "-t", "dr", "#{pane_in_mode}"],
        "copy mode left",
        |o| o.trim() == "0",
    );
}

#[test]
fn copy_mode_counts_and_jumps_by_keyboard() {
    let t = Tmxr::new("jump");
    let s = t.attach(&["new", "-s", "jp"]);
    s.wait_for("status line", |text| text.contains("jp"));
    s.send(b"echo jumpme-a-b-c\r");
    s.wait_for("echoed", |text| text.matches("jumpme-a-b-c").count() >= 2);
    s.send(PREFIX);
    s.send(b"[");
    t.wait_run(
        &["display-message", "-p", "-t", "jp", "#{pane_in_mode}"],
        "copy mode",
        |o| o.trim() == "1",
    );
    s.send(b"?");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    s.send(b"jumpme\r");
    // 2f- lands on the second dash; v E y copies from there to the WORD end.
    s.send(b"2f-");
    s.send(b"vEy");
    t.wait_run(&["show-buffer"], "copied text", |o| o == "-b-c");
}

#[test]
fn copy_mode_highlights_search_matches() {
    const YELLOW: u32 = 0xe0_af_68;
    const MAUVE: u32 = 0xbb_9a_f7;
    let t = Tmxr::new("matches");
    let s = t.attach(&["new", "-s", "hl"]);
    s.wait_for("status line", |text| text.contains("hl"));
    // The marker starts with @, which no temp-dir name in the prompt holds:
    // counting a plain letter flaked when the random name contained it.
    s.send(b"echo @zq-hit @zq-hit\r");
    s.wait_for("echoed", |text| text.matches("@zq-hit").count() >= 4);
    s.send(PREFIX);
    s.send(b"[");
    s.send(b"?");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    s.send(b"@zq-hit\r");
    // Every match on screen is highlighted, the one under the cursor apart.
    let backgrounds = |s: &Screen| -> Vec<String> {
        (0..ROWS - 1)
            .flat_map(|row| s.row_cells(row))
            .filter(|c| c.0 == "@")
            .map(|c| c.2)
            .collect()
    };
    s.wait_for("highlighted", |_| {
        let bgs = backgrounds(&s);
        bgs.len() >= 4 && bgs.iter().filter(|b| **b == rgb(MAUVE)).count() == 1
    });
    let bgs = backgrounds(&s);
    let where_marker: Vec<(u16, usize)> = (0..ROWS - 1)
        .flat_map(|row| {
            s.row_cells(row)
                .into_iter()
                .enumerate()
                .filter(|(_, c)| c.0 == "@")
                .map(move |(col, _)| (row, col))
        })
        .collect();
    assert!(
        bgs.iter().all(|b| *b == rgb(YELLOW) || *b == rgb(MAUVE)),
        "{bgs:?} at {where_marker:?}\n{}",
        s.text()
    );
}

#[test]
fn copy_pipe_sends_the_selection_to_copy_command() {
    let t = Tmxr::new("pipe");
    let sink = t.dir.path().join("piped.txt");
    #[cfg(unix)]
    let command = format!("cat > '{}'", sink.display());
    #[cfg(windows)]
    let command = format!("$input | Set-Content -NoNewline -Path '{}'", sink.display());
    let s = t.attach(&["new", "-s", "pp"]);
    s.wait_for("status line", |text| text.contains("pp"));
    t.run(&["set-option", "-g", "copy-command", &command]);
    // The command is PowerShell, and shell commands run with default-shell.
    #[cfg(windows)]
    t.run(&["set-option", "-g", "default-shell", "powershell.exe"]);
    s.send(b"echo pipeme-77\r");
    s.wait_for("echoed", |text| text.matches("pipeme-77").count() >= 2);
    s.send(PREFIX);
    s.send(b"[");
    s.send(b"?");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    s.send(b"pipeme\r");
    s.send(b"vE");
    // copy-pipe-and-cancel with no command uses copy-command.
    std::thread::sleep(Duration::from_millis(200));
    t.run(&["send-keys", "-t", "pp", "-X", "copy-pipe-and-cancel"]);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let got = std::fs::read_to_string(&sink).unwrap_or_default();
        if got.trim_end() == "pipeme-77" {
            break;
        }
        assert!(Instant::now() < deadline, "sink holds {got:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(t.run(&["show-buffer"]), "pipeme-77");
}

/// Unix only: it needs a shell that can write to its error output before
/// reading. The fix is the same on Windows.
#[cfg(unix)]
#[test]
fn copy_pipe_feeds_commands_that_write_first_or_read_little() {
    let t = Tmxr::new("pipebig");
    t.run(&[
        "new-session",
        "-d",
        "-s",
        "pb",
        "seq 1 40000; exec sleep 1000",
    ]);
    t.wait_run(&["capture-pane", "-p", "-t", "pb"], "the output", |o| {
        o.lines().any(|l| l == "40000")
    });
    let pipe_all = |command: &str| {
        t.run(&["copy-mode", "-t", "pb"]);
        for cmd in [
            "history-top",
            "begin-selection",
            "history-bottom",
            "end-of-line",
        ] {
            t.run(&["send-keys", "-t", "pb", "-X", cmd]);
        }
        t.run(&[
            "send-keys",
            "-t",
            "pb",
            "-X",
            "copy-pipe-and-cancel",
            command,
        ]);
    };
    let wait_file = |path: &Path, what: &str, ok: &dyn Fn(&str) -> bool| {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let got = std::fs::read_to_string(path).unwrap_or_default();
            if ok(&got) {
                return got;
            }
            assert!(Instant::now() < deadline, "{what}: {got:?}");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    // A command that fills its error output before reading still gets all
    // of the selection, far more than a pipe holds.
    let count = t.dir.path().join("count");
    pipe_all(&format!(
        "head -c 100000 /dev/zero >&2; wc -c > '{}'",
        count.display()
    ));
    let got = wait_file(&count, "the byte count", &|s| !s.trim().is_empty());
    let bytes: usize = got.trim().parse().unwrap();
    assert!(bytes > 200_000, "{bytes}");
    // One that reads a byte and is done did not fail.
    let done = t.dir.path().join("done");
    pipe_all(&format!(
        "head -c 1 > /dev/null; echo ok > '{}'",
        done.display()
    ));
    wait_file(&done, "the command ran", &|s| s.trim() == "ok");
    std::thread::sleep(Duration::from_millis(500));
    let messages = t.run(&["show-messages"]);
    assert!(!messages.contains("copy-pipe"), "{messages}");
}

#[test]
fn copy_mode_refresh_picks_up_new_output() {
    let t = Tmxr::new("refresh");
    let s = t.attach(&["new", "-s", "rf"]);
    s.wait_for("status line", |text| text.contains("rf"));
    s.send(PREFIX);
    s.send(b"[");
    t.wait_run(
        &["display-message", "-p", "-t", "rf", "#{pane_in_mode}"],
        "copy mode",
        |o| o.trim() == "1",
    );
    // Output arriving meanwhile goes to the live pane, not the snapshot.
    t.run(&["send-keys", "-t", "rf", "echo late-arrival", "Enter"]);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!s.text().contains("late-arrival"), "{}", s.text());
    // r takes a fresh snapshot, still in copy mode.
    s.send(b"r");
    s.wait_for("refreshed", |text| text.contains("late-arrival"));
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "rf", "#{pane_in_mode}"])
            .trim(),
        "1"
    );
}

#[test]
fn copy_mode_shows_a_pending_count_and_marked_border() {
    let t = Tmxr::new("indicators");
    let s = t.attach(&["new", "-s", "in"]);
    s.wait_for("status line", |text| text.contains("in"));

    // A count being typed shows until the command that uses it.
    s.send(PREFIX);
    s.send(b"[");
    t.wait_run(
        &["display-message", "-p", "-t", "in", "#{pane_in_mode}"],
        "copy mode",
        |o| o.trim() == "1",
    );
    s.send(b"5");
    s.wait_for("count shown", |text| text.contains("(repeat) 5"));
    s.send(b"k");
    s.wait_for("count used", |text| !text.contains("(repeat)"));
    s.send(b"q");

    // The marked pane's border is drawn reversed.
    let reversed = |s: &Screen| -> usize {
        let emu = s.emu.lock().unwrap();
        let screen = emu.screen();
        (0..ROWS - 1)
            .flat_map(|row| (0..COLS).map(move |col| (row, col)))
            .filter(|(row, col)| screen.cell(*row, *col).is_some_and(|c| c.inverse()))
            .count()
    };
    t.run(&["split-window", "-h", "-t", "in"]);
    s.wait_for("split", |text| text.contains('│'));
    let before = reversed(&s);
    t.run(&["select-pane", "-m", "-t", "in.0"]);
    s.wait_for("marked border", |_| reversed(&s) > before);
    t.run(&["select-pane", "-M"]);
    s.wait_for("mark cleared", |_| reversed(&s) == before);
}

#[test]
fn emacs_copy_mode_selects_with_its_own_keys() {
    let t = Tmxr::new("emacs");
    let s = t.attach(&["new", "-s", "em"]);
    s.wait_for("status line", |text| text.contains("em"));
    let bad = t.output(&["set-option", "-g", "mode-keys", "bogus"]);
    assert!(
        !bad.status.success() && String::from_utf8_lossy(&bad.stderr).contains("vi or emacs"),
        "{bad:?}"
    );
    t.run(&["set-option", "-g", "mode-keys", "emacs"]);
    s.send(b"echo @emacs-copy\r");
    s.wait_for("echoed", |text| {
        text.lines().any(|l| l.trim_end() == "@emacs-copy")
    });
    s.send(PREFIX);
    s.send(b"[");
    // C-r searches up to the output line's start.
    s.send(b"\x12");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    s.send(b"@emacs-copy\r");
    s.wait_for("search done", |text| !text.contains("(search up)"));
    // C-Space, then M-1 M-0 C-f: ten cells right, and M-w copies.
    s.send(b"\x00");
    s.send(b"\x1b1");
    s.send(b"\x1b0");
    s.send(b"\x06");
    s.send(b"\x1bw");
    t.wait_run(&["show-buffer"], "the selection copied", |o| {
        o.trim_end() == "@emacs-copy"
    });
}

#[test]
fn copy_end_of_line_takes_the_rest_of_the_line() {
    let t = Tmxr::new("eol");
    let s = t.attach(&["new", "-s", "eo"]);
    s.wait_for("status line", |text| text.contains("eo"));
    s.send(b"echo head @eol-copy tail\r");
    s.wait_for("echoed", |text| {
        text.lines().any(|l| l.trim_end() == "head @eol-copy tail")
    });
    s.send(PREFIX);
    s.send(b"[");
    s.send(b"?");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    // The output line is the nearest match above the prompt.
    s.send(b"@eol-copy\r");
    s.wait_for("search done", |text| !text.contains("(search up)"));
    s.send(b"D");
    t.wait_run(&["show-buffer"], "the rest of the line copied", |o| {
        o.trim_end() == "@eol-copy tail"
    });
}

#[test]
fn double_and_triple_clicks_copy_a_word_and_a_line() {
    let t = Tmxr::new("clicks");
    let s = t.attach(&["new", "-s", "ck"]);
    s.wait_for("status line", |text| text.contains("ck"));
    s.wait_mouse(true);
    s.send(b"echo head @clicked tail\r");
    s.wait_for("echoed", |text| {
        text.lines().any(|l| l.trim_end() == "head @clicked tail")
    });
    let row = s
        .text()
        .lines()
        .position(|l| l.trim_end() == "head @clicked tail")
        .expect("output line");
    let row = u16::try_from(row).unwrap();
    // DoubleClick1Pane: the word under the mouse (cols 6..=12, "clicked";
    // the @ is punctuation, its own word).
    s.click(0, 8, row);
    s.click(0, 8, row);
    t.wait_run(&["show-buffer"], "the word copied", |o| {
        o.trim_end() == "clicked"
    });
    // TripleClick1Pane: the whole line.
    s.click(0, 2, row);
    s.click(0, 2, row);
    s.click(0, 2, row);
    t.wait_run(&["show-buffer"], "the line copied", |o| {
        o.trim_end() == "head @clicked tail"
    });

    // The second press is SecondClick1Pane; DoubleClick1Pane comes from the
    // click timer, so a third press in time cancels it.
    let opt = |t: &Tmxr, name: &str| {
        t.run(&["display-message", "-p", &format!("#{{@{name}}}")])
            .trim()
            .to_owned()
    };
    t.run(&[
        "bind-key",
        "-n",
        "SecondClick1Pane",
        "set-option -g @second yes",
    ]);
    t.run(&[
        "bind-key",
        "-n",
        "DoubleClick1Pane",
        "set-option -g @double fired",
    ]);
    t.run(&["set-option", "-g", "@double", "none"]);
    s.click(0, 2, row);
    s.click(0, 2, row);
    s.click(0, 2, row);
    t.wait_run(
        &["display-message", "-p", "#{@second}"],
        "SecondClick",
        |o| o.trim() == "yes",
    );
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(
        opt(&t, "double"),
        "none",
        "a triple click fired DoubleClick"
    );
    // A double click alone does, once the click time has passed.
    std::thread::sleep(Duration::from_millis(400));
    s.click(0, 2, row);
    s.click(0, 2, row);
    t.wait_run(
        &["display-message", "-p", "#{@double}"],
        "DoubleClick",
        |o| o.trim() == "fired",
    );
}

#[test]
fn emacs_search_moves_as_you_type() {
    let t = Tmxr::new("incsearch");
    let s = t.attach(&["new", "-s", "is"]);
    s.wait_for("status line", |text| text.contains("is"));
    t.run(&["set-option", "-g", "mode-keys", "emacs"]);
    s.send(b"echo @inc-search\r");
    s.wait_for("echoed", |text| {
        text.lines().any(|l| l.trim_end() == "@inc-search")
    });
    s.send(PREFIX);
    s.send(b"[");
    s.send(b"\x12");
    s.wait_for("search prompt", |text| text.contains("(search up)"));
    // Typed, never entered: Escape leaves the prompt with the cursor where
    // the typing took it.
    s.send(b"@inc-search");
    s.wait_for("typed", |text| text.contains("(search up) @inc-search"));
    s.send(b"\x1b");
    s.wait_for("prompt closed", |text| !text.contains("(search up)"));
    // C-Space, M-1 M-0 C-f, M-w: the eleven cells from the match.
    s.send(b"\x00");
    s.send(b"\x1b1");
    s.send(b"\x1b0");
    s.send(b"\x06");
    s.send(b"\x1bw");
    t.wait_run(&["show-buffer"], "the match copied", |o| {
        o.trim_end() == "@inc-search"
    });
}

#[test]
fn prefix_y_copies_the_shells_command_line() {
    let t = Tmxr::new("yankline");
    // A shell with line editing: dash (Ubuntu's /bin/sh) has none.
    let (new, shell) = if cfg!(windows) {
        (vec!["new", "-s", "yk"], "cmd")
    } else {
        (
            vec!["new", "-s", "yk", "bash --norc --noprofile -i"],
            "bash",
        )
    };
    let s = t.attach(&new);
    s.wait_for("status line", |text| text.contains("yk"));
    t.wait_run(
        &[
            "display-message",
            "-p",
            "-t",
            "yk",
            "#{pane_current_command}",
        ],
        "the shell",
        |o| o.trim().eq_ignore_ascii_case(shell),
    );
    // Its prompt too: keys typed before the line editor reads them are
    // echoed by the terminal, then drawn again after the prompt.
    t.wait_prompt("yk");
    // Typed, not run, with the cursor left in the middle of it.
    s.send(b"echo yank-me-now");
    s.wait_for("the typing", |text| text.contains("echo yank-me-now"));
    for _ in 0..4 {
        s.send(b"\x1b[D");
    }
    s.send(PREFIX);
    s.send(b"y");
    t.wait_run(&["show-buffer"], "the command line copied", |o| {
        o == "echo yank-me-now"
    });
}

#[test]
fn prefix_y_uses_the_shells_prompt_mark() {
    let t = Tmxr::new("yankmark");
    let (new, shell, mark) = if cfg!(windows) {
        (vec!["new", "-s", "ym"], "cmd", r"prompt $P$G$e]133;B$e\")
    } else {
        (
            vec!["new", "-s", "ym", "bash --norc --noprofile -i"],
            "bash",
            r"PS1='$ \[\e]133;B\a\]'",
        )
    };
    let s = t.attach(&new);
    s.wait_for("status line", |text| text.contains("ym"));
    t.wait_run(
        &[
            "display-message",
            "-p",
            "-t",
            "ym",
            "#{pane_current_command}",
        ],
        "the shell",
        |o| o.trim().eq_ignore_ascii_case(shell),
    );
    t.wait_prompt("ym");
    // The prompt now ends with an OSC 133;B mark.
    s.send(mark.as_bytes());
    s.send(b"\r");
    std::thread::sleep(Duration::from_millis(500));
    s.send(b"echo yank-me-now");
    s.wait_for("the typing", |text| text.contains("echo yank-me-now"));
    for _ in 0..4 {
        s.send(b"\x1b[D");
    }
    s.send(PREFIX);
    s.send(b"y");
    t.wait_run(&["show-buffer"], "the command line copied", |o| {
        o == "echo yank-me-now"
    });
    // With the mark no keys were sent: the cursor is still mid-word.
    s.send(b"Z");
    s.wait_for("Z typed where the cursor was", |text| {
        text.contains("echo yank-meZ-now")
    });
}

#[test]
fn append_selection_and_toggle_position_do_what_tmuxs_do() {
    let t = Tmxr::new("appendsel");
    let s = t.attach(&["new", "-s", "ap"]);
    s.wait_for("status line", |text| text.contains("ap"));
    t.wait_prompt("ap");
    t.run(&["send-keys", "-t", "ap", "echo apword", "Enter"]);
    s.wait_for("the output", |text| {
        text.lines().any(|l| l.trim_end() == "apword")
    });
    t.run(&["set-buffer", "base-"]);
    t.run(&["copy-mode", "-t", "ap"]);
    s.wait_for("the position", |text| text.contains("[0/"));
    // toggle-position hides the indicator, and shows it again.
    let x = |args: &[&str]| {
        let mut cmd = vec!["send-keys", "-t", "ap", "-X"];
        cmd.extend_from_slice(args);
        t.run(&cmd);
    };
    x(&["toggle-position"]);
    s.wait_for("the position hidden", |text| !text.contains("[0/"));
    x(&["toggle-position"]);
    s.wait_for("the position back", |text| text.contains("[0/"));
    // The selection goes on the end of the newest buffer, and copy mode
    // ends.
    x(&["search-backward", "apword"]);
    x(&["begin-selection"]);
    x(&["-N", "5", "cursor-right"]);
    x(&["append-selection-and-cancel"]);
    assert_eq!(t.run(&["show-buffer"]), "base-apword");
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "ap", "#{pane_in_mode}"])
            .trim(),
        "0"
    );
}

#[test]
fn star_and_hash_search_for_the_word_at_the_cursor() {
    let t = Tmxr::new("starhash");
    let s = t.attach(&["new", "-s", "sh"]);
    s.wait_for("status line", |text| text.contains("sh"));
    t.wait_prompt("sh");
    t.run(&[
        "send-keys",
        "-t",
        "sh",
        "echo one target two target",
        "Enter",
    ]);
    s.wait_for("the output", |text| {
        text.lines()
            .any(|l| l.trim_end() == "one target two target")
    });
    t.run(&["copy-mode", "-t", "sh"]);
    let x = |args: &[&str]| {
        let mut cmd = vec!["send-keys", "-t", "sh", "-X"];
        cmd.extend_from_slice(args);
        t.run(&cmd);
    };
    // The output line, nearest above; then onto its first "target".
    x(&["search-backward", "one target"]);
    x(&["next-word"]);
    let at = |t: &Tmxr| {
        t.run(&[
            "display-message",
            "-p",
            "-t",
            "sh",
            "#{copy_cursor_word} #{copy_cursor_x}",
        ])
        .trim()
        .to_owned()
    };
    assert_eq!(at(&t), "target 4");
    // vi *: on to the next "target"; #: back.
    s.send(b"*");
    t.wait_run(
        &["display-message", "-p", "-t", "sh", "#{copy_cursor_x}"],
        "*",
        |o| o.trim() == "15",
    );
    s.send(b"#");
    t.wait_run(
        &["display-message", "-p", "-t", "sh", "#{copy_cursor_x}"],
        "#",
        |o| o.trim() == "4",
    );
}
