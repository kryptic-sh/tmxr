//! The server: one thread owns all state and processes [`Event`]s in order.

mod attach;
mod input;
mod panes;
mod shell;
pub mod signals;
mod windows;

pub use shell::{pipe_to_shell, shell_succeeds, shell_text};

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use ratatui::Terminal;
use tmxr_command::Key;
use tmxr_config::Config;
use tmxr_proto::socket::Endpoint;
use tmxr_proto::{ClientMsg, Hello, PROTOCOL_VERSION, ServerMsg};
use tmxr_term::PtyEvent;
use tracing::{debug, info, warn};

use crate::backend::AnsiBackend;
use crate::cmds::{Ctx, Outcome};
use crate::keys::KeyTables;
use crate::model::{ClientId, Environment, Pane, PaneId, Session, SessionId, Window, WindowId};
use crate::overlay::Overlay;

/// Everything that can wake the server.
pub enum Event {
    Connected {
        id: ClientId,
        tx: SyncSender<ServerMsg>,
        /// The user the client's process runs as.
        user: crate::access::UserId,
        /// `server-access -r` made that user read-only.
        read_only: bool,
    },
    Msg(ClientId, ClientMsg),
    Disconnected(ClientId),
    /// End of output or exit of a pane's program, tagged with the pane's
    /// spawn.
    Pty(PaneId, u64, PtyEvent),
    /// Output is waiting in the pane's queue.
    PtyOutput(PaneId, u64),
    /// A `run-shell` job finished.
    JobDone(Box<crate::jobs::Done>),
    /// A line for the message log from work done off the state thread.
    Log(String),
    /// `pipe-pane -I`: the command printed this, to be typed into the pane.
    PipeInput {
        pane: PaneId,
        pipe: u64,
        bytes: Vec<u8>,
    },
}

pub struct Client {
    pub id: ClientId,
    pub tx: SyncSender<ServerMsg>,
    pub hello: Option<Hello>,
    pub att: Option<Attached>,
    /// The user its process runs as.
    pub user: crate::access::UserId,
    /// `server-access -r` made its user read-only.
    pub user_read_only: bool,
    /// It attached with `attach -r`.
    pub attach_read_only: bool,
}

impl Client {
    /// Its keys reach no pane and its commands change nothing.
    pub fn read_only(&self) -> bool {
        self.user_read_only || self.attach_read_only
    }
}

pub struct Attached {
    pub session: SessionId,
    pub cols: u16,
    pub rows: u16,
    pub term: Option<Terminal<AnsiBackend>>,
    /// Key table the next key is looked up in.
    pub table: String,
    pub repeat_until: Option<Instant>,
    pub overlay: Option<Overlay>,
    pub message: Option<(String, Instant)>,
    pub last_session: Option<SessionId>,
    pub dirty: bool,
    /// The last frame was dropped (slow client); repaint everything.
    pub full_redraw: bool,
    /// Running the lock command (`lock-client`) until it reports back.
    pub locked: bool,
    pub last_input: Instant,
    pub drag: Option<crate::mouse::Drag>,
    /// The mouse button being held, if any.
    pub press: Option<crate::mouse::Press>,
    /// The last button press, to count double and triple clicks.
    pub click: Option<crate::mouse::Click>,
    pub status_ranges: crate::render::StatusRanges,
    /// Whether this client was last told to capture the mouse.
    pub mouse: bool,
}

/// Paste buffer.
pub struct Buffer {
    pub name: String,
    pub data: String,
}

pub struct Server {
    pub endpoint: Endpoint,
    pub pid: u32,
    pub cfg: Config,
    pub cfg_path: Option<PathBuf>,
    pub events: Sender<Event>,
    pub sessions: BTreeMap<SessionId, Session>,
    pub windows: BTreeMap<WindowId, Window>,
    pub panes: BTreeMap<PaneId, Pane>,
    pub clients: BTreeMap<ClientId, Client>,
    pub keys: KeyTables,
    pub prefix: Key,
    pub navigator: Option<regex::Regex>,
    /// Newest first.
    pub buffers: VecDeque<Buffer>,
    pub messages: VecDeque<String>,
    pub host: String,
    next_session: SessionId,
    next_window: WindowId,
    /// The id the next pane (or popup) takes.
    pub next_pane: PaneId,
    next_spawn: u64,
    next_buffer: u32,
    /// The id the next `pipe-pane` pipe takes.
    pub next_pipe: u64,
    had_session: bool,
    pub exiting: bool,
    last_status: Instant,
    /// Per-pane foreground command cache refreshed on the status interval.
    pub commands: HashMap<PaneId, String>,
    /// Sessions were restored at start-up and no client has attached yet: a
    /// bare `tmxr` attaches to them instead of creating a new session.
    pub restored_pending: bool,
    /// The marked pane (`select-pane -m`): the default source of
    /// `join-pane`, `swap-pane` and `swap-window`. Read it through
    /// [`Server::marked_pane`], which forgets a pane that has closed.
    pub marked: Option<PaneId>,
    /// `set-environment -g` changes over the server's own environment.
    pub global_env: Environment,
    /// Entries submitted at prompts, oldest first, by prompt type.
    pub prompt_history: BTreeMap<String, Vec<String>>,
    /// `wait-for` channels and the command clients waiting on them.
    pub waits: crate::waits::Waits<crate::jobs::Done>,
    /// `copy-command-line` under way, waiting for its pane to settle.
    pub copy_line: Option<crate::copyline::CopyLine>,
    /// The user running the server, always admitted.
    pub owner: crate::access::UserId,
    /// Other users `server-access` admits; shared with the accept thread.
    pub acl: crate::access::SharedAcl,
    /// Who could open the endpoint when it was bound (`socket-access`).
    pub socket_access: tmxr_proto::socket::Access,
    /// Global hooks (`set-hook -g`).
    pub hooks: crate::hooks::HookTable,
    /// Events whose hooks run once the current event is handled.
    pub pending_hooks: Vec<(String, Ctx, crate::hooks::HookScope)>,
    /// A hook's commands are running: they fire no further hooks.
    pub in_hook: bool,
    last_save: Instant,
    /// What resurrect last wrote, so an unchanged layout is not saved again.
    pub last_saved: Option<crate::resurrect::Save>,
    /// The time clock-mode panes last showed, to redraw them each minute.
    clock_shown: Option<(u8, u8)>,
}

/// The navigator pattern, anchored to the whole process name. An invalid
/// pattern is reported and leaves the navigator off: `C-h/j/k/l` then always
/// move between panes.
fn navigator_regex(pattern: &str) -> Result<regex::Regex, String> {
    regex::Regex::new(&format!("^(?:{pattern})$")).map_err(|e| format!("navigator.pattern: {e}"))
}

/// Why `allow-passthrough` does nothing on this platform.
pub const PASSTHROUGH_UNSUPPORTED: &str = "allow-passthrough: needs the conpty.dll and \
     OpenConsole.exe tmxr's Windows release ships beside tmxr.exe (Windows' own ConPTY drops \
     the DCS terminator)";

/// How long one pass of the loop handles queued events before it renders,
/// so a pane flooding output still repaints.
const DRAIN_BUDGET: Duration = Duration::from_millis(20);
/// Rows the status line takes.
pub const STATUS_ROWS: u16 = 1;
/// Messages kept for `show-messages`.
const MESSAGE_LOG: usize = 100;
/// Paste buffers kept (tmux's `buffer-limit`).
const BUFFER_LIMIT: usize = 50;

impl Server {
    pub fn new(
        endpoint: Endpoint,
        cfg: Config,
        cfg_path: Option<PathBuf>,
        events: Sender<Event>,
    ) -> Self {
        let (keys, mut key_errors) = KeyTables::from_config(&cfg);
        let prefix = cfg.prefix.parse().unwrap_or_else(|_| {
            Key::new(
                crossterm::event::KeyCode::Char('b'),
                crossterm::event::KeyModifiers::CONTROL,
            )
        });
        if cfg.allow_passthrough && !tmxr_term::emulator::passthrough_supported() {
            key_errors.push(PASSTHROUGH_UNSUPPORTED.to_owned());
        }
        let navigator = match navigator_regex(&cfg.navigator.pattern) {
            Ok(re) => Some(re),
            Err(e) => {
                key_errors.push(e);
                None
            }
        };
        let mut srv = Self {
            endpoint,
            pid: std::process::id(),
            cfg,
            cfg_path,
            events,
            sessions: BTreeMap::new(),
            windows: BTreeMap::new(),
            panes: BTreeMap::new(),
            clients: BTreeMap::new(),
            keys,
            prefix,
            navigator,
            buffers: VecDeque::new(),
            messages: VecDeque::new(),
            host: crate::util::hostname(),
            next_session: 0,
            next_window: 0,
            next_pane: 0,
            next_spawn: 0,
            next_buffer: 0,
            next_pipe: 0,
            had_session: false,
            exiting: false,
            last_status: Instant::now(),
            commands: HashMap::new(),
            restored_pending: false,
            marked: None,
            global_env: Environment::default(),
            prompt_history: BTreeMap::new(),
            waits: crate::waits::Waits::default(),
            copy_line: None,
            owner: String::new(),
            acl: crate::access::SharedAcl::default(),
            socket_access: tmxr_proto::socket::Access::Owner,
            hooks: crate::hooks::HookTable::new(),
            pending_hooks: Vec::new(),
            in_hook: false,
            last_save: Instant::now(),
            last_saved: None,
            clock_shown: None,
        };
        for e in key_errors {
            srv.log_message(e);
        }
        srv
    }

    /// Process events until the server should exit.
    pub fn run(mut self, rx: Receiver<Event>) {
        let tick = Duration::from_millis(250);
        loop {
            // Wake for a pending DoubleClick or command-line copy on time,
            // not at the next tick.
            let due = crate::mouse::next_double_click(&self)
                .into_iter()
                .chain(self.copy_line.map(|c| c.due()))
                .min();
            let wait = due.map_or(tick, |due| {
                due.saturating_duration_since(Instant::now()).min(tick)
            });
            match rx.recv_timeout(wait) {
                Ok(ev) => {
                    self.handle(ev);
                    self.run_pending_hooks();
                    let until = Instant::now() + DRAIN_BUDGET;
                    while Instant::now() < until
                        && let Ok(ev) = rx.try_recv()
                    {
                        self.handle(ev);
                        self.run_pending_hooks();
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.tick();
            self.render_all();
            if signals::exit_requested() {
                info!("exit requested by the system");
                self.exiting = true;
            }
            if self.exiting {
                break;
            }
        }
        if let Err(e) = crate::resurrect::save_if_changed(&mut self) {
            warn!("resurrect save on exit failed: {e}");
        }
        signals::mark_saved();
        for c in self.clients.values() {
            let _ = c.tx.try_send(ServerMsg::Detached {
                reason: "server exited".into(),
            });
        }
        for p in self.panes.values_mut() {
            let _ = p.pty.kill();
        }
        info!("server exiting");
    }

    fn handle(&mut self, ev: Event) {
        match ev {
            Event::Connected {
                id,
                tx,
                user,
                read_only,
            } => {
                self.clients.insert(
                    id,
                    Client {
                        id,
                        tx,
                        hello: None,
                        att: None,
                        user,
                        user_read_only: read_only,
                        attach_read_only: false,
                    },
                );
            }
            Event::Disconnected(id) => {
                // Its replies wait for nothing any more; a bind's list
                // still goes on when signalled.
                self.waits
                    .forget(|d| d.then.reply.as_ref().is_some_and(|r| r.0 == id));
                if let Some(c) = self.clients.remove(&id)
                    && c.att.is_some()
                {
                    debug!(client = id, "attached client went away");
                }
            }
            Event::Msg(id, msg) => self.client_msg(id, msg),
            Event::Pty(pane, spawn, ev) => {
                // A respawned pane's previous program can still report.
                if self.panes.get(&pane).is_some_and(|p| p.spawn == spawn) {
                    self.pty_event(pane, ev);
                } else if let PtyEvent::Exited(code) = ev {
                    self.popup_exited(pane, spawn, code);
                }
            }
            Event::PtyOutput(pane, spawn) if !self.panes.contains_key(&pane) => {
                self.popup_output(pane, spawn);
            }
            Event::PtyOutput(pane, spawn) => {
                let bytes = self
                    .panes
                    .get(&pane)
                    .filter(|p| p.spawn == spawn)
                    .map(|p| p.output.0.take())
                    .unwrap_or_default();
                if !bytes.is_empty() {
                    self.pty_event(pane, PtyEvent::Output(bytes));
                }
            }
            Event::JobDone(done) => self.job_done(*done),
            Event::Log(line) => self.log_message(line),
            Event::PipeInput { pane, pipe, bytes } => {
                // Only from the pane's open pipe: one since closed is done.
                if let Some(p) = self
                    .panes
                    .get_mut(&pane)
                    .filter(|p| p.pipe.as_ref().is_some_and(|x| x.id == pipe))
                    && let Err(e) = p.pty.write(&bytes)
                {
                    tracing::debug!(pane, error = %e, "pipe-pane input not written");
                }
            }
        }
    }

    pub(crate) fn send(&mut self, client: ClientId, msg: ServerMsg) {
        let Some(c) = self.clients.get_mut(&client) else {
            return;
        };
        match c.tx.try_send(msg) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                if let Some(a) = c.att.as_mut() {
                    a.full_redraw = true;
                }
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    fn client_msg(&mut self, id: ClientId, msg: ClientMsg) {
        match msg {
            ClientMsg::Hello(hello) => {
                let ok = hello.protocol == PROTOCOL_VERSION;
                self.send(
                    id,
                    ServerMsg::Hello {
                        protocol: PROTOCOL_VERSION,
                        version: env!("CARGO_PKG_VERSION").into(),
                        pid: self.pid,
                    },
                );
                if !ok {
                    self.finish_command(
                        id,
                        Outcome::error(format!(
                            "protocol mismatch: server {} speaks {PROTOCOL_VERSION}, client {} speaks {} \
                             (run `tmxr kill-server` after upgrading)",
                            env!("CARGO_PKG_VERSION"),
                            hello.version,
                            hello.protocol
                        )),
                    );
                    return;
                }
                if let Some(c) = self.clients.get_mut(&id) {
                    c.hello = Some(hello);
                }
            }
            ClientMsg::Command(argv) => {
                if self
                    .clients
                    .get(&id)
                    .and_then(|c| c.hello.as_ref())
                    .is_none()
                {
                    self.finish_command(id, Outcome::error("no hello".into()));
                    return;
                }
                let argv = if argv.is_empty() && std::mem::take(&mut self.restored_pending) {
                    vec!["attach-session".to_owned()]
                } else if argv.is_empty() {
                    vec!["new-session".to_owned()]
                } else {
                    argv
                };
                let ctx = self.command_client_ctx(id);
                let out = crate::cmds::run_argv_list(self, &ctx, &argv);
                self.conclude_command(id, &ctx, out);
            }
            ClientMsg::Input(ev) => self.input(id, ev),
            ClientMsg::Detach => self.detach(id, "detached"),
            ClientMsg::Resumed | ClientMsg::LockFailed(_) => {
                // The shell had the terminal: repaint everything, and tell
                // the client its mouse mode again.
                if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                    a.full_redraw = true;
                    a.mouse = false;
                    a.dirty = true;
                    a.locked = false;
                    // Unlocking is activity: lock-after-time starts again.
                    a.last_input = Instant::now();
                }
                if let ClientMsg::LockFailed(why) = msg {
                    self.show_message(id, why);
                }
            }
        }
    }

    fn client_terminal(&self, id: ClientId) -> Option<&tmxr_proto::TerminalInfo> {
        self.clients
            .get(&id)
            .and_then(|c| c.hello.as_ref())
            .and_then(|h| h.terminal.as_ref())
    }

    /// Context for a command sent by a (not yet attached) client: its pane is
    /// the one named by `TMXR_PANE` if it runs inside this server.
    fn command_client_ctx(&self, id: ClientId) -> Ctx {
        let hello = self.clients.get(&id).and_then(|c| c.hello.as_ref());
        let env = |k: &str| {
            hello.and_then(|h| h.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()))
        };
        let pane = env(tmxr_proto::socket::ENV_TMXR_PANE)
            .and_then(|v| v.strip_prefix('%').and_then(|n| n.parse().ok()))
            .filter(|p| self.panes.contains_key(p))
            .filter(|_| {
                env(tmxr_proto::socket::ENV_TMXR)
                    .and_then(|v| tmxr_proto::socket::parse_tmxr_env(&v))
                    .is_some_and(|(_, pid, _)| pid == self.pid)
            });
        Ctx {
            client: Some(id),
            pane,
            key: None,
            cwd: hello.map(|h| PathBuf::from(&h.cwd)),
            env: hello.map(|h| h.env.clone()).unwrap_or_default(),
            mouse: None,
        }
    }

    /// Finish command client `id`'s command list: hold the reply for a job
    /// (`run-shell`, `wait-for`), attach, or reply.
    pub fn conclude_command(&mut self, id: ClientId, ctx: &Ctx, mut out: Outcome) {
        // The reply, and the rest of the list, wait for the job.
        if let Some(job) = out.job.take() {
            let then = crate::jobs::Then {
                ctx: ctx.clone(),
                reply: Some((id, out)),
            };
            self.start_job(job, then);
            return;
        }
        match out.attach {
            Some(session) if self.client_terminal(id).is_some() => {
                self.attach(id, session);
                if !out.stderr.is_empty() {
                    self.show_message(id, out.stderr);
                }
            }
            _ => self.finish_command(id, out),
        }
    }

    fn finish_command(&mut self, id: ClientId, out: Outcome) {
        self.send(
            id,
            ServerMsg::CommandResult {
                status: out.status,
                stdout: out.stdout,
                stderr: out.stderr,
            },
        );
        // Dropping the client drops its queue; the writer thread flushes the
        // result and closes the connection.
        if self.clients.get(&id).is_some_and(|c| c.att.is_none()) {
            self.clients.remove(&id);
        }
    }

    // ── Messages, clipboard, buffers ───────────────────────────────────────

    pub fn log_message(&mut self, msg: String) {
        if self.messages.len() == MESSAGE_LOG {
            self.messages.pop_front();
        }
        self.messages.push_back(msg);
    }

    pub fn show_message(&mut self, id: ClientId, msg: String) {
        self.log_message(msg.clone());
        let until = Instant::now() + Duration::from_millis(self.cfg.display_time);
        if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
            a.message = Some((msg, until));
            a.dirty = true;
        }
    }

    pub fn add_buffer(&mut self, data: String, name: Option<String>) {
        if let Some(n) = &name
            && let Some(b) = self.buffers.iter_mut().find(|b| &b.name == n)
        {
            b.data = data;
            return;
        }
        let name = name.unwrap_or_else(|| {
            let n = format!("buffer{}", self.next_buffer);
            self.next_buffer += 1;
            n
        });
        self.buffers.push_front(Buffer { name, data });
        self.buffers.truncate(BUFFER_LIMIT);
    }

    /// Put text on the clipboard: OSC 52 to every attached client (works
    /// locally and over SSH), plus the local clipboard via hjkl-clipboard.
    pub fn set_clipboard(&mut self, text: &str) {
        if self.cfg.set_clipboard == "off" {
            return;
        }
        let b64 = crate::util::base64(text.as_bytes());
        self.forward_osc52(&b64);
        crate::util::local_clipboard(text);
    }

    fn forward_osc52(&mut self, b64: &str) {
        if self.cfg.set_clipboard == "off" {
            return;
        }
        let seq = format!("\x1b]52;c;{b64}\x07");
        let ids: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| c.att.is_some())
            .map(|c| c.id)
            .collect();
        for id in ids {
            if let Some(t) = self
                .clients
                .get_mut(&id)
                .and_then(|c| c.att.as_mut())
                .and_then(|a| a.term.as_mut())
            {
                t.backend_mut().push_raw(&seq);
            }
            self.mark_client_dirty(id);
        }
    }

    /// The marked pane, if it still exists.
    pub fn marked_pane(&self) -> Option<PaneId> {
        self.marked.filter(|p| self.panes.contains_key(p))
    }

    /// Whether `window` holds the marked pane (its `M` flag).
    pub fn window_is_marked(&self, window: WindowId) -> bool {
        self.marked_pane()
            .is_some_and(|p| self.panes[&p].window == window)
    }

    /// `allow-passthrough`: write a program's passthrough payload as is to
    /// every client showing its pane's window.
    /// Send a pane's passthrough to the clients showing its window, from the
    /// cell where the pane's program printed it, as tmux does: an inline
    /// image lands where it belongs, not wherever the last frame left the
    /// terminal's cursor.
    fn forward_passthrough(&mut self, pane: PaneId, data: &[u8], (row, col): (u16, u16)) {
        let Some((wid, rect)) = self.panes.get(&pane).map(|p| (p.window, p.rect)) else {
            return;
        };
        let mut out = format!("\x1b[{};{}H", rect.y + row + 1, rect.x + col + 1).into_bytes();
        out.extend_from_slice(data);
        let viewers: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| {
                c.att.as_ref().is_some_and(|a| {
                    self.sessions
                        .get(&a.session)
                        .and_then(Session::current_window)
                        == Some(wid)
                })
            })
            .map(|c| c.id)
            .collect();
        for id in viewers {
            self.send(id, ServerMsg::Output(out.clone()));
        }
    }

    // ── Rendering ───────────────────────────────────────────────────────────

    pub fn mark_client_dirty(&mut self, id: ClientId) {
        if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
            a.dirty = true;
        }
    }

    pub fn mark_session_dirty(&mut self, session: SessionId) {
        for c in self.clients.values_mut() {
            if let Some(a) = c.att.as_mut()
                && a.session == session
            {
                a.dirty = true;
            }
        }
    }

    pub fn mark_window_dirty(&mut self, window: WindowId) {
        let sessions: Vec<SessionId> = self
            .sessions
            .values()
            .filter(|s| s.current_window() == Some(window))
            .map(|s| s.id)
            .collect();
        for s in sessions {
            self.mark_session_dirty(s);
        }
    }

    pub fn mark_all_dirty(&mut self) {
        for c in self.clients.values_mut() {
            if let Some(a) = c.att.as_mut() {
                a.dirty = true;
            }
        }
    }

    fn tick(&mut self) {
        crate::mouse::fire_double_clicks(self);
        self.advance_copy_line();
        self.lock_idle_clients();
        let now = Instant::now();
        for c in self.clients.values_mut() {
            if let Some(a) = c.att.as_mut() {
                if a.message.as_ref().is_some_and(|(_, t)| *t <= now) {
                    a.message = None;
                    a.dirty = true;
                }
                if a.table != "root" && a.repeat_until.is_some_and(|t| t <= now) {
                    a.table = "root".into();
                    a.repeat_until = None;
                    a.dirty = true;
                }
                if a.overlay.as_ref().is_some_and(Overlay::expired) {
                    a.overlay = None;
                    a.dirty = true;
                } else if a.overlay.as_mut().is_some_and(Overlay::tick) {
                    a.dirty = true;
                }
            }
        }
        let every = self.cfg.resurrect.auto_save_minutes;
        if every > 0
            && !self.sessions.is_empty()
            && now.duration_since(self.last_save) >= Duration::from_secs(every * 60)
        {
            self.last_save = now;
            if let Err(e) = crate::resurrect::save_if_changed(self) {
                self.log_message(format!("resurrect auto-save failed: {e}"));
            }
        }
        // monitor-silence: a window out of sight and quiet for long enough.
        let quiet: Vec<WindowId> = self
            .windows
            .values()
            .filter(|w| {
                let secs = w.monitor_silence.unwrap_or(self.cfg.monitor_silence);
                secs > 0
                    && !w.silence_raised
                    && now.duration_since(w.last_output) >= Duration::from_secs(secs)
            })
            .map(|w| w.id)
            .collect();
        for w in quiet {
            if let Some(win) = self.windows.get_mut(&w) {
                win.silence_raised = true;
            }
            crate::alerts::raise(self, w, crate::alerts::Alert::Silence);
        }
        let clock_windows: Vec<WindowId> = self
            .panes
            .values()
            .filter(|p| p.clock)
            .map(|p| p.window)
            .collect();
        if !clock_windows.is_empty() {
            let time = tmxr_term::localtime::hour_minute();
            if time != self.clock_shown {
                self.clock_shown = time;
                for w in clock_windows {
                    self.mark_window_dirty(w);
                }
            }
        }
        if now.duration_since(self.last_status)
            >= Duration::from_secs(self.cfg.status_interval.max(1))
        {
            self.last_status = now;
            crate::util::refresh_commands(self);
            self.mark_all_dirty();
        }
    }

    fn render_all(&mut self) {
        let ids: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| c.att.as_ref().is_some_and(|a| a.dirty))
            .map(|c| c.id)
            .collect();
        for id in ids {
            let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
                continue;
            };
            att.dirty = false;
            // The `mouse` option reaches the client's terminal with its next
            // frame, after attaching or after the option changes.
            let mouse = self.cfg.mouse;
            let tell_mouse = std::mem::replace(&mut att.mouse, mouse) != mouse;
            let full = std::mem::take(&mut att.full_redraw);
            let Some(mut term) = att.term.take() else {
                continue;
            };
            if full {
                let _ = term.clear();
            }
            let ranges = crate::render::draw(self, id, &mut term);
            let bytes = term.backend_mut().take();
            if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                a.term = Some(term);
                a.status_ranges = ranges;
            }
            if !bytes.is_empty() {
                self.send(id, ServerMsg::Output(bytes));
            }
            if tell_mouse {
                self.send(id, ServerMsg::Mouse(mouse));
            }
        }
    }

    pub fn reload_config(&mut self) -> Result<(), String> {
        let (cfg, _) = tmxr_config::load(self.cfg_path.as_deref()).map_err(|e| e.to_string())?;
        let (keys, mut errors) = KeyTables::from_config(&cfg);
        if let Ok(p) = cfg.prefix.parse() {
            self.prefix = p;
        }
        self.navigator = match navigator_regex(&cfg.navigator.pattern) {
            Ok(re) => Some(re),
            Err(e) => {
                errors.push(e);
                None
            }
        };
        self.keys = keys;
        self.cfg = cfg;
        self.apply_rgb_colour();
        self.mark_all_dirty();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

/// `split-window -l` size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitSize {
    Cells(u16),
    Percent(u16),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigator_pattern_matches_whole_names_and_reports_errors() {
        let pattern = &tmxr_config::defaults().navigator.pattern;
        let re = navigator_regex(pattern).unwrap();
        for editor in ["hjkl", "nvim", "vim", "/usr/bin/nvim", "fzf", "sqeel"] {
            assert!(re.is_match(editor), "{editor}");
        }
        for other in ["bash", "pwsh", "nvim-qt-launcher", "xhjkl"] {
            assert!(!re.is_match(other), "{other}");
        }
        let err = navigator_regex("(vim").unwrap_err();
        assert!(err.starts_with("navigator.pattern: "), "{err}");
    }
}
