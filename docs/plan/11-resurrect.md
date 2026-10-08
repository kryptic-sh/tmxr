# 11 — Save and restore (resurrect)

Stretch goal for the MVP; designed now so the model keeps what it needs.

## What is saved

Matching tmux-resurrect's defaults:

- Every session: name, window order, current and last window.
- Every window: index, name (and whether it was auto-named), layout tree
  (`hjkl-layout` shape + ratios, serialised by tmxr since the tree type is not
  serde), zoom, active and last pane, `synchronize-panes`.
- Every pane: index, current directory, title, and the **foreground command** if
  it is in `resurrect.processes` (tmux-resurrect's default list:
  `vi vim nvim emacs man less more tail top htop irssi weechat mutt`, plus
  `hjkl` and `sqeel`). Anything else restores as a shell in the same directory.
- Optional (`resurrect.capture-pane-contents`, default off): the visible pane
  text, replayed into the restored pane before the shell starts.

## Format and location

`~/.local/share/tmxr/resurrect/` (via `hjkl-xdg`):

- `tmxr-<timestamp>.json` — one save. `serde_json`, versioned
  (`{"version": 1, …}`), human-readable on purpose: people hand-edit these.
- `last` — a small file naming the newest save (a symlink on Unix would not work
  on Windows; a pointer file works everywhere).
- Writes go through `hjkl_fs::atomic::write_atomic`; old saves are pruned to the
  newest N (`resurrect.keep`, default 20).

## Triggers

- `prefix C-s` → `resurrect-save`; `prefix C-r` → `resurrect-restore`.
- Periodic auto-save every `resurrect.auto-save-minutes` (tmux-continuum's idea;
  0 disables), only when something changed since the last save.
- Save on `kill-server` and on server shutdown by signal (best-effort).
- **Auto-restore**: with `resurrect.restore-on-start = true`, a newly started
  server restores the last save before creating the client's session. When the
  restore produced sessions, a bare `tmxr` attaches to the most recent one
  instead of creating a new session.

## Restore rules

- A session that already exists by name is skipped (tmux-resurrect does the
  same), so restore is safe to run twice.
- Missing directories fall back to `$HOME` with a message.
- Commands are re-run in the pane's directory with the pane's shell
  (`default-shell -c '<command>'` on Unix, `pwsh -c` on Windows), so `hjkl`
  comes back as `hjkl` in the right place. Arguments are only restored for
  allowlisted programs (`resurrect.restore-args`, like resurrect's `~vim`
  strategies), because blindly re-running saved argv is how restore runs
  something destructive twice.
