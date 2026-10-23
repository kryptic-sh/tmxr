//! The server: one thread owns all state and processes [`Event`]s in order.

mod attach;
mod history;
mod input;
mod options;
mod panes;
mod shell;
pub mod signals;
mod windows;

pub use shell::{ChildEnv, pipe_to_shell, shell_succeeds, shell_text};

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use ratatui::Terminal;
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
    /// A line from a `#(…)` format job, or its end.
    FormatJob(Box<crate::format_jobs::JobOutput>),
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
    /// It attached with `-f ignore-size`.
    pub ignore_size: bool,
    /// `-f no-detach-on-destroy`: when its session is destroyed and
    /// `detach-on-destroy` would detach it, it moves to another session.
    pub no_detach_on_destroy: bool,
    /// The standard input it sent for `load-buffer -`, until that takes it.
    pub stdin: Option<Vec<u8>>,
    /// When it connected (`#{client_created}`).
    pub created: std::time::SystemTime,
    /// The default foreground and background colours its terminal said
    /// it has, as RGB, for programs' colour queries.
    pub terminal_colours: (Option<u32>, Option<u32>),
    /// Its terminal's color scheme, as last reported (`#{client_theme}`).
    pub theme: Option<crossterm::event::ColorScheme>,
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
    /// The cursor shape (DECSCUSR's `Ps`) last sent to the terminal.
    pub cursor_style: u8,
    /// The cursor colour last sent to the terminal; `None` for its own.
    pub cursor_colour: Option<u32>,
    /// When `table` was entered, for `prefix-timeout`.
    pub table_at: Instant,
    pub repeat_until: Option<Instant>,
    pub overlay: Option<Overlay>,
    /// A status message, and when it goes; `None` lasts until a key.
    pub message: Option<(String, Option<Instant>)>,
    /// Keys leave the message up (`display-message -N`), for one that
    /// goes by itself.
    pub message_keep: bool,
    /// The message's `#[…]` are styles (`display-message`'s), not text.
    pub message_styled: bool,
    pub last_session: Option<SessionId>,
    pub dirty: bool,
    /// Only pane output needs drawing: the status line is drawn as it was
    /// (`status_cache`), as tmux redraws its status line on its timer and on
    /// other events, not on output.
    pub content_dirty: bool,
    /// The status line as last drawn, for a frame only output asked for.
    pub status_cache: Option<crate::render::StatusCache>,
    /// The last frame was dropped (slow client); repaint everything.
    pub full_redraw: bool,
    /// Running the lock command (`lock-client`) until it reports back.
    pub locked: bool,
    pub last_input: Instant,
    /// A lone Escape held for `escape-time`: a key before this goes as
    /// Meta and that key, as tmux's tty_keys_next.
    pub pending_escape: Option<Instant>,
    pub drag: Option<crate::mouse::Drag>,
    /// The mouse button being held, if any.
    pub press: Option<crate::mouse::Press>,
    /// The last button press, to count double and triple clicks.
    pub click: Option<crate::mouse::Click>,
    pub status_ranges: crate::render::StatusRanges,
    /// Whether this client was last told to capture the mouse.
    pub mouse: bool,
    /// Whether this client was last told to ask for focus reports.
    pub focus_events: bool,
    /// When this client's status timer last fired (`status-interval`).
    pub status_at: Instant,
    /// Focused: from attaching until the terminal reports focus out, then
    /// as it reports (`focused` in `#{client_flags}`).
    pub focused: bool,
    /// No input for `client-away-time`: windows stop taking its size.
    pub away: bool,
    /// The title last given to the terminal (`set-titles`).
    pub title: Option<String>,
    /// `refresh-client -L` / `-R` / `-U` / `-D`: the view of a window larger
    /// than the client held at this offset, until `-c` or another window.
    pub pan: Option<(WindowId, u16, u16)>,
}

/// Paste buffer.
pub struct Buffer {
    pub name: String,
    pub data: String,
    /// Named by tmxr (`bufferN`), not by `-b`: only these count against
    /// `buffer-limit` and make way for new ones, as tmux's.
    pub automatic: bool,
    /// When it was made (`#{buffer_created}`).
    pub created: std::time::SystemTime,
}

/// How many bytes of a buffer tmux's paste_make_sample shows.
const SAMPLE_BYTES: usize = 200;

impl Buffer {
    fn new(name: String, data: String, automatic: bool) -> Self {
        Self {
            name,
            data,
            automatic,
            created: std::time::SystemTime::now(),
        }
    }

    /// tmux's buffer_sample (paste_make_sample): the first bytes, escaped
    /// as vis(3) with VIS_CSTYLE, VIS_OCTAL, VIS_TAB and VIS_NL does, cut
    /// with `...` when there is more.
    pub fn sample(&self) -> String {
        let mut end = self.data.len().min(SAMPLE_BYTES);
        while !self.data.is_char_boundary(end) {
            end -= 1;
        }
        let mut out = String::new();
        for c in self.data[..end].chars() {
            match c {
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                '\\' => out.push_str("\\\\"),
                c if c.is_control() && u32::from(c) < 0x80 => {
                    let _ = write!(out, "\\{:03o}", u32::from(c));
                }
                c => out.push(c),
            }
        }
        if self.data.len() > SAMPLE_BYTES || out.len() > SAMPLE_BYTES {
            let mut cut = out.len().min(SAMPLE_BYTES);
            while !out.is_char_boundary(cut) {
                cut -= 1;
            }
            out.truncate(cut);
            out.push_str("...");
        }
        out
    }

    /// The `buffer_*` format variables for this buffer.
    pub fn var(&self, name: &str) -> Option<String> {
        match name {
            "buffer_name" => Some(self.name.clone()),
            "buffer_size" => Some(self.data.len().to_string()),
            "buffer_sample" => Some(self.sample()),
            "buffer_full" => Some(self.data.clone()),
            "buffer_created" => Some(
                self.created
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs())
                    .to_string(),
            ),
            _ => None,
        }
    }
}

pub struct Server {
    pub endpoint: Endpoint,
    pub pid: u32,
    /// When the server started (`#{start_time}`).
    pub started: std::time::SystemTime,
    pub cfg: Config,
    pub cfg_path: Option<PathBuf>,
    /// The file the config was last read from, none when it is the defaults
    /// alone (`#{config_files}`).
    pub cfg_file: Option<PathBuf>,
    pub events: Sender<Event>,
    pub sessions: BTreeMap<SessionId, Session>,
    pub windows: BTreeMap<WindowId, Window>,
    pub panes: BTreeMap<PaneId, Pane>,
    pub clients: BTreeMap<ClientId, Client>,
    pub keys: KeyTables,
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
    /// When the server started, for [`STARTUP_GRACE`].
    up_since: Instant,
    pub exiting: bool,
    /// When the socket was last shown as in use ([`SOCKET_REFRESH`]).
    socket_refreshed: Instant,
    /// When foreground commands were last refreshed.
    last_status: Instant,
    /// Per-pane foreground command cache refreshed on the status interval.
    pub commands: HashMap<PaneId, String>,
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
    /// Session groups (`new-session -t`), in the order they were made.
    pub groups: Vec<crate::groups::SessionGroup>,
    /// `#(…)` format jobs, kept behind a lock as formats read them through
    /// a shared reference.
    pub format_jobs: std::sync::Mutex<crate::format_jobs::FormatJobs>,
    /// `copy-command-line` under way, waiting for its pane to settle.
    pub copy_line: Option<crate::copyline::CopyLine>,
    /// Copies to the local desktop's clipboard.
    pub clipboard: crate::clipboard::Local,
    /// The user running the server, always admitted.
    pub owner: crate::access::UserId,
    /// The threads writing to clients, waited for as the server exits.
    pub writers: crate::conn::Writers,
    /// Other users `server-access` admits; shared with the accept thread.
    pub acl: crate::access::SharedAcl,
    /// Who could open the endpoint when it was bound (`socket-access`).
    pub socket_access: tmxr_proto::socket::Access,
    /// Global hooks (`set-hook -g`).
    pub hooks: crate::hooks::HookTable,
    /// Events whose hooks run once the current event is handled.
    pub pending_hooks: Vec<(String, Ctx, crate::hooks::HookScope)>,
    /// The state the notification hooks last saw (`notify`).
    pub watched: crate::notify::Watched,
    /// Why the config file was not used when the server started, shown to
    /// the first client that attaches.
    pub config_error: Option<String>,
    /// A hook's commands are running: they fire no further hooks.
    pub in_hook: bool,
    /// The command running now, for `#{command}`.
    pub running: Option<&'static str>,
    last_save: Instant,
    /// What resurrect last wrote, so an unchanged layout is not saved again.
    pub last_saved: Option<crate::resurrect::Save>,
    /// The time clock-mode panes last showed, to redraw them when it
    /// changes (each second, for a clock that shows seconds).
    clock_shown: Option<(u8, u8, u8)>,
}

/// The navigator pattern, anchored to the whole process name. An invalid
/// pattern is reported and leaves the navigator off: `C-h/j/k/l` then always
/// move between panes.
fn navigator_regex(pattern: &str) -> Result<regex::Regex, String> {
    regex::Regex::new(&format!("^(?:{pattern})$")).map_err(|e| format!("navigator.pattern: {e}"))
}

/// The size tmux gives a client whose terminal it cannot ask (tmux's
/// `tty_resize`, and a command client's size for `new-session`).
pub const DEFAULT_CLIENT_SIZE: (u16, u16) = (80, 24);

/// A client terminal's size as tmux's `tty_resize` takes it: a side the
/// terminal reports as 0 is the side of [`DEFAULT_CLIENT_SIZE`].
pub(crate) fn terminal_size(cols: u16, rows: u16) -> (u16, u16) {
    let (default_cols, default_rows) = DEFAULT_CLIENT_SIZE;
    (
        if cols == 0 { default_cols } else { cols },
        if rows == 0 { default_rows } else { rows },
    )
}

/// Why `allow-passthrough` does nothing on this platform.
pub const PASSTHROUGH_UNSUPPORTED: &str = "allow-passthrough: needs the conpty.dll and \
     OpenConsole.exe tmxr's Windows release ships beside tmxr.exe (Windows' own ConPTY drops \
     the DCS terminator)";

/// How long one pass of the loop handles queued events before it renders,
/// so a pane flooding output still repaints.
const DRAIN_BUDGET: Duration = Duration::from_millis(20);

/// How often the socket is shown as in use (`Endpoint::refresh`): well
/// within the 10 days after which Arch's systemd-tmpfiles removes an unused
/// file from `/tmp`.
const SOCKET_REFRESH: Duration = Duration::from_secs(60 * 60);

/// How long a server that has had no session waits for a client before it
/// goes (with `exit-empty`): longer than a client waits for the server it
/// starts, so one whose start outlasted its client's wait does not run on
/// with nothing in it.
const STARTUP_GRACE: Duration = Duration::from_secs(15);
const _: () = assert!(STARTUP_GRACE.as_secs() > tmxr_proto::SPAWN_TIMEOUT.as_secs());

/// When foreground commands need a wakeup of their own to refresh, `every`
/// after `last` did: none when a status line is redrawn (at one of
/// `status_due`) within half an interval of it, as `tick` refreshes them
/// along with a redraw from half their interval on, so an idle server wakes
/// once for both.
fn refresh_deadline(last: Instant, every: Duration, status_due: &[Instant]) -> Option<Instant> {
    let refresh = last + every;
    let near = |&s: &Instant| s + every / 2 >= refresh && s <= refresh + every / 2;
    (!status_due.iter().any(near)).then_some(refresh)
}
impl Server {
    pub fn new(
        endpoint: Endpoint,
        cfg: Config,
        cfg_path: Option<PathBuf>,
        events: Sender<Event>,
    ) -> Self {
        let (keys, mut key_errors) = KeyTables::from_config(&cfg);
        if cfg.window.allow_passthrough && !tmxr_term::emulator::passthrough_supported() {
            key_errors.push(PASSTHROUGH_UNSUPPORTED.to_owned());
        }
        let navigator = match navigator_regex(&cfg.navigator.pattern) {
            Ok(re) => Some(re),
            Err(e) => {
                key_errors.push(e);
                None
            }
        };
        let (hooks, hook_errors) = crate::hooks::from_config(&cfg);
        key_errors.extend(hook_errors);
        let mut srv = Self {
            endpoint,
            pid: std::process::id(),
            started: std::time::SystemTime::now(),
            cfg,
            cfg_path,
            cfg_file: None,
            events,
            sessions: BTreeMap::new(),
            windows: BTreeMap::new(),
            panes: BTreeMap::new(),
            clients: BTreeMap::new(),
            keys,
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
            up_since: Instant::now(),
            exiting: false,
            socket_refreshed: Instant::now(),
            last_status: Instant::now(),
            commands: HashMap::new(),
            marked: None,
            global_env: Environment::default(),
            prompt_history: BTreeMap::new(),
            waits: crate::waits::Waits::default(),
            groups: Vec::new(),
            format_jobs: std::sync::Mutex::default(),
            copy_line: None,
            clipboard: crate::clipboard::Local::default(),
            owner: String::new(),
            writers: crate::conn::Writers::default(),
            acl: crate::access::SharedAcl::default(),
            socket_access: tmxr_proto::socket::Access::Owner,
            hooks,
            pending_hooks: Vec::new(),
            watched: crate::notify::Watched::default(),
            config_error: None,
            in_hook: false,
            running: None,
            last_save: Instant::now(),
            last_saved: None,
            clock_shown: None,
        };
        for e in key_errors {
            srv.log_message(e);
        }
        srv
    }

    /// After an event or a tick: queue the hooks for what changed, run every
    /// queued hook, then take what those hooks changed in without firing.
    fn settle(&mut self) {
        self.sync_groups();
        self.update_pane_focus();
        self.queue_changes();
        self.run_pending_hooks();
        self.watched = crate::notify::Watched::of(self);
    }

    /// When the soonest of [`Server::tick`]'s timed duties falls due, so the
    /// server sleeps until then rather than waking on a timer, as tmux's does
    /// (on a laptop, battery spent for nothing).
    fn next_due(&self, now: Instant) -> Instant {
        let secs = Duration::from_secs;
        let mut due: Vec<Instant> = crate::mouse::next_double_click(self)
            .into_iter()
            .chain(self.copy_line.map(|c| c.due()))
            .chain(self.next_dead_line())
            .chain(self.next_escape())
            .collect();
        if !self.exiting {
            due.push(self.socket_refreshed + SOCKET_REFRESH);
            if !self.had_session && self.cfg.exit_empty {
                due.push(self.up_since + STARTUP_GRACE);
            }
        }
        let away = self.cfg.client_away_time;
        let mut status_due = Vec::new();
        for c in self.clients.values() {
            let Some(a) = c.att.as_ref() else {
                continue;
            };
            due.extend(a.message.as_ref().and_then(|(_, t)| *t));
            due.extend(a.repeat_until);
            due.extend(a.overlay.as_ref().and_then(|o| o.next_tick(now)));
            if away != 0 && !a.away {
                due.push(a.last_input + secs(away));
            }
            let opts = self.session_options(a.session);
            if opts.status_interval != 0 {
                status_due.push(a.status_at + secs(opts.status_interval));
            }
            let lock = self.client_session_options(c.id);
            if lock.lock_after_time != 0 && !lock.lock_command.is_empty() && !a.locked {
                due.push(a.last_input + secs(lock.lock_after_time));
            }
        }
        let every = self.cfg.resurrect.auto_save_minutes;
        if every > 0 && !self.sessions.is_empty() {
            due.push(self.last_save + secs(every * 60));
        }
        for w in self.windows.values() {
            let silence = self.window_options(w.id).monitor_silence;
            if silence > 0 && !w.silence_raised {
                due.push(w.last_output + secs(silence));
            }
        }
        if self.panes.values().any(|p| p.clock) {
            // The clock shows seconds: the next one, on the wall clock.
            let into_second = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(Duration::ZERO, |d| {
                    Duration::from_nanos(d.subsec_nanos().into())
                });
            due.push(now + (secs(1) - into_second));
        }
        let interval = self
            .sessions
            .keys()
            .map(|&s| self.session_options(s).status_interval)
            .min()
            .unwrap_or(self.cfg.session.status_interval);
        due.extend(refresh_deadline(
            self.last_status,
            secs(interval.max(1)),
            &status_due,
        ));
        due.extend(status_due);
        due.into_iter().min().unwrap_or(now + SOCKET_REFRESH)
    }

    /// Process events until the server should exit.
    pub fn run(mut self, rx: Receiver<Event>) {
        loop {
            let now = Instant::now();
            let wait = self.next_due(now).saturating_duration_since(now);
            match rx.recv_timeout(wait) {
                Ok(ev) => {
                    self.handle(ev);
                    self.settle();
                    let until = Instant::now() + DRAIN_BUDGET;
                    while Instant::now() < until
                        && let Ok(ev) = rx.try_recv()
                    {
                        self.handle(ev);
                        self.settle();
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.tick();
            self.settle();
            self.check_exit();
            self.render_all();
            if signals::exit_requested() {
                info!("exit requested by the system");
                self.begin_exit();
            }
            if signals::rebind_requested() && !self.exiting {
                self.rebind();
            }
            if self.exiting {
                break;
            }
        }
        if let Err(e) = crate::resurrect::save_if_changed(&mut self) {
            warn!("resurrect save on exit failed: {e}");
        }
        self.save_prompt_history();
        signals::mark_saved();
        for c in self.clients.values() {
            let _ = c.tx.try_send(ServerMsg::Detached {
                reason: "server exited".into(),
                hangup: false,
                exec: None,
            });
        }
        for p in self.panes.values_mut() {
            let _ = p.pty.kill();
        }
        self.stop_format_jobs();
        info!("server exiting");
    }

    /// Exit once this pass of the loop is done. The endpoint goes now, before
    /// the reply to whatever asked is sent, so the asker's next command
    /// cannot reach this server on its way out.
    /// Accept clients from `listener` on a thread of its own.
    pub fn spawn_accept(
        &self,
        listener: interprocess::local_socket::Listener,
    ) -> std::io::Result<()> {
        let events = self.events.clone();
        let acl = std::sync::Arc::clone(&self.acl);
        let owner = self.owner.clone();
        let writers = std::sync::Arc::clone(&self.writers);
        std::thread::Builder::new()
            .name("tmxr-accept".into())
            .spawn(move || crate::conn::accept_loop(listener, events, &acl, &owner, &writers))
            .map(drop)
    }

    /// Bind the socket again (SIGUSR1), as tmux's server_signal does, for a
    /// server whose socket file was removed: whatever is at its path goes,
    /// and a new listener there gets its own accept thread. The previous
    /// thread keeps waiting on the old socket, which no client can reach
    /// any more: one idle thread for each time this is asked.
    fn rebind(&mut self) {
        let bound = self
            .endpoint
            .release()
            .and_then(|()| self.endpoint.listen(self.socket_access))
            .and_then(|listener| self.spawn_accept(listener));
        match bound {
            Ok(()) => info!("socket bound again"),
            Err(e) => warn!("could not bind the socket again: {e}"),
        }
    }

    pub fn begin_exit(&mut self) {
        if !self.exiting {
            self.exiting = true;
            if let Err(e) = self.endpoint.release() {
                warn!("could not remove the socket: {e}");
            }
        }
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
                        ignore_size: false,
                        no_detach_on_destroy: false,
                        stdin: None,
                        created: std::time::SystemTime::now(),
                        terminal_colours: (None, None),
                        theme: None,
                    },
                );
            }
            Event::Disconnected(id) => {
                // Its replies wait for nothing any more; a bind's list
                // still goes on when signalled.
                self.waits
                    .forget(|d| d.then.reply.as_ref().is_some_and(|r| r.0 == id));
                self.forget_format_jobs(id);
                if let Some(c) = self.clients.remove(&id)
                    && c.att.is_some()
                {
                    debug!(client = id, "attached client went away");
                    self.latest_gone(id);
                    self.check_unattached();
                }
            }
            Event::Msg(id, msg) => self.client_msg(id, msg),
            Event::Pty(pane, spawn, ev) => {
                // A respawned pane's previous program can still report.
                if self.panes.get(&pane).is_some_and(|p| p.spawn == spawn) {
                    self.pty_event(pane, ev);
                } else if let PtyEvent::Exited { code, .. } = ev {
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
            Event::FormatJob(output) => self.format_job_output(*output),
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
            // A client too slow for its frames: this one is dropped, and it
            // is repainted whole on a later pass (the loop makes one each
            // tick), until one fits.
            Err(TrySendError::Full(_)) => {
                if let Some(a) = c.att.as_mut() {
                    a.full_redraw = true;
                    a.dirty = true;
                }
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    fn client_msg(&mut self, id: ClientId, msg: ClientMsg) {
        match msg {
            ClientMsg::Hello(mut hello) => {
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
                if let Some(t) = hello.terminal.as_mut() {
                    (t.cols, t.rows) = terminal_size(t.cols, t.rows);
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
                let argv = if argv.is_empty() {
                    let hello = self.clients.get(&id).and_then(|c| c.hello.as_ref());
                    bare_command(hello.map_or("", |h| h.cwd.as_str()))
                } else {
                    argv
                };
                let ctx = self.command_client_ctx(id);
                let out = crate::cmds::run_argv_list(self, &ctx, &argv);
                self.conclude_command(id, &ctx, out);
            }
            // The client's input comes in pieces, joined here.
            ClientMsg::Stdin(data) => {
                if let Some(c) = self.clients.get_mut(&id) {
                    c.stdin.get_or_insert_with(Vec::new).extend(data);
                }
            }
            ClientMsg::Input(events) => self.inputs(id, events),
            ClientMsg::TerminalColours { fg, bg } => {
                if let Some(c) = self.clients.get_mut(&id) {
                    c.terminal_colours = (fg, bg);
                }
            }
            ClientMsg::Detach => self.detach(id, "detached"),
            ClientMsg::Resumed | ClientMsg::LockFailed(_) => {
                // The shell had the terminal: repaint everything, and tell
                // the client its mouse mode again.
                if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                    a.full_redraw = true;
                    a.mouse = false;
                    a.focus_events = false;
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

    /// Keep `msg` for `show-messages`, the oldest going past
    /// `message-limit`.
    pub fn log_message(&mut self, msg: String) {
        self.messages.push_back(msg);
        while self.messages.len() > self.cfg.message_limit {
            self.messages.pop_front();
        }
    }

    pub fn show_message(&mut self, id: ClientId, msg: String) {
        self.show_message_for(id, msg, None, false, false);
    }

    /// Show `msg` on client `id` for `delay` milliseconds, else for the
    /// session's `display-time`; 0 keeps it until a key, as tmux's. With
    /// `keep`, a key does not take a timed message down
    /// (`display-message -N`). A `styled` message's `#[…]` are styles,
    /// as tmux's for `display-message`; others' are text.
    pub fn show_message_for(
        &mut self,
        id: ClientId,
        msg: String,
        delay: Option<u64>,
        keep: bool,
        styled: bool,
    ) {
        self.log_message(msg.clone());
        let time = delay.unwrap_or_else(|| self.client_session_options(id).display_time);
        let until = (time > 0).then(|| Instant::now() + Duration::from_millis(time));
        if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
            a.message_keep = keep && until.is_some();
            a.message_styled = styled;
            a.message = Some((msg, until));
            a.dirty = true;
        }
    }

    /// Add a paste buffer, or with `name` set the one of that name, which
    /// then counts as named, as tmux's `paste_set`: one of that name already
    /// there is replaced by a new one, newest of all. The oldest automatic
    /// buffer makes way once there are more than `buffer-limit`.
    pub fn add_buffer(&mut self, data: String, name: Option<String>) {
        let Some(name) = name else {
            self.add_buffer_prefixed(data, "buffer");
            return;
        };
        self.buffers.retain(|b| b.name != name);
        self.buffers.push_front(Buffer::new(name, data, false));
        self.trim_buffers();
    }

    /// Add an automatic buffer named `prefix` and the next buffer number,
    /// skipping names already taken, as tmux's paste_add.
    pub fn add_buffer_prefixed(&mut self, data: String, prefix: &str) {
        let name = loop {
            let n = format!("{prefix}{}", self.next_buffer);
            self.next_buffer += 1;
            if !self.buffers.iter().any(|b| b.name == n) {
                break n;
            }
        };
        self.buffers.push_front(Buffer::new(name, data, true));
        self.trim_buffers();
    }

    /// Drop the oldest automatic buffers beyond `buffer-limit`.
    fn trim_buffers(&mut self) {
        while self.buffers.iter().filter(|b| b.automatic).count() > self.cfg.buffer_limit {
            if let Some(oldest) = self.buffers.iter().rposition(|b| b.automatic) {
                self.buffers.remove(oldest);
            }
        }
    }

    /// A copy from copy mode to the clipboard: [`Server::set_clipboard`],
    /// and `pane-set-clipboard` for `pane` unless set-clipboard is off, as
    /// tmux's window_copy_copy_buffer.
    pub fn copy_to_clipboard(&mut self, pane: PaneId, text: &str) {
        if self.cfg.set_clipboard == "off" {
            return;
        }
        self.set_clipboard(text);
        self.pane_set_clipboard(pane);
    }

    /// A program's OSC 52 copy: taken only with set-clipboard on (external
    /// leaves it to tmxr's own copies), when its data decodes, then passed
    /// on, made a buffer and `pane-set-clipboard` fired, as tmux's
    /// input_osc_52.
    pub fn program_clipboard(&mut self, pane: PaneId, b64: &str) {
        if self.cfg.set_clipboard != "on" {
            return;
        }
        let Some(data) = crate::util::base64_decode(b64).filter(|d| !d.is_empty()) else {
            return;
        };
        self.forward_osc52(b64);
        self.pane_set_clipboard(pane);
        self.add_buffer(String::from_utf8_lossy(&data).into_owned(), None);
    }

    fn pane_set_clipboard(&mut self, pane: PaneId) {
        let ctx = crate::cmds::Ctx {
            pane: Some(pane),
            ..crate::cmds::Ctx::default()
        };
        self.queue_hook(
            "pane-set-clipboard",
            ctx,
            crate::hooks::HookScope::default(),
        );
    }

    /// Put text on the clipboard: OSC 52 to every attached client (works
    /// locally and over SSH), plus the local desktop's of the client last
    /// typed in.
    pub fn set_clipboard(&mut self, text: &str) {
        if self.cfg.set_clipboard == "off" {
            return;
        }
        let b64 = crate::util::base64(text.as_bytes());
        self.forward_osc52(&b64);
        self.local_clipboard(None, text);
    }

    /// The local desktop's clipboard of `client`, else of the attached
    /// client last typed in, else of the server's own environment.
    fn local_clipboard(&mut self, client: Option<ClientId>, text: &str) {
        let client = client.or_else(|| {
            self.clients
                .values()
                .filter_map(|c| c.att.as_ref().map(|a| (c.id, a.last_input)))
                .max_by_key(|(_, t)| *t)
                .map(|(id, _)| id)
        });
        let env = client
            .and_then(|id| self.clients.get(&id))
            .and_then(|c| c.hello.as_ref())
            .map(|h| h.env.clone());
        self.clipboard.copy(text, env.as_deref());
    }

    /// The clipboard of one client (`set-buffer -w`, `load-buffer -w`), as
    /// tmux's tty_set_selection on its target client: OSC 52 to `client`
    /// alone, and the local desktop's as every tmxr copy.
    pub fn set_client_clipboard(&mut self, client: Option<ClientId>, text: &str) {
        if self.cfg.set_clipboard == "off" {
            return;
        }
        self.local_clipboard(client, text);
        let Some(id) = client else {
            return;
        };
        let seq = format!("\x1b]52;c;{}\x07", crate::util::base64(text.as_bytes()));
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

    /// Redraw every client of a session holding `window`: its panes where it
    /// is the current window, its name and flags in every window list.
    pub fn mark_window_dirty(&mut self, window: WindowId) {
        let sessions: Vec<SessionId> = self
            .sessions
            .values()
            .filter(|s| s.windows.values().any(|&w| w == window))
            .map(|s| s.id)
            .collect();
        for s in sessions {
            self.mark_session_dirty(s);
        }
    }

    /// Redraw the panes of `window` for its output, on the clients showing
    /// it, leaving their status lines as drawn.
    pub fn mark_window_output(&mut self, window: WindowId) {
        let sessions: Vec<SessionId> = self
            .sessions
            .values()
            .filter(|s| s.current_window() == Some(window))
            .map(|s| s.id)
            .collect();
        for c in self.clients.values_mut() {
            if let Some(a) = c.att.as_mut()
                && sessions.contains(&a.session)
            {
                a.content_dirty = true;
            }
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
        self.flush_escapes();
        self.advance_copy_line();
        self.write_due_dead_lines();
        self.mark_away_clients();
        self.lock_idle_clients();
        self.tidy_format_jobs();
        let now = Instant::now();
        if !self.exiting && now.duration_since(self.socket_refreshed) >= SOCKET_REFRESH {
            self.socket_refreshed = now;
            if let Err(e) = self.endpoint.refresh() {
                warn!("could not refresh the socket: {e}");
            }
        }
        // The table a client goes back to once a repeat ends.
        let roots: BTreeMap<SessionId, String> = self
            .sessions
            .keys()
            .map(|&s| (s, self.default_key_table(s)))
            .collect();
        for c in self.clients.values_mut() {
            if let Some(a) = c.att.as_mut() {
                if a.message
                    .as_ref()
                    .is_some_and(|(_, t)| t.is_some_and(|t| t <= now))
                {
                    a.message = None;
                    a.dirty = true;
                }
                let root = roots.get(&a.session).map_or("root", String::as_str);
                if a.table != root && a.repeat_until.is_some_and(|t| t <= now) {
                    root.clone_into(&mut a.table);
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
                let secs = self.window_options(w.id).monitor_silence;
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
            let time = tmxr_command::localtime::hour_minute_second();
            if time != self.clock_shown {
                self.clock_shown = time;
                for w in clock_windows {
                    self.mark_window_dirty(w);
                }
            }
        }
        // Each client's status line redraws on its own timer, at its
        // session's `status-interval`, none at 0, as tmux's
        // `status_timer_callback`.
        let due: Vec<ClientId> = self
            .clients
            .values()
            .filter_map(|c| {
                let a = c.att.as_ref()?;
                let interval = self.session_options(a.session).status_interval;
                (interval != 0 && now.duration_since(a.status_at) >= Duration::from_secs(interval))
                    .then_some(c.id)
            })
            .collect();
        let status_redrawn = !due.is_empty();
        for id in due {
            if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                a.status_at = now;
                a.dirty = true;
            }
        }
        // Foreground commands and automatic window names refresh at the
        // shortest `status-interval` any session has, at least each second.
        // A status line redrawn from half that on refreshes them early, so the
        // two share a wakeup instead of each waking an idle server.
        let interval = self
            .sessions
            .keys()
            .map(|&s| self.session_options(s).status_interval)
            .min()
            .unwrap_or(self.cfg.session.status_interval);
        let every = Duration::from_secs(interval.max(1));
        let since = now.duration_since(self.last_status);
        if since >= every || (status_redrawn && since >= every / 2) {
            self.last_status = now;
            crate::util::refresh_commands(self);
        }
    }

    fn render_all(&mut self) {
        let ids: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| c.att.as_ref().is_some_and(|a| a.dirty || a.content_dirty))
            .map(|c| c.id)
            .collect();
        for id in ids {
            // The `mouse` option reaches the client's terminal with its next
            // frame, after attaching or after the option changes.
            let mouse = self.client_session_options(id).mouse;
            let focus = self.cfg.focus_events;
            let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
                continue;
            };
            // Output alone keeps the status line as it was drawn.
            let status = if att.dirty {
                None
            } else {
                att.status_cache.take()
            };
            att.dirty = false;
            att.content_dirty = false;
            let tell_mouse = std::mem::replace(&mut att.mouse, mouse) != mouse;
            let tell_focus = std::mem::replace(&mut att.focus_events, focus) != focus;
            let full = std::mem::take(&mut att.full_redraw);
            let Some(mut term) = att.term.take() else {
                continue;
            };
            if full {
                let _ = term.clear();
            }
            let (ranges, status) = crate::render::draw(self, id, &mut term, status);
            let mut bytes = term.backend_mut().take();
            let title = crate::render::title(self, id);
            let cursor_style = crate::render::cursor_style(self, id);
            let cursor_colour = crate::render::cursor_colour(self, id);
            if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                a.term = Some(term);
                a.status_ranges = ranges;
                a.status_cache = status;
                // So does the cursor shape.
                if a.cursor_style != cursor_style {
                    a.cursor_style = cursor_style;
                    bytes.extend_from_slice(format!("\x1b[{cursor_style} q").as_bytes());
                }
                // And its colour, as tmux's Cs and Cr.
                if a.cursor_colour != cursor_colour {
                    a.cursor_colour = cursor_colour;
                    let set = match cursor_colour {
                        Some(rgb) => {
                            let [_, r, g, b] = rgb.to_be_bytes();
                            format!("\x1b]12;rgb:{r:02x}/{g:02x}/{b:02x}\x07")
                        }
                        None => "\x1b]112\x07".to_owned(),
                    };
                    bytes.extend_from_slice(set.as_bytes());
                }
                // The title goes out only when it changes, as tmux's.
                if title.is_some() && a.title != title {
                    a.title = title;
                    let t = a.title.as_deref().unwrap_or_default();
                    bytes.extend_from_slice(format!("\x1b]0;{t}\x07").as_bytes());
                }
            }
            if !bytes.is_empty() {
                self.send(id, ServerMsg::Output(bytes));
            }
            if tell_mouse {
                self.send(id, ServerMsg::Mouse(mouse));
            }
            if tell_focus {
                self.send(id, ServerMsg::FocusEvents(focus));
            }
        }
    }

    /// Apply what the config file at `path` sets over what is running, as
    /// tmux's `source-file` runs a file's commands: each option, user
    /// option, bind and hook it names takes its value there, and everything
    /// else keeps its own, changes made since the server started included.
    pub fn overlay_config(&mut self, path: &std::path::Path) -> Result<(), String> {
        let (file, _) = tmxr_config::load(Some(path)).map_err(|e| e.to_string())?;
        let sets = tmxr_config::file_sets(path)?;
        let mut errors = Vec::new();
        for name in &sets.options {
            self.cfg.copy_option(&file, name);
        }
        for name in &sets.user_options {
            match file.options.get(name) {
                Some(value) => self.cfg.options.insert(name.clone(), value.clone()),
                None => self.cfg.options.remove(name),
            };
        }
        for (table, key) in &sets.binds {
            let bind_key = match key.parse::<tmxr_command::BindKey>() {
                Ok(k) => k,
                Err(e) => {
                    errors.push(format!("config [keys.{table}]: {e}"));
                    continue;
                }
            };
            // What the file leaves bound, as a whole reload would: a bind,
            // or none for `false`.
            match file.keys.get(table).and_then(|t| t.get(key)) {
                Some(tmxr_config::BindEntry::Bind(b)) => self.keys.bind(
                    table,
                    bind_key,
                    crate::keys::BindSpec {
                        cmd: b.cmd.clone(),
                        note: b.note.clone(),
                        repeat: b.repeat,
                    },
                ),
                _ => {
                    self.keys.unbind(table, &bind_key);
                }
            }
        }
        for name in &sets.hooks {
            if !crate::hooks::known(name) {
                errors.push(format!("hooks.{name}: unknown hook"));
                continue;
            }
            match file.hooks.get(name) {
                Some(commands) => self.hooks.insert(name.clone(), commands.clone()),
                None => self.hooks.remove(name),
            };
        }
        if sets.navigator {
            self.cfg.navigator = file.navigator.clone();
            self.navigator = match navigator_regex(&self.cfg.navigator.pattern) {
                Ok(re) => Some(re),
                Err(e) => {
                    errors.push(e);
                    None
                }
            };
        }
        if sets.resurrect {
            self.cfg.resurrect = file.resurrect;
        }
        self.apply_rgb_colour();
        self.mark_all_dirty();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

/// What a bare `tmxr` runs: the session named after the directory it was
/// run in, attached if it is there already (`new-session -A -s <name>`), as
/// the owner's `t` script did with tmux. A `:` in the name, which a session
/// name cannot hold, becomes `_`; a directory without a name (`/`, `C:\`)
/// gets tmux's numbered session.
pub fn bare_command(cwd: &str) -> Vec<String> {
    let name = std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().replace(':', "_"))
        .filter(|n| !n.is_empty());
    match name {
        Some(name) => vec!["new-session".into(), "-A".into(), "-s".into(), name],
        None => vec!["new-session".into()],
    }
}

/// `split-window -l` size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitSize {
    Cells(u16),
    Percent(u16),
}

/// The file a config was read from, none for the defaults alone.
pub(crate) fn config_file(source: tmxr_config::ConfigSource) -> Option<PathBuf> {
    match source {
        tmxr_config::ConfigSource::File(path) => Some(path),
        tmxr_config::ConfigSource::Defaults => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_commands_wake_the_server_only_away_from_a_status_redraw() {
        let start = Instant::now();
        let every = Duration::from_secs(5);
        let at = |ms: u64| start + Duration::from_millis(ms);
        // No status line: their own deadline.
        assert_eq!(refresh_deadline(start, every, &[]), Some(at(5000)));
        // A redraw just after it, or up to half an interval before or after:
        // none, the redraw refreshes them.
        for redraw in [5002, 2500, 7500] {
            assert_eq!(
                refresh_deadline(start, every, &[at(redraw)]),
                None,
                "{redraw}"
            );
        }
        // A redraw further off: their own again.
        for redraw in [2499, 7501] {
            assert_eq!(
                refresh_deadline(start, every, &[at(redraw)]),
                Some(at(5000)),
                "{redraw}"
            );
        }
        // Any of several clients' redraws near it will do.
        assert_eq!(refresh_deadline(start, every, &[at(9000), at(5100)]), None);
    }

    #[test]
    fn a_bare_tmxr_opens_the_session_named_after_its_directory() {
        let cmd = |cwd: &str| bare_command(cwd).join(" ");
        assert_eq!(cmd("/home/me/my.app"), "new-session -A -s my.app");
        assert_eq!(cmd("/home/me/a:b"), "new-session -A -s a_b");
        assert_eq!(cmd("/"), "new-session");
        assert_eq!(cmd(""), "new-session");
        if cfg!(windows) {
            assert_eq!(cmd(r"C:\Users\me\tmxr"), "new-session -A -s tmxr");
            assert_eq!(cmd(r"C:\"), "new-session");
        }
    }

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
