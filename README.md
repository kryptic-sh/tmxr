# tmxr

A tmux-style terminal multiplexer for Linux, macOS and Windows. Rust binary.
Client/server.

[![CI](https://github.com/kryptic-sh/tmxr/actions/workflows/ci.yml/badge.svg)](https://github.com/kryptic-sh/tmxr/actions/workflows/ci.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Sessions, windows and panes that outlive the terminal you started them in, built
on the [hjkl](https://github.com/kryptic-sh/hjkl) crates. Part of the
[kryptic.sh](https://kryptic.sh) suite.

## What it will do

- A long-lived **server** owns every session, window, pane and child process;
  **clients** attach from any terminal and detach without killing anything — the
  tmux model, on all three platforms (ConPTY + named pipes on Windows).
- tmux's command language and key names:
  `tmxr split-window -h -c '#{pane_current_path}'` means what it means in tmux.
- Ships an opinionated tmux setup as its defaults: vim-style pane selection,
  splits and new windows in the current directory, `M-h`/`M-l` window cycling,
  vi copy mode, synchronized panes on `prefix x`.
- Built-in vim/hjkl navigation: `C-h/j/k/l` move between panes, or pass through
  to vim / hjkl / fzf when one is in front — no plugin needed.
- A fuzzy session picker (`prefix s`) on `hjkl-picker`.
- Tokyo Night status line.
- Session save/restore à la tmux-resurrect.

## Status

**Pre-alpha: nothing works yet.** This repository holds the plan and the
workspace scaffold; the binary parses its command line and reports that the
server is not implemented. The design lives in
[docs/plan/](docs/plan/00-index.md) and the build order in
[docs/plan/15-milestones.md](docs/plan/15-milestones.md).

## Building

```sh
cargo build --release
```

The workspace MSRV is **Rust 1.95** (`rust-version` in `Cargo.toml`).

## Layout

| Crate          | Role                                                                     |
| -------------- | ------------------------------------------------------------------------ |
| `tmxr`         | Binary (`apps/tmxr`) — CLI, dispatch to client / command client / server |
| `tmxr-proto`   | Client/server wire protocol                                              |
| `tmxr-command` | tmux-compatible command language, key names, formats                     |
| `tmxr-config`  | TOML config and the built-in default binds                               |
| `tmxr-term`    | One pane's terminal: PTY, vt100, input encoding, process inspection      |
| `tmxr-server`  | Sessions, windows, panes, key tables, rendering                          |
| `tmxr-client`  | Attach client: terminal setup, input pump, output writer                 |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) or open an issue / PR.

## License

MIT. See [LICENSE](LICENSE).
