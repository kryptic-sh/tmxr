//! Compose one client's frame: panes, borders, status line, overlays.

use std::fmt::Write as _;

use ratatui::Terminal;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use tmxr_command::format::{self, Colour};
use tmxr_config::{PaneIndicators, PaneLines};

use crate::backend::AnsiBackend;
use crate::copy::cell_style;
use crate::model::{ClientId, PaneId, WindowId};
use crate::overlay::{Overlay, PickerOverlay, Previewed};
use crate::server::Server;
use crate::vars::Vars;

/// A status line as drawn: its rows' cells and click ranges, with where it
/// sat and how wide it was, so a frame drawn only for pane output repeats it.
#[derive(Debug, Clone)]
pub struct StatusCache {
    top: u16,
    cols: u16,
    cells: Vec<Vec<ratatui::buffer::Cell>>,
    ranges: StatusRanges,
}

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
pub(crate) fn apply(base: Style, s: &format::Style) -> Style {
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
        (s.blink, Modifier::SLOW_BLINK),
        (s.hidden, Modifier::HIDDEN),
        (s.strikethrough, Modifier::CROSSED_OUT),
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

/// A menu's or popup's box over `area`, as tmux 3.6's `screen_write_box`:
/// the border drawn with `lines` in `style`, and `title` on the top border
/// from its third cell in the same style (its `#[…]` styles applied),
/// centred there for `#[align=centre]`.
pub(crate) fn draw_box(
    buf: &mut Buffer,
    area: Rect,
    lines: crate::overlay::Lines,
    style: Style,
    title: &str,
) {
    Block::default()
        .borders(Borders::ALL)
        .border_set(lines.border())
        .border_style(style)
        .render(area, buf);
    if title.is_empty() || area.width < 5 {
        return;
    }
    let (left, right) = (area.x + 2, area.x + area.width - 2);
    let centre = title.contains("align=centre") || title.contains("align=center");
    let x = if centre {
        left + (right - left).saturating_sub(runs_width(title)) / 2
    } else {
        left
    };
    draw_runs(buf, x, area.y, right, style, title);
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
    let (sx, sy) = (
        att.cols,
        att.rows.saturating_sub(srv.status_rows(att.session)),
    );
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

/// The cursor shape client `id` shows, as DECSCUSR's `Ps`: what the
/// active pane's program asked for, else its `cursor-style`, as tmux's
/// `tty_update_mode`.
pub fn cursor_style(srv: &Server, id: ClientId) -> u8 {
    let Some(pane) = srv
        .clients
        .get(&id)
        .and_then(|c| c.att.as_ref())
        .and_then(|a| srv.active_pane_of_session(a.session))
    else {
        return 0;
    };
    match srv.panes.get(&pane).map_or(0, |p| p.emu.cursor_style()) {
        0 => srv.pane_options(pane).cursor_style.ps(),
        ps => ps,
    }
}

/// Draw the client's frame. Returns the status-line window ranges.
pub fn draw(
    srv: &Server,
    id: ClientId,
    term: &mut Terminal<AnsiBackend>,
    status: Option<StatusCache>,
) -> (StatusRanges, Option<StatusCache>) {
    let mut ranges = StatusRanges::default();
    let mut drawn = None;
    let Some(att) = srv.clients.get(&id).and_then(|c| c.att.as_ref()) else {
        return (ranges, None);
    };
    let (cols, rows) = (att.cols, att.rows);
    let session = srv.sessions.get(&att.session);
    let window = session.and_then(|s| s.current_window());
    let _ = term.draw(|f| {
        let buf = f.buffer_mut();
        let pane_rows = rows.saturating_sub(srv.status_rows(att.session));
        let top = srv.window_top(att.session);
        let mut cursor = None;
        if let Some(wid) = window {
            cursor = match srv.windows.get(&wid) {
                // Larger than the client, or below a status line at the
                // top: drawn whole, then the part in view copied out.
                Some(win) if win.cols > cols || win.rows > pane_rows || top > 0 => {
                    let (ox, oy) = window_offset(srv, att);
                    let (w, h) = (win.cols.max(cols), win.rows.max(pane_rows));
                    let mut whole = Buffer::empty(Rect::new(0, 0, w, h));
                    let at = draw_window(srv, id, wid, &mut whole, w, h);
                    for y in 0..pane_rows.min(h.saturating_sub(oy)) {
                        for x in 0..cols.min(w.saturating_sub(ox)) {
                            if let (Some(cell), Some(out)) =
                                (whole.cell((x + ox, y + oy)), buf.cell_mut((x, y + top)))
                            {
                                *out = cell.clone();
                            }
                        }
                    }
                    at.and_then(|c| {
                        let (x, y) = (c.x.checked_sub(ox)?, c.y.checked_sub(oy)?);
                        (x < cols && y < pane_rows).then_some(Position::new(x, y + top))
                    })
                }
                _ => draw_window(srv, id, wid, buf, cols, pane_rows),
            };
        }
        (ranges, drawn) = match status.filter(|s| still_fits(srv, att, s, cols, rows)) {
            Some(cache) => {
                put_status(buf, &cache);
                (cache.ranges.clone(), Some(cache))
            }
            None => {
                let ranges = draw_status(srv, id, buf, cols, rows);
                let cache = take_status(srv, att, buf, cols, rows, &ranges);
                (ranges, cache)
            }
        };
        if let Some(c) = draw_overlay(srv, id, buf, cols, rows) {
            cursor = Some(c);
        }
        if let Some(c) = cursor {
            f.set_cursor_position(c);
        }
    });
    (ranges, drawn)
}

/// Where the status line sits on a client `rows` high: its first row and how
/// many it has, none when it is off or the client is too short for it.
fn status_place(srv: &Server, att: &crate::server::Attached, rows: u16) -> Option<(u16, u16)> {
    let lines = drawn_status_rows(srv, att);
    (lines != 0 && rows > lines).then(|| (status_row(srv, att, rows), lines))
}

/// Whether `cache` still fits the status line's place and width.
fn still_fits(
    srv: &Server,
    att: &crate::server::Attached,
    cache: &StatusCache,
    cols: u16,
    rows: u16,
) -> bool {
    cache.cols == cols
        && status_place(srv, att, rows)
            .is_some_and(|(top, lines)| cache.top == top && cache.cells.len() == usize::from(lines))
}

/// The status line just drawn into `buf`, kept for frames drawn for output.
fn take_status(
    srv: &Server,
    att: &crate::server::Attached,
    buf: &Buffer,
    cols: u16,
    rows: u16,
    ranges: &StatusRanges,
) -> Option<StatusCache> {
    let (top, lines) = status_place(srv, att, rows)?;
    let cells = (top..top + lines)
        .map(|y| {
            (0..cols)
                .filter_map(|x| buf.cell((x, y)).cloned())
                .collect()
        })
        .collect();
    Some(StatusCache {
        top,
        cols,
        cells,
        ranges: ranges.clone(),
    })
}

/// Draw a kept status line back into `buf`.
fn put_status(buf: &mut Buffer, cache: &StatusCache) {
    for (dy, row) in (0u16..).zip(&cache.cells) {
        for (x, cell) in (0u16..).zip(row) {
            if let Some(out) = buf.cell_mut((x, cache.top + dy)) {
                *out = cell.clone();
            }
        }
    }
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
    let rects = srv.pane_cells(wid);
    let vars = Vars::for_pane(srv, Some(win.active), Some(client));
    let options = srv.window_options(wid);
    let border = style_option(&options.pane_border_style, &vars);
    let active_border = style_option(&options.pane_active_border_style, &vars);
    let copy = PaneStyles {
        selection: style_option(&options.mode_style, &vars),
        matched: style_option(&options.copy_mode_match_style, &vars),
        current: style_option(&options.copy_mode_current_match_style, &vars),
        clock: style_option(&format!("fg={}", options.clock_mode_colour), &vars),
        clock_style: options.clock_mode_style,
    };
    let w = win.cols.min(cols);
    let h = win.rows.min(rows);

    let mut cursor = None;
    for (pid, cell) in &rects {
        // The pane's own cells: its layout cell less any scrollbar.
        let r = srv.panes.get(pid).map_or(*cell, |p| p.rect);
        if let Some(c) = draw_pane(srv, *pid, r, buf, w, h, &copy)
            && *pid == win.active
        {
            cursor = Some(c);
        }
        // A pane may style its own bar (`set -p pane-scrollbars-style`).
        let scrollbar = style_option(&srv.pane_options(*pid).pane_scrollbars_style, &vars);
        crate::scrollbar::draw(srv, *pid, buf, scrollbar);
    }

    // Borders: every cell of the window area that no pane covers.
    let covered = |x: u16, y: u16| {
        rects
            .iter()
            .any(|(_, r)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
    };
    // A window smaller than the client gets a border on its right and bottom
    // edges, as tmux's screen_redraw_cell_border, and padding beyond.
    let bw = if cols > win.cols { win.cols + 1 } else { w };
    let bh = if rows > win.rows { win.rows + 1 } else { h };
    let edge = |x: u16, y: u16| x == win.cols || y == win.rows;
    let is_border = |x: i32, y: i32| {
        x >= 0
            && y >= 0
            && x < i32::from(bw)
            && y < i32::from(bh)
            && (edge(x as u16, y as u16) || !covered(x as u16, y as u16))
    };
    let rect_of = |pane: PaneId| rects.iter().find(|(p, _)| *p == pane).map(|(_, r)| *r);
    let active = rect_of(win.active);
    // Two panes: their shared border coloured by halves (tmux's
    // screen_redraw_two_panes), with `colour` or `both` indicators.
    let halves = matches!(
        options.pane_border_indicators,
        PaneIndicators::Colour | PaneIndicators::Both
    );
    let splits = match rects.as_slice() {
        [_, (_, second)] if halves => (second.x != 0, second.y != 0),
        _ => (false, false),
    };
    let arrows = matches!(
        options.pane_border_indicators,
        PaneIndicators::Arrows | PaneIndicators::Both
    );
    // The pane a border cell belongs to, as tmux's screen_redraw_check_cell:
    // the first, from the active pane on in order, whose border it is.
    let panes = win.panes();
    let start = panes.iter().position(|p| *p == win.active).unwrap_or(0);
    let owner = |x: u16, y: u16| {
        panes[start..].iter().chain(&panes[..start]).find_map(|p| {
            rect_of(*p)
                .filter(|r| border_side(*r, x, y, splits).is_some())
                .map(|r| (*p, r))
        })
    };
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
    for y in 0..bh {
        for x in 0..bw {
            if covered(x, y) {
                continue;
            }
            // The window's own edge belongs to no pane, as tmux's
            // screen_redraw_check_cell: pane-border-style, no indicators.
            let on_edge = edge(x, y);
            let (xi, yi) = (i32::from(x), i32::from(y));
            let (l, r, u, d) = (
                is_border(xi - 1, yi),
                is_border(xi + 1, yi),
                is_border(xi, yi - 1),
                is_border(xi, yi + 1),
            );
            // tmux's cell types (CELL_TOPBOTTOM …, `tmux.h`), less one.
            let cell = match (l || r, u || d) {
                (true, false) => 1,
                (false, true) | (false, false) => 0,
                (true, true) => match (l, r, u, d) {
                    (true, true, true, true) => 10,
                    (false, true, true, true) => 8,
                    (true, false, true, true) => 9,
                    (true, true, false, true) => 6,
                    (true, true, true, false) => 7,
                    (false, true, false, true) => 2,
                    (true, false, false, true) => 3,
                    (false, true, true, false) => 4,
                    _ => 5,
                },
            };
            let side = active
                .filter(|_| !on_edge)
                .and_then(|a| border_side(a, x, y, splits));
            let owned = if on_edge { None } else { owner(x, y) };
            let number;
            let mut ch = match options.pane_border_lines {
                PaneLines::Number => {
                    // Its pane's index.
                    let index = owned.and_then(|(p, _)| panes.iter().position(|q| *q == p));
                    number = index.map_or_else(
                        || "*".to_owned(),
                        |i| ((i as u32 + options.pane_base_index) % 10).to_string(),
                    );
                    number.as_str()
                }
                lines => border_char(lines, cell),
            };
            // An arrow one cell into the pane whose border it is, pointing
            // at the active pane, as tmux's screen_redraw_draw_borders_cell.
            if arrows
                && let Some(side) = side
                && let Some((_, owner)) = owned
            {
                let along = x == owner.x + 1
                    && (cell == 1
                        || (cell == 6 && side == Side::Bottom)
                        || (cell == 7 && side == Side::Top));
                let down = y == owner.y + 1
                    && (cell == 0
                        || (cell == 8 && side == Side::Right)
                        || (cell == 9 && side == Side::Left));
                if along || down {
                    ch = match side {
                        Side::Left => "→",
                        Side::Right => "←",
                        Side::Top => "↓",
                        Side::Bottom => "↑",
                    };
                }
            }
            let mut style = if side.is_some() && rects.len() > 1 {
                active_border
            } else {
                border
            };
            if !on_edge && touches(marked, x, y) {
                style = style.add_modifier(Modifier::REVERSED);
            }
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(ch).set_style(style);
            }
        }
    }
    // Each pane's status line on its border, as tmux's
    // screen_redraw_make_pane_status: two cells in, its width less two.
    let status = options.pane_border_status;
    if status != tmxr_config::PaneStatus::Off {
        for (pid, cell) in &rects {
            if cell.w < 4 {
                continue;
            }
            let y = match status {
                tmxr_config::PaneStatus::Top => cell.y.checked_sub(1),
                _ => Some(cell.y + cell.h),
            };
            let Some(y) = y.filter(|y| *y < h) else {
                continue;
            };
            let pvars = Vars::for_pane(srv, Some(*pid), Some(client));
            let style = if *pid == win.active {
                active_border
            } else {
                border
            };
            let format = &srv.pane_options(*pid).pane_border_format;
            let text = format::expand_time(format, &pvars);
            let x = cell.x + 2;
            let end = (cell.x + cell.w).min(w);
            if x < end {
                // Laid out as tmux's format_draw, over the border line.
                crate::format_draw::draw_at(buf, (x, y), end - x, style, &text, false);
            }
        }
    }
    // Beyond a window smaller than the client (`resize-window`), as tmux.
    // `fill-character` when it is one character a cell wide, as tmux's
    // window_set_fill_character; else the border lines' outside cell.
    let fill = &options.fill_character;
    let fill_ok = fill.chars().count() == 1 && ratatui::text::Line::raw(fill.as_str()).width() == 1;
    let outside = match options.pane_border_lines {
        _ if fill_ok => fill.as_str(),
        PaneLines::Simple => ".",
        PaneLines::Spaces => " ",
        _ => "·",
    };
    for y in 0..rows {
        for x in 0..cols {
            if (x >= bw || y >= bh)
                && let Some(cell) = buf.cell_mut((x, y))
            {
                cell.set_symbol(outside).set_style(border);
            }
        }
    }
    cursor
}

/// A side of a pane's border.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// Which of `r`'s borders the cell (`x`, `y`) is on, as tmux's
/// screen_redraw_pane_border without pane status lines (a scrollbar is
/// inside the layout cell): with `splits` (two panes side by side, two
/// stacked), only the half of the shared border nearer the pane.
fn border_side(
    r: hjkl_layout::LayoutRect,
    x: u16,
    y: u16,
    (hsplit, vsplit): (bool, bool),
) -> Option<Side> {
    let (ex, ey) = (r.x + r.w, r.y + r.h);
    if x >= r.x && x < ex && y >= r.y && y < ey {
        return None;
    }
    if (r.y == 0 || y + 1 >= r.y) && y <= ey {
        if r.x == 0 && x == r.w && (!hsplit || y <= r.h / 2) {
            return Some(Side::Right);
        }
        if r.x != 0 {
            if x + 1 == r.x && (!hsplit || y > r.h / 2) {
                return Some(Side::Left);
            }
            if x == ex {
                return Some(Side::Right);
            }
        }
    }
    if vsplit {
        if r.y == 0 && y == r.h && x <= r.w / 2 {
            return Some(Side::Bottom);
        }
        if r.y != 0 && y + 1 == r.y && x > r.w / 2 {
            return Some(Side::Top);
        }
    } else if (r.x == 0 || x >= r.x) && x <= ex {
        if r.y != 0 && y + 1 == r.y {
            return Some(Side::Top);
        }
        if y == ey {
            return Some(Side::Bottom);
        }
    }
    None
}

/// The character `lines` draws a border cell of tmux's type `cell` (one
/// less than its `CELL_TOPBOTTOM` … `CELL_JOIN`) with: tmux 3.6's
/// `CELL_BORDERS` (as UTF-8), `tty_acs_double_borders`,
/// `tty_acs_heavy_borders` and `SIMPLE_BORDERS`.
fn border_char(lines: PaneLines, cell: usize) -> &'static str {
    const SINGLE: [&str; 11] = ["│", "─", "┌", "┐", "└", "┘", "┬", "┴", "├", "┤", "┼"];
    const DOUBLE: [&str; 11] = ["║", "═", "╔", "╗", "╚", "╝", "╦", "╩", "╠", "╣", "╬"];
    const HEAVY: [&str; 11] = ["┃", "━", "┏", "┓", "┗", "┛", "┳", "┻", "┣", "┫", "╋"];
    const SIMPLE: [&str; 11] = ["|", "-", "+", "+", "+", "+", "+", "+", "+", "+", "+"];
    let set = match lines {
        PaneLines::Double => &DOUBLE,
        PaneLines::Heavy => &HEAVY,
        PaneLines::Simple => &SIMPLE,
        PaneLines::Spaces => return " ",
        PaneLines::Single | PaneLines::Number => &SINGLE,
    };
    set[cell]
}

/// Draw one pane. Returns its cursor position on screen when shown.
/// How a pane's modes draw: copy mode's selection and search matches, and
/// clock mode's digits.
struct PaneStyles {
    selection: Style,
    matched: Style,
    current: Style,
    clock: Style,
    clock_style: tmxr_config::ClockModeStyle,
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
    let base = pane_default_style(srv, pid);
    if pane.clock {
        draw_clock(buf, Rect::new(r.x, r.y, w, h), copy.clock, copy.clock_style);
        return None;
    }
    if let Some(cm) = &pane.copy {
        for row in 0..h {
            let y = cm.top + usize::from(row);
            let line = cm.lines.get(y);
            let matches = cm.match_spans(y);
            for col in 0..w {
                let x = usize::from(col);
                let cell = line.and_then(|l| l.cell(x));
                let mut style =
                    with_default_colours(cell.map(|c| c.style).unwrap_or_default(), base);
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
                let sym = cell.map_or(" ", |c| if c.symbol.is_empty() { " " } else { &c.symbol });
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
    draw_screen(pane.emu.screen(), buf, Rect::new(r.x, r.y, w, h), base)
}

/// The colours pane `pid`'s cells take where its program set none, as
/// tmux's tty_default_colours: `window-active-style`'s for its window's
/// active pane, where that sets a colour, else `window-style`'s. Only the
/// foreground and background.
fn pane_default_style(srv: &Server, pid: PaneId) -> Style {
    let colours = pane_default_colours(srv, pid);
    let fg = colours.fg.map(colour);
    let bg = colours.bg.map(colour);
    Style {
        fg,
        bg,
        ..Style::default()
    }
}

/// [`pane_default_style`]'s colours as tmux's: none where it is the
/// terminal's default (`#{pane_fg}` / `#{pane_bg}`).
pub(crate) fn pane_default_colours(srv: &Server, pid: PaneId) -> format::Style {
    let Some(pane) = srv.panes.get(&pid) else {
        return format::Style::default();
    };
    let options = srv.pane_options(pid);
    let vars = Vars::for_pane(srv, Some(pid), None);
    let parse = |spec: &str| {
        let mut s = format::Style::default();
        s.apply(&format::expand(spec, &vars));
        s
    };
    let inactive = parse(&options.window_style);
    let active = srv
        .windows
        .get(&pane.window)
        .is_some_and(|w| w.active == pid)
        .then(|| parse(&options.window_active_style));
    let set = |c: Option<Colour>| c.filter(|c| *c != Colour::Default);
    format::Style {
        fg: active.and_then(|a| set(a.fg)).or_else(|| set(inactive.fg)),
        bg: active.and_then(|a| set(a.bg)).or_else(|| set(inactive.bg)),
        ..format::Style::default()
    }
}

/// `style` with `base`'s colours where it has the terminal's default.
fn with_default_colours(style: Style, base: Style) -> Style {
    let unset = |c: Option<Color>| c.is_none_or(|c| c == Color::Reset);
    Style {
        fg: if unset(style.fg) {
            base.fg.or(style.fg)
        } else {
            style.fg
        },
        bg: if unset(style.bg) {
            base.bg.or(style.bg)
        } else {
            style.bg
        },
        ..style
    }
}

/// Draw a program's screen into `area`. Returns where its cursor is, if
/// shown and inside the area.
fn draw_screen(
    screen: &vt100::Screen,
    buf: &mut Buffer,
    area: Rect,
    base: Style,
) -> Option<Position> {
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
                    out.set_symbol(sym)
                        .set_style(with_default_colours(cell_style(c), base));
                }
                None => {
                    out.set_symbol(" ").set_style(base);
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
    let ranges = StatusRanges::default();
    let Some(att) = srv.clients.get(&id).and_then(|c| c.att.as_ref()) else {
        return ranges;
    };
    // `status off`: no line, and nothing on it to click.
    let lines = drawn_status_rows(srv, att);
    if lines == 0 || rows < lines + 1 {
        return ranges;
    }
    let y = status_row(srv, att, rows);
    let Some(session) = srv.sessions.get(&att.session) else {
        return ranges;
    };
    let active = srv.active_pane_of_session(session.id);
    let vars = Vars::for_pane(srv, active, Some(id));
    let opts = srv.session_options(session.id);
    // status-fg / status-bg over status-style unless `default`, as tmux's
    // status_redraw.
    let mut spec = opts.status_style.clone();
    for (side, colour) in [("fg", &opts.status_fg), ("bg", &opts.status_bg)] {
        if &**colour != "default" {
            let _ = write!(spec, ",{side}={colour}");
        }
    }
    let base = style_option(&spec, &vars);
    // The lines after the first, each its status-format item expanded and
    // laid out as tmux's format_draw; they stay under a message or prompt.
    let item = |line: u16| {
        tmxr_config::OptionValue::items(&opts.status_format)
            .and_then(|items| items.get(&u32::from(line)))
            .map_or("", String::as_str)
    };
    for line in 1..lines {
        let text = format::expand_time(item(line), &vars);
        crate::format_draw::draw(buf, y + line, cols, base, &text);
    }
    clear_row(buf, y, cols, base);

    // A message or prompt takes message-line's line (the last there is if
    // fewer), as tmux's: over the first, it is all that line shows.
    let ask_row = y + opts.message_line.line().min(lines.saturating_sub(1));
    if draw_ask(buf, att, &opts, &vars, ask_row, cols) && ask_row == y {
        return ranges;
    }

    // The first line drawn from its format, tmux's default unless set, its
    // clicks from its ranges.
    let text = format::expand_time(item(0), &vars);
    status_ranges(&crate::format_draw::draw(buf, y, cols, base, &text))
}

/// `row` cleared to blanks in `style`.
fn clear_row(buf: &mut Buffer, row: u16, cols: u16, style: Style) {
    buf.set_style(Rect::new(0, row, cols, 1), style);
    for x in 0..cols {
        if let Some(c) = buf.cell_mut((x, row)) {
            c.set_symbol(" ");
        }
    }
}

/// `att`'s message, prompt or y/n question drawn on `row`, if it has one:
/// a `display-message` text laid out by its styles as tmux's, any other as
/// it is, in message-style (message-command-style in vi's command mode).
fn draw_ask(
    buf: &mut Buffer,
    att: &crate::server::Attached,
    opts: &tmxr_config::SessionOptions,
    vars: &Vars<'_>,
    row: u16,
    cols: u16,
) -> bool {
    let message_style = style_option(&opts.message_style, vars);
    let (style, text, styled) = match (&att.message, &att.overlay) {
        (Some((msg, _)), _) => (message_style, msg.clone(), att.message_styled),
        (None, Some(Overlay::Prompt(p))) => {
            let style = if p.command_mode {
                style_option(&opts.message_command_style, vars)
            } else {
                message_style
            };
            let text = format!("{}{}", p.prompt, p.input.iter().collect::<String>());
            (style, text, false)
        }
        (None, Some(Overlay::Confirm { prompt, .. })) => (message_style, prompt.clone(), false),
        _ => return false,
    };
    if styled {
        crate::format_draw::draw(buf, row, cols, style, &text);
    } else {
        clear_row(buf, row, cols, style);
        buf.set_stringn(0, row, &text, usize::from(cols), style);
    }
    true
}

/// What clicks reach on a line `format_draw` drew, from its ranges: the
/// left part up to the last left range's end, the right part from the
/// first right range's start, and each window's cells.
fn status_ranges(spans: &[(u16, u16, crate::format_draw::Range)]) -> StatusRanges {
    use crate::format_draw::Range;
    let mut ranges = StatusRanges::default();
    for &(start, end, range) in spans {
        match range {
            Range::Left => ranges.left_end = ranges.left_end.max(end),
            Range::Right => ranges.right_start = ranges.right_start.min(start),
            Range::Window(idx) => match ranges.windows.last_mut() {
                Some(last) if last.2 == idx && last.1 == start => last.1 = end,
                _ => ranges.windows.push((start, end, idx)),
            },
            Range::None => {}
        }
    }
    ranges
}

/// The rows the status line takes on `att`'s screen: its session's, or
/// one over the window's last row while a message or prompt shows with
/// `status off`, as tmux's screen_redraw_set_context.
fn drawn_status_rows(srv: &Server, att: &crate::server::Attached) -> u16 {
    let lines = srv.status_rows(att.session);
    let asking = matches!(
        att.overlay,
        Some(Overlay::Prompt(_) | Overlay::Confirm { .. })
    );
    if att.message.is_some() || asking {
        lines.max(1)
    } else {
        lines
    }
}

/// The client row `att`'s status line, message or prompt is drawn on: the
/// first with `status-position top`, else the first of the bottom rows it
/// takes.
fn status_row(srv: &Server, att: &crate::server::Attached, rows: u16) -> u16 {
    if srv.status_at_top(att.session) {
        0
    } else {
        rows.saturating_sub(drawn_status_rows(srv, att))
    }
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
    let opts = srv.client_window_options(id);
    let mode_style = style_option(&opts.mode_style, &vars);
    let border = style_option(&opts.pane_active_border_style, &vars);
    // The window's rows, top..y.
    let top = srv.window_top(att.session);
    let y = top + rows.saturating_sub(srv.status_rows(att.session));
    // A menu's or popup's own style (-s, -S, -H), else the option's.
    let look = |spec: &Option<String>, option: &str| {
        style_option(spec.as_deref().unwrap_or(option), &vars)
    };
    let menu_styles = |l: &crate::overlay::BoxLook| {
        (
            look(&l.style, &opts.menu_style),
            look(&l.border_style, &opts.menu_border_style),
            look(&l.selected_style, &opts.menu_selected_style),
        )
    };
    match att.overlay.as_ref()? {
        Overlay::Prompt(p) => {
            // Tab's menu: its names under the word, against the status line.
            if let Some(open) = &p.menu {
                let mut menu = open.menu.clone();
                menu.look.lines = crate::overlay::Lines::of(opts.menu_border_lines);
                let lead: String = p.input[..open.word.0].iter().collect();
                let word_x = Line::raw(format!("{}{lead}{}", p.prompt, open.prefix)).width() as u16;
                let row = status_row(srv, att, rows);
                let (_, h) = menu.size();
                let top_row = if srv.status_at_top(att.session) {
                    row + 1
                } else {
                    row.saturating_sub(h)
                };
                menu.at = Some((word_x.saturating_sub(2), top_row));
                menu.draw(buf, cols, rows, menu_styles(&menu.look));
            }
            let before: String = p.input[..p.cursor].iter().collect();
            let x = Line::raw(format!("{}{before}", p.prompt)).width() as u16;
            let y = status_row(srv, att, rows);
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
            let opts = srv.session_options(att.session);
            let other = label_style(&opts.display_panes_colour);
            let active = label_style(&opts.display_panes_active_colour);
            for (pid, r) in srv.pane_cells(win.id) {
                let Some((label, _)) = labels.iter().find(|(_, p)| *p == pid) else {
                    continue;
                };
                let text = format!(" {label} ");
                let w = text.len() as u16;
                let x = r.x + r.w.saturating_sub(w) / 2;
                let row = top + r.y + r.h / 2;
                if row < y {
                    let style = if pid == win.active { active } else { other };
                    buf.set_stringn(x, row, &text, usize::from(r.w), style);
                }
            }
            None
        }
        Overlay::Text { lines, top: first } => {
            let area = Rect::new(0, top, cols, y - top);
            Clear.render(area, buf);
            let shown = lines.iter().skip(*first).take(usize::from(y - top));
            for (i, line) in shown.enumerate() {
                buf.set_stringn(0, top + i as u16, line, usize::from(cols), Style::default());
            }
            let tag = format!("[{}/{}]", first + 1, lines.len());
            let tw = tag.len() as u16;
            if tw < cols {
                buf.set_string(cols - tw, top, &tag, mode_style);
            }
            None
        }
        Overlay::Picker(p) => draw_picker(srv, p, buf, cols, (top, y), border, mode_style),
        Overlay::Menu(m) => {
            m.draw(buf, cols, rows, menu_styles(&m.look));
            None
        }
        Overlay::Popup(p) => {
            let area = p.rect.intersection(Rect::new(0, 0, cols, rows));
            Clear.render(area, buf);
            buf.set_style(area, look(&p.look.style, &opts.popup_style));
            if p.border {
                let lines = p.look.lines.unwrap_or_default();
                let style = look(&p.look.border_style, &opts.popup_border_style);
                draw_box(buf, area, lines, style, &p.title);
            }
            let inner = p.inner().intersection(area);
            let cursor = draw_screen(p.emu.screen(), buf, inner, Style::default());
            cursor.filter(|_| !p.exited)
        }
    }
}

/// Most rows the picker list takes when a preview sits under it.
const PICKER_LIST_ROWS: u16 = 15;
/// The title client `id`'s terminal gets: `set-titles-string` for its
/// session's active pane, when that session has `set-titles` on. Control
/// characters are dropped, so a program's title cannot carry escape
/// sequences to the terminal.
pub fn title(srv: &Server, id: ClientId) -> Option<String> {
    let session = srv.clients.get(&id)?.att.as_ref()?.session;
    let opts = srv.session_options(session);
    if !opts.set_titles {
        return None;
    }
    let vars = Vars::for_pane(srv, srv.active_pane_of_session(session), Some(id));
    let title = format::expand_time(&opts.set_titles_string, &vars);
    Some(title.chars().filter(|c| !c.is_control()).collect())
}

/// Fewest rows worth giving a preview.
const PREVIEW_MIN_ROWS: u16 = 3;

/// The picker overlay: query row, matches, and for sessions and windows a
/// preview of the highlighted entry's active pane, centred in the window's
/// rows `top..y`. Returns the query cursor while typing.
fn draw_picker(
    srv: &Server,
    p: &PickerOverlay,
    buf: &mut Buffer,
    cols: u16,
    (top, y): (u16, u16),
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
    let room = y.saturating_sub(top).saturating_sub(2);
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
        top + y.saturating_sub(top).saturating_sub(h) / 2,
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
    if !p.typing() {
        return None;
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
const CLOCK_FONT: [[&str; 5]; 14] = [
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
    ["#####", "#...#", "#####", "#...#", "#...#"], // A
    ["#####", "#...#", "#####", "#....", "#...."], // P
    ["#...#", "##.##", "#.#.#", "#...#", "#...#"], // M
];

/// Clock mode: the local time in big digits in the middle of the pane, or as
/// plain text when the pane is too small for them, laid out as tmux's
/// `window_clock_draw_screen`.
fn draw_clock(buf: &mut Buffer, area: Rect, colour: Style, style: tmxr_config::ClockModeStyle) {
    Clear.render(area, buf);
    let Some((h, m, s)) = tmxr_command::localtime::hour_minute_second() else {
        return;
    };
    let text = style.text(h, m, s);
    let len = text.chars().count() as u16;
    if area.width < 6 * len || area.height < 6 {
        if area.width >= len && area.height != 0 {
            let x = area.x + area.width / 2 - len / 2;
            let y = area.y + area.height / 2;
            buf.set_stringn(x, y, &text, usize::from(area.width), colour);
        }
        return;
    }
    // A blank (the 12-hour clock's padding and gap) takes a glyph's room.
    let glyphs: Vec<Option<usize>> = text
        .chars()
        .map(|c| match c {
            '0'..='9' => c.to_digit(10).map(|d| d as usize),
            ':' => Some(10),
            'A' => Some(11),
            'P' => Some(12),
            'M' => Some(13),
            _ => None,
        })
        .collect();
    let paint = Style::default().bg(colour.fg.unwrap_or(Color::Blue));
    let x0 = area.x + area.width / 2 - 3 * len;
    let y0 = area.y + area.height / 2 - 3;
    for (i, g) in glyphs.iter().enumerate() {
        let Some(g) = g else {
            continue;
        };
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
    fn style_attributes_reach_the_terminal() {
        let mut st = format::Style::default();
        st.apply("strikethrough,blink,hidden");
        let out = apply(Style::default(), &st);
        for m in [
            Modifier::CROSSED_OUT,
            Modifier::SLOW_BLINK,
            Modifier::HIDDEN,
        ] {
            assert!(out.add_modifier.contains(m), "{m:?}");
        }
        st.apply("noblink");
        assert!(apply(out, &st).sub_modifier.contains(Modifier::SLOW_BLINK));
    }

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
