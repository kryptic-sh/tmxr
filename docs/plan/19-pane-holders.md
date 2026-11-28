# 19 — Pane holders: upgrading the server without losing the panes

Asked for by the owner (2026-11-27): when tmxr is upgraded and a newer client
meets an older server, the server should be replaced by the new version while
the programs running in its panes keep running. Of the four ways considered (the
server re-executing itself, handing its fds to a new server, pane holders, a
resurrect restart), the owner chose pane holders: each pane's terminal and
program live in a small process that outlives the server, which reconnects to it
after a restart. It is the one that works the same on Unix and Windows. Not
started; this plan is for working on it later. It was reviewed (2026-11-27) by a
reader who checked it against the code and the platforms' behaviour; what that
found is folded in below.

## What a pane is today

Read from the tree (2026-11-27); symbol names, as the code is the reference.

- **Terminal and program:** `tmxr-term` `pty.rs` `Pty::spawn` opens the terminal
  through `portable-pty` (a pty on Unix, ConPTY on Windows, the bundled
  `conpty.dll` when it sits beside the executable) and starts three threads per
  pane: a reader (`PtyEvent::Output`, then `PtyEvent::Eof`), a writer (an
  unbounded queue, so a program that stops reading never stalls the server), and
  a waiter (`waitpid` on Unix, `WaitForSingleObject` on Windows, giving
  `PtyEvent::Exited`). The program's environment is built by the server
  (`server/panes.rs` `start_pty`, `child_env`: `TERM`, `TMXR`, `TMXR_PANE`,
  `COLORTERM`, the session environment). portable-pty's child calls `setsid` and
  takes the pty as its controlling terminal, so the program is a session leader
  of its own and closing the master hangs it up.
- **Output:** the reader pushes into `output.rs` `OutputQueue`, which blocks it
  past `QUEUE_LIMIT`; the server loop takes the bytes and `pty_event` feeds the
  pane's `Emulator` (`tmxr-term` `emulator.rs`, on `vt1k`, which wraps `vte`),
  then handles query replies, titles, bells, OSC 52, passthrough, pipe-pane,
  renames and activity. `Emulator` also holds two splitters (`DcsSplitter`,
  `ModeSplitter`) that carry partial sequences between calls.
- **Input:** keys, pastes, mouse reports and focus reports are encoded from the
  emulator's modes (`input_modes`, `focus_reports`) and written with
  `Pty::write`.
- **Query replies:** the emulator answers DA1, DA2, XTVERSION, DECRQM, DSR,
  DECRQSS and the kitty query; the server answers OSC 10/11/12, the theme query
  and OSC 4 from its clients' terminals (`answer_queries`, `requests.rs`), and
  OSC 10/11 need an attached client. All are written from the server loop.
- **Process inspection:** `tmxr-term` `process.rs` finds the foreground program
  from `tcgetpgrp` on the master fd (Unix) or the newest child (ToolHelp,
  Windows), and reads names, arguments and directories by pid. It runs
  synchronously from format expansion (`vars.rs`), automatic rename
  (`util::refresh_commands`) and resurrect. `pane_tty` needs the master fd.
- **Lifetime:** the server kills every pane when it exits (`Server::run`);
  `kill-pane` and `respawn-pane` kill through `Pty::kill` (SIGHUP to the pid on
  Unix, `TerminateProcess` on Windows).
- **Identity:** a command run in a pane finds its pane through `TMXR_PANE` only
  when the pid in `TMXR` is the running server's (`Server::command_client_ctx`).
- **Persistence:** resurrect (`resurrect::Save`) keeps sessions, windows,
  layouts and each pane's command, directory and title, and restores them with
  new programs and new ids. Nothing serializes an emulator: `vt1k` (an external
  kryptic-sh crate) and `tmxr-term` have no serde, and `vte`'s parser state is
  private.
- **Starting detached processes:** `tmxr-client` `spawn.rs` `spawn_detached`
  starts the server and drops its `Child`; the started process is not waited
  for.

## The design

### A holder per pane

`tmxr __holder` is started for each pane (popups are short-lived and stay in the
server). It is the pane program's parent, so it alone reaps it and reads its
exit status, on both systems; the program stays a session leader of its own, as
now, so the holder is not its controlling process and closing the holder's
master (a `Close`, or the holder dying) hangs the program up as closing a pane
does today. The holder is detached as the server is (`setsid` on Unix,
`DETACHED_PROCESS` on Windows, nothing inherited); the server waits for each
holder it starts (a waiting thread per holder, or a double fork), so a holder
that exits leaves no zombie, and `spawn_detached` moves to a crate both sides
share.

It owns:

- the terminal: the pty master or the ConPTY with the program's process handle,
  opened with `portable-pty` as today;
- an output ring: every byte the program wrote, numbered from 0, the last
  `RING_BYTES` of it kept (a constant to size by measurement);
- the program's exit status and its last output, kept until a server
  acknowledges them;
- on Windows, the answer to ConPTY's startup cursor-position query (below).

What to run reaches it in a `Spawn` message (argv, directory, environment,
variables to remove, size, login shell, spawn number, pane id), never on its
command line, which other users can read; it starts the program only then.

One process per pane rather than one for the whole server (the owner's decision,
2026-11-28): a fault costs one pane, a holder stays small enough to rarely
change, and a server restart touches nothing shared. It costs a process per pane
(to measure).

### The emulator stays in the server

The emulator changes often (it is where tmux compatibility lives) and the server
needs it directly for drawing, copy mode, `capture-pane`, input encoding and
most query replies. A holder that parsed output would have to change with it,
and every upgrade would need new holders. So the holder carries bytes and the
server parses them, and three things carry a pane's state across a restart:

- **A snapshot, for an upgrade.** Before it exits for an upgrade, the old server
  writes each pane's emulator state (grid, history, history times, modes, title,
  palette, kitty flags, prompt marks, sync hold, and the two splitters'
  contents) in a versioned format, to a file per pane in its private state
  directory (not over the holder protocol: a large history exceeds a frame's
  `MAX_FRAME`, and a copy in each holder doubles memory). `vte`'s state cannot
  be saved, so the server keeps, for each pane, the output number of the last
  point where the parser was idle (no sequence or UTF-8 character open, both
  splitters empty) and the snapshot is of that point: the new server resumes the
  ring from there. This needs serde (or a hand-written format) in `vt1k`,
  released on its own, and in `tmxr-term`.
- **A modes record, always.** Whenever a pane's input-affecting modes change
  (bracketed paste, application cursor and keypad, mouse modes and encoding,
  focus reports, the kitty flag stacks, the alternate screen), the server tells
  the holder, which keeps the latest with its output number. It is small, so it
  costs little, and after a crash it is what keeps a multi-line paste from
  running as commands in a shell that turned bracketed paste on long ago.
- **A replay, after a crash.** With no snapshot, the new server feeds the ring
  into a fresh emulator, then applies the modes record. Anything older than the
  ring is lost, and a ring that starts inside a sequence shows garbage until the
  next one; Ctrl-L redraws most programs. That is enough (the owner's decision,
  2026-11-28): crashes are rare, and periodic snapshots would cost writes and
  CPU in proportion to history. They can be added if crashes prove to matter.

**Replayed output has no effects.** Feeding old bytes through `pty_event` as it
stands would answer every query in them again (typing DA and DSR replies into
the program as input) and redo their clipboard writes, passthrough, renames,
bells and pipe-pane writes. So the server takes output in two modes: replay
(update the screen only) for bytes the program wrote before the server
connected, and live for the rest. A query the program asked while no server was
connected gets no answer; most programs time out on their queries, and one that
does not would have hung in tmux's detached server too. That is accepted, not
solved.

### The ConPTY startup query

ConPTY asks `ESC[6n` once at start and draws nothing until it is answered (the
backlog records a guessed but unreproduced cause for a stray F3: a reply that
comes too late). The holder answers that first query itself, from its reader,
before any other output, and removes it from what it forwards; the server's
emulator never sees it, so it is never answered twice. Whether this cures the F3
is to be shown, not assumed.

### `pipe-pane` in the holder

`pipe-pane` sends the bytes the program wrote (not anything the emulator made,
`pty_event` passes it the raw output) and, with `-I`, writes the pipe's output
to the program as input. Both are bytes the holder already carries, so the
holder runs the pipe's child (the owner's decision, 2026-11-28: the holder
carries the I/O, the server keeps the parsing). The server expands the command's
formats and sends `Pipe` with the result; the holder starts the child, feeds it
from its reader and its output to the terminal, and reports `PipeEnded` when it
exits. The pipe then survives an upgrade and a crash with no gap, and `-o`
(toggle) stays the server's, from the holder's `Hello`.

### The holder protocol

Holders outlive upgrades, so a server meets holders older than itself, and,
since a server starts holders from the installed executable (below), holders
newer than itself too. Each side speaks the lower of the two versions.

- **Framing:** every frame starts with a fixed prefix (a magic number and the
  holder protocol version), then postcard, as `tmxr-proto` `codec.rs` frames.
  postcard cannot skip fields, so no message's layout ever changes: a change is
  a new message, offered as a capability. Golden-byte tests pin each version's
  frames, as `codec.rs` tests pin the client protocol's.
- **Server to holder:** `Hello` (version, capabilities, the server's token),
  `Spawn(...)` (also after the program has exited: `respawn-pane` starts the new
  program in the same holder, in a fresh terminal, so the pane keeps its holder
  and endpoint), `Input(bytes)`, `Resize(cols, rows)`, `Signal(hangup | kill)`,
  `TakeFrom(number)`, `Modes(number, record)`, `Ack(number)`,
  `Pipe(argv, directory, environment, out, in)` and `Unpipe` (below), `Going`
  (an upgrade: keep everything from the snapshot's number, stop reading the
  terminal rather than drop it), `Close` (the pane is gone: end the program and
  exit).
- **Holder to server:** `Hello` (version, capabilities, whether it runs the
  bundled ConPTY, the program's pid and tty name, its spawn number and pane id,
  its ring's oldest number, its modes record, its pipe if one runs),
  `Output(number, bytes)`, `Eof`, `Exited(code, signal)`, `PipeEnded`.
- **No round trips on the hot path:** format expansion runs synchronously, so
  process inspection must not wait on a holder. The tty name never changes and
  comes in `Hello`; the foreground process group comes from the program's pid
  (`/proc/<pid>/stat`'s tpgid on Linux, `e_tpgid` from `KERN_PROC_PID` on macOS;
  Windows already works by pid), so `process.rs` takes a pid, not a `Pty`.
- **Flow control:** replaces the in-process `OutputQueue` block. The holder
  stops reading the terminal while more than a window of unacknowledged output
  is outstanding, which holds the program back as the queue does today. With no
  server connected after a crash it keeps reading and drops the oldest bytes
  past `RING_BYTES`; after `Going` it stops reading instead, so nothing after
  the snapshot is lost. `TakeFrom` answers with the ring's oldest number, so a
  server can tell when bytes are missing and fall back to a replay.
- **Never blocking the server:** each holder connection has its own writer
  thread and queue on the server's side (as clients do), and the holder queues
  input for the program; a program that stops reading never stalls the server,
  and output and input cannot deadlock each other.

### Endpoints and their security

- Holder endpoints are always owner-only, whatever the server's own
  `socket-access` or `-S` path says: sockets in `<base>/tmxr-<uid>/` on Unix,
  pipes with the owner-only DACL on Windows.
- Each is named from a hash of the server's full endpoint path and the pane id,
  not the server's label (`-S` has none, and labels can collide).
- Windows pipe names share one namespace across users: a holder creates its pipe
  with `FILE_FLAG_FIRST_PIPE_INSTANCE`, names it by the user's SID (not
  `USERNAME`), and each side checks the other's user (the pipe's client or
  server process id and its token's SID); on Unix, the socket's peer
  credentials. A server checks every holder it finds by listing the same way.
- Holder sockets are refreshed against `/tmp` cleaners as the server's is
  (`Endpoint::refresh`), and stale ones are removed as `listen` removes a stale
  server socket.

### Restart and adoption

- **The state file:** a server's whole model, written to its private state
  directory: sessions, windows, links and groups, layouts, options at every
  scope, hooks, key bindings, the environment, buffers, user options, marks,
  pane and session ids, the `next_pane` and `next_spawn` counters, and which
  holder each pane is. Much more than `resurrect::Save`, which it is not: it is
  its own versioned format.
- **On start,** a server that finds a state file adopts it instead of running
  `restore_on_start` and the config's session setup (so nothing is created
  twice): it rebuilds the model with the same ids, connects to each holder,
  checks it, and takes up its ring. A holder that does not answer is a pane
  whose program is gone (shown dead). The state file and snapshots are renamed
  aside before loading, so one that crashes the new server is not loaded again
  at every start.
- **Identity:** the state file keeps a server-instance id, which goes in `TMXR`
  alongside the pid; `command_client_ctx` accepts either, so commands in adopted
  panes keep their pane.
- **A server that never comes back:** after `Going`, holders wait for a new
  server with no time limit (the upgrade failing must not end the programs). A
  holder that lost its server without notice does the same by default, so a
  crash never costs a program and the next server adopts it; a server option
  (the owner's decision, 2026-11-28; its name is to choose) sets a timeout after
  which such a holder ends its program instead, and the holder learns it in
  `Hello`. A new server takes a holder over from a hung old one by presenting
  the token the state file holds.

### Upgrading

- **Asking for it:** a client that meets a server speaking another protocol
  cannot send it a command (the server refuses a mismatched `Hello`; the
  `kill-server` fix works only because the server's `Hello` never changes and
  the client then signals the pid). So the request is a frozen frame read before
  `Hello` is decoded (a magic number, a version, an opcode), pinned by a golden
  test; or an OS mechanism (SIGUSR2 on Unix, a named event on Windows). Servers
  from before this feature have neither; for them the client falls back to
  ending the server and offers resurrect, as now.
- **Only upwards:** a client upgrades a server only when its protocol is newer,
  so an old client left running cannot downgrade the server, and two clients
  cannot bounce it between versions.
- **The handoff:** the old server writes its state and snapshots, sends its
  holders `Going`, sends each attached client `Detached` with
  `exec: ["tmxr", "attach", …]` (the field `detach-client -E` already uses), and
  exits; the requesting client starts the new server, which adopts, and the
  clients come back on the new binary.
- **`kill-server` with holders:** ending the server must not orphan programs.
  `kill-server` asks the server to close its holders, through the same frozen
  request when the versions differ; a server ended any other way leaves holders
  that wait as after a crash.
- **Which executable runs holders:** after a package upgrade, an old server's
  `current_exe()` can be gone (`/usr/bin/tmxr (deleted)` on Linux; Homebrew
  cleans old versions), so it starts new holders from the installed executable
  (`/proc/self/exe` on Linux where the path is gone), and those can be newer
  than it: the protocol's "lower version" rule covers it.
- **Windows' locked executables:** a running holder locks the executable it was
  started from and its directory's `conpty.dll` (portable-pty loads it by name)
  and `OpenConsole.exe` (each pane runs one). Scoop installs each version in its
  own directory behind a `current` junction, but `scoop update` refuses while
  any process from the app runs unless `ignore_running_processes` is set, so
  holders would block every Scoop update. So holders run from a per-version
  cache copy (the owner's decision, 2026-11-28): the server copies its
  executable, `conpty.dll` and `OpenConsole.exe` to a directory named by version
  under the user's local app data, and starts holders from there; a copy no
  running holder uses is removed. A separate holder package and documenting
  Scoop's `ignore_running_processes` were declined: one more package to ship,
  and a setting every user would have to know (and nothing for other installs).
- **The bundled ConPTY:** the holder reports whether it runs the bundled ConPTY
  in its `Hello`, since that, not the server's own directory
  (`passthrough_supported`), decides passthrough and DECRQSS for its pane.

## Phases

1. **A proof of concept on Windows and Unix.** A holder that runs a shell, a
   server that loses and regains it, by hand: to find the Windows surprises
   (ConPTY lifetime, locking, pipes) first, not last.
2. **A snapshot format for the emulator.** Versioned serialization of `vt1k`'s
   screen and `tmxr-term`'s `Emulator` state, the idle-point tracking, and
   round-trip tests over the emulator tests' screens; serde in `vt1k` is its own
   release. Useful on its own (resurrect could keep scrollback).
3. **The holder and its protocol, behind an option.** `__holder`, the protocol
   in `tmxr-proto` with its golden tests, the endpoints and their checks, a
   `Pty` stand-in on the server's side, process inspection by pid, the modes
   record, replay mode, and a server option (`pane-holders`, off by default).
   Every existing test runs both ways in CI.
4. **Adoption.** The state file, ids kept, finding and checking holders, replay,
   the timeouts, the instance id in `TMXR`.
5. **Upgrade.** The frozen request, upwards only, the handoff, `kill-server`,
   the executable to start holders from, the fallback for older servers.
6. **Windows hardening.** The startup DSR, the per-version cache copy, the
   bundled ConPTY report; tested on a Windows machine.
7. **Measure, then make it the default.** Output throughput, key latency and
   memory per pane with and without holders, in a quiet window; turn the option
   on once the numbers and a release's use allow it.

## Tests and fuzzing

Every phase lands with its tests and its fuzz targets in the same change, not
afterwards: a holder outlives the server that started it, so a bug in what it
accepts is one an upgrade cannot fix. The fuzzing follows
[20-fuzzing.md](20-fuzzing.md) (cargo-fuzz, a short run on every push, a long
one nightly, every crash kept as a regression input).

**Tests**, beyond running the whole suite with holders on and off:

- each message and the frame prefix pinned by golden bytes, per protocol
  version;
- a new server against holders of each released holder protocol version, and a
  new holder against each released server;
- `kill -9` of the server while panes run, then adoption, with output written in
  the gap;
- ring overflow with and without `Going`, and `TakeFrom` reporting the gap;
- a replay that must not answer old queries or repeat clipboard writes, renames,
  bells or pipe writes (each checked by what reaches the program and the
  clients);
- snapshot then resume against feeding the same output to one emulator straight
  through: the screens, modes and history must match;
- `respawn-pane` in the same holder, `pipe-pane` across an upgrade with no byte
  lost or doubled, the orphan timeout option on and off;
- a state file that crashes the server is moved aside and not loaded twice;
- endpoints: another user cannot connect (Unix peer credentials, Windows SID
  check) and a squatted pipe name is refused, on Windows a pipe created first by
  another user;
- the Windows per-version cache copy: holders run from it, a Scoop update
  succeeds with panes open, unused copies are removed.

**Fuzz targets**, each with its invariant:

- `holder_frame`: any bytes read as a holder message or an error, in both
  directions and at every protocol version; never a panic, never an allocation
  past the frame limit.
- `holder_session`: a sequence of well-formed messages in any order (a `Spawn`
  twice, `Input` before `Spawn`, `TakeFrom` past the ring) against a holder with
  a stand-in terminal: no panic, and the ring's numbering stays contiguous.
- `snapshot_load`: any bytes as a snapshot file are loaded or refused; one that
  loads gives a screen within its stated size.
- `snapshot_roundtrip`: output fed to an emulator, snapshot taken at the idle
  point, loaded, the rest fed: equal to feeding all of it straight through (the
  strongest check of the idle-point rule).
- `state_load`: any bytes as a state file are adopted or refused, never a panic;
  one that loads keeps every id it names.
- `modes_record`: any record bytes are refused or give modes the input encoder
  accepts.
- `replay_has_no_effects`: any output replayed gives no reply bytes, clipboard
  writes, renames or pipe writes.

## Decided

The owner settled the plan's open questions on 2026-11-28: a holder per pane,
orphaned holders keep their programs unless an option says otherwise, a
per-version cache copy on Windows, ring replay after a crash, `respawn-pane` in
the same holder, and `pipe-pane` in the holder. Each is written into the section
it belongs to above.

## Not covered yet

- Copy mode, popups, menus and other server-only state are rebuilt after an
  upgrade as after an attach, not carried.
- Performance of the extra hop is unknown until it is measured.
