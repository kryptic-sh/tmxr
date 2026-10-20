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
- **End to end** by `apps/tmxr/tests/e2e/` (harness in `main.rs`, tests by area
  in its modules): the real binary in a ConPTY locally and in a pty on every CI
  platform, each test shown to fail when the code it covers is broken. It drives
  attaching, typing, splits, navigation, the pickers, copy mode (vi and emacs),
  buffers, mouse clicks, drags and the wheel, every command family including
  hooks, menus, popups, locking, `run-shell` / `if-shell` ordering and linked
  windows, resurrect, and the status line's cells and colours. The test names
  say what each covers.
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
- **`detach-on-destroy`** (`Server::kill_session`): tmxr always moves a client
  whose session is destroyed to the most recently used other session, tmux's
  `detach-on-destroy off`; tmux's default is `on`, which detaches it. Options:
  add the session option with tmux's values (`on`, `off`, `no-detached`,
  `previous`, `next`) defaulting to `on` as tmux does (a change for whoever
  relies on staying attached), or defaulting to `off` as tmxr behaves now
  (matches tmux only where a config sets it off). Either makes
  `-f no-detach-on-destroy` possible.
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
  - **Global window user options exist at run time only**
    (`Config::window_user`): the config file's `[options]` are the global
    session ones, so `--import-tmux-conf` puts a `set -wg @x` line there, and
    `customize-mode` lists only the session ones (its Enter writes
    `set-option -g`, which would set the other table).
  - **Fewer server options**: `show -s` lists the ones tmxr has
    (`cmds::options::SERVER_OPTIONS`).
  - **Array options** (`tmxr_config::Array`, checked against tmux 3.6 by
    `corpus/options.toml`): `update-environment` is the only one; tmux's others
    (`command-alias`, `terminal-features`, `status-format`, `user-keys`,
    `pane-colours`, `codepoint-widths`) are options tmxr does not have. The
    config file keeps it as a list, so `--import-tmux-conf` leaves out a
    `set -g update-environment[3] X` line (an unknown key there), and
    `customize-mode` shows it as one value, all items joined, whose Enter sets
    the whole array. `set -g x[i] value` on an option that is not an array
    errors as tmux's (`not an array`), but a bad index errors as
    `invalid option`, where tmux says `ambiguous option`.
  - **Pane commands refresh on one timer**: each client's status line redraws at
    its session's `status-interval`, as tmux's, but the foreground commands
    behind `#{pane_current_command}` and automatic window names refresh at the
    shortest one any session has, at least each second (`Server::tick`); tmux
    renames on its own timer, not the status line's.
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
  - **Notification hooks follow a change, not the call that made it**
    (`notify::Watched`): after each event (and each tick) the server compares
    session and window names, each session's current window and windows, each
    window's active pane, layout and size, each pane's mode and title and each
    client's session and size with what it saw last, and queues
    `session-renamed`, `session-window-changed`, `window-linked` / `-unlinked`,
    `window-renamed`, `window-pane-changed`, `window-layout-changed`,
    `window-resized`, `pane-mode-changed`, `pane-title-changed`,
    `client-session-changed` and `client-resized`. So a hook fires once per
    event however many times the thing changed in it (`select-layout` fires
    `window-layout-changed` twice in tmux 3.6, once in tmxr), and a change made
    by a hook's own commands fires nothing. Which action fires which hook was
    compared with tmux 3.6 on 2026-10-18
    (`state_changes_fire_tmuxs_notification_hooks`). `command-error` runs for a
    command that fails as it runs, from a command client too, and not for one
    that does not parse (tmux 3.6, 2026-10-18: `select-window` and `kill-pane`
    at a missing target each fired it once, an unknown command none); an earlier
    note here said tmux fired none for a client's failed command line, which
    that probe contradicts.
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
  - `join-pane -p` (and `move-pane -p`) is not taken: tmux 3.6's reads its value
    from `-l` (`cmd_join_pane_exec` calls
    `args_strtonum_and_expand(args, 'l', ...)` under `-p`), so `-p` alone fails
    there and with `-l` it is `-l`. tmxr takes `-l` (`30%` for a percentage),
    and `split-window -p` works.
  - A window cannot be linked twice into the same session (tmux allows it).
    Windows are found in a session by id in many places (`Session::index_of`,
    unlinking, the status line's current window); a second index for the same
    window would make each of those pick one arbitrarily. No one has asked for
    it.
  - `customize-mode` is a picker over the options and binds whose Enter puts the
    setting command in the prompt, not tmux's tree with `d` / `u` keys: it was
    built when typing always filtered. Pickers now open in a normal mode
    (2026-10-17), so tmux's letter keys could be added there; not done, and no
    one has asked. Its `-f` is a text query, not a format filter.
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
- **Client flags** (`cmds::session::client_flags`): `-f` takes `read-only` and
  `ignore-size`, and refuses tmux's others by name: `no-output`, `pause-after`
  and `wait-exit` are control-mode flags, and tmxr has no control mode;
  `active-pane` needs a per-client active pane; `no-detach-on-destroy` needs
  `detach-on-destroy`, below. `!flag` and `refresh-client -f` / `-F` work as
  tmux's (`!read-only` leaves a read-only client read-only). `#{client_flags}`
  lists only what tmxr keeps: `attached`, `focused`, `ignore-size` and
  `read-only`, not tmux's `suspended` or `UTF-8` (tmxr does not track either). A
  client is `focused` from attaching, as tmux's, until its terminal reports
  focus out; the terminal is asked for focus reports only with
  `focus-events on`, as tmux's (`ServerMsg::FocusEvents`), and reports that come
  anyway are taken, as tmux's `tty_keys_next` takes them. On Windows the console
  hands crossterm focus events whether asked or not, and ConPTY asks its own
  terminal for them when it starts, so `focus-events off` does not keep them out
  there. Not compared with a running tmux: the oracle has no attached client.
- **Pane focus** (`Server::update_pane_focus`, from tmux's
  `window_pane_update_focus`): a pane is focused while it is the active pane of
  a window a focused client shows, and a program that asked (mode 1004) gets
  `CSI I` / `CSI O` when that changes, with `pane-focus-in` / `-out`. tmux
  counts only a popup or a menu as covering the pane; tmxr's pickers are
  overlays too (tmux's are pane modes) but leave the pane focused, as tmux's
  would. Checked end to end on Windows and Linux
  (`a_pane_that_asked_is_told_when_it_gains_and_loses_focus`); a client's own
  focus out reaching the pane is not tested. The test's fixture reads the
  reports as raw bytes on Unix (`log_focus_bytes`): crossterm's reader lost the
  first one in about half of the Linux runs although tmxr had written it (seen
  with a log line in `update_pane_focus`). Why crossterm loses it was not found.
- **tmux default binds**: most of tmux's defaults are bound (see above);
  `prefix C-z` (suspend-client) errors on Windows, which has no job control.
  Alerts follow tmux 3.6's, as recorded from a running tmux on 2026-10-10 (WSL
  Ubuntu, a client on a pty): `*-action` and `visual-*` as tmux's, the `alert-*`
  hooks, and flags per session's link (`alerts::raise`, checked for a bell
  against tmux by `corpus/formats.toml`). Not looked at more closely: an
  activity alert in the window in sight rings on every burst of output, as
  tmux's did there.
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
- **`popups_and_menus_are_placed_where_tmux_places_them` failed on macOS**
  (2026-10-18, CI run 39067127625, the `no test writes real user state` job, all
  three tries): after the test sent a lone Escape, the menu stayed open until
  the 30 s timeout. The regular macOS test job passed in the same run, and
  re-running the failed job passed. Again in run 39078626637
  (`test macos-latest`, all three tries; the rerun passed): two of about thirty
  macOS jobs that day, none before it as far as checked, so something from that
  day's changes (the terminal title, focus reports, the notification hooks) may
  make it likelier. Not reproduced or explained.
- **Handoffs to tmxr** at a program's edge: hjkl 0.42.2 (its `App::nav_step`,
  unit-tested in hjkl's suite) and nvim with
  [tmxr-navigator.nvim](https://github.com/kryptic-sh/tmxr-navigator.nvim)
  (tested in its own CI against a tmxr release). tmxr's CI job `handoff` runs
  both against the commit under test on Linux, macOS and Windows:
  `apps/tmxr/tests/handoff/hjkl.sh` and the plugin's `tests/integration.sh`, at
  the releases pinned in the job (`HJKL_VERSION`, `NAVIGATOR_VERSION`). The hjkl
  script fails with hjkl 0.42.1, which has no handoff (checked on Windows,
  2026-10-11).
- **The outer terminal's title** (`set-titles`, `render::title`): on by default,
  where tmux's is off, with a `set-titles-string` that takes the program's
  title, else `#S:#W` (asked for 2026-10-18). "The program set no title" is read
  as `#{pane_title}` equal to `#{host}`, the value tmux and tmxr give a pane
  with none, so a program that titles itself with the bare host name shows as
  unset. Not done: the terminal's own title is not saved and given back on
  detach (`CSI 22;0t` / `CSI 23;0t`), as tmux does not either; a title set while
  a client is attached reaches it with the next frame, as any change.
- **A resize just after attaching can go unseen on Linux** (2026-10-18). In
  `refresh_client_sets_and_clears_client_flags`, a client resized as soon as its
  status line showed kept its old size on the server for the test's 30 s. This
  happened in CI (run 39064458482, `test ubuntu-latest`, all three tries) and in
  about half of the runs in WSL Ubuntu when other e2e tests ran alongside; it
  never happened alone. A log of the client's events showed the `x` typed after
  the resize but no `Resize` event, so crossterm never saw a SIGWINCH. The cause
  was not found: crossterm's event source exists by then (`Guard::enter`'s
  keyboard probe makes it), and the patched crossterm turns SIGWINCH into a
  resize plainly. The test resizes again until the server has the size. A
  separate gap, not shown to matter: the hello's size is read at connect, before
  crossterm watches for resizes, so a resize between connect and attach would
  also be missed. A related finding: the e2e key-log fixture lost a focus report
  (`CSI I`, written by the server) in about one full Linux run in four when it
  arrived before the fixture's first crossterm `read`, and in none of twelve
  once the fixture polled first (`key_log_fixture`). Whether crossterm drops
  input that comes before its reader starts, and whether that is what loses the
  client's resize, is not established.
- **From a code review by a sub-agent (2026-10-18).** Found by reading code and
  tmux 3.6's source; nothing was run. Items marked _checked_ were confirmed by
  reading the code again; the rest are as reported.
  - `source-file PATH` sets `cfg_path` and replaces the whole config, losing
    every run-time `set -g`; a later plain `source-file` reloads `PATH`.
  - One config error at start throws the whole file away for the defaults
    (`tmxr_server::run`); the first client to attach is now told so, but the
    file's good parts are not kept. tmux runs a config line by line and keeps
    what worked; tmxr's TOML is read whole.
  - Performance: entering copy mode copies every history row into
    `copy::snapshot` (a String per cell) on the server thread; Windows takes two
    process snapshots per pane per second in `util::refresh_commands`;
    `#{pane_current_path}` inspects the process on each expansion; `#{m:…}`
    compiles its regex on each expansion. Not measured.
  - Large files: `copy/mod.rs` keeps `CopyMode` with its motions and search
    together; they could part next.
  - A message over `MAX_FRAME` from the client (`load-buffer -` of more than 16
    MiB) fails on the client before it is sent; the server turns a reply that
    large into an error (`conn.rs`).
- **tmux 3.6 features tmxr lacks** (the same review; compared against tmux's
  `cmd-*.c`, `options-table.c` and `format.c`, not run):
  - Formats: `#()` jobs run as tmux's (`format_jobs.rs`), but a job made to run
    again by a change in its expanded text leaves its earlier run going where
    tmux kills it, and a client's jobs are kept until it goes, where tmux also
    drops ones unused for an hour (format_job_tidy); modifiers `w:` (it needs a
    display-width measure `tmxr-command` does not have, unicode-width being a
    new dependency); `c:` gives no colour for X11's names (`orchid`,
    `navy blue`), which tmux 3.6's colour_byname takes from a table of 578
    (`tmxr_command::colour::rgb` reads everything else tmux reads, compared in
    the oracle; copying the table is the open choice); a syntax error ends the
    expansion as tmux's for a comparison, `m:` or `R:` without its comma
    (`format::expand_item`), but not for tmux's other failures, not compared: a
    loop or `N:` whose lookup fails (`format_loop_*`, `format_window_name`
    returning none); many of tmux's format variables (compare tmux's
    `format_table` with `vars.rs` for the list; `origin_flag`, `wrap_flag`,
    `insert_flag`, `scroll_region_*`, `alternate_saved_*`, `pane_tabs` and
    `cursor_shape` / `cursor_colour` need terminal state the `vt100` crate does
    not expose); styles `align=` `fill=` `list=` `range=` `push-default` /
    `pop-default`, strikethrough, blink, overline, curly and coloured
    underlines.
  - Options: the status line's first row is tmxr's own (`render::draw_status`,
    from `status-left`, the window list and `status-right`) while
    `status-format[0]` is tmux's default; set to anything else, and the rows
    after it (`status 2` to `5`), are drawn from `status-format` by
    `format_draw`, which takes tmux's `align`, `fill`, `default` /
    `push-default` / `pop-default`, lists and the left, right and window ranges,
    but not a list's markers and focus when it is too long for its room, nor
    pane, session or user ranges; clicks reach only the first row's ranges. With
    `status off` and `status-position top`, a message or prompt covers the
    window's first row; tmux's `screen_redraw_set_context` moves the window down
    a row for it instead; (not tested: `send-keys -M` under a top status line or
    in a panned view, which now measure from the mouse's window cell,
    `MouseTarget::col` / `row`, not its client cell);
    `window-status-activity-style` and `-bell-style` default to `default` where
    tmux's are `reverse`, on purpose (tmux's would change tmxr's theme, which
    shows the alert flags in its window format); colour options (`status-fg` /
    `-bg`, `display-panes-colour`, `clock-mode-colour` and the rest, all `Word`)
    keep any text as typed, where tmux 3.6 (seen 2026-10-20) refuses `nocolour`
    or `colour300` with `bad colour` and shows `#ABCDEF` as `#abcdef`; checking
    them as tmux does was considered and declined for now: tmxr's own defaults
    put formats in colour options (`display-panes-colour = "#{@thm_blue}"`),
    which tmux's check would refuse, so it needs those expanded first or the
    theme moved; server `editor`. `escape-time` is kept but never read.
  - Commands: `capture-pane -a` (it needs the saved normal screen, which `vt100`
    does not expose). `-e` writes only the attributes `vt100` keeps (bold, dim,
    italics, underline, reverse; not blink, hidden, strikethrough, overline or
    the underline styles and colour), and `38;5;n` for n below 16 as the plain
    or bright code, as `vt100` stores both alike. A line's allocation (what
    `-N`, `-e` and a plain capture run to) is worked out from what was written
    (`allocated` in `cmds/pane.rs`, as grid_expand_line rounds), where tmux's
    comes from the line's whole history, so a line written long and then cut
    short differs; an erased cell counts as unwritten (known divergence
    `capture_pane_t_counts_an_erased_cell_as_written`). The oracle cannot catch
    a difference on a capture's last line, as it trims the output's end. tmux
    also keeps a tab as the tab character, where `vt100` keeps the spaces it
    moved over; `split-window -I` and `display-message -I` read the client's
    whole input before showing it, where tmux shows it as it arrives (tmxr's
    client sends standard input ahead of the command); an empty pane is saved by
    resurrect as a pane running the shell; `display-message -v`;
    `display-message -a` lists tmxr's variables (`vars::NAMES`), fewer than
    tmux's; the `choose-*` pickers' tagging and sort keys (tmux's `t`, `T`,
    `O`), which tmxr's picker has no counterpart for; `choose-tree -F` replaces
    the text after a row's mark, where tmux's is the whole tree line;
    `command-prompt` Tab's menus (tmux opens a menu of the matches when several
    share no longer start, and of a session's windows after `-tname:`; tmxr
    completes as far as they agree and stops), and the `target` /
    `window-target` prompt types' completion (`overlay::prompt::completion`);
    smaller flags: `source-file -v`, several paths and globs (each load replaces
    the whole TOML config, so several files have no clear meaning yet);
    `source-file` of a tmux command file (`%if`, `%hidden`).
  - Hooks and modes: tmux hooks tmxr does not fire: `client-light-theme` /
    `-dark-theme`. Copy mode lacks `next-prompt` / `previous-prompt` (they need
    a prompt mark on every history line, as tmux keeps per grid line; `Emulator`
    keeps only the last OSC 133 input mark, and vt100's lines carry no flags, so
    the marks would have to be counted alongside the history as it scrolls).
    Word selection (`select-word`, `selection-mode word`) keeps to one line when
    it snaps back to a word's start, where tmux's grid reader follows a wrapped
    line; and the selection is as made only while the cursor stays where
    `select-word` / `select-line` left it, where tmux snaps again if the cursor
    comes back there. Not compared with tmux: the cursor motions under
    `select-line` (tmux's `lineflag` moves the cursor to a line's end or start
    on up / down), and on lines holding wide characters.

- **`server did not start` under a full `cargo test`** (Windows, three times on
  2026-10-20: `lock_hands_the_keys_to_the_lock_command`, then
  `layout_matches_tmux` and `option_scopes_match_tmux` in one run): a client
  gave up waiting for its server's pipe (`wait_for_server` in `tmxr-client`,
  `SPAWN_TIMEOUT`). Each passed alone and in the next full run. Two more
  one-offs under a full run the same day, each passing alone:
  `move_window_renumbers_and_replaces` found its session's windows gone, and
  `alerts_follow_the_action_and_visual_options` attached to a server with
  `no sessions`; a guess is a pane's `cmd.exe` failing to start under the load,
  its session and server going with it. Not new: run twice each the same day,
  the e2e suite alone failed one test (a different one each time) in one of two
  runs at `4e59d9a`, from before that day's work, and the same at the day's end,
  so the rate is unchanged; a full `cargo test` fails one about every run. The
  servers did start, only late: after the third, two `__server` processes for
  the failed run's pipes (`oracle-58296-44`, `-45`) were still running 20
  minutes on with no session, so the client's wait elapsed before the server
  opened its pipe. Such a server never exits: `Server::check_exit` waits for a
  first session (`had_session`, there because tmxr's server starts apart from
  its client, unlike tmux's forked one) and none comes. Options, neither taken:
  the server exits with no session and no client once a startup grace longer
  than `SPAWN_TIMEOUT` has passed (a test of it waits out the grace), or the
  client keeps the spawned `Child` and kills it when it gives up (the child is
  dropped at once today). Why the start took so long under load is not known.
- **`word_separators_and_wrap_search_reach_copy_mode` failed once** (Windows,
  full e2e suite, 2026-10-18) at its first `#{copy_cursor_word}` check; four
  runs alone and the next full run passed. The failure output was not kept, so
  the cause is unknown: a guess is ConPTY redrawing the pane after
  `cmd /c echo`, moving the line `history-top` lands on.
- **Flags the command table accepts that change nothing** (from an audit of
  every `cmd!` entry against its arm and helpers, 2026-10-18; the ones that did
  matter are implemented or refused since). Harmless, as what happens anyway:
  `customize-mode -N` (tmxr's picker has no preview), `-v` on `split-window` /
  `join-pane` / `move-pane`, `rotate-window -U`, `choose-tree -s`,
  `set-option -s` (tmxr picks the table from the name), `clear-history -H` (tmxr
  keeps no hyperlinks), and `-Z` on `choose-buffer`, `choose-client`,
  `choose-tree`, `find-window` and `customize-mode` (tmux zooms the pane under
  its pane modes; tmxr's pickers are full-screen overlays). Separately,
  `source-file` replaces the config path and reloads everything, where tmux runs
  the file's commands.
- **`list-keys` prints a `{ … }` block as a quoted string**: tmux prints a
  bind's commands re-parsed (`cmd_list_print`), as tmxr now does
  (`print_command_list`), but tmxr's tokenizer turns a `{ … }` block into one
  string word, so a command-prompt template given in braces prints quoted where
  tmux prints `{ … }`; and `$VAR` prints as written, where tmux expanded it when
  the bind was made. tmxr's own default binds and their notes also differ from
  tmux's, so only binds made in a test compare.
- **The full e2e suite kills its `wsl.exe` session** (2026-10-18): run as
  `wsl.exe -e bash -lc 'cargo test -p tmxr-cli --test e2e …'`, output stops and
  nothing after the run prints, though a log file shows every test finished.
  Twice the WSL VM restarted with it (`uptime` minutes old, `/tmp` emptied).
  Single tests and the oracle run fine. Not investigated; a guess is a test
  signalling a process group the session belongs to (`detach-client -P` sends
  SIGHUP to client parents). Workaround: log to a file in `$HOME` and read it
  from a new session.
- **`C-r` in an incremental search does nothing in tmux 3.6**, and tmxr does the
  same on purpose (seen 2026-10-20): `command-prompt -i` passes the text after
  `C-r` as `-text`, which tmux parses as flags of `search-backward-incremental`
  (its table takes none), so the command is dropped without a word; it works
  only when the template puts `--` first, as tmux's own default bind does not.
  tmxr's `copy::refuses_args` drops the same commands. Not compared on its own:
  where a search down leaves the cursor under emacs keys (tmux moves it past the
  match, `window_copy_search`); tmxr's default `C-r` / `C-s` binds also lack
  tmux's `-I "#{pane_search_string}"`, as tmxr has no `pane_search_string`.
- **Session groups follow tmux by comparison, not by call** (`groups.rs`): tmux
  calls session_group_synchronize_from wherever a command changes a session's
  windows; tmxr keeps each group's window list as last synchronised and, after
  every command and in `Server::settle`, copies the list of whichever member no
  longer matches it to the others. Two members changed between passes would lose
  one's change; no command changes two, as `swap-window`, `move-window` and
  `link-window` between members are refused as tmux refuses them
  (`Server::grouped_together`). A member whose current window went takes its
  last, else a lower, else a higher one: tmux's session_last / session_previous
  / session_next on the old list, not compared beyond the oracle's case. Not
  done: resurrect saves the members' windows as linked windows, not as a group,
  so a restore has no group.
- **`#{pane_dead_signal}` is a number**, as tmux 3.6 shows it on Linux (seen
  2026-10-20: `15` for SIGTERM, from tmux's compat `sig2name`). Where the system
  has `sys_signame` (macOS, the BSDs) tmux may show a name instead; not checked.
  tmxr reads the signal itself on Unix (`wait_exit` in `tmxr-term/src/pty.rs`,
  `waitpid`), which no dynamic-analysis tool covers: CI runs none, and Miri
  cannot run the call.
- **`assume-paste-time` times keys only**: tmux takes the gap from every input
  event that reaches its key handler, mouse events included, where tmxr's
  `Attached::key_at` is set by keys alone (`client_key`), so a key right after a
  click is timed from the key before it. Not compared with tmux. The e2e harness
  sets `assume-paste-time = 0`, as its tests type whole key sequences in one
  write.
- **A dead pane's line waits for quiet output, not unread bytes**: tmux writes
  `remain-on-exit-format` once the pane's terminal has no unread bytes
  (`window_pane_destroy_ready`, `FIONREAD`); tmxr's exit and output come from
  different threads, so it writes the line once output has been quiet for
  `DEAD_LINE_QUIET` or ended (`PtyEvent::Eof`). Not tested: that a pane reads
  nothing after the line, as tmux closes its terminal there; on Linux the
  shell's exit hangs the terminal up, so no output came after it in any attempt.
- **Alternate screen switches are made over vt100's**
  (`Emulator::switch_screen`, matching tmux 3.6 in the oracle for 47, 1047 and
  1049, on and off): switching on goes through vt100's 1049, which saves the
  cursor in the main screen's own slot, so a program's earlier `ESC 7` there is
  lost (tmux keeps 1049's cursor apart from it); and the cursor moves written
  after a switch are absolute, which origin mode (`ESC [ ? 6 h`) on the main
  screen would turn relative to its scroll region when 47 / 1047 switch off.
  Neither compared with tmux.
- **Pane status lines skip tmux's finer border rules**: with
  `pane-border-status`, tmux's `screen_redraw_pane_border` and `check_cell`
  treat the status text's cells as inside the pane and change which pane owns
  the border cells around them; tmxr draws the borders as without status lines
  and writes the text over them, which matched tmux 3.6 in the oracle's two- and
  stacked-pane cases. Not compared: mouse clicks on a status line, and windows
  larger than the client (the text is drawn in window cells).
- **Time expansion is not carried into everything tmux carries it into**:
  `format::expand_time` fills in strftime sequences at each level of a format
  (nested parts, `E:` values, modifier arguments) as tmux's `FORMAT_EXPAND_TIME`
  does, and the status line, the terminal title and `display-message` use it.
  Read against tmux 3.6 (2026-10-20), not run: `pane-border-format` takes it, as
  screen_redraw's format_expand_time; menus' items and the `-T` titles of menus
  and popups do not, as tmux's format_single. A `#()` job's command and output
  expand without it, and the whole format's strftime pass comes first (so a `%`
  in a command wants `%%`), as tmux's format_expand1 and format_job_get (read,
  not run). strftime covers the C locale's common conversions
  (`format::strftime`); `%Z`, `%z`, `%U`, `%V`, `%W`, `%G`, `%g` and `%x` / `%X`
  are kept as written.
- **tmux's `link-window` and `move-window` drop the mark** of a pane in the
  window they place (2026-10-18, tmux 3.6): `server_link_window` moves
  `marked_pane.wl` to the new link but keeps `marked_pane.s`, so the mark no
  longer validates. tmxr keeps the mark; not copied, as it reads as a tmux bug.
- **tmux's `session_bell_flag`, `session_activity_flag` and
  `session_silence_flag` do not ask the session** (tmux 3.6 `format.c`, read,
  not run): the loop over the session's windows returns on its first pass, so
  the bell flag is the session's first window's, and the other two test the
  format's target window (`ft->wl`), not the loop's. tmxr has none of the three;
  `session_alert` / `session_alerts` give the session's alerts correctly.
  Options: add them with tmux's answers (scripts get what tmux gives) or with
  the evident intent (any window alerted); not decided, so not added.
- **tmux crashes on a session loop's current format from a detached client**
  (2026-10-18, tmux 3.6): `display -p '#{S:a,b}'` run from the command line
  takes down the server, as `format_loop_sessions` reads the client's session,
  which a command client has none of. tmxr marks no session current there. The
  oracle case for the loops therefore leaves that form out; the e2e test
  `client_variables_tell_its_times_and_last_session` covers it with an attached
  client.
- **Oracle checks must be free of side effects**: `compare` in
  `tmxr-compat-oracle` reads a check again until both sides agree, so a check
  that creates something (`new-window -P`) runs twice on a mismatch and fails
  with a misleading error from the other side. Commands that print what they
  made are tested in e2e instead, with the expected strings confirmed against
  tmux 3.6 by hand.

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
