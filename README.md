# tmxr

A tmux-style terminal multiplexer for Linux, macOS and Windows. Rust binary.
Client/server.

[![CI](https://github.com/kryptic-sh/tmxr/actions/workflows/ci.yml/badge.svg)](https://github.com/kryptic-sh/tmxr/actions/workflows/ci.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Sessions, windows and panes that outlive the terminal you started them in, built
on the [hjkl](https://github.com/kryptic-sh/hjkl) crates. Part of the
[kryptic.sh](https://kryptic.sh) suite.

## Status

**Early: v0.1.0 is the first release.** The MVP in
[the plan](docs/plan/00-index.md) is implemented and tested end to end on Linux,
macOS and Windows in CI. What is missing or behaves differently from tmux is
listed in [docs/backlog.md](docs/backlog.md).

## What it does

- A long-lived **server** owns every session, window, pane and child process;
  **clients** attach from any terminal and detach without killing anything — the
  tmux model, on all three platforms (openpty and Unix sockets; ConPTY and an
  owner-only named pipe on Windows; `server-access` can admit other users). The
  server starts on demand.
- tmux's **command language**, key names and formats:
  `tmxr split-window -h -c '#{pane_current_path}'` means what it means in tmux,
  on the command line, in binds and at the `prefix :` prompt.
  `tmxr list-commands` lists the commands.
- **Defaults are an opinionated tmux setup**: vim-style pane selection
  (`prefix h/j/k/l`), splits and new windows in the current directory
  (`prefix '` `"` `;` `%` `c`), `M-h` / `M-l` to cycle windows, synchronized
  panes on `prefix x`, vi copy mode with `v` / `C-v` / `y` (or tmux's emacs
  table with `mode-keys emacs`), tmux-sensible and tmux-yank binds, and tmux's
  own default binds. `prefix ?` lists every bind with a note.
- **vim / hjkl navigation built in**: `C-h/j/k/l` move between panes, or go to
  the program in front when it is vim, hjkl or fzf. hjkl hands the keys back at
  its edge by itself; for nvim, install
  [tmxr-navigator.nvim](https://github.com/kryptic-sh/tmxr-navigator.nvim).
- **Fuzzy pickers** on `hjkl-picker`: sessions (`prefix s`), windows
  (`prefix w`), paste buffers (`prefix =`), and `prefix f` to find a window. As
  in hjkl: type to filter, arrows or `C-n` / `C-p` to move, `Escape` to close.
  In the session picker `C-x` kills, `C-r` renames, and `Enter` on a new name
  creates that session; the highlighted session's pane shows below the list.
- **Tokyo Night status line** in the catppuccin layout, built from tmux formats
  you can restyle.
- **Session save / restore** like tmux-resurrect: `prefix C-s` / `C-r`,
  auto-save, and restore when the server starts. Each server (`-L` label) keeps
  its own saves.
- Mouse (click to focus, drag borders, wheel into copy mode, drag to copy), all
  rebindable as tmux's mouse keys (`MouseDown1Pane`, `WheelUpPane`, …), OSC 52
  and the local clipboard, and tmux passthrough (`allow-passthrough`) for inline
  images. On Windows this needs the `conpty.dll` and `OpenConsole.exe` the
  release zip ships beside `tmxr.exe` (see Installing).

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

hjkl 0.42.2 or later hands `C-h/j/k/l` over to tmxr panes at the edge of its own
splits.

## Configuration

Settings and binds live in `~/.config/tmxr/config.toml` (on every OS), layered
over the built-in defaults in
[crates/tmxr-config/defaults.toml](crates/tmxr-config/defaults.toml). Binds are
tmux command strings, and keys are tmux's names (`C-b`, `M-h`) or hjkl's
(`<C-b>`, `<CR>`):

```toml
mouse = false

[keys.prefix]
x = false                                                  # remove a default
"|" = { cmd = "split-window -h", note = "Split side by side" }
```

`prefix R` (or `tmxr source-file`) reloads it.

## Windows: the current directory

New panes and windows open in the current pane's directory (`prefix "` `%` `'`
`;` `c` pass `-c "#{pane_current_path}"`, as the tmux config does). tmxr reads
that directory from the program in front: cmd's `cd` is picked up by itself.
PowerShell's `Set-Location` does not change its process's directory, so
PowerShell has to announce it; put this in your `$PROFILE` (Windows Terminal's
prompt snippet; tmxr reads the same OSC 9;9 sequence):

```powershell
function prompt {
  $loc = $executionContext.SessionState.Path.CurrentLocation
  $out = ""
  if ($loc.Provider.Name -eq "FileSystem") {
    $out += "$([char]27)]9;9;`"$($loc.ProviderPath)`"$([char]27)\"
  }
  $out += "PS $loc$('>' * ($nestedPromptLevel + 1)) "
  return $out
}
```

Shells that emit OSC 7 (`file://host/path`) work too. What a shell announces
wins over what tmxr reads from the process.

## Installing

```sh
# AUR (Arch Linux)
yay -S tmxr-bin

# Homebrew (macOS)
brew install kryptic-sh/tap/tmxr

# Scoop (Windows)
scoop bucket add kryptic-sh https://github.com/kryptic-sh/scoop-bucket
scoop install tmxr

# crates.io (the package is tmxr-cli; the binary is tmxr)
cargo install tmxr-cli

# Debian/Ubuntu, Fedora, Alpine: the .deb / .rpm / .apk from the latest release
```

Or download the archive for your platform from
[GitHub Releases](https://github.com/kryptic-sh/tmxr/releases) and put `tmxr`
(`tmxr.exe` on Windows) on your `PATH`. Each file has a `.sha256` beside it.
There are builds for Linux (x86_64 and aarch64, glibc 2.28+ or static musl, and
`.deb` / `.rpm` packages, which install completions and a man page), macOS
(Apple silicon and Intel) and Windows (x86_64).

The Windows zip also carries `conpty.dll` and `OpenConsole.exe`, Microsoft's
ConPTY (MIT, `ConPTY-LICENSE.txt`); keep them beside `tmxr.exe`. Windows' own
ConPTY drops the end of a program's tmux passthrough, so without them inline
images stay off. The Scoop package includes them; tmxr built from source
(`cargo install tmxr-cli`) uses Windows' own.

Shell completions and the man page come from the binary itself:
`tmxr --completions <bash|zsh|fish|powershell|elvish|nushell>` and `tmxr --man`.

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
