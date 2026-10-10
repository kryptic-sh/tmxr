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
- **Seen in a real terminal** (2026-10-10, Windows Terminal 1.24, by
  screenshot): two panes, the vertical border and the catppuccin status line
  draw as the tests expect. hjkl's own handoff to tmxr was checked too (see
  Cross-repo work). Not looked at in any other terminal, nor with vim in front
  of the navigator.
- **Not run on Linux/macOS locally**: the `cfg(unix)` arms (socket dir checks,
  peer uid, `setsid` spawn, `/proc` and `proc_pidinfo` inspection, the client's
  `/bin/sh` lock command) are compiled and tested by CI only, which runs on
  every push to `main`.
- **The server's pipe refuses other Windows users**: CI's `pipe-acl` job makes a
  second local account, whose `tmxr ls` on the owner's pipe fails with
  `Access is denied. (os error 5)` while the owner's succeeds. Shown to go red
  (2026-10-09) with Everyone added to the DACL.

## Known gaps and follow-ups

- **Mouse use as in the owner's tmux config** (asked for 2026-10-11). The tmux
  compatibility oracle (`crates/tmxr-compat-oracle`, `corpus/mouse.toml`) drives
  a tmux 3.6 client and a tmxr client with the same SGR mouse events in CI and
  compares the results: click focus, drag to copy, double and triple click (and
  the keys two clicks send), the wheel into and out of copy mode, border drags,
  clicks and the wheel on the status line, the right-click menu and middle-click
  paste all match. Its first runs found the wheel scrolling as it entered copy
  mode and the binds missing tmux's alternate-screen and copy-mode conditions,
  fixed. Menus take the mouse as tmux's (`Menu::mouse`, from `menu_key_cb`),
  checked by the oracle's menu cases. Still missing: nothing has been tried by
  hand in a real terminal.
- **tmux default binds**: all of tmux 3.6's 267 are bound (compared with its
  `list-keys`, 2026-10-11) except the 18 digit binds (`1`-`9` in vi, `M-1`-`M-9`
  in emacs) that open a "(repeat)" prompt, declined: tmxr's copy mode takes
  counts directly (`5j`). The pane menu's hyperlink items never show, as tmxr
  keeps no OSC 8 links.
- **Scrollbars** follow tmux 3.6 (`layout_fix_panes`, the slider formula,
  `window_copy_scroll1`), checked against tmux's pane sizes. Differences: the
  three options are server-wide (tmux also sets them per window, and the style
  per pane); `MouseDragEnd1ScrollbarSlider` is never sent; and outside copy mode
  the slider reads the history size by cloning the pane's screen
  (`Emulator::history_size`), every frame while bars show, which costs with a
  long history. Not measured.
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
- **`display-popup` / `display-menu` gaps.** Placement (`-x` / `-y`, letters,
  numbers and the `popup_*` variables) is transcribed from tmux 3.6's
  `cmd_display_menu_get_pos` and checked against a running tmux
  (`popups_and_menus_are_placed_where_tmux_places_them`); tmux's `menu_*`
  variables are not defined. `-b` draws single, rounded, double, heavy or no
  lines (tmux's `simple` and `padded` are errors); `-k` and `-N` are accepted
  and ignored. Closing a running popup kills its command (`Popup`'s `Drop`, as
  `kill-pane` does). On Windows closing the ConPTY ends the program anyway, so
  removing the kill leaves the e2e test green there; on Linux and macOS it is
  what ends it: without it `display_popup_runs_a_command_over_the_panes` failed
  on both ("the popup's command outlived it"; CI run 38060138527, a throwaway
  branch, 2026-10-10).
- **tmux command shorthand** (requested 2026-10-09: `tmux a` for
  `attach-session` and the like). The lookup works as tmux's does
  (`tmxr_command::table::lookup`): exact name or alias first, then an
  unambiguous prefix of a full name; unit-tested there. Compared with a real
  tmux 3.6's `list-commands` on 2026-10-10 (WSL Ubuntu): all 90 of its commands
  are in the table and all 77 of its aliases match (the comparison was shown to
  catch a changed alias and a missing command). Checked against a running server
  on 2026-10-10: `pipe`, `custom`, `linkw`, `popup`, `resizew` and `lockc`
  resolve, and `displ` is ambiguous as in tmux.
- **Differences from tmux kept on purpose** (considered and declined):
  - A window cannot be linked twice into the same session (tmux allows it).
    Windows are found in a session by id in many places (`Session::index_of`,
    unlinking, the status line's current window); a second index for the same
    window would make each of those pick one arbitrarily. No one has asked for
    it.
  - `customize-mode` is a picker over the options and binds whose Enter puts the
    setting command in the prompt, not tmux's tree with `d` / `u` keys: the
    owner chose hjkl-style pickers, where typing always filters, so letter keys
    cannot be commands. Its `-f` is a text query, not a format filter.
- **tmux default binds**: most of tmux's defaults are bound (see above);
  `prefix C-z` (suspend-client) errors on Windows, which has no job control.
  Alerts follow tmux 3.6's, as recorded from a running tmux on 2026-10-10 (WSL
  Ubuntu, a client on a pty): flags unless a client is looking at the window,
  `*-action` and `visual-*` as tmux's, the `alert-*` hooks. Not compared:
  windows linked into two sessions (tmux flags each session's link; tmxr has one
  flag per window), and an activity alert in the window in sight rings on every
  burst of output, as tmux's did there, which was not looked at more closely.
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
- **Resurrect**: a pane's title is saved and restored, but a restored program
  that titles itself wins, and on Windows ConPTY reports cmd's own title as it
  starts, so there a saved title barely shows. Arguments are restored only for
  programs in `resurrect.restore-args`. Arguments that are not UTF-8 are not
  saved.
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
  on the bundled one. Tried with a real image protocol (2026-10-10, Windows
  Terminal 1.24, bundled ConPTY, by screenshot). Passthrough used to be sent
  from wherever the previous frame left the terminal's cursor, stale when text
  came just before the image; it now goes from the pane's cursor
  (`Emulator::take_passthrough` returns it, checked by tests). Whether that
  changed anything visible here is not established: a red sixel drew in the
  right place before the fix, and a single transparent sixel after it was not
  drawn (the frame stayed intact that time). Open: printed three in a row,
  Windows Terminal showed only one, in the last one's colour, at the first one's
  place, over a black area. The client's exact output (captured in a test)
  replayed straight into Windows Terminal drew all three correctly, so the bytes
  and their positions are right; what differs live is that frame redraws arrive
  between the images. Why Windows Terminal then loses them is a guess (text over
  image cells), not checked. tmux 3.6 (WSL Ubuntu, `allow-passthrough on`, two
  panes) in the same Windows Terminal gave the same result, its black area also
  hiding the second pane and the status line, so this is not tmxr-specific. Not
  tried in other terminals.
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
- **Handoffs to tmxr** at a program's edge: hjkl 0.42.2 (its `App::nav_step`,
  unit-tested in hjkl's suite) and nvim with
  [tmxr-navigator.nvim](https://github.com/kryptic-sh/tmxr-navigator.nvim)
  (tested in its own CI against a tmxr release). tmxr's CI job `handoff` runs
  both against the commit under test on Linux, macOS and Windows:
  `apps/tmxr/tests/handoff/hjkl.sh` and the plugin's `tests/integration.sh`, at
  the releases pinned in the job (`HJKL_VERSION`, `NAVIGATOR_VERSION`). The hjkl
  script fails with hjkl 0.42.1, which has no handoff (checked on Windows,
  2026-10-11).

## Not yet verified

- **An Escape and a mouse event in one read** (seen in CI on ubuntu,
  2026-10-11): a test sent Escape and, at once, a right-click; the client read
  `ESC ESC [<2;1;30M` as Alt+Escape and the rest as typed text, so the click
  reached the shell as `^[[<2;1;30M`. A person cannot press Escape and click
  that close together; tmux's handling of the same bytes was not compared.
- **`socket-acl` flakes**: a traced repeat on macOS (run 38062106864) was a
  `kill-server` that printed "server closed the connection": the server exited
  before its reply was written, and right after a `kill-server` a new client
  could still reach the dying server. Both fixed (`conn::flush`,
  `Server::begin_exit`); a probe of 300 start/kill rounds lost 3 replies across
  ubuntu and macOS before, none after. The first ubuntu failure
  (run 38057623410) printed the same message but also stalled about 34 s first,
  which this does not explain; untraced, as it predates `set -x`.
- The `.deb` and `.rpm` are installed and run in clean Debian and Fedora
  containers by CI's `install-packages` job on every push to `main` (x86_64
  only; the aarch64 packages are built, not installed). The macOS release
  binaries and the `.apk` were not run outside CI's smoke step.
