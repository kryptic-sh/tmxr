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

## Plan sections that the implementation changed

Update these plan files to match the code (or change the code back):

- **02-architecture**: the server uses plain threads and an `mpsc` channel, not
  tokio; no `tokio` dependency. The theme is not an `hjkl-theme` file — the
  Tokyo Night palette lives as `@thm_*` user options in
  `crates/tmxr-config/defaults.toml` and the status formats reference them.
- **03-protocol**: the client sends its whole environment in `Hello.env`; the
  server keeps only `update-environment` names for sessions.
- **08-copy-mode**: implemented with tmxr's own motions over a snapshot grid
  (route 2), not the hjkl engine. Motion set: h j k l w b e W B E 0 ^ $ g G H M
  L, C-u/d/b/f/y/e, `/` `?` `n` `N` (literal, smart-case), v / V / C-v.
- **10-config**: `prefix Y` uses `set-buffer -w "#{pane_current_path}"`; tmxr's
  `set-buffer` format-expands its data (tmux's does not).
- **11-resurrect**: panes restore the program by name only (no arguments); saves
  happen on server exit and every `auto-save-minutes`.

## Known gaps and follow-ups

- **Restore-on-start applies to every socket label**, including throwaway `-L`
  servers; the save on exit likewise overwrites `last` from whichever server
  exits. Consider per-label save directories.
- **`swap-pane -s`**, **`join-pane`**, **`move-window`**, **`swap-window`**,
  **`respawn-pane`**, `remain-on-exit`, `display-panes` overlay (currently
  prints indices), clock mode, emacs copy mode: not implemented.
- **tmux-yank `prefix y`** (copy the shell's command line) is not bound.
- **Mouse capture is always on in the client**; with `mouse = false` the server
  ignores mouse events but the terminal's own selection still needs Shift.
- **Windows `kill()` on a pane child** returned "There are no more files" (os
  error 18) in a test; panes are closed by dropping the ConPTY instead. Check
  whether `kill-pane` actually terminates programs on Windows.
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
