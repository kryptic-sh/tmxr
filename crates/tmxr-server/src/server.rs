//! The server: one thread owns all state and processes [`Event`]s in order.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::time::{Duration, Instant, SystemTime};

use crossterm::event::{Event as TermEvent, KeyEvent, KeyEventKind};
use hjkl_layout::{LayoutTree, SplitDir};
use ratatui::{Terminal, TerminalOptions, Viewport, layout::Rect};
use tmxr_command::Key;
use tmxr_config::Config;
use tmxr_proto::socket::Endpoint;
use tmxr_proto::{ClientMsg, Hello, PROTOCOL_VERSION, ServerMsg};
use tmxr_term::{Emulator, Pty, PtyEvent, SpawnSpec, encode_key, encode_paste};
use tracing::{debug, info, warn};

use crate::backend::AnsiBackend;
use crate::cmds::{Ctx, Outcome};
use crate::keys::KeyTables;
use crate::model::{ClientId, Pane, PaneId, Session, SessionId, Window, WindowId};
use crate::overlay::{Overlay, OverlayAction};

/// Everything that can wake the server.
pub enum Event {
    Connected {
        id: ClientId,
        tx: SyncSender<ServerMsg>,
    },
    Msg(ClientId, ClientMsg),
    Disconnected(ClientId),
    Pty(PaneId, PtyEvent),
    /// A `run-shell` command finished.
    Shell {
        client: Option<ClientId>,
        output: String,
    },
    /// A command `if-shell` chose once its shell command finished.
    Run {
        ctx: Ctx,
        cmd: String,
    },
}

pub struct Client {
    pub id: ClientId,
    pub tx: SyncSender<ServerMsg>,
    pub hello: Option<Hello>,
    pub att: Option<Attached>,
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
    pub last_input: Instant,
    pub drag: Option<crate::mouse::Drag>,
    pub status_ranges: crate::render::StatusRanges,
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
    next_pane: PaneId,
    next_buffer: u32,
    had_session: bool,
    pub exiting: bool,
    last_status: Instant,
    /// Per-pane foreground command cache refreshed on the status interval.
    pub commands: HashMap<PaneId, String>,
    /// Sessions were restored at start-up and no client has attached yet: a
    /// bare `tmxr` attaches to them instead of creating a new session.
    pub restored_pending: bool,
    last_save: Instant,
}

/// The navigator pattern, anchored to the whole process name. An invalid
/// pattern is reported and leaves the navigator off: `C-h/j/k/l` then always
/// move between panes.
fn navigator_regex(pattern: &str) -> Result<regex::Regex, String> {
    regex::Regex::new(&format!("^(?:{pattern})$")).map_err(|e| format!("navigator.pattern: {e}"))
}

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
            next_buffer: 0,
            had_session: false,
            exiting: false,
            last_status: Instant::now(),
            commands: HashMap::new(),
            restored_pending: false,
            last_save: Instant::now(),
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
            match rx.recv_timeout(tick) {
                Ok(ev) => {
                    self.handle(ev);
                    while let Ok(ev) = rx.try_recv() {
                        self.handle(ev);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.tick();
            self.render_all();
            if self.exiting {
                break;
            }
        }
        if let Err(e) = crate::resurrect::save(&mut self) {
            warn!("resurrect save on exit failed: {e}");
        }
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
            Event::Connected { id, tx } => {
                self.clients.insert(
                    id,
                    Client {
                        id,
                        tx,
                        hello: None,
                        att: None,
                    },
                );
            }
            Event::Disconnected(id) => {
                if let Some(c) = self.clients.remove(&id)
                    && c.att.is_some()
                {
                    debug!(client = id, "attached client went away");
                }
            }
            Event::Msg(id, msg) => self.client_msg(id, msg),
            Event::Pty(pane, ev) => self.pty_event(pane, ev),
            Event::Shell { client, output } => {
                let output = output.trim_end().to_owned();
                if let Some(c) = client
                    && !output.is_empty()
                {
                    let lines: Vec<String> = output.lines().map(str::to_owned).collect();
                    if lines.len() == 1 {
                        self.show_message(c, output);
                    } else if let Some(a) = self.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                        a.overlay = Some(Overlay::text(lines));
                        a.dirty = true;
                    }
                }
            }
            Event::Run { ctx, cmd } => {
                let out = crate::cmds::run_string(self, &ctx, &cmd);
                if let Some(c) = ctx.client {
                    self.report(c, &out);
                }
            }
        }
    }

    fn send(&mut self, client: ClientId, msg: ServerMsg) {
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
            ClientMsg::Input(ev) => self.input(id, ev),
            ClientMsg::Detach => self.detach(id, "detached"),
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

    // ── Attaching ───────────────────────────────────────────────────────────

    pub fn attach(&mut self, id: ClientId, session: SessionId) {
        let Some(term) = self.client_terminal(id).cloned() else {
            return;
        };
        let (cols, rows) = (term.cols.max(1), term.rows.max(1));
        let area = Rect::new(0, 0, cols, rows);
        let terminal = Terminal::with_options(
            AnsiBackend::new(cols, rows),
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )
        .ok();
        let Some(c) = self.clients.get_mut(&id) else {
            return;
        };
        c.att = Some(Attached {
            session,
            cols,
            rows,
            term: terminal,
            table: "root".into(),
            repeat_until: None,
            overlay: None,
            message: None,
            last_session: None,
            dirty: true,
            full_redraw: true,
            last_input: Instant::now(),
            drag: None,
            status_ranges: Vec::new(),
        });
        self.restored_pending = false;
        self.send(id, ServerMsg::Attached);
        self.touch_session(session);
        self.size_session(session);
    }

    pub fn detach(&mut self, id: ClientId, reason: &str) {
        let Some(c) = self.clients.get_mut(&id) else {
            return;
        };
        let Some(att) = c.att.take() else {
            return;
        };
        let name = self
            .sessions
            .get(&att.session)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let reason = if reason == "detached" {
            format!("detached (from session {name})")
        } else {
            reason.to_owned()
        };
        // Leave the alternate screen state as the client found it; the client
        // restores its own terminal modes on receiving this.
        self.send(id, ServerMsg::Detached { reason });
        self.clients.remove(&id);
    }

    /// Point an attached client at another session.
    pub fn switch_client(&mut self, id: ClientId, session: SessionId) {
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        if att.session != session {
            att.last_session = Some(att.session);
            att.session = session;
        }
        att.dirty = true;
        att.full_redraw = true;
        self.touch_session(session);
        self.size_session(session);
    }

    pub fn touch_session(&mut self, session: SessionId) {
        if let Some(s) = self.sessions.get_mut(&session) {
            s.last_used = Instant::now();
        }
    }

    /// Size every window of `session` for the attached client that used it
    /// most recently (tmux's `window-size latest`).
    pub fn size_session(&mut self, session: SessionId) {
        let size = self
            .clients
            .values()
            .filter_map(|c| c.att.as_ref())
            .filter(|a| a.session == session)
            .max_by_key(|a| a.last_input)
            .map(|a| (a.cols, a.rows.saturating_sub(STATUS_ROWS).max(1)));
        let Some((cols, rows)) = size else {
            return;
        };
        let windows: Vec<WindowId> = self
            .sessions
            .get(&session)
            .map(|s| s.windows.values().copied().collect())
            .unwrap_or_default();
        for w in windows {
            if let Some(win) = self.windows.get_mut(&w)
                && (win.cols, win.rows) != (cols, rows)
            {
                win.cols = cols;
                win.rows = rows;
                self.relayout(w);
            }
        }
    }

    /// Recompute pane rects for a window and resize panes whose size changed.
    pub fn relayout(&mut self, window: WindowId) {
        let Some(win) = self.windows.get(&window) else {
            return;
        };
        for (pid, r) in win.visible_rects() {
            if let Some(p) = self.panes.get_mut(&pid) {
                let resized = (p.rect.w, p.rect.h) != (r.w, r.h);
                p.rect = r;
                if resized && r.w > 0 && r.h > 0 {
                    p.emu.resize(r.h, r.w);
                    if let Err(e) = p.pty.resize(r.h, r.w) {
                        debug!(pane = pid, error = %e, "pty resize failed");
                    }
                    if let Some(cm) = p.copy.as_mut() {
                        cm.resize(r.w, r.h);
                    }
                }
            }
        }
        self.mark_window_dirty(window);
    }

    // ── Sessions, windows, panes ────────────────────────────────────────────

    pub fn alloc_session_id(&mut self) -> SessionId {
        let id = self.next_session;
        self.next_session += 1;
        id
    }

    /// The first unused numeric session name ("0", "1", …), as tmux does.
    pub fn next_session_name(&self) -> String {
        (0..)
            .map(|n: u32| n.to_string())
            .find(|n| !self.sessions.values().any(|s| &s.name == n))
            .unwrap_or_default()
    }

    pub fn new_session(
        &mut self,
        name: Option<String>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        window_name: Option<String>,
        argv: Vec<String>,
        size: (u16, u16),
    ) -> Result<SessionId, String> {
        let name = match name {
            Some(n) if self.sessions.values().any(|s| s.name == n) => {
                return Err(format!("duplicate session: {n}"));
            }
            Some(n) if n.is_empty() || n.contains([':', '.']) => {
                return Err(format!("bad session name: {n}"));
            }
            Some(n) => n,
            None => self.next_session_name(),
        };
        let id = self.alloc_session_id();
        self.sessions.insert(
            id,
            Session {
                id,
                name,
                windows: BTreeMap::new(),
                current: self.cfg.base_index,
                last: None,
                cwd: cwd.clone(),
                env,
                created: SystemTime::now(),
                last_used: Instant::now(),
            },
        );
        if let Err(e) = self.new_window(id, None, window_name, Some(cwd), argv, size, true) {
            self.sessions.remove(&id);
            return Err(e);
        }
        self.had_session = true;
        Ok(id)
    }

    /// Create a window in `session` at `index` (or the first free index).
    #[allow(clippy::too_many_arguments)]
    pub fn new_window(
        &mut self,
        session: SessionId,
        index: Option<u32>,
        name: Option<String>,
        cwd: Option<PathBuf>,
        argv: Vec<String>,
        size: (u16, u16),
        select: bool,
    ) -> Result<WindowId, String> {
        let s = self.sessions.get(&session).ok_or("no such session")?;
        let index = match index {
            Some(i) if s.windows.contains_key(&i) => return Err(format!("index {i} in use")),
            Some(i) => i,
            None => (self.cfg.base_index..)
                .find(|i| !s.windows.contains_key(i))
                .unwrap_or(self.cfg.base_index),
        };
        let cwd = cwd.unwrap_or_else(|| s.cwd.clone());
        let env = s.env.clone();
        let wid = self.next_window;
        self.next_window += 1;
        let pid = self.next_pane;
        let (cols, rows) = size;
        let auto_name = name.is_none();
        self.windows.insert(
            wid,
            Window {
                id: wid,
                name: name.unwrap_or_default(),
                auto_name,
                layout: LayoutTree::Leaf(pid as usize),
                active: pid,
                last_pane: None,
                zoomed: false,
                synchronize: false,
                bell: false,
                cols,
                rows,
                preset: 0,
            },
        );
        if let Err(e) = self.spawn_pane(pid, wid, session, &argv, cwd, &env, cols, rows) {
            self.windows.remove(&wid);
            return Err(e);
        }
        if let Some(win) = self.windows.get_mut(&wid)
            && win.auto_name
        {
            win.name = crate::util::program_name(&argv, self.cfg.default_shell.as_deref());
        }
        let s = self.sessions.get_mut(&session).ok_or("no such session")?;
        s.windows.insert(index, wid);
        if select || s.windows.len() == 1 {
            if s.current != index && s.windows.contains_key(&s.current) {
                s.last = Some(s.current);
            }
            s.current = index;
        }
        self.mark_session_dirty(session);
        Ok(wid)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_pane(
        &mut self,
        pid: PaneId,
        window: WindowId,
        session: SessionId,
        argv: &[String],
        cwd: PathBuf,
        session_env: &[(String, String)],
        cols: u16,
        rows: u16,
    ) -> Result<(), String> {
        let mut env: Vec<(String, String)> = session_env.to_vec();
        env.push((
            "TERM".into(),
            crate::util::pane_term(&self.cfg.default_terminal),
        ));
        env.push(("COLORTERM".into(), "truecolor".into()));
        env.push((
            tmxr_proto::socket::ENV_TMXR.into(),
            tmxr_proto::socket::format_tmxr_env(&self.endpoint, self.pid, session),
        ));
        env.push((tmxr_proto::socket::ENV_TMXR_PANE.into(), format!("%{pid}")));
        let argv = if argv.is_empty() {
            self.cfg
                .default_shell
                .clone()
                .map(|s| vec![s])
                .unwrap_or_default()
        } else if argv.len() == 1 && argv[0].contains(' ') {
            crate::util::shell_command(&argv[0])
        } else {
            argv.to_vec()
        };
        let cwd = if cwd.is_dir() {
            cwd
        } else {
            crate::util::home_dir()
        };
        let spec = SpawnSpec {
            argv,
            cwd: Some(cwd.clone()),
            env,
            rows,
            cols,
        };
        let events = self.events.clone();
        let sink: Arc<dyn Fn(PtyEvent) + Send + Sync> = Arc::new(move |e| {
            let _ = events.send(Event::Pty(pid, e));
        });
        let pty = Pty::spawn(&spec, sink).map_err(|e| format!("could not start pane: {e}"))?;
        self.next_pane = self.next_pane.max(pid + 1);
        self.panes.insert(
            pid,
            Pane {
                id: pid,
                window,
                pty,
                emu: Emulator::new(rows.max(1), cols.max(1), self.cfg.history_limit),
                rect: hjkl_layout::LayoutRect::new(0, 0, cols, rows),
                start_cwd: cwd,
                copy: None,
            },
        );
        Ok(())
    }

    /// Split `target` and start a new pane. `horizontal` is tmux's `-h`
    /// (side by side); `before` is `-b`; `size` is the new pane's cells.
    #[allow(clippy::too_many_arguments)]
    pub fn split(
        &mut self,
        target: PaneId,
        horizontal: bool,
        before: bool,
        size: Option<SplitSize>,
        cwd: PathBuf,
        argv: Vec<String>,
        focus: bool,
    ) -> Result<PaneId, String> {
        let wid = self.panes.get(&target).ok_or("no such pane")?.window;
        let session = self.session_of_window(wid).ok_or("window has no session")?;
        let env = self
            .sessions
            .get(&session)
            .map(|s| s.env.clone())
            .unwrap_or_default();
        let new = self.next_pane;
        let win = self.windows.get_mut(&wid).ok_or("no such window")?;
        win.zoomed = false;
        let dir = if horizontal {
            SplitDir::Vertical
        } else {
            SplitDir::Horizontal
        };
        let target_rect = self.panes.get(&target).map(|p| p.rect).unwrap_or_default();
        let len = if horizontal {
            target_rect.w
        } else {
            target_rect.h
        };
        if len < 3 {
            return Err("pane too small".into());
        }
        // Fraction of the split given to the *first* child.
        let new_cells = match size {
            Some(SplitSize::Cells(n)) => f32::from(n.min(len - 2)),
            Some(SplitSize::Percent(p)) => f32::from(len) * f32::from(p.min(100)) / 100.0,
            None => f32::from(len) / 2.0,
        };
        let new_frac = (new_cells / f32::from(len)).clamp(0.05, 0.95);
        let ratio = if before { new_frac } else { 1.0 - new_frac };
        let t = target as usize;
        let n = new as usize;
        win.layout.replace_leaf(t, move |id| {
            let (a, b) = if before {
                (LayoutTree::Leaf(n), LayoutTree::Leaf(id))
            } else {
                (LayoutTree::Leaf(id), LayoutTree::Leaf(n))
            };
            LayoutTree::split(dir, ratio, a, b)
        });
        let rects = crate::layout::pane_rects(&win.layout, win.cols, win.rows);
        let r = rects
            .iter()
            .find(|(p, _)| *p == new)
            .map(|(_, r)| *r)
            .unwrap_or_default();
        if let Err(e) = self.spawn_pane(new, wid, session, &argv, cwd, &env, r.w, r.h) {
            if let Some(win) = self.windows.get_mut(&wid) {
                let _ = win.layout.remove_leaf(n);
            }
            return Err(e);
        }
        if focus {
            self.select_pane(new);
        }
        self.relayout(wid);
        Ok(new)
    }

    pub fn session_of_window(&self, window: WindowId) -> Option<SessionId> {
        self.sessions
            .values()
            .find(|s| s.windows.values().any(|w| *w == window))
            .map(|s| s.id)
    }

    pub fn select_pane(&mut self, pane: PaneId) {
        let Some(wid) = self.panes.get(&pane).map(|p| p.window) else {
            return;
        };
        if let Some(win) = self.windows.get_mut(&wid)
            && win.active != pane
        {
            win.last_pane = Some(win.active);
            win.active = pane;
            if win.zoomed {
                win.zoomed = false;
                self.relayout(wid);
            }
        }
        self.mark_window_dirty(wid);
    }

    /// Make window `index` current in `session`.
    pub fn select_window(&mut self, session: SessionId, index: u32) -> Result<(), String> {
        let s = self.sessions.get_mut(&session).ok_or("no such session")?;
        if !s.windows.contains_key(&index) {
            return Err(format!("window not found: {index}"));
        }
        if s.current != index {
            s.last = Some(s.current);
            s.current = index;
        }
        if let Some(w) = s.windows.get(&index).copied()
            && let Some(win) = self.windows.get_mut(&w)
        {
            win.bell = false;
        }
        self.mark_session_dirty(session);
        Ok(())
    }

    pub fn kill_pane(&mut self, pane: PaneId) {
        if let Some(mut p) = self.panes.remove(&pane) {
            let _ = p.pty.kill();
            self.commands.remove(&pane);
            self.remove_pane_from_window(p.window, pane);
        }
    }

    fn remove_pane_from_window(&mut self, wid: WindowId, pane: PaneId) {
        let Some(win) = self.windows.get_mut(&wid) else {
            return;
        };
        match win.layout.remove_leaf(pane as usize) {
            Ok(focus) => {
                if win.active == pane {
                    win.active = win
                        .last_pane
                        .filter(|l| *l != pane && win.layout.contains(*l as usize))
                        .unwrap_or(focus as PaneId);
                }
                if win.last_pane == Some(pane) {
                    win.last_pane = None;
                }
                win.zoomed = false;
                self.relayout(wid);
            }
            Err(_) => self.kill_window(wid),
        }
    }

    pub fn kill_window(&mut self, wid: WindowId) {
        let Some(win) = self.windows.remove(&wid) else {
            return;
        };
        for p in win.panes() {
            if let Some(mut pane) = self.panes.remove(&p) {
                let _ = pane.pty.kill();
            }
            self.commands.remove(&p);
        }
        let Some(sid) = self
            .sessions
            .values()
            .find(|s| s.windows.values().any(|w| *w == wid))
            .map(|s| s.id)
        else {
            return;
        };
        let renumber = self.cfg.renumber_windows;
        let base = self.cfg.base_index;
        let s = self.sessions.get_mut(&sid).expect("session found above");
        let index = s.index_of(wid);
        if let Some(i) = index {
            s.windows.remove(&i);
        }
        if s.windows.is_empty() {
            self.kill_session(sid);
            return;
        }
        if index == Some(s.current) {
            // tmux moves to the last window, else the next one.
            let next = s
                .last
                .filter(|l| s.windows.contains_key(l))
                .or_else(|| s.windows.range(s.current..).next().map(|(i, _)| *i))
                .or_else(|| s.windows.keys().next_back().copied());
            if let Some(n) = next {
                s.current = n;
            }
            s.last = None;
        }
        if renumber {
            let current_win = s.windows.get(&s.current).copied();
            let last_win = s.last.and_then(|l| s.windows.get(&l).copied());
            let ordered: Vec<WindowId> = s.windows.values().copied().collect();
            s.windows = ordered
                .iter()
                .enumerate()
                .map(|(i, w)| (base + i as u32, *w))
                .collect();
            if let Some(c) = current_win.and_then(|w| s.index_of(w)) {
                s.current = c;
            }
            s.last = last_win.and_then(|w| s.index_of(w));
        }
        self.mark_session_dirty(sid);
    }

    pub fn kill_session(&mut self, sid: SessionId) {
        let Some(s) = self.sessions.remove(&sid) else {
            return;
        };
        for w in s.windows.values().copied().collect::<Vec<_>>() {
            if let Some(win) = self.windows.remove(&w) {
                for p in win.panes() {
                    if let Some(mut pane) = self.panes.remove(&p) {
                        let _ = pane.pty.kill();
                    }
                }
            }
        }
        // Clients on the dead session move to another one, or detach when
        // there is none (tmux's detach-on-destroy off / on fallback).
        let fallback = self
            .sessions
            .values()
            .max_by_key(|s| s.last_used)
            .map(|s| s.id);
        let on_it: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| c.att.as_ref().is_some_and(|a| a.session == sid))
            .map(|c| c.id)
            .collect();
        for c in on_it {
            match fallback {
                Some(f) => self.switch_client(c, f),
                None => self.detach(c, "exited"),
            }
        }
        if self.sessions.is_empty() && self.had_session {
            self.exiting = true;
        }
    }

    fn pty_event(&mut self, pid: PaneId, ev: PtyEvent) {
        match ev {
            PtyEvent::Output(bytes) => {
                let Some(p) = self.panes.get_mut(&pid) else {
                    return;
                };
                let replies = p.emu.process(&bytes);
                if !replies.is_empty() {
                    let _ = p.pty.write(&replies);
                }
                let bell = p.emu.take_bell();
                let clips = p.emu.take_clipboard();
                let wid = p.window;
                if bell && let Some(w) = self.windows.get_mut(&wid) {
                    w.bell = true;
                }
                for (_, data) in clips {
                    self.forward_osc52(&data);
                }
                self.mark_window_dirty(wid);
            }
            PtyEvent::Exited(code) => {
                debug!(pane = pid, ?code, "pane exited");
                if self.panes.contains_key(&pid) {
                    self.panes.remove(&pid);
                    self.commands.remove(&pid);
                    let wid = self
                        .windows
                        .values()
                        .find(|w| w.layout.contains(pid as usize))
                        .map(|w| w.id);
                    if let Some(w) = wid {
                        self.remove_pane_from_window(w, pid);
                    }
                }
            }
            PtyEvent::Eof => {}
        }
    }

    // ── Input ───────────────────────────────────────────────────────────────

    fn input(&mut self, id: ClientId, ev: TermEvent) {
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        att.last_input = Instant::now();
        let session = att.session;
        match ev {
            TermEvent::Key(k) if k.kind != KeyEventKind::Release => self.key(id, k),
            TermEvent::Key(_) => {}
            TermEvent::Paste(text) => {
                if let Some(ov) = self
                    .clients
                    .get_mut(&id)
                    .and_then(|c| c.att.as_mut())
                    .and_then(|a| a.overlay.as_mut())
                {
                    ov.paste(&text);
                    self.mark_client_dirty(id);
                } else {
                    self.paste_to_active(session, &text);
                }
            }
            TermEvent::Resize(cols, rows) => {
                if let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                    att.cols = cols.max(1);
                    att.rows = rows.max(1);
                    att.full_redraw = true;
                    att.dirty = true;
                    if let Some(t) = att.term.as_mut() {
                        t.backend_mut().set_size(att.cols, att.rows);
                        let _ = t.resize(Rect::new(0, 0, att.cols, att.rows));
                    }
                }
                self.size_session(session);
            }
            TermEvent::Mouse(m) => crate::mouse::handle(self, id, m),
            TermEvent::FocusGained | TermEvent::FocusLost => {}
        }
    }

    pub fn active_pane_of_session(&self, session: SessionId) -> Option<PaneId> {
        let w = self.sessions.get(&session)?.current_window()?;
        self.windows.get(&w).map(|w| w.active)
    }

    fn key(&mut self, id: ClientId, ev: KeyEvent) {
        let Some(att) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
            return;
        };
        if let Some(mut ov) = att.overlay.take() {
            let action = ov.key(&ev);
            match action {
                OverlayAction::Keep => {
                    if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                        a.overlay = Some(ov);
                    }
                }
                OverlayAction::Close => {}
                OverlayAction::Run(cmd) => self.run_bind(id, &cmd, None),
                OverlayAction::Switch(sid) => self.switch_client(id, sid),
                OverlayAction::SwitchWindow(sid, idx) => {
                    self.switch_client(id, sid);
                    let _ = self.select_window(sid, idx);
                }
            }
            self.mark_client_dirty(id);
            return;
        }
        let key = Key::from_event(&ev);
        let session = att.session;
        let now = Instant::now();
        let repeating = att.repeat_until.is_some_and(|t| t > now);
        let table = std::mem::replace(&mut att.table, "root".into());
        att.repeat_until = None;
        if table != "root" {
            att.dirty = true;
            if let Some(b) = self.keys.get(&table, &key).cloned() {
                if !repeating || b.repeat {
                    if b.repeat
                        && let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut())
                    {
                        a.table.clone_from(&table);
                        a.repeat_until = Some(now + Duration::from_millis(self.cfg.repeat_time));
                    }
                    self.run_bind(id, &b.cmd, Some(ev));
                    return;
                }
            } else if !repeating {
                return;
            }
            // A non-repeatable key while repeating is handled as a fresh key.
        }
        if key == self.prefix {
            if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                a.table = "prefix".into();
                a.dirty = true;
            }
            return;
        }
        let active = self.active_pane_of_session(session);
        let in_copy = active
            .and_then(|p| self.panes.get(&p))
            .is_some_and(|p| p.copy.is_some());
        if in_copy {
            if let Some(b) = self.keys.get("copy-mode-vi", &key).cloned() {
                self.run_bind(id, &b.cmd, Some(ev));
            }
            return;
        }
        if let Some(b) = self.keys.get("root", &key).cloned() {
            self.run_bind(id, &b.cmd, Some(ev));
            return;
        }
        if let Some(p) = active {
            self.send_key_to_pane(p, &ev);
        }
    }

    /// Run a bind's command list for an attached client; errors become a
    /// status-line message.
    pub fn run_bind(&mut self, id: ClientId, cmd: &str, key: Option<KeyEvent>) {
        let session = self
            .clients
            .get(&id)
            .and_then(|c| c.att.as_ref())
            .map(|a| a.session);
        let pane = session.and_then(|s| self.active_pane_of_session(s));
        let ctx = Ctx {
            client: Some(id),
            pane,
            key,
            cwd: None,
            env: Vec::new(),
        };
        let out = crate::cmds::run_string(self, &ctx, cmd);
        self.report(id, &out);
    }

    /// Show what a command run for an attached client printed: an error or a
    /// one-line result in the status line, longer output in a text view.
    fn report(&mut self, id: ClientId, out: &Outcome) {
        if !out.stderr.is_empty() {
            self.show_message(id, out.stderr.trim_end().to_owned());
        } else if !out.stdout.is_empty() {
            let lines: Vec<String> = out.stdout.lines().map(str::to_owned).collect();
            if lines.len() == 1 {
                self.show_message(id, lines[0].clone());
            } else if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                a.overlay = Some(Overlay::text(lines));
            }
        }
        self.mark_client_dirty(id);
    }

    /// Encode a key for a pane (and, with synchronize-panes, its siblings).
    pub fn send_key_to_pane(&mut self, pane: PaneId, ev: &KeyEvent) {
        let always = self.cfg.extended_keys == "always";
        let targets = self.input_targets(pane);
        for t in targets {
            if let Some(p) = self.panes.get_mut(&t) {
                let bytes = encode_key(ev, p.emu.input_modes(), always);
                if !bytes.is_empty() {
                    let _ = p.pty.write(&bytes);
                }
            }
        }
    }

    pub fn paste_to_active(&mut self, session: SessionId, text: &str) {
        if let Some(p) = self.active_pane_of_session(session) {
            self.paste_to_pane(p, text);
        }
    }

    pub fn paste_to_pane(&mut self, pane: PaneId, text: &str) {
        for t in self.input_targets(pane) {
            if let Some(p) = self.panes.get_mut(&t) {
                let bytes = encode_paste(text, p.emu.input_modes());
                let _ = p.pty.write(&bytes);
            }
        }
    }

    /// The pane itself, plus its siblings when the window synchronizes input.
    fn input_targets(&self, pane: PaneId) -> Vec<PaneId> {
        let Some(wid) = self.panes.get(&pane).map(|p| p.window) else {
            return Vec::new();
        };
        match self.windows.get(&wid) {
            Some(w) if w.synchronize => w.panes(),
            _ => vec![pane],
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
                if a.overlay.as_mut().is_some_and(Overlay::tick) {
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
            if let Err(e) = crate::resurrect::save(self) {
                self.log_message(format!("resurrect auto-save failed: {e}"));
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
        self.mark_all_dirty();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    /// Give `pane` (already removed from its old window's layout) a window of
    /// its own in `session`.
    pub fn adopt_pane(
        &mut self,
        session: SessionId,
        pane: PaneId,
        name: String,
        cols: u16,
        rows: u16,
        select: bool,
    ) -> Result<WindowId, String> {
        let s = self.sessions.get(&session).ok_or("no such session")?;
        let index = (self.cfg.base_index..)
            .find(|i| !s.windows.contains_key(i))
            .unwrap_or(self.cfg.base_index);
        let wid = self.next_window;
        self.next_window += 1;
        self.windows.insert(
            wid,
            Window {
                id: wid,
                name,
                auto_name: true,
                layout: LayoutTree::Leaf(pane as usize),
                active: pane,
                last_pane: None,
                zoomed: false,
                synchronize: false,
                bell: false,
                cols,
                rows,
                preset: 0,
            },
        );
        if let Some(p) = self.panes.get_mut(&pane) {
            p.window = wid;
        }
        let s = self.sessions.get_mut(&session).ok_or("no such session")?;
        s.windows.insert(index, wid);
        if select {
            s.last = Some(s.current);
            s.current = index;
        }
        self.mark_session_dirty(session);
        Ok(wid)
    }
}

/// `run-shell`: run `line` with the shell off the server thread and report
/// its output to `client` (unless `background`).
pub fn run_shell(srv: &Server, client: Option<ClientId>, line: String, background: bool) {
    let events = srv.events.clone();
    let _ = std::thread::Builder::new()
        .name("tmxr-run-shell".into())
        .spawn(move || {
            let text = match shell_output(&line) {
                Ok(o) => {
                    let mut t = String::from_utf8_lossy(&o.stdout).into_owned();
                    t.push_str(&String::from_utf8_lossy(&o.stderr));
                    if !o.status.success() && t.is_empty() {
                        t = format!("'{line}' returned {}", o.status.code().unwrap_or(-1));
                    }
                    t
                }
                Err(e) => format!("'{line}' failed: {e}"),
            };
            let client = if background { None } else { client };
            let _ = events.send(Event::Shell {
                client,
                output: text,
            });
        });
}

/// `if-shell`: run `line` in the background, then run `then` if it exited
/// successfully, else `otherwise`, with `ctx` as the command context.
pub fn if_shell(srv: &Server, ctx: Ctx, line: String, then: String, otherwise: Option<String>) {
    let events = srv.events.clone();
    let _ = std::thread::Builder::new()
        .name("tmxr-if-shell".into())
        .spawn(move || {
            let ok = shell_output(&line).is_ok_and(|o| o.status.success());
            if let Some(cmd) = if ok { Some(then) } else { otherwise } {
                let _ = events.send(Event::Run { ctx, cmd });
            }
        });
}

/// Run a shell command line to completion with no input.
fn shell_output(line: &str) -> std::io::Result<std::process::Output> {
    let argv = crate::util::shell_command(line);
    std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .output()
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
