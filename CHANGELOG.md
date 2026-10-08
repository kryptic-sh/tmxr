# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **First implementation of the tmxr client/server.** A detached server is
  started on demand and owns sessions, windows and panes (openpty / ConPTY,
  `vt100` emulation); clients attach over a Unix socket or an owner-only named
  pipe, or run one command (`tmxr ls`, `tmxr split-window -h`, …) in tmux's
  command language.
- **Built-in defaults port the owner's tmux config**: vim pane selection, splits
  and new windows in the current directory, `M-h`/`M-l`, synchronize panes on
  `prefix x`, vi copy mode with `v`/`C-v`/`y`, tmux-sensible binds,
  vim/hjkl-aware `C-h/j/k/l` navigation, Tokyo Night status line in the
  catppuccin layout. Overridable from `~/.config/tmxr/config.toml`.
- **Fuzzy session and window picker** (`prefix s` / `prefix w`) on
  `hjkl-picker`, moving with `j`/`k` or arrows.
- **Save and restore of sessions** (`prefix C-s` / `C-r`), auto-save and restore
  when the server starts.
