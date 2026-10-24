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
  Cross-repo work). Alacritty 0.17 (2026-10-21, on Arch, in a Hyprland nested in
  `kwin_wayland --virtual`, screenshot with `grim`): two panes, the border with
  tmux's half-coloured active indicator, bold red, underline, accented letters,
  CJK wide characters and the status line draw right. Not looked at in any other
  terminal, nor with vim in front of the navigator.
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

- **Synchronized updates are asked for, not configured**
  (`terminal::probe_sync`): at its first attach a client sends DECRQM for mode
  2026 and DA1 and reads the answers straight from the terminal (up to
  `PROBE_WAIT`, which only a terminal that answers neither waits out); where
  mode 2026 is known it wraps each frame from the server in `CSI ? 2026 h` / `l`
  (`Guard::write_frame`). This goes beyond tmux, which sends them only where
  terminfo or `terminal-features` says so (decided by the owner, 2026-10-21).
  Keys typed in the few milliseconds the probe reads are read by it and lost.
  Windows' client does not ask (its console hands over input records). Tested
  end to end against the harness's terminal answering DECRQM. Alacritty 0.17
  answers the probe with `CSI ? 2026 ; 2 $ y` then DA1 `CSI ? 6 c` (2026-10-21),
  so the client turns them on there, and its frames looked right in the
  screenshot above; tearing without them was not compared.
- **The local clipboard follows the copying client** (`clipboard::Local`): the
  display and SSH variables come from that client's environment, or the attached
  client last typed in, and on Linux and the BSDs each copy runs
  `tmxr __clipboard`, which holds the selection until the next copy stops it or
  the server is gone. Seen on Hyprland (2026-10-20): a server started without
  `WAYLAND_DISPLAY` copied to the Wayland clipboard through an attached client
  (the build before this copied nothing), one helper at a time, gone within its
  second once the server was. On X11 (2026-10-21, a client with only `DISPLAY`,
  the Xwayland of a Hyprland nested in `kwin_wayland --virtual`): the helper
  owned the X `CLIPBOARD` and an X11 client read back what `set-buffer -w`
  copied; the helper was gone after `kill-server`. Across a compositor restart
  (2026-10-22, the nested Hyprland killed and started again, a client attached
  in Alacritty each time): the helper whose compositor went stays, idle (no CPU
  over 5 s) and holding nothing, until the next copy stops it, which then
  reaches the new compositor; a helper could instead end when its connection
  does, which hjkl-clipboard does not report. With no client attached a copy
  (`set-buffer -w` from a script) reaches no local clipboard: the helper gets
  the server's environment, not the command client's. Not tested in CI, which
  has no display; nor the macOS and Windows arms (they set the clipboard in the
  server, as before). A selection another program takes over leaves the helper
  running, holding nothing, until the next tmxr copy. Idle cost: none while it
  holds a copy (2026-10-22, nested Hyprland, context switches over 10 s: 0.0 a
  second with hjkl-clipboard 0.43.4, 19.9 with 0.42, whose Wayland thread polled
  every 50 ms; the helper's own thread sleeps on Linux, `wait_for_server`,
  `PR_SET_PDEATHSIG`). hjkl-clipboard sends X11 INCR chunks 2 ms apart, as
  Hyprland 0.56's XWayland bridge loses chunks sent back to back
  (hyprwm/Hyprland discussion 16612).
- **CSIs the emulator makes beside vt100** (`modes::ModeSplitter`, from tmux's
  `input_csi_table`): SCP / RCP, REP (only right after a printed character, as
  tmux's), CBT and DECSCUSR (the shape and blinking tmxr keeps); HVP and HPA are
  vt100's own since 0.18. btop's boxes, which broke for want of HVP (vt100
  0.16), matched tmux 3.8 on every row by their box-drawing cells (2026-10-20,
  120x40, Linux). Not compared: btop's colours (`capture-pane -e` differs run to
  run with its live graphs). SCP / RCP go through vt100's `ESC 7` / `ESC 8`,
  which save the attributes too, as tmux's `input_save_state` does. HT, CBT, HTS
  (`ESC H`) and TBC (`CSI g`) use tab stops the emulator keeps as tmux does
  (`Emulator::tab`, compared by `tab_stops_as_tmux` and
  `tab_stops_reset_as_tmux`), where vt100's are fixed every 8 columns; an HT
  inside an OSC or DCS string stays the string's. CHT (`CSI n I`) is ignored, as
  tmux 3.8 ignores it (probed the same day). The scroll region is kept by the
  emulator as tmux keeps it (`Emulator::set_region`, compared by
  `scroll_region_as_tmux` and `scroll_region_kept_across_screens_as_tmux`): one
  for the terminal, where vt100 keeps one per screen, so it is written into
  vt100's again after each switch; a region of fewer than two rows is ignored,
  where vt100 takes the whole screen; and IL / DL outside it move the rows from
  the cursor to the bottom, through a region set for the moment
  (`Emulator::lines`, compared by
  `lines_inserted_and_deleted_outside_the_region_as_tmux`), including tmux's IL
  there of as many rows as are left changing nothing. Not compared: the region
  with origin mode on across a screen switch (vt100 keeps origin mode per screen
  too, so the cursor put back after the switch is placed relative to the region
  there).
- **Mouse use as in the owner's tmux config** (asked for 2026-10-11). The tmux
  compatibility oracle (`crates/tmxr-compat-oracle`, `corpus/mouse.toml`) drives
  a tmux 3.6 client and a tmxr client with the same SGR mouse events in CI and
  compares the results: click focus, drag to copy, double and triple click (and
  the keys two clicks send), the wheel into and out of copy mode, border drags,
  clicks and the wheel on the status line, the right-click menu and middle-click
  paste all match. Its first runs found the wheel scrolling as it entered copy
  mode and the binds missing tmux's alternate-screen and copy-mode conditions,
  fixed. Menus take the mouse as tmux's (`Menu::mouse`, from `menu_key_cb`),
  checked by the oracle's menu cases. Tried by hand in Alacritty 0.17
  (2026-10-22, an X11 Alacritty on the Xwayland of a Hyprland nested in
  `kwin_wayland --virtual`, driven by `xdotool` there; `wlrctl`'s virtual
  pointer moves Hyprland 0.56's cursor but its buttons and wheel reach no
  client): clicks focus panes, the wheel enters copy mode, `q` leaves it, double
  and triple clicks copy a word and a line, a drag copies its text, a border
  drag resizes, and a middle click pastes. It found a right click's menu closing
  at once: the press and release came in one read, and tmxr gave the release to
  the menu the press had opened, where tmux's `server_client_handle_key` gives
  an overlay only input that arrives once it is open
  (`a_right_tap_keeps_the_menu_it_opens`; the oracle's `click` writes the two 80
  ms apart, as a deliberate click, and tmux closes the menu then). The client
  now sends what it reads together as one message (`ClientMsg::Input`,
  `Server::inputs`) for that; tmxr's prompts and pickers still take keys read
  with the key that opened them, where tmux's prompt does not (its modes do), so
  a command typed in one go reaches the prompt.
- **crossterm is patched** until a release carries the fixes: crossterm 0.29's
  Unix parser read `ESC ESC` as a plain Escape, so Alt+Escape lost its Alt
  (upstream as crossterm-rs/crossterm#1143, branch `fix-alt-escape` of
  kryptic-sh/crossterm); and its mio event source, which mio wakes
  edge-triggered, lost input in two ways (both fixed 2026-10-20, offered
  upstream as crossterm-rs/crossterm#1154 and #1155): it returned on the first
  tty event of a poll's batch, so a SIGWINCH ready in the same wakeup as a key
  was never seen, and it stopped reading once a read that filled its 1024-byte
  buffer gave an event, so the rest of a burst waited for the next key (now read
  on while `FIONREAD` says bytes wait). Also on that branch: upstream's fix for
  an unknown finished `CSI ?` report read as incomplete, which swallowed every
  later key (crossterm-rs/crossterm#1106, cherry-picked), and the terminal's
  color-scheme report read as `Event::ColorScheme`, which upstream drops (not
  offered upstream yet; it adds a variant to `Event`). tmxr builds against all
  of them on 0.29.0 (branch `alt-escape-0.29`, pinned by `rev` in the root
  `Cargo.toml`'s `[patch.crates-io]`, and allowed in `deny.toml`); each
  event-source fix has an e2e test that fails without it (run on Linux)
  (`a_resize_and_a_key_arriving_together_both_reach_the_server`,
  `keys_typed_faster_than_the_client_reads_all_arrive`). The owner's report of
  the first in use (2026-10-20, Hyprland with Alacritty: closing the window
  beside the client's grew it, and tmxr kept the old size) was reproduced
  2026-10-21 in Hyprland 0.56 nested in a `kwin_wayland --virtual` (nothing on
  the desktop): with B focused beside A, which runs a client, closing B grows A
  and focuses it; 0.9.0's client kept its size in 2 of 11 runs, the fixed one in
  none of 23. Once a crossterm release has them, upgrade and drop both. A
  `[patch]` applies only to builds from this workspace: the release binaries and
  packages have the fixes, but `cargo install tmxr` from crates.io builds
  against crates.io's crossterm without them. Still different from tmux: an
  Escape and a mouse event in one read (`known_divergences.toml`), as crossterm
  drops the rest of a read once it has an event. Left as it is in the same
  `try_read`: a read error other than `WouldBlock` or `Interrupted` (EIO once
  the terminal hangs up) loops on the read without end; tmxr's client does not
  get there, as the hangup's SIGHUP ends it first (a pty closed under an
  attached client, 2026-10-20: the client exited).
- **tmux default binds**: all of tmux 3.6's 267 are bound (compared with its
  `list-keys`, 2026-10-11) except the 18 digit binds (`1`-`9` in vi, `M-1`-`M-9`
  in emacs) that open a "(repeat)" prompt, declined: tmxr's copy mode takes
  counts directly (`5j`).
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
    `corpus/options.toml`): `update-environment` and `status-format`; tmux's
    others (`command-alias`, `terminal-features`, `user-keys`, `pane-colours`,
    `codepoint-widths`) are options tmxr does not have. The config file keeps it
    as a list, so `--import-tmux-conf` leaves out a
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
    checked against tmux 3.6 by `corpus/options.toml`): in tmux's table order,
    the global tables listing every hook (the unset ones bare) and the others
    only the hooks set, each command re-parsed as tmux prints it; the global
    session table leaves out the one hook tmxr does not fire (`after-queue`;
    known divergence `show_hooks_g_lists_the_hooks_tmxr_fires`).
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
- **A pane made taller takes rows back from the history** as tmux 3.6's
  `screen_resize_y` does, through vt100 0.18's `set_size`
  (`growing_takes_back_the_rows_a_shrink_pushed_as_tmux_does`). Still different:
  tmux takes back only the rows scrolled since the screen was last cleared
  (`hscrolled`, which `CSI 2 J` and `clear-history` reset), vt100 any row its
  history has; and once the history has dropped rows at its limit, vt100 brings
  blank rows back in their place, where tmux brings none. Neither is compared
  with tmux.
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
- **A pane given no command runs a login shell** (`SpawnSpec::login_shell`), as
  tmux's spawn does, so the machine's login profile runs in it: on macOS
  `path_helper` reorders `PATH` and the prompt gets longer, which broke e2e
  tests in CI (run 39479217254, 2026-10-20); the e2e harness therefore sets
  `default-command` to the shell, and only
  `a_pane_without_a_command_runs_a_login_shell` clears it. Compared with tmux
  3.6 on Arch (`-sh` against `-sh`, the oracle's window 0); macOS's
  `path_helper` effect seen in CI, not compared with tmux there.
- **A removed socket is bound again on SIGUSR1** (`Server::rebind`), as tmux's
  `server_signal` does (`sigusr1_binds_a_removed_socket_again`). Each time
  leaves the previous accept thread waiting on the old socket, which no client
  can reach: interprocess's `Listener` gives no way to wake its `accept`, and
  `shutdown` would not wake it on macOS anyway. The socket directory moved out
  of `$XDG_RUNTIME_DIR` on 2026-10-20, as logind deletes that one at the user's
  last logout (`socket_base`); checked with `Linger=no` on Arch, where
  `systemd-user-runtime-dir stop` removes it. In `/tmp`, Arch's systemd-tmpfiles
  removes a file whose newest timestamp is 10 days old, so the server renews the
  socket's ctime each hour (`Endpoint::refresh`, from `Server::tick`, as tmux's
  `server_update_socket` chmods it); the refresh is unit-tested, the hourly call
  is not observed by any test. A real logout (2026-10-22, Arch, an SSH login to
  localhost as the session): a server started in it kept running once the
  session ended (logind left it `closing`, as `KillUserProcesses` is off by
  default and lingering was off), and answered `list-sessions` on its socket.
  With `KillUserProcesses=yes` logind would end it, as it would tmux's. Not
  tried: macOS's own cleaning of `$TMPDIR`.
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
- **Client flags** (`cmds::session::client_flags`): `-f` takes `read-only`,
  `ignore-size` and `no-detach-on-destroy`, and refuses tmux's others by name:
  `no-output`, `pause-after` and `wait-exit` are control-mode flags, and tmxr
  has no control mode; `active-pane` needs a per-client active pane. `!flag` and
  `refresh-client -f` / `-F` work as tmux's (`!read-only` leaves a read-only
  client read-only). `#{client_flags}` lists only what tmxr keeps: `attached`,
  `focused`, `ignore-size`, `no-detach-on-destroy`, `read-only` and `UTF-8`
  (every client, as tmxr writes nothing else, where tmux's follows the client's
  locale), not tmux's `suspended`. `#{client_tty}` is the client's terminal on
  Unix (`terminal::tty_name`, `ttyname_r`, which no dynamic-analysis tool
  covers: CI runs none, and Miri cannot run the call) and empty on Windows;
  `#{client_name}` is tmxr's client id, where tmux's is the tty (and `-c` takes
  it), not changed: every `-c` target and the e2e tests name clients by id. A
  client is `focused` from attaching, as tmux's, until its terminal reports
  focus out; the terminal is asked for focus reports only with
  `focus-events on`, as tmux's (`ServerMsg::FocusEvents`), and reports that come
  anyway are taken, as tmux's `tty_keys_next` takes them. On Windows the console
  hands crossterm focus events whether asked or not, and ConPTY asks its own
  terminal for them when it starts, so `focus-events off` does not keep them out
  there. Compared with tmux by `client_flags_as_tmux` (flags and
  `refresh-client -f`); the focus reports are not.
- **Pane focus** (`Server::update_pane_focus`, from tmux's
  `window_pane_update_focus`): a pane is focused while it is the active pane of
  a window a focused client shows, and a program that asked (mode 1004) gets
  `CSI I` / `CSI O` when that changes, with `pane-focus-in` / `-out`. tmux
  counts only a popup or a menu as covering the pane; tmxr's pickers are
  overlays too (tmux's are pane modes) but leave the pane focused, as tmux's
  would. Checked end to end on Windows and Linux
  (`a_pane_that_asked_is_told_when_it_gains_and_loses_focus`), a client's own
  focus out and back in included. The test's fixture reads the reports as raw
  bytes on Unix (`log_focus_bytes`): crossterm's reader lost the first one in
  about half of the Linux runs although tmxr had written it (seen with a log
  line in `update_pane_focus`; WSL Ubuntu, 2026-10-18). Why crossterm lost it
  was not found. With today's patched crossterm (2026-10-21, Arch) the fixture
  reading through crossterm instead lost none: 20 runs of the test alone and 6
  full e2e suites. Not tried in WSL or CI, so the raw-byte reading stays;
  switching it back to crossterm would test crossterm's focus path too.
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
- **Idle cost** (measured 2026-10-21 on the Arch laptop, an attached session
  doing nothing, context switches a second): tmxr's client 0.2 and server 0.6
  (0.4 since 2026-10-23, below), tmux 3.8's 0.0 and 0.1, from tmxr's 20.2 and
  4.5 before the client waited for input and the server for its next deadline
  (`Server::next_due`, `an_idle_client_and_server_do_not_wake`), and 0.8 for the
  server before `util::refresh_commands` (foreground processes, for automatic
  renames) rode on the status line's redraw (`refresh_deadline`): it had kept
  its own timer, a few milliseconds out of phase, so the two woke the server
  apart. What is left is one status redraw per `status-interval`, 5 as
  tmux-sensible sets it where tmux's own is 15, costing a wakeup on the server's
  main and client-write threads each (measured 2026-10-23, release build, 60 s:
  12 each, the client's main thread 12, every other thread 0). The client-read
  thread woke as often until `conn::wait_readable`: blocked in `read`, Linux
  woke it each time the client took a frame off the socket (a socketpair probe:
  50 wakeups for 50 writes, none with `poll` first); it now waits in `poll` for
  input (`a_clients_reader_sleeps_while_frames_go_out`). The writer thread's
  wakeup is the design: the main thread hands each frame to it through a queue,
  where tmux writes from its one thread. With `status-interval 0` the server
  wakes once a second instead, for `refresh_commands`, which then keeps a
  one-second floor; tmux checks a pane's name when the pane prints.
  `Server::next_due` must name every timed duty `tick` runs; one added to `tick`
  without a deadline there runs only when something else wakes the server.
  Windows' client still polls (crossterm's console reader cannot be woken),
  measured nowhere.
- **`#{pane_current_command}` on Windows** still reads the copy
  `util::refresh_commands` keeps, up to a refresh old, as a lookup there
  snapshots every process; Unix reads it when asked (`vars.rs`). Measured
  2026-10-22 (release build, about 320 processes): a live read cost 9 to 29 ms
  an expansion (`display -p` with 20 of them, 40 calls, against
  `#{pane_index}`), the cached copy nothing measurable, so the copy stays; a
  status line naming each window's command would take a snapshot a window a
  redraw.
- **OSC 8 hyperlinks** (decided by the owner 2026-10-22: kept by the vt100 fork,
  doy/vt100-rust#51 (with `Screen::clear_hyperlinks` for `clear-history -H`),
  which costs a cell 2 bytes of text): render puts the number of a link and
  underline style and colour together in the frame cell's `underline_color`
  (`backend::Links`, a table of decorations; nothing else uses that field), so
  ratatui's diff redraws a cell whose link or underline alone changed, and
  `AnsiBackend::draw` writes it as OSC 8 and SGR
  (`a_programs_hyperlink_reaches_the_client`). The owner chose this over an
  overdraw pass or tmxr's own diff. Still unlike tmux: links go to every client,
  where tmux sends them only to terminals whose `terminal-features` have
  `hyperlinks`. Copy mode's snapshot keeps a cell's link the same way, in its
  style's `underline_color` (`copy::link_of`, a cell stays 40 bytes), resolved
  through the pane's vt100 table when drawn
  (`copy_mode_keeps_a_lines_hyperlink`). A client's and a pane's link tables
  only grow (`u16` of them in a pane, then text is unlinked). Seen drawing
  cleanly in Alacritty 0.17 (2026-10-22, nested Hyprland), and a click on a link
  there, with tmxr's mouse off, gave Alacritty's hint command the link's URL (an
  X11 Alacritty driven by `xdotool`; the hint set to a command that only records
  it).
- **vt1k replaced vt100** (the owner's call, 2026-10-24): each pane's emulator
  is [vt1k](https://github.com/kryptic-sh/vt1k), kryptic-sh's own crate on vte,
  pinned by `rev` in the root `Cargo.toml`. Its API and behaviour follow vt100
  0.18 with the kryptic-sh fork's additions (the fork's branches `scroll-alloc`,
  `visible-row`, `trim-on-scroll`, `line-cells`, `hyperlinks`, `saved-screen`,
  `reflow`, `sgr-colon` and `underline-styles`, now retired), held to it by
  vt1k's `tests/parity.rs`, which feeds random sessions to both and compares
  everything either shows; entries here that say what vt100 does describe vt1k
  too. Where vt100 panicked (a combining character after a widened one on a
  one-column screen, `CSI @` past a row's end) vt1k does not, and viewing its
  history no longer clears the rows' wrap flags. Rows of history are packed, so
  `Screen::line_cells` hands out a `Cow`. Measured on the Arch laptop (release
  builds, 50000 full lines of 80 columns, server RSS): 155 MB with vt100, 21 MB
  with vt1k (tmux 3.8: 38 MB). Under a flood (`seq 1 2000000` into a detached
  120x40 pane, end by `wait-for`, four performance cores) the server took
  0.74–0.86 s against vt100's 0.59–0.73 s: packing each line costs more than
  vt100's copy; the parsers alone took 517 ms against 486 ms. Not looked into
  further. The vt100 fork's packed-history branch (`packed-history`, never
  pushed) is superseded.
- **crates.io cannot take tmxr-server, tmxr-client or tmxr** while they depend
  on git: vt1k (kryptic-sh's emulator, not on crates.io yet) and the crossterm
  fork (tmxr-server and tmxr-client match its `Event::ColorScheme`, which
  crates.io's crossterm 0.29.0 lacks). v0.10.0 (2026-10-23) published on
  crates.io only tmxr-proto, tmxr-command, tmxr-config and tmxr-term, so
  crates.io's tmxr stays at 0.9.0. Decided by the owner (2026-10-23): no
  crates.io publishing for now; the `publish-crates` job is off (`if: false`,
  out of `tag-release-status`'s needs) and the README installs with
  `cargo install --git`. Publishing vt1k would remove one of the two blockers; a
  crossterm fork crate would sit beside crates.io's crossterm that hjkl's crates
  and ratatui use, which crosses at one call, (`hjkl_picker_tui::handle_key`'s
  `KeyEvent`), so it needs a conversion there or hjkl moved to the fork too.
- **Copy mode's copy of a coloured history takes three times tmux's memory**
  (measured 2026-10-24, release builds, 50000 lines of 80 columns, server RSS).
  Lines of plain ASCII in the default style are kept a byte a cell
  (`copy::model::Cells::Plain`): 28 MB in copy mode against 22 MB out of it. But
  a line with any style keeps full 48-byte cells (`copy::Cell`): with a green
  word on each line, 212 MB in copy mode against 21 MB out of it (vt100: 297 and
  137 MB). tmux 3.8 takes 69 MB in copy mode, 38 MB out of it (plain lines). The
  way in: keep a styled line as its text and runs of styles, which means
  `Line::cell` and `Line::cells` handing out cells by value instead of by
  reference (each of copy mode's call sites).
- **No frame-rate cap**: the server renders after each pass of its loop, and a
  pass stops handling events after `DRAIN_BUDGET` so a flood still repaints.
  Each pane's output waits in a bounded queue (`output::QUEUE_LIMIT`) that
  blocks the reader when full; before that queue, an unbounded channel let `seq`
  on Linux CI bury a key press under 6–9 s of output (2026-10-09).
  `detach_stays_responsive_under_flood_output` checks every CI platform detaches
  within 5 s under a flood. Frame rate itself is not capped or measured;
  considered and not done (2026-10-14): tmux has no cap either (it redraws once
  per pass of its event loop, as tmxr does), and nothing has shown a need. A cap
  is worth adding only with a measurement showing frames wasted. Measured
  2026-10-22 on the Arch laptop: a 120x40 client in a pty, its pane printing
  `seq 1 2000000` (its end signalled by `wait-for`), was sent 20160 to 132753
  bytes by tmxr (release build, three runs) and 1961163 to 1997129 by tmux 3.8,
  in 0.63–0.73 s against tmux's 0.67–0.85 s: tmxr already sends a fraction of
  what tmux does under a flood, so a cap would save nothing measurable there.
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
- **A resize just after attaching went unseen on Linux** (2026-10-18), in
  `refresh_client_sets_and_clears_client_flags` under load (CI run 39064458482):
  the client's log showed the `x` typed after the resize but no `Resize` event.
  The cause was crossterm's dropped SIGWINCH (see "crossterm is patched"),
  reproduced deterministically on 2026-10-20 (Arch Linux): a resize and a key
  arriving while a lock command runs, when the client reads nothing, lost the
  resize every time with the old crossterm and none with the fix
  (`a_resize_and_a_key_arriving_together_both_reach_the_server`, which passes
  with the old crossterm when no key comes). The flags test resizes once again.
  A separate gap, not shown to matter: the hello's size is read at connect,
  before crossterm watches for resizes, so a resize between connect and attach
  would be missed. Not established: whether the same fix explains the focus
  report the e2e key-log fixture lost in about one full Linux run in four when
  it arrived before the fixture's first crossterm `read` (none in twelve once
  the fixture polled first, `key_log_fixture`), or the one the "Pane focus"
  entry describes. A standalone probe the same day (a program sleeping in raw
  mode while `CSI I` was written, then reading with crossterm) lost none of 20,
  with the old crossterm, so input that comes before the reader exists is not
  lost by itself.
- **From a code review by a sub-agent (2026-10-18).** Found by reading code and
  tmux 3.6's source; nothing was run. Items marked _checked_ were confirmed by
  reading the code again; the rest are as reported.
  - A config error at start leaves out only the settings it is in
    (`tmxr_config::load_keeping`, since 2026-10-23), and the first client to
    attach is told which; a file that is not TOML still gives the defaults, as
    it has no settings to keep. `source-file` of a `.toml` keeps a file's good
    settings the same way (`tmxr_config::load_file`), applying only those and
    failing with the rest named.
  - Performance: entering copy mode still copies the whole history
    (`copy::snapshot`), but cheaply since 2026-10-21 (5–6 ms for 50000 lines,
    see "vt100 is patched"); `util::refresh_commands` takes one process snapshot
    each `status-interval` (5 s by default) for every pane on Windows
    (`process::Processes`; it took two a pane before 2026-10-22, and one costs
    about 5 ms with 348 processes, release build); `#{pane_current_path}`
    inspects the process on each expansion: on Windows, a release build showed
    no measurable cost against `#{pane_index}` (40 calls of 20 expansions each,
    2026-10-20; a debug build took about 28 ms an expansion). Not cached, as a
    cache would give `split-window -c '#{pane_current_path}'` a directory from
    before a `cd`. (`#{m:…}` and `s/…/` keep their compiled patterns,
    `format::compiled`: 89 µs an expansion with two patterns before, 7 µs after,
    release build, 2026-10-20.)
- **tmux 3.6 features tmxr lacks** (the same review; compared against tmux's
  `cmd-*.c`, `options-table.c` and `format.c`, not run):
  - Formats: `#()` jobs run as tmux's (`format_jobs.rs`), and a job no format
    has started for an hour is dropped (`FormatJobs::tidy`, unit tested; the
    server's tick calling it is not observed by any test); a job's run is
    stopped by killing its own process (SIGKILL), where tmux sends SIGTERM, and
    in both a command's own children outlive it unless the shell `exec`s them; a
    syntax error ends the expansion as tmux's for a comparison, `m:` or `R:`
    without its comma (`format::expand_item`), and a loop or `N:` whose lookup
    fails gives what tmux's does (compared by
    `loops_and_n_whose_lookups_fail_as_tmux`); many of tmux's format variables
    (compare tmux's `format_table` with `vars.rs` for the list; `cursor_colour`
    and `cursor_very_visible` are kept from OSC 12 / 112 and `CSI 34 l` / `h`
    (`cursor_colour_and_very_visible_as_tmux`), and the client's terminal takes
    the active pane's cursor colour as tmux's Cs / Cr
    (`cursor_style_and_programs_shape_the_clients_cursor`; seen red in Alacritty
    0.17 after a pane's `OSC 12 ; red`, 2026-10-22, nested Hyprland); unlike
    tmux, it is sent to every terminal, where tmux sends it only where terminfo
    or `terminal-features` has `ccolour`, and a leaving client always sends OSC
    112, where tmux does only if it set a colour (as tmxr already did with the
    cursor shape); `cursor-colour` gives the colour where the program sets none
    (`cursor_colour_falls_back_to_the_option`); a program's OSC 10, 11 and 12
    queries are answered as tmux's, ended as each query was
    (`modes::Piece::ColourQuery`, split from the output as vt100 does not say
    how an OSC ended; `Server::colour_answers`), from `window-style` /
    `window-active-style` and the cursor's colour
    (`colour_queries_are_answered_from_the_window_style`), and where those leave
    the terminal's own colour, the client terminal's, which the client asks with
    OSC 10 / 11 beside its DECRQM probe at attach (`ClientMsg::TerminalColours`;
    `colour_queries_get_the_client_terminals_colours`; in Alacritty 0.17 on
    2026-10-22 a pane read back Alacritty's own colours, and Neovim (`-u NONE`)
    in tmxr in an Alacritty with a light background set `background=light`,
    where under tmxr 0.9.0 it guessed `dark`; Alacritty answers OSC 10 and 11,
    in the query's terminator, but not OSC 12). Unlike tmux, the client asks
    once, at its first attach, where tmux asks again on a theme change (mode
    2031, which Alacritty lacks), and the Windows client does not ask; A
    program's OSC 4 palette entries (`tmxr_term::palette`) are drawn as it set
    them and its queries for them answered
    (`palette_entries_a_program_sets_are_answered`,
    `a_programs_palette_colour_is_drawn`), but a query for an entry not set gets
    no answer, where tmux asks the client's terminal and passes its answer on.
    tmux's `pane-colours` (a default palette under the program's) is missing:
    adding it needs `tmxr_config::Array` read by index, a config-file form for a
    sparse array, `with_palette` falling back to it, and the emulator told it,
    as it answers OSC 4 queries itself (considered 2026-10-22, not started:
    little used); an empty OSC 104 leaves the OSC 10 / 11 colours, as tmux 3.6
    shows it (its colour_palette_clear resets them, but nothing marks the pane's
    style changed, so they stay until something else does). A program's OSC 10 /
    11 colours are its pane's, as tmux's palette `fg` / `bg`, with tmux's
    precedence: in the active pane over `window-style` and under
    `window-active-style`, in another under `window-style`
    (`a_programs_default_colours_are_the_panes`); `cursor_shape` and
    `cursor_blinking` are kept from DECSCUSR and `CSI ? 12`; insert mode and
    autowrap are kept, and drawn, by vt1k
    (`insert_mode_and_no_autowrap_draw_as_tmux`); `origin_flag` from `CSI ? 6`,
    split out by `modes::ModeSplitter` and passed on to vt100); and overline
    (vt100 keeps none). Underline styles and colours are kept (the fork's
    `underline-styles`; the owner chose on 2026-10-23 to grow its cell from 32
    to 36 bytes for them, over a per-screen table or fewer content bytes) and
    sent as tmux's `usstyle` would to a client whose `TERM` names a terminal
    with `Smulx` in ncurses 6.5's terminfo, or kitty or Ghostty
    (`Server::client_underline_styles`); tmxr reads no terminfo, so a terminal
    outside that list gets a plain underline. Unlike tmux, the colour goes with
    the styles: those entries lack `Setulc`, so tmux 3.6 sends no underline
    colour to Alacritty or foot unless `terminal-features` adds `usstyle`,
    though both draw it. Seen in Alacritty 0.17 (2026-10-24, nested Hyprland):
    single, double, curly, coloured curly, dotted and dashed as written.
    Measured the same day (release builds, 50000 full lines of 80 columns): the
    server took 138 MB before and 155 MB after, and in copy mode 298 and 345 MB,
    its snapshot cell (`copy::Cell`) grown from 40 to 48 bytes by
    `copy::Underline`.
  - Options: every status row is drawn from its `status-format` item by
    `format_draw` (the first from tmux's default unless set), which takes tmux's
    `align`, `fill`, `default` / `push-default` / `pop-default`, lists (cut
    around their focus with their markers, laid out as tmux's `format_draw_left`
    / `_centre` / `_right` / `_absolute_centre`, compared by the
    `a_status_list_*` cases) and its left, right, window, pane, session and user
    ranges (a click sets the mouse's target as tmux's server_client_check_mouse,
    and `#{mouse_status_range}` names the range, a user range's text cut to
    `format_draw::USER_RANGE_MAX` bytes; compared by `corpus/mouse.toml`);
    clicks reach only the first row's ranges. With `status off` and
    `status-position top`, a message covers the window's first row, as tmux 3.6
    shows it (compared 2026-10-20): tmux's `screen_redraw_set_context` would
    draw the window a row down, but a message asks only for the status line to
    be drawn and freezes the terminal (`status_message_set`, `TTY_FREEZE`), so
    the shift shows only after a full redraw (a resize, `refresh-client`), which
    tmxr does not copy. tmxr does not freeze either: a pane's output is drawn
    while a message shows, where tmux holds it until the message goes (not with
    `display-message -C`); (not tested: `send-keys -M` under a top status line
    or in a panned view, which now measure from the mouse's window cell,
    `MouseTarget::col` / `row`, not its client cell);
    `window-status-activity-style` and `-bell-style` default to `default` where
    tmux's are `reverse`, on purpose (tmux's would change tmxr's theme, which
    shows the alert flags in its window format); colour options (`status-fg` /
    `-bg`, `display-panes-colour`, `clock-mode-colour`, `cursor-colour` and the
    rest, all `Word`) keep any text as typed, where tmux 3.6 (seen 2026-10-20)
    refuses `nocolour` or `colour300` with `bad colour` and shows `#ABCDEF` as
    `#abcdef` (and, seen 2026-10-22, refuses `none` for `cursor-colour` though
    it shows its default as `none`, and shows `orchid` there as `#da70d6`);
    checking them as tmux does was considered and declined for now: tmxr's own
    defaults put formats in colour options
    (`display-panes-colour = "#{@thm_blue}"`), which tmux's check would refuse,
    so it needs those expanded first or the theme moved. `editor` edits a buffer
    from `choose-buffer` (`e`) in a popup, as tmux's `popup_editor`, the popup
    taking the picker's place (tmxr's overlay is one at a time) and the picker
    opened again by its command once the editor is done, so it comes back on its
    first row, where tmux's buffer mode keeps the buffer highlighted; on
    Windows, where tmux has no default, `editor` falls back to `notepad` (a
    choice, not tmux's). `escape-time` holds a lone Escape on the server
    (`Server::escape_time`), where tmux's client reads its terminal with the
    wait; tmxr's client reads through crossterm, which already reads an Escape
    and a key that arrive together as Meta and the key, so only keys arriving
    apart are joined there (tmxr's default is 0, tmux's 10).
  - Commands: `capture-pane -a` reads the normal screen as vt100 keeps it under
    the alternate one (doy/vt100-rust#52), where tmux's is a copy taken when the
    alternate screen came on; the two could differ if the pane is resized while
    the alternate screen shows (vt100 resizes the hidden grid too; not
    compared). `capture-pane -e` writes only the attributes `vt100` keeps (bold,
    dim, italics, underline, reverse; not blink, hidden, strikethrough, overline
    or the underline styles and colour), and `38;5;n` for n below 16 as the
    plain or bright code, as `vt100` stores both alike. A line's allocation
    (what `-N`, `-e` and a plain capture run to) is worked out from what was
    written (`allocated` in `cmds/pane.rs`, as `grid_expand_line` rounds), where
    tmux's comes from the line's whole history, so a line written long and then
    cut short differs; an erased cell counts as unwritten (known divergence
    `capture_pane_t_counts_an_erased_cell_as_written`). The oracle cannot catch
    a difference on a capture's last line, as it trims the output's end. tmux
    also keeps a tab as the tab character, where `vt100` keeps the spaces it
    moved over; `split-window -I` and `display-message -I` read the client's
    whole input before showing it, where tmux shows it as it arrives (tmxr's
    client sends standard input ahead of the command); an empty pane is restored
    by resurrect empty, without what was shown in it; `display-message -v`;
    `display-message -a` lists tmxr's variables (`vars::NAMES`), fewer than
    tmux's; the `choose-*` pickers' tagging and sort keys (tmux's `t`, `T`,
    `O`), which tmxr's picker has no counterpart for; `choose-tree -F` replaces
    the text after a row's mark, where tmux's is the whole tree line;
    `command-prompt` Tab completes and offers menus as tmux's (names, sessions,
    a session's windows, the `target` and `window-target` types; `prompt_tab_*`
    in `corpus/keys.toml`), though which ten names a long menu keeps was matched
    by result (the last ten), not read from tmux's code.
  - Hooks and modes: the client's theme follows its terminal as tmux's: the
    client sends `CSI ? 2031 h` and `CSI ? 996 n` as its terminal starts and
    `CSI ? 2031 l` as it stops, and the terminal's `CSI ? 997 ; 1 / 2 n`
    (through the patched crossterm's `Event::ColorScheme`) sets
    `#{client_theme}` and runs `client-dark-theme` / `client-light-theme`
    (`the_terminals_color_scheme_is_the_clients_theme`; in foot 1.28 on
    2026-10-23 a client read `dark` at attach and followed foot's `SIGUSR2` /
    `SIGUSR1` to `light` and back, each running its hook; Alacritty 0.17 does
    not report, answering DECRQM for 2031 with `0`). Unlike tmux, the client
    does not ask for its terminal's colours again on a change, so an answer to a
    pane's OSC 10 / 11 query keeps the colours from attach, A pane's
    `CSI ? 996 n` is answered as tmux's window*pane_get_theme
    (`Server::pane_theme`; `a_panes_color_scheme_query_is_answered`). A pane's
    mode 2031 is not kept and gets no reports: tmux 3.6 sends them only when
    something marks the pane's theme changed (a new pane, a style option,
    `select-pane`, OSC 10 / 11, a pane moved), not when a client's theme
    changes, which is little use; not done. Copy mode's `next-prompt` /
    `previous-prompt` (compared with tmux 3.6 in `corpus/keys.toml`) read OSC
    133 marks the emulator keeps by line number (`Emulator::prompt_lines`, rows
    lost off the history counted by vt100's `on_scroll`); a mark stays on its
    number where a scroll region or a line insert or delete moves lines within
    the screen, where tmux's line flags move with the line (shells do neither).
    Word and line selection moving away and back to where `select-word` /
    `select-line` left the cursor (by characters, words and lines) is compared
    with tmux (`\*\_selection_back*\*`in`corpus/keys.toml`); other paths back
    were not.

- **`server did not start` under a full `cargo test`** (Windows, three times on
  2026-10-20: `lock_hands_the_keys_to_the_lock_command`, then
  `layout_matches_tmux` and `option_scopes_match_tmux` in one run): a client
  gave up waiting for its server's pipe (`wait_for_server` in `tmxr-client`,
  `SPAWN_TIMEOUT`). Each passed alone and in the next full run. Two more
  one-offs under a full run the same day, each passing alone:
  `move_window_renumbers_and_replaces` found its session's windows gone, and
  `alerts_follow_the_action_and_visual_options` attached to a server with
  `no sessions`; a guess is a pane's `cmd.exe` failing to start under the load,
  its session and server going with it. Again on 2026-10-22:
  `remain_on_exit_failed_monitor_bell_and_rename_format` found
  `no server running` at its first check, just after its `new-session -d`, and
  passed alone three times running. Not new: run twice each the same day, the
  e2e suite alone failed one test (a different one each time) in one of two runs
  at `4e59d9a`, from before that day's work, and the same at the day's end, so
  the rate is unchanged; a full `cargo test` fails one about every run. Load
  decides it: with other programs busy, the e2e suite alone failed five or six
  tests in a run, and with the machine idle none (178 of 178, the same day).
  Timing checks fail the same way under load:
  `run_shell_holds_its_command_list_and_reply` (`run-shell -b` returning within
  2 s; 2.65 s and 2.28 s in two full runs), `display_menu_runs_the_item_picked`
  (3.1 s to show) and `display_popup_runs_a_command_over_the_panes`, each
  passing alone three times running. The servers did start, only late: after the
  third, two `__server` processes for the failed run's pipes (`oracle-58296-44`,
  `-45`) were still running 20 minutes on with no session, so the client's wait
  elapsed before the server opened its pipe. Such a server used to run for ever
  with no session; since 2026-10-22 one that has had no session and has no
  client goes after `STARTUP_GRACE` (`Server::check_exit`, with `exit-empty`;
  `a_server_nothing_reaches_goes_after_its_grace`), longer than the client's
  `SPAWN_TIMEOUT`, which a const assertion holds it to. Considered and declined:
  the client killing the server it spawned when it gives up, as a server that
  did open its pipe may by then serve another client, and how a busy pipe
  reaches `wait_for_server` on Windows was not established. Why the start took
  so long under load is still not known, so the failures themselves remain.
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
  `set-option -s` (tmxr picks the table from the name), and `-Z` on
  `choose-buffer`, `choose-client`, `choose-tree`, `find-window` and
  `customize-mode` (tmux zooms the pane under its pane modes; tmxr's pickers are
  full-screen overlays).
- **What `list-keys` prints of a bind**: its commands re-parsed and its `{ … }`
  blocks in braces, as tmux's (`blocks_print_in_braces_as_tmux`; a
  `tmxr_command::Line` carries which words were blocks to `bind-key`). A bind
  made by a command has its `$VAR` expanded when it is made, as tmux's (compared
  by `a_binds_variables_print_expanded_as_tmux`); one from tmxr's TOML config
  keeps it, to expand when it runs, and prints it as `${VAR}`. tmxr's own
  default binds and their notes also differ from tmux's, so only binds made in a
  test compare.
- **The full e2e suite kills its `wsl.exe` session** (2026-10-18): run as
  `wsl.exe -e bash -lc 'cargo test -p tmxr-cli --test e2e …'`, output stops and
  nothing after the run prints, though a log file shows every test finished.
  Twice the WSL VM restarted with it (`uptime` minutes old, `/tmp` emptied).
  Single tests and the oracle run fine. Not investigated; a guess is a test
  signalling a process group the session belongs to (`detach-client -P` sends
  SIGHUP to client parents). Workaround: log to a file in `$HOME` and read it
  from a new session.
- **Copy mode against tmux 3.6, command by command** (2026-10-23): each
  copy-mode command was run from one start in tmux 3.6 and tmxr, and the cursor,
  view, selection and search formats compared (a scratch script, not kept; the
  fixes have oracle cases in `corpus/keys.toml`). Still differing:
  - Under vi keys a search down from a marked match starts after the whole match
    (`window_copy_move_after_search_mark`); tmxr's starts one cell on, so a
    match overlapping the one at the cursor is found where tmux's skips it.
    Searches, word moves, word selection and bracket matching otherwise follow
    `mode-keys` as tmux's (compared from many starts, both modes, `wrap-search`
    on and off).
  - A search matches within a line, where tmux's also match across a wrap, so
    `#{search_count}` can count fewer.

- **`C-r` in an incremental search does nothing in tmux 3.6**, and tmxr does the
  same on purpose (seen 2026-10-20): `command-prompt -i` passes the text after
  `C-r` as `-text`, which tmux parses as flags of `search-backward-incremental`
  (its table takes none), so the command is dropped without a word; it works
  only when the template puts `--` first, as tmux's own default bind does not.
  tmxr's `copy::refuses_args` drops the same commands. tmxr's default `C-r` /
  `C-s` binds also lack tmux's `-I "#{pane_search_string}"`, as tmxr has no
  `pane_search_string`.
- **Session groups follow tmux by comparison, not by call** (`groups.rs`): tmux
  calls `session_group_synchronize_from` wherever a command changes a session's
  windows; tmxr keeps each group's window list as last synchronised and, after
  every command and in `Server::settle`, copies the list of whichever member no
  longer matches it to the others. Two members changed between passes would lose
  one's change; no command changes two, as `swap-window`, `move-window` and
  `link-window` between members are refused as tmux refuses them
  (`Server::grouped_together`). A member whose current window went takes its
  last, else a lower, else a higher one: tmux's `session_last` /
  `session_previous` / `session_next` on the old list, not compared beyond the
  oracle's case. Not done: resurrect saves the members' windows as linked
  windows, not as a group, so a restore has no group.
- **`#{pane_dead_signal}` is a number**, as tmux 3.6 shows it on Linux (seen
  2026-10-20: `15` for SIGTERM, from tmux's compat `sig2name`). Where the system
  has `sys_signame` (macOS, the BSDs) tmux may show a name instead; not checked.
  tmxr reads the signal itself on Unix (`wait_exit` in `tmxr-term/src/pty.rs`,
  `waitpid`), which no dynamic-analysis tool covers: CI runs none, and Miri
  cannot run the call.
- **The e2e harness sets `assume-paste-time = 0`**, as its tests type whole key
  sequences in one write; the tests that time pastes set it themselves.
- **A dead pane's line waits for quiet output, not unread bytes**: tmux writes
  `remain-on-exit-format` once the pane's terminal has no unread bytes
  (`window_pane_destroy_ready`, `FIONREAD`); tmxr's exit and output come from
  different threads, so it writes the line once output has been quiet for
  `DEAD_LINE_QUIET` or ended (`PtyEvent::Eof`). Not tested: that a pane reads
  nothing after the line, as tmux closes its terminal there; on Linux the
  shell's exit hangs the terminal up, so no output came after it in any attempt.
- **A width change reflows the screen** as tmux's `screen_reflow`, through the
  vt100 fork's `Screen::set_reflow` (branch `reflow`, carried on `tmxr-0.18`;
  not offered upstream yet, as it builds on #47–#52): each line's rows joined
  and split at the new width (history included, blank rows below counted, as
  tmux's grid keeps them), the cursor kept in its line or moved to its end, the
  normal screen left as it was under the alternate screen until that goes, and a
  wide character counted three columns where a row breaks, as tmux's
  `grid_reflow` counts its padding cell (all compared with tmux 3.6:
  `a_width_change_rewraps_lines_as_tmux` and the three after it in
  `corpus/layout.toml`). Copy mode rewraps its own copy the same way
  (`copy::model::reflow`, as tmux's `window_copy_resize`;
  `copy_mode_rewraps_on_a_width_change_as_tmux`), a second implementation of the
  same rules on copy mode's cells, kept beside the fork's. Prompt marks follow
  their lines (`Emulator::follow_reflow`); the input mark is dropped, so
  `prefix y` falls back to the shell's line keys until the next prompt. Seen in
  Alacritty 0.17 under Hyprland's retiling (2026-10-23).
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
  `screen_redraw`'s `format_expand_time`; menus' items and the `-T` titles of
  menus and popups do not, as tmux's `format_single`. A `#()` job's command and
  output expand without it, and the whole format's strftime pass comes first (so
  a `%` in a command wants `%%`), as tmux's `format_expand1` and
  `format_job_get` (read, not run). strftime covers the C locale's common
  conversions (`format::strftime`); `%Z` is kept as written: `LocalTime` has no
  zone name, and Windows' is a long display name (`Taipei Standard Time`) where
  glibc gives `CST`.
- **tmux's `link-window` and `move-window` drop the mark** of a pane in the
  window they place (2026-10-18, tmux 3.6): `server_link_window` moves
  `marked_pane.wl` to the new link but keeps `marked_pane.s`, so the mark no
  longer validates. tmxr keeps the mark; not copied, as it reads as a tmux bug.
- **`session_bell_flag`, `session_activity_flag`, `session_silence_flag` differ
  from tmux 3.6 on purpose** (decided by the owner, 2026-10-22): tmux's loop
  over the session's windows returns on its first pass, so its bell flag is the
  first window's and the other two test the format's target window (read in
  `format.c`, not run); tmxr's are set when any window has the alert
  (`session_alert_flags_look_at_every_window`).
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
  aarch64 (on an Arm runner). The `.apk` of 0.9.0 was installed and run by hand
  in a clean Alpine 3.23.3 container (podman, 2026-10-22): its checksum matched
  the release's, it installed, and a detached session's output, `display -p` and
  `kill-server` worked. The macOS release binaries were not run outside CI's
  smoke step.
