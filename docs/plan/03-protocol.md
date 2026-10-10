# 03 — Client/server protocol

## Transport

`interprocess::local_socket`:

| Platform | Endpoint                                                                                         |
| -------- | ------------------------------------------------------------------------------------------------ |
| Linux    | `$TMXR_TMPDIR` or `$XDG_RUNTIME_DIR/tmxr` or `$TMPDIR/tmxr-<uid>` (dir `0700`), socket `<label>` |
| macOS    | `$TMXR_TMPDIR` or `$TMPDIR/tmxr-<uid>` (dir `0700`), socket `<label>`                            |
| Windows  | named pipe `\\.\pipe\tmxr-<username>-<label>`                                                    |

`-L <label>` picks the label (default `default`); `-S <path>` gives an explicit
socket path (on Windows, a pipe name). Precedence: `-S`, then `-L`, then the
`TMXR` environment variable's socket (so a command run inside a pane talks to
_its_ server), then the default.

### Access control

- Unix: a label's directory is created `0700` and verified (owner, mode, not a
  symlink) — the same treatment as `buffr`'s single-instance socket. A `-S`
  socket's directory is the user's choice, as in tmux.
- Windows: the pipe is created with a protected DACL granting its owner and
  SYSTEM.
- `socket-access`, read when the server starts, opens the endpoint further:
  `users` makes the socket file `0666` and adds read/write for authenticated
  users to the pipe's DACL. It is fixed at start because the pipe's DACL is:
  `interprocess` creates each new pipe instance from the listener's options,
  without `WRITE_DAC`, and holds its lock while waiting for a client, so neither
  the server nor the accept thread can change it afterwards.
- Every accepted connection is identified by the user its process runs as (the
  peer's uid from `SO_PEERCRED` / `getpeereid`; on Windows the SID in the token
  of the process `GetNamedPipeClientProcessId` names) and admitted only when it
  is the owner, root or SYSTEM, or a user `server-access` named. Others are
  dropped before the server sees them. A `server-access -r` user's clients, and
  an `attach -r` client, are read-only: their input reaches no pane and their
  commands are limited to `cmds::access::READ_ONLY`.

## Framing

Each message is `u32` little-endian length + `postcard`-encoded body. Frames
over `MAX_FRAME` (16 MiB) are a protocol error and close the connection — the
reader never allocates from an untrusted length beyond the cap.

## Handshake

A connection starts with a `Hello` each way, then the client sends one
`Command`:

```rust
ClientMsg::Hello(Hello {
    protocol: u32,                  // PROTOCOL_VERSION, bumped on any wire change
    version: String,                // CARGO_PKG_VERSION, for messages
    cwd: String,                    // default -c for new sessions
    env: Vec<(String, String)>,     // the client's whole environment
    terminal: Option<TerminalInfo>, // { cols, rows, term, job_control }; None = cannot attach
})
ServerMsg::Hello { protocol: u32, version: String, pid: u32 }
```

A protocol mismatch is reported to the user with both versions and a hint
(`tmxr kill-server` after upgrading), never silently tolerated.

## Messages

```rust
enum ClientMsg {
    Hello(Hello),
    Command(Vec<String>),           // argv after `tmxr`; empty = default command
    Input(crossterm::event::Event), // key, mouse, paste, focus, resize
    Detach,                         // terminal hangup
    Resumed,                        // back from suspend-client: redraw everything
}

enum ServerMsg {
    Hello { protocol: u32, version: String, pid: u32 },
    Attached,                       // the command attached; Output follows
    Output(Vec<u8>),                // bytes for the client's terminal (frame diff, OSC 52, bell)
    CommandResult { status: i32, stdout: String, stderr: String },
    Detached { reason: String },    // detach, session killed, server exit
    Mouse(bool),                    // capture the mouse or not (`mouse` option)
    Suspend,                        // stop the client (suspend-client)
}
```

Notes:

- An attaching command (`attach`, `new` without `-d`) turns the connection into
  an attached client, answered with `Attached`; any other command gets one
  `CommandResult` and the connection closes.
- `crossterm` events go over the wire as-is (crossterm's `serde` feature), so
  key decoding happens once, in the client, with crossterm's per-platform
  backends. The server never parses terminal input bytes.
- `Output` is opaque to the client. Terminal features are not negotiated: the
  server always emits 24-bit colour and OSC 52.
- `suspend-client` sends `Suspend`: the client restores its terminal, stops
  itself with `SIGTSTP`, and once the shell continues it re-enters raw mode and
  sends `Resumed`, on which the server repaints. Only a client whose hello says
  `job_control` (Unix) is asked; on Windows the command fails.
- The client captures the mouse only when told: the server sends `Mouse` with
  the client's first frame and again whenever the `mouse` option changes, so
  with `mouse off` the terminal's own selection works without Shift.
- `Hello.env` carries the client's whole environment. The server keeps the names
  listed in `update-environment` on the session it creates (so new panes get the
  attaching client's `DISPLAY`, `WAYLAND_DISPLAY`, `SSH_*`, …, like tmux), and
  reads `TMXR_PANE` from a command client to resolve its default target.

## Versioning rule

Any change to a wire type bumps `PROTOCOL_VERSION`. A test serialises a fixed
sample of every message and compares against checked-in bytes, so an accidental
wire change fails CI rather than at a user's attach.
