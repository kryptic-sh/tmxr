# 09 — Session picker

Replaces tmux's `choose-tree -Zs` on `prefix s` (and `choose-tree -Zw` on
`prefix w` for windows).

## Behaviour

- Opens as a centred overlay on the requesting client (the panes keep rendering
  underneath).
- Rows: one per session — `name`, window count, `(attached)` marker, the current
  session marked. Ordered by most recently used, current session first-but-one
  so `Enter` straight away goes to the _previous_ session (fast toggle, like
  `switch-client -l`).
- **Typing filters** with `hjkl-fuzzy` scoring; matched characters highlighted.
- **Moving**: `j`/`k`, `Down`/`Up`, `C-n`/`C-p`, `C-j`/`C-k`. Because typing
  must also work, the picker is modal the way hjkl's pickers are: it opens in
  insert (query) mode where letters go to the query and arrows / `C-n` / `C-p` /
  `C-j` / `C-k` move; `Escape` switches to normal mode where `j`/`k` move, `/`
  or `i` returns to the query, and `Escape` again closes. This is the
  `hjkl-picker` interaction model, unchanged.
- `Enter` switches the client to the selected session; `Escape`/`C-c` cancels.
- Extra actions (post-MVP but cheap): `C-x` kill session (with confirm), `C-r`
  rename, typing a name that matches nothing + `Enter` creates a session with
  that name.
- Preview: the selected session's active window, rendered small (the picker's
  preview pane) — the `PickerLogic::preview` hook gets a text snapshot of the
  pane. Off on narrow clients.

## Implementation

- `SessionSource: hjkl_picker::PickerLogic` in `tmxr-server::picker` —
  `FilterInMemory`, `preserve_source_order() = true` (MRU order), `select(idx)`
  → `PickerAction::Custom(Box::new(PickTarget::Session(id)))`.
- Key routing: the overlay owns the client's input while open; keys go to
  `hjkl_picker_tui::handle_key(&mut picker, key)` (crossterm `KeyEvent`, which
  is what the client already sends).
- Rendering: the list and input row are drawn by tmxr with the theme's picker
  styles (the hjkl app draws its picker list itself too; `hjkl-picker-tui`
  supplies the preview pane renderer).
- The same widget backs the window picker (`WindowSource`) and later a buffer
  picker (`choose-buffer`).
