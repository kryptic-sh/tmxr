use super::*;

#[test]
fn a_programs_clipboard_makes_a_buffer_and_fires_pane_set_clipboard() {
    let t = Tmxr::new("osc52");
    t.run(&["new-session", "-d", "-s", "cl"]);
    t.run(&["set-hook", "-g", "pane-set-clipboard", "set -ga @clip x"]);
    // A program that copies "hi" (base64 aGk=) after a moment.
    #[cfg(unix)]
    let copier = |marker: &str| {
        vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            format!(r"sleep 1; printf '\033]52;c;aGk=\007{marker}'; sleep 30"),
        ]
    };
    #[cfg(windows)]
    let copier = |marker: &str| {
        vec![
            "powershell.exe".to_owned(),
            "-NoProfile".to_owned(),
            "-Command".to_owned(),
            format!(
                "Start-Sleep 1; [Console]::Out.Write([string][char]27 + ']52;c;aGk=' + [char]7 + '{marker}'); Start-Sleep 30"
            ),
        ]
    };
    let run_copier = |window: &str, marker: &str| {
        let mut args = vec![
            "new-window".to_owned(),
            "-d".to_owned(),
            "-t".to_owned(),
            window.to_owned(),
        ];
        args.extend(copier(marker));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        t.run(&args);
        t.wait_run(&["capture-pane", "-p", "-t", window], "the copy", |o| {
            o.contains(marker)
        });
    };
    let buffers = || t.run(&["list-buffers", "-F", "#{buffer_sample}"]);
    // external, tmux's default: a program's copy is left alone.
    t.run(&["set", "-g", "set-clipboard", "external"]);
    run_copier("cl:1", "first-done");
    assert_eq!(buffers().trim(), "");
    assert_eq!(t.run(&["show", "-gv", "@clip"]).trim(), "");
    // on: it is passed on, made a buffer, and the hook runs.
    t.run(&["set", "-g", "set-clipboard", "on"]);
    run_copier("cl:2", "second-done");
    t.wait_run(&["show", "-gv", "@clip"], "the hook", |o| o.trim() == "x");
    assert_eq!(buffers().trim(), "hi");
    // A copy in copy mode fires it too.
    t.run(&["copy-mode", "-t", "cl:2"]);
    for cmd in [
        "history-top",
        "begin-selection",
        "cursor-right",
        "copy-selection",
    ] {
        t.run(&["send-keys", "-t", "cl:2", "-X", cmd]);
    }
    t.wait_run(&["show", "-gv", "@clip"], "the copy's hook", |o| {
        o.trim() == "xx"
    });
}

#[test]
fn buffer_limit_and_message_limit_are_server_options() {
    let t = Tmxr::new("limits");
    let s = t.attach(&["new", "-s", "lm"]);
    s.wait_for("the status line", |text| text.contains("lm"));
    t.run(&["set", "-s", "buffer-limit", "3"]);
    for i in 0..5 {
        t.run(&["set-buffer", &format!("b{i}")]);
    }
    assert_eq!(t.run(&["list-buffers"]).lines().count(), 3);
    assert_eq!(
        t.run(&["show", "-s", "buffer-limit"]).trim(),
        "buffer-limit 3"
    );
    assert!(
        !t.output(&["set", "-s", "buffer-limit", "0"])
            .status
            .success()
    );
    t.run(&["set", "-s", "message-limit", "2"]);
    // Shown on the client, a message is kept too.
    let client = t
        .run(&["list-clients", "-F", "#{client_name}"])
        .trim()
        .to_owned();
    for m in ["first", "second", "third"] {
        t.run(&["display-message", "-c", &client, m]);
    }
    assert_eq!(t.run(&["show-messages"]), "second\nthird\n");
}

#[test]
fn the_buffer_limit_drops_only_automatic_buffers() {
    let t = Tmxr::new("buflimit");
    t.run(&["new-session", "-d", "-s", "bl"]);
    t.run(&["set-buffer", "-b", "keep", "k"]);
    for i in 0..52 {
        t.run(&["set-buffer", &format!("a{i}")]);
    }
    let names = |t: &Tmxr| -> Vec<String> {
        t.run(&["list-buffers", "-F", "#{buffer_name}"])
            .lines()
            .map(str::to_owned)
            .collect()
    };
    // As tmux 3.6: the 50 newest automatic buffers, and the named one.
    let listed = names(&t);
    assert_eq!(listed.len(), 51, "{listed:?}");
    assert_eq!(listed.first().map(String::as_str), Some("buffer51"));
    assert_eq!(listed.last().map(String::as_str), Some("keep"));
    assert!(!listed.contains(&"buffer1".to_owned()), "{listed:?}");
    // Set by name, an automatic buffer counts as named from then on.
    t.run(&["set-buffer", "-b", "buffer10", "named"]);
    t.run(&["set-buffer", "z1"]);
    t.run(&["set-buffer", "z2"]);
    let listed = names(&t);
    assert_eq!(listed.len(), 52, "{listed:?}");
    assert!(listed.contains(&"buffer10".to_owned()), "{listed:?}");
}

#[test]
fn buffers_save_to_and_load_from_files() {
    let t = Tmxr::new("buffers");
    t.run(&["new-session", "-d", "-s", "b"]);
    let file = t.dir.path().join("buffer.txt");
    let path = file.display().to_string();

    t.run(&["set-buffer", "-b", "one", "hello"]);
    t.run(&["save-buffer", "-b", "one", &path]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello");
    t.run(&["save-buffer", "-a", "-b", "one", &path]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "hellohello");
    assert_eq!(t.run(&["save-buffer", "-b", "one", "-"]), "hello");

    std::fs::write(&file, "from a file\n").unwrap();
    t.run(&["load-buffer", "-b", "two", &path]);
    assert_eq!(t.run(&["show-buffer", "-b", "two"]), "from a file\n");

    // -: what was piped into the command, as tmux's.
    let mut piped = tmxr_term::child::command(env!("CARGO_BIN_EXE_tmxr"))
        .args(t.args(&["load-buffer", "-b", "three", "-"]))
        .envs(t.env())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    piped
        .stdin
        .take()
        .unwrap()
        .write_all(b"from a pipe\n")
        .unwrap();
    let out = piped.wait_with_output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(t.run(&["show-buffer", "-b", "three"]), "from a pipe\n");
}

/// The server runs detached, with no console: a shell command it starts
/// must not be given a console window of its own (one per `run-shell`).
#[cfg(windows)]
#[test]
fn shell_commands_open_no_console_window() {
    let t = Tmxr::new("nowindow");
    t.run(&["new-session", "-d", "-s", "nw"]);
    let me = std::env::current_exe().unwrap();
    let line = format!(
        "set {CONSOLE_PROBE}=1&& {} console_window_probe --exact --ignored --nocapture",
        me.display()
    );
    let out = t.run(&["run-shell", &line]);
    assert!(out.contains("console-window=false"), "{out}");
}

#[test]
fn wait_for_blocks_until_signalled_or_unlocked() {
    let t = Tmxr::new("waitfor");
    t.run(&["new-session", "-d", "-s", "wf"]);
    let spawn = |args: &[&str]| {
        tmxr_term::child::command(env!("CARGO_BIN_EXE_tmxr"))
            .args(t.args(args))
            .envs(t.env())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };
    let exits_within = |child: &mut std::process::Child, d: Duration| {
        let deadline = Instant::now() + d;
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait().unwrap() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    };

    let mut waiter = spawn(&["wait", "done"]);
    assert!(
        exits_within(&mut waiter, Duration::from_millis(500)).is_none(),
        "returned before the signal"
    );
    t.run(&["wait-for", "-S", "done"]);
    let status = exits_within(&mut waiter, TIMEOUT).expect("not released by the signal");
    assert!(status.success());

    assert!(
        t.output(&["wait-for", "-L", "lk"]).status.success(),
        "a free lock is taken at once"
    );
    let mut second = spawn(&["wait-for", "-L", "lk"]);
    assert!(
        exits_within(&mut second, Duration::from_millis(500)).is_none(),
        "took a held lock"
    );
    t.run(&["wait-for", "-U", "lk"]);
    assert!(
        exits_within(&mut second, TIMEOUT)
            .expect("not handed the lock")
            .success()
    );

    // Mid-list, the rest of the list waits for the signal.
    let mut chained = spawn(&["wait-for", "mid", ";", "set-option", "-g", "@after", "yes"]);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(t.run(&["display-message", "-p", "#{@after}"]).trim(), "");
    t.run(&["wait-for", "-S", "mid"]);
    assert!(
        exits_within(&mut chained, TIMEOUT)
            .expect("not released")
            .success()
    );
    assert_eq!(t.run(&["display-message", "-p", "#{@after}"]).trim(), "yes");

    // A bind waits too, with no command client behind it.
    let s = t.attach(&["attach", "-t", "wf"]);
    s.wait_for("status line", |text| text.contains("wf"));
    t.run(&[
        "bind-key",
        "-n",
        "F8",
        "wait-for bound ; set-option -g @bound yes",
    ]);
    s.send(b"\x1b[19~");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(t.run(&["display-message", "-p", "#{@bound}"]).trim(), "");
    t.run(&["wait-for", "-S", "bound"]);
    t.wait_run(
        &["display-message", "-p", "#{@bound}"],
        "the bind's rest",
        |o| o.trim() == "yes",
    );
}

#[test]
fn run_shell_children_get_tmxr_and_the_session_environment() {
    let t = Tmxr::new("jobenv");
    t.run(&["new-session", "-d", "-s", "je"]);
    t.run(&["set-environment", "-t", "je", "E2E_JOB", "fromsession"]);
    let var = |name: &str| {
        if cfg!(windows) {
            format!("echo %{name}%")
        } else {
            format!("echo ${name}")
        }
    };
    let show = |f: &str| {
        t.run(&["display-message", "-p", "-t", "je", f])
            .trim()
            .to_owned()
    };
    let want = format!(
        "{},{},{}",
        show("#{socket_path}"),
        show("#{pid}"),
        show("#{session_id}").trim_start_matches('$')
    );
    assert_eq!(t.run(&["run-shell", "-t", "je", &var("TMXR")]).trim(), want);
    assert_eq!(
        t.run(&["run-shell", "-t", "je", &var("E2E_JOB")]).trim(),
        "fromsession"
    );
    // So a tmxr command a job runs reaches this server, not the default one.
    let has = format!(
        "{} has-session -t je && echo found",
        env!("CARGO_BIN_EXE_tmxr")
    );
    assert_eq!(t.run(&["run-shell", "-t", "je", &has]).trim(), "found");
}

#[test]
fn run_shell_holds_its_command_list_and_reply() {
    let t = Tmxr::new("runshell");
    t.run(&["new-session", "-d", "-s", "rs"]);
    // A command client gets the output, and the commands after it wait.
    let out = t.run(&[
        "run-shell",
        "echo from-shell",
        ";",
        "display-message",
        "-p",
        "after",
    ]);
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    assert_eq!(lines, ["from-shell", "after"], "{out:?}");
    // -d delays, here with no command at all: the rest of the list waits.
    let started = Instant::now();
    t.run(&[
        "run-shell",
        "-d",
        "0.5",
        ";",
        "set-option",
        "-g",
        "@after",
        "yes",
    ]);
    assert!(
        started.elapsed() >= Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(t.run(&["display-message", "-p", "#{@after}"]).trim(), "yes");
    // -C runs a tmux command; -c sets the shell's directory.
    t.run(&["run-shell", "-C", "set-option -g @ran yes"]);
    assert_eq!(t.run(&["display-message", "-p", "#{@ran}"]).trim(), "yes");
    let dir = t.dir.path().join("there");
    std::fs::create_dir(&dir).unwrap();
    let pwd = if cfg!(windows) { "cd" } else { "pwd" };
    let shown = t.run(&["run-shell", "-c", dir.to_str().unwrap(), pwd]);
    assert!(shown.trim().ends_with("there"), "{shown:?}");
    // -b does not wait.
    let started = Instant::now();
    t.run(&["run-shell", "-b", "-d", "3", "set-option -g @late yes"]);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    // A failing command says so.
    assert!(t.run(&["run-shell", "exit 3"]).contains("returned 3"));
    // From a bind: the output shows on the client, then the rest runs.
    let s = t.attach(&["attach", "-t", "rs"]);
    s.wait_for("status line", |text| text.contains("rs"));
    t.run(&[
        "bind-key",
        "-n",
        "F7",
        "run-shell 'echo bound-out' ; set-option -g @bound yes",
    ]);
    s.send(b"[18~");
    s.wait_for("the output", |text| text.contains("bound-out"));
    t.wait_run(
        &["display-message", "-p", "#{@bound}"],
        "the rest of the bind",
        |o| o.trim() == "yes",
    );
}

#[test]
fn list_commands_take_formats_and_filters() {
    let t = Tmxr::new("lists");
    // What tmux 3.6 printed for the same commands on the same sessions.
    t.run(&["new-session", "-d", "-s", "a", "-x", "100", "-y", "30"]);
    t.run(&["new-window", "-d", "-t", "a:1", "-n", "second"]);
    t.run(&["split-window", "-h", "-t", "a:0"]);
    t.run(&["new-session", "-d", "-s", "b", "-x", "100", "-y", "30"]);
    t.run(&["set-buffer", "-b", "one", "hello world"]);
    // -x / -y are the window's size, status row not taken out.
    assert_eq!(
        t.run(&[
            "display-message",
            "-p",
            "-t",
            "b",
            "#{window_width}x#{window_height}"
        ])
        .trim(),
        "100x30"
    );
    let list = |args: &[&str]| t.run(args);
    assert_eq!(
        list(&["list-sessions", "-F", "#{session_name} #{session_windows}"]),
        "a 2\nb 1\n"
    );
    assert_eq!(
        list(&[
            "list-sessions",
            "-F",
            "#{session_name}",
            "-f",
            "#{==:#{session_name},b}"
        ]),
        "b\n"
    );
    assert_eq!(
        list(&[
            "list-windows",
            "-a",
            "-F",
            "#{session_name}:#{window_index}"
        ]),
        "a:0\na:1\nb:0\n"
    );
    assert_eq!(
        list(&[
            "list-panes",
            "-s",
            "-t",
            "a",
            "-F",
            "#{window_index}.#{pane_index}"
        ]),
        "0.0\n0.1\n1.0\n"
    );
    assert_eq!(
        list(&[
            "list-panes",
            "-a",
            "-F",
            "#{session_name}:#{window_index}.#{pane_index}",
            "-f",
            "#{pane_active}"
        ]),
        "a:0.1\na:1.0\nb:0.0\n"
    );
    // Pane edges, inclusive, and which window edges each pane touches.
    let left_width: u16 = t
        .run(&["display-message", "-p", "-t", "a:0.0", "#{pane_width}"])
        .trim()
        .parse()
        .unwrap();
    assert_eq!(
        list(&[
            "list-panes",
            "-t",
            "a:0",
            "-F",
            "#{pane_left} #{pane_top} #{pane_right} #{pane_bottom} #{pane_at_left}#{pane_at_right}#{pane_at_top}#{pane_at_bottom}"
        ]),
        format!(
            "0 0 {} 29 1011\n{} 0 99 29 0111\n",
            left_width - 1,
            left_width + 1
        )
    );
    assert_eq!(
        list(&[
            "list-buffers",
            "-F",
            "#{buffer_name} #{buffer_size} #{buffer_sample}"
        ]),
        "one 11 hello world\n"
    );
    let s = t.attach(&["attach", "-t", "b"]);
    s.wait_for("status line", |text| text.contains('b'));
    assert_eq!(
        list(&[
            "list-clients",
            "-F",
            "#{client_session} #{client_width}x#{client_height}"
        ]),
        format!("b {COLS}x{ROWS}\n")
    );
}

#[test]
fn a_brace_block_runs_as_commands() {
    let t = Tmxr::new("blocks");
    let s = t.attach(&["new", "-s", "bl"]);
    s.wait_for("status line", |text| text.contains("bl"));
    // tmux's block: both commands are if-shell's one branch.
    t.run(&[
        "bind-key",
        "-n",
        "F5",
        "if-shell -F 1 { set-option -g @a x ; set-option -g @b y }",
    ]);
    s.send(b"\x1b[15~");
    t.wait_run(&["display-message", "-p", "#{@a}#{@b}"], "both ran", |o| {
        o.trim() == "xy"
    });
}
