//! Pane geometry on top of `hjkl-layout`'s split tree.
//!
//! Leaves of a window's `LayoutTree` are pane ids. `window_rects` gives each
//! pane's rect with one-cell separators carved out between siblings; the
//! cells no pane covers are the borders. Directional navigation is geometric,
//! like tmux's: the neighbour is the pane on the other side of the separator,
//! not the next leaf in tree order.

use hjkl_layout::{LayoutRect, LayoutTree, SplitDir};

use crate::model::PaneId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

pub fn rect(x: u16, y: u16, w: u16, h: u16) -> LayoutRect {
    LayoutRect::new(x, y, w, h)
}

/// Every pane's rect inside a `cols` × `rows` window area.
pub fn pane_rects(tree: &LayoutTree, cols: u16, rows: u16) -> Vec<(PaneId, LayoutRect)> {
    tree.window_rects(rect(0, 0, cols, rows))
        .into_iter()
        .map(|(id, r)| (id as PaneId, r))
        .collect()
}

fn overlap(a0: u16, a_len: u16, b0: u16, b_len: u16) -> u16 {
    let start = a0.max(b0);
    let end = (a0 + a_len).min(b0 + b_len);
    end.saturating_sub(start)
}

/// The pane next to `from` in direction `dir`, wrapping to the far edge like
/// tmux when there is none. Among several neighbours, `prefer` (the pane
/// focused before `from`) wins if it is one of them, so moving back and forth
/// returns where you came from; otherwise the one level with `from`'s top
/// (left/right) or left edge (up/down), then the topmost-leftmost.
pub fn neighbour(
    rects: &[(PaneId, LayoutRect)],
    from: PaneId,
    prefer: Option<PaneId>,
    dir: Dir,
    cols: u16,
    rows: u16,
) -> Option<PaneId> {
    let r = rects.iter().find(|(id, _)| *id == from)?.1;
    let adjacent = |c: &LayoutRect| match dir {
        Dir::Left => c.x + c.w + 1 == r.x,
        Dir::Right => r.x + r.w + 1 == c.x,
        Dir::Up => c.y + c.h + 1 == r.y,
        Dir::Down => r.y + r.h + 1 == c.y,
    };
    let at_far_edge = |c: &LayoutRect| match dir {
        Dir::Left => c.x + c.w == cols,
        Dir::Right => c.x == 0,
        Dir::Up => c.y + c.h == rows,
        Dir::Down => c.y == 0,
    };
    let shared = |c: &LayoutRect| match dir {
        Dir::Left | Dir::Right => overlap(c.y, c.h, r.y, r.h),
        Dir::Up | Dir::Down => overlap(c.x, c.w, r.x, r.w),
    };
    let level = |c: &LayoutRect| match dir {
        Dir::Left | Dir::Right => r.y >= c.y && r.y < c.y + c.h,
        Dir::Up | Dir::Down => r.x >= c.x && r.x < c.x + c.w,
    };
    let best = |pred: &dyn Fn(&LayoutRect) -> bool| {
        let candidates: Vec<&(PaneId, LayoutRect)> = rects
            .iter()
            .filter(|(id, c)| *id != from && pred(c) && shared(c) > 0)
            .collect();
        if let Some(p) = prefer.filter(|p| candidates.iter().any(|(id, _)| id == p)) {
            return Some(p);
        }
        candidates
            .into_iter()
            .min_by_key(|(_, c)| (!level(c), c.y, c.x))
            .map(|(id, _)| *id)
    };
    best(&adjacent).or_else(|| best(&at_far_edge))
}

/// Move the separator nearest to `pane` along `dir` by `cells`.
/// Returns whether anything changed.
pub fn resize(
    tree: &mut LayoutTree,
    pane: PaneId,
    dir: Dir,
    cells: u16,
    cols: u16,
    rows: u16,
) -> bool {
    let split_dir = match dir {
        Dir::Left | Dir::Right => SplitDir::Vertical,
        Dir::Up | Dir::Down => SplitDir::Horizontal,
    };
    let delta = match dir {
        Dir::Left | Dir::Up => -i32::from(cells),
        Dir::Right | Dir::Down => i32::from(cells),
    };
    resize_in(
        tree,
        pane as usize,
        split_dir,
        delta,
        rect(0, 0, cols, rows),
    )
}

/// `resize-pane -x` / `-y`: make `pane`'s side of the nearest split across
/// `horizontal` (side by side, for `-x`) `size` cells, as tmux's
/// `layout_resize_pane_to`. Returns whether anything changed.
pub fn resize_to(
    tree: &mut LayoutTree,
    pane: PaneId,
    horizontal: bool,
    size: u16,
    cols: u16,
    rows: u16,
) -> bool {
    let want = if horizontal {
        SplitDir::Vertical
    } else {
        SplitDir::Horizontal
    };
    let area = rect(0, 0, cols, rows);
    let Some((first, now)) = side_at(tree, pane as usize, want, area) else {
        return false;
    };
    // The separator moves right (down) to grow the first side, left (up)
    // to grow the second.
    let grow = i32::from(size) - i32::from(now);
    resize_in(
        tree,
        pane as usize,
        want,
        if first { grow } else { -grow },
        area,
    )
}

/// At the split [`resize_in`] would move for `id` along `want` (the
/// deepest one across that holds it and is not fixed): whether `id` is on
/// its first side, and that side's length.
fn side_at(tree: &LayoutTree, id: usize, want: SplitDir, area: LayoutRect) -> Option<(bool, u16)> {
    let LayoutTree::Split {
        dir,
        ratio,
        fixed,
        a,
        b,
        ..
    } = tree
    else {
        return None;
    };
    let geo = hjkl_layout::split_geometry(area, *dir, *ratio, *fixed);
    let (child, child_area, first) = if tree_contains(a, id) {
        (a, geo.a, true)
    } else if tree_contains(b, id) {
        (b, geo.b, false)
    } else {
        return None;
    };
    side_at(child, id, want, child_area).or_else(|| {
        (*dir == want && fixed.is_none()).then_some((
            first,
            match want {
                SplitDir::Vertical => child_area.w,
                _ => child_area.h,
            },
        ))
    })
}

fn resize_in(
    tree: &mut LayoutTree,
    id: usize,
    want: SplitDir,
    delta: i32,
    area: LayoutRect,
) -> bool {
    let LayoutTree::Split {
        dir,
        ratio,
        fixed,
        a,
        b,
        ..
    } = tree
    else {
        return false;
    };
    if !tree_contains(a, id) && !tree_contains(b, id) {
        return false;
    }
    let geo = hjkl_layout::split_geometry(area, *dir, *ratio, *fixed);
    let inner = if tree_contains(a, id) {
        resize_in(a, id, want, delta, geo.a)
    } else {
        resize_in(b, id, want, delta, geo.b)
    };
    if inner || *dir != want || fixed.is_some() {
        return inner;
    }
    let len = match want {
        SplitDir::Vertical => area.w,
        _ => area.h,
    };
    if len < 3 {
        return false;
    }
    let cells = (f32::from(len) * *ratio).round() as i32 + delta;
    let cells = cells.clamp(2, i32::from(len) - 1);
    let new = cells as f32 / f32::from(len);
    let changed = (new - *ratio).abs() > f32::EPSILON;
    *ratio = new;
    changed
}

fn tree_contains(t: &LayoutTree, id: usize) -> bool {
    t.contains(id)
}

/// A layout cell as tmux's layout strings write one: its size and place,
/// and a pane's id or the cells it is split into (side by side for a
/// vertical split, `{…}`, stacked otherwise, `[…]`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cell {
    rect: LayoutRect,
    kind: CellKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CellKind {
    Pane(usize),
    Split(SplitDir, Vec<Cell>),
}

/// tmux's `layout_checksum` of a layout string's body.
fn layout_checksum(body: &str) -> u16 {
    body.bytes().fold(0u16, |csum, b| {
        ((csum >> 1) | ((csum & 1) << 15)).wrapping_add(u16::from(b))
    })
}

/// The cells of `tree` over `area`, a run of splits the same way written as
/// one cell, as tmux keeps them.
fn cell_of(tree: &LayoutTree, area: LayoutRect) -> Cell {
    match tree {
        LayoutTree::Split {
            dir,
            ratio,
            fixed,
            a,
            b,
            ..
        } => {
            let geo = hjkl_layout::split_geometry(area, *dir, *ratio, *fixed);
            let mut children = Vec::new();
            for child in [cell_of(a, geo.a), cell_of(b, geo.b)] {
                match child.kind {
                    CellKind::Split(d, inner) if d == *dir => children.extend(inner),
                    _ => children.push(child),
                }
            }
            Cell {
                rect: area,
                kind: CellKind::Split(*dir, children),
            }
        }
        LayoutTree::Leaf(id) => Cell {
            rect: area,
            kind: CellKind::Pane(*id),
        },
        _ => Cell {
            rect: area,
            kind: CellKind::Pane(0),
        },
    }
}

fn write_cell(cell: &Cell, out: &mut String) {
    use std::fmt::Write as _;
    let r = cell.rect;
    let _ = write!(out, "{}x{},{},{}", r.w, r.h, r.x, r.y);
    match &cell.kind {
        CellKind::Pane(id) => {
            let _ = write!(out, ",{id}");
        }
        CellKind::Split(dir, children) => {
            let (open, close) = if *dir == SplitDir::Vertical {
                ('{', '}')
            } else {
                ('[', ']')
            };
            out.push(open);
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_cell(child, out);
            }
            out.push(close);
        }
    }
}

/// `tree` over a `cols` × `rows` window as tmux's `layout_dump` writes it
/// (`#{window_layout}`): `checksum,cell`.
pub fn dump(tree: &LayoutTree, cols: u16, rows: u16) -> String {
    let mut body = String::new();
    write_cell(&cell_of(tree, rect(0, 0, cols, rows)), &mut body);
    format!("{:04x},{body}", layout_checksum(&body))
}

/// A layout string read as tmux's `layout_parse` reads one: the checksum
/// must match and the cells nest properly. Returns the tree, the pane ids
/// it names kept as the leaves' (callers lay it over their panes with
/// [`reuse`]), and the size it is for, which tmux resizes the window to.
pub fn parse(layout: &str) -> Result<(LayoutTree, (u16, u16)), String> {
    let invalid = || "invalid layout".to_owned();
    let (sum, body) = layout.split_once(',').ok_or_else(invalid)?;
    let sum = u16::from_str_radix(sum, 16).map_err(|_| invalid())?;
    if sum != layout_checksum(body) {
        return Err(invalid());
    }
    let mut rest = body;
    let cell = parse_cell(&mut rest, 0).ok_or_else(invalid)?;
    if !rest.is_empty() {
        return Err(invalid());
    }
    Ok((tree_of(&cell), (cell.rect.w, cell.rect.h)))
}

/// Digits at the start of `s`, taken off it.
fn number(s: &mut &str) -> Option<u16> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let n = s[..end].parse().ok()?;
    *s = &s[end..];
    Some(n)
}

/// How deep a layout's splits may nest. Each level that splits takes a
/// pane and its border off a side, so no layout of a window a terminal can
/// show comes near it; a deeper string is refused rather than read and
/// laid out by recursion that would run out of stack. tmux has no limit.
const MOST_NESTED: usize = 1000;

fn parse_cell(s: &mut &str, depth: usize) -> Option<Cell> {
    if depth > MOST_NESTED {
        return None;
    }
    let w = number(s)?;
    *s = s.strip_prefix('x')?;
    let h = number(s)?;
    *s = s.strip_prefix(',')?;
    let x = number(s)?;
    *s = s.strip_prefix(',')?;
    let y = number(s)?;
    let rect = rect(x, y, w, h);
    // `,ID` for a pane, unless it is the next cell's `WxH`.
    if let Some(after) = s.strip_prefix(',') {
        let mut probe = after;
        if let Some(id) = number(&mut probe)
            && !probe.starts_with('x')
        {
            *s = probe;
            return Some(Cell {
                rect,
                kind: CellKind::Pane(usize::from(id)),
            });
        }
    }
    let (dir, close) = match s.chars().next() {
        Some('{') => (SplitDir::Vertical, '}'),
        Some('[') => (SplitDir::Horizontal, ']'),
        // A cell with no id: a pane tmux numbers itself.
        _ => {
            return Some(Cell {
                rect,
                kind: CellKind::Pane(0),
            });
        }
    };
    let mut children = Vec::new();
    loop {
        *s = &s[1..];
        children.push(parse_cell(s, depth + 1)?);
        if !s.starts_with(',') {
            break;
        }
    }
    *s = s.strip_prefix(close)?;
    Some(Cell {
        rect,
        kind: CellKind::Split(dir, children),
    })
}

/// A cell as a tree: each run of children a chain of splits, the first
/// child given its cells and border of the run's length.
fn tree_of(cell: &Cell) -> LayoutTree {
    match &cell.kind {
        CellKind::Pane(id) => LayoutTree::Leaf(*id),
        CellKind::Split(dir, children) => {
            let len = if *dir == SplitDir::Vertical {
                cell.rect.w
            } else {
                cell.rect.h
            };
            chain(*dir, children, len)
        }
    }
}

fn chain(dir: SplitDir, children: &[Cell], len: u16) -> LayoutTree {
    match children {
        [] => LayoutTree::Leaf(0),
        [only] => tree_of(only),
        [first, rest @ ..] => {
            let size = if dir == SplitDir::Vertical {
                first.rect.w
            } else {
                first.rect.h
            };
            // hjkl-layout gives the first child round(len × ratio) cells,
            // its border among them.
            let ratio = f32::from(size + 1) / f32::from(len.max(1));
            LayoutTree::split(
                dir,
                ratio,
                tree_of(first),
                chain(dir, rest, len.saturating_sub(size + 1)),
            )
        }
    }
}

/// `old` laid over `panes`, as tmux's `layout_parse` lays a layout over a
/// window's panes: its cells go to them in order, after cells beyond their
/// number go from the bottom right. More panes than cells is an error.
pub fn reuse(old: &LayoutTree, panes: &[PaneId]) -> Result<LayoutTree, String> {
    let mut tree = old.clone();
    let mut cells = tree.leaves();
    if panes.len() > cells.len() {
        return Err(format!(
            "have {} panes but need {}",
            panes.len(),
            cells.len()
        ));
    }
    while cells.len() > panes.len() {
        let last = cells.pop().ok_or("invalid layout")?;
        tree.remove_leaf(last).map_err(|_| "invalid layout")?;
    }
    let to: std::collections::HashMap<usize, usize> = cells
        .iter()
        .zip(panes)
        .map(|(cell, pane)| (*cell, *pane as usize))
        .collect();
    Ok(relabel(&tree, &|id| to.get(&id).copied().unwrap_or(id)))
}

/// Rebuild `tree` with every leaf id passed through `f`.
pub fn relabel(tree: &LayoutTree, f: &dyn Fn(usize) -> usize) -> LayoutTree {
    match tree {
        LayoutTree::Leaf(id) => LayoutTree::Leaf(f(*id)),
        LayoutTree::Split {
            dir, ratio, a, b, ..
        } => LayoutTree::split(*dir, *ratio, relabel(a, f), relabel(b, f)),
        other => other.clone(),
    }
}

/// Exchange two panes' positions.
pub fn swap(tree: &LayoutTree, x: PaneId, y: PaneId) -> LayoutTree {
    let (x, y) = (x as usize, y as usize);
    relabel(tree, &|id| {
        if id == x {
            y
        } else if id == y {
            x
        } else {
            id
        }
    })
}

/// Move every pane one position forward (`up == false`) or backward in
/// `order` (the window's panes), keeping the shape.
pub fn rotate(tree: &LayoutTree, order: &[PaneId], up: bool) -> LayoutTree {
    let n = order.len();
    if n < 2 {
        return tree.clone();
    }
    relabel(tree, &|id| {
        let Some(i) = order.iter().position(|p| *p as usize == id) else {
            return id;
        };
        let j = if up { (i + 1) % n } else { (i + n - 1) % n };
        order[j] as usize
    })
}

/// `select-layout -E`: spread out evenly the panes beside `pane` — the run of
/// same-direction splits its parent split belongs to, which is the layout
/// cell tmux spreads. Splits outside that run keep their ratios.
pub fn spread(tree: &LayoutTree, pane: PaneId) -> LayoutTree {
    let pane = pane as usize;
    let dirs = path_dirs(tree, pane);
    let Some(&dir) = dirs.last() else {
        return tree.clone();
    };
    // The run starts at the highest split above the parent in that direction.
    let start = dirs.iter().rposition(|d| *d != dir).map_or(0, |i| i + 1);
    spread_from(tree, pane, start, dir)
}

/// Directions of the splits from the root down to `pane`'s parent.
fn path_dirs(tree: &LayoutTree, pane: usize) -> Vec<SplitDir> {
    match tree {
        LayoutTree::Split { dir, a, b, .. } if tree.contains(pane) => {
            let below = if a.contains(pane) { a } else { b };
            let mut dirs = vec![*dir];
            dirs.extend(path_dirs(below, pane));
            dirs
        }
        _ => Vec::new(),
    }
}

/// Rebuild `tree`, evening out the run that starts `depth` splits down the
/// path to `pane`.
fn spread_from(tree: &LayoutTree, pane: usize, depth: usize, dir: SplitDir) -> LayoutTree {
    if depth == 0 {
        return even_run(tree, dir);
    }
    match tree {
        LayoutTree::Split {
            dir: d,
            ratio,
            a,
            b,
            ..
        } => {
            let (a, b) = if a.contains(pane) {
                (spread_from(a, pane, depth - 1, dir), (**b).clone())
            } else {
                ((**a).clone(), spread_from(b, pane, depth - 1, dir))
            };
            LayoutTree::split(*d, *ratio, a, b)
        }
        other => other.clone(),
    }
}

/// Give every member of a run of `dir` splits the same share.
fn even_run(tree: &LayoutTree, dir: SplitDir) -> LayoutTree {
    match tree {
        LayoutTree::Split { dir: d, a, b, .. } if *d == dir => {
            let (wa, wb) = (run_members(a, dir), run_members(b, dir));
            LayoutTree::split(
                dir,
                wa as f32 / (wa + wb) as f32,
                even_run(a, dir),
                even_run(b, dir),
            )
        }
        other => other.clone(),
    }
}

/// How many members of a `dir` run `tree` holds.
fn run_members(tree: &LayoutTree, dir: SplitDir) -> usize {
    match tree {
        LayoutTree::Split { dir: d, a, b, .. } if *d == dir => {
            run_members(a, dir) + run_members(b, dir)
        }
        _ => 1,
    }
}

/// tmux's preset layouts, in `next-layout` order.
pub const PRESETS: &[&str] = &[
    "even-horizontal",
    "even-vertical",
    "main-horizontal",
    "main-horizontal-mirrored",
    "main-vertical",
    "main-vertical-mirrored",
    "tiled",
];

/// `ids` side by side along `dir` over `len` cells, sized as tmux's
/// `layout_spread_cell`: each the same, a border between them, and the
/// cells left over one each to the first panes.
fn spread_cells(dir: SplitDir, ids: &[usize], len: u16) -> LayoutTree {
    match ids {
        [] => LayoutTree::Leaf(0),
        [one] => LayoutTree::Leaf(*one),
        [first, rest @ ..] => {
            let n = u16::try_from(ids.len()).unwrap_or(u16::MAX);
            let each = len.saturating_sub(n - 1) / n;
            let extra = len.saturating_add(1).saturating_sub(n * (each + 1));
            let first_cells = if extra > 0 && each > 0 {
                each + 1
            } else {
                each
            };
            if first_cells == 0 || len < 2 {
                return even(dir, ids);
            }
            // hjkl-layout gives the first child round(len × ratio) cells,
            // its border among them.
            let ratio = f32::from(first_cells + 1) / f32::from(len);
            LayoutTree::split(
                dir,
                ratio,
                LayoutTree::Leaf(*first),
                spread_cells(dir, rest, len - first_cells - 1),
            )
        }
    }
}

fn even(dir: SplitDir, ids: &[usize]) -> LayoutTree {
    match ids {
        [] => LayoutTree::Leaf(0),
        [one] => LayoutTree::Leaf(*one),
        [first, rest @ ..] => LayoutTree::split(
            dir,
            1.0 / ids.len() as f32,
            LayoutTree::Leaf(*first),
            even(dir, rest),
        ),
    }
}

/// tmux's `main-pane-height` / `-width` and `other-pane-height` / `-width`:
/// cells, or a percentage, as tmux takes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainPane {
    pub height: String,
    pub width: String,
    pub other_height: String,
    pub other_width: String,
}

impl Default for MainPane {
    /// tmux's defaults.
    fn default() -> Self {
        Self {
            height: "24".into(),
            width: "80".into(),
            other_height: "0".into(),
            other_width: "0".into(),
        }
    }
}

/// tmux's `args_string_percentage`: `N` from 0 to `max`, or `N%` (0 to 100)
/// of `of` if that is at most `max`. `None` for anything else.
pub(crate) fn size_spec(spec: &str, max: u16, of: u16) -> Option<u16> {
    let n = match spec.strip_suffix('%') {
        Some(pct) => {
            let pct: u32 = pct.parse().ok().filter(|p| *p <= 100)?;
            u16::try_from(u32::from(of) * pct / 100).ok()?
        }
        None => spec.parse().ok()?,
    };
    (n <= max).then_some(n)
}

/// The main pane's cells and the other panes' across `len` cells, a border
/// between them: tmux 3.6's `layout_set_main_h` / `_v`, `default` when
/// `main` does not fit (24 rows, 80 columns).
fn main_split(len: u16, main: &str, other: &str, default: u16) -> (u16, u16) {
    let avail = len.saturating_sub(1);
    let mut main_cells = size_spec(main, avail, avail).unwrap_or(default);
    let other_cells;
    if main_cells + 1 >= avail {
        main_cells = if avail <= 2 { 1 } else { avail - 1 };
        other_cells = 1;
    } else {
        match size_spec(other, avail, avail).filter(|o| *o > 0) {
            Some(o) if avail - o >= main_cells => {
                other_cells = o;
                main_cells = avail - o;
            }
            _ => other_cells = avail - main_cells,
        }
    }
    (main_cells, other_cells)
}

/// Arrange `panes` (in order) in the named preset for a `cols` x `rows`
/// window. `None` for an unknown name.
pub fn preset(
    name: &str,
    panes: &[PaneId],
    (cols, rows): (u16, u16),
    sizes: &MainPane,
) -> Option<LayoutTree> {
    let ids: Vec<usize> = panes.iter().map(|p| *p as usize).collect();
    if ids.is_empty() {
        return None;
    }
    // The first pane is the main one, sized as tmux sizes it; mirrored, it
    // goes below or to the right of the rest instead of above or to the
    // left. hjkl-layout gives the first child round(len × ratio) cells,
    // its border among them.
    let main = |dir: SplitDir, rest_dir: SplitDir, mirrored: bool| match ids.as_slice() {
        [one] => LayoutTree::Leaf(*one),
        [first, rest @ ..] => {
            let (len, main_cells, other_cells) = if dir == SplitDir::Horizontal {
                let (m, o) = main_split(rows, &sizes.height, &sizes.other_height, 24);
                (rows, m, o)
            } else {
                let (m, o) = main_split(cols, &sizes.width, &sizes.other_width, 80);
                (cols, m, o)
            };
            // The other panes share the length across the split.
            let across = if dir == SplitDir::Horizontal {
                cols
            } else {
                rows
            };
            let others = spread_cells(rest_dir, rest, across);
            let len = f32::from(len.max(1));
            if mirrored {
                let ratio = f32::from(other_cells + 1) / len;
                LayoutTree::split(dir, ratio, others, LayoutTree::Leaf(*first))
            } else {
                let ratio = f32::from(main_cells + 1) / len;
                LayoutTree::split(dir, ratio, LayoutTree::Leaf(*first), others)
            }
        }
        [] => LayoutTree::Leaf(0),
    };
    Some(match name {
        "even-horizontal" => spread_cells(SplitDir::Vertical, &ids, cols),
        "even-vertical" => spread_cells(SplitDir::Horizontal, &ids, rows),
        "main-horizontal" => main(SplitDir::Horizontal, SplitDir::Vertical, false),
        "main-horizontal-mirrored" => main(SplitDir::Horizontal, SplitDir::Vertical, true),
        "main-vertical" => main(SplitDir::Vertical, SplitDir::Horizontal, false),
        "main-vertical-mirrored" => main(SplitDir::Vertical, SplitDir::Horizontal, true),
        "tiled" => {
            let n = ids.len();
            let cols = (1..=n).find(|c| c * c >= n).unwrap_or(1);
            let rows: Vec<LayoutTree> = ids
                .chunks(cols)
                .map(|row| even(SplitDir::Vertical, row))
                .collect();
            stack(rows)
        }
        _ => return None,
    })
}

fn stack(mut rows: Vec<LayoutTree>) -> LayoutTree {
    let n = rows.len();
    let first = rows.remove(0);
    if rows.is_empty() {
        return first;
    }
    LayoutTree::split(SplitDir::Horizontal, 1.0 / n as f32, first, stack(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0 | (1 / 2): a left pane and a right column split top/bottom.
    fn three() -> LayoutTree {
        LayoutTree::split(
            SplitDir::Vertical,
            0.5,
            LayoutTree::Leaf(0),
            LayoutTree::split(
                SplitDir::Horizontal,
                0.5,
                LayoutTree::Leaf(1),
                LayoutTree::Leaf(2),
            ),
        )
    }

    #[test]
    fn rects_tile_the_window_with_one_cell_borders() {
        let rects = pane_rects(&three(), 80, 24);
        let area: u32 = rects
            .iter()
            .map(|(_, r)| u32::from(r.w) * u32::from(r.h))
            .sum();
        // One separator column (24 cells) and one separator row in the right
        // column (40 cells).
        assert_eq!(area + 24 + 40, 80 * 24);
    }

    #[test]
    fn navigation_is_geometric_and_wraps() {
        let rects = pane_rects(&three(), 80, 24);
        assert_eq!(neighbour(&rects, 0, None, Dir::Right, 80, 24), Some(1));
        // Coming back from the bottom-right pane returns to it.
        assert_eq!(neighbour(&rects, 0, Some(2), Dir::Right, 80, 24), Some(2));
        // A preference that is not a neighbour is ignored.
        assert_eq!(neighbour(&rects, 1, Some(0), Dir::Down, 80, 24), Some(2));
        assert_eq!(neighbour(&rects, 2, None, Dir::Left, 80, 24), Some(0));
        assert_eq!(neighbour(&rects, 1, None, Dir::Down, 80, 24), Some(2));
        assert_eq!(neighbour(&rects, 2, None, Dir::Up, 80, 24), Some(1));
        // Wrap: left of the leftmost pane is the rightmost column.
        assert_eq!(neighbour(&rects, 0, None, Dir::Left, 80, 24), Some(1));
        // Wrap: down from the bottom-right pane is the top-right pane.
        assert_eq!(neighbour(&rects, 2, None, Dir::Down, 80, 24), Some(1));
        // A single pane has no neighbour at all.
        let one = pane_rects(&LayoutTree::Leaf(7), 80, 24);
        assert_eq!(neighbour(&one, 7, None, Dir::Left, 80, 24), None);
    }

    #[test]
    fn resize_moves_the_nearest_matching_separator() {
        let mut t = three();
        let before = pane_rects(&t, 80, 24);
        assert!(resize(&mut t, 0, Dir::Right, 5, 80, 24));
        let after = pane_rects(&t, 80, 24);
        let w = |rs: &[(PaneId, LayoutRect)], id| rs.iter().find(|(p, _)| *p == id).unwrap().1.w;
        assert_eq!(w(&after, 0), w(&before, 0) + 5);
        // Pane 1 is in a vertical split too (the outer one), so -L from it
        // moves the outer separator left.
        assert!(resize(&mut t, 1, Dir::Left, 5, 80, 24));
        assert_eq!(w(&pane_rects(&t, 80, 24), 0), w(&before, 0));
        // A lone pane cannot resize.
        let mut lone = LayoutTree::Leaf(0);
        assert!(!resize(&mut lone, 0, Dir::Left, 1, 80, 24));
    }

    #[test]
    fn swap_and_rotate_keep_the_shape() {
        let t = swap(&three(), 0, 2);
        assert_eq!(t.leaves(), vec![2, 1, 0]);
        let r = rotate(&three(), &[0, 1, 2], true);
        assert_eq!(r.leaves(), vec![1, 2, 0]);
        let r = rotate(&three(), &[0, 1, 2], false);
        assert_eq!(r.leaves(), vec![2, 0, 1]);
        // By the pane list, not the layout: each pane takes the cell of the
        // one after it in the list.
        let r = rotate(&three(), &[0, 2, 1], true);
        assert_eq!(r.leaves(), vec![2, 0, 1]);
    }

    #[test]
    fn spread_evens_out_the_run_beside_the_pane() {
        // 0 | (1 | 2) at 50/50 then 50/50: widths 39/19/20 of 80.
        let run = LayoutTree::split(
            SplitDir::Vertical,
            0.5,
            LayoutTree::Leaf(0),
            LayoutTree::split(
                SplitDir::Vertical,
                0.5,
                LayoutTree::Leaf(1),
                LayoutTree::Leaf(2),
            ),
        );
        let widths = |t: &LayoutTree| -> Vec<u16> {
            pane_rects(t, 80, 24).iter().map(|(_, r)| r.w).collect()
        };
        let before = widths(&run);
        assert!(before[0] > before[1] + 10, "{before:?}");
        let after = widths(&spread(&run, 1));
        let (min, max) = (after.iter().min().unwrap(), after.iter().max().unwrap());
        assert!(max - min <= 1, "{after:?}");

        // Above a run in another direction, a split keeps its ratio.
        let tall = LayoutTree::split(SplitDir::Horizontal, 0.7, run, LayoutTree::Leaf(3));
        let LayoutTree::Split { ratio, .. } = spread(&tall, 1) else {
            unreachable!()
        };
        assert!((ratio - 0.7).abs() < f32::EPSILON);
        // A lone pane has nothing to spread.
        assert_eq!(spread(&LayoutTree::Leaf(0), 0).leaves(), vec![0]);
    }

    #[test]
    fn mirrored_presets_put_the_main_pane_below_or_right() {
        let rects = |name: &str| {
            pane_rects(
                &preset(name, &[1, 2, 3], (100, 40), &MainPane::default()).unwrap(),
                100,
                40,
            )
        };
        let main = |name: &str| rects(name).into_iter().find(|(p, _)| *p == 1).unwrap().1;
        // main-horizontal: the main pane across the top; mirrored, the bottom.
        let (top, bottom) = (main("main-horizontal"), main("main-horizontal-mirrored"));
        assert_eq!((top.y, top.w), (0, 100));
        assert_eq!((bottom.y + bottom.h, bottom.w), (40, 100));
        assert!(
            rects("main-horizontal-mirrored")
                .iter()
                .all(|(_, r)| r.y <= bottom.y)
        );
        // main-vertical: the main pane down the left; mirrored, the right.
        let (left, right) = (main("main-vertical"), main("main-vertical-mirrored"));
        assert_eq!((left.x, left.h), (0, 40));
        assert_eq!((right.x + right.w, right.h), (100, 40));
        // Still the larger share.
        assert!(right.w >= 50, "{right:?}");
    }

    #[test]
    fn main_pane_sizes_are_tmuxs() {
        // tmux 3.6, a 30-row window: the main pane's rows and the others'.
        assert_eq!(main_split(30, "24", "0", 24), (24, 5));
        assert_eq!(main_split(30, "50%", "0", 24), (14, 15));
        assert_eq!(main_split(30, "3", "0", 24), (3, 26));
        // Too tall for the window: tmux's default instead.
        assert_eq!(main_split(30, "40", "0", 24), (24, 5));
        // Others that would shrink the main pane are not taken.
        assert_eq!(main_split(30, "24", "10", 24), (24, 5));
        assert_eq!(main_split(30, "3", "10", 24), (19, 10));
    }

    #[test]
    fn presets_place_every_pane() {
        let panes = [3, 4, 5, 6, 7];
        for name in PRESETS {
            let t = preset(name, &panes, (100, 40), &MainPane::default()).unwrap();
            let mut leaves = t.leaves();
            leaves.sort_unstable();
            assert_eq!(leaves, vec![3, 4, 5, 6, 7], "{name}");
            let rects = pane_rects(&t, 100, 40);
            assert!(
                rects.iter().all(|(_, r)| r.w > 0 && r.h > 0),
                "{name}: {rects:?}"
            );
        }
        // even-horizontal really is even.
        let rects = pane_rects(
            &preset(
                "even-horizontal",
                &[1, 2, 3, 4],
                (83, 10),
                &MainPane::default(),
            )
            .unwrap(),
            83,
            10,
        );
        let widths: Vec<u16> = rects.iter().map(|(_, r)| r.w).collect();
        assert!(
            widths.iter().max().unwrap() - widths.iter().min().unwrap() <= 1,
            "{widths:?}"
        );
        assert!(preset("nope", &panes, (100, 40), &MainPane::default()).is_none());
    }

    #[test]
    fn layout_strings_round_trip_with_tmuxs_checksum() {
        // Strings tmux 3.6 wrote for an 80x24 window (2026-10-18): pane 0
        // on the left, 1 and 2 stacked on the right; then pane 0 split, its
        // new pane 3 joining the same left-right cell; and one pane alone.
        for tmux in [
            "d67e,80x24,0,0{40x24,0,0,0,39x24,41,0[39x12,41,0,1,39x11,41,13,2]}",
            "ff26,80x24,0,0{20x24,0,0,0,19x24,21,0,3,39x24,41,0[39x12,41,0,1,39x11,41,13,2]}",
            "b25d,80x24,0,0,0",
        ] {
            let (tree, size) = parse(tmux).unwrap();
            assert_eq!(size, (80, 24));
            assert_eq!(dump(&tree, 80, 24), tmux);
        }
        assert_eq!(parse("0000,80x24,0,0,1").unwrap_err(), "invalid layout");
        assert!(parse("d67e,80x24,0,0{40x24,0,0,0").is_err());
    }

    /// Splits nested past [`MOST_NESTED`] are refused, not read until the
    /// stack runs out; up to it they are read.
    #[test]
    fn a_layout_nests_only_so_deep() {
        let nested = |depth: usize| {
            let body = format!(
                "{}80x23,0,0,0{}",
                "80x23,0,0[".repeat(depth),
                "]".repeat(depth)
            );
            format!("{:04x},{body}", layout_checksum(&body))
        };
        assert!(parse(&nested(MOST_NESTED)).is_ok());
        assert_eq!(
            parse(&nested(MOST_NESTED + 1)).unwrap_err(),
            "invalid layout"
        );
        assert!(parse(&nested(100_000)).is_err());
    }

    #[test]
    fn even_layouts_give_the_spare_cells_to_the_first_panes() {
        // tmux's layout_spread_cell: 30 rows for two panes and a border
        // are 15 and 14 (checked against tmux 3.6).
        let heights = |n: usize, rows: u16| -> Vec<u16> {
            let panes: Vec<PaneId> = (1..=n as PaneId).collect();
            let t = preset("even-vertical", &panes, (80, rows), &MainPane::default()).unwrap();
            pane_rects(&t, 80, rows).iter().map(|(_, r)| r.h).collect()
        };
        assert_eq!(heights(2, 30), [15, 14]);
        assert_eq!(heights(3, 10), [3, 3, 2]);
        assert_eq!(heights(4, 83), [20, 20, 20, 20]);
    }
}
