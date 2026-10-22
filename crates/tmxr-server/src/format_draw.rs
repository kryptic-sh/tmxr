//! A status line drawn from a format, as tmux's format_draw lays one out:
//! its text in left, centre, right and absolute-centre parts by
//! `#[align=…]`, the line filled by `#[fill=…]`, and `#[default]`,
//! `#[push-default]` and `#[pop-default]` against the line's style. A list
//! (`#[list=…]`) goes where its alignment puts it, cut down around its focus
//! with its markers when it is too long for its room, the text after it
//! (`#[nolist]`) beside it. The left, right and window ranges (mouse
//! targets) are kept; pane, session and user ranges are not.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use tmxr_command::format;

/// An `#[align=…]`; `Default` until one is given, which a list opened
/// under it never leaves, so tmux draws no such list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Align {
    Default,
    Left,
    Centre,
    Right,
    AbsoluteCentre,
}

/// Which of tmux's screens a run is drawn into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Left,
    Centre,
    Right,
    AbsoluteCentre,
    List,
    ListLeft,
    ListRight,
    After,
}

/// What a click on a run reaches, as tmux's `#[range=…]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Range {
    None,
    Left,
    Right,
    /// A pane by id (`range=pane|%1`).
    Pane(u32),
    /// A window by index (`range=window|1`).
    Window(u32),
    /// A session by id (`range=session|$1`).
    Session(u32),
    /// Text of the config's own (`range=user|text`), for
    /// `#{mouse_status_range}`: cut to [`USER_RANGE_MAX`] bytes, as tmux's.
    User(String),
}

/// The most bytes of a user range's text kept, as tmux's `range_string`.
pub const USER_RANGE_MAX: usize = 15;

impl Range {
    /// What `range=` names (its text after `range=`), as tmux's style_parse
    /// reads it; `None` where it reads none.
    fn parse(spec: &str) -> Option<Self> {
        let (kind, arg) = match spec.split_once('|') {
            Some((kind, arg)) => (kind, Some(arg)),
            None => (spec, None),
        };
        let id = |prefix: char| {
            let digits = arg?.strip_prefix(prefix)?;
            digits
                .bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| digits.parse().ok())?
        };
        Some(match kind.to_ascii_lowercase().as_str() {
            "left" if arg.is_none() => Self::Left,
            "right" if arg.is_none() => Self::Right,
            "pane" => Self::Pane(id('%')?),
            "window" => {
                let digits = arg?;
                Self::Window(
                    digits
                        .bytes()
                        .all(|b| b.is_ascii_digit())
                        .then(|| digits.parse().ok())??,
                )
            }
            "session" => Self::Session(id('$')?),
            "user" => {
                let text = arg?;
                let cut = (0..=USER_RANGE_MAX.min(text.len()))
                    .rev()
                    .find(|&i| text.is_char_boundary(i))
                    .unwrap_or(0);
                Self::User(text[..cut].to_owned())
            }
            _ => return None,
        })
    }
}

/// One run of the line: its text, its style, the part it goes in and its
/// range.
#[derive(Debug, Clone, PartialEq)]
struct Run {
    text: String,
    style: Style,
    part: Part,
    range: Range,
}

/// How the runs are laid out: where the list goes and the columns of its
/// focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ListLayout {
    align: Align,
    focus: (u16, u16),
}

/// The part text under `align` goes in before a list has ended.
fn part_of(align: Align) -> Part {
    match align {
        Align::Default | Align::Left => Part::Left,
        Align::Centre => Part::Centre,
        Align::Right => Part::Right,
        Align::AbsoluteCentre => Part::AbsoluteCentre,
    }
}

/// Where a list is in tmux's format_draw: not yet opened, in it, or ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListState {
    Never,
    In,
    Ended,
}

/// `text` split into runs over `base`, the style `#[fill=…]` gives the rest
/// of the line, and where the list goes, as tmux's format_draw sorts them.
fn runs(text: &str, base: Style) -> (Vec<Run>, Option<Style>, ListLayout) {
    let mut runs = Vec::new();
    let (mut current, mut default) = (base, base);
    let mut align = Align::Default;
    let mut list = ListMode::Off;
    let mut range = Range::None;
    let mut fill = None;
    let mut state = ListState::Never;
    let mut list_align = Align::Default;
    // Text under an alignment goes to its part, or after the list once
    // the list under it has ended.
    let mut after: Vec<Align> = Vec::new();
    let mut part = Part::Left;
    let mut list_width = 0u16;
    let mut have_marker = (false, false);
    let (mut focus_start, mut focus_end) = (None, None);
    for piece in format::draw_pieces(text) {
        let spec = match piece {
            format::Piece::Text(text) => {
                if part == Part::List {
                    list_width = list_width.saturating_add(width(&text));
                }
                match part {
                    Part::ListLeft => have_marker.0 = true,
                    Part::ListRight => have_marker.1 = true,
                    _ => {}
                }
                runs.push(Run {
                    text,
                    style: current,
                    part,
                    range: range.clone(),
                });
                continue;
            }
            format::Piece::Style(spec) => spec,
        };
        for item in spec.split([',', ' ']).filter(|i| !i.is_empty()) {
            match item {
                "default" | "none" => current = default,
                "push-default" => default = current,
                "pop-default" => default = base,
                "norange" => range = Range::None,
                "nolist" => list = ListMode::Off,
                "list=on" => list = ListMode::On,
                "list=focus" => list = ListMode::Focus,
                "list=left-marker" => list = ListMode::LeftMarker,
                "list=right-marker" => list = ListMode::RightMarker,
                _ if item.len() > 6 && item[..6].eq_ignore_ascii_case("range=") => {
                    range = Range::parse(&item[6..]).unwrap_or(Range::None);
                }
                _ => {
                    if let Some(a) = item.strip_prefix("align=") {
                        align = match a {
                            "left" => Align::Left,
                            "centre" | "center" => Align::Centre,
                            "right" => Align::Right,
                            "absolute-centre" | "absolute-center" => Align::AbsoluteCentre,
                            _ => align,
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
        // The list state after this style, as tmux's format_draw checks it.
        match list {
            ListMode::On => {
                if state != ListState::In {
                    state = ListState::In;
                    list_align = align;
                }
                if focus_start.is_some() && focus_end.is_none() {
                    focus_end = Some(list_width);
                }
                part = Part::List;
            }
            ListMode::Focus => {
                if state == ListState::In && focus_start.is_none() {
                    focus_start = Some(list_width);
                }
            }
            ListMode::Off => {
                if state == ListState::In {
                    if focus_start.is_some() && focus_end.is_none() {
                        focus_end = Some(list_width);
                    }
                    after.push(list_align);
                    if list_align == Align::Left {
                        after.push(Align::Default);
                    }
                    state = ListState::Ended;
                }
                part = if after.contains(&align) {
                    Part::After
                } else {
                    part_of(align)
                };
            }
            ListMode::LeftMarker | ListMode::RightMarker => {
                let left = list == ListMode::LeftMarker;
                let had = if left { have_marker.0 } else { have_marker.1 };
                if state == ListState::In && !had {
                    if focus_start.is_some() && focus_end.is_none() {
                        (focus_start, focus_end) = (None, None);
                    }
                    part = if left {
                        Part::ListLeft
                    } else {
                        Part::ListRight
                    };
                }
            }
        }
    }
    if focus_start.is_some() && focus_end.is_none() {
        focus_end = Some(list_width);
    }
    let focus = match (focus_start, focus_end) {
        (Some(s), Some(e)) => (s, e),
        _ => (0, 0),
    };
    let layout = ListLayout {
        align: list_align,
        focus,
    };
    (runs, fill, layout)
}

/// A style's `list=` state, which later styles keep until one changes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListMode {
    Off,
    On,
    Focus,
    LeftMarker,
    RightMarker,
}

fn width(text: &str) -> u16 {
    u16::try_from(Line::raw(text).width()).unwrap_or(u16::MAX)
}

/// One part drawn on a line of its own, and the ranges in it.
struct Screen {
    buf: Buffer,
    width: u16,
    ranges: Vec<(u16, u16, Range)>,
}

impl Screen {
    fn of(runs: &[Run], part: Part) -> Self {
        let mine: Vec<&Run> = runs.iter().filter(|r| r.part == part).collect();
        let total = mine
            .iter()
            .map(|r| width(&r.text))
            .fold(0u16, u16::saturating_add);
        let mut buf = Buffer::empty(Rect::new(0, 0, total.max(1), 1));
        let mut ranges = Vec::new();
        let mut x = 0;
        for r in mine {
            let (nx, _) = buf.set_stringn(x, 0, &r.text, usize::from(total - x), r.style);
            if r.range != Range::None && nx > x {
                ranges.push((x, nx, r.range.clone()));
            }
            x = nx;
        }
        Self {
            buf,
            width: total,
            ranges,
        }
    }
}

/// The line being drawn: its buffer, row, width and the ranges placed.
struct Target<'a> {
    buf: &'a mut Buffer,
    /// The line's first column on the buffer.
    x0: u16,
    y: u16,
    cols: u16,
    spans: Vec<(u16, u16, Range)>,
}

impl Target<'_> {
    /// `width` columns of `s` from `start` drawn at `offset`, as tmux's
    /// format_draw_put, with its ranges cut to them.
    fn put(&mut self, s: &Screen, offset: u16, start: u16, width: u16) {
        for i in 0..width {
            let (x, from) = (offset.saturating_add(i), start.saturating_add(i));
            if x >= self.cols || from >= s.width {
                break;
            }
            if let Some(cell) = self.buf.cell_mut((self.x0 + x, self.y)) {
                *cell = s.buf[(from, 0)].clone();
            }
        }
        let end = start.saturating_add(width);
        for (a, b, range) in &s.ranges {
            let (a, b) = (*a, *b);
            if b <= start || a >= end {
                continue;
            }
            let (a, b) = (a.max(start) - start + offset, b.min(end) - start + offset);
            let (a, b) = (a.min(self.cols), b.min(self.cols));
            if b > a {
                self.spans.push((self.x0 + a, self.x0 + b, range.clone()));
            }
        }
    }

    /// The list in `width` columns at `offset`, cut down around its focus
    /// with its markers when it is too long, as tmux's
    /// format_draw_put_list.
    fn put_list(&mut self, parts: &Parts, mut offset: u16, mut width: u16, focus: (u16, u16)) {
        let (list, left, right) = (&parts.list, &parts.list_left, &parts.list_right);
        if width >= list.width {
            self.put(list, offset, 0, width);
            return;
        }
        let centre = focus.0 + (focus.1.saturating_sub(focus.0)) / 2;
        let mut start = centre.saturating_sub(width / 2);
        if start + width > list.width {
            start = list.width - width;
        }
        if start != 0 && width > left.width {
            self.put(left, offset, 0, left.width);
            offset += left.width;
            start += left.width;
            width -= left.width;
        }
        if start + width < list.width && width > right.width {
            self.put(right, offset + width - right.width, 0, right.width);
            width -= right.width;
        }
        self.put(list, offset, start, width);
    }
}

/// Every part of the line, drawn.
struct Parts {
    left: Screen,
    centre: Screen,
    right: Screen,
    absolute: Screen,
    list: Screen,
    list_left: Screen,
    list_right: Screen,
    after: Screen,
}

/// Draw `text` (expanded) as one status line at row `y`, `cols` wide, over
/// `base`, laid out as tmux's format_draw by where its list goes. Returns
/// where each run with a range went: start, end and range.
pub fn draw(
    buf: &mut Buffer,
    y: u16,
    cols: u16,
    base: Style,
    text: &str,
) -> Vec<(u16, u16, Range)> {
    draw_at(buf, (0, y), cols, base, text, true)
}

/// [`draw`] in `cols` cells from `(x, y)`. With `clear` the cells are
/// blanked in `base` first, as tmux clears a status line; without it only
/// the text is drawn, over what is there (a pane's border line), unless a
/// `#[fill=…]` paints the rest, as tmux's format_draw does.
pub fn draw_at(
    buf: &mut Buffer,
    (x0, y): (u16, u16),
    cols: u16,
    base: Style,
    text: &str,
    clear: bool,
) -> Vec<(u16, u16, Range)> {
    let (runs, fill, layout) = runs(text, base);
    if clear || fill.is_some() {
        let line = fill.unwrap_or(base);
        buf.set_style(Rect::new(x0, y, cols, 1), line);
        for x in x0..x0.saturating_add(cols) {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ");
            }
        }
    }
    let parts = Parts {
        left: Screen::of(&runs, Part::Left),
        centre: Screen::of(&runs, Part::Centre),
        right: Screen::of(&runs, Part::Right),
        absolute: Screen::of(&runs, Part::AbsoluteCentre),
        list: Screen::of(&runs, Part::List),
        list_left: Screen::of(&runs, Part::ListLeft),
        list_right: Screen::of(&runs, Part::ListRight),
        after: Screen::of(&runs, Part::After),
    };
    let mut target = Target {
        buf,
        x0,
        y,
        cols,
        spans: Vec::new(),
    };
    // With no list left in its room, the text after it joins the part the
    // list was in, as tmux's.
    let with_after = |part: Part| {
        let joined: Vec<Run> = runs
            .iter()
            .filter(|r| r.part == part || r.part == Part::After)
            .map(|r| Run { part, ..r.clone() })
            .collect();
        Screen::of(&joined, part)
    };
    match layout.align {
        Align::Default => draw_none(
            &mut target,
            &parts.left,
            &parts.centre,
            &parts.right,
            &parts.absolute,
        ),
        Align::Left => draw_left(&mut target, &parts, layout.focus, || with_after(Part::Left)),
        Align::Centre => draw_centre(&mut target, &parts, layout.focus, || {
            with_after(Part::Centre)
        }),
        Align::Right => draw_right(&mut target, &parts, layout.focus, || {
            with_after(Part::Right)
        }),
        Align::AbsoluteCentre => draw_absolute_centre(&mut target, &parts, layout.focus),
    }
    target.spans
}

/// No list: the centre trimmed first, then the right, then the left; the
/// right against the line's end and the centre between them, as tmux's
/// format_draw_none.
fn draw_none(
    t: &mut Target<'_>,
    left: &Screen,
    centre: &Screen,
    right: &Screen,
    absolute: &Screen,
) {
    let available = t.cols;
    let (mut wl, mut wc, mut wr) = (left.width, centre.width, right.width);
    while wl + wc + wr > available {
        if wc > 0 {
            wc -= 1;
        } else if wr > 0 {
            wr -= 1;
        } else {
            wl -= 1;
        }
    }
    t.put(left, 0, 0, wl);
    t.put(right, available - wr, right.width - wr, wr);
    let middle = wl + ((available - wr) - wl) / 2;
    t.put(
        centre,
        (middle).saturating_sub(wc / 2),
        centre.width / 2 - wc / 2,
        wc,
    );
    t.put(
        absolute,
        available.saturating_sub(absolute.width) / 2,
        0,
        absolute.width,
    );
}

/// The widths left of each part once the line is trimmed in `order` (an
/// index into left, centre, right, list, after) until it fits.
fn trim(parts: &Parts, available: u16, order: [usize; 5]) -> [u16; 5] {
    let mut w = [
        parts.left.width,
        parts.centre.width,
        parts.right.width,
        parts.list.width,
        parts.after.width,
    ];
    while w.iter().map(|&x| u32::from(x)).sum::<u32>() > u32::from(available) {
        let Some(&i) = order.iter().find(|&&i| w[i] > 0) else {
            break;
        };
        w[i] -= 1;
    }
    w
}

const LEFT: usize = 0;
const CENTRE: usize = 1;
const RIGHT: usize = 2;
const LIST: usize = 3;
const AFTER: usize = 4;

/// A list on the left, after the left part, as tmux's format_draw_left:
/// trimmed first the centre, then the list, the right, after the list and
/// the left.
fn draw_left(t: &mut Target<'_>, p: &Parts, focus: (u16, u16), joined: impl Fn() -> Screen) {
    let available = t.cols;
    let [wl, wc, wr, wli, wa] = trim(p, available, [CENTRE, LIST, RIGHT, AFTER, LEFT]);
    if wli == 0 {
        return draw_none(t, &joined(), &p.centre, &p.right, &p.absolute);
    }
    t.put(&p.left, 0, 0, wl);
    t.put(&p.right, available - wr, p.right.width - wr, wr);
    t.put(&p.after, wl + wli, 0, wa);
    let from = wl + wli + wa;
    let centre = from + ((available - wr) - from) / 2;
    t.put(
        &p.centre,
        centre.saturating_sub(wc / 2),
        p.centre.width / 2 - wc / 2,
        wc,
    );
    t.put_list(p, wl, wli, focus);
    t.put(
        &p.absolute,
        available.saturating_sub(p.absolute.width) / 2,
        0,
        p.absolute.width,
    );
}

/// A list in the centre, with the centre part before it and the text
/// after it beside it, as tmux's format_draw_centre: trimmed first the
/// list, then after it, the centre, the right and the left.
fn draw_centre(t: &mut Target<'_>, p: &Parts, focus: (u16, u16), joined: impl Fn() -> Screen) {
    let available = t.cols;
    let [wl, wc, wr, wli, wa] = trim(p, available, [LIST, AFTER, CENTRE, RIGHT, LEFT]);
    if wli == 0 {
        return draw_none(t, &p.left, &joined(), &p.right, &p.absolute);
    }
    t.put(&p.left, 0, 0, wl);
    t.put(&p.right, available - wr, p.right.width - wr, wr);
    let middle = wl + ((available - wr) - wl) / 2;
    let list_at = middle.saturating_sub(wli / 2);
    t.put(&p.centre, list_at.saturating_sub(wc), 0, wc);
    t.put(&p.after, list_at + wli, 0, wa);
    t.put_list(p, list_at, wli, focus);
    t.put(
        &p.absolute,
        available.saturating_sub(p.absolute.width) / 2,
        0,
        p.absolute.width,
    );
}

/// A list on the right, after the right part and before the text after it
/// against the line's end, as tmux's format_draw_right: trimmed first the
/// centre, then the list, the right, after the list and the left.
fn draw_right(t: &mut Target<'_>, p: &Parts, focus: (u16, u16), joined: impl Fn() -> Screen) {
    let available = t.cols;
    let [wl, wc, wr, wli, wa] = trim(p, available, [CENTRE, LIST, RIGHT, AFTER, LEFT]);
    if wli == 0 {
        return draw_none(t, &p.left, &p.centre, &joined(), &p.absolute);
    }
    t.put(&p.left, 0, 0, wl);
    t.put(&p.after, available - wa, p.after.width - wa, wa);
    let right_at = available - wr - wli - wa;
    t.put(&p.right, right_at, 0, wr);
    let centre = wl + (right_at - wl) / 2;
    t.put(
        &p.centre,
        centre.saturating_sub(wc / 2),
        p.centre.width / 2 - wc / 2,
        wc,
    );
    t.put_list(p, available - wli - wa, wli, focus);
    t.put(
        &p.absolute,
        available.saturating_sub(p.absolute.width) / 2,
        0,
        p.absolute.width,
    );
}

/// A list in the absolute centre, after the absolute-centre part and
/// before the text after it, all in the middle of the line, over the left,
/// centre and right, as tmux's format_draw_absolute_centre.
fn draw_absolute_centre(t: &mut Target<'_>, p: &Parts, focus: (u16, u16)) {
    let available = t.cols;
    let (mut wl, mut wc, mut wr) = (p.left.width, p.centre.width, p.right.width);
    while wl + wc + wr > available {
        if wc > 0 {
            wc -= 1;
        } else if wr > 0 {
            wr -= 1;
        } else {
            wl -= 1;
        }
    }
    let (mut wli, mut wa, mut wab) = (p.list.width, p.after.width, p.absolute.width);
    while wli + wa + wab > available {
        if wli > 0 {
            wli -= 1;
        } else if wa > 0 {
            wa -= 1;
        } else {
            wab -= 1;
        }
    }
    t.put(&p.left, 0, 0, wl);
    t.put(&p.right, available - wr, p.right.width - wr, wr);
    let middle = wl + ((available - wr) - wl) / 2;
    t.put(
        &p.centre,
        middle.saturating_sub(wc / 2),
        p.centre.width / 2 - wc / 2,
        wc,
    );
    let mut at = (available - wli - wa - wab) / 2;
    t.put(&p.absolute, at, 0, wab);
    at += wab;
    t.put_list(p, at, wli, focus);
    t.put(&p.after, at + wli, 0, wa);
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
        // tmux's: halfway between the left and right parts, less half the
        // centre (compared by the oracle's status-line cases).
        assert_eq!(text_of(&buf, 0), "ab       mid      yz");
    }

    #[test]
    fn default_follows_push_and_pop() {
        let base = Style::default().bg(Color::Blue);
        let (runs, _, _) = runs(
            "#[fg=red]a#[push-default]#[fg=green]b#[default]c#[pop-default]#[default]d",
            base,
        );
        let fg: Vec<_> = runs.iter().map(|r| r.style.fg).collect();
        assert_eq!(fg, [Some(RED), Some(GREEN), Some(RED), None]);
    }

    #[test]
    fn ranges_are_kept_where_their_runs_go() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 1));
        let spans = draw(
            &mut buf,
            0,
            12,
            Style::default(),
            "#[range=left]L#[norange]-#[range=window|3]w3#[range=pane|%1]p#[range=session|$2]s#[range=user|a-long-user-range-text]u#[range=pane|1]x#[align=right range=right]R",
        );
        assert_eq!(
            spans,
            [
                (0, 1, Range::Left),
                (2, 4, Range::Window(3)),
                (4, 5, Range::Pane(1)),
                (5, 6, Range::Session(2)),
                (6, 7, Range::User("a-long-user-ran".into())),
                (11, 12, Range::Right)
            ]
        );
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
            "#[fill=red]#[list=on align=left]#[list=left-marker]<#[list=right-marker]>#[list=on]x#[nolist]",
        );
        assert_eq!(text_of(&buf, 0), "x     ");
        assert_eq!(buf[(5, 0)].style().bg, Some(RED));
    }
}
