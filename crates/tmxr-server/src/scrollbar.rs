//! tmux's pane scrollbars (`pane-scrollbars`, tmux 3.5), transcribed from
//! tmux 3.6: a column `width` cells wide on a pane's right or left, `pad`
//! blank cells between it and the pane, its slider showing where the view
//! sits in the pane's history. The pane's program gets what is left of its
//! layout cell (`layout_fix_panes`).

use hjkl_layout::LayoutRect;
use ratatui::buffer::Buffer;
use ratatui::style::Style;
use tmxr_command::MouseLocation;
use tmxr_config::{ScrollbarPosition, Scrollbars};

use crate::model::{Pane, PaneId};
use crate::server::Server;

/// A pane's scrollbar: its side, width and padding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bar {
    pub left: bool,
    pub width: u16,
    pub pad: u16,
}

/// `width=` and `pad=` from `pane-scrollbars-style`: 1 and 0 when not
/// given, and never a width under 1, as tmux's.
pub fn size(style: &str) -> (u16, u16) {
    let (mut width, mut pad) = (1, 0);
    for item in style.split([',', ' ']) {
        if let Some(n) = item
            .strip_prefix("width=")
            .and_then(|n| n.parse::<u16>().ok())
        {
            width = n.max(1);
        } else if let Some(n) = item.strip_prefix("pad=").and_then(|n| n.parse().ok()) {
            pad = n;
        }
    }
    (width, pad)
}

/// The bar `pane` shows, if any (tmux's `window_pane_show_scrollbar`):
/// always with `on`, in copy mode with `modal`, and never while its program
/// uses the alternate screen.
pub fn bar(srv: &Server, pane: PaneId) -> Option<Bar> {
    let p = srv.panes.get(&pane)?;
    let show = match srv.cfg.pane_scrollbars {
        Scrollbars::Off => false,
        Scrollbars::Modal => p.copy.is_some(),
        Scrollbars::On => true,
    };
    if !show || p.emu.screen().alternate_screen() {
        return None;
    }
    let (width, pad) = size(&srv.cfg.pane_scrollbars_style);
    Some(Bar {
        left: srv.cfg.pane_scrollbars_position == ScrollbarPosition::Left,
        width,
        pad,
    })
}

/// What a pane's program gets of its layout `cell` beside `bar`: tmux's
/// `layout_fix_panes`, which keeps the pane at least a column wide.
pub fn content(cell: LayoutRect, bar: Option<Bar>) -> LayoutRect {
    let Some(b) = bar else {
        return cell;
    };
    let mut r = cell;
    let side = b.width + b.pad;
    if b.left {
        if cell.w <= b.width {
            r.x = cell.x + cell.w.saturating_sub(1);
            r.w = 1;
        } else {
            r.w = cell.w.saturating_sub(side).max(1);
            r.x = cell.x + cell.w - r.w;
        }
    } else if cell.w <= side {
        r.w = 1;
    } else {
        r.w = cell.w - side;
    }
    r
}

/// The slider in a bar `height` rows high: its top row and its rows (tmux's
/// `screen_redraw_draw_pane_scrollbar`). Outside copy mode it sits at the
/// bottom, as tall as the screen's share of screen and history; in copy
/// mode it follows the view.
pub fn slider(p: &Pane, height: u16) -> (u16, u16) {
    let h = f64::from(height);
    let (top, rows) = match &p.copy {
        None => {
            let total = h + p.emu.history_size() as f64;
            let rows = (h * (h / total)) as u16;
            (height.saturating_sub(rows), rows)
        }
        Some(cm) => {
            let history = cm.lines.len().saturating_sub(usize::from(cm.rows)) as f64;
            let total = history + h;
            let rows = (h * (h / total)) as u16;
            let top = ((h + 1.0) * (cm.top as f64 / total)) as u16;
            (top, rows)
        }
    };
    let rows = rows.max(1);
    let top = if top >= height {
        height.saturating_sub(1)
    } else {
        top
    };
    (top, rows)
}

/// Where in `pane`'s bar the window cell `(col, row)` is (tmux's
/// `server_client_check_mouse_in_pane`): above the slider, on it (with the
/// row it was grabbed at, counted from the slider's top) or below it.
/// `None` outside the bar, its padding included.
pub fn hit(srv: &Server, pane: PaneId, col: u16, row: u16) -> Option<(MouseLocation, u16)> {
    let b = bar(srv, pane)?;
    let p = srv.panes.get(&pane)?;
    let r = p.rect;
    let x = if b.left {
        r.x.checked_sub(b.pad + b.width)?
    } else {
        r.x + r.w + b.pad
    };
    if col < x || col >= x + b.width || row < r.y || row >= r.y + r.h {
        return None;
    }
    let (top, rows) = slider(p, r.h);
    let at = row - r.y;
    Some(if at < top {
        (MouseLocation::ScrollbarUp, 0)
    } else if at < top + rows {
        (MouseLocation::ScrollbarSlider, at - top)
    } else {
        (MouseLocation::ScrollbarDown, 0)
    })
}

/// Draw `pane`'s bar beside it in `buf`, which is in window cells: the bar
/// in `style`, the slider with its colours swapped, the padding blank.
pub fn draw(srv: &Server, pane: PaneId, buf: &mut Buffer, style: Style) {
    let (Some(b), Some(p)) = (bar(srv, pane), srv.panes.get(&pane)) else {
        return;
    };
    let r = p.rect;
    let (top, rows) = slider(p, r.h);
    let slider_style = Style {
        fg: style.bg,
        bg: style.fg,
        ..style
    };
    let x0 = if b.left {
        r.x.saturating_sub(b.width + b.pad)
    } else {
        r.x + r.w
    };
    for i in 0..b.width + b.pad {
        let pad = if b.left { i >= b.width } else { i < b.pad };
        for j in 0..r.h {
            let Some(cell) = buf.cell_mut((x0 + i, r.y + j)) else {
                continue;
            };
            cell.reset();
            if !pad {
                let on_slider = j >= top && j < top + rows;
                cell.set_style(if on_slider { slider_style } else { style });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_style_gives_width_and_pad_as_tmuxs() {
        assert_eq!(size("bg=black,fg=white,width=1,pad=0"), (1, 0));
        assert_eq!(size("width=3,pad=2"), (3, 2));
        // Unset: 1 and 0; a width under 1 is 1.
        assert_eq!(size("bg=red"), (1, 0));
        assert_eq!(size("width=0"), (1, 0));
    }

    #[test]
    fn a_pane_keeps_a_column_beside_its_bar() {
        let cell = LayoutRect::new(10, 0, 20, 5);
        let right = Bar {
            left: false,
            width: 2,
            pad: 1,
        };
        let left = Bar {
            left: true,
            ..right
        };
        assert_eq!(content(cell, None), cell);
        assert_eq!(content(cell, Some(right)), LayoutRect::new(10, 0, 17, 5));
        assert_eq!(content(cell, Some(left)), LayoutRect::new(13, 0, 17, 5));
        // Too narrow for the bar: one column, as tmux's PANE_MINIMUM.
        let narrow = LayoutRect::new(10, 0, 2, 5);
        assert_eq!(content(narrow, Some(right)), LayoutRect::new(10, 0, 1, 5));
        assert_eq!(content(narrow, Some(left)), LayoutRect::new(11, 0, 1, 5));
    }
}
