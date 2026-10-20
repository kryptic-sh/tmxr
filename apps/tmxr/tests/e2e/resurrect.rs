use super::*;

#[test]
fn resurrect_restores_a_multi_pane_layout() {
    let t = Tmxr::new("resurrect");
    t.run(&["new-session", "-d", "-s", "main"]);
    t.run(&["split-window", "-h", "-t", "main"]);
    t.run(&["split-window", "-v", "-t", "main"]);
    t.run(&["resize-pane", "-L", "-t", "main", "5"]);
    // Pane 2 active, pane 0 the last pane.
    t.run(&["select-pane", "-t", "main.0"]);
    t.run(&["select-pane", "-t", "main.2"]);
    t.wait_run(&["list-panes", "-t", "main"], "three panes", |o| {
        o.lines().count() == 3
    });
    // `0: [38x23] %0 (active)`: pane ids change on restore, so drop them and
    // compare index, size and the active mark.
    let layout = |t: &Tmxr| -> String {
        t.run(&["list-panes", "-t", "main"])
            .lines()
            .map(|l| {
                l.split_whitespace()
                    .filter(|w| !w.starts_with('%'))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let saved = layout(&t);
    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);

    // A new server restores the save before running its first command.
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        config.replace("restore-on-start = false", "restore-on-start = true"),
    )
    .unwrap();
    t.run(&["new-session", "-d", "-s", "other"]);
    let restored = layout(&t);
    assert_eq!(restored.lines().count(), 3, "{restored}");
    assert_eq!(restored, saved);
    // The last pane came back too: select-pane -l goes to pane 0.
    t.run(&["select-pane", "-l", "-t", "main"]);
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "main", "#{pane_index}"])
            .trim(),
        "0"
    );
}

#[test]
fn resurrect_skips_unchanged_saves_and_reports_missing_dirs() {
    let t = Tmxr::new("resave");
    let saves = |t: &Tmxr| -> usize {
        let root = t.dir.path().join("data").join("tmxr").join("resurrect");
        std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .flat_map(|server| {
                std::fs::read_dir(server.path())
                    .into_iter()
                    .flatten()
                    .flatten()
            })
            .filter(|f| f.file_name().to_string_lossy().ends_with(".json"))
            .count()
    };
    let gone = t.dir.path().join("soon-gone");
    std::fs::create_dir(&gone).unwrap();
    let gone_str = gone.display().to_string();
    t.run(&["new-session", "-d", "-s", "keep", "-c", &gone_str]);
    // Saves include titles: one arriving late would change the next save.
    t.wait_settled("keep");
    t.run(&["resurrect-save"]);
    assert_eq!(saves(&t), 1);
    // Nothing changed: the save on exit writes nothing new.
    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);
    assert_eq!(saves(&t), 1);

    // The session's directory disappears; restoring says so.
    std::fs::remove_dir(&gone).unwrap();
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        config.replace("restore-on-start = false", "restore-on-start = true"),
    )
    .unwrap();
    t.run(&["new-session", "-d", "-s", "other"]);
    let messages = t.run(&["show-messages"]);
    assert!(messages.contains("missing, started in $HOME"), "{messages}");
    assert!(messages.contains("soon-gone"), "{messages}");
}

#[cfg(unix)]
#[test]
fn sigterm_saves_sessions_before_the_server_exits() {
    let t = Tmxr::new("sigterm");
    t.run(&["new-session", "-d", "-s", "keepme"]);
    let pid = t.run(&["display-message", "-p", "-t", "keepme", "#{pid}"]);
    let pid = pid.trim();
    assert!(pid.parse::<u32>().is_ok(), "{pid:?}");
    let killed = tmxr_term::child::command("kill")
        .args(["-TERM", pid])
        .status()
        .unwrap();
    assert!(killed.success());
    t.wait_run(&["ls"], "server gone", str::is_empty);
    let root = t.dir.path().join("data").join("tmxr").join("resurrect");
    let saved: Vec<String> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|server| {
            std::fs::read_dir(server.path())
                .into_iter()
                .flatten()
                .flatten()
        })
        .filter(|f| f.file_name().to_string_lossy().ends_with(".json"))
        .map(|f| std::fs::read_to_string(f.path()).unwrap_or_default())
        .collect();
    assert!(saved.iter().any(|s| s.contains("keepme")), "{saved:?}");
}

#[test]
fn resurrect_restores_allowlisted_programs_with_their_arguments() {
    // A program with arguments; on Windows the pane's cmd runs it as a child.
    #[cfg(unix)]
    let (program, start, args) = ("sleep", vec!["sleep", "300"], vec!["300"]);
    #[cfg(windows)]
    let (program, start, args) = (
        "PING",
        vec!["cmd.exe", "/d", "/c", "ping -n 300 127.0.0.1 >NUL"],
        vec!["-n", "300", "127.0.0.1"],
    );
    let t = Tmxr::new("resargs");
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        format!("{config}processes = [{program:?}]\nrestore-args = [{program:?}]\n"),
    )
    .unwrap();
    let mut new = vec!["new-session", "-d", "-s", "main"];
    new.extend(&start);
    t.run(&new);
    let running = |t: &Tmxr| {
        t.wait_run(
            &[
                "display-message",
                "-p",
                "-t",
                "main",
                "#{pane_current_command}",
            ],
            "the program running",
            |o| o.trim().eq_ignore_ascii_case(program),
        );
    };
    let saved_args = |t: &Tmxr| saved_args(&t.newest_save());
    running(&t);
    t.run(&["resurrect-save"]);
    assert_eq!(saved_args(&t), args);

    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        config.replace("restore-on-start = false", "restore-on-start = true"),
    )
    .unwrap();
    t.run(&["new-session", "-d", "-s", "other"]);
    // The restored program got its arguments back: a fresh save reads them
    // from the running process again.
    running(&t);
    std::thread::sleep(Duration::from_millis(5));
    t.run(&["resurrect-save"]);
    assert_eq!(saved_args(&t), args);
    t.run(&["kill-server"]);
}

/// What Windows sends at logoff or shutdown, sent by hand: the server saves
/// its sessions and exits.
#[cfg(windows)]
#[test]
fn windows_session_end_saves_and_exits() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SMTO_BLOCK, SendMessageTimeoutW, WM_ENDSESSION,
    };
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(Some(0)).collect() };
    let t = Tmxr::new("sessend");
    t.run(&["new-session", "-d", "-s", "kept-at-logoff"]);
    let pid = t
        .run(&["display-message", "-p", "#{pid}"])
        .trim()
        .to_owned();
    let (class, title) = (wide("tmxr-server"), wide(&format!("tmxr-server {pid}")));
    let deadline = Instant::now() + TIMEOUT;
    let hwnd = loop {
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let h = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
        if !h.is_null() {
            break h;
        }
        assert!(Instant::now() < deadline, "no session-end window for {pid}");
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut result = 0usize;
    // ENDSESSION_LOGOFF; the call returns once the server's handler has.
    // SAFETY: `hwnd` is a window handle, `result` a live usize.
    let sent = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_ENDSESSION,
            1,
            0x8000_0000,
            SMTO_BLOCK,
            10_000,
            &mut result,
        )
    };
    assert_ne!(sent, 0, "WM_ENDSESSION was not delivered");
    t.wait_run(&["ls"], "server gone", str::is_empty);
    let root = t.dir.path().join("data").join("tmxr").join("resurrect");
    let saved: Vec<String> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|server| {
            std::fs::read_dir(server.path())
                .into_iter()
                .flatten()
                .flatten()
        })
        .filter(|f| f.file_name().to_string_lossy().ends_with(".json"))
        .map(|f| std::fs::read_to_string(f.path()).unwrap_or_default())
        .collect();
    assert!(
        saved.iter().any(|s| s.contains("kept-at-logoff")),
        "{saved:?}"
    );
}

#[test]
fn resurrect_matches_command_lines_as_tmux_resurrect() {
    // A program with arguments; on Windows the pane's cmd runs it as a child.
    #[cfg(unix)]
    let (program, start) = ("sleep", vec!["sleep", "300"]);
    #[cfg(windows)]
    let (program, start) = (
        "PING",
        vec!["cmd.exe", "/d", "/c", "ping -n 300 127.0.0.1 >NUL"],
    );
    let t = Tmxr::new("resmatch");
    let base = std::fs::read_to_string(&t.config).unwrap();
    let mut new = vec!["new-session", "-d", "-s", "main"];
    new.extend(&start);
    t.run(&new);
    t.wait_run(
        &[
            "display-message",
            "-p",
            "-t",
            "main",
            "#{pane_current_command}",
        ],
        "the program running",
        |o| o.trim().eq_ignore_ascii_case(program),
    );
    let save_with = |t: &Tmxr, processes: &str| {
        std::fs::write(
            &t.config,
            format!(
                "{base}processes = [{processes}]
"
            ),
        )
        .unwrap();
        t.run(&["source-file"]);
        t.run(&["resurrect-save"]);
        t.newest_save()
    };
    // ~: a command line holding the text, restored whole.
    #[cfg(unix)]
    let (anywhere, whole) = ("\"~eep 30\"", vec!["300"]);
    #[cfg(windows)]
    let (anywhere, whole) = ("\"~300 127.0\"", vec!["-n", "300", "127.0.0.1"]);
    assert_eq!(saved_args(&save_with(&t, anywhere)), whole);
    // ->: a command line starting with the match, restored as the command.
    #[cfg(unix)]
    let (swap, restored) = ("\"sleep 300->sleep 'a b'\"", vec!["a b"]);
    #[cfg(windows)]
    let (swap, restored) = ("\"ping -n 300->ping -n 'a b'\"", vec!["-n", "a b"]);
    assert_eq!(saved_args(&save_with(&t, swap)), restored);
    // Neither matches: nothing restored.
    let none = save_with(&t, "\"~not-in-it\", \"zzz->x\"");
    assert!(!none.contains("\"command\": \""), "{none}");
}

#[test]
fn resurrect_restores_a_linked_window_as_one_window() {
    let t = Tmxr::new("reslink");
    t.run(&["new-session", "-d", "-s", "a"]);
    t.run(&["new-window", "-d", "-t", "a:1", "-n", "shared"]);
    t.run(&["new-session", "-d", "-s", "b", "-n", "own"]);
    t.run(&["link-window", "-d", "-s", "a:shared", "-t", "b:5"]);
    // In c the linked window is the first one, which a session is made with.
    t.run(&["new-session", "-d", "-s", "c", "-n", "later"]);
    t.run(&["move-window", "-s", "c:0", "-t", "c:9"]);
    t.run(&["link-window", "-d", "-s", "a:shared", "-t", "c:0"]);
    t.run(&["resurrect-save"]);
    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);

    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        config.replace("restore-on-start = false", "restore-on-start = true"),
    )
    .unwrap();
    t.run(&["new-session", "-d", "-s", "other"]);
    let id = |target: &str| {
        t.run(&["display-message", "-p", "-t", target, "#{window_id}"])
            .trim()
            .to_owned()
    };
    let shared = id("a:shared");
    assert!(!shared.is_empty());
    assert_eq!(id("b:5"), shared, "b's link");
    assert_eq!(id("c:0"), shared, "c's first window");
    assert_eq!(
        t.run(&["display-message", "-p", "-t", "c:0", "#{window_linked}"])
            .trim(),
        "1"
    );
    // c kept its own window, and no copy of the shared one exists.
    let windows = |s: &str| t.run(&["list-windows", "-t", s]).lines().count();
    assert_eq!(windows("c"), 2);
    assert_eq!(windows("b"), 2);
    assert!(!id("c:9").is_empty());
}

#[test]
fn resurrect_keeps_a_manual_window_size() {
    let t = Tmxr::new("ressize");
    t.run(&["new-session", "-d", "-s", "rz"]);
    t.run(&["resize-window", "-t", "rz", "-x", "41", "-y", "11"]);
    t.run(&["resurrect-save"]);
    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        config.replace("restore-on-start = false", "restore-on-start = true"),
    )
    .unwrap();
    t.run(&["new-session", "-d", "-s", "other"]);
    // A client attached would size an automatic window to itself.
    let s = t.attach(&["attach", "-t", "rz"]);
    s.wait_for("status line", |text| text.contains("rz"));
    std::thread::sleep(Duration::from_millis(300));
    let size = t.run(&[
        "display-message",
        "-p",
        "-t",
        "rz",
        "#{window_width}x#{window_height}",
    ]);
    assert_eq!(size.trim(), "41x11");
}

#[test]
fn resurrect_keeps_a_mirrored_windows_pane_numbers() {
    let t = Tmxr::new("resmirror");
    t.run(&["new-session", "-d", "-s", "rm", "-x", "100", "-y", "30"]);
    t.run(&["split-window", "-t", "rm"]);
    t.run(&["split-window", "-t", "rm"]);
    t.run(&["select-layout", "-t", "rm", "main-horizontal-mirrored"]);
    // Pane 0, the main pane, laid out at the bottom.
    let where_ = || {
        t.run(&[
            "list-panes",
            "-t",
            "rm",
            "-F",
            "#{pane_index}:#{?#{pane_top},low,high}",
        ])
    };
    assert_eq!(
        where_().split_whitespace().collect::<Vec<_>>(),
        ["0:low", "1:high", "2:high"]
    );
    t.run(&["resurrect-save"]);
    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        config.replace("restore-on-start = false", "restore-on-start = true"),
    )
    .unwrap();
    t.run(&["new-session", "-d", "-s", "other"]);
    t.wait_run(&["list-panes", "-t", "rm"], "restored", |o| {
        o.lines().count() == 3
    });
    assert_eq!(
        where_().split_whitespace().collect::<Vec<_>>(),
        ["0:low", "1:high", "2:high"]
    );
}

#[test]
fn select_pane_titles_a_pane_and_resurrect_keeps_it() {
    let t = Tmxr::new("titles");
    t.run(&["new-session", "-d", "-s", "ti"]);
    t.wait_settled("ti");
    let title = |t: &Tmxr| {
        t.run(&["display-message", "-p", "-t", "ti", "#{pane_title}"])
            .trim()
            .to_owned()
    };
    t.run(&["select-pane", "-t", "ti", "-T", "my-title-#{session_name}"]);
    assert_eq!(title(&t), "my-title-ti");
    t.run(&["resurrect-save"]);
    assert!(
        t.newest_save().contains("\"title\": \"my-title-ti\""),
        "{}",
        t.newest_save()
    );
    // Restored, until the program titles itself; on Windows ConPTY reports
    // cmd's own title as it starts, so only Unix's sh keeps it.
    if cfg!(unix) {
        t.run(&["kill-server"]);
        t.wait_run(&["ls"], "server gone", str::is_empty);
        let config = std::fs::read_to_string(&t.config).unwrap();
        std::fs::write(
            &t.config,
            config.replace("restore-on-start = false", "restore-on-start = true"),
        )
        .unwrap();
        t.run(&["new-session", "-d", "-s", "other"]);
        assert_eq!(title(&t), "my-title-ti");
    }
}

/// An empty pane (`split-window -I`) is restored empty, with no program,
/// not as a pane running the shell.
#[test]
fn resurrect_restores_an_empty_pane_empty() {
    let t = Tmxr::new("resempty");
    t.run(&["new-session", "-d", "-s", "re"]);
    let made = t.output_with_stdin(&["split-window", "-I", "-t", "re"], b"text\n");
    assert!(made.status.success(), "{made:?}");
    let pids = |t: &Tmxr| t.run(&["list-panes", "-t", "re", "-F", "#{pane_pid}"]);
    t.wait_run(
        &["list-panes", "-t", "re", "-F", "#{pane_pid}"],
        "two panes",
        |o| o.lines().count() == 2,
    );
    assert_eq!(pids(&t).lines().nth(1), Some("0"), "the pane is empty");
    t.run(&["resurrect-save"]);
    t.run(&["kill-server"]);
    t.wait_run(&["ls"], "server gone", str::is_empty);
    let config = std::fs::read_to_string(&t.config).unwrap();
    std::fs::write(
        &t.config,
        config.replace("restore-on-start = false", "restore-on-start = true"),
    )
    .unwrap();
    t.run(&["new-session", "-d", "-s", "other"]);
    let restored = pids(&t);
    let restored: Vec<&str> = restored.lines().collect();
    assert_eq!(restored.len(), 2, "{restored:?}");
    assert_ne!(restored[0], "0", "the first pane runs its shell");
    assert_eq!(restored[1], "0", "the empty pane is empty again");
}
