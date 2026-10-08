//! Mouse handling for an attached client (`mouse on`).
//!
//! - A pane whose program asked for the mouse gets the event, re-encoded
//!   relative to the pane.
//! - Otherwise: a click focuses the pane, a drag on a border resizes, a drag
//!   inside a pane selects in copy mode and copies on release (tmux-yank's
//!   `MouseDragEnd1Pane`), the wheel scrolls through copy mode, and a click on
//!   a window in the status line selects it.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::cmds::Ctx;
use crate::layout::Dir;
use crate::model::{ClientId, PaneId};
use crate::server::{STATUS_ROWS, Server};

/// An in-progress drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    /// Moving the border to the right of / below `pane`.
    Border {
        pane: PaneId,
        vertical: bool,
        at: u16,
    },
    /// Selecting text in `pane` in copy mode.
    Select { pane: PaneId },
}

/// Lines the wheel scrolls per notch, as tmux's default binds.
const WHEEL_LINES: usize = 5;

pub fn handle(srv: &mut Server, id: ClientId, m: MouseEvent) {
    if !srv.cfg.mouse {
        return;
    }
    let Some(att) = srv.clients.get(&id).and_then(|c| c.att.as_ref()) else {
        return;
    };
    let session = att.session;
    let rows = att.rows;
    let drag = att.drag;
    if m.row >= rows.saturating_sub(STATUS_ROWS) {
        if let MouseEventKind::Down(MouseButton::Left) = m.kind {
            let hit = att
                .status_ranges
                .iter()
                .find(|(a, b, _)| m.column >= *a && m.column < *b)
                .map(|(_, _, idx)| *idx);
            if let Some(idx) = hit {
                let _ = srv.select_window(session, idx);
            }
        }
        return;
    }
    let Some(wid) = srv.sessions.get(&session).and_then(|s| s.current_window()) else {
        return;
    };
    let Some(win) = srv.windows.get(&wid) else {
        return;
    };
    let rects = win.visible_rects();
    let hit = rects
        .iter()
        .find(|(_, r)| m.column >= r.x && m.column < r.x + r.w && m.row >= r.y && m.row < r.y + r.h)
        .map(|(p, r)| (*p, *r));

    // Continue a drag first: it owns the mouse until the button is released.
    if let Some(d) = drag {
        match (d, m.kind) {
            (Drag::Border { pane, vertical, at }, MouseEventKind::Drag(MouseButton::Left)) => {
                let now = if vertical { m.column } else { m.row };
                if now != at {
                    let (dir, cells) = match (vertical, now > at) {
                        (true, true) => (Dir::Right, now - at),
                        (true, false) => (Dir::Left, at - now),
                        (false, true) => (Dir::Down, now - at),
                        (false, false) => (Dir::Up, at - now),
                    };
                    if let Some(w) = srv.windows.get_mut(&wid) {
                        let (c, r) = (w.cols, w.rows);
                        crate::layout::resize(&mut w.layout, pane, dir, cells, c, r);
                    }
                    srv.relayout(wid);
                    set_drag(
                        srv,
                        id,
                        Some(Drag::Border {
                            pane,
                            vertical,
                            at: now,
                        }),
                    );
                }
                return;
            }
            (Drag::Select { pane }, MouseEventKind::Drag(MouseButton::Left)) => {
                move_copy_cursor(srv, pane, m.column, m.row);
                return;
            }
            (Drag::Select { pane }, MouseEventKind::Up(MouseButton::Left)) => {
                set_drag(srv, id, None);
                let ctx = Ctx {
                    client: Some(id),
                    pane: Some(pane),
                    ..Ctx::default()
                };
                let _ = crate::copy::command(srv, &ctx, pane, "copy-selection-and-cancel", &[]);
                return;
            }
            (_, MouseEventKind::Up(_)) => {
                set_drag(srv, id, None);
                return;
            }
            _ => {}
        }
    }

    let Some((pane, rect)) = hit else {
        // On a border: start a resize drag.
        if let MouseEventKind::Down(MouseButton::Left) = m.kind
            && let Some(d) = border_at(&rects, m.column, m.row)
        {
            set_drag(srv, id, Some(d));
        }
        return;
    };

    let in_copy = srv.panes.get(&pane).is_some_and(|p| p.copy.is_some());
    let wants_mouse = srv
        .panes
        .get(&pane)
        .is_some_and(|p| p.emu.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None);

    if wants_mouse && !in_copy {
        if let MouseEventKind::Down(_) = m.kind {
            srv.select_pane(pane);
        }
        if let Some(p) = srv.panes.get_mut(&pane) {
            let s = p.emu.screen();
            let bytes = tmxr_term::encode::encode_mouse(
                &m,
                m.column - rect.x,
                m.row - rect.y,
                s.mouse_protocol_mode(),
                s.mouse_protocol_encoding(),
            );
            if !bytes.is_empty() {
                let _ = p.pty.write(&bytes);
            }
        }
        return;
    }

    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            srv.select_pane(pane);
            if in_copy {
                move_copy_cursor(srv, pane, m.column, m.row);
                if let Some(cm) = srv.panes.get_mut(&pane).and_then(|p| p.copy.as_mut()) {
                    cm.anchor = None;
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            srv.select_pane(pane);
            crate::copy::enter(srv, pane, false);
            move_copy_cursor(srv, pane, m.column, m.row);
            if let Some(cm) = srv.panes.get_mut(&pane).and_then(|p| p.copy.as_mut()) {
                cm.apply("begin-selection", None);
            }
            set_drag(srv, id, Some(Drag::Select { pane }));
        }
        MouseEventKind::ScrollUp => {
            crate::copy::enter(srv, pane, false);
            scroll(srv, pane, "scroll-up");
        }
        MouseEventKind::ScrollDown if in_copy => {
            scroll(srv, pane, "scroll-down");
            // Scrolling back to the bottom leaves copy mode, as tmux's wheel
            // binds do with `copy-mode -e`.
            let at_bottom = srv
                .panes
                .get(&pane)
                .and_then(|p| p.copy.as_ref())
                .is_some_and(|c| c.position().0 == 0 && c.anchor.is_none());
            if at_bottom {
                let ctx = Ctx::default();
                let _ = crate::copy::command(srv, &ctx, pane, "cancel", &[]);
            }
        }
        _ => {}
    }
}

fn set_drag(srv: &mut Server, id: ClientId, drag: Option<Drag>) {
    if let Some(a) = srv.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
        a.drag = drag;
    }
}

fn scroll(srv: &mut Server, pane: PaneId, cmd: &str) {
    if let Some(cm) = srv.panes.get_mut(&pane).and_then(|p| p.copy.as_mut()) {
        for _ in 0..WHEEL_LINES {
            cm.apply(cmd, None);
        }
    }
    if let Some(w) = srv.panes.get(&pane).map(|p| p.window) {
        srv.mark_window_dirty(w);
    }
}

fn move_copy_cursor(srv: &mut Server, pane: PaneId, col: u16, row: u16) {
    let Some(p) = srv.panes.get_mut(&pane) else {
        return;
    };
    let r = p.rect;
    if let Some(cm) = p.copy.as_mut() {
        let y = cm.top + usize::from(row.saturating_sub(r.y).min(r.h.saturating_sub(1)));
        cm.cy = y.min(cm.lines.len().saturating_sub(1));
        cm.cx = usize::from(col.saturating_sub(r.x).min(r.w.saturating_sub(1)));
    }
    let w = p.window;
    srv.mark_window_dirty(w);
}

/// The border cell at (`col`, `row`): which pane it belongs to (the one left
/// of / above it) and its orientation.
fn border_at(rects: &[(PaneId, hjkl_layout::LayoutRect)], col: u16, row: u16) -> Option<Drag> {
    for (p, r) in rects {
        if r.x + r.w == col && row >= r.y && row < r.y + r.h {
            return Some(Drag::Border {
                pane: *p,
                vertical: true,
                at: col,
            });
        }
        if r.y + r.h == row && col >= r.x && col < r.x + r.w {
            return Some(Drag::Border {
                pane: *p,
                vertical: false,
                at: row,
            });
        }
    }
    None
}
