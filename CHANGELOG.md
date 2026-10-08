# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/kryptic-sh/tmxr/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/kryptic-sh/tmxr/releases/tag/v0.1.0
