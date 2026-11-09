//! Mouse handling for an attached client, as tmux does it.
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
//!
//! With `mouse off` no key is bound: each event goes straight to the
//! program under it, and the client's terminal reports the mouse only
//! while the active pane's program asks for it ([`capture`]).

use std::time::{Duration, Instant};

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use tmxr_command::{MouseAction, MouseKey, MouseLocation};

use crate::cmds::Ctx;
use crate::layout::Dir;
use crate::model::{ClientId, PaneId, SessionId, WindowId};
use crate::server::Server;

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
    /// Dragging `pane`'s scrollbar slider, held `grab` rows below its top.
    Slider { pane: PaneId, grab: u16 },
}

/// A held mouse button: where it went down, and whether it has moved since,
/// which turns its release into `MouseDragEnd` and locates the drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Press {
    pub col: u16,
    pub row: u16,
    pub dragged: bool,
}

/// A press, and how many presses in a row it ends: one with the same
/// button and modifiers, on the same pane or other place, within
/// [`CLICK_TIME`] of the last counts on, wherever in it (tmux's
/// `click_button`, `click_loc` and `click_wp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Click {
    pub at: Instant,
    pub button: u8,
    pub mods: KeyModifiers,
    pub location: MouseLocation,
    pub pane: Option<PaneId>,
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
    /// On a scrollbar's slider: how many rows below its top it was taken.
    pub grab: u16,
    /// Not for the pane's program (tmux's `ignore`): a `DoubleClick`.
    pub ignore: bool,
}

const MODS: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SHIFT);

use crate::format_draw::Range;

pub fn handle(srv: &mut Server, id: ClientId, m: MouseEvent) {
    let Some(att) = srv.clients.get(&id).and_then(|c| c.att.as_ref()) else {
        return;
    };
    // An open menu takes every mouse event, as tmux's, whose overlay has
    // the keys before the `mouse` option is looked at.
    if matches!(att.overlay, Some(crate::overlay::Overlay::Menu(_))) {
        srv.menu_mouse(id, m);
        return;
    }
    let press = att.press;
    // A mouse event is input as a key is, for assume-paste-time's gap, as
    // tmux's server_client_key_callback times both.
    let session = att.session;
    if let Some(s) = srv.sessions.get_mut(&session) {
        s.key_at = Some(std::time::Instant::now());
    }
    if !srv.client_session_options(id).mouse {
        pass_through(srv, id, m);
        return;
    }
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
            (MouseAction::Down(button(b)), here)
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
        MouseEventKind::Moved => (MouseAction::Move, here),
        // tmux has no wheel key for the wheel sideways: it is buttons 6
        // and 7, a press with no release (server_client_check_mouse), which
        // counts toward a double click as any press does.
        MouseEventKind::ScrollLeft => (MouseAction::Down(6), here),
        MouseEventKind::ScrollRight => (MouseAction::Down(7), here),
    };
    let Some(target) = locate(srv, id, m, at) else {
        return;
    };
    // A press counts toward a double click where it lands.
    let action = match action {
        MouseAction::Down(b) => count_click(srv, id, &target, b),
        other => other,
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

/// With `mouse` off nothing is bound to the mouse: each event goes to the
/// program in the pane under it, as tmux's server_client_key_callback
/// forwards mouse keys when the option is off. As its input_key_mouse, one
/// off every pane (on the status line, a border or a scrollbar), on a pane
/// in copy mode, or for a program that did not ask for the mouse goes
/// nowhere. On a pane showing the clock a press ends the clock, as any key
/// does (tmux's window_clock_key), and goes no further.
fn pass_through(srv: &mut Server, id: ClientId, m: MouseEvent) {
    if srv.client_read_only(id) {
        return;
    }
    let Some(target) = locate(srv, id, m, (m.column, m.row)) else {
        return;
    };
    if target.location != MouseLocation::Pane {
        return;
    }
    // Copy mode takes no mouse key (tmux's has no `.key`); the clock's is
    // [`forward`]'s to give.
    let in_copy = |p: PaneId| srv.panes.get(&p).is_some_and(|p| p.copy.is_some());
    if target.pane.is_none_or(in_copy) {
        return;
    }
    let ctx = Ctx {
        client: Some(id),
        pane: target.pane,
        mouse: Some(target),
        ..Ctx::default()
    };
    let _ = forward(srv, &ctx);
}

/// Whether `pane` shows a mode (copy mode or the clock) over its program,
/// which then gets no mouse: tmux's window_pane_key gives the mode the key.
pub(crate) fn in_mode(srv: &Server, pane: PaneId) -> bool {
    srv.panes
        .get(&pane)
        .is_some_and(|p| p.copy.is_some() || p.clock)
}

/// Whether client `id`'s terminal should report the mouse, as tmux's
/// server_client_reset_state sets its mouse modes on every loop: always
/// with `mouse` on; with it off, the modes of what the client shows on
/// top, an open menu (tmux's menu screen asks for every motion) or else
/// its active pane, when the program there asked for the mouse and no mode
/// covers it. Any other overlay asks for none: tmux's prompts, its modes in
/// a pane and `display-panes` have screens with no mouse modes, and tmxr's
/// popups take no mouse.
///
/// One bool, not the program's own modes: the terminal reports every
/// motion and [`forward`] encodes each event in the mode and encoding the
/// program asked for, which gives it the bytes tmux's narrower terminal
/// modes would.
pub fn capture(srv: &Server, id: ClientId) -> bool {
    if srv.client_session_options(id).mouse {
        return true;
    }
    let Some(att) = srv.clients.get(&id).and_then(|c| c.att.as_ref()) else {
        return false;
    };
    match att.overlay {
        Some(crate::overlay::Overlay::Menu(_)) => true,
        Some(_) => false,
        None => srv
            .active_pane_of_session(att.session)
            .filter(|&p| !in_mode(srv, p))
            .and_then(|p| srv.panes.get(&p))
            .is_some_and(|p| p.emu.screen().mouse_protocol_mode() != vt1k::MouseProtocolMode::None),
    }
}

/// Record a press of `button` at `target` and return its key: `Down`, or
/// the second or third press in a row; a fourth starts over.
fn count_click(srv: &mut Server, id: ClientId, target: &MouseTarget, button: u8) -> MouseAction {
    let Some(att) = srv.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else {
        return MouseAction::Down(button);
    };
    let now = Instant::now();
    let mods = target.event.modifiers & MODS;
    let count = match att.click {
        Some(c)
            if c.button == button
                && c.mods == mods
                && c.location == target.location
                && c.pane == target.pane
                && now.duration_since(c.at) <= CLICK_TIME
                && c.count < 3 =>
        {
            c.count + 1
        }
        _ => 1,
    };
    att.click = Some(Click {
        at: now,
        button,
        mods,
        location: target.location,
        pane: target.pane,
        count,
        double: None,
    });
    match count {
        2 => MouseAction::SecondClick(button),
        3 => MouseAction::TripleClick(button),
        _ => MouseAction::Down(button),
    }
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
        // With `mouse` turned off since the second click, tmux forwards
        // the DoubleClick to the pane, which ignores it.
        if !srv.client_session_options(id).mouse {
            continue;
        }
        // The pane's program had the second press already: `send-keys -M`
        // passes this one nowhere, as tmux's ignored DoubleClick event.
        let target = MouseTarget {
            ignore: true,
            ..target
        };
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
    let table = match target.pane {
        Some(p) if on_pane && in_copy => srv.copy_table(p),
        _ => "root",
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
    // Not bound in copy mode's table, the root table's bind runs, as tmux's
    // server_client_key_callback tries the root table after a mode's.
    let key = key.into();
    let bind = srv
        .keys
        .get(table, &key)
        .or_else(|| srv.keys.get("root", &key).filter(|_| table != "root"))
        .cloned();
    match bind {
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
        grab: 0,
        ignore: false,
    };
    let sess = srv.sessions.get(&session)?;
    let lines = srv.status_rows(session);
    let top = srv.window_top(session);
    let on_status = if srv.status_at_top(session) {
        row < lines
    } else {
        row >= att.rows.saturating_sub(lines)
    };
    if lines > 0 && on_status {
        // The first line's ranges only: the lines after it are drawn from
        // status-format, whose ranges tmxr does not keep.
        let first = if srv.status_at_top(session) {
            0
        } else {
            att.rows.saturating_sub(lines)
        };
        let (location, range) = if row == first {
            att.status_ranges.at(col)
        } else {
            (MouseLocation::StatusDefault, None)
        };
        t.location = location;
        // A range's target, as tmux's server_client_check_mouse sets it: a
        // pane or session that is gone makes the click nothing.
        match range {
            Some(Range::Window(i)) => t.window = sess.windows.get(i).map(|w| (*i, *w)),
            Some(Range::Pane(p)) => {
                srv.panes.get(p)?;
                t.pane = Some(*p);
            }
            Some(Range::Session(s)) => {
                srv.sessions.get(s)?;
                t.session = *s;
            }
            _ => {}
        }
        return Some(t);
    }
    let wid = sess.current_window()?;
    t.window = Some((sess.current, wid));
    // In window cells: the view may be panned across a larger window.
    let (ox, oy) = crate::render::window_offset(srv, att);
    let (col, row) = (col + ox, row - top + oy);
    (t.col, t.row) = (col, row);
    let rects = srv.pane_cells(wid);
    if let Some((pane, _)) = rects
        .iter()
        .find(|(_, r)| col >= r.x && col < r.x + r.w && row >= r.y && row < r.y + r.h)
    {
        t.pane = Some(*pane);
        t.location = match crate::scrollbar::hit(srv, *pane, col, row) {
            Some((location, grab)) => {
                t.grab = grab;
                location
            }
            None => MouseLocation::Pane,
        };
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
    // In window cells, as the drag's start was found.
    let (col, row) = window_cell(srv, id, m);
    match (drag, m.kind) {
        (Drag::Border { pane, vertical, at }, MouseEventKind::Drag(MouseButton::Left)) => {
            let now = if vertical { col } else { row };
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
        (Drag::Slider { pane, grab }, MouseEventKind::Drag(MouseButton::Left)) => {
            scroll_to_slider(srv, pane, grab, row, false);
        }
        (Drag::Select { pane }, MouseEventKind::Drag(MouseButton::Left)) => {
            move_copy_cursor(srv, pane, col, row);
        }
        (Drag::Select { pane }, MouseEventKind::Up(MouseButton::Left)) => {
            set_drag(srv, id, None);
            set_press(srv, id, None);
            // The copy table's MouseDragEnd1Pane decides what the selection
            // is for (tmux-yank: copy it and leave copy mode).
            if let Some(target) = target_on(srv, id, m, pane, MouseLocation::Pane, (col, row)) {
                dispatch(srv, id, MouseAction::DragEnd(1), target);
            }
        }
        (drag, MouseEventKind::Up(b)) => {
            set_drag(srv, id, None);
            set_press(srv, id, None);
            // tmux ends every drag with a MouseDragEnd key where the button
            // was let go, except a slider's, which stays on the slider.
            let target = match drag {
                Drag::Slider { pane, .. } => {
                    target_on(srv, id, m, pane, MouseLocation::ScrollbarSlider, (col, row))
                }
                _ => locate(srv, id, m, (m.column, m.row)),
            };
            if let Some(target) = target {
                dispatch(srv, id, MouseAction::DragEnd(button(b)), target);
            }
        }
        // Another button or the wheel during a drag: handled as usual.
        _ => return false,
    }
    true
}

/// Where client `id`'s mouse event `m` is in its window's cells: the view
/// may be panned across a larger window, below a status line at the top.
fn window_cell(srv: &Server, id: ClientId, m: MouseEvent) -> (u16, u16) {
    let (ox, oy, top) = srv
        .clients
        .get(&id)
        .and_then(|c| c.att.as_ref())
        .map_or((0, 0, 0), |a| {
            let (ox, oy) = crate::render::window_offset(srv, a);
            (ox, oy, srv.window_top(a.session))
        });
    (m.column + ox, m.row.saturating_sub(top) + oy)
}

/// A mouse key's target on `pane` at window cell `at`, as `location`.
fn target_on(
    srv: &Server,
    id: ClientId,
    event: MouseEvent,
    pane: PaneId,
    location: MouseLocation,
    (col, row): (u16, u16),
) -> Option<MouseTarget> {
    let session = srv.clients.get(&id)?.att.as_ref()?.session;
    let window = srv.panes.get(&pane)?.window;
    let index = srv.sessions.get(&session).and_then(|s| s.index_of(window));
    Some(MouseTarget {
        event,
        location,
        col,
        row,
        session,
        window: index.map(|i| (i, window)),
        pane: Some(pane),
        border: None,
        grab: 0,
        ignore: false,
    })
}

/// `send-keys -M`: pass the bind's mouse event to the program in the pane
/// under it, in the encoding that program asked for (nothing if it did not).
pub fn forward(srv: &mut Server, ctx: &Ctx) -> Result<(), String> {
    let t = ctx.mouse.ok_or("send-keys -M needs a mouse event")?;
    let pane = t.pane.ok_or("no pane under the mouse")?;
    if t.ignore {
        return Ok(());
    }
    // A pane showing the clock takes the event for itself, and ends; no
    // mode takes the mouse moving (tmux's window_pane_key drops it).
    let motion = t.event.kind == MouseEventKind::Moved;
    if srv.panes.get(&pane).is_some_and(|p| p.clock) {
        if !motion {
            srv.clock_takes_key(pane, ctx.client.is_some());
        }
        return Ok(());
    }
    // Where the event itself is, as tmux's input_key_mouse reports it: a
    // drag's key is located at its press, but its motion and release are
    // told where they happen. In window cells, as the pane's rect.
    let (col, row) = match ctx.client {
        Some(id) => window_cell(srv, id, t.event),
        None => (t.col, t.row),
    };
    let p = srv.panes.get_mut(&pane).ok_or("no pane under the mouse")?;
    if p.input_off {
        return Ok(());
    }
    let r = p.rect;
    let col = col.saturating_sub(r.x).min(r.w.saturating_sub(1));
    let row = row.saturating_sub(r.y).min(r.h.saturating_sub(1));
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

/// `copy-mode -S`: the scrollbar slider under the mouse follows it, the
/// pane in copy mode, until the button is let go (`-e`: leaving copy mode
/// at the bottom).
pub fn slider_drag(srv: &mut Server, ctx: &Ctx, exit: bool) -> Result<(), String> {
    let t = ctx.mouse.ok_or("copy-mode -S needs a mouse event")?;
    let pane = t.pane.ok_or("no pane under the mouse")?;
    let id = ctx.client.ok_or("no client")?;
    let (oy, top) = srv
        .clients
        .get(&id)
        .and_then(|c| c.att.as_ref())
        .map_or((0, 0), |a| {
            (
                crate::render::window_offset(srv, a).1,
                srv.window_top(a.session),
            )
        });
    crate::copy::enter(srv, pane, false);
    if let Some(cm) = srv.panes.get_mut(&pane).and_then(|p| p.copy.as_mut())
        && exit
    {
        cm.scroll_exit = true;
    }
    scroll_to_slider(
        srv,
        pane,
        t.grab,
        t.event.row.saturating_sub(top) + oy,
        exit,
    );
    set_drag(srv, id, Some(Drag::Slider { pane, grab: t.grab }));
    Ok(())
}

/// Move `pane`'s copy-mode view so its slider follows the mouse at window
/// row `my`, held `grab` rows below the slider's top: tmux 3.6's
/// `window_copy_scroll1`, the inverse of the slider's own formula. With
/// `exit`, reaching the bottom leaves copy mode.
fn scroll_to_slider(srv: &mut Server, pane: PaneId, grab: u16, my: u16, exit: bool) {
    let Some(p) = srv.panes.get_mut(&pane) else {
        return;
    };
    let (top, h) = (i32::from(p.rect.y), i32::from(p.rect.h));
    let (_, slider_h) = crate::scrollbar::slider(p, p.rect.h);
    let (my, grab, slider_h) = (i32::from(my), i32::from(grab), i32::from(slider_h));
    let new_y = if my <= top + grab {
        0
    } else if my - grab > top + h - slider_h {
        h - slider_h
    } else {
        my - top - grab + 1
    };
    let Some(cm) = p.copy.as_mut() else {
        return;
    };
    let size = cm.lines.len().saturating_sub(usize::from(cm.rows));
    let offset =
        (f64::from(new_y.max(0)) * ((size as f64 + f64::from(h)) / f64::from(h.max(1)))) as usize;
    cm.top = offset.min(size);
    let rows = usize::from(cm.rows.max(1));
    cm.cy = cm.cy.clamp(cm.top, cm.top + rows - 1);
    let at_bottom = cm.top == size;
    let window = p.window;
    srv.mark_window_dirty(window);
    if exit && at_bottom {
        if let Some(p) = srv.panes.get_mut(&pane) {
            p.copy = None;
        }
        srv.refit_scrollbar(pane);
    }
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
                cm.apply("clear-selection", None);
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
