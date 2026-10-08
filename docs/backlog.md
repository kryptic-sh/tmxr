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
- CI (commit db38208) runs the unit and e2e tests green on Linux, macOS and
  Windows; the Unix arms are exercised there, not locally.

## Known gaps and follow-ups

- **Not implemented**: emacs copy mode.
- **tmux default binds**: all of tmux's defaults are bound; `prefix C-z`
  (suspend-client) errors on Windows, which has no job control. Alerts for `M-n`
  / `M-p` are bells only (no `monitor-activity` / `monitor-silence`).
  `find-window` matches window names through the picker, not pane contents or
  titles as tmux's does.
- **Mouse binds**: mouse events are built into `mouse.rs` and cannot be rebound
  (`MouseDown1Pane` and friends are not key names).
- **No 256-colour fallback, by design for now**: colours are always 24-bit RGB,
  which is what the tmux config asks for (`terminal-overrides ",*:RGB"` forces
  RGB on every terminal). Only for a user whose terminal lacks true colour (e.g.
  macOS Terminal.app) would a fallback matter: map RGB to the nearest xterm-256
  index in `AnsiBackend` when the client's environment does not advertise true
  colour (`COLORTERM`, `WT_SESSION`, …), behind an option.
- **No frame-rate cap**: the server renders after each drained batch of events.
  Measured on Windows (2026-10-09, a pane echoing a counter in an endless cmd
  loop): about 19 KB/s of frames reached the client and `prefix d` still
  detached in 50 ms, so it is not a problem there; ConPTY itself coalesces
  output. A Unix pty can produce far more and is not measured.
- **Resurrect**: pane titles are not saved (restored programs set their own); an
  argument allowlist (`resurrect.restore-args`, like resurrect's `~vim`
  strategies) is not implemented, and would need each pane's full command line
  (`/proc/<pid>/cmdline`, `KERN_PROCARGS2`, a Windows process's PEB). On Windows
  nothing saves when the machine shuts down.
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
- **Passthrough on Windows is unsupported.** ConPTY forwards a program's DCS but
  drops its `ESC \` terminator (seen with PowerShell writing one on Windows 11
  build 26300), so `tmxr_term::emulator::PASSTHROUGH_SUPPORTED` is false there
  and `allow-passthrough` only logs that. Revisit if ConPTY changes. The Unix
  forwarding is exercised by CI only, and not with a real image protocol.
- **Windows `pane_current_path`** relies on shell integration (OSC 7 / OSC 9;9);
  without it splits open in the pane's start directory. The README's PowerShell
  prompt snippet provides it (verified by hand: after `cd C:\Windows` in such a
  pane, `#{pane_current_path}` read `C:\Windows`); cmd.exe has no such hook.
- **Windows pipe DACL** `D:P(A;;GA;;;OW)(A;;GA;;;SY)` was verified to admit the
  owner; that it rejects another user is unverified.

- **One `leaky` test, unidentified.** A cold workspace `cargo nextest run` on
  Windows on 2026-10-09 reported "80 passed (1 leaky)" (a test's child process
  still held its output after it ended); four reruns that day and seven more
  later were clean (those later runs did turn up a flaky test, since fixed), and
  nextest's summary did not name the leaking test. Likely a server or pane
  process outliving `kill-server` by a moment; not investigated further.

## Decisions awaiting the owner

The provisional decisions in
[plan/16-open-questions.md](plan/16-open-questions.md) (index base, `prefix X`
kill-pane, theme, config format, vim navigation, picker mode, Windows current
directory) are defaults the implementation follows until answered.

## Cross-repo work

- **hjkl `$TMXR` fall-through: committed in hjkl, not pushed.** hjkl commit
  `074ef614` ("feat(app): hand edge navigation to tmxr panes", on hjkl `main`,
  one ahead of `origin/main`) makes `dispatch_tmux_navigate` run
  `tmxr select-pane` when `$TMXR` is set, checked before `$TMUX` (the decision
  is `handoff_program` in `apps/hjkl/src/app/window.rs`, unit-tested). hjkl's
  workspace fmt, clippy and tests passed locally (Windows). Left unpushed
  because pushing to hjkl was not part of the tmxr work the owner authorised;
  push it, then milestone M4's navigator crosses from hjkl splits into tmxr
  panes. Not tested end to end with a real hjkl in a tmxr pane.
- **nvim navigation.** vim-tmux-navigator shells out to `tmux`; a snippet or
  plugin option calling `tmxr select-pane` is needed for plain nvim.

## Not yet verified

- The release pipeline (phase 2 of plan/14) does not exist yet; nothing about
  packaging has been exercised.
