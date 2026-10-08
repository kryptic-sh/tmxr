# 08 — Copy mode

`mode-keys vi` (set in the config) is the only mode in the MVP; emacs copy mode
is backlog.

## Entering

`prefix [` (`copy-mode`), `prefix PageUp` (`copy-mode -u`), mouse wheel up in a
pane that has not requested the mouse, and `MouseDrag1Pane` (start a selection
directly), as tmux does.

## Model

On entry the pane's scrollback + visible screen is **snapshotted** into a text
grid with per-cell styles (vt100's `Screen` with `set_scrollback(offset)` gives
rows at any offset). The pane keeps running underneath; new output goes to the
live parser and tmxr shows the frozen view, exactly like tmux (and like tmux,
`refresh-from-pane` / re-entering picks up new output).

State: cursor `(line, col)` in the snapshot, view top line, selection anchor,
selection kind (char / line / rectangle), last search, last `f/t` char, marks.

## Engine

Two routes, decided by a spike at the start of M6:

1. **hjkl engine (preferred).** Load the snapshot text into
   `hjkl_buffer::View::from_str` and drive an `hjkl_engine::Editor` with
   `modifiable = false`. That brings vim's own motions, visual / visual-line /
   visual-block, `/` `?` search with `n`/`N`, counts, marks and `%` — the same
   motion code the user's editor runs, so muscle memory matches exactly. The
   picker already pulls `hjkl-engine` and `hjkl-buffer` into the build, so the
   dependency cost is already paid. tmux names (`begin-selection`,
   `rectangle-toggle`, `copy-selection-and-cancel`, …) become thin adapters over
   editor actions so `send-keys -X` and user binds keep tmux semantics.
2. **Own motions.** If the engine needs a host/rendering surface that does not
   fit (it is an editor first), implement the tmux `copy-mode-vi` command set
   directly over the grid using `hjkl_engine::motions::*` where its free
   functions (`Cursor + Query`) can run over a grid adapter.

Either way the copy-mode commands are exposed as tmux's `send-keys -X <name>`
names, so the key table in [06](06-keys-and-bindings.md#copy-mode-vi) is just
binds.

## Copying

`copy-selection*` / `copy-pipe*`:

1. Push the text to the paste buffer stack (`buffer0`, `buffer1`, …).
2. With `set-clipboard on` (default `external`): send OSC 52 to the client(s) —
   works locally and over SSH, as tmux-yank's OSC 52 path does.
3. Also set the **local** clipboard via `hjkl-clipboard` when the server runs on
   the same machine as a desktop session (Wayland/X11/macOS/Windows), covering
   terminals that ignore OSC 52. `copy-command` overrides this with a shell
   command, like tmux.

`!` (copy without newlines), `Y`, `M-y` (copy and paste) and the drag-end copy
come from tmux-yank's binds.

## Rendering

The pane shows the snapshot at the view offset with the selection in the
`selection` style, the cursor drawn as a block, and the tmux position indicator
`[offset/history]` at the top right. Search matches are highlighted.
