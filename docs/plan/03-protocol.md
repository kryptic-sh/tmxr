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

- Unix: the directory is created `0700` and verified (owner, mode, not a
  symlink) — the same treatment as `buffr`'s single-instance socket. Every
  accepted connection is checked with `SO_PEERCRED` / `getpeereid`; a peer that
  is not our uid is dropped.
- Windows: the pipe is created with a security descriptor granting only the
  current user's SID (and SYSTEM). If `interprocess` cannot set one, the default
  DACL (creator + admins + SYSTEM) is the fallback and that gap is recorded in
  the backlog.

## Framing

Each message is `u32` little-endian length + `postcard`-encoded body. Frames
over `MAX_FRAME` (16 MiB) are a protocol error and close the connection — the
reader never allocates from an untrusted length beyond the cap.

## Handshake

The first frame each way is a `Hello`:

```rust
ClientMsg::Hello {
    protocol: u32,          // PROTOCOL_VERSION, bumped on any wire change
    version: String,        // CARGO_PKG_VERSION, for messages
    kind: ClientKind,       // Attach { size, term, features, env, cwd } | Command { cwd, env }
}
ServerMsg::Hello { protocol: u32, version: String, pid: u32 }
```

A protocol mismatch is reported to the user with both versions and a hint
(`tmxr kill-server` after upgrading), never silently tolerated.

## Messages

```rust
enum ClientMsg {
    Hello { .. },
    Input(crossterm::event::Event),   // key, mouse, paste, focus, resize
    Command(Vec<String>),             // argv after `tmxr`, e.g. ["split-window","-h"]
    Detach,
}

enum ServerMsg {
    Hello { .. },
    Output(Vec<u8>),                  // bytes for the client's terminal (frame diff, OSC 52, bell)
    CommandResult { status: i32, stdout: String, stderr: String },
    Detached { reason: DetachReason }, // detach, session killed, server exit, replaced
    Exit,                              // server shutting down
}
```

Notes:

- `crossterm` events go over the wire as-is (crossterm's `serde` feature), so
  key decoding happens once, in the client, with crossterm's per-platform
  backends. The server never parses terminal input bytes.
- `Output` is opaque to the client. Terminal-feature differences between clients
  (true colour, kitty keyboard, OSC 52 support) are reported in `Hello.features`
  and handled by the server when it renders.
- A `Command` connection may also be issued by an attached client (for the
  command prompt the server already has it; this is for scripts).
- `Hello.env` carries the client's `TERM`, `COLORTERM`, `SSH_TTY`,
  `WAYLAND_DISPLAY`, `DISPLAY`, `XDG_*` and anything in the `update-environment`
  option, so new panes get the attaching client's environment like tmux's
  `update-environment` (the tmux config adds the Wayland variables to it).

## Versioning rule

Any change to a wire type bumps `PROTOCOL_VERSION`. A test serialises a fixed
sample of every message and compares against checked-in bytes, so an accidental
wire change fails CI rather than at a user's attach.
