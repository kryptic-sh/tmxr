//! Format variables (`#{session_name}`, `#{pane_current_path}`, …).

use std::path::PathBuf;

use tmxr_command::format::Context;

use crate::model::{ClientId, PaneId, SessionId, WindowId};
use crate::server::Server;

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
        "mouse_word" => crate::copy::word_at(&line(), usize::from(x)),
        "mouse_line" => String::from_iter(line()).trim_end().to_owned(),
        "mouse_hyperlink" => String::new(),
        _ => return None,
    })
}

fn flag(b: bool) -> String {
    if b { "1" } else { "0" }.to_owned()
}

impl Context for Vars<'_> {
    fn get(&self, name: &str) -> Option<String> {
        let srv = self.srv;
        if let Some(v) = srv.cfg.options.get(name) {
            return Some(v.clone());
        }
        let session = self.session.and_then(|s| srv.sessions.get(&s));
        let window = self.window.and_then(|w| srv.windows.get(&w));
        let pane = self.pane.and_then(|p| srv.panes.get(&p));
        let att = self
            .client
            .and_then(|c| srv.clients.get(&c))
            .and_then(|c| c.att.as_ref());
        Some(match name {
            "host" => srv.host.clone(),
            "host_short" => srv.host.split('.').next().unwrap_or(&srv.host).to_owned(),
            "pid" => srv.pid.to_string(),
            "version" => env!("CARGO_PKG_VERSION").to_owned(),
            "socket_path" => srv.endpoint.path().display().to_string(),
            "client_prefix" => flag(att.is_some_and(|a| a.table == "prefix")),
            "client_key_table" => att.map(|a| a.table.clone())?,
            "client_name" => self.client?.to_string(),
            "client_width" => att?.cols.to_string(),
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
            "session_attached" => {
                let sid = session?.id;
                srv.clients
                    .values()
                    .filter(|c| c.att.as_ref().is_some_and(|a| a.session == sid))
                    .count()
                    .to_string()
            }
            "window_index" => session?.index_of(window?.id)?.to_string(),
            "window_id" => format!("@{}", window?.id),
            "window_name" => window?.name.clone(),
            "window_panes" => window?.panes().len().to_string(),
            "window_linked" => flag(srv.sessions_of_window(window?.id).len() > 1),
            "window_width" => window?.cols.to_string(),
            "window_height" => window?.rows.to_string(),
            "window_zoomed_flag" => flag(window?.zoomed),
            "window_bell_flag" => flag(window?.bell),
            "window_activity_flag" => flag(window?.activity),
            "window_silence_flag" => flag(window?.silence),
            "window_active" => flag(session?.current_window() == Some(window?.id)),
            "window_flags" => {
                let s = session?;
                let w = window?;
                let idx = s.index_of(w.id);
                w.flags(
                    idx == Some(s.current),
                    idx.is_some() && idx == s.last,
                    srv.window_is_marked(w.id),
                )
            }
            "pane_synchronized" => flag(window?.synchronize),
            "pane_dead" => flag(pane?.dead.is_some()),
            "pane_dead_status" => pane?.dead.flatten()?.to_string(),
            "pane_marked" => flag(srv.marked_pane() == Some(pane?.id)),
            "pane_marked_set" => flag(srv.marked_pane().is_some()),
            "pane_id" => format!("%{}", pane?.id),
            "pane_index" => {
                let w = window?;
                let i = w.panes().iter().position(|p| Some(*p) == self.pane)?;
                (i as u32 + srv.cfg.pane_base_index).to_string()
            }
            "pane_active" => flag(window?.active == pane?.id),
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
