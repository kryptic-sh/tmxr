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
- **Typing filters** on the session name with `hjkl-picker`'s fuzzy scoring;
  matched characters highlighted.
- **Moving**: `j`/`k`, `Down`/`Up`, `C-n`/`C-p`, `C-j`/`C-k`. Because typing
  must also work, the picker is modal the way hjkl's pickers are: it opens in
  insert (query) mode where letters go to the query and arrows / `C-n` / `C-p` /
  `C-j` / `C-k` move; `Escape` switches to normal mode where `j`/`k` move
  (`g`/`G` to the ends), `/`, `i` or `a` returns to the query, and `Escape` or
  `q` closes.
- `Enter` switches the client to the selected session; `Escape`/`C-c` cancels.
- Actions: `C-x` kills the highlighted session after a y/n, `C-r` renames it
  through a prompt, and `Enter` on a name no session matches creates that
  session and switches to it. Each runs as a command addressing the session by
  its `$id`.
- Not implemented: a preview of the selected session's active window.

## Implementation

`crates/tmxr-server/src/overlay.rs`:

- `Source: hjkl_picker::PickerLogic` holds a fixed list of rows built when the
  picker opens, with `preserve_source_order() = true` (so filtering keeps the
  MRU order) and no preview. `select(idx)` →
  `PickerAction::Custom(Box::new(Target::Session(id)))`.
- `PickerOverlay` wraps `hjkl_picker::Picker` and adds the insert / normal mode
  itself; keys arrive as the crossterm `KeyEvent`s the client already sends.
- Rendering: the list and input row are drawn by tmxr (`render.rs`) with the
  `mode-style` for the selected row.
- The same overlay backs the window picker (`prefix w`, `choose-tree -w`: every
  session's windows, the current session's first), `find-window` (`prefix f`:
  the window picker opened with the prompt's text as its query) and the buffer
  picker (`prefix =`, `choose-buffer`: Enter pastes the buffer into the pane).
