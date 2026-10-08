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
- **Verified end to end on Windows** by `apps/tmxr/tests/e2e.rs` (real binary in
  a ConPTY, shown to fail when an assertion is broken): attach, typing into the
  pane, `prefix %` split, `prefix h`, the `C-l` navigator in a plain shell,
  `prefix d` detach, `ls`, reattach with output preserved, and command clients
  starting and stopping a server.
- **Not yet verified anywhere**: how the status line and borders look in a real
  terminal emulator, mouse, copy mode by keyboard, the picker by keyboard
  (unit-tested only), resurrect restore of multi-pane layouts, and the navigator
  passing keys through to a real hjkl/vim.
- **Not run on Linux/macOS locally**: the `cfg(unix)` arms (socket dir checks,
  peer uid, `setsid` spawn, `/proc` and `proc_pidinfo` inspection) are only
  compiled and tested by CI.
- CI (commit a6648ab) runs the unit and e2e tests green on Linux, macOS and
  Windows; the Unix arms are exercised there, not locally.

## Known gaps and follow-ups

- **Commands not implemented**: `join-pane`, `move-window`, `swap-window`,
  `respawn-pane`, `save-buffer`, `load-buffer`, `if-shell` (all listed in
  plan/10's MVP set), `swap-pane -s`, `remain-on-exit`, `display-panes` overlay
  (currently prints indices), clock mode, emacs copy mode.
- **Copy mode gaps** (plan/08): counts, `f`/`t`/`F`/`T`/`;`/`,`, marks, `%`,
  `copy-pipe*`, `copy-command`, `refresh-from-pane`, highlighting of search
  matches.
- **Key notation**: hjkl's `<C-x>` form is not accepted (plan/01 and plan/06
  once promised it); mouse events are built into `mouse.rs` and cannot be
  rebound (`MouseDown1Pane` and friends are not key names).
- **Session picker gaps** (plan/09): no `C-x` kill / `C-r` rename actions, no
  create-on-unmatched-`Enter`, no preview of the selected session's window.
- **No 256-colour fallback**: colours are always 24-bit RGB. A terminal without
  true colour gets wrong colours.
- **No frame-rate cap**: the server renders after each drained batch of events.
  A pane producing output continuously renders as often as batches arrive; not
  measured.
- **Resurrect**: pane titles and a window's last pane are not saved; a missing
  directory falls back to `$HOME` without a message; auto-save writes even when
  nothing changed; an argument allowlist (`resurrect.restore-args`, like
  resurrect's `~vim` strategies) is not implemented; nothing saves on a signal.
- **`server.rs` and `cmds.rs` keep growing** (each over a thousand lines). Split
  along their seams — client lifecycle and attach out of `server.rs`, command
  families (session/window/pane/buffer/options) out of `cmds.rs` — as part of
  the next change that touches them.
- **tmux-yank `prefix y`** (copy the shell's command line) is not bound.
- **Mouse capture is always on in the client**; with `mouse = false` the server
  ignores mouse events but the terminal's own selection still needs Shift.
- **Windows `kill()` on a pane child** returned "There are no more files" (os
  error 18) in a test; not a bug in practice. `Server::kill_pane` calls `kill()`
  and then drops the pane, closing the ConPTY, and that does end the programs
  (which of the two does it was not isolated): verified by hand on 2026-10-09
  with the real binary, both a pane running `ping -t` directly and `cmd.exe`
  running it (no `PING.EXE` left 2–3 s after `kill-pane`).
- **Windows foreground process** = newest direct child of the pane's process
  that is ≥ 0.5 s old (skips prompt helpers like starship). A program launched
  less than 0.5 s before `C-h` is not yet seen as in front.
- **DCS passthrough** (`allow-passthrough on` in the tmux config, used by image
  protocols) is not supported by `vt100`; needs a pre-parser or a vt100 patch.
- **Windows `pane_current_path`** relies on shell integration (OSC 7 / OSC 9;9);
  without it splits open in the pane's start directory.
- **Windows pipe DACL** `D:P(A;;GA;;;OW)(A;;GA;;;SY)` was verified to admit the
  owner; that it rejects another user is unverified.

## Decisions awaiting the owner

The provisional decisions in
[plan/16-open-questions.md](plan/16-open-questions.md) (index base, `prefix X`
kill-pane, theme, config format, vim navigation, picker mode, Windows current
directory) are defaults the implementation follows until answered.

## Cross-repo work

- **hjkl `$TMXR` fall-through.** `dispatch_tmux_navigate` in
  `hjkl/apps/hjkl/src/app/window.rs` only hands off to `tmux select-pane` when
  `$TMUX` is set. It needs the same branch for `$TMXR` → `tmxr select-pane` for
  the `C-h/j/k/l` navigator to cross from hjkl splits into tmxr panes (milestone
  M4). The tmxr side (`navigate-pane`, `TMXR`/`TMXR_PANE`, command clients
  targeting their own pane) is implemented.
- **nvim navigation.** vim-tmux-navigator shells out to `tmux`; a snippet or
  plugin option calling `tmxr select-pane` is needed for plain nvim.

## Not yet verified

- The release pipeline (phase 2 of plan/14) does not exist yet; nothing about
  packaging has been exercised.
