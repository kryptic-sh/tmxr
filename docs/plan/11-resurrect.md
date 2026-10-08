# 11 — Save and restore (resurrect)

Stretch goal for the MVP; designed now so the model keeps what it needs.

## What is saved

`crates/tmxr-server/src/resurrect.rs`, following tmux-resurrect's defaults:

- Every session: name, start directory, window order, current and last window.
- Every window: index, name (and whether it was auto-named), layout tree
  (`hjkl-layout` shape + ratios, serialised by tmxr since the tree type is not
  serde), zoom, active pane, `synchronize-panes`.
- Every pane: current directory, and the **foreground program's name** if it is
  in `resurrect.processes` (tmux-resurrect's default list:
  `vi vim nvim emacs man less more tail top htop irssi weechat mutt`, plus
  `hjkl` and `sqeel`). Anything else restores as a shell in the same directory.

Not saved: pane titles, a window's last pane, program arguments, pane contents
(tmux-resurrect's `capture-pane-contents`).

## Format and location

`<data dir>/tmxr/resurrect/` (via `hjkl-xdg`; `~/.local/share` on Linux):

- `tmxr-<unix millis>.json` — one save. `serde_json`, versioned
  (`{"version": 1, …}`), pretty-printed on purpose: people hand-edit these. A
  save with another version is refused, not guessed at.
- `last` — a small file naming the newest save (a symlink on Unix would not work
  on Windows; a pointer file works everywhere).
- Both are written to a temporary file and renamed into place. Old saves are
  pruned to the newest `resurrect.keep`.

## Triggers

- `prefix C-s` → `resurrect-save`; `prefix C-r` → `resurrect-restore`.
- Periodic auto-save every `resurrect.auto-save-minutes` (tmux-continuum's idea;
  0 disables) while any session exists, whether or not anything changed.
- Save when the server exits (last session closed or `kill-server`). Nothing
  saves on a signal.
- **Auto-restore**: with `resurrect.restore-on-start = true`, a newly started
  server restores the last save before handling any client. When the restore
  produced sessions, the first bare `tmxr` attaches instead of creating a new
  session.

## Restore rules

- A session that already exists by name is skipped (tmux-resurrect does the
  same), so restore is safe to run twice.
- A directory that no longer exists falls back to `$HOME`, silently.
- A saved program is started by name as the pane's command, in the pane's
  directory, so `hjkl` comes back as `hjkl` in the right place. Arguments are
  never saved, because blindly re-running a saved command line is how restore
  runs something destructive twice; an allowlist of programs whose arguments are
  safe to restore (resurrect's `~vim` strategies) is backlog.
