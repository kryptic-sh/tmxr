# 12 — Platforms

Linux, macOS and Windows are all tier 1: CI runs the full test suite on each,
and no feature ships as "Unix only" without the Windows side either implemented
or failing loudly with a clear message.

| Concern             | Linux                                                                                                                                                                                               | macOS                     | Windows                                                                                                                                                                                                                                                                                                                   |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| PTY                 | openpty (portable-pty)                                                                                                                                                                              | openpty (portable-pty)    | ConPTY (portable-pty); Windows 10 1809+                                                                                                                                                                                                                                                                                   |
| IPC                 | Unix domain socket                                                                                                                                                                                  | Unix domain socket        | Named pipe                                                                                                                                                                                                                                                                                                                |
| Socket protection   | `0700` dir + `SO_PEERCRED`                                                                                                                                                                          | `0700` dir + `getpeereid` | pipe DACL for the current user                                                                                                                                                                                                                                                                                            |
| Daemonise           | `setsid` in `pre_exec`                                                                                                                                                                              | `setsid` in `pre_exec`    | `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP`, no console                                                                                                                                                                                                                                                                |
| Default shell       | `$SHELL` → `/bin/sh`                                                                                                                                                                                | `$SHELL` → `/bin/zsh`     | `pwsh.exe` → `powershell.exe` → `%COMSPEC%`                                                                                                                                                                                                                                                                               |
| Foreground command  | `tcgetpgrp` + `/proc/*/comm`                                                                                                                                                                        | `tcgetpgrp` + `proc_name` | Toolhelp snapshot, deepest descendant of the pane child                                                                                                                                                                                                                                                                   |
| Current directory   | `/proc/*/cwd`                                                                                                                                                                                       | `proc_pidinfo` vnode path | OSC 7 / OSC 9;9 from the shell prompt (gap, see 04)                                                                                                                                                                                                                                                                       |
| Client terminal I/O | ttyr (stdin bytes)                                                                                                                                                                                  | ttyr (stdin bytes)        | ttyr: console key records, or under Windows Terminal (`WT_SESSION`) VT input with win32-input-mode; VT output                                                                                                                                                                                                             |
| Signals / shutdown  | SIGTERM/SIGHUP → save + exit                                                                                                                                                                        | same                      | `kill-server` / console-close of client only                                                                                                                                                                                                                                                                              |
| Keys                | the kitty keyboard protocol where the terminal answers its query; else xterm's modifyOtherKeys where its XTVERSION or DA2 name, or `terminal-features`, gives it tmux's `extkeys`; legacy otherwise | same                      | console key records (releases, AltGr, Ctrl+4 and the keypad kept), in Windows Terminal as its win32-input-mode; the tier as the probe finds it under Windows Terminal, legacy elsewhere (no probe; no modifyOtherKeys, which the client tells the server); C-h / C-j fixed up both ways outside Windows Terminal (see 06) |

Notes:

- **Windows constants** (`DETACHED_PROCESS = 0x00000008`,
  `CREATE_NEW_PROCESS_GROUP = 0x00000200`) are spelled out locally rather than
  imported, per the house rule for ABI values that moved between `windows-sys`
  versions.
- **ConPTY quirks** to plan for: ConPTY re-renders and can reorder/merge output
  sequences, swallows some DEC private modes, and needs the output pipe drained
  continuously or the child blocks. The reader thread per pane already drains
  continuously; e2e tests on Windows assert on screen contents, not on exact
  byte streams.
- **Child processes** (`run-shell`, `if-shell`, `copy-pipe`, `pipe-pane`, the
  lock command) are made by `tmxr_term::child::command`, which marks them
  `CREATE_NO_WINDOW` on Windows when the process starting them has no console:
  the detached server has none, and without the flag every shell command it ran
  opened a console window. `clippy.toml` disallows `Command::new` so a new spawn
  goes through it.
- **The Windows client's input** (plan 18 step 8): under Windows Terminal
  (`WT_SESSION` set, `TMXR` not) the client turns on the console's VT input and
  Windows Terminal's win32-input-mode, so keys arrive as console key records
  encoded in VT (`KeyEncoding::Windows`) and everything else (the mouse, the
  probe's answers, colour reports) as the terminal sends it; the probe runs
  there, so the tier and colours are what the terminal says. Elsewhere (conhost,
  ConPTY behind Alacritty or VS Code) it reads the console's key records, asks
  nothing (answers would arrive as typed keys), and reads a terminal's C-j and
  C-h bytes, which the console reports as Ctrl+Enter and Ctrl+Backspace, as tmux
  reads them. A console key record's release is real in both, so the server
  holds its press for it rather than making one. Not yet run on a real Windows
  machine: see the backlog's "To check by hand on Windows".
- **Windows Terminal** passes OSC 52 and true colour; legacy `conhost` does not
  do OSC 52 — the local clipboard path (`hjkl-clipboard`) covers it.
- Platform-specific code lives behind `#[cfg]` in `tmxr-term::process` and
  `tmxr-client::spawn`, each with every platform's arm implemented and each arm
  covered by a test that runs on that platform in CI.
