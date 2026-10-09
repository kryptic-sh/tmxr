# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Inline images on Windows**: the Windows release bundles Microsoft's ConPTY
  (`conpty.dll`, `OpenConsole.exe`, MIT), which keeps the end of a program's
  tmux passthrough that Windows' own ConPTY drops; tmxr turns passthrough on
  when they sit beside `tmxr.exe`.
- **Windows: sessions are saved at logoff and shutdown**, as SIGTERM / SIGHUP
  already did on Unix. The server keeps a hidden window for Windows' session-end
  message; before, only the periodic auto-save protected them.

## [0.2.2] - 2026-10-09

### Added

- `select-pane -Z` keeps a zoomed window zoomed on the pane it selects, as in
  tmux (tmxr-navigator.nvim's `preserve_zoom` uses it).

## [0.2.1] - 2026-10-09

### Fixed

- Windows panes keep the environment of the shell that started tmxr. They used
  to get the registry's variables in its place, so a `PATH` extended before
  starting tmxr (a venv, a toolchain, a dev build) was lost in every pane.

## [0.2.0] - 2026-10-09

### Added

- **Mouse keys**: `MouseDown1Pane`, `MouseDrag1Border`, `MouseDragEnd1Pane`,
  `WheelUpPane`, `MouseDown1Status` and the rest of tmux's mouse key names can
  be bound and unbound in any key table, with `-t =` for the pane or window
  under the mouse and the `-M` flags of `send-keys`, `copy-mode` and
  `resize-pane`. The mouse behaviour that was built in is now default binds, so
  `unbind -n WheelUpPane` (for example) turns one part of it off.
- `copy-mode -e`: scrolling back to the bottom leaves copy mode, as the wheel
  does. The flag was accepted before but did nothing.
- `#{mouse_any_flag}`: whether the pane's program asked for the mouse.
- **Emacs copy mode**: `mode-keys emacs` switches copy mode to tmux's emacs
  table (`copy-mode`: `C-Space`, `M-w`, `C-s` / `C-r`, `M-1`… counts, …).
  `mode-keys` now rejects values other than `vi` and `emacs`.
- **Resurrect restores arguments** for programs in the new
  `resurrect.restore-args` list (tmux-resurrect's defaults and `hjkl`), so
  `less app.log` comes back as `less app.log`, not `less`.
- Copy-mode commands `previous-matching-bracket`, `next-paragraph` /
  `previous-paragraph`, `goto-line` and `copy-end-of-line` (with its
  `-and-cancel` and `copy-pipe-` forms), bound as in tmux: `{` `}` `:` `D` in vi
  mode, `C-M-b` `M-{` `M-}` `g` `C-k` in emacs mode.
- `DoubleClick1Pane` / `TripleClick1Pane` mouse keys, bound by default to copy
  the word or line clicked (the copy-mode `select-word` command is new too).
- **Windows: splits and new windows open in cmd's current directory** without
  any shell integration: tmxr reads the directory from the program's process
  (its PEB). What a shell announces (OSC 7 / OSC 9;9) still comes first, which
  PowerShell needs.
- **Incremental search**: emacs copy mode's `C-s` / `C-r` move to the match as
  you type, through `command-prompt -i` and the `search-forward-incremental` /
  `search-backward-incremental` copy commands.
- nvim support for `C-h/j/k/l` lives in the new
  [tmxr-navigator.nvim](https://github.com/kryptic-sh/tmxr-navigator.nvim)
  plugin, tmxr's vim-tmux-navigator.

### Changed

- **Pickers work like hjkl's**: one mode, where typing always filters, arrows
  and `C-n` / `C-p` move and `Escape` closes. The vi-style normal mode (`j` /
  `k`, `g` / `G`, `C-j` / `C-k`) is gone.

### Fixed

- A pane printing faster than the server can parse (`seq` on Linux) no longer
  delays key presses by seconds or grows the server's memory without bound: each
  pane's output waits in a bounded queue that slows the program down, and the
  server repaints during a flood.
- `C-Space` binds now fire on Windows, where the terminal reports the key (byte
  0x00) as `C-2`; `C-@` names the same key, as in tmux.
- A save made right after a program started (`prefix C-s`) now records it; saves
  used to read a list of foreground programs refreshed only on the status tick.

## [0.1.0] - 2026-10-09

### Added

- **First implementation of the tmxr client/server.** A detached server is
  started on demand and owns sessions, windows and panes (openpty / ConPTY,
  `vt100` emulation); clients attach over a Unix socket or an owner-only named
  pipe, or run one command (`tmxr ls`, `tmxr split-window -h`, …) in tmux's
  command language.
- **Built-in defaults port the owner's tmux config**: vim pane selection, splits
  and new windows in the current directory, `M-h`/`M-l`, synchronize panes on
  `prefix x`, vi copy mode with `v`/`C-v`/`y`, tmux-sensible and tmux-yank
  binds, tmux's own default binds (`display-panes` numbers on `prefix q`, alert
  windows on `M-n`/`M-p`, even spread on `prefix E`, …), vim/hjkl-aware
  `C-h/j/k/l` navigation, Tokyo Night status line in the catppuccin layout.
  Overridable from `~/.config/tmxr/config.toml`.
- **The tmux command set of the plan's MVP**, including moving windows and panes
  between sessions (`move-window`, `swap-window`, `join-pane`, `swap-pane -s`),
  `respawn-pane`, `if-shell` and buffers to and from files.
- **Fuzzy pickers** on `hjkl-picker` for sessions (`prefix s`), windows
  (`prefix w`, and `prefix f` to find one) and paste buffers (`prefix =`),
  moving with `j`/`k` or arrows.
- **Mouse** (`mouse on`): click to focus, drag borders, wheel into copy mode,
  drag to copy. With `mouse off` the client leaves the mouse to the terminal.
- **tmux passthrough** (`allow-passthrough`, on by default as in the tmux
  config) forwards programs' `ESC P tmux; …` sequences, such as inline images,
  to the outer terminal. Unix only: ConPTY drops the sequence's terminator.
- **Save and restore of sessions** (`prefix C-s` / `C-r`), auto-save and restore
  when the server starts. Each server (`-L` label or `-S` socket) keeps its own
  saves.
- **Release binaries** for Linux (glibc 2.28 and musl, x86_64 and aarch64, plus
  `.deb` and `.rpm`), macOS (Apple silicon and Intel) and Windows (x86_64),
  published to GitHub Releases with `.sha256` files, and shell completions and a
  man page from the hidden `tmxr --completions <shell>` and `tmxr --man`.

[Unreleased]: https://github.com/kryptic-sh/tmxr/compare/v0.2.2...HEAD
[0.2.2]: https://github.com/kryptic-sh/tmxr/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/kryptic-sh/tmxr/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/kryptic-sh/tmxr/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/kryptic-sh/tmxr/releases/tag/v0.1.0
