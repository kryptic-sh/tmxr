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
use crate::overlay::Overlay;
use crate::server::{STATUS_ROWS, Server};
use crate::vars::Vars;

/// `(first column, end column, window index)` of each window in the status
/// line, for mouse clicks.
pub type StatusRanges = Vec<(u16, u16, u32)>;

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
fn draw_runs(buf: &mut Buffer, x: u16, y: u16, max_x: u16, base: Style, text: &str) -> u16 {
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
fn runs_width(text: &str) -> u16 {
    format::styled(text)
        .iter()
        .map(|(_, t)| Line::raw(t.as_str()).width() as u16)
        .sum()
}

/// Draw the client's frame. Returns the status-line window ranges.
pub fn draw(srv: &Server, id: ClientId, term: &mut Terminal<AnsiBackend>) -> StatusRanges {
    let mut ranges = StatusRanges::new();
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
            cursor = draw_window(srv, id, wid, buf, cols, pane_rows);
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
    let mode_style = style_option(&srv.cfg.status.mode_style, &vars);
    let w = win.cols.min(cols);
    let h = win.rows.min(rows);

    let mut cursor = None;
    for (pid, r) in &rects {
        if let Some(c) = draw_pane(srv, *pid, *r, buf, w, h, mode_style)
            && *pid == win.active
        {
            cursor = Some(c);
        }
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
    let active = rects
        .iter()
        .find(|(p, _)| *p == win.active)
        .map(|(_, r)| *r);
    let touches_active = |x: u16, y: u16| {
        active.is_some_and(|r| {
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
            let style = if touches_active(x, y) && rects.len() > 1 {
                active_border
            } else {
                border
            };
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(ch).set_style(style);
            }
        }
    }
    cursor
}

/// Draw one pane. Returns its cursor position on screen when shown.
fn draw_pane(
    srv: &Server,
    pid: PaneId,
    r: hjkl_layout::LayoutRect,
    buf: &mut Buffer,
    max_w: u16,
    max_h: u16,
    mode_style: Style,
) -> Option<Position> {
    let pane = srv.panes.get(&pid)?;
    let w = r.w.min(max_w.saturating_sub(r.x));
    let h = r.h.min(max_h.saturating_sub(r.y));
    if let Some(cm) = &pane.copy {
        for row in 0..h {
            let y = cm.top + usize::from(row);
            let line = cm.lines.get(y);
            for col in 0..w {
                let cell = line.and_then(|l| l.cells.get(usize::from(col)));
                let mut style = cell.map(|c| c.style).unwrap_or_default();
                if cm.selected(y, usize::from(col)) {
                    style = style.patch(mode_style);
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
        let tag = format!("[{offset}/{total}]");
        let tw = tag.len() as u16;
        if tw < w {
            buf.set_string(r.x + w - tw, r.y, &tag, mode_style);
        }
        return None;
    }
    let screen = pane.emu.screen();
    for row in 0..h {
        for col in 0..w {
            let Some(out) = buf.cell_mut((r.x + col, r.y + row)) else {
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
    (cy < h && cx < w).then(|| Position::new(r.x + cx, r.y + cy))
}

fn draw_status(srv: &Server, id: ClientId, buf: &mut Buffer, cols: u16, rows: u16) -> StatusRanges {
    let mut ranges = StatusRanges::new();
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
        ranges.push((start, x, *idx));
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
    match att.overlay.as_ref()? {
        Overlay::Prompt(p) => {
            let before: String = p.input[..p.cursor].iter().collect();
            let x = Line::raw(format!("{}{before}", p.prompt)).width() as u16;
            Some(Position::new(x.min(cols.saturating_sub(1)), y))
        }
        Overlay::Confirm { .. } => None,
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
        Overlay::Picker(p) => {
            let picker = &p.picker;
            let w = cols.saturating_sub(4).clamp(10, 70);
            let list_rows = picker.matched().clamp(1, 15) as u16;
            let h = (list_rows + 3).min(y.saturating_sub(2)).max(4);
            let area = Rect::new(
                (cols.saturating_sub(w)) / 2,
                y.saturating_sub(h) / 2,
                w.min(cols),
                h,
            );
            Clear.render(area, buf);
            let mode = if p.insert { "" } else { " [normal]" };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(border)
                .title(format!(
                    " {} {}/{}{mode} ",
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
            let visible = usize::from(inner.height.saturating_sub(1));
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
                Paragraph::new(Line::from(spans))
                    .render(Rect::new(inner.x, row, inner.width, 1), buf);
            }
            p.insert.then(|| {
                let x = inner.x + 2 + Line::raw(query).width() as u16;
                Position::new(x.min(inner.x + inner.width.saturating_sub(1)), inner.y)
            })
        }
    }
}
