# 04 — Panes and terminal emulation (`tmxr-term`)

## Spawning

- `portable-pty`'s `native_pty_system()` → `openpty(size)` → `spawn_command`.
- Command: the pane's command if given, else `default-command`, else
  `default-shell`, else `$SHELL` (Unix) / `pwsh.exe` → `powershell.exe` →
  `%COMSPEC%` (Windows, first found on `PATH`).
- Working directory: `-c` if given (after `#{format}` expansion), else the
  session's start directory. `-c '#{pane_current_path}'` is what every split and
  new-window bind in the default config uses, so the current-path lookup below
  is MVP-critical.
- Environment: the server's global environment, overlaid by the session's
  (captured from the creating client's `update-environment` variables), plus
  `TERM=<default-terminal>` (`tmux-256color` in the config; falls back to
  `xterm-256color` if the terminfo entry is missing — tmux has the same
  problem), `COLORTERM=truecolor`, `TMXR`, `TMXR_PANE`.

## Emulation

One `vt100::Parser` per pane, sized to the pane, with scrollback `history-limit`
(50 000 — tmux-sensible's value).

What vt100 gives us directly: the cell grid with attributes and colours, wide
characters, alternate screen, cursor position/visibility, application cursor and
keypad modes, bracketed paste mode, mouse protocol mode and encoding,
scrollback, title, bell.

What we hook through `vt100::Callbacks`:

| Callback                     | Use                                                                                                                |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `set_window_title`           | `pane_title`, `set-titles`                                                                                         |
| `audible_bell`               | bell flag on the window (status `#{window_bell_flag}`), bell forwarded to clients per `bell-action`                |
| `copy_to_clipboard` (OSC 52) | `set-clipboard on`: store a paste buffer **and** forward OSC 52 to attached clients (how tmux-yank-less apps copy) |
| `unhandled_osc`              | OSC 7 (`file://host/path`) and OSC 9;9 (Windows Terminal) → `pane_current_path` hint                               |
| `unhandled_csi`              | `CSI > 4 ; n m` (modifyOtherKeys) and `CSI > flags u` / `CSI < u` (kitty keys) → the pane's extended-keys state    |

Not supported by vt100 and deferred: DCS passthrough (`allow-passthrough on` in
the config, used for image protocols) — recorded in the backlog, since it needs
either a vt100 patch or a pre-parser that splits passthrough sequences out of
the stream before vt100 sees them.

## Input encoding (key → bytes)

The server encodes a crossterm `KeyEvent` for a pane from that pane's modes:

- Printable characters → UTF-8. `Alt` → `ESC` prefix.
- `Ctrl` + letter / `@[\]^_` → C0 control. `C-h` is `0x08`.
- Cursor keys → `CSI A` or `SS3 A` depending on application-cursor mode;
  Home/End/PageUp/… and F-keys per xterm.
- Modified special keys → xterm `CSI 1;<mod>X` / `CSI <n>;<mod>~`.
- Keys with no legacy encoding (`C-;`, `C-S-h`, `C-Enter`) → `CSI <code>;<mod>u`
  when the pane enabled kitty keys or modifyOtherKeys, or always when
  `extended-keys always` (the config's setting); dropped otherwise.
- Paste → wrapped in `ESC [200~ … ESC [201~` when the pane enabled bracketed
  paste.
- Mouse → re-encoded relative to the pane's origin in the pane's requested
  protocol (X10 / normal / button / any-event; default / UTF-8 / SGR). Mouse
  events for panes that did not ask for the mouse drive tmxr itself (focus,
  resize drag, wheel → copy mode).

This is the piece with the most edge cases, so it is a pure function
(`encode_key(&KeyEvent, &PaneModes) -> Vec<u8>`) with a table-driven test
against xterm's documented sequences.

## Process inspection

Needed for the navigator (`pane_current_command`) and for
`#{pane_current_path}`:

| Platform | Foreground process                                                                     | Current directory                                                      |
| -------- | -------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| Linux    | `tcgetpgrp` on the PTY master (`MasterPty::process_group_leader`) → `/proc/<pid>/comm` | `/proc/<pid>/cwd`                                                      |
| macOS    | same pgrp → `proc_pidpath` / `proc_name`                                               | `proc_pidinfo(PROC_PIDVNODEPATHINFO)`                                  |
| Windows  | deepest descendant of the pane's child in a `CreateToolhelp32Snapshot` walk            | OSC 7 / OSC 9;9 from the shell; else the directory the pane started in |

The Windows current-directory gap is real (reading another process's PEB is
fragile and needs elevated rights for some processes). The shell-integration OSC
route is the supported answer; the plan ships a PowerShell/pwsh prompt snippet
in the README that emits OSC 7, and records the gap in the backlog.

## Pane lifecycle

- Exit → `PaneExited`. With `remain-on-exit off` (default) the pane is closed
  and the layout reflows; closing the last pane closes the window, the last
  window closes the session, the last session exits the server (tmux default
  `exit-empty on`).
- Resize → `MasterPty::resize` + `Parser::set_size`. Coalesced: one resize per
  render, not per drag event.
- `respawn-pane`, `remain-on-exit` are post-MVP.
