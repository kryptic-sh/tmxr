//! Format variables (`#{session_name}`, `#{pane_current_path}`, …).

use std::path::PathBuf;

use tmxr_command::format::{Context, LoopItem, LoopKind, NameKind};

use crate::model::{ClientId, PaneId, SessionId, WindowId};
use crate::server::{Attached, Server};

/// Every variable [`Vars`] answers, sorted, for `display-message -a`.
pub const NAMES: &[&str] = &[
    "active_window_index",
    "alternate_on",
    "alternate_saved_x",
    "alternate_saved_y",
    "buffer_created",
    "buffer_full",
    "buffer_mode_format",
    "buffer_name",
    "buffer_sample",
    "buffer_size",
    "client_activity",
    "client_created",
    "client_flags",
    "client_height",
    "client_key_table",
    "client_last_session",
    "client_mode_format",
    "client_name",
    "client_pid",
    "client_prefix",
    "client_readonly",
    "client_session",
    "client_termname",
    "client_theme",
    "client_tty",
    "client_uid",
    "client_user",
    "client_width",
    "command",
    "config_files",
    "copy_cursor_hyperlink",
    "copy_cursor_line",
    "copy_cursor_word",
    "copy_cursor_x",
    "copy_cursor_y",
    "cursor_blinking",
    "cursor_character",
    "cursor_colour",
    "cursor_flag",
    "cursor_shape",
    "cursor_very_visible",
    "cursor_x",
    "cursor_y",
    "history_limit",
    "history_size",
    "host",
    "host_short",
    "insert_flag",
    "keypad_cursor_flag",
    "keypad_flag",
    "last_window_index",
    "loop_last_flag",
    "mouse_all_flag",
    "mouse_any_flag",
    "mouse_button_flag",
    "mouse_sgr_flag",
    "mouse_standard_flag",
    "mouse_utf8_flag",
    "next_session_id",
    "origin_flag",
    "pane_active",
    "pane_at_bottom",
    "pane_at_left",
    "pane_at_right",
    "pane_at_top",
    "pane_bg",
    "pane_bottom",
    "pane_current_command",
    "pane_current_path",
    "pane_dead",
    "pane_dead_signal",
    "pane_dead_status",
    "pane_dead_time",
    "pane_fg",
    "pane_format",
    "pane_height",
    "pane_id",
    "pane_in_mode",
    "pane_index",
    "pane_input_off",
    "pane_key_mode",
    "pane_last",
    "pane_left",
    "pane_marked",
    "pane_marked_set",
    "pane_mode",
    "pane_pid",
    "pane_pipe",
    "pane_right",
    "pane_search_string",
    "pane_start_command",
    "pane_start_path",
    "pane_synchronized",
    "pane_tabs",
    "pane_title",
    "pane_top",
    "pane_tty",
    "pane_unseen_changes",
    "pane_width",
    "pid",
    "rectangle_toggle",
    "scroll_position",
    "scroll_region_lower",
    "scroll_region_upper",
    "search_count",
    "search_count_partial",
    "search_match",
    "search_present",
    "search_timed_out",
    "selection_active",
    "selection_end_x",
    "selection_end_y",
    "selection_present",
    "selection_start_x",
    "selection_start_y",
    "server_sessions",
    "session_activity",
    "session_activity_flag",
    "session_alert",
    "session_alerts",
    "session_attached",
    "session_attached_list",
    "session_bell_flag",
    "session_created",
    "session_format",
    "session_group",
    "session_group_attached",
    "session_group_attached_list",
    "session_group_list",
    "session_group_many_attached",
    "session_group_size",
    "session_grouped",
    "session_id",
    "session_last_attached",
    "session_many_attached",
    "session_marked",
    "session_name",
    "session_path",
    "session_silence_flag",
    "session_stack",
    "session_windows",
    "sixel_support",
    "socket_path",
    "start_time",
    "top_line_time",
    "tree_mode_format",
    "uid",
    "user",
    "version",
    "window_active",
    "window_active_clients",
    "window_active_clients_list",
    "window_active_sessions",
    "window_active_sessions_list",
    "window_activity",
    "window_activity_flag",
    "window_bell_flag",
    "window_cell_height",
    "window_cell_width",
    "window_end_flag",
    "window_flags",
    "window_format",
    "window_height",
    "window_id",
    "window_index",
    "window_last_flag",
    "window_layout",
    "window_linked",
    "window_linked_sessions",
    "window_linked_sessions_list",
    "window_marked_flag",
    "window_name",
    "window_panes",
    "window_raw_flags",
    "window_silence_flag",
    "window_stack_index",
    "window_start_flag",
    "window_visible_layout",
    "window_width",
    "window_zoomed_flag",
    "wrap_flag",
];

/// tmux 3.6's default `choose-buffer` format, as its `#{buffer_mode_format}`.
const BUFFER_MODE_FORMAT: &str = "#{t/p:buffer_created}: #{buffer_sample}";

/// tmux 3.6's default `choose-client` format (`#{client_mode_format}`).
const CLIENT_MODE_FORMAT: &str = "#{t/p:client_activity}: session #{session_name}";

/// tmux 3.6's default `choose-tree` format (`#{tree_mode_format}`).
const TREE_MODE_FORMAT: &str = "#{?pane_format,#{?pane_marked,#[reverse],}#{pane_current_command}#{?pane_active,*,}#{?pane_marked,M,}#{?#{&&:#{pane_title},#{!=:#{pane_title},#{host_short}}},: \"#{pane_title}\",},window_format,#{?window_marked_flag,#[reverse],}#{window_name}#{window_flags}#{?#{&&:#{==:#{window_panes},1},#{&&:#{pane_title},#{!=:#{pane_title},#{host_short}}}},: \"#{pane_title}\",},#{session_windows} windows#{?session_grouped, (group #{session_group}: #{session_group_list}),}#{?session_attached, (attached),}}";

/// tmux's cell width in pixels for a terminal that reports none.
const CELL_WIDTH_PIXELS: u32 = 16;

/// tmux's cell height in pixels for a terminal that reports none.
const CELL_HEIGHT_PIXELS: u32 = 32;

/// Variables for one (session, window, pane, client) combination.
#[derive(Clone, Copy)]
pub struct Vars<'a> {
    pub srv: &'a Server,
    pub session: Option<SessionId>,
    pub window: Option<WindowId>,
    pub pane: Option<PaneId>,
    pub client: Option<ClientId>,
    /// What the format is for (`#{pane_format}` and the like).
    pub kind: Kind,
}

/// What a format is expanded for, as tmux's format type: the most specific
/// of the session, window and pane its caller named, before the rest are
/// filled in (a window's rows carry its active pane and stay windows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Unknown,
    Session,
    Window,
    Pane,
}

impl Kind {
    /// The kind of a format given these.
    pub fn of(session: Option<SessionId>, window: Option<WindowId>, pane: Option<PaneId>) -> Self {
        if pane.is_some() {
            Self::Pane
        } else if window.is_some() {
            Self::Window
        } else if session.is_some() {
            Self::Session
        } else {
            Self::Unknown
        }
    }
}

impl<'a> Vars<'a> {
    /// Variables for `pane`, filling in its window and session.
    pub fn for_pane(srv: &'a Server, pane: Option<PaneId>, client: Option<ClientId>) -> Self {
        let window = pane.and_then(|p| srv.panes.get(&p)).map(|p| p.window);
        // A linked window's session is the client's when it holds it.
        let session = window.and_then(|w| {
            client
                .and_then(|c| srv.clients.get(&c))
                .and_then(|c| c.att.as_ref())
                .map(|a| a.session)
                .filter(|s| srv.sessions.get(s).is_some_and(|s| s.index_of(w).is_some()))
                .or_else(|| srv.session_of_window(w))
        });
        Self {
            srv,
            session,
            window,
            pane,
            client,
            kind: Kind::of(None, None, pane),
        }
    }
}

/// The directory a pane's foreground program is in: the process's own cwd
/// where the platform exposes it, else what the shell reported with OSC 7,
/// else where the pane started.
pub fn pane_current_path(srv: &Server, pane: PaneId) -> Option<PathBuf> {
    let p = srv.panes.get(&pane)?;
    let process = || tmxr_term::process::current_dir(&p.pty);
    let reported = || p.emu.cwd().map(PathBuf::from);
    // On Unix a shell's `cd` changes its process's directory, the surest
    // source. On Windows PowerShell's `Set-Location` does not, so what the
    // shell reports (OSC 7 / OSC 9;9) comes first there.
    let dir = if cfg!(windows) {
        reported().or_else(process)
    } else {
        process().or_else(reported)
    };
    Some(dir.unwrap_or_else(|| p.start_cwd.clone()))
}

/// tmux's `mouse_*` variables for a command a mouse event ran: where in its
/// pane the mouse was, and the word and line under it (in copy mode, as the
/// view shows them). `None` for other names, or away from a pane.
pub fn mouse_var(
    srv: &Server,
    client: Option<ClientId>,
    mouse: &crate::mouse::MouseTarget,
    name: &str,
) -> Option<String> {
    if !name.starts_with("mouse_") || name == "mouse_any_flag" {
        return None;
    }
    // The range under a click on the status line's first row, read from
    // client `client`'s line as tmux's format_cb_mouse_status_range does.
    if name == "mouse_status_range" {
        use crate::format_draw::Range;
        use tmxr_command::MouseLocation as L;
        let att = srv.clients.get(&client?)?.att.as_ref()?;
        let first = if srv.status_at_top(att.session) {
            0
        } else {
            att.rows.saturating_sub(srv.status_rows(att.session))
        };
        let on_status = matches!(
            mouse.location,
            L::Status | L::StatusLeft | L::StatusRight | L::StatusDefault
        );
        if !on_status || mouse.row != first {
            return Some(String::new());
        }
        return Some(match att.status_ranges.at(mouse.col) {
            (L::StatusLeft, _) => "left".to_owned(),
            (L::StatusRight, _) => "right".to_owned(),
            (_, Some(Range::Pane(_))) => "pane".to_owned(),
            (_, Some(Range::Window(_))) => "window".to_owned(),
            (_, Some(Range::Session(_))) => "session".to_owned(),
            (_, Some(Range::User(text))) => text.clone(),
            _ => String::new(),
        });
    }
    let p = srv.panes.get(&mouse.pane?)?;
    let (x, y) = (
        mouse.col.checked_sub(p.rect.x)?,
        mouse.row.checked_sub(p.rect.y)?,
    );
    let line = || -> Vec<char> {
        match &p.copy {
            Some(cm) => cm.view_line(usize::from(y)).unwrap_or_default(),
            None => p
                .emu
                .screen()
                .rows(0, p.rect.w)
                .nth(usize::from(y))
                .unwrap_or_default()
                .chars()
                .collect(),
        }
    };
    Some(match name {
        "mouse_x" => x.to_string(),
        "mouse_y" => y.to_string(),
        "mouse_word" => {
            let separators = &srv.pane_session_options(Some(p.id)).word_separators;
            crate::copy::word_at(&line(), usize::from(x), separators)
        }
        "mouse_line" => String::from_iter(line()).trim_end().to_owned(),
        // The link under the mouse, as tmux's (in copy mode, as the view
        // shows it).
        "mouse_hyperlink" => {
            let screen = p.emu.screen();
            match &p.copy {
                Some(cm) => cm.view_link(usize::from(y), usize::from(x)),
                None => screen.cell(y, x).and_then(vt100::Cell::hyperlink_id),
            }
            .and_then(|id| screen.hyperlink(id))
            .map(|h| h.uri.clone())
            .unwrap_or_default()
        }
        _ => return None,
    })
}

fn flag(b: bool) -> String {
    if b { "1" } else { "0" }.to_owned()
}

/// The session's windows that have alerts, in index order, each with its
/// alerts as tmux marks them: `#` activity, `!` bell, `~` silence.
fn alerts(srv: &Server, session: &crate::model::Session) -> Vec<(u32, String)> {
    session
        .windows
        .iter()
        .filter_map(|(index, wid)| {
            let w = srv.windows.get(wid)?;
            let marks: String = [(&w.activity, '#'), (&w.bell, '!'), (&w.silence, '~')]
                .into_iter()
                .filter(|(set, _)| set.contains(&session.id))
                .map(|(_, mark)| mark)
                .collect();
            (!marks.is_empty()).then_some((*index, marks))
        })
        .collect()
}

/// The names of the attached clients `keep` takes, in the order they
/// connected as tmux's client list, joined by commas.
fn client_names(srv: &Server, keep: impl Fn(&Attached) -> bool) -> String {
    srv.clients
        .values()
        .filter(|c| c.att.as_ref().is_some_and(&keep))
        .map(|c| c.id.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// The names of the sessions `keep` takes, in name order, joined by commas.
fn session_names(srv: &Server, keep: impl Fn(&crate::model::Session) -> bool) -> String {
    srv.sessions_by_name()
        .iter()
        .filter_map(|id| srv.sessions.get(id))
        .filter(|s| keep(s))
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// A time as tmux's time variables give one: seconds since the epoch.
fn epoch(t: std::time::SystemTime) -> String {
    t.duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        .to_string()
}

/// `#{client_flags}`: the flags tmux's `server_client_get_flags` lists that
/// tmxr keeps, in its order. Every client is `UTF-8`, as tmxr writes
/// nothing else, where tmux's is when the client's locale is.
fn client_flags(c: &crate::server::Client) -> String {
    let att = c.att.as_ref();
    [
        (att.is_some(), "attached"),
        (att.is_some_and(|a| a.focused), "focused"),
        (c.ignore_size, "ignore-size"),
        (c.no_detach_on_destroy, "no-detach-on-destroy"),
        (c.read_only(), "read-only"),
        (true, "UTF-8"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect::<Vec<_>>()
    .join(",")
}

impl<'a> Context for Vars<'a> {
    fn job(&self, cmd: &str, expanded: &str) -> Option<String> {
        Some(self.srv.format_job(self.client, cmd, expanded))
    }

    fn name_exists(&self, kind: NameKind, name: &str) -> Option<bool> {
        let srv = self.srv;
        Some(match kind {
            NameKind::Window => srv
                .sessions
                .get(&self.session?)?
                .windows
                .values()
                .filter_map(|w| srv.windows.get(w))
                .any(|w| w.name == name),
            NameKind::Session => srv.sessions.values().any(|s| s.name == name),
        })
    }

    fn loop_items(&self, kind: LoopKind) -> Option<Vec<LoopItem<'_>>> {
        let srv = self.srv;
        let item = |vars: Vars<'a>, current, index, name: &str, activity| LoopItem {
            ctx: Box::new(vars),
            current,
            index,
            name: name.to_owned(),
            activity,
        };
        // A session's or window's own window and pane, as tmux fills them in.
        let in_window = |session, window: WindowId, kind| Vars {
            srv,
            session,
            window: Some(window),
            pane: srv.windows.get(&window).map(|w| w.active),
            client: self.client,
            kind,
        };
        Some(match kind {
            LoopKind::Sessions => {
                let current = self
                    .client
                    .and_then(|c| srv.clients.get(&c))
                    .and_then(|c| c.att.as_ref())
                    .map(|a| a.session);
                srv.sessions
                    .values()
                    .map(|s| {
                        let mut vars = Vars {
                            session: Some(s.id),
                            window: None,
                            pane: None,
                            kind: Kind::Session,
                            ..*self
                        };
                        if let Some(w) = s.current_window() {
                            vars = in_window(Some(s.id), w, Kind::Session);
                        }
                        item(
                            vars,
                            current == Some(s.id),
                            u64::from(s.id),
                            &s.name,
                            s.activity,
                        )
                    })
                    .collect()
            }
            LoopKind::Windows => {
                let s = srv.sessions.get(&self.session?)?;
                s.windows
                    .iter()
                    .filter_map(|(index, wid)| {
                        let w = srv.windows.get(wid)?;
                        let activity = std::time::SystemTime::now() - w.last_output.elapsed();
                        let vars = in_window(Some(s.id), *wid, Kind::Window);
                        Some(item(
                            vars,
                            *index == s.current,
                            u64::from(*index),
                            &w.name,
                            activity,
                        ))
                    })
                    .collect()
            }
            LoopKind::Panes => {
                let w = srv.windows.get(&self.window?)?;
                w.panes()
                    .into_iter()
                    .map(|pid| {
                        let vars = Vars {
                            pane: Some(pid),
                            kind: Kind::Pane,
                            ..*self
                        };
                        let activity = std::time::SystemTime::now();
                        item(vars, w.active == pid, u64::from(pid), "", activity)
                    })
                    .collect()
            }
            LoopKind::Clients => srv
                .clients
                .values()
                .map(|c| {
                    let vars = Vars {
                        client: Some(c.id),
                        ..*self
                    };
                    // A client's activity is its last key or mouse event,
                    // else when it connected, as tmux's activity_time.
                    let activity = c.att.as_ref().map_or(c.created, |a| {
                        std::time::SystemTime::now() - a.last_input.elapsed()
                    });
                    item(vars, false, u64::from(c.id), &c.id.to_string(), activity)
                })
                .collect(),
        })
    }

    fn pane_lines(&self) -> Option<Vec<String>> {
        let screen = self.srv.panes.get(&self.pane?)?.emu.screen();
        Some(screen.rows(0, tmxr_term::screen_size(screen).1).collect())
    }

    fn get(&self, name: &str) -> Option<String> {
        let srv = self.srv;
        if name.starts_with('@')
            && let Some(v) = srv.user_option(self.session, self.pane, self.window, name)
        {
            return Some(v);
        }
        let session = self.session.and_then(|s| srv.sessions.get(&s));
        let window = self.window.and_then(|w| srv.windows.get(&w));
        let pane = self.pane.and_then(|p| srv.panes.get(&p));
        let att = self
            .client
            .and_then(|c| srv.clients.get(&c))
            .and_then(|c| c.att.as_ref());
        let attached = |sid: SessionId| {
            srv.clients
                .values()
                .filter(|c| c.att.as_ref().is_some_and(|a| a.session == sid))
                .count()
        };
        // The session the marked pane counts as marked in. tmux keeps the
        // one it was marked in (marked_pane.s); tmxr keeps only the pane, so
        // the first session holding its window.
        let marked_session = || {
            let window = srv.panes.get(&srv.marked_pane()?)?.window;
            srv.session_of_window(window)
        };
        Some(match name {
            "host" => srv.host.clone(),
            "host_short" => srv.host.split('.').next().unwrap_or(&srv.host).to_owned(),
            "pid" => srv.pid.to_string(),
            "version" => env!("CARGO_PKG_VERSION").to_owned(),
            "start_time" => epoch(srv.started),
            "config_files" => srv
                .cfg_file
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            "next_session_id" => format!("${}", srv.next_session_id()),
            "socket_path" => srv.endpoint.path().display().to_string(),
            "client_prefix" => flag(att.is_some_and(|a| a.table == "prefix")),
            "client_key_table" => att.map(|a| a.table.clone())?,
            "client_name" => self.client?.to_string(),
            "client_flags" => client_flags(srv.clients.get(&self.client?)?),
            "client_readonly" => flag(srv.clients.get(&self.client?)?.read_only()),
            "client_width" => att?.cols.to_string(),
            // Set by a format loop for its last item.
            "loop_last_flag" => "0".to_owned(),
            "client_created" => epoch(srv.clients.get(&self.client?)?.created),
            // The users the server and the client run as: a uid on Unix, a
            // SID on Windows, and that user's name.
            "uid" => srv.owner.clone(),
            "user" => crate::access::user_name(&srv.owner).ok()?,
            "client_uid" => srv.clients.get(&self.client?)?.user.clone(),
            "client_user" => {
                crate::access::user_name(&srv.clients.get(&self.client?)?.user).ok()?
            }
            "client_pid" => srv
                .clients
                .get(&self.client?)?
                .hello
                .as_ref()?
                .pid
                .to_string(),
            // Its last key or mouse event, as tmux's activity_time.
            "client_activity" => epoch(std::time::SystemTime::now() - att?.last_input.elapsed()),
            "client_last_session" => srv.sessions.get(&att?.last_session?)?.name.clone(),
            "client_height" => att?.rows.to_string(),
            "client_session" => srv.sessions.get(&att?.session)?.name.clone(),
            "client_theme" => match srv.clients.get(&self.client?)?.theme? {
                crossterm::event::ColorScheme::Dark => "dark".to_owned(),
                crossterm::event::ColorScheme::Light => "light".to_owned(),
            },
            "client_termname" => srv
                .clients
                .get(&self.client?)?
                .hello
                .as_ref()?
                .terminal
                .as_ref()?
                .term
                .clone(),
            "client_tty" => srv
                .clients
                .get(&self.client?)?
                .hello
                .as_ref()?
                .terminal
                .as_ref()?
                .tty
                .clone(),
            "session_name" => session?.name.clone(),
            "session_id" => format!("${}", session?.id),
            "session_windows" => session?.windows.len().to_string(),
            "session_attached" => attached(session?.id).to_string(),
            // A session group's, nothing outside one (but session_grouped).
            "session_grouped" => flag(srv.group_of(session?.id).is_some()),
            "session_group" => srv.group_of(session?.id)?.name.clone(),
            "session_group_size" => srv.group_of(session?.id)?.sessions.len().to_string(),
            "session_group_list" => {
                let g = srv.group_of(session?.id)?;
                g.sessions
                    .iter()
                    .filter_map(|s| srv.sessions.get(s))
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            }
            "session_group_attached" | "session_group_many_attached" => {
                let n: usize = srv
                    .group_of(session?.id)?
                    .sessions
                    .iter()
                    .map(|s| attached(*s))
                    .sum();
                if name == "session_group_attached" {
                    n.to_string()
                } else {
                    flag(n > 1)
                }
            }
            "session_group_attached_list" => {
                let g = srv.group_of(session?.id)?;
                client_names(srv, |a| g.sessions.contains(&a.session))
            }
            "session_attached_list" => {
                let sid = session?.id;
                client_names(srv, |a| a.session == sid)
            }
            "session_path" => session?.cwd.display().to_string(),
            // Each kind of alert any of its windows has, once, as tmux's.
            "session_alert" => {
                let mut seen = String::new();
                for (_, marks) in alerts(srv, session?) {
                    for c in marks.chars() {
                        if !seen.contains(c) {
                            seen.push(c);
                        }
                    }
                }
                seen
            }
            // Whether any of the session's windows has that alert for it. tmux
            // 3.6 means the same but looks at one window only (the first
            // for a bell, the format's own for the others), so it can say 0
            // where a later window is alerted.
            "session_activity_flag" | "session_bell_flag" | "session_silence_flag" => {
                let mark = match name {
                    "session_activity_flag" => '#',
                    "session_bell_flag" => '!',
                    _ => '~',
                };
                flag(alerts(srv, session?).iter().any(|(_, m)| m.contains(mark)))
            }
            // The alerted windows' indexes, each with its alerts.
            "session_alerts" => alerts(srv, session?)
                .into_iter()
                .map(|(index, marks)| format!("{index}{marks}"))
                .collect::<Vec<_>>()
                .join(","),
            "session_created" => epoch(session?.created),
            "session_activity" => epoch(session?.activity),
            "session_last_attached" => epoch(session?.last_attached?),
            "session_many_attached" => flag(attached(session?.id) > 1),
            "session_marked" => flag(marked_session() == Some(session?.id)),
            "server_sessions" => srv.sessions.len().to_string(),
            // As tmux's: the current window's index, and the highest.
            "active_window_index" => session?.current.to_string(),
            "last_window_index" => session?.windows.keys().next_back()?.to_string(),
            "window_index" => session?.index_of(window?.id)?.to_string(),
            "window_id" => format!("@{}", window?.id),
            "window_name" => window?.name.clone(),
            "window_panes" => window?.panes().len().to_string(),
            "window_linked" => flag(srv.sessions_of_window(window?.id).len() > 1),
            // When one of its panes last printed, as tmux's activity_time.
            "window_activity" => {
                let ago = window?.last_output.elapsed();
                epoch(std::time::SystemTime::now() - ago)
            }
            "window_start_flag" => {
                flag(session?.windows.keys().next() == session?.index_of(window?.id).as_ref())
            }
            "window_end_flag" => {
                flag(session?.windows.keys().next_back() == session?.index_of(window?.id).as_ref())
            }
            // The sessions holding it, and those showing it as their current
            // window, by name as tmux's lists are.
            "window_linked_sessions" => srv.sessions_of_window(window?.id).len().to_string(),
            "window_linked_sessions_list" => {
                let wid = window?.id;
                session_names(srv, |s| s.windows.values().any(|w| *w == wid))
            }
            "window_active_sessions" => {
                let wid = window?.id;
                srv.sessions
                    .values()
                    .filter(|s| s.current_window() == Some(wid))
                    .count()
                    .to_string()
            }
            "window_active_clients" | "window_active_clients_list" => {
                let wid = window?.id;
                let showing = |a: &Attached| {
                    srv.sessions
                        .get(&a.session)
                        .and_then(|s| s.current_window())
                        == Some(wid)
                };
                if name == "window_active_clients" {
                    srv.clients
                        .values()
                        .filter(|c| c.att.as_ref().is_some_and(showing))
                        .count()
                        .to_string()
                } else {
                    client_names(srv, showing)
                }
            }
            "window_active_sessions_list" => {
                let wid = window?.id;
                session_names(srv, |s| s.current_window() == Some(wid))
            }
            "window_marked_flag" => {
                flag(srv.window_is_marked(window?.id) && marked_session() == Some(session?.id))
            }
            "window_width" => window?.cols.to_string(),
            "window_height" => window?.rows.to_string(),
            "window_zoomed_flag" => flag(window?.zoomed),
            // As tmux's: the whole layout even when zoomed, and what shows.
            "window_layout" => {
                let w = window?;
                crate::layout::dump(&w.layout, w.cols, w.rows)
            }
            "window_visible_layout" => {
                let w = window?;
                if w.zoomed {
                    let only = hjkl_layout::LayoutTree::Leaf(w.active as usize);
                    crate::layout::dump(&only, w.cols, w.rows)
                } else {
                    crate::layout::dump(&w.layout, w.cols, w.rows)
                }
            }
            "window_bell_flag" => flag(window?.bell.contains(&session?.id)),
            "window_activity_flag" => flag(window?.activity.contains(&session?.id)),
            "window_silence_flag" => flag(window?.silence.contains(&session?.id)),
            "window_active" => flag(session?.current_window() == Some(window?.id)),
            "window_last_flag" => {
                let s = session?;
                flag(s.last().is_some() && s.last() == s.index_of(window?.id))
            }
            // tmux's window_printable_flags: escaped for a format drawn
            // after (`##` for activity), and raw.
            "window_flags" | "window_raw_flags" => {
                let s = session?;
                let w = window?;
                let idx = s.index_of(w.id);
                let flags = w.flags(
                    s.id,
                    idx == Some(s.current),
                    idx.is_some() && idx == s.last(),
                    srv.window_is_marked(w.id),
                );
                if name == "window_flags" {
                    flags.replace('#', "##")
                } else {
                    flags
                }
            }
            "pane_synchronized" => flag(srv.pane_options(pane?.id).synchronize_panes),
            // An empty pane has no program, so counts as dead, as tmux's
            // (its fd is -1).
            "pane_dead" => flag(pane?.dead.is_some() || pane?.pty.is_empty()),
            "pane_start_path" => pane?.start_cwd.display().to_string(),
            // Each word quoted as tmux's args_escape, as its
            // cmd_stringify_argv joins them.
            "pane_start_command" => pane?
                .argv
                .iter()
                .map(|w| tmxr_command::escape_arg(w))
                .collect::<Vec<_>>()
                .join(" "),
            "history_limit" => pane?.emu.history_limit().to_string(),
            "cursor_flag" => flag(!pane?.emu.screen().hide_cursor()),
            "keypad_flag" => flag(pane?.emu.screen().application_keypad()),
            "keypad_cursor_flag" => flag(pane?.emu.screen().application_cursor()),
            "pane_input_off" => flag(pane?.input_off),
            "pane_dead_status" => pane?.dead.flatten()?.to_string(),
            "pane_dead_time" => epoch(pane?.dead_time?),
            "pane_dead_signal" => pane?.dead_signal?.to_string(),
            "pane_marked" => flag(srv.marked_pane() == Some(pane?.id)),
            "pane_marked_set" => flag(srv.marked_pane().is_some()),
            "pane_id" => format!("%{}", pane?.id),
            "pane_index" => {
                let w = window?;
                let i = w.panes().iter().position(|p| Some(*p) == self.pane)?;
                (i as u32 + srv.window_options(w.id).pane_base_index).to_string()
            }
            "pane_active" => flag(window?.active == pane?.id),
            "pane_last" => flag(window?.last_pane == Some(pane?.id)),
            "pane_title" => pane?
                .emu
                .title()
                .map_or_else(|| srv.host.clone(), str::to_owned),
            "pane_current_path" => pane_current_path(srv, pane?.id)?.display().to_string(),
            "pane_current_command" => {
                let p = pane?;
                let cached = srv.commands.get(&p.id).cloned();
                let live = || tmxr_term::process::foreground_command(&p.pty);
                // Read when asked, as tmux does, so a program just started is
                // named at once: on Unix that is a system call and a read. On
                // Windows a lookup snapshots every process, so the copy
                // `refresh_commands` keeps is read, up to a refresh old.
                if cfg!(windows) {
                    cached.or_else(live)
                } else {
                    live().or(cached)
                }
                .unwrap_or_default()
            }
            // 0 for an empty pane, as tmux's.
            "pane_pid" => {
                let p = pane?;
                if p.pty.is_empty() {
                    "0".to_owned()
                } else {
                    p.pty.pid()?.to_string()
                }
            }
            // tmux's: whether the format is for a pane, a window or a
            // session (`Kind`).
            "buffer_mode_format" => BUFFER_MODE_FORMAT.to_owned(),
            "client_mode_format" => CLIENT_MODE_FORMAT.to_owned(),
            "tree_mode_format" => TREE_MODE_FORMAT.to_owned(),
            // The current window, then the ones before it, latest first,
            // as tmux's lastw stack.
            "session_stack" => {
                let s = session?;
                std::iter::once(s.current)
                    .chain(s.stack.iter().copied())
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            }
            // Where the window is on that stack from 1, 0 off it.
            "window_stack_index" => {
                let s = session?;
                let idx = s.index_of(window?.id);
                s.stack
                    .iter()
                    .position(|&i| Some(i) == idx)
                    .map_or(0, |p| p + 1)
                    .to_string()
            }
            // tmux's names for the pane's modifyOtherKeys level (what it
            // asked for, or `always` gave it when it started).
            "pane_key_mode" => match pane?.emu.input_modes().modify_other_keys {
                0 => "VT10x",
                1 => "Ext 1",
                _ => "Ext 2",
            }
            .to_owned(),
            "pane_format" => flag(self.kind == Kind::Pane),
            "window_format" => flag(self.kind == Kind::Window),
            "session_format" => flag(self.kind == Kind::Session),
            // The pane's default colours, as tmux's tty_default_colours.
            "pane_fg" | "pane_bg" => {
                let colours = crate::render::pane_default_colours(srv, pane?.id);
                let c = if name == "pane_fg" {
                    colours.fg
                } else {
                    colours.bg
                };
                tmxr_command::format::colour_name(
                    c.unwrap_or(tmxr_command::format::Colour::Default),
                )
            }
            // The command whose formats these are, as tmux's.
            "command" => srv.running?.to_owned(),
            // A cell's size in pixels: tmux's defaults for a terminal that
            // reports none (DEFAULT_XPIXEL / DEFAULT_YPIXEL), as tmxr's
            // client sends no pixel size.
            "window_cell_width" => {
                window?;
                CELL_WIDTH_PIXELS.to_string()
            }
            "window_cell_height" => {
                window?;
                CELL_HEIGHT_PIXELS.to_string()
            }
            // The cursor DECSCUSR set, by tmux's names.
            "cursor_shape" => match pane?.emu.cursor_style() {
                1 | 2 => "block",
                3 | 4 => "underline",
                5 | 6 => "bar",
                _ => "default",
            }
            .to_owned(),
            "cursor_blinking" => flag(pane?.emu.cursor_blinking()),
            // The modes the pane's program set, as tmux's MODE_INSERT and
            // MODE_WRAP.
            "origin_flag" => flag(pane?.emu.origin_mode()),
            "pane_tabs" => {
                let stops: Vec<String> = pane?.emu.tab_stops().map(|c| c.to_string()).collect();
                stops.join(",")
            }
            "scroll_region_upper" => pane?.emu.scroll_region().0.to_string(),
            "scroll_region_lower" => pane?.emu.scroll_region().1.to_string(),
            "insert_flag" => flag(pane?.emu.insert_and_wrap().0),
            "wrap_flag" => flag(pane?.emu.insert_and_wrap().1),
            // tmxr draws no sixel images.
            "sixel_support" => "0".to_owned(),
            "pane_unseen_changes" => flag(pane?.copy.as_ref().is_some_and(|c| c.unseen_changes)),
            "pane_search_string" => pane?
                .last_search
                .as_ref()
                .map_or_else(String::new, |s| s.needle.clone()),
            "pane_tty" => pane?
                .pty
                .tty_name()
                .map(|t| t.display().to_string())
                .unwrap_or_default(),
            "copy_cursor_word" => pane?.copy.as_ref()?.cursor_word(),
            "copy_cursor_line" => pane?.copy.as_ref()?.cursor_line(),
            "copy_cursor_hyperlink" => {
                let pane = pane?;
                pane.copy
                    .as_ref()?
                    .cursor_link()
                    .and_then(|id| pane.emu.screen().hyperlink(id))
                    .map(|h| h.uri.clone())
                    .unwrap_or_default()
            }
            "rectangle_toggle" => flag(pane?.copy.as_ref()?.rectangle()),
            "top_line_time" => pane?.copy.as_ref()?.top_line_time().to_string(),
            // A search's marks, its match count and the match at the
            // cursor, as tmux's window_copy_formats; tmxr's searches never
            // stop short, so never time out or count only part.
            "search_present" => flag(pane?.copy.as_ref()?.marks),
            "search_count" => pane?.copy.as_ref()?.search_count.to_string(),
            "search_count_partial" | "search_timed_out" => {
                pane?.copy.as_ref()?;
                "0".to_owned()
            }
            "search_match" => pane?.copy.as_ref()?.match_at_cursor().unwrap_or_default(),
            // tmux's top paste buffer's, as format_defaults adds them.
            "buffer_created" | "buffer_full" | "buffer_name" | "buffer_sample" | "buffer_size" => {
                srv.buffers.front()?.var(name)?
            }
            // tmux's: the column, and the row in the view.
            "copy_cursor_x" => pane?.copy.as_ref()?.cx.to_string(),
            "copy_cursor_y" => {
                let cm = pane?.copy.as_ref()?;
                cm.cy.saturating_sub(cm.top).to_string()
            }
            "pane_mode" => {
                if pane?.copy.is_some() {
                    "copy-mode".to_owned()
                } else {
                    String::new()
                }
            }
            "alternate_on" => flag(pane?.emu.screen().alternate_screen()),
            // The program's, else `cursor-colour` as set.
            "cursor_colour" => match pane?.emu.cursor_colour() {
                Some(rgb) => format!("#{rgb:06x}"),
                None => srv.pane_options(pane?.id).cursor_colour.to_string(),
            },
            "cursor_very_visible" => flag(pane?.emu.cursor_very_visible()),
            // tmux's saved_cx / saved_cy, UINT_MAX until a switch sets them.
            "alternate_saved_x" | "alternate_saved_y" => {
                let saved = pane?.emu.alternate_saved();
                let (row, col) =
                    saved.map_or((u32::MAX, u32::MAX), |(r, c)| (u32::from(r), u32::from(c)));
                if name.ends_with('x') { col } else { row }.to_string()
            }
            // tmux's: how far the copy-mode view is above the bottom, and
            // whether something is selected; only in copy mode.
            "scroll_position" => pane?.copy.as_ref()?.position().0.to_string(),
            // As tmux's window_copy_formats: where the selection began (its
            // line counted from the history's oldest) and its other end, the
            // cursor until stop-selection fixes it; active while it follows
            // the cursor, present while its ends differ.
            "selection_present" => {
                let bounds = pane?.copy.as_ref()?.selection_bounds();
                flag(bounds.is_some_and(|(start, end)| start != end))
            }
            "selection_active" => {
                let cm = pane?.copy.as_ref()?;
                flag(cm.anchor.is_some() && cm.sel_end.is_none())
            }
            "selection_start_x" => pane?.copy.as_ref()?.selection_bounds()?.0.1.to_string(),
            "selection_start_y" => pane?.copy.as_ref()?.selection_bounds()?.0.0.to_string(),
            "selection_end_x" => pane?.copy.as_ref()?.selection_bounds()?.1.1.to_string(),
            "selection_end_y" => pane?.copy.as_ref()?.selection_bounds()?.1.0.to_string(),
            "pane_in_mode" => flag(pane?.copy.is_some()),
            "history_size" => pane?.emu.history_size().to_string(),
            "pane_pipe" => flag(pane?.pipe.is_some()),
            "mouse_any_flag" => {
                flag(pane?.emu.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None)
            }
            // The mouse mode the pane's program set: 1000, 1002, 1003.
            "mouse_standard_flag" | "mouse_button_flag" | "mouse_all_flag" => {
                use vt100::MouseProtocolMode::{AnyMotion, ButtonMotion, PressRelease};
                let mode = pane?.emu.screen().mouse_protocol_mode();
                flag(match name {
                    "mouse_standard_flag" => mode == PressRelease,
                    "mouse_button_flag" => mode == ButtonMotion,
                    _ => mode == AnyMotion,
                })
            }
            "mouse_utf8_flag" => flag(pane?.emu.mouse_utf8()),
            "mouse_sgr_flag" => flag(
                pane?.emu.screen().mouse_protocol_encoding() == vt100::MouseProtocolEncoding::Sgr,
            ),
            "pane_width" => pane?.rect.w.to_string(),
            "pane_height" => pane?.rect.h.to_string(),
            // tmux's: the pane's cells in its window, edges inclusive.
            "pane_left" => pane?.rect.x.to_string(),
            "pane_top" => pane?.rect.y.to_string(),
            "pane_right" => (pane?.rect.x + pane?.rect.w).saturating_sub(1).to_string(),
            "pane_bottom" => (pane?.rect.y + pane?.rect.h).saturating_sub(1).to_string(),
            "pane_at_left" => flag(pane?.rect.x == 0),
            "pane_at_top" => flag(pane?.rect.y == 0),
            "pane_at_right" => flag(pane?.rect.x + pane?.rect.w >= window?.cols),
            "pane_at_bottom" => flag(pane?.rect.y + pane?.rect.h >= window?.rows),
            "cursor_x" => pane?.emu.screen().cursor_position().1.to_string(),
            "cursor_y" => pane?.emu.screen().cursor_position().0.to_string(),
            // The character under the cursor: a space for an empty cell, and
            // nothing for the second half of a wide one, as tmux's.
            "cursor_character" => {
                let screen = pane?.emu.screen();
                let (row, col) = screen.cursor_position();
                let cell = screen.cell(row, col)?;
                if cell.is_wide_continuation() {
                    String::new()
                } else if cell.has_contents() {
                    cell.contents().to_owned()
                } else {
                    " ".to_owned()
                }
            }
            _ => return self.option(name),
        })
    }
}

impl Vars<'_> {
    /// An option's value by name, as tmux's format_find reads one: the
    /// pane's (with its window's), the window's, the session's, then the
    /// server's.
    fn option(&self, name: &str) -> Option<String> {
        let srv = self.srv;
        let window = self
            .pane
            .map(|p| srv.pane_options(p).show(name))
            .or_else(|| self.window.map(|w| srv.window_options(w).show(name)))
            .unwrap_or_else(|| srv.cfg.window.show(name));
        let session = || {
            self.session.map_or_else(
                || srv.cfg.session.show(name),
                |s| srv.session_options(s).show(name),
            )
        };
        window
            .or_else(session)
            .or_else(|| crate::cmds::server_option(srv, name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names `get` matches on, read from this file.
    fn matched_names() -> std::collections::BTreeSet<&'static str> {
        let src = include_str!("vars.rs");
        let start = src.find("    fn get(&self, name: &str)").expect("get");
        let body = &src[start..];
        let body = &body[..body.find("\n    }\n}").expect("end of get")];
        let arm = regex::Regex::new(r#"^\s*((?:"[a-z0-9_]+"\s*\|\s*)*"[a-z0-9_]+")\s*=>"#)
            .expect("arm pattern");
        let name = regex::Regex::new(r#""([a-z0-9_]+)""#).expect("name pattern");
        body.lines()
            .filter_map(|line| arm.captures(line))
            .flat_map(|c| {
                name.captures_iter(c.get(1).map_or("", |m| m.as_str()))
                    .filter_map(|n| n.get(1).map(|m| m.as_str()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn names_lists_every_variable_in_order() {
        let listed: std::collections::BTreeSet<_> = NAMES.iter().copied().collect();
        assert_eq!(listed, matched_names());
        assert!(NAMES.windows(2).all(|w| w[0] < w[1]), "sorted, once each");
    }
}
