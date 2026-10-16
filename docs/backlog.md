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
- **Not run on macOS locally**: the `cfg(unix)` arms (socket dir checks, peer
  uid, `setsid` spawn, `/proc` and `proc_pidinfo` inspection, the client's
  `/bin/sh` lock command) are tested by CI on every push to `main`. Linux ones
  can be run here too: WSL Ubuntu has cargo and tmux 3.6 (a clone in WSL's own
  filesystem builds fast); `detach_client_p_hangs_up_the_clients_parent` was
  shown to fail there with `hang_up_parent` doing nothing (2026-10-16).
- **The server's pipe refuses other Windows users**: CI's `pipe-acl` job makes a
  second local account, whose `tmxr ls` on the owner's pipe fails with
  `Access is denied. (os error 5)` while the owner's succeeds. Shown to go red
  (2026-10-09) with Everyone added to the DACL.

## Decisions waiting on the owner

- **OSC 8 hyperlinks** (tmux keeps a pane's links: the pane menu's link items,
  `#{mouse_hyperlink}`). tmxr's screen is the `vt100` crate's, which drops OSC
  8, so a link never reaches tmxr. Options: fork `vt100` (as crossterm is
  patched, `[patch.crates-io]` on a kryptic-sh branch) to keep a cell's link; or
  move to tmxr's own grid, a much larger change. Both are a dependency decision;
  nothing was started.
- **A pane made taller again gets blank rows** (see below): the same choice, a
  `vt100` fork that can move history rows back onto the screen, or tmxr's own
  grid.
- **Fuzzing the command parser and the frame codec** (`docs/plan/14`, a weekly
  cron job): needs `cargo-fuzz` and `libfuzzer-sys`, new dependencies on a
  nightly toolchain, or `proptest`-style property tests on stable, also new.

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
- **crossterm is patched for Alt+Escape** until a release carries the fix:
  crossterm 0.29's Unix parser read `ESC ESC` as a plain Escape, so Alt+Escape
  lost its Alt. The fix is upstream as crossterm-rs/crossterm#1143 (branch
  `fix-alt-escape` of kryptic-sh/crossterm); tmxr builds against a backport to
  0.29.0 (branch `alt-escape-0.29`, pinned by `rev` in the root `Cargo.toml`'s
  `[patch.crates-io]`, and allowed in `deny.toml`). Once a crossterm release has
  it, upgrade and drop both. A `[patch]` applies only to builds from this
  workspace: the release binaries and packages have the fix, but
  `cargo install tmxr` from crates.io builds against crates.io's crossterm
  without it. Still different from tmux: an Escape and a mouse event in one read
  (`known_divergences.toml`), as crossterm drops the rest of a read once it has
  an event.
- **tmux default binds**: all of tmux 3.6's 267 are bound (compared with its
  `list-keys`, 2026-10-11) except the 18 digit binds (`1`-`9` in vi, `M-1`-`M-9`
  in emacs) that open a "(repeat)" prompt, declined: tmxr's copy mode takes
  counts directly (`5j`). The pane menu's hyperlink items never show, as tmxr
  keeps no OSC 8 links.
- **Scrollbars** follow tmux 3.6 (`layout_fix_panes`, the slider formula,
  `window_copy_scroll1`), checked against tmux's pane sizes.
- **Option scopes** follow tmux 3.6: global, per session, per window and per
  pane (`cmds::scoped`, checked against a running tmux by
  `corpus/options.toml`). Still different:
  - **The global user options are one table**: tmux keeps `set -g @x` (global
    session options) and `set -wg @x` (global window options) apart, so its
    `show -g @x` does not see one set with `-wg`, and in formats a global window
    `@x` is found before a session's own; tmxr has one global table, found last.
  - **Fewer server options**: `show -s` lists the ones tmxr has
    (`cmds::options::SERVER_OPTIONS`); `update-environment` stays a config
    value, not settable by `set-option` and not per session.
  - **One status-line timer**: the status lines redraw at the shortest
    `status-interval` any session has, where tmux times each session's.
  - **A linked window reads the first session holding it** for its session
    options (`Server::window_session_options`: `default-shell` for its panes,
    say); alerts do read each session's own `*-action` and `visual-*`.
  - **Resurrect does not save session options** (tmux-resurrect does not
    either): a restored session has only the global ones.
  - **Hooks in `show-options -H` and `show-hooks`** (`scoped::show_hooks`,
    checked against tmux 3.6 by `corpus/options.toml`): only hooks that are set
    are listed, where tmux lists every hook it has, the empty ones as a bare
    name (so `show -gH` and `show -AH` differ from tmux's by those lines). A
    hook's commands print as they were given (`display hi`), where tmux prints
    them parsed (`display-message hi`).
  - **`show-options` quoting is tmux's, quirk included** (`escape_arg`, from
    `args_escape`): a value with a `"` and a backslash but none of ` #';${}%` is
    single-quoted with the backslash doubled, which reads back doubled, in tmux
    as in tmxr. Kept to match tmux's output. Octal escapes that are not valid
    UTF-8 become U+FFFD when read (`tokenize::escape`); tmux keeps the raw
    bytes.
  - **Synchronized input skips panes as tmux's does** (`input_targets`, from
    `window_pane_copy_key`): in a mode, dead, or hidden by a zoom. The copy mode
    and zoom cases were compared with tmux 3.6 (2026-10-14); they and the
    clock-mode case are tested end to end, each shown to fail without its check.
    The dead-pane case is not tested: a dead pane has no program to read input,
    so nothing shows whether it was sent.
- **A pane made taller again gets blank rows at the bottom**: tmux 3.6
  (`screen_resize_y`) first takes back the rows a shrink pushed into the history
  (`hscrolled`), so a split then closed restores the screen; tmxr's
  `Emulator::resize` cannot, as vt100 has no way to move history rows back onto
  the screen. Shrinking does follow tmux (it scrolls the rows off with CSI S,
  which also means a program's own scroll region, if set, scrolls instead of the
  whole screen). Fixing growth needs a vt100 change (a fork, as with crossterm)
  or tmxr's own grid.
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
  (`popups_and_menus_are_placed_where_tmux_places_them`). Boxes are drawn as
  tmux 3.6's `screen_write_box` in each of its `-b` line types (checked against
  a running tmux, 2026-10-11). (tmux 3.6 defines no `menu_*` format variables;
  an earlier entry here said it did.) `-k`, `-N` and a `display-popup` on an
  open popup follow tmux 3.6 (`popup_key_cb`, `popup_modify`). Closing a running
  popup kills its command (`Popup`'s `Drop`, as `kill-pane` does). On Windows
  closing the ConPTY ends the program anyway, so removing the kill leaves the
  e2e test green there; on Linux and macOS it is what ends it: without it
  `display_popup_runs_a_command_over_the_panes` failed on both ("the popup's
  command outlived it"; CI run 38060138527, a throwaway branch, 2026-10-10).
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
  - Session names may hold dots (asked for 2026-10-12): tmux 3.6 turns a dot in
    `new-session -s` / `rename-session` into `_`. A target without `:` that
    exactly names a dotted session is that session (`target::names_session`); to
    mean window `a`, pane `b` of the current session while a session is named
    `a.b`, write `:a.b`.
- **Ctrl+Enter / Ctrl+Backspace on Windows outside Windows Terminal** become C-j
  / C-h (`terminal::console_key`, 2026-10-12): the console reports the bytes a
  terminal sends for C-j and C-h as those chords, so they cannot be told apart.
  Whether keys come as bytes is guessed from `WT_SESSION`, so a terminal started
  from a Windows Terminal shell inherits it and keeps C-j / C-h broken. Checked
  through ConPTY in the e2e tests (raw bytes and win32-input-mode events); not
  tried by hand in Alacritty or VS Code. Fixing it at the source needs the key
  event's character, which crossterm drops for Enter and Backspace.
- **Attaching differs from tmux in what is left** (`cmds::session::attach_to`,
  2026-10-14): `-f` takes `read-only` only and refuses tmux's other client flags
  by name.
- **tmux default binds**: most of tmux's defaults are bound (see above);
  `prefix C-z` (suspend-client) errors on Windows, which has no job control.
  Alerts follow tmux 3.6's, as recorded from a running tmux on 2026-10-10 (WSL
  Ubuntu, a client on a pty): flags unless a client is looking at the window,
  `*-action` and `visual-*` as tmux's, the `alert-*` hooks. Not compared:
  windows linked into two sessions (tmux flags each session's link; tmxr has one
  flag per window), and an activity alert in the window in sight rings on every
  burst of output, as tmux's did there, which was not looked at more closely.
- **`rgb-colour auto`** decides from the client's environment only, not from
  terminfo as tmux does: `COLORTERM=truecolor` / `24bit`, a `TERM` ending in
  `-direct`, or `WT_SESSION` give 24-bit colour; a `TERM` without `256col` in it
  gives the 16 (`Server::client_depth`), so `TERM=xterm` gets the 16 as in tmux,
  even on a terminal that takes more. Terminals that take 24-bit colour without
  saying so (some `TERM_PROGRAM`s) get the 256 under `auto`; the default `on`
  sends 24-bit to every terminal, as the tmux config's
  `terminal-overrides ",*:RGB"` does. The 16 are sent as `90`–`97` for the
  bright ones; there is no 8-colour mode (bold for bright, as tmux uses on a
  terminal with 8).
- **No frame-rate cap**: the server renders after each pass of its loop, and a
  pass stops handling events after `DRAIN_BUDGET` so a flood still repaints.
  Each pane's output waits in a bounded queue (`output::QUEUE_LIMIT`) that
  blocks the reader when full; before that queue, an unbounded channel let `seq`
  on Linux CI bury a key press under 6–9 s of output (2026-10-09).
  `detach_stays_responsive_under_flood_output` checks every CI platform detaches
  within 5 s under a flood. Frame rate itself is not capped or measured;
  considered and not done (2026-10-14): tmux has no cap either (it redraws once
  per pass of its event loop, as tmxr does), and nothing has shown a need. A cap
  is worth adding only with a measurement showing frames wasted, for example the
  bytes sent to a client under a flood against what it can show.
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
- **`if_shell_picks_a_command_by_status_or_format` failed once** (2026-10-14, a
  full local Windows run): the
  `if-shell 'exit 0' "run-shell 'echo deep'" ; display -p after` step did not
  give `["deep", "after"]`. Its output was not captured; it passed 5 runs alone
  and 2 full runs right after. If it recurs, keep the `out` the assertion
  prints.
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

- **Console windows across the org's test suites** (2026-10-14): every Rust
  repo's `cargo test --workspace` was run on Windows from a shell with a
  console, with a watcher logging each new console or Windows Terminal window
  (polled every 100 ms, so a window shorter than that could be missed). Only
  tmxr's opened any (the server's `run-shell` children, fixed by
  `tmxr_term::child`); gpur, hjkl, hodl, inbx, infr, krypt, langr, lyr, pikr,
  sqeel and hrdr opened none, hrdr's `Start-Process -WindowStyle Hidden` tests
  included. Not checked, as their tests did not run here: buffr (its CEF build
  script failed), crcbl (the linker ran out of memory, LNK1102), tikr (its tests
  do not find `tikr_paper` / `tikr_backtest`) and wayr (Wayland only). sqeel's
  `sqeel-config` has two tests failing on Windows
  (`connection_filesystem_roundtrip`,
  `inline_password_roundtrip_percent_decodes`); not looked into. A process run
  without a console (a GUI, a scheduled task) would give their plain spawns
  windows; crcbl's `crcbl_spawn::command` and tmxr's `tmxr_term::child::command`
  are the pattern for that.

- **`socket-acl` flakes**: a traced repeat on macOS (run 38062106864) was a
  `kill-server` that printed "server closed the connection": the server exited
  before its reply was written, and right after a `kill-server` a new client
  could still reach the dying server. Both fixed (`conn::flush`,
  `Server::begin_exit`); a probe of 300 start/kill rounds lost 3 replies across
  ubuntu and macOS before, none after. The first ubuntu failure
  (run 38057623410) printed the same message but also stalled about 34 s first,
  which this does not explain; untraced, as it predates `set -x`.
- The `.deb` and `.rpm` are installed and run in clean Debian and Fedora
  containers by CI's `install-packages` job on every push to `main`, x86_64 and
  aarch64 (on an Arm runner). The macOS release binaries and the `.apk` were not
  run outside CI's smoke step.
