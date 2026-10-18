//! Format variables (`#{session_name}`, `#{pane_current_path}`, …).

use std::path::PathBuf;

use tmxr_command::format::Context;

use crate::model::{ClientId, PaneId, SessionId, WindowId};
use crate::server::{Attached, Server};

/// Variables for one (session, window, pane, client) combination.
pub struct Vars<'a> {
    pub srv: &'a Server,
    pub session: Option<SessionId>,
    pub window: Option<WindowId>,
    pub pane: Option<PaneId>,
    pub client: Option<ClientId>,
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
/// `mouse_hyperlink` is empty: tmxr keeps no OSC 8 links.
pub fn mouse_var(srv: &Server, mouse: &crate::mouse::MouseTarget, name: &str) -> Option<String> {
    if !name.starts_with("mouse_") || name == "mouse_any_flag" {
        return None;
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
        "mouse_hyperlink" => String::new(),
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
/// tmxr keeps, in its order.
fn client_flags(c: &crate::server::Client) -> String {
    let att = c.att.as_ref();
    [
        (att.is_some(), "attached"),
        (att.is_some_and(|a| a.focused), "focused"),
        (c.ignore_size, "ignore-size"),
        (c.read_only(), "read-only"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect::<Vec<_>>()
    .join(",")
}

impl Context for Vars<'_> {
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
            "socket_path" => srv.endpoint.path().display().to_string(),
            "client_prefix" => flag(att.is_some_and(|a| a.table == "prefix")),
            "client_key_table" => att.map(|a| a.table.clone())?,
            "client_name" => self.client?.to_string(),
            "client_flags" => client_flags(srv.clients.get(&self.client?)?),
            "client_readonly" => flag(srv.clients.get(&self.client?)?.read_only()),
            "client_width" => att?.cols.to_string(),
            "client_created" => epoch(srv.clients.get(&self.client?)?.created),
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
            "client_termname" => srv
                .clients
                .get(&self.client?)?
                .hello
                .as_ref()?
                .terminal
                .as_ref()?
                .term
                .clone(),
            "session_name" => session?.name.clone(),
            "session_id" => format!("${}", session?.id),
            "session_windows" => session?.windows.len().to_string(),
            "session_attached" => attached(session?.id).to_string(),
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
                flag(s.last.is_some() && s.last == s.index_of(window?.id))
            }
            "window_flags" => {
                let s = session?;
                let w = window?;
                let idx = s.index_of(w.id);
                w.flags(
                    s.id,
                    idx == Some(s.current),
                    idx.is_some() && idx == s.last,
                    srv.window_is_marked(w.id),
                )
            }
            "pane_synchronized" => flag(srv.pane_options(pane?.id).synchronize_panes),
            "pane_dead" => flag(pane?.dead.is_some()),
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
                srv.commands
                    .get(&p.id)
                    .cloned()
                    .or_else(|| tmxr_term::process::foreground_command(&p.pty))
                    .unwrap_or_default()
            }
            "pane_pid" => pane?.pty.pid()?.to_string(),
            "copy_cursor_word" => pane?.copy.as_ref()?.cursor_word(),
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
            // tmux's: how far the copy-mode view is above the bottom, and
            // whether something is selected; only in copy mode.
            "scroll_position" => pane?.copy.as_ref()?.position().0.to_string(),
            "selection_present" => flag(pane?.copy.as_ref()?.anchor.is_some()),
            "pane_in_mode" => flag(pane?.copy.is_some()),
            "history_size" => pane?.emu.history_size().to_string(),
            "pane_pipe" => flag(pane?.pipe.is_some()),
            "mouse_any_flag" => {
                flag(pane?.emu.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None)
            }
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
            _ => return None,
        })
    }
}
