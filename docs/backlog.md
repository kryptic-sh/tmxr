# Backlog

Work raised and not finished, decisions still open, and gaps in what has been
verified. Delete an entry when it ships; `git log` keeps the history.

## Where the implementation stands (session of 2026-10-09)

The first implementation pass covers most of milestones M1–M7 in code; only part
of it has been exercised end to end. Each item below says what was verified and
what was not.

- **Built and unit-tested** (Windows locally; CI covers Linux/macOS):
  `tmxr-proto` (codec, golden wire bytes, endpoints, owner-only pipe DACL),
  `tmxr-command` (tokenizer, getopt flags, command table, key names, formats),
  `tmxr-config` (defaults = tmux.conf port, layering, parity test against the
  tmux binds), `tmxr-term` (PTY spawn incl. ConPTY, vt100 hooks, DA/DSR replies,
  key/paste/mouse encoding, foreground-process inspection), `tmxr-server`
  (layout geometry and navigation, ANSI backend, overlays incl. the hjkl-picker
  session picker, copy-mode motions/selection/search, resurrect save/load),
  `tmxr-client`.
- **Verified by hand on Windows** with the real binary: auto-spawned detached
  server, `new -d`, `ls`, `split-window -t`, `list-panes`, `display -p` with
  formats, `send-keys`, `capture-pane -p`, `kill-server`, resurrect
  save-on-exit + restore-on-start.
- **Verified end to end** by `apps/tmxr/tests/e2e.rs` (real binary in a ConPTY
  locally on Windows and in a pty on every CI platform; each test shown to fail
  when the code it covers is broken): attach, typing into the pane, `prefix %`
  split, `prefix h`, `prefix d` detach, `ls`, reattach with output preserved,
  command clients starting and stopping a server; the `C-l` navigator moving
  focus past a plain program and passing the key through to a program named
  `hjkl`; the `prefix s` picker by `Enter`, by typing a filter and by `j` in
  normal mode; copy mode by keyboard (`prefix [`, `?` search, `v` `E` `y`) into
  a buffer and `prefix ]` pasting it; `save-buffer` / `load-buffer`; `if-shell`
  with and without `-F`; `move-window` / `swap-window` within and across
  sessions; `join-pane` and `swap-pane -s` across windows; `respawn-pane -k`
  surviving the old program's exit; resurrect restoring a three-pane layout into
  a new server; the status line's catppuccin layout and Tokyo Night colours,
  cell by cell, including the session block turning red while the prefix is
  pending.
- **Not yet verified anywhere**: how the status line and borders look in a real
  terminal emulator (the e2e test checks the status line's cells and colours in
  the vt100 emulator, not a terminal's rendering of the glyphs), mouse, and the
  navigator with a real hjkl or vim in front (the test uses a renamed system
  program, so only the name match and the key routing are covered).
- **Not run on Linux/macOS locally**: the `cfg(unix)` arms (socket dir checks,
  peer uid, `setsid` spawn, `/proc` and `proc_pidinfo` inspection) are only
  compiled and tested by CI.
- CI (commit 7d26b7a) runs the unit and e2e tests green on Linux, macOS and
  Windows; the Unix arms are exercised there, not locally.
- **The server's pipe refuses other Windows users**: CI's `pipe-acl` job makes a
  second local account, whose `tmxr ls` on the owner's pipe fails with
  `Access is denied. (os error 5)` while the owner's succeeds. Shown to go red
  (2026-10-09) with Everyone added to the DACL.

## Known gaps and follow-ups

- **tmux command shorthand** (requested 2026-10-09: `tmux a` for
  `attach-session` and the like). The lookup already works as tmux's does
  (`tmxr_command::table::lookup`): exact name or alias first, then an
  unambiguous prefix of a full name, so `tmxr a`, `ls`, `new`, `kill-ser` and
  `splitw` resolve; unit-tested there. Every tmux alias on a list of tmux 3.x's
  aliases written from memory is present; check it against a real
  `tmux list-commands`. The gap is the tmux commands tmxr does not have, whose
  names and shorthands are therefore unknown: `customize-mode`, `display-popup`,
  `link-window`, `server-access`, `unlink-window` (`move-pane`,
  `previous-layout`, `show-window-options`, `start-server`, `set-environment`,
  `show-environment`, `clear-history`, `respawn-window` `pipe-pane` (output
  only; `-I` is an error), `show-prompt-history`, `clear-prompt-history` and
  `wait-for` (ending its command list, from a command client), `set-hook` and
  `show-hooks` (global and session hooks; `after-<command>` plus
  `client-attached`, `client-detached`, `pane-exited`, `session-closed`,
  `session-created`; no pane or window hooks) and `display-menu` (centred;
  tmux's placement and style flags are accepted and not followed),
  `lock-client`, `lock-session` and `lock-server` (`lock-command`, default
  `lock -np`; no `lock-after-time`) and `resize-window` (a manual size, undone
  with `set-window-option window-size latest`; resurrect does not save it) were
  added on 2026-10-09). Checked against a running server on 2026-10-09: `ls`,
  `list-s`, `show` (`show-options`) and `kill-ser` resolve, `displ` is ambiguous
  as in tmux, and `pipe` is an unknown command where `tmux pipe` runs
  `pipe-pane`.
- **tmux default binds**: all of tmux's defaults are bound; `prefix C-z`
  (suspend-client) errors on Windows, which has no job control. Alerts for `M-n`
  / `M-p` are bells only (no `monitor-activity` / `monitor-silence`).
  `find-window` matches window names through the picker, not pane contents or
  titles as tmux's does.
- **Mouse keys**: `SecondClick` is not recognised, and a double click sends
  `DoubleClick1…` in place of a second `MouseDown1…` (tmux sends both, the
  `DoubleClick` after a timer). The default double- and triple-click binds copy
  at once; tmux's show the selection for 0.3 s first (`run -d0.3`, which tmxr's
  `run-shell` lacks). `MouseDown1StatusLeft` / `StatusRight` / `StatusDefault`
  are not recognised either: the whole status line is `Status`.
- **No 256-colour fallback, by design for now**: colours are always 24-bit RGB,
  which is what the tmux config asks for (`terminal-overrides ",*:RGB"` forces
  RGB on every terminal). Only for a user whose terminal lacks true colour (e.g.
  macOS Terminal.app) would a fallback matter: map RGB to the nearest xterm-256
  index in `AnsiBackend` when the client's environment does not advertise true
  colour (`COLORTERM`, `WT_SESSION`, …), behind an option.
- **No frame-rate cap**: the server renders after each pass of its loop, and a
  pass stops handling events after `DRAIN_BUDGET` so a flood still repaints.
  Each pane's output waits in a bounded queue (`output::QUEUE_LIMIT`) that
  blocks the reader when full; before that queue, an unbounded channel let `seq`
  on Linux CI bury a key press under 6–9 s of output (2026-10-09).
  `detach_stays_responsive_under_flood_output` checks every CI platform detaches
  within 5 s under a flood. Frame rate itself is still not capped or measured.
- **Resurrect**: pane titles are not saved (restored programs set their own).
  Arguments are restored only for programs in `resurrect.restore-args`;
  tmux-resurrect's `~` (match anywhere in the command line) and `->` (restore a
  different command) strategies are not implemented. Arguments that are not
  UTF-8 are not saved.
- **tmux-yank `prefix y`** (copy the shell's command line) is not bound, on
  purpose for now. tmux-yank's `copy_line.sh` sends `C-a` to the shell, enters
  copy mode at the shell's cursor, selects to the end of the command and sends
  `C-e`; it works because each step is a separate `tmux` call, slow enough for
  the shell to redraw. As one tmxr command list the copy-mode snapshot would be
  taken before the shell moved its cursor. Doing it properly needs a delay
  between steps (an `if-shell "sleep 0.1"` works on Unix only) or shell
  integration (OSC 133 prompt marks) to find the command line.
- **Windows `kill()` on a pane child** returned "There are no more files" (os
  error 18) in a test; not a bug in practice. `Server::kill_pane` calls `kill()`
  and then drops the pane, closing the ConPTY, and that does end the programs
  (which of the two does it was not isolated): verified by hand on 2026-10-09
  with the real binary, both a pane running `ping -t` directly and `cmd.exe`
  running it (no `PING.EXE` left 2–3 s after `kill-pane`).
- **Windows foreground process** = newest direct child of the pane's process
  that is ≥ 0.5 s old (skips prompt helpers like starship). A program launched
  less than 0.5 s before `C-h` is not yet seen as in front.
- **Passthrough on Windows needs the bundled ConPTY.** Windows' built-in ConPTY
  forwards a program's DCS but drops its `ESC \` terminator (Windows 11 build
  26300), with or without `PSEUDOCONSOLE_PASSTHROUGH_MODE`, which it accepts and
  ignores. Microsoft's newer ConPTY keeps it, so the Windows release ships its
  `conpty.dll` and `OpenConsole.exe` (NuGet `Microsoft.Windows.Console.ConPTY`
  1.25.260930003, pinned by SHA-256 and signature-checked in
  `pkg/windows/fetch-conpty.sh`) and `emulator::passthrough_supported` turns
  passthrough on when both sit beside `tmxr.exe`. Builds from source use the
  built-in ConPTY and keep it off. CI's `test-conpty` job runs the Windows suite
  on the bundled one. Not tried with a real image protocol on the outer
  terminal, on any platform.
- **Windows `pane_current_path`** prefers what the shell announces (OSC 7 / OSC
  9;9), then reads the foreground process's directory from its PEB
  (`process::win::current_dir`, x64 offsets; tested with cmd's `cd /d`).
  PowerShell's `Set-Location` does not change its process's directory, so
  PowerShell still needs the README's prompt snippet. A 32-bit (WOW64) program's
  directory is read from its 32-bit PEB (tested with `SysWOW64\cmd.exe`). An
  elevated program's is not verified (expected to fail to open, falling back to
  the start directory).

- **The `leaky` tests had a cause, fixed on 2026-10-09**: a server started by a
  test inherited the test's own output pipe (Windows passes every inheritable
  handle on), so a server still exiting after `kill-server` held it past the
  test's end. The same leak made a script reading `tmxr new -d`'s caller's
  output wait until the server exited (a cmd pipeline took 127 s instead of 1
  s). Servers no longer inherit anything (`spawn::spawn_detached` on Windows,
  `close_inherited_fds` on Unix). If nextest reports "leaky" again, its `LEAK`
  line names the test; it would be a different cause.
- **`session_picker_previews_the_highlighted_session` timed out once** (30 s) in
  a full Windows run on 2026-10-09 that took 50 s instead of the usual 20 s, and
  passed on nextest's retry. 15 isolated runs and 5 more full runs were clean.
  Not investigated; the timeout message was not captured. It timed out once more
  on 2026-10-09, again in a slow (57 s) full run, and passed on retry; 8 full
  runs after it were clean (26 s each). Both failures came under load. The panic
  message names the `wait_for` step that timed out; next time, keep the whole
  failure output rather than filtering it.

## Decisions awaiting the owner

The provisional decisions in
[plan/16-open-questions.md](plan/16-open-questions.md) (index base, `prefix X`
kill-pane, theme, config format, vim navigation, picker mode, Windows current
directory) are defaults the implementation follows until answered.

## Release channels

Every channel of hrdr's pipeline is in CI from 2026-10-09 (`publish-crates`,
`aur-bin`, `brew-tap`, `scoop-bucket`, `alpine`), and kryptic-sh/tmxr was added
to the org secrets `AUR_SSH_KEY` and `BREW_SSH_KEY` (`SCOOP_SSH_KEY` and
`CARGO_REGISTRY_TOKEN` are shared with every org repo). crates.io's `tmxr` is
another project (crates.io owner `slaptijack`), so the app crate is `tmxr-cli`,
installing the `tmxr` binary. The kryptic.sh page (`/projects/tmxr/`) went live
the same day.

## Cross-repo work

- **hjkl `$TMXR` fall-through** is in hjkl `main` (`074ef614`, CI green on
  2026-10-09) but not in an hjkl release yet: `dispatch_tmux_navigate` runs
  `tmxr select-pane` when `$TMXR` is set. Not tested end to end with a real hjkl
  in a tmxr pane.
- **nvim navigation** is the separate
  [tmxr-navigator.nvim](https://github.com/kryptic-sh/tmxr-navigator.nvim)
  plugin (CI green on Linux, macOS and Windows, nvim stable and nightly).

## Not yet verified

- The `.deb` and `.rpm` packages are built and their contents listed in CI, but
  were never installed on a Debian or Fedora system; the macOS and Linux release
  binaries were not run outside CI's smoke step.
