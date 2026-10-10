# Backlog

Work raised and not finished, decisions still open, and gaps in what has been
verified. Delete an entry when it ships; `git log` keeps the history.

## Where the implementation stands (2026-10-10)

Milestones M1–M7 are in code, and every tmux 3.x command is in the command
table. What has been verified, and how:

- **Unit tests** in every crate (Windows locally; CI on Linux and macOS too):
  `tmxr-proto` (codec, golden wire bytes, endpoints, owner-only pipe DACL),
  `tmxr-command` (tokenizer, getopt flags, command table, key names, formats),
  `tmxr-config` (defaults = tmux.conf port, layering, parity with the tmux
  binds), `tmxr-term` (PTY spawn incl. ConPTY, vt100 hooks, key/paste/mouse
  encoding, foreground-process inspection), `tmxr-server` (layout, ANSI backend
  and colour mapping, overlays, menus, copy mode, resurrect), and `tmxr-client`.
- **End to end** by `apps/tmxr/tests/e2e.rs`: the real binary in a ConPTY
  locally and in a pty on every CI platform, each test shown to fail when the
  code it covers is broken. It drives attaching, typing, splits, navigation, the
  pickers, copy mode (vi and emacs), buffers, mouse clicks, drags and the wheel,
  every command family including hooks, menus, popups, locking, `run-shell` /
  `if-shell` ordering and linked windows, resurrect, and the status line's cells
  and colours. The test names say what each covers.
- **Not yet verified anywhere**: how the status line and borders look in a real
  terminal emulator (the tests check cells and colours in a vt100 emulator, not
  a terminal's rendering of the glyphs), and the navigator with a real hjkl or
  vim in front (the test uses a renamed system program, so only the name match
  and the key routing are covered).
- **Not run on Linux/macOS locally**: the `cfg(unix)` arms (socket dir checks,
  peer uid, `setsid` spawn, `/proc` and `proc_pidinfo` inspection, the client's
  `/bin/sh` lock command) are compiled and tested by CI only, which runs on
  every push to `main`.
- **The server's pipe refuses other Windows users**: CI's `pipe-acl` job makes a
  second local account, whose `tmxr ls` on the owner's pipe fails with
  `Access is denied. (os error 5)` while the owner's succeeds. Shown to go red
  (2026-10-09) with Everyone added to the DACL.

## Known gaps and follow-ups

- **Windows: an Enter sent before the shell's first read can stall**
  (2026-10-10). Probing with the real binary, `new-window -d` then at once
  `send-keys 'echo markN' Enter`, about 1 in 100 cmd panes showed the text but
  never ran it; one more Enter then ran the line and gave an extra empty prompt,
  so the first Enter had reached ConPTY and was held there until more input
  arrived. Rates varied with load (later rounds of 240 showed none, with the
  keys in one write or two), so the cause is not pinned beyond "ConPTY, before
  the shell reads". The e2e tests that type into a new pane wait for its prompt
  first (`Tmxr::wait_prompt`);
  `environment_commands_set_show_and_reach_new_panes` failed this way once.
- **`a_32_bit_programs_directory_is_read` failed once** (2026-10-10, CI's
  bundled-ConPTY Windows job, run 38044583669): the 32-bit `cmd`'s directory
  read as `C:\Windows\System32` for the whole 20 s. It passed 25 runs in a row
  alone locally. Probable cause, found and fixed the same day: `newest_child`
  took any process naming the pane's pid as its parent, and Windows keeps a dead
  parent's pid in its children while pids are reused, so under a busy parallel
  run an orphan of an older process (a console host, which runs in `System32`)
  could pass for the pane's child. Children created before the pane's process
  are now skipped (`pick_child`, unit-tested). Not confirmed as this failure's
  cause; if it recurs, log which process `current_dir` read.
- **`server-access` coverage**: a second user is exercised by CI on Windows
  (`pipe-acl`), Linux and macOS (`socket-acl`), each shown to fail with
  admission broken. `socket-access` is read at start only (see
  plan/03-protocol.md for why the pipe cannot change). A Windows peer is
  identified through its process, so a client whose process exits between
  connecting and being checked is refused. A label's socket on Unix sits in
  tmxr's private directory, unreachable by other users whatever `socket-access`
  says; `server-access -a` then says to use `-S`, as tmux documents.
- **`display-popup` gaps.** `-x` / `-y` take a number or `C` (tmux's other
  position forms and formats are errors); `-b` draws single, rounded, double,
  heavy or no lines (tmux's `simple` and `padded` are errors); `-k` and `-N` are
  accepted and ignored. A menu's `-O` and `-x` / `-y` are accepted and not
  followed: it is centred. Closing a running popup kills its command (`Popup`'s
  `Drop`, as `kill-pane` does); removing that kill does not turn the e2e test
  red on Windows, because closing the ConPTY ends the program anyway. Whether
  the explicit kill is what ends it on Unix (where the PTY reader thread holds
  its own copy of the master) is not verified.
- **tmux command shorthand** (requested 2026-10-09: `tmux a` for
  `attach-session` and the like). The lookup works as tmux's does
  (`tmxr_command::table::lookup`): exact name or alias first, then an
  unambiguous prefix of a full name; unit-tested there. Every tmux alias on a
  list of tmux 3.x's aliases written from memory is present; check it against a
  real `tmux list-commands`. Every tmux 3.x command is in the table. Checked
  against a running server on 2026-10-10: `pipe`, `custom`, `linkw`, `popup`,
  `resizew` and `lockc` resolve, and `displ` is ambiguous as in tmux.
- **Partial tmux commands**: `wait-for` must end its command list and come from
  a command client; hooks are global or per session (no pane or window hooks); a
  window cannot be linked twice into the same session (tmux allows it);
  `customize-mode` is a picker over the options and binds whose Enter puts the
  setting command in the prompt, without tmux's tree, per-scope options or `d` /
  `u` keys, and its `-f` is a text query, not a format filter.
- **tmux default binds**: all of tmux's defaults are bound; `prefix C-z`
  (suspend-client) errors on Windows, which has no job control. Alerts for `M-n`
  / `M-p` are bells, `monitor-activity` and `monitor-silence`; tmux's
  `visual-activity` / `visual-bell` / `visual-silence` messages and
  `activity-action` / `bell-action` / `silence-action` are not implemented
  (alerts only set window flags).
- **Click sequence not checked against tmux**: tmxr sends `MouseDown1…` for the
  first press, `SecondClick1…` (only) for a second press in the same cell within
  the click time, `TripleClick1…` for a third, and `DoubleClick1…` from a timer
  when no third press came. That is tmux 3.3's `server_client_check_mouse` as
  recalled, not compared with a running tmux: if tmux also sends `MouseDown1…`
  on the second press, binds on it would fire once fewer here. The default
  double- and triple-click binds then show the selection for 0.3 s
  (`run-shell -d 0.3`) before copying, as tmux's do.
- **`rgb-colour auto`** decides from the client's environment only:
  `COLORTERM=truecolor` / `24bit`, a `TERM` ending in `-direct`, or
  `WT_SESSION`. Terminals that take 24-bit colour without saying so (some
  `TERM_PROGRAM`s) get the 256 under `auto`; the default `on` sends 24-bit to
  every terminal, as the tmux config's `terminal-overrides ",*:RGB"` does. There
  is no 16-colour fallback.
- **No frame-rate cap**: the server renders after each pass of its loop, and a
  pass stops handling events after `DRAIN_BUDGET` so a flood still repaints.
  Each pane's output waits in a bounded queue (`output::QUEUE_LIMIT`) that
  blocks the reader when full; before that queue, an unbounded channel let `seq`
  on Linux CI bury a key press under 6–9 s of output (2026-10-09).
  `detach_stays_responsive_under_flood_output` checks every CI platform detaches
  within 5 s under a flood. Frame rate itself is still not capped or measured.
- **Resurrect**: pane titles are not saved (restored programs set their own).
  Arguments are restored only for programs in `resurrect.restore-args`.
  Arguments that are not UTF-8 are not saved.
- **`prefix y` (`copy-command-line`) without prompt marks relies on the shell's
  line keys**: a shell that sends OSC 133 prompt marks needs no keys, but one
  that does not, in vi mode or bound differently, does not move to the line's
  ends on `C-a` / `C-e` (Home / End on Windows), and the copy is then wrong, as
  with tmux-yank's. That path waits for the pane to settle, but a shell slower
  than `copyline::STEP_LIMIT` to redraw is read early. A mark is dropped once
  the history is full (`Emulator::input_start`), since lines then scroll away
  uncounted. Tested end to end with cmd (Windows; ConPTY passes OSC 133 through)
  and bash (CI), with and without marks; not PowerShell, zsh or fish.
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
- **`session_picker_previews_the_highlighted_session` timed out now and then**
  (twice recorded, both in slow full runs). Probable cause, found 2026-10-10: it
  typed `echo preview-marker` into a session made a moment before, the ConPTY
  Enter stall above, so the marker never printed. It now waits for the prompt
  first, as do the other tests that type into a new pane; 10 full runs without
  retries afterwards had no failure, against 2 failures in the 16 runs before.
  If it fails again, the stall is not the whole story.
- **hjkl `$TMXR` fall-through** is in hjkl `main` (`074ef614`, CI green on
  2026-10-09) but not in an hjkl release yet: `dispatch_tmux_navigate` runs
  `tmxr select-pane` when `$TMXR` is set. Not tested end to end with a real hjkl
  in a tmxr pane.
- **nvim navigation** is the separate
  [tmxr-navigator.nvim](https://github.com/kryptic-sh/tmxr-navigator.nvim)
  plugin (CI green on Linux, macOS and Windows, nvim stable and nightly).

## Not yet verified

- The `.deb` and `.rpm` are installed and run in clean Debian and Fedora
  containers by CI's `install-packages` job on every push to `main` (x86_64
  only; the aarch64 packages are built, not installed). The macOS release
  binaries and the `.apk` were not run outside CI's smoke step.
