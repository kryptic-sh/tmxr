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
  PowerShell still needs the README's prompt snippet. The PEB read of a 32-bit
  (WOW64) program is not verified, nor of an elevated one (expected to fail to
  open, falling back to the start directory).

- **One `leaky` test, unidentified.** A cold workspace `cargo nextest run` on
  Windows on 2026-10-09 reported "80 passed (1 leaky)" (a test's child process
  still held its output after it ended); four reruns that day and seven more
  later were clean (those later runs did turn up a flaky test, since fixed), and
  nextest's summary did not name the leaking test. Likely a server or pane
  process outliving `kill-server` by a moment; not investigated further. It came
  back once more on 2026-10-09 (143 tests, "1 leaky") and not in the 4 runs
  after. Next time, run without filtering: nextest's `LEAK` line names the test.
- **`session_picker_previews_the_highlighted_session` timed out once** (30 s) in
  a full Windows run on 2026-10-09 that took 50 s instead of the usual 20 s, and
  passed on nextest's retry. 15 isolated runs and 5 more full runs were clean.
  Not investigated; the timeout message was not captured.

## Decisions awaiting the owner

The provisional decisions in
[plan/16-open-questions.md](plan/16-open-questions.md) (index base, `prefix X`
kill-pane, theme, config format, vim navigation, picker mode, Windows current
directory) are defaults the implementation follows until answered.

## Release channels not set up

v0.1.0 ships to GitHub Releases only (the owner's call, 2026-10-09). Left out of
hrdr's pipeline, with what each needs:

- **crates.io**: the name `tmxr` belongs to another project (slaptijack's tmux
  workspace launcher, crates.io owner `slaptijack`); `tmxr-proto` …
  `tmxr-client` were free. Needs a package name decided (e.g. `tmxr-cli`
  installing the `tmxr` binary) plus hrdr's `publish-crates` job with its
  topological order.
- **AUR (`tmxr-bin`), Homebrew tap, Scoop bucket**: need kryptic-sh/tmxr added
  to the org secrets `AUR_SSH_KEY`, `BREW_SSH_KEY` and `SCOOP_SSH_KEY`, which
  are limited to selected repos (changing that needs `admin:org`), then hrdr's
  `aur-bin`, `brew-tap`, `scoop-bucket` jobs and `pkg/` templates renamed.
- **Alpine `.apk`**: needs only hrdr's `alpine` job and
  `pkg/alpine/APKBUILD.in`.
- **Site entry** on kryptic-sh.github.io (plan/14 lists the files).

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
