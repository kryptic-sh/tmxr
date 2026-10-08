# 02 — Architecture

## Processes

```
 terminal A ──► tmxr client ─┐                       ┌─► pane %0  (pty ◄─► shell)
                             │   local socket /      │
 terminal B ──► tmxr client ─┼── named pipe ──► tmxr server ─► pane %1  (pty ◄─► hjkl)
                             │   (postcard frames)   │
 `tmxr split-window -h` ─────┘                       └─► pane %2  (pty ◄─► fish)
   (one-shot command client)
```

- **Server** (`tmxr` re-executed as the hidden `tmxr __server` subcommand). Owns
  all state, every PTY and every child process. One server per socket label
  (`-L name`, default `default`) or explicit path (`-S path`), as in tmux.
- **Attached client** (`tmxr`, `tmxr attach`, `tmxr new`). Puts the terminal in
  raw mode, forwards input events and size changes, writes whatever bytes the
  server sends straight to stdout.
- **Command client** (`tmxr <command> [args]`, e.g. from a script or from hjkl
  at a split edge). Connects, sends one command list, prints the reply, exits
  with the command's status.

The first client to find no server spawns one (detached from the terminal:
`setsid` on Unix, `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` on Windows) and
retries the connection with backoff. See [12-platforms.md](12-platforms.md).

## Workspace layout

Follows the org convention of `apps/<bin>` + `crates/<bin>-*`.

| Path                  | Crate          | Owns                                                                                                                                    |
| --------------------- | -------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `apps/tmxr`           | `tmxr`         | The binary: clap CLI, dispatch to client / command client / server, tracing setup                                                       |
| `crates/tmxr-proto`   | `tmxr-proto`   | Wire types (`ClientMsg`, `ServerMsg`), framing codec, protocol version, socket path resolution                                          |
| `crates/tmxr-command` | `tmxr-command` | tmux command language: tokenizer, command-list parser, command table + flag specs, key-name parsing, `#{format}` expansion              |
| `crates/tmxr-config`  | `tmxr-config`  | TOML schema, embedded defaults (the tmux.conf port), load/layer/validate via `hjkl-config`                                              |
| `crates/tmxr-term`    | `tmxr-term`    | One pane's terminal: PTY spawn (portable-pty), reader thread, `vt100` parser, OSC hooks, key/mouse → bytes encoding, process inspection |
| `crates/tmxr-server`  | `tmxr-server`  | Session/window/pane model, layouts, command execution, key tables, copy mode, picker, renderer/compositor, status line, resurrect       |
| `crates/tmxr-client`  | `tmxr-client`  | Terminal setup/teardown (raw mode, alt screen, mouse, kitty keys), event pump, output writer, server spawn + connect                    |

Splitting `tmxr-term` and `tmxr-command` out of the server is deliberate: both
are pure enough to unit-test without a server, and both are where most of the
correctness risk lives (byte encodings and parsing).

`tmxr-server` will grow; it is split internally by module from the start
(`model/`, `cmd/`, `render/`, `copy/`, `picker/`, `resurrect/`) so no file
becomes the monolith.

## Server internals

The server runs a **tokio** runtime with a single **state task** that owns the
model (`Server { sessions, windows, panes, clients, buffers, options, keys }`).
Nothing else touches the model; everything talks to it over an `mpsc` channel of
`Event`s. That makes every mutation sequential without locks.

```
           ┌────────────── Event channel ───────────────┐
 pane reader threads ─► PaneOutput(pane, bytes) ──────► │
 pane exit watcher    ─► PaneExited(pane, status) ────► │   state task
 client conn tasks    ─► ClientMsg(client, msg) ──────► │   (model + commands
 timers               ─► Tick / StatusInterval ───────► │    + render)
           └────────────────────────────────────────────┘
                                   │
                    per-client outbound queue (ServerMsg)
                                   ▼
                           client conn task ─► socket
```

- **PTY I/O.** `portable-pty` readers are blocking, so each pane gets a
  dedicated OS thread that reads into a buffer and sends `PaneOutput`. Writes to
  the PTY happen on the state task (they are small) through the `MasterPty`
  writer.
- **Parsing.** The state task feeds `PaneOutput` into the pane's `vt100::Parser`
  and marks the pane's windows dirty.
- **Rendering** is pull-based and rate-limited: after a batch of events the
  state task renders every client whose visible window is dirty, at most once
  per frame interval (default 8 ms, so bursts of output coalesce). See
  [07-rendering-status-theme.md](07-rendering-status-theme.md).
- **Back-pressure.** A client whose outbound queue is full gets its next frame
  skipped and a full redraw once it drains (frames are diffs against what the
  client last _received_, so dropping is safe only with a full redraw after).

## Client internals

The client is deliberately dumb and uses plain threads (no tokio):

- **Input thread**: `crossterm::event::read()` → `ClientMsg::Input(event)`. On
  Windows this reads console input records, on Unix it parses stdin — either way
  the server receives the same structured events.
- **Main thread**: reads `ServerMsg` frames; `Output(bytes)` → stdout,
  `Detached` / `Exited` → restore terminal, print the reason, exit.
- Resize events arrive through crossterm on every platform (`Event::Resize`).
- Terminal restore runs from a guard **and** a panic hook, so a crash never
  leaves the user's terminal in raw mode.

## Data flow of a keystroke

1. Client reads `KeyEvent{ code: Char('h'), modifiers: CONTROL }`, sends it.
2. Server looks up the client's current key table (`root`, or `prefix` after the
   prefix key, or `copy-mode-vi` while in copy mode).
3. `root C-h` is bound to the navigator command: the server inspects the active
   pane's foreground process; `hjkl` matches the navigator pattern, so the key
   is encoded for the pane (`0x08`, or `CSI 104;5u` if the pane enabled extended
   keys) and written to its PTY.
4. hjkl has no split to the left, runs `tmxr select-pane -L` (seeing `TMXR` in
   its environment) — a command client — and the server moves focus.
5. The focused pane's border changes colour; the next frame goes out.

## Dependencies (initial)

House-precedent versions; added with `cargo add` at scaffold time, not
hand-written.

| Crate                                          | Used by             | Notes                                               |
| ---------------------------------------------- | ------------------- | --------------------------------------------------- |
| `clap` 4 (derive)                              | app                 | CLI                                                 |
| `anyhow`                                       | app, server, client | binary-level errors                                 |
| `thiserror` 2                                  | library crates      | typed errors                                        |
| `tracing`, `tracing-subscriber`                | all                 | logs to `~/.local/state/tmxr/logs/` (never the tty) |
| `serde`, `postcard`                            | proto               | wire encoding                                       |
| `interprocess` 2 (+ `tokio` feature)           | proto/server/client | UDS / named pipes                                   |
| `tokio`                                        | server              | runtime                                             |
| `crossterm` 0.29 (+ `serde`)                   | client, server      | terminal I/O, event types on the wire               |
| `ratatui` 0.30                                 | server              | frame buffer + widgets                              |
| `portable-pty` 0.9                             | term                | PTYs                                                |
| `vt100` 0.16                                   | term                | terminal emulation per pane                         |
| `regex`                                        | server              | navigator pattern, copy-mode search                 |
| `hjkl-layout`                                  | server              | pane trees                                          |
| `hjkl-picker`, `hjkl-picker-tui`, `hjkl-fuzzy` | server              | session picker                                      |
| `hjkl-theme`, `hjkl-theme-tui`                 | server              | Tokyo Night palette → ratatui styles                |
| `hjkl-config`, `hjkl-xdg`                      | config              | TOML loading, paths                                 |
| `hjkl-fs`                                      | server              | atomic writes, locks, pid liveness (resurrect)      |
| `hjkl-clipboard`                               | server              | local clipboard for copy mode                       |
| `hjkl-keymap`                                  | command             | `<C-x>` notation parsing / printing                 |
| `libc` (unix) / `windows-sys` 0.61 (windows)   | term, client        | setsid, process inspection, toolhelp snapshots      |
| dev: `tempfile`, `portable-pty`, `vt100`       | tests               | temp sockets/dirs, e2e PTY harness (hjkl/hrdr)      |

Anything not on this list needs a call before it is added.
