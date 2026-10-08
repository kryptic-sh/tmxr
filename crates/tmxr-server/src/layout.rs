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
/// layout order, keeping the shape.
pub fn rotate(tree: &LayoutTree, up: bool) -> LayoutTree {
    let leaves = tree.leaves();
    let n = leaves.len();
    if n < 2 {
        return tree.clone();
    }
    relabel(tree, &|id| {
        let i = leaves.iter().position(|l| *l == id).unwrap_or(0);
        let j = if up { (i + 1) % n } else { (i + n - 1) % n };
        leaves[j]
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
    "main-vertical",
    "tiled",
];

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

/// Arrange `panes` (in order) in the named preset. `None` for an unknown name.
pub fn preset(name: &str, panes: &[PaneId]) -> Option<LayoutTree> {
    let ids: Vec<usize> = panes.iter().map(|p| *p as usize).collect();
    if ids.is_empty() {
        return None;
    }
    let main = |dir: SplitDir, rest_dir: SplitDir| match ids.as_slice() {
        [one] => LayoutTree::Leaf(*one),
        [first, rest @ ..] => {
            LayoutTree::split(dir, 0.6, LayoutTree::Leaf(*first), even(rest_dir, rest))
        }
        [] => LayoutTree::Leaf(0),
    };
    Some(match name {
        "even-horizontal" => even(SplitDir::Vertical, &ids),
        "even-vertical" => even(SplitDir::Horizontal, &ids),
        "main-horizontal" => main(SplitDir::Horizontal, SplitDir::Vertical),
        "main-vertical" => main(SplitDir::Vertical, SplitDir::Horizontal),
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
        let r = rotate(&three(), true);
        assert_eq!(r.leaves(), vec![1, 2, 0]);
        let r = rotate(&three(), false);
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
    fn presets_place_every_pane() {
        let panes = [3, 4, 5, 6, 7];
        for name in PRESETS {
            let t = preset(name, &panes).unwrap();
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
        let rects = pane_rects(&preset("even-horizontal", &[1, 2, 3, 4]).unwrap(), 83, 10);
        let widths: Vec<u16> = rects.iter().map(|(_, r)| r.w).collect();
        assert!(
            widths.iter().max().unwrap() - widths.iter().min().unwrap() <= 1,
            "{widths:?}"
        );
        assert!(preset("nope", &panes).is_none());
    }
}
