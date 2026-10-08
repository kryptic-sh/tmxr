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

`tmxr-server` is split by module, one per concern: `model`, `server` (the event
loop), `conn`, `cmds`, `keys`, `layout`, `render` + `backend`, `overlay`
(prompt, confirm, text view, picker), `copy`, `mouse`, `resurrect`, `target`,
`vars`.

## Server internals

The server uses plain OS threads and one `std::sync::mpsc` channel; there is no
async runtime. A single **state thread** owns the model
(`Server { sessions, windows, panes, clients, buffers, cfg, keys, … }`). Nothing
else touches the model; every other thread sends it an `Event`, so every
mutation is sequential without locks.

```
           ┌────────────── Event channel ────────────────┐
 pane reader/waiter ─► Pty(pane, Output | Eof | Exited) ─► │
 accept thread      ─► Connected(client, outbound tx) ───► │   state thread
 client reader      ─► Msg(client, ClientMsg) ───────────► │   (model + commands
 client reader      ─► Disconnected(client) ─────────────► │    + render)
 run-shell thread   ─► Shell(client, output) ────────────► │
           └─────────────────────────────────────────────┘
                                   │
                per-client bounded outbound queue (ServerMsg)
                                   ▼
                       client writer thread ─► socket
```

- **PTY I/O.** `portable-pty` readers are blocking, so each pane has a reader
  thread (sends `Output`, then `Eof`) and a waiter thread (sends `Exited`).
  Writes to the PTY happen on the state thread (they are small).
- **Parsing.** The state thread feeds `Output` into the pane's
  `tmxr_term::Emulator` (`vt100` plus reply/OSC hooks) and marks the pane's
  window dirty.
- **Timers.** The state thread waits on the channel with a 250 ms timeout and
  runs `tick` after every wake: message and key-repeat expiry, overlay timers,
  resurrect auto-save.
- **Rendering.** After draining every queued event the state thread renders each
  attached client whose view is dirty. There is no fixed frame interval: a burst
  of output that arrives together is coalesced by the drain, not by a timer. See
  [07-rendering-status-theme.md](07-rendering-status-theme.md).
- **Back-pressure.** Each client's outbound queue is bounded (`CLIENT_QUEUE` in
  `conn.rs`). When it is full the frame is dropped and the client's next frame
  is a full redraw (frames are diffs against what the client last _received_, so
  dropping is safe only with a full redraw after).

## Client internals

The client is deliberately dumb and uses plain threads:

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

## Dependencies

House-precedent versions, added with `cargo add`, not hand-written.

| Crate                                        | Used by             | Notes                                            |
| -------------------------------------------- | ------------------- | ------------------------------------------------ |
| `clap` 4 (derive)                            | app                 | CLI                                              |
| `thiserror` 2                                | library crates      | typed errors                                     |
| `tracing`, `tracing-subscriber`              | server, app         | logs to `<state dir>/tmxr/logs/` (never the tty) |
| `serde`, `postcard`                          | proto               | wire encoding                                    |
| `interprocess` 2                             | proto/server/client | UDS / named pipes                                |
| `crossterm` 0.29 (+ `serde`)                 | client, server      | terminal I/O, event types on the wire            |
| `ratatui` 0.30                               | server              | frame buffer + widgets                           |
| `portable-pty` 0.9                           | term                | PTYs                                             |
| `vt100` 0.16                                 | term                | terminal emulation per pane                      |
| `regex`                                      | server              | navigator pattern                                |
| `hjkl-layout`                                | server              | pane trees                                       |
| `hjkl-picker`, `hjkl-picker-tui`             | server              | session picker (fuzzy scoring via `hjkl-fuzzy`)  |
| `hjkl-config`, `hjkl-xdg`, `toml`            | config, server, app | TOML loading, paths                              |
| `serde_json`                                 | server              | resurrect save files                             |
| `hjkl-clipboard`                             | server              | local clipboard for copy mode                    |
| `libc` (unix) / `windows-sys` 0.61 (windows) | term, client        | setsid, process inspection, toolhelp snapshots   |
| dev: `tempfile`, `portable-pty`, `vt100`     | tests               | temp sockets/dirs, e2e PTY harness (hjkl/hrdr)   |

Anything not on this list needs a call before it is added.
