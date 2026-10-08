# 05 — Sessions, windows, panes, layout

## Model

Same shape as tmux:

```
Server
 ├─ sessions: id → Session { id: $N, name, windows: index → WindowId, current, last, created, cwd, env }
 ├─ windows:  id → Window  { id: @N, name, auto_name, layout: hjkl_layout::LayoutTree, active pane, last pane,
 │                           zoomed: Option<PaneId>, flags (bell/activity/silence), options (synchronize-panes …) }
 ├─ panes:    id → Pane    { id: %N, term: tmxr_term::Pane, window, title, copy_mode: Option<CopyMode>, … }
 ├─ clients:  id → Client  { session, size, key_table, prompt/picker/overlay, last frame, features }
 └─ buffers:  paste buffers (stack, `buffer-limit` 50)
```

Ids (`$`, `@`, `%`) are monotonic and never reused, as in tmux, so targets in
scripts stay stable. Window _indices_ are per-session and are what the user
sees; `renumber-windows on` (in the config) closes gaps after a window closes.

A window belongs to exactly one session in the MVP (`link-window` is a
non-goal). Panes belong to one window.

## Indices

`base-index 0` and `pane-base-index 0`, as set in the config. (The config's
comment says "start at 1"; the values say 0 — tmxr follows the values. See
[16-open-questions.md](16-open-questions.md).)

## Targets

tmux target syntax: `-t session:window.pane` with `$id`, `@id`, `%id`, names,
indices, prefix matches, and the special tokens `{last}` / `!`, `{next}` / `+`,
`{previous}` / `-`, `{left}`/`{right}`/`{up}`/`{down}` for panes. Resolution
lives in `tmxr-server::target` with the client's current session/window/pane as
the default — a command client inherits the pane from `TMXR_PANE`, so
`tmxr select-pane -L` run inside a pane acts on _that_ pane's window.

## Layout

Each window's pane arrangement is an `hjkl_layout::LayoutTree` (binary splits
with a ratio). Mapping:

| tmux                                 | hjkl-layout                                                           |
| ------------------------------------ | --------------------------------------------------------------------- |
| `split-window -v` (top/bottom)       | `replace_leaf(active, Split { Horizontal, 0.5, active, new })`        |
| `split-window -h` (left/right)       | `replace_leaf(active, Split { Vertical, 0.5, active, new })`          |
| `-b` (new pane before)               | children swapped                                                      |
| `-l N` / `-l N%`                     | ratio from size, or `split_fixed` for absolute sizes                  |
| `select-pane -L/-R/-U/-D`            | `neighbor_left/right/above/below` (nearest by geometry, as tmux does) |
| `kill-pane`                          | `remove_leaf`                                                         |
| `resize-pane -L/-R/-U/-D N`          | adjust the enclosing split's ratio from `last_rect`                   |
| `resize-pane -Z` (zoom)              | window flag; render the zoomed pane full-size, tree untouched         |
| `swap-pane -U/-D`, `rotate-window`   | leaf id permutation over `leaves()`                                   |
| `select-layout even-horizontal` etc. | rebuild the tree from `leaves()` with the preset shape                |
| `break-pane`, `join-pane`            | `remove_leaf` here, new window / `replace_leaf` there                 |

Pane borders take one cell between siblings, which is exactly what
`window_rects` already reserves. Sizes are recomputed from the client size minus
the status line on every resize.

`select-pane` wrap-around: tmux wraps (`-L` from the leftmost pane goes to the
rightmost). vim-tmux-navigator users expect that too, so wrapping is kept; the
wrap target is the pane at the far edge nearest the current pane's centre.

## Multiple clients per session

Window size follows tmux 3.x defaults (`window-size latest`): the window takes
the size of the client that most recently had input; smaller clients see it
cropped. `aggressive-resize on` (tmux-sensible) is the behaviour.

## Synchronize panes

`set-window-option synchronize-panes` (bound to `prefix x`) toggles a window
flag; while set, input to the active pane is also written to every other pane in
the window (each encoded for that pane's own modes). The status line shows it
(see the theme doc).

## Automatic window names

`automatic-rename on`: the window name tracks the active pane's foreground
command (`#{pane_current_command}`), refreshed on the status interval, until the
user renames the window.
