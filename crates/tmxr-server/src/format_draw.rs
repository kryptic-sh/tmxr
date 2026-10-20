//! A status line drawn from a format, as tmux's format_draw lays one out:
//! its text in left, centre, right and absolute-centre parts by
//! `#[align=…]`, the line filled by `#[fill=…]`, and `#[default]`,
//! `#[push-default]` and `#[pop-default]` against the line's style. A list
//! (`#[list=…]`) is drawn in its place, without tmux's markers for a list
//! too long for its room, and ranges (mouse targets) are not kept.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use tmxr_command::format;

/// Where a run of the line goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Centre,
    Right,
    AbsoluteCentre,
}

/// One run of the line: its text, its style and the part it goes in.
#[derive(Debug, Clone, PartialEq)]
struct Run {
    text: String,
    style: Style,
    align: Align,
}

/// `text` split into runs over `base`, and the style `#[fill=…]` gives the
/// rest of the line.
fn runs(text: &str, base: Style) -> (Vec<Run>, Option<Style>) {
    let mut runs = Vec::new();
    let (mut current, mut default) = (base, base);
    let mut align = Align::Left;
    let mut fill = None;
    // Inside a list's left or right marker: shown by tmux only when the
    // list is too long, so left out here.
    let mut marker = false;
    let mut rest = text;
    loop {
        let (chunk, after) = match rest.find("#[") {
            Some(at) => (&rest[..at], Some(&rest[at + 2..])),
            None => (rest, None),
        };
        if !chunk.is_empty() && !marker {
            runs.push(Run {
                text: chunk.to_owned(),
                style: current,
                align,
            });
        }
        let Some(after) = after else {
            break;
        };
        let Some(end) = after.find(']') else {
            break;
        };
        for item in after[..end].split([',', ' ']).filter(|i| !i.is_empty()) {
            match item {
                "default" | "none" => current = default,
                "push-default" => default = current,
                "pop-default" => default = base,
                "norange" | "nolist" => marker = false,
                "list=on" | "list=focus" => marker = false,
                "list=left-marker" | "list=right-marker" => marker = true,
                _ if item.starts_with("range=") => {}
                _ => {
                    if let Some(a) = item.strip_prefix("align=") {
                        align = match a {
                            "centre" | "center" => Align::Centre,
                            "right" => Align::Right,
                            "absolute-centre" | "absolute-center" => Align::AbsoluteCentre,
                            _ => Align::Left,
                        };
                    } else if let Some(c) = item.strip_prefix("fill=") {
                        let mut s = format::Style::default();
                        s.apply(&format!("bg={c}"));
                        fill = Some(crate::render::apply(base, &s));
                    } else {
                        let mut s = format::Style::default();
                        s.apply(item);
                        current = crate::render::apply(current, &s);
                    }
                }
            }
        }
        rest = &after[end + 1..];
    }
    (runs, fill)
}

fn width(text: &str) -> u16 {
    u16::try_from(Line::raw(text).width()).unwrap_or(u16::MAX)
}

/// Draw `text` (expanded) as one status line at row `y`, `cols` wide, over
/// `base`: the left part from the left edge, the right part against the
/// right edge, the centre part centred between them and the absolute-centre
/// part on the whole line, each cut where it meets what is drawn already.
pub fn draw(buf: &mut Buffer, y: u16, cols: u16, base: Style, text: &str) {
    let (runs, fill) = runs(text, base);
    let line = fill.unwrap_or(base);
    buf.set_style(Rect::new(0, y, cols, 1), line);
    for x in 0..cols {
        if let Some(c) = buf.cell_mut((x, y)) {
            c.set_symbol(" ");
        }
    }
    let part = |align: Align| runs.iter().filter(move |r| r.align == align);
    let part_width = |align: Align| part(align).map(|r| width(&r.text)).sum::<u16>();
    let mut draw_part = |align: Align, from: u16, to: u16| {
        let mut x = from;
        for r in part(align) {
            if x >= to {
                break;
            }
            let (nx, _) = buf.set_stringn(x, y, &r.text, usize::from(to - x), r.style);
            x = nx;
        }
        x
    };
    let left_end = draw_part(Align::Left, 0, cols);
    let right_start = cols.saturating_sub(part_width(Align::Right)).max(left_end);
    draw_part(Align::Right, right_start, cols);
    let room = right_start.saturating_sub(left_end);
    let centre = left_end + room.saturating_sub(part_width(Align::Centre)) / 2;
    draw_part(Align::Centre, centre, right_start);
    let absolute = cols.saturating_sub(part_width(Align::AbsoluteCentre)) / 2;
    draw_part(Align::AbsoluteCentre, absolute.max(left_end), right_start);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    /// `red` and `green`, as tmux's colours 1 and 2.
    const RED: Color = Color::Indexed(1);
    const GREEN: Color = Color::Indexed(2);

    fn text_of(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_owned())
            .collect()
    }

    #[test]
    fn parts_go_left_centre_and_right() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        draw(
            &mut buf,
            0,
            20,
            Style::default(),
            "ab#[align=centre]mid#[align=right]yz",
        );
        assert_eq!(text_of(&buf, 0), "ab      mid       yz");
    }

    #[test]
    fn default_follows_push_and_pop() {
        let base = Style::default().bg(Color::Blue);
        let (runs, _) = runs(
            "#[fg=red]a#[push-default]#[fg=green]b#[default]c#[pop-default]#[default]d",
            base,
        );
        let fg: Vec<_> = runs.iter().map(|r| r.style.fg).collect();
        assert_eq!(fg, [Some(RED), Some(GREEN), Some(RED), None]);
    }

    #[test]
    fn markers_go_and_fill_paints_the_rest() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 1));
        let base = Style::default();
        draw(
            &mut buf,
            0,
            6,
            base,
            "#[fill=red]#[list=on]#[list=left-marker]<#[list=right-marker]>#[list=on]x#[nolist]",
        );
        assert_eq!(text_of(&buf, 0), "x     ");
        assert_eq!(buf[(5, 0)].style().bg, Some(RED));
    }
}
