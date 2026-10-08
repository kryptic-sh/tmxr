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
live parser and tmxr shows the frozen view, exactly like tmux; leaving and
re-entering copy mode picks up new output.

State: cursor `(line, col)` in the snapshot, view top line, selection anchor,
selection kind (char / line / rectangle) and the last search.

## Engine

tmxr implements tmux's `copy-mode-vi` commands itself, over the snapshot grid
(`crates/tmxr-server/src/copy.rs`). The plan's first choice was to drive an
`hjkl_engine::Editor` over the snapshot; it was set aside because the engine is
an editor first and wants a host and rendering surface that a frozen grid does
not have, while the command set copy mode needs is small.

Commands, under tmux's `send-keys -X` names so user binds keep tmux semantics:

- Cursor: `cursor-left/right/up/down`, `start-of-line`, `back-to-indentation`,
  `end-of-line`, `next-word`, `previous-word`, `next-word-end` and the `-space`
  (WORD) forms, `history-top/bottom`, `top/middle/bottom-line`.
- Scrolling: `halfpage-up/down`, `page-up/down`, `scroll-up/down`.
- Selection: `begin-selection`, `select-line`, `rectangle-toggle`,
  `clear-selection`.
- Search: `search-forward`, `search-backward`, `search-again`, `search-reverse`.
  Matching is literal, case-insensitive unless the needle has an uppercase
  letter (smart case).
- Jumps on the cursor's line: `jump-forward`, `jump-backward`,
  `jump-to-forward`, `jump-to-backward` (vi's `f F t T`), `jump-again` and
  `jump-reverse` (`;` `,`). Given no character, a jump waits for the next key,
  as vi does, rather than through tmux's one-key prompt.
- `copy-selection`, `copy-selection-and-cancel`,
  `copy-selection-no-newlines-and-cancel`, `cancel`.

Counts: digits typed in copy mode (`0` only after another digit, since `0` alone
is `start-of-line`) repeat the next command, and a count before a jump repeats
the jump (`3tx`). `send-keys -X -N count` does the same from a bind.

The default binds in [06](06-keys-and-bindings.md#copy-mode-vi) map vi keys onto
these, plus `set-mark` / `jump-to-mark` (`X`, `M-x`; jumping swaps the mark and
the cursor), `next-matching-bracket` (`%`, across lines, counting nesting) and
`refresh-from-pane` (`r`: a fresh snapshot keeping the cursor, selection and
search).

## Copying

`copy-selection*`:

1. Push the text to the paste buffer stack (`buffer0`, `buffer1`, …).
2. Unless `set-clipboard` is `off`: send OSC 52 to the attached clients (works
   locally and over SSH, as tmux-yank's OSC 52 path does), and set the **local**
   clipboard through `hjkl-clipboard` on a background thread, covering terminals
   that ignore OSC 52.

`copy-pipe` / `copy-pipe-and-cancel` copy as above and also send the text to a
shell command on its standard input: the command's argument, else the
`copy-command` option. The command runs off the state thread; a failure goes to
the message log.

`!` (copy without newlines), `Y`, `M-y` (copy and paste) and the drag-end copy
come from tmux-yank's binds.

## Rendering

The pane shows the snapshot at the view offset with the selection in the
`mode-style` style, the cursor drawn as a block, and the tmux position indicator
`[offset/history]` at the top right. The last search's matches on screen are
drawn in `copy-mode-match-style`, the one under the cursor in
`copy-mode-current-match-style` (tmux's option names).
