//! Compose one client's frame: panes, borders, status line, overlays.

use ratatui::Terminal;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use tmxr_command::format::{self, Colour};

use crate::backend::AnsiBackend;
use crate::copy::cell_style;
use crate::model::{ClientId, PaneId, WindowId};
use crate::overlay::{Overlay, PickerOverlay, Previewed};
use crate::server::{STATUS_ROWS, Server};
use crate::vars::Vars;

/// `(first column, end column, window index)` of each window in the status
/// line, for mouse clicks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRanges {
    /// Each window's cells in the window list: start, end, window index.
    pub windows: Vec<(u16, u16, u32)>,
    /// Where `status-left` ends.
    pub left_end: u16,
    /// Where `status-right` starts.
    pub right_start: u16,
}

impl Default for StatusRanges {
    /// Nothing drawn (a message or prompt in the status line): every cell is
    /// outside the left, right and window parts.
    fn default() -> Self {
        Self {
            windows: Vec::new(),
            left_end: 0,
            right_start: u16::MAX,
        }
    }
}

impl StatusRanges {
    /// What a click at `col` of the status line is on, and the window index
    /// when it is a window.
    pub fn at(&self, col: u16) -> (tmxr_command::MouseLocation, Option<u32>) {
        use tmxr_command::MouseLocation as L;
        if let Some((_, _, idx)) = self.windows.iter().find(|(a, b, _)| col >= *a && col < *b) {
            (L::Status, Some(*idx))
        } else if col < self.left_end {
            (L::StatusLeft, None)
        } else if col >= self.right_start {
            (L::StatusRight, None)
        } else {
            (L::StatusDefault, None)
        }
    }
}

fn colour(c: Colour) -> Color {
    match c {
        Colour::Default => Color::Reset,
        Colour::Indexed(i) => Color::Indexed(i),
        Colour::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Apply a parsed tmux style on top of `base`.
fn apply(base: Style, s: &format::Style) -> Style {
    let mut out = base;
    if let Some(fg) = s.fg {
        out = out.fg(colour(fg));
    }
    if let Some(bg) = s.bg {
        out = out.bg(colour(bg));
    }
    for (flag, m) in [
        (s.bold, Modifier::BOLD),
        (s.dim, Modifier::DIM),
        (s.italic, Modifier::ITALIC),
        (s.underscore, Modifier::UNDERLINED),
        (s.reverse, Modifier::REVERSED),
    ] {
        match flag {
            Some(true) => out = out.add_modifier(m),
            Some(false) => out = out.remove_modifier(m),
            None => {}
        }
    }
    out
}

/// A style option (`fg=…,bg=…`), format-expanded first.
fn style_option(spec: &str, vars: &Vars<'_>) -> Style {
    let mut s = format::Style::default();
    s.apply(&format::expand(spec, vars));
    apply(Style::default(), &s)
}

/// Draw expanded, styled text at (`x`, `y`) without passing `max_x`.
/// Returns the column after the text.
pub(crate) fn draw_runs(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    max_x: u16,
    base: Style,
    text: &str,
) -> u16 {
    let mut x = x;
    for (st, chunk) in format::styled(text) {
        if x >= max_x {
            break;
        }
        let style = apply(base, &st);
        let (nx, _) = buf.set_stringn(x, y, &chunk, usize::from(max_x - x), style);
        x = nx;
    }
    x
}

/// Display width of expanded, styled text.
pub(crate) fn runs_width(text: &str) -> u16 {
    format::styled(text)
        .iter()
        .map(|(_, t)| Line::raw(t.as_str()).width() as u16)
        .sum()
}

/// Where a client's view of its window starts, for a window larger than
/// the client (tmux 3.6's `tty_window_offset1`): a pan holds the view,
/// clamped to the window; otherwise it follows the active pane's cursor,
/// centring it once it would leave the first screenful.
pub fn window_offset(srv: &Server, att: &crate::server::Attached) -> (u16, u16) {
    let Some(win) = srv
        .sessions
        .get(&att.session)
        .and_then(crate::model::Session::current_window)
        .and_then(|w| srv.windows.get(&w))
    else {
        return (0, 0);
    };
    let (sx, sy) = (att.cols, att.rows.saturating_sub(STATUS_ROWS));
    if sx >= win.cols && sy >= win.rows {
        return (0, 0);
    }
    let clamp = |o: u16, view: u16, size: u16| if view >= size { 0 } else { o.min(size - view) };
    if let Some((w, ox, oy)) = att.pan
        && w == win.id
    {
        return (clamp(ox, sx, win.cols), clamp(oy, sy, win.rows));
    }
    let Some(p) = srv.panes.get(&win.active) else {
        return (0, 0);
    };
    let screen = p.emu.screen();
    if screen.hide_cursor() {
        return (0, 0);
    }
    let (row, col) = screen.cursor_position();
    let follow = |c: u16, view: u16, size: u16| {
        if c < view {
            0
        } else if c > size.saturating_sub(view) {
            size.saturating_sub(view)
        } else {
            c - view / 2
        }
    };
    (
        follow(p.rect.x + col, sx, win.cols),
        follow(p.rect.y + row, sy, win.rows),
    )
}

/// Draw the client's frame. Returns the status-line window ranges.
pub fn draw(srv: &Server, id: ClientId, term: &mut Terminal<AnsiBackend>) -> StatusRanges {
    let mut ranges = StatusRanges::default();
    let Some(att) = srv.clients.get(&id).and_then(|c| c.att.as_ref()) else {
        return ranges;
    };
    let (cols, rows) = (att.cols, att.rows);
    let session = srv.sessions.get(&att.session);
    let window = session.and_then(|s| s.current_window());
    let _ = term.draw(|f| {
        let buf = f.buffer_mut();
        let pane_rows = rows.saturating_sub(STATUS_ROWS);
        let mut cursor = None;
        if let Some(wid) = window {
            cursor = match srv.windows.get(&wid) {
                // Larger than the client: drawn whole, then the part in
                // view copied out.
                Some(win) if win.cols > cols || win.rows > pane_rows => {
                    let (ox, oy) = window_offset(srv, att);
                    let mut whole = Buffer::empty(Rect::new(0, 0, win.cols, win.rows));
                    let at = draw_window(srv, id, wid, &mut whole, win.cols, win.rows);
                    for y in 0..pane_rows.min(win.rows.saturating_sub(oy)) {
                        for x in 0..cols.min(win.cols.saturating_sub(ox)) {
                            if let (Some(cell), Some(out)) =
                                (whole.cell((x + ox, y + oy)), buf.cell_mut((x, y)))
                            {
                                *out = cell.clone();
                            }
                        }
                    }
                    at.and_then(|c| {
                        let (x, y) = (c.x.checked_sub(ox)?, c.y.checked_sub(oy)?);
                        (x < cols && y < pane_rows).then_some(Position::new(x, y))
                    })
                }
                _ => draw_window(srv, id, wid, buf, cols, pane_rows),
            };
        }
        ranges = draw_status(srv, id, buf, cols, rows);
        if let Some(c) = draw_overlay(srv, id, buf, cols, rows) {
            cursor = Some(c);
        }
        if let Some(c) = cursor {
            f.set_cursor_position(c);
        }
    });
    ranges
}

/// Draw panes and borders. Returns where the cursor belongs, if visible.
fn draw_window(
    srv: &Server,
    client: ClientId,
    wid: WindowId,
    buf: &mut Buffer,
    cols: u16,
    rows: u16,
) -> Option<Position> {
    let win = srv.windows.get(&wid)?;
    let rects = win.visible_rects();
    let vars = Vars::for_pane(srv, Some(win.active), Some(client));
    let border = style_option(&srv.cfg.status.pane_border_style, &vars);
    let active_border = style_option(&srv.cfg.status.pane_active_border_style, &vars);
    let copy = PaneStyles {
        selection: style_option(&srv.cfg.status.mode_style, &vars),
        matched: style_option(&srv.cfg.status.copy_mode_match_style, &vars),
        current: style_option(&srv.cfg.status.copy_mode_current_match_style, &vars),
        clock: style_option(&format!("fg={}", srv.cfg.status.clock_mode_colour), &vars),
    };
    let w = win.cols.min(cols);
    let h = win.rows.min(rows);

    let scrollbar = style_option(&srv.cfg.pane_scrollbars_style, &vars);
    let mut cursor = None;
    for (pid, cell) in &rects {
        // The pane's own cells: its layout cell less any scrollbar.
        let r = srv.panes.get(pid).map_or(*cell, |p| p.rect);
        if let Some(c) = draw_pane(srv, *pid, r, buf, w, h, &copy)
            && *pid == win.active
        {
            cursor = Some(c);
        }
        crate::scrollbar::draw(srv, *pid, buf, scrollbar);
    }

    // Borders: every cell of the window area that no pane covers.
    let covered = |x: u16, y: u16| {
        rects
            .iter()
            .any(|(_, r)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
    };
    let is_border = |x: i32, y: i32| {
        x >= 0 && y >= 0 && x < i32::from(w) && y < i32::from(h) && !covered(x as u16, y as u16)
    };
    let rect_of = |pane: PaneId| rects.iter().find(|(p, _)| *p == pane).map(|(_, r)| *r);
    let active = rect_of(win.active);
    // The marked pane's border is drawn reversed, as tmux does.
    let marked = srv.marked_pane().and_then(rect_of);
    let touches = |rect: Option<hjkl_layout::LayoutRect>, x: u16, y: u16| {
        rect.is_some_and(|r| {
            let (x, y) = (i32::from(x), i32::from(y));
            let (rx, ry, rw, rh) = (
                i32::from(r.x),
                i32::from(r.y),
                i32::from(r.w),
                i32::from(r.h),
            );
            x >= rx - 1 && x <= rx + rw && y >= ry - 1 && y <= ry + rh
        })
    };
    for y in 0..h {
        for x in 0..w {
            if covered(x, y) {
                continue;
            }
            let (xi, yi) = (i32::from(x), i32::from(y));
            let (l, r, u, d) = (
                is_border(xi - 1, yi),
                is_border(xi + 1, yi),
                is_border(xi, yi - 1),
                is_border(xi, yi + 1),
            );
            let ch = match (l || r, u || d) {
                (true, false) => "─",
                (false, true) | (false, false) => "│",
                (true, true) => match (l, r, u, d) {
                    (true, true, true, true) => "┼",
                    (false, true, true, true) => "├",
                    (true, false, true, true) => "┤",
                    (true, true, false, true) => "┬",
                    (true, true, true, false) => "┴",
                    (false, true, false, true) => "┌",
                    (true, false, false, true) => "┐",
                    (false, true, true, false) => "└",
                    _ => "┘",
                },
            };
            let mut style = if touches(active, x, y) && rects.len() > 1 {
                active_border
            } else {
                border
            };
            if touches(marked, x, y) {
                style = style.add_modifier(Modifier::REVERSED);
            }
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(ch).set_style(style);
            }
        }
    }
    // Beyond a window smaller than the client (`resize-window`), as tmux.
    for y in 0..rows {
        for x in 0..cols {
            if (x >= w || y >= h)
                && let Some(cell) = buf.cell_mut((x, y))
            {
                cell.set_symbol("·").set_style(border);
            }
        }
    }
    cursor
}

/// Draw one pane. Returns its cursor position on screen when shown.
/// How a pane's modes draw: copy mode's selection and search matches, and
/// clock mode's digits.
struct PaneStyles {
    selection: Style,
    matched: Style,
    current: Style,
    clock: Style,
}

fn draw_pane(
    srv: &Server,
    pid: PaneId,
    r: hjkl_layout::LayoutRect,
    buf: &mut Buffer,
    max_w: u16,
    max_h: u16,
    copy: &PaneStyles,
) -> Option<Position> {
    let pane = srv.panes.get(&pid)?;
    let w = r.w.min(max_w.saturating_sub(r.x));
    let h = r.h.min(max_h.saturating_sub(r.y));
    if pane.clock {
        draw_clock(buf, Rect::new(r.x, r.y, w, h), copy.clock);
        return None;
    }
    if let Some(cm) = &pane.copy {
        for row in 0..h {
            let y = cm.top + usize::from(row);
            let line = cm.lines.get(y);
            let matches = cm.match_spans(y);
            for col in 0..w {
                let x = usize::from(col);
                let cell = line.and_then(|l| l.cells.get(x));
                let mut style = cell.map(|c| c.style).unwrap_or_default();
                if let Some((start, _)) = matches.iter().find(|(s, e)| (*s..*e).contains(&x)) {
                    let current = y == cm.cy && *start == cm.cx;
                    style = style.patch(if current { copy.current } else { copy.matched });
                }
                if cm.selected(y, x) {
                    style = style.patch(copy.selection);
                }
                if y == cm.cy && usize::from(col) == cm.cx {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                let sym = cell.map_or(" ", |c| {
                    if c.symbol.is_empty() {
                        " "
                    } else {
                        c.symbol.as_str()
                    }
                });
                if let Some(out) = buf.cell_mut((r.x + col, r.y + row)) {
                    out.set_symbol(sym).set_style(style);
                }
            }
        }
        // tmux's position indicator, top right.
        let (offset, total) = cm.position();
        // A count being typed shows first, as tmux's `(repeat)` prompt;
        // `toggle-position` hides the position itself.
        let position = if cm.hide_position {
            String::new()
        } else {
            format!("[{offset}/{total}]")
        };
        let tag = match cm.count {
            0 => position,
            n => format!("(repeat) {n} {position}").trim_end().to_owned(),
        };
        let tw = tag.len() as u16;
        if tw < w {
            buf.set_string(r.x + w - tw, r.y, &tag, copy.selection);
        }
        return None;
    }
    let cursor = draw_screen(pane.emu.screen(), buf, Rect::new(r.x, r.y, w, h));
    if let Some(code) = pane.dead {
        // tmux's remain-on-exit line, over the pane's last row.
        let status = code.map_or_else(|| "unknown".to_owned(), |c| c.to_string());
        let line = format!("Pane is dead (status {status})");
        if h > 0 {
            let row = r.y + h - 1;
            buf.set_style(Rect::new(r.x, row, w, 1), copy.selection);
            buf.set_stringn(r.x, row, &line, usize::from(w), copy.selection);
        }
        return None;
    }
    cursor
}

/// Draw a program's screen into `area`. Returns where its cursor is, if
/// shown and inside the area.
fn draw_screen(screen: &vt100::Screen, buf: &mut Buffer, area: Rect) -> Option<Position> {
    for row in 0..area.height {
        for col in 0..area.width {
            let Some(out) = buf.cell_mut((area.x + col, area.y + row)) else {
                continue;
            };
            match screen.cell(row, col) {
                Some(c) if c.is_wide_continuation() => {
                    out.set_symbol(" ");
                }
                Some(c) => {
                    let sym = if c.has_contents() { c.contents() } else { " " };
                    out.set_symbol(sym).set_style(cell_style(c));
                }
                None => {
                    out.set_symbol(" ").set_style(Style::default());
                }
            }
        }
    }
    if screen.hide_cursor() {
        return None;
    }
    let (cy, cx) = screen.cursor_position();
    (cy < area.height && cx < area.width).then(|| Position::new(area.x + cx, area.y + cy))
}

fn draw_status(srv: &Server, id: ClientId, buf: &mut Buffer, cols: u16, rows: u16) -> StatusRanges {
    let mut ranges = StatusRanges::default();
    let Some(att) = srv.clients.get(&id).and_then(|c| c.att.as_ref()) else {
        return ranges;
    };
    if rows < STATUS_ROWS + 1 {
        return ranges;
    }
    let y = rows - STATUS_ROWS;
    let Some(session) = srv.sessions.get(&att.session) else {
        return ranges;
    };
    let active = srv.active_pane_of_session(session.id);
    let vars = Vars::for_pane(srv, active, Some(id));
    let base = style_option(&srv.cfg.status.style, &vars);
    buf.set_style(Rect::new(0, y, cols, 1), base);
    for x in 0..cols {
        if let Some(c) = buf.cell_mut((x, y)) {
            c.set_symbol(" ");
        }
    }

    // A message or prompt replaces the status line, as in tmux.
    let message_style = style_option(&srv.cfg.status.message_style, &vars);
    if let Some((msg, _)) = &att.message {
        buf.set_style(Rect::new(0, y, cols, 1), message_style);
        buf.set_stringn(0, y, msg, usize::from(cols), message_style);
        return ranges;
    }
    match &att.overlay {
        Some(Overlay::Prompt(p)) => {
            buf.set_style(Rect::new(0, y, cols, 1), message_style);
            let text = format!("{}{}", p.prompt, p.input.iter().collect::<String>());
            buf.set_stringn(0, y, &text, usize::from(cols), message_style);
            return ranges;
        }
        Some(Overlay::Confirm { prompt, .. }) => {
            buf.set_style(Rect::new(0, y, cols, 1), message_style);
            buf.set_stringn(0, y, prompt, usize::from(cols), message_style);
            return ranges;
        }
        _ => {}
    }

    let right = format::expand(&srv.cfg.status.right, &vars);
    let right_w = runs_width(&right)
        .min(srv.cfg.status.right_length)
        .min(cols);
    let limit = cols - right_w;
    let left = format::expand(&srv.cfg.status.left, &vars);
    let mut x = draw_runs(
        buf,
        0,
        y,
        limit.min(srv.cfg.status.left_length),
        base,
        &left,
    );
    ranges.left_end = x;
    ranges.right_start = cols - right_w;
    for (idx, wid) in &session.windows {
        let Some(win) = srv.windows.get(wid) else {
            continue;
        };
        let wvars = Vars {
            srv,
            session: Some(session.id),
            window: Some(*wid),
            pane: Some(win.active),
            client: Some(id),
        };
        let fmt = if *idx == session.current {
            &srv.cfg.status.window_current_format
        } else {
            &srv.cfg.status.window_format
        };
        let text = format::expand(fmt, &wvars);
        let start = x;
        x = draw_runs(buf, x, y, limit, base, &text);
        ranges.windows.push((start, x, *idx));
        if x >= limit {
            break;
        }
    }
    draw_runs(buf, cols - right_w, y, cols, base, &right);
    ranges
}

/// Draw the open overlay. Returns a cursor position if it wants one.
fn draw_overlay(
    srv: &Server,
    id: ClientId,
    buf: &mut Buffer,
    cols: u16,
    rows: u16,
) -> Option<Position> {
    let att = srv.clients.get(&id)?.att.as_ref()?;
    let vars = Vars::for_pane(srv, None, Some(id));
    let mode_style = style_option(&srv.cfg.status.mode_style, &vars);
    let border = style_option(&srv.cfg.status.pane_active_border_style, &vars);
    let y = rows.saturating_sub(STATUS_ROWS);
    // A menu's or popup's own style (-s, -S, -H), else `or`.
    let look =
        |spec: &Option<String>, or: Style| spec.as_deref().map_or(or, |s| style_option(s, &vars));
    match att.overlay.as_ref()? {
        Overlay::Prompt(p) => {
            let before: String = p.input[..p.cursor].iter().collect();
            let x = Line::raw(format!("{}{before}", p.prompt)).width() as u16;
            Some(Position::new(x.min(cols.saturating_sub(1)), y))
        }
        Overlay::Confirm { .. } => None,
        Overlay::Panes { labels, .. } => {
            let win = srv
                .sessions
                .get(&att.session)
                .and_then(|s| s.current_window())
                .and_then(|w| srv.windows.get(&w))?;
            let label_style = |colour: &str| {
                style_option(&format!("fg={colour}"), &vars)
                    .add_modifier(Modifier::REVERSED | Modifier::BOLD)
            };
            let other = label_style(&srv.cfg.status.display_panes_colour);
            let active = label_style(&srv.cfg.status.display_panes_active_colour);
            for (pid, r) in win.visible_rects() {
                let Some((label, _)) = labels.iter().find(|(_, p)| *p == pid) else {
                    continue;
                };
                let text = format!(" {label} ");
                let w = text.len() as u16;
                let x = r.x + r.w.saturating_sub(w) / 2;
                let row = r.y + r.h / 2;
                if row < y {
                    let style = if pid == win.active { active } else { other };
                    buf.set_stringn(x, row, &text, usize::from(r.w), style);
                }
            }
            None
        }
        Overlay::Text { lines, top } => {
            let area = Rect::new(0, 0, cols, y);
            Clear.render(area, buf);
            for (i, line) in lines.iter().skip(*top).take(usize::from(y)).enumerate() {
                buf.set_stringn(0, i as u16, line, usize::from(cols), Style::default());
            }
            let tag = format!("[{}/{}]", top + 1, lines.len());
            let tw = tag.len() as u16;
            if tw < cols {
                buf.set_string(cols - tw, 0, &tag, mode_style);
            }
            None
        }
        Overlay::Picker(p) => draw_picker(srv, p, buf, cols, y, border, mode_style),
        Overlay::Menu(m) => {
            let styles = (
                look(&m.look.style, Style::default()),
                look(&m.look.border_style, border),
                look(&m.look.selected_style, mode_style),
            );
            m.draw(buf, cols, y, styles);
            None
        }
        Overlay::Popup(p) => {
            let area = p.rect.intersection(Rect::new(0, 0, cols, rows));
            Clear.render(area, buf);
            buf.set_style(area, look(&p.look.style, Style::default()));
            if p.border {
                let mut block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(p.look.lines.unwrap_or_default())
                    .border_style(look(&p.look.border_style, border));
                if !p.title.is_empty() {
                    block = block.title(p.title.as_str());
                }
                block.render(area, buf);
            }
            let inner = p.inner().intersection(area);
            let cursor = draw_screen(p.emu.screen(), buf, inner);
            cursor.filter(|_| !p.exited)
        }
    }
}

/// Most rows the picker list takes when a preview sits under it.
const PICKER_LIST_ROWS: u16 = 15;
/// Fewest rows worth giving a preview.
const PREVIEW_MIN_ROWS: u16 = 3;

/// The picker overlay: query row, matches, and for sessions and windows a
/// preview of the highlighted entry's active pane. `y` is the first row
/// below the window area. Returns the query cursor in insert mode.
fn draw_picker(
    srv: &Server,
    p: &PickerOverlay,
    buf: &mut Buffer,
    cols: u16,
    y: u16,
    border: Style,
    mode_style: Style,
) -> Option<Position> {
    let picker = &p.picker;
    let preview_pane = p.previewed().and_then(|v| {
        let window = match v {
            Previewed::Session(s) => srv.sessions.get(&s)?.current_window()?,
            Previewed::Window(s, i) => *srv.sessions.get(&s)?.windows.get(&i)?,
        };
        srv.windows.get(&window).map(|w| w.active)
    });
    let w = cols
        .saturating_sub(4)
        .clamp(10, if preview_pane.is_some() { 100 } else { 70 });
    let list_rows = (picker.matched() as u16).clamp(1, PICKER_LIST_ROWS);
    let list_h = list_rows + 3;
    let room = y.saturating_sub(2);
    let preview_rows = match preview_pane {
        Some(_) => room.saturating_sub(list_h + 1),
        None => 0,
    };
    let preview_rows = if preview_rows >= PREVIEW_MIN_ROWS {
        preview_rows
    } else {
        0
    };
    let h = if preview_rows > 0 {
        list_h + 1 + preview_rows
    } else {
        list_h
    }
    .min(room)
    .max(4);
    let area = Rect::new(
        (cols.saturating_sub(w)) / 2,
        y.saturating_sub(h) / 2,
        w.min(cols),
        h,
    );
    Clear.render(area, buf);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(format!(
            " {} {}/{} ",
            picker.title(),
            picker.matched(),
            picker.total()
        ));
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.height == 0 {
        return None;
    }
    let query = p.query();
    Paragraph::new(Line::from(vec![
        Span::styled("> ", border),
        Span::raw(query.clone()),
    ]))
    .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);
    let list_space = if preview_rows > 0 {
        list_rows
    } else {
        inner.height.saturating_sub(1)
    };
    let visible = usize::from(list_space);
    let matched = picker.matched();
    let start = picker
        .selected
        .saturating_sub(visible.saturating_sub(1))
        .min(matched.saturating_sub(visible.min(matched)));
    let end = (start + visible).min(matched);
    // Matched characters take the picker's accent: the active border
    // colour, like its frame and prompt.
    let highlight = border.add_modifier(Modifier::BOLD);
    for (i, (label, hits, _)) in picker.visible_rows(start..end).into_iter().enumerate() {
        let row = inner.y + 1 + i as u16;
        let selected = start + i == picker.selected;
        let base = if selected {
            mode_style
        } else {
            Style::default()
        };
        let spans: Vec<Span> = label
            .chars()
            .enumerate()
            .map(|(ci, ch)| {
                let st = if hits.contains(&ci) {
                    base.patch(highlight)
                } else {
                    base
                };
                Span::styled(ch.to_string(), st)
            })
            .collect();
        if selected {
            buf.set_style(Rect::new(inner.x, row, inner.width, 1), mode_style);
        }
        Paragraph::new(Line::from(spans)).render(Rect::new(inner.x, row, inner.width, 1), buf);
    }
    if let Some(pid) = preview_pane.filter(|_| preview_rows > 0) {
        let sep = inner.y + 1 + list_rows;
        buf.set_string(
            inner.x,
            sep,
            "\u{2500}".repeat(usize::from(inner.width)),
            border,
        );
        let rows = preview_rows.min(inner.y + inner.height - sep - 1);
        draw_preview(
            srv,
            pid,
            buf,
            Rect::new(inner.x, sep + 1, inner.width, rows),
        );
    }
    let x = inner.x + 2 + Line::raw(query).width() as u16;
    Some(Position::new(
        x.min(inner.x + inner.width.saturating_sub(1)),
        inner.y,
    ))
}

/// A pane's screen in `area`: the rows up to its cursor (its latest output),
/// cropped on the right.
fn draw_preview(srv: &Server, pane: PaneId, buf: &mut Buffer, area: Rect) {
    let Some(p) = srv.panes.get(&pane) else {
        return;
    };
    let screen = p.emu.screen();
    let (cursor_row, _) = screen.cursor_position();
    let first = (cursor_row + 1).saturating_sub(area.height);
    for row in 0..area.height {
        for col in 0..area.width {
            let cell = screen.cell(first + row, col);
            let style = cell.map(cell_style).unwrap_or_default();
            let sym = cell
                .map(vt100::Cell::contents)
                .filter(|s| !s.is_empty())
                .unwrap_or(" ");
            if let Some(out) = buf.cell_mut((area.x + col, area.y + row)) {
                out.set_symbol(sym).set_style(style);
            }
        }
    }
}

/// tmux's clock font: 5x5 cells per glyph, `#` painted.
const CLOCK_FONT: [[&str; 5]; 11] = [
    ["#####", "#...#", "#...#", "#...#", "#####"], // 0
    ["....#", "....#", "....#", "....#", "....#"], // 1
    ["#####", "....#", "#####", "#....", "#####"], // 2
    ["#####", "....#", "#####", "....#", "#####"], // 3
    ["#...#", "#...#", "#####", "....#", "....#"], // 4
    ["#####", "#....", "#####", "....#", "#####"], // 5
    ["#####", "#....", "#####", "#...#", "#####"], // 6
    ["#####", "....#", "....#", "....#", "....#"], // 7
    ["#####", "#...#", "#####", "#...#", "#####"], // 8
    ["#####", "#...#", "#####", "....#", "#####"], // 9
    [".....", "..#..", ".....", "..#..", "....."], // :
];

/// Clock mode: the local time in big digits in the middle of the pane, or as
/// plain `HH:MM` when the pane is too small for them.
fn draw_clock(buf: &mut Buffer, area: Rect, colour: Style) {
    Clear.render(area, buf);
    let Some((h, m)) = tmxr_term::localtime::hour_minute() else {
        return;
    };
    let text = format!("{h:02}:{m:02}");
    let glyphs: Vec<usize> = text
        .chars()
        .map(|c| c.to_digit(10).map_or(10, |d| d as usize))
        .collect();
    let big_w = (glyphs.len() * 6 - 1) as u16;
    if area.width < big_w || area.height < 5 {
        let x = area.x + area.width.saturating_sub(5) / 2;
        let y = area.y + area.height / 2;
        buf.set_stringn(x, y, &text, usize::from(area.width), colour);
        return;
    }
    let paint = Style::default().bg(colour.fg.unwrap_or(Color::Blue));
    let x0 = area.x + (area.width - big_w) / 2;
    let y0 = area.y + (area.height - 5) / 2;
    for (i, g) in glyphs.iter().enumerate() {
        for (row, bits) in CLOCK_FONT[*g].iter().enumerate() {
            for (col, bit) in bits.chars().enumerate() {
                if bit == '#'
                    && let Some(cell) = buf.cell_mut((x0 + (i * 6 + col) as u16, y0 + row as u16))
                {
                    cell.set_symbol(" ").set_style(paint);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmxr_command::MouseLocation as L;

    #[test]
    fn status_clicks_land_on_left_windows_gap_and_right() {
        let r = StatusRanges {
            windows: vec![(5, 10, 0), (10, 14, 1)],
            left_end: 5,
            right_start: 30,
        };
        assert_eq!(r.at(0), (L::StatusLeft, None));
        assert_eq!(r.at(4), (L::StatusLeft, None));
        assert_eq!(r.at(5), (L::Status, Some(0)));
        assert_eq!(r.at(13), (L::Status, Some(1)));
        assert_eq!(r.at(14), (L::StatusDefault, None));
        assert_eq!(r.at(30), (L::StatusRight, None));
        // A message over the status line: nothing to click on.
        assert_eq!(StatusRanges::default().at(3), (L::StatusDefault, None));
    }
}
