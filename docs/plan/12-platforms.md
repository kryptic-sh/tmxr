# 12 — Platforms

Linux, macOS and Windows are all tier 1: CI runs the full test suite on each,
and no feature ships as "Unix only" without the Windows side either implemented
or failing loudly with a clear message.

| Concern             | Linux                                                                           | macOS                     | Windows                                                        |
| ------------------- | ------------------------------------------------------------------------------- | ------------------------- | -------------------------------------------------------------- |
| PTY                 | openpty (portable-pty)                                                          | openpty (portable-pty)    | ConPTY (portable-pty); Windows 10 1809+                        |
| IPC                 | Unix domain socket                                                              | Unix domain socket        | Named pipe                                                     |
| Socket protection   | `0700` dir + `SO_PEERCRED`                                                      | `0700` dir + `getpeereid` | pipe DACL for the current user                                 |
| Daemonise           | `setsid` in `pre_exec`                                                          | `setsid` in `pre_exec`    | `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP`, no console     |
| Default shell       | `$SHELL` → `/bin/sh`                                                            | `$SHELL` → `/bin/zsh`     | `pwsh.exe` → `powershell.exe` → `%COMSPEC%`                    |
| Foreground command  | `tcgetpgrp` + `/proc/*/comm`                                                    | `tcgetpgrp` + `proc_name` | Toolhelp snapshot, deepest descendant of the pane child        |
| Current directory   | `/proc/*/cwd`                                                                   | `proc_pidinfo` vnode path | OSC 7 / OSC 9;9 from the shell prompt (gap, see 04)            |
| Client terminal I/O | crossterm (stdin bytes)                                                         | crossterm (stdin bytes)   | crossterm (console input records + VT output)                  |
| Signals / shutdown  | SIGTERM/SIGHUP → save + exit                                                    | same                      | `kill-server` / console-close of client only                   |
| Keys                | kitty keyboard via `hjkl-kitty` when the terminal supports it; legacy otherwise | same                      | Windows console reports modifiers natively (`C-h` ≠ Backspace) |

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
- **Windows Terminal** passes OSC 52 and true colour; legacy `conhost` does not
  do OSC 52 — the local clipboard path (`hjkl-clipboard`) covers it.
- Platform-specific code lives behind `#[cfg]` in `tmxr-term::process` and
  `tmxr-client::spawn`, each with every platform's arm implemented and each arm
  covered by a test that runs on that platform in CI.
