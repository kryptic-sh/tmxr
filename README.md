# tmxr

A tmux-style terminal multiplexer for Linux, macOS and Windows. Rust binary.
Client/server.

[![CI](https://github.com/kryptic-sh/tmxr/actions/workflows/ci.yml/badge.svg)](https://github.com/kryptic-sh/tmxr/actions/workflows/ci.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Sessions, windows and panes that outlive the terminal you started them in, built
on the [hjkl](https://github.com/kryptic-sh/hjkl) crates. Part of the
[kryptic.sh](https://kryptic.sh) suite.

## Status

**Early, unreleased.** The MVP in [the plan](docs/plan/00-index.md) is
implemented and tested end to end on Linux, macOS and Windows in CI, but there
is no release yet: build it from source. What is missing or behaves differently
from tmux is listed in [docs/backlog.md](docs/backlog.md).

## What it does

- A long-lived **server** owns every session, window, pane and child process;
  **clients** attach from any terminal and detach without killing anything — the
  tmux model, on all three platforms (openpty and Unix sockets; ConPTY and an
  owner-only named pipe on Windows). The server starts on demand.
- tmux's **command language**, key names and formats:
  `tmxr split-window -h -c '#{pane_current_path}'` means what it means in tmux,
  on the command line, in binds and at the `prefix :` prompt.
  `tmxr list-commands` lists the commands.
- **Defaults are an opinionated tmux setup**: vim-style pane selection
  (`prefix h/j/k/l`), splits and new windows in the current directory
  (`prefix '` `"` `;` `%` `c`), `M-h` / `M-l` to cycle windows, synchronized
  panes on `prefix x`, vi copy mode with `v` / `C-v` / `y`, tmux-sensible and
  tmux-yank binds, and tmux's own default binds. `prefix ?` lists every bind
  with a note.
- **vim / hjkl navigation built in**: `C-h/j/k/l` move between panes, or go to
  the program in front when it is vim, hjkl or fzf — no plugin needed.
- **Fuzzy pickers** on `hjkl-picker`: sessions (`prefix s`), windows
  (`prefix w`), paste buffers (`prefix =`), and `prefix f` to find a window.
  Type to filter; `Escape` then `j` / `k` to move. In the session picker `C-x`
  kills, `C-r` renames, and `Enter` on a new name creates that session.
- **Tokyo Night status line** in the catppuccin layout, built from tmux formats
  you can restyle.
- **Session save / restore** like tmux-resurrect: `prefix C-s` / `C-r`,
  auto-save, and restore when the server starts. Each server (`-L` label) keeps
  its own saves.
- Mouse (click to focus, drag borders, wheel into copy mode, drag to copy), OSC
  52 and the local clipboard, and tmux passthrough (`allow-passthrough`) for
  inline images — Unix only; see the backlog for why not Windows.

## Using it

```sh
tmxr                     # new session (or attach to restored ones)
tmxr new -s work         # named session
tmxr attach -t work      # attach; `prefix d` detaches
tmxr ls                  # list sessions
tmxr -L scratch          # a separate server with its own sessions
tmxr kill-server
```

The prefix is `C-b`. Inside a pane, `tmxr <command>` talks to the server that
pane belongs to (via `$TMXR`), as `tmux` does.

For hjkl's own splits to hand `C-h/j/k/l` over to tmxr panes, hjkl needs its
`$TMXR` handoff, which is not in an hjkl release yet.

## Configuration

Settings and binds live in `~/.config/tmxr/config.toml` (on every OS), layered
over the built-in defaults in
[crates/tmxr-config/defaults.toml](crates/tmxr-config/defaults.toml). Binds are
tmux command strings:

```toml
mouse = false

[keys.prefix]
x = false                                                  # remove a default
"|" = { cmd = "split-window -h", note = "Split side by side" }
```

`prefix R` (or `tmxr source-file`) reloads it.

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
