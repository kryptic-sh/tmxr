# 01 — Overview

## What tmxr is

`tmxr` is a terminal multiplexer written in Rust: a tmux alternative built from
the kryptic-sh house crates (the `hjkl-*` family in particular). Like tmux it is
a **client/server** program — a long-lived server owns every session, window,
pane and child process; thin clients attach to it from any terminal, render what
the server sends and forward input. Detaching a client leaves everything
running.

It targets **Linux, macOS and Windows** as first-class platforms from day one.

## Baseline

The feature baseline is two things:

1. **tmux itself** — the base feature set (sessions, windows, panes, splits,
   layouts, copy mode, buffers, command prompt, the `tmux <command>` CLI) is the
   source of inspiration for anything not spelled out below.
2. **mxaddict's tmux config** —
   [`dotfiles/.config/tmux/tmux.conf`](https://github.com/mxaddict/dotfiles/blob/main/.config/tmux/tmux.conf)
   and its plugins (`catppuccin/tmux`, `christoomey/vim-tmux-navigator`,
   `tmux-resurrect`, `tmux-sensible`, `tmux-yank`, `tpm`), plus
   `plugin-notes.sh` which documents every plugin bind. tmxr ships that setup
   **as its built-in defaults**: a fresh install with no config file behaves
   like that tmux config. The plugins are not plugins in tmxr — their behaviour
   is native.

The full key table derived from that config is in
[06-keys-and-bindings.md](06-keys-and-bindings.md).

## MVP scope

The MVP is a usable daily driver for that config. In scope:

1. **Server** — background process owning sessions/windows/panes, started on
   demand by the first client, survives detach, `kill-server` to stop.
2. **Client that renders the session** — attach/detach, resize, full-colour
   rendering of panes, borders and the status line, mouse.
3. **Session switcher** — `prefix s` opens a fuzzy picker over sessions built on
   `hjkl-picker` / `hjkl-fuzzy`; move with `j`/`k` or arrow keys, type to
   filter, `Enter` to switch ([09-session-picker.md](09-session-picker.md)).
4. **vim / hjkl navigation** — `C-h/j/k/l` (and `C-\`) move between tmxr panes,
   or pass the key through to vim / fzf / hjkl / sqeel when one is in the
   foreground, exactly like `vim-tmux-navigator`
   ([06-keys-and-bindings.md](06-keys-and-bindings.md#vim--hjkl-navigation)).
5. **Tokyo Night colour theme** — the palette the rest of the desktop uses
   (alacritty, fish, bat, nvim: folke's "Night" variant).
6. **Status bar** — same layout and window ("tab") rules as the current tmux
   config: catppuccin's `basic` window style, empty `status-left`, session and
   host modules on the right, `█` separators
   ([07-rendering-status-theme.md](07-rendering-status-theme.md)).

All the binds from the tmux config (vim pane selection, `M-h`/`M-l` window
cycling, `'` `"` `;` `%` splits in the current directory, `c` new window in the
current directory, `x` synchronize-panes, vi copy mode with `v` / `C-v` / `y`)
are MVP.

### Stretch (MVP if it fits)

- **Session save/restore** à la `tmux-resurrect`, including auto-restore when
  the server starts with nothing to attach to
  ([11-resurrect.md](11-resurrect.md)).
- **Clipboard integration** à la `tmux-yank` beyond plain `y` in copy mode
  (`prefix Y` copy the pane's directory, `Y` / `M-y` copy-and-paste).
- `tmux-sensible`'s binds and options (they cost almost nothing once the key
  tables exist, so they are planned as MVP defaults).

## Non-goals (for now)

- **tmux wire or socket compatibility.** tmxr cannot talk to a tmux server and
  tmux cannot talk to tmxr. The _command language_ is tmux-compatible
  (`split-window -h -c '#{pane_current_path}'` means the same thing); the
  protocol is not.
- **Reading `tmux.conf`.** Config is TOML, house style
  ([10-config-and-commands.md](10-config-and-commands.md)). Binds inside it are
  tmux command strings, so porting a config is mechanical.
- **A plugin system / tpm.** The plugins that matter are built in.
- Control mode (`tmux -CC`), `pipe-pane`, `link-window`, nested-session
  niceties, multiple clients sharing one session with different sizes beyond
  tmux's `aggressive-resize` behaviour. Deferred to the backlog.

## Decisions taken in this plan

| Decision          | Choice                                                 | Why                                                                                   |
| ----------------- | ------------------------------------------------------ | ------------------------------------------------------------------------------------- |
| Process model     | tmux-style server + client, server spawned on demand   | Requested; detach/attach is the point of a multiplexer                                |
| Who renders       | **Server** composes each client's frame, client blits  | Same as tmux; keeps the client trivial and every platform identical                   |
| Terminal emulator | `vt100` 0.16 per pane                                  | Org precedent (hjkl/gpur/hrdr tests); exposes cells, modes, scrollback, OSC callbacks |
| PTY               | `portable-pty` 0.9 (openpty / ConPTY)                  | Org precedent; one API on all three platforms                                         |
| IPC transport     | `interprocess` 2 local sockets (UDS / named pipe)      | Org precedent (`buffr` single-instance); one API on all three platforms               |
| Wire encoding     | length-prefixed `postcard` frames (serde)              | Org precedent (`hjkl-app`); compact for frame data, no JSON-escaping of raw bytes     |
| Frame composition | `ratatui` `Buffer` + custom in-memory `Backend`        | Reuses ratatui diffing and the `hjkl-*-tui` widgets (picker, statusline, theme)       |
| Pane layout tree  | `hjkl-layout`                                          | Already has split/remove/neighbour/equalize/swap — what `select-pane -L` etc. need    |
| Key notation      | tmux names (`C-b`, `M-h`, `C-\`) in config and CLI     | Users port binds verbatim                                                             |
| Config            | TOML via `hjkl-config` / `hjkl-xdg`, defaults embedded | House style; `~/.config/tmxr/config.toml` on every OS                                 |
| Fuzzy picker      | `hjkl-picker` + `hjkl-fuzzy`                           | Requested                                                                             |
| Clipboard         | OSC 52 to the client tty, plus `hjkl-clipboard` local  | OSC 52 works over SSH; local copy covers terminals without OSC 52                     |
| Theme             | Tokyo Night (Night) palette, catppuccin `basic` layout | Requested: colours of the desktop, layout of the current tmux status line             |
| Index base        | `base-index 0`, `pane-base-index 0`                    | What the config actually sets (its comment says "start at 1"; the values say 0) — §16 |

## Naming

- Binary and repo: `tmxr`. Crates: `tmxr-*` under `crates/`, the binary under
  `apps/tmxr`.
- Environment inside panes: `TMXR` (`<socket>,<server pid>,<session id>`) and
  `TMXR_PANE` (`%<pane id>`), mirroring `TMUX` / `TMUX_PANE`.
- Config / data / state live under `tmxr` in the XDG-style directories
  `hjkl-xdg` resolves (`~/.config/tmxr`, `~/.local/share/tmxr`,
  `~/.local/state/tmxr`) on every platform.
