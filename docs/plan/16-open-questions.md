# 16 — Open questions

Decisions the plan made provisionally, and what the owner decided.

## Decided (2026-10-09)

1. **`base-index`.** The tmux config comments "Start windows and panes at 1, not
   0" but sets `base-index 0` / `pane-base-index 0`. Kept at 0, the values the
   config sets.
2. **`prefix x` / kill-pane.** The config rebinds `x` to synchronize-panes,
   which removes tmux's only kill-pane bind. Kept tmxr's added `prefix X` →
   `confirm-before kill-pane`.
3. **Theme.** Kept: Tokyo Night colours with the catppuccin status-line layout.
   Role mapping in [07](07-rendering-status-theme.md#ui-roles).
4. **Config format.** Kept: TOML with tmux command strings for binds, not a
   `tmux.conf` reader. A `tmxr import-tmux-conf` converter is possible later.
5. **Pickers** work the way hjkl's do: one mode, typing filters, arrows and
   `C-n`/`C-p` move, `Escape` closes. (The vi-style normal mode with `j`/`k`
   that tmxr had first was dropped.)

6. **Windows current directory.** Read from the foreground process's PEB; what
   the shell announces (OSC 7 / OSC 9;9) still comes first, for PowerShell.

7. **vim (not hjkl) navigation.** vim-tmux-navigator in nvim calls `tmux`, so it
   does not hand off to tmxr. Done as a Lua plugin in its own repo,
   [tmxr-navigator.nvim](https://github.com/kryptic-sh/tmxr-navigator.nvim).

Nothing is open.
