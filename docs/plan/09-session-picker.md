# 09 — Session picker

Replaces tmux's `choose-tree -Zs` on `prefix s` (and `choose-tree -Zw` on
`prefix w` for windows).

## Behaviour

- Opens as a centred overlay on the requesting client (the panes keep rendering
  underneath).
- Rows: one per session — `name`, window count, `(attached)` marker, the current
  session marked `*`. The current session is first, the rest most recently used
  first, and the picker opens on the second row so `Enter` straight away goes to
  the _previous_ session (fast toggle, like `switch-client -l`).
- **Two modes.** It opens in normal mode, where `j`/`k`, `Down`/`Up` and
  `C-n`/`C-p` move and `q`, `Escape` or `C-c` close. `i`, `I`, `a`, `A` or `/`
  start typing; `Escape` goes back to normal mode, keeping the query.
- **Typing filters** on the session name with `hjkl-picker`'s fuzzy scoring;
  matched characters highlighted. Arrows and `C-n`/`C-p` still move.
- `Enter` switches the client to the selected session, in either mode.
- Actions: `C-x` kills the highlighted session after a y/n, `C-r` renames it
  through a prompt, and `Enter` on a name no session matches creates that
  session and switches to it. Each runs as a command addressing the session by
  its `$id`.
- Preview: the session picker is the list alone, and the client shows the
  highlighted session in place of its own (status line and all) while it is
  open; `Enter` switches to it, and closing the picker any other way leaves the
  client on the session it never left. The window picker shows the highlighted
  window's active pane under its list, colours included, cropped to the rows up
  to its cursor, when at least three rows are left for it.
- A session that printed since a client last showed it (the picker's preview
  counts) has a green `+` where the current one has `*`.

## Implementation

`crates/tmxr-server/src/overlay.rs`:

- `Source: hjkl_picker::PickerLogic` holds a fixed list of rows built when the
  picker opens, with `preserve_source_order() = true` (so filtering keeps the
  MRU order) and no `hjkl-picker` preview: that one is a text buffer, and the
  pane preview keeps its colours by copying the pane's cells instead.
  `select(idx)` → `PickerAction::Custom(Box::new(Target::Session(id, name)))`.
- `PickerOverlay` keeps each row's label and target, so the renderer can find
  the highlighted row's target from its label (`previewed()`).
- `PickerOverlay` wraps `hjkl_picker::Picker`. In normal mode it handles the
  keys itself; while typing it hands them to `hjkl_picker_tui::handle_key`, as
  hjkl does, except `Escape`. The session picker's `C-x` / `C-r` come first in
  both; keys arrive as the crossterm `KeyEvent`s the client already sends.
- Rendering: the list, input row and preview are drawn by tmxr (`draw_picker` in
  `render.rs`) with the `mode-style` for the selected row; `render::shown` is
  the session a client draws, the session picker's highlighted one while it is
  open.
- The same overlay backs the window picker (`prefix w`, `choose-tree -w`: every
  session's windows, the current session's first), `find-window` (`prefix f`:
  the window picker opened with the prompt's text as its query) and the buffer
  picker (`prefix =`, `choose-buffer`: Enter pastes the buffer into the pane).
