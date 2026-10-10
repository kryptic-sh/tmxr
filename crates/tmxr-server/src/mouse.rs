//! Mouse handling for an attached client (`mouse on`), as tmux does it.
//!
//! Each event becomes a mouse key (`MouseDown1Pane`, `WheelUpPane`,
//! `MouseDragEnd1Pane`, …), looked up in the root table, or in the copy-mode
//! table for a pane in copy mode. Its commands run with the mouse as their
//! target: `-t =`, `send-keys -M` (pass the event to the pane's program),
//! `copy-mode -M` (select by dragging) and `resize-pane -M` (drag a border).
//! A pane event no bind claims goes to the pane's program, when it asked for
//! the mouse. The default binds in `defaults.toml` give tmux's behaviour.
//!
//! A drag those commands start then follows the mouse until the button is
//! released.

use std::time::{Duration, Instant};

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use tmxr_command::{MouseAction, MouseKey, MouseLocation};

use crate::cmds::Ctx;
use crate::layout::Dir;
use crate::model::{ClientId, PaneId, SessionId, WindowId};
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

/// A held mouse button: where it went down, and whether it has moved since,
/// which turns its release into `MouseDragEnd` and locates the drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Press {
    pub col: u16,
    pub row: u16,
    pub dragged: bool,
}

/// A press, and how many presses in a row it ends: a press in the same
/// cell with the same button within [`CLICK_TIME`] of the last counts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Click {
    pub at: Instant,
    pub col: u16,
    pub row: u16,
    pub button: u8,
    pub count: u8,
    /// After a second click: where the `DoubleClick` goes, sent once
    /// `CLICK_TIME` passes with no third click.
    pub double: Option<MouseTarget>,
}

/// How soon a press must follow the last to count as a double or triple
/// click (tmux's fixed `KEYC_CLICK_TIMEOUT`).
const CLICK_TIME: Duration = Duration::from_millis(300);

/// What a mouse bind acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseTarget {
    pub event: MouseEvent,
    pub location: MouseLocation,
    /// Where the key happened: the event, or for a drag the press that
    /// started it.
    pub col: u16,
    pub row: u16,
    pub session: SessionId,
    /// The window under the mouse: its index and id.
    pub window: Option<(u32, WindowId)>,
    /// The pane under the mouse, or the one left of / above a border.
    pub pane: Option<PaneId>,
    /// The border under the mouse, as `resize-pane -M` would drag it.
    pub border: Option<Drag>,
}

const MODS: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SHIFT);

pub fn handle(srv: &mut Server, id: ClientId, m: MouseEvent) {
    if !srv.cfg.mouse {
        return;
    }
    let Some(att) = srv.clients.get(&id).and_then(|c| c.att.as_ref()) else {
        return;
    };
    let press = att.press;
    if follow_drag(srv, id, m) {
        return;
    }
    let here = (m.column, m.row);
    let (action, at) = match m.kind {
        MouseEventKind::Down(b) => {
            set_press(
                srv,
                id,
                Some(Press {
                    col: m.column,
                    row: m.row,
                    dragged: false,
                }),
            );
            let n = button(b);
            let action = match count_click(srv, id, m, n) {
                2 => MouseAction::SecondClick(n),
                3 => MouseAction::TripleClick(n),
                _ => MouseAction::Down(n),
            };
            (action, here)
        }
        MouseEventKind::Up(b) => {
            set_press(srv, id, None);
            match press {
                Some(p) if p.dragged => (MouseAction::DragEnd(button(b)), (p.col, p.row)),
                _ => (MouseAction::Up(button(b)), here),
            }
        }
        MouseEventKind::Drag(b) => {
            let p = press.unwrap_or(Press {
                col: m.column,
                row: m.row,
                dragged: false,
            });
            set_press(srv, id, Some(Press { dragged: true, ..p }));
            (MouseAction::Drag(button(b)), (p.col, p.row))
        }
        MouseEventKind::ScrollUp => (MouseAction::WheelUp, here),
        MouseEventKind::ScrollDown => (MouseAction::WheelDown, here),
        _ => return,
    };
    let Some(target) = locate(srv, id, m, at) else {
        return;
    };
    if matches!(action, MouseAction::SecondClick(_))
        && let Some(click) = srv
            .clients
            .get_mut(&id)
            .and_then(|c| c.att.as_mut())
            .and_then(|a| a.click.as_mut())
    {
        click.double = Some(target);
    }
    dispatch(srv, id, action, target);
    // A drag a bind just started takes this event as its first motion.
    if matches!(action, MouseAction::Drag(_)) {
        follow_drag(srv, id, m);
    }
}

/// Record a press of `button` and return how many presses in a row it
/// makes: 1, 2 (a double click) or 3; a fourth starts over.
fn count_click(srv: &mut Server, id: ClientId, m: MouseEvent, button: u8) -> u8 {
    let Some(att) = srv.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
        return 1;
    };
    let now = Instant::now();
    let count = match att.click {
        Some(c)
            if c.button == button
                && (c.col, c.row) == (m.column, m.row)
                && now.duration_since(c.at) <= CLICK_TIME
                && c.count < 3 =>
        {
            c.count + 1
        }
        _ => 1,
    };
    att.click = Some(Click {
        at: now,
        col: m.column,
        row: m.row,
        button,
        count,
        double: None,
    });
    count
}

/// When the next pending `DoubleClick` is due, for the server loop to wake.
pub fn next_double_click(srv: &Server) -> Option<Instant> {
    srv.clients
        .values()
        .filter_map(|c| c.att.as_ref()?.click.as_ref())
        .filter(|c| c.double.is_some())
        .map(|c| c.at + CLICK_TIME)
        .min()
}

/// Send each `DoubleClick` whose second click no third followed in time,
/// as tmux's click timer does.
pub fn fire_double_clicks(srv: &mut Server) {
    let now = Instant::now();
    let due: Vec<(ClientId, u8, MouseTarget)> = srv
        .clients
        .values_mut()
        .filter_map(|c| {
            let click = c.att.as_mut()?.click.as_mut()?;
            if now.duration_since(click.at) < CLICK_TIME {
                return None;
            }
            Some((c.id, click.button, click.double.take()?))
        })
        .collect();
    for (id, b, target) in due {
        dispatch(srv, id, MouseAction::DoubleClick(b), target);
    }
}

/// tmux's button numbers: 1 left, 2 middle, 3 right.
fn button(b: MouseButton) -> u8 {
    match b {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
    }
}

/// Run the bind for `action` at `target`, or pass a pane event nothing binds
/// to the pane's program.
fn dispatch(srv: &mut Server, id: ClientId, action: MouseAction, target: MouseTarget) {
    let in_copy = target
        .pane
        .and_then(|p| srv.panes.get(&p))
        .is_some_and(|p| p.copy.is_some());
    let on_pane = target.location == MouseLocation::Pane;
    let table = if on_pane && in_copy {
        srv.copy_table()
    } else {
        "root"
    };
    let key = MouseKey {
        action,
        location: target.location,
        mods: target.event.modifiers & MODS,
    };
    let ctx = Ctx {
        client: Some(id),
        pane: target
            .pane
            .or_else(|| srv.active_pane_of_session(target.session)),
        mouse: Some(target),
        ..Ctx::default()
    };
    match srv.keys.get(table, &key.into()).cloned() {
        Some(b) => srv.run_bind_ctx(id, &ctx, &b.cmd),
        None if on_pane && !in_copy && !srv.client_read_only(id) => {
            let _ = forward(srv, &ctx);
        }
        None => {}
    }
}

/// Where the mouse is, as a bind sees it; `None` off every pane, border and
/// status line.
fn locate(
    srv: &Server,
    id: ClientId,
    event: MouseEvent,
    (col, row): (u16, u16),
) -> Option<MouseTarget> {
    let att = srv.clients.get(&id)?.att.as_ref()?;
    let session = att.session;
    let mut t = MouseTarget {
        event,
        location: MouseLocation::Status,
        col,
        row,
        session,
        window: None,
        pane: None,
        border: None,
    };
    let sess = srv.sessions.get(&session)?;
    if row >= att.rows.saturating_sub(STATUS_ROWS) {
        let (location, idx) = att.status_ranges.at(col);
        t.location = location;
        t.window = idx.and_then(|i| Some((i, *sess.windows.get(&i)?)));
        return Some(t);
    }
    let wid = sess.current_window()?;
    t.window = Some((sess.current, wid));
    // In window cells: the view may be panned across a larger window.
    let (ox, oy) = crate::render::window_offset(srv, att);
    let (col, row) = (col + ox, row + oy);
    (t.col, t.row) = (col, row);
    let rects = srv.windows.get(&wid)?.visible_rects();
    if let Some((pane, _)) = rects
        .iter()
        .find(|(_, r)| col >= r.x && col < r.x + r.w && row >= r.y && row < r.y + r.h)
    {
        t.location = MouseLocation::Pane;
        t.pane = Some(*pane);
        return Some(t);
    }
    let border = border_at(&rects, col, row)?;
    t.location = MouseLocation::Border;
    t.border = Some(border);
    if let Drag::Border { pane, .. } = border {
        t.pane = Some(pane);
    }
    Some(t)
}

/// Move an in-progress drag with the mouse, ending it when the button is
/// released. Whether the event belonged to a drag.
fn follow_drag(srv: &mut Server, id: ClientId, m: MouseEvent) -> bool {
    let Some(drag) = srv
        .clients
        .get(&id)
        .and_then(|c| c.att.as_ref())
        .and_then(|a| a.drag)
    else {
        return false;
    };
    match (drag, m.kind) {
        (Drag::Border { pane, vertical, at }, MouseEventKind::Drag(MouseButton::Left)) => {
            let now = if vertical { m.column } else { m.row };
            if now != at {
                let (dir, cells) = match (vertical, now > at) {
                    (true, true) => (Dir::Right, now - at),
                    (true, false) => (Dir::Left, at - now),
                    (false, true) => (Dir::Down, now - at),
                    (false, false) => (Dir::Up, at - now),
                };
                if let Some(wid) = srv.panes.get(&pane).map(|p| p.window) {
                    if let Some(w) = srv.windows.get_mut(&wid) {
                        let (c, r) = (w.cols, w.rows);
                        crate::layout::resize(&mut w.layout, pane, dir, cells, c, r);
                    }
                    srv.relayout(wid);
                }
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
        }
        (Drag::Select { pane }, MouseEventKind::Drag(MouseButton::Left)) => {
            move_copy_cursor(srv, pane, m.column, m.row);
        }
        (Drag::Select { pane }, MouseEventKind::Up(MouseButton::Left)) => {
            set_drag(srv, id, None);
            set_press(srv, id, None);
            // The copy table's MouseDragEnd1Pane decides what the selection
            // is for (tmux-yank: copy it and leave copy mode).
            let session = srv
                .clients
                .get(&id)
                .and_then(|c| c.att.as_ref())
                .map(|a| a.session);
            let window = srv.panes.get(&pane).map(|p| p.window);
            if let (Some(session), Some(window)) = (session, window) {
                let index = srv.sessions.get(&session).and_then(|s| s.index_of(window));
                let target = MouseTarget {
                    event: m,
                    location: MouseLocation::Pane,
                    col: m.column,
                    row: m.row,
                    session,
                    window: index.map(|i| (i, window)),
                    pane: Some(pane),
                    border: None,
                };
                dispatch(srv, id, MouseAction::DragEnd(1), target);
            }
        }
        (_, MouseEventKind::Up(_)) => {
            set_drag(srv, id, None);
            set_press(srv, id, None);
        }
        // Another button or the wheel during a drag: handled as usual.
        _ => return false,
    }
    true
}

/// `send-keys -M`: pass the bind's mouse event to the program in the pane
/// under it, in the encoding that program asked for (nothing if it did not).
pub fn forward(srv: &mut Server, ctx: &Ctx) -> Result<(), String> {
    let t = ctx.mouse.ok_or("send-keys -M needs a mouse event")?;
    let pane = t.pane.ok_or("no pane under the mouse")?;
    let p = srv.panes.get_mut(&pane).ok_or("no pane under the mouse")?;
    let r = p.rect;
    let col = t
        .event
        .column
        .saturating_sub(r.x)
        .min(r.w.saturating_sub(1));
    let row = t.event.row.saturating_sub(r.y).min(r.h.saturating_sub(1));
    let s = p.emu.screen();
    let bytes = tmxr_term::encode::encode_mouse(
        &t.event,
        col,
        row,
        s.mouse_protocol_mode(),
        s.mouse_protocol_encoding(),
    );
    if !bytes.is_empty() {
        let _ = p.pty.write(&bytes);
    }
    Ok(())
}

/// `copy-mode -M`: enter copy mode in the pane under the mouse and select
/// from where the drag started.
pub fn copy_mode_drag(srv: &mut Server, ctx: &Ctx) -> Result<(), String> {
    let t = ctx.mouse.ok_or("copy-mode -M needs a mouse event")?;
    let pane = t.pane.ok_or("no pane under the mouse")?;
    crate::copy::enter(srv, pane, false);
    start_selection(srv, ctx, pane, t);
    Ok(())
}

/// `resize-pane -M`: drag the border under the mouse.
pub fn resize_drag(srv: &mut Server, ctx: &Ctx) -> Result<(), String> {
    let t = ctx.mouse.ok_or("resize-pane -M needs a mouse event")?;
    let border = t.border.ok_or("not on a border")?;
    let id = ctx.client.ok_or("no client")?;
    set_drag(srv, id, Some(border));
    Ok(())
}

/// `send-keys -X begin-selection` / `clear-selection` / `select-word` /
/// `select-line` from a mouse bind on a pane in copy mode act at the mouse,
/// as in tmux: begin starts a drag selection there, the others move the
/// cursor there first. Whether it was handled in full.
pub fn copy_command_at_mouse(srv: &mut Server, ctx: &Ctx, pane: PaneId, name: &str) -> bool {
    let Some(t) = ctx.mouse.filter(|t| t.pane == Some(pane)) else {
        return false;
    };
    if !srv.panes.get(&pane).is_some_and(|p| p.copy.is_some()) {
        return false;
    }
    match name {
        "begin-selection" => start_selection(srv, ctx, pane, t),
        // Select the word or line under the mouse: move there, then let the
        // command run as usual.
        "select-word" | "select-line" => {
            move_copy_cursor(srv, pane, t.col, t.row);
            return false;
        }
        "clear-selection" => {
            move_copy_cursor(srv, pane, t.col, t.row);
            if let Some(cm) = srv.panes.get_mut(&pane).and_then(|p| p.copy.as_mut()) {
                cm.anchor = None;
            }
        }
        _ => return false,
    }
    true
}

fn start_selection(srv: &mut Server, ctx: &Ctx, pane: PaneId, t: MouseTarget) {
    move_copy_cursor(srv, pane, t.col, t.row);
    if let Some(cm) = srv.panes.get_mut(&pane).and_then(|p| p.copy.as_mut()) {
        cm.apply("begin-selection", None);
    }
    if let Some(id) = ctx.client {
        set_drag(srv, id, Some(Drag::Select { pane }));
    }
}

fn set_drag(srv: &mut Server, id: ClientId, drag: Option<Drag>) {
    if let Some(a) = srv.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
        a.drag = drag;
    }
}

fn set_press(srv: &mut Server, id: ClientId, press: Option<Press>) {
    if let Some(a) = srv.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
        a.press = press;
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
