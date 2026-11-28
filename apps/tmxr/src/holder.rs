//! `tmxr __holder <ENDPOINT>`: the proof of concept of a pane holder
//! (`docs/plan/19-pane-holders.md`, phase 1).
//!
//! A holder runs one program on a pseudo-terminal and outlives the
//! connections to it: a server connects, has the program started, and may
//! go away and come back while the program keeps running and its output
//! keeps going into the holder's ring.
//!
//! - It listens on ENDPOINT, owner-only: a socket file `0600` on Unix, a
//!   pipe with the owner-only DACL on Windows. Who connects is not checked
//!   beyond that yet.
//! - What to run comes in a [`ToHolder::Spawn`] over the connection, never
//!   on the command line, which other users can read.
//! - It serves one connection at a time. A new connection takes the place of
//!   the current one, which is no longer read or written and ends when its
//!   peer closes it.
//! - It exits on [`ToHolder::Close`] once the program has ended: sent a
//!   hangup on Unix, then killed if it is still running after
//!   `KILL_GRACE`; terminated on Windows. If the program has not been
//!   reaped by [`CLOSE_WAIT`] the holder exits anyway, with a failure.
//! - It also exits once the program has exited by itself and a connection
//!   has been sent that, after all its output. On Windows it first waits,
//!   for at most `FLUSH_LIMIT`, for the connection to read it.
//! - A failure to start the program is reported to the connection
//!   ([`FromHolder::Error`]); a connection the holder cannot serve is
//!   dropped. Neither ends the holder.
//!
//! The protocol ([`proto`]) is throwaway: no version, no capabilities.
//! Output is written to the connection from the holder's loop, so a server
//! that stops reading holds the holder up; flow control is the real
//! protocol's.

mod proto;

use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use interprocess::local_socket::Stream;
use interprocess::local_socket::traits::ListenerExt as _;
use tmxr_proto::codec::{read_msg, write_msg};
use tmxr_proto::socket::{Access, Endpoint};
use tmxr_term::{Pty, PtyEvent, SpawnSpec};

use proto::{FromHolder, ToHolder};

/// Hidden subcommand a holder runs as: `tmxr __holder <ENDPOINT>`.
pub const HOLDER_ARG: &str = "__holder";

/// Output kept for a server that connects later: the newest this many bytes.
const RING_BYTES: usize = 1 << 20;

/// Largest piece of the ring sent in one [`FromHolder::Output`] on a
/// [`ToHolder::TakeFrom`].
const REPLAY_PIECE: usize = 64 * 1024;

/// How long after the program's exit the holder waits for its output
/// stream to end. The exit is reported once the stream has ended, so no
/// output comes after it, or once this has passed, so a stream that does
/// not end cannot hold the report back for good. The stream can outlive the
/// exit:
///
/// - Linux: the program's exit sends its foreground group a hangup but
///   leaves the terminal open, so a child that ignores SIGHUP keeps the
///   stream open and its writes still arrive (`holder_poc`'s
///   `a_child_ignoring_hangups_writes_before_the_exit_is_reported`).
/// - macOS: the terminal is probably revoked when its session leader exits,
///   ending the stream; unverified.
/// - ConPTY: the stream outlives the exit until the pseudo console is
///   closed, which the holder does on the exit.
const EXIT_DRAIN: Duration = Duration::from_secs(1);

/// How long a [`ToHolder::Close`] waits for the program to be reaped before
/// the holder exits anyway, with a failure.
const CLOSE_WAIT: Duration = Duration::from_secs(5);

/// How long a program sent a hangup by [`ToHolder::Close`] has to exit
/// before it is killed (Unix; Windows terminates it at once).
#[cfg(unix)]
const KILL_GRACE: Duration = Duration::from_secs(1);
#[cfg(unix)]
const _: () = assert!(KILL_GRACE.as_millis() < CLOSE_WAIT.as_millis());

/// How long the holder waits for a connection to read the last it was sent
/// before exiting (Windows: a named pipe's server end closing discards what
/// was not read).
#[cfg(windows)]
const FLUSH_LIMIT: Duration = Duration::from_secs(5);

/// How long the accept loop waits after a failed accept, so a failure that
/// lasts (no descriptors left) does not spin.
const ACCEPT_RETRY: Duration = Duration::from_millis(100);

/// The cursor-position query ConPTY sends at start, and the answer the
/// holder gives it: the top-left corner, where a new pane's cursor is.
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
const CURSOR_REPLY: &[u8] = b"\x1b[1;1R";

/// What the holder's loop waits for.
enum Event {
    Connected(Stream),
    Msg(u64, ToHolder),
    /// A connection's reader ended.
    Gone(u64),
    Pty(PtyEvent),
}

/// `tmxr __holder <ENDPOINT>`, given the arguments after `__holder`.
pub fn main(args: &[String]) -> ExitCode {
    let [endpoint] = args else {
        eprintln!("tmxr: usage: tmxr {HOLDER_ARG} <ENDPOINT>");
        return ExitCode::FAILURE;
    };
    #[cfg(unix)]
    tmxr_server::close_inherited_fds();
    let result = Endpoint::resolve(Some(Path::new(endpoint)), None).and_then(|e| run(&e));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tmxr holder: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Serve as a holder at `endpoint` until the program is closed or has
/// exited and been reported.
pub fn run(endpoint: &Endpoint) -> io::Result<()> {
    let listener = endpoint.listen(Access::Owner)?;
    let (tx, rx) = channel();
    let accepted = tx.clone();
    std::thread::Builder::new()
        .name("tmxr-holder-accept".into())
        .spawn(move || {
            for conn in listener.incoming() {
                match conn {
                    Ok(stream) => {
                        if accepted.send(Event::Connected(stream)).is_err() {
                            return;
                        }
                    }
                    // One failed accept does not stop the next.
                    Err(e) => {
                        eprintln!("tmxr holder: accept failed: {e}");
                        std::thread::sleep(ACCEPT_RETRY);
                    }
                }
            }
        })?;
    let result = Holder::new(tx, tmxr_term::pty::CONPTY).serve(&rx);
    let released = endpoint.release();
    result.and(released)
}

/// The current connection.
struct Conn {
    id: u64,
    stream: Arc<Stream>,
    /// The number from which output is sent; `None` until `TakeFrom`.
    from: Option<u64>,
}

struct Holder {
    events: Sender<Event>,
    ring: Ring,
    query: StartupQuery,
    conn: Option<Conn>,
    /// The newest connection's id. Only its messages are handled, and they
    /// still are after writing to it failed and it was dropped as
    /// [`Holder::conn`], so input it sent just before going counts. Once a
    /// newer connection has been accepted, whatever an older one sent and
    /// was not yet handled (an `Input`, a `Close`) is dropped as stale.
    newest: u64,
    pty: Option<Pty>,
    pid: Option<u32>,
    /// The program's exit, once the waiter has seen it.
    exit: Option<(Option<u32>, Option<i32>)>,
    /// The output stream has ended, or [`EXIT_DRAIN`] has passed since the
    /// exit.
    drained: bool,
    /// When [`EXIT_DRAIN`] ends.
    drain_until: Option<Instant>,
    /// When a `Close` gives up waiting for the program.
    close_until: Option<Instant>,
    /// When a `Close` kills a program that did not exit on the hangup.
    #[cfg(unix)]
    kill_at: Option<Instant>,
}

impl Holder {
    fn new(events: Sender<Event>, conpty: bool) -> Self {
        Self {
            events,
            ring: Ring::default(),
            query: StartupQuery::new(conpty && false),
            conn: None,
            newest: 0,
            pty: None,
            pid: None,
            exit: None,
            drained: false,
            drain_until: None,
            close_until: None,
            #[cfg(unix)]
            kill_at: None,
        }
    }

    /// Handle events until the holder is done.
    fn serve(&mut self, events: &Receiver<Event>) -> io::Result<()> {
        loop {
            #[cfg(unix)]
            let kill_at = self.kill_at;
            #[cfg(windows)]
            let kill_at = None;
            let deadline = [self.drain_until, self.close_until, kill_at]
                .into_iter()
                .flatten()
                .min();
            let event = match deadline {
                Some(at) => match events.recv_timeout(at.saturating_duration_since(Instant::now()))
                {
                    Ok(event) => Some(event),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return Ok(()),
                },
                None => match events.recv() {
                    Ok(event) => Some(event),
                    Err(_) => return Ok(()),
                },
            };
            let done = match event {
                Some(event) => self.handle(event)?,
                None => self.deadline_passed()?,
            };
            if done {
                return Ok(());
            }
        }
    }

    /// Handle one event; `true` when the holder is done.
    fn handle(&mut self, event: Event) -> io::Result<bool> {
        match event {
            Event::Connected(stream) => {
                self.connect(stream);
                Ok(false)
            }
            Event::Msg(id, msg) if id == self.newest => self.message(msg),
            // From a connection a newer one replaced.
            Event::Msg(..) => Ok(false),
            Event::Gone(id) => {
                if self.conn.as_ref().is_some_and(|c| c.id == id) {
                    self.conn = None;
                }
                Ok(false)
            }
            Event::Pty(PtyEvent::Output(bytes)) => {
                let (bytes, answer) = self.query.filter(&bytes);
                if answer {
                    self.write_program(CURSOR_REPLY);
                }
                self.output(&bytes);
                Ok(false)
            }
            Event::Pty(PtyEvent::Eof) => {
                let held = self.query.finish();
                self.output(&held);
                self.drained = true;
                self.drain_until = None;
                Ok(self.report_exit())
            }
            Event::Pty(PtyEvent::Exited { code, signal }) => {
                self.exit = Some((code, signal));
                if self.close_until.is_some() {
                    return Ok(true);
                }
                // Closing the terminal ends a ConPTY's output stream, which
                // it does not end for the program's exit; on Unix the stream
                // ends once every holder of the terminal has closed it.
                self.pty = None;
                if !self.drained {
                    self.drain_until = Some(Instant::now() + EXIT_DRAIN);
                }
                Ok(self.report_exit())
            }
        }
    }

    /// A deadline passed: the exit's drain, or a `Close`'s grace or wait;
    /// `true` when the holder is done.
    fn deadline_passed(&mut self) -> io::Result<bool> {
        let now = Instant::now();
        if self.close_until.is_some_and(|at| at <= now) {
            return Err(io::Error::other(format!(
                "the program ({}) had not ended {CLOSE_WAIT:?} after Close",
                self.pid
                    .map_or_else(|| "no pid".to_owned(), |p| p.to_string())
            )));
        }
        #[cfg(unix)]
        if self.kill_at.is_some_and(|at| at <= now) {
            self.kill_at = None;
            self.kill_hard();
        }
        if self.drain_until.is_some_and(|at| at <= now) {
            self.drain_until = None;
            self.drained = true;
            return Ok(self.report_exit());
        }
        Ok(false)
    }

    /// Kill a closed program that ignored its hangup. Only while its exit
    /// has not been seen: until then the waiter has not reported it reaped,
    /// so its pid is still its own (short of the moment between the reap
    /// and the report).
    #[cfg(unix)]
    fn kill_hard(&self) {
        let Some(pid) = self.pid.and_then(|p| libc::pid_t::try_from(p).ok()) else {
            return;
        };
        if self.exit.is_some() {
            return;
        }
        // SAFETY: kill takes no pointers.
        if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
            eprintln!(
                "tmxr holder: could not kill the program: {}",
                io::Error::last_os_error()
            );
        }
    }

    /// A new connection replaces the current one, and is greeted if the
    /// program runs. One the holder cannot start a reader for is dropped,
    /// and the current one goes on being served.
    fn connect(&mut self, stream: Stream) {
        let id = self.newest + 1;
        let stream = Arc::new(stream);
        let recv = Arc::clone(&stream);
        let events = self.events.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("tmxr-holder-read-{id}"))
            .spawn(move || {
                loop {
                    match read_msg::<_, ToHolder>(&mut &*recv) {
                        Ok(Some(msg)) => {
                            if events.send(Event::Msg(id, msg)).is_err() {
                                return;
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            eprintln!("tmxr holder: connection {id} read ended: {e}");
                            break;
                        }
                    }
                }
                // The holder's loop may be gone already; then nobody needs
                // to know.
                let _ = events.send(Event::Gone(id));
            });
        if let Err(e) = spawned {
            eprintln!("tmxr holder: could not serve connection {id}: {e}");
            return;
        }
        self.newest = id;
        self.conn = Some(Conn {
            id,
            stream,
            from: None,
        });
        if self.pid.is_some() || self.exit.is_some() {
            self.hello();
        }
    }

    /// Handle a message from the current connection; `true` when the
    /// holder is done.
    fn message(&mut self, msg: ToHolder) -> io::Result<bool> {
        match msg {
            ToHolder::Spawn {
                argv,
                cwd,
                env,
                rows,
                cols,
            } => {
                if self.pty.is_some() || self.exit.is_some() {
                    return Ok(false);
                }
                let spec = SpawnSpec {
                    argv,
                    cwd,
                    env,
                    rows,
                    cols,
                    ..SpawnSpec::default()
                };
                let events = self.events.clone();
                let sink: Arc<dyn Fn(PtyEvent) + Send + Sync> = Arc::new(move |e| {
                    // The holder's loop is gone only when it is exiting.
                    let _ = events.send(Event::Pty(e));
                });
                match Pty::spawn(&spec, sink) {
                    Ok(pty) => {
                        self.pid = pty.pid();
                        self.pty = Some(pty);
                        self.hello();
                    }
                    Err(e) => {
                        let error = format!("could not start {:?}: {e}", spec.argv);
                        self.send(&FromHolder::Error(error));
                    }
                }
            }
            ToHolder::Input(bytes) => self.write_program(&bytes),
            ToHolder::Resize { rows, cols } => {
                if let Some(pty) = &self.pty
                    && let Err(e) = pty.resize(rows, cols)
                {
                    eprintln!("tmxr holder: resize failed: {e}");
                }
            }
            ToHolder::TakeFrom(number) => {
                let (mut at, bytes) = self.ring.from(number);
                for piece in bytes.chunks(REPLAY_PIECE) {
                    self.send(&FromHolder::Output {
                        number: at,
                        bytes: piece.to_vec(),
                    });
                    at += piece.len() as u64;
                }
                if let Some(conn) = &mut self.conn {
                    conn.from = Some(number.max(self.ring.next()));
                }
                return Ok(self.report_exit());
            }
            ToHolder::Close => {
                let Some(pty) = &mut self.pty else {
                    return Ok(true);
                };
                if let Err(e) = pty.kill() {
                    eprintln!("tmxr holder: could not end the program: {e}");
                }
                let now = Instant::now();
                self.close_until = Some(now + CLOSE_WAIT);
                #[cfg(unix)]
                {
                    self.kill_at = Some(now + KILL_GRACE);
                }
            }
        }
        Ok(false)
    }

    /// Queue bytes for the program. Once it has exited they go nowhere, and
    /// its exit is reported by the waiter.
    fn write_program(&mut self, bytes: &[u8]) {
        if let Some(pty) = &mut self.pty
            && let Err(e) = pty.write(bytes)
        {
            eprintln!("tmxr holder: the program's input is closed: {e}");
        }
    }

    /// Keep output in the ring and send it on if the connection takes it.
    fn output(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let number = self.ring.push(bytes);
        let Some(from) = self.conn.as_ref().and_then(|c| c.from) else {
            return;
        };
        let end = number + bytes.len() as u64;
        if end <= from {
            return;
        }
        let skip = usize::try_from(from.saturating_sub(number)).unwrap_or(bytes.len());
        self.send(&FromHolder::Output {
            number: number.max(from),
            bytes: bytes[skip..].to_vec(),
        });
    }

    fn hello(&mut self) {
        let hello = FromHolder::Hello {
            pid: self.pid,
            ring_start: self.ring.start,
            next: self.ring.next(),
        };
        self.send(&hello);
    }

    /// Report the program's exit to a connection taking output, once its
    /// output has all been sent; `true` once reported (and, on Windows, read
    /// or `FLUSH_LIMIT` passed).
    fn report_exit(&mut self) -> bool {
        let Some((code, signal)) = self.exit else {
            return false;
        };
        if !self.drained || !self.conn.as_ref().is_some_and(|c| c.from.is_some()) {
            return false;
        }
        if !self.send(&FromHolder::Exited { code, signal }) {
            return false;
        }
        if let Some(conn) = &self.conn {
            flush(&conn.stream);
        }
        true
    }

    /// Send to the current connection; a connection that fails is dropped.
    fn send(&mut self, msg: &FromHolder) -> bool {
        let Some(conn) = &self.conn else {
            return false;
        };
        match write_msg(&mut &*conn.stream, msg) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("tmxr holder: connection {} write failed: {e}", conn.id);
                self.conn = None;
                false
            }
        }
    }
}

/// Nothing to wait for before exiting: a Unix socket keeps what was sent
/// for its peer after the sender has gone.
#[cfg(unix)]
fn flush(_: &Arc<Stream>) {}

/// Wait, for at most [`FLUSH_LIMIT`], until the peer has read everything
/// sent: a named pipe's server end closing discards what its client has not
/// read, and interprocess's own `flush` of a local socket does nothing.
/// `FlushFileBuffers` waits as long as the client does not read, so it runs
/// on a thread of its own, left behind if the limit passes. Not run on
/// Windows yet.
#[cfg(windows)]
fn flush(stream: &Arc<Stream>) {
    let (done, flushed) = channel();
    let stream = Arc::clone(stream);
    let spawned = std::thread::Builder::new()
        .name("tmxr-holder-flush".into())
        .spawn(move || {
            let Stream::NamedPipe(pipe) = &*stream;
            // Nobody receives this once the holder has stopped waiting.
            let _ = done.send(pipe.inner().flush());
        });
    if let Err(e) = spawned {
        eprintln!("tmxr holder: could not flush the pipe: {e}");
        return;
    }
    match flushed.recv_timeout(FLUSH_LIMIT) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => eprintln!("tmxr holder: pipe not flushed: {e}"),
        Err(_) => eprintln!("tmxr holder: the pipe was not read within {FLUSH_LIMIT:?}"),
    }
}

/// The program's output, numbered from 0, the newest [`RING_BYTES`] kept.
#[derive(Debug, Default)]
struct Ring {
    bytes: VecDeque<u8>,
    /// The number of the oldest byte kept.
    start: u64,
}

impl Ring {
    /// The number the next byte pushed gets.
    fn next(&self) -> u64 {
        self.start + self.bytes.len() as u64
    }

    /// Keep `data`, dropping the oldest bytes past [`RING_BYTES`]; the
    /// number of its first byte.
    fn push(&mut self, data: &[u8]) -> u64 {
        let number = self.next();
        self.bytes.extend(data);
        let excess = self.bytes.len().saturating_sub(RING_BYTES);
        self.bytes.drain(..excess);
        self.start += excess as u64;
        number
    }

    /// The bytes from `number` on, or from the oldest kept if that is newer,
    /// with the number of the first.
    fn from(&self, number: u64) -> (u64, Vec<u8>) {
        let first = number.clamp(self.start, self.next());
        let skip = usize::try_from(first - self.start).unwrap_or(self.bytes.len());
        (first, self.bytes.range(skip..).copied().collect())
    }
}

/// Finds ConPTY's first [`CURSOR_QUERY`] in the output and takes it out,
/// so the holder answers it and no server ever sees it (the plan's
/// "ConPTY startup query"). Until it is found, a tail that could be the
/// start of one is held back for the next read. It takes the first one
/// anywhere in the stream, relying on ConPTY sending its own before any of
/// the program's output, which it does when created inheriting the cursor
/// (`PSEUDOCONSOLE_INHERIT_CURSOR`, as portable-pty 0.9.0 creates it);
/// otherwise a program's own first query would be swallowed.
#[derive(Debug)]
struct StartupQuery {
    pending: bool,
    held: Vec<u8>,
}

impl StartupQuery {
    fn new(pending: bool) -> Self {
        Self {
            pending,
            held: Vec::new(),
        }
    }

    /// The output to keep, and whether the query was in it.
    fn filter(&mut self, bytes: &[u8]) -> (Vec<u8>, bool) {
        if !self.pending {
            return (bytes.to_vec(), false);
        }
        let mut data = std::mem::take(&mut self.held);
        data.extend_from_slice(bytes);
        if let Some(at) = data
            .windows(CURSOR_QUERY.len())
            .position(|w| w == CURSOR_QUERY)
        {
            self.pending = false;
            data.drain(at..at + CURSOR_QUERY.len());
            return (data, true);
        }
        let keep = (1..CURSOR_QUERY.len())
            .rev()
            .find(|&n| data.ends_with(&CURSOR_QUERY[..n]))
            .unwrap_or(0);
        self.held = data.split_off(data.len() - keep);
        (data, false)
    }

    /// The output stream ended: whatever was held back.
    fn finish(&mut self) -> Vec<u8> {
        self.pending = false;
        std::mem::take(&mut self.held)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_numbers_bytes_and_keeps_the_newest() {
        let mut ring = Ring::default();
        assert_eq!(ring.push(b"abc"), 0);
        assert_eq!(ring.push(b"de"), 3);
        assert_eq!(ring.from(0), (0, b"abcde".to_vec()));
        assert_eq!(ring.from(3), (3, b"de".to_vec()));
        assert_eq!(ring.from(9), (5, Vec::new()));

        let big = vec![b'x'; RING_BYTES];
        assert_eq!(ring.push(&big), 5);
        assert_eq!(ring.start, 5);
        assert_eq!(ring.next(), 5 + RING_BYTES as u64);
        // Older than the oldest kept: from the oldest kept.
        let (first, bytes) = ring.from(0);
        assert_eq!((first, bytes.len()), (5, RING_BYTES));
        assert!(bytes.iter().all(|&b| b == b'x'));
    }

    #[test]
    fn the_first_cursor_query_is_taken_out_even_split() {
        let mut q = StartupQuery::new(true);
        assert_eq!(q.filter(b"\x1b[?25l\x1b["), (b"\x1b[?25l".to_vec(), false));
        assert_eq!(q.filter(b"6nhi"), (b"hi".to_vec(), true));
        // Only the first.
        assert_eq!(q.filter(b"\x1b[6n"), (b"\x1b[6n".to_vec(), false));

        let mut q = StartupQuery::new(true);
        assert_eq!(q.filter(b"a\x1b[6nb"), (b"ab".to_vec(), true));

        let mut q = StartupQuery::new(true);
        assert_eq!(q.filter(b"x\x1b"), (b"x".to_vec(), false));
        assert_eq!(q.finish(), b"\x1b".to_vec());

        let mut q = StartupQuery::new(false);
        assert_eq!(q.filter(b"\x1b[6n"), (b"\x1b[6n".to_vec(), false));
    }
}
