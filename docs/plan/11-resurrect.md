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
  `hjkl` and `sqeel`), read when the save is made. Its **arguments** too, if it
  is also in `resurrect.restore-args` (the same list without `sqeel`, whose
  arguments can hold a connection string's password): `/proc/<pid>/cmdline` on
  Linux, `KERN_PROCARGS2` on macOS, the process's command line split by
  `CommandLineToArgvW` on Windows. Anything else restores as a shell in the same
  directory.

Each window's last pane is saved too. Not saved: pane titles, arguments of
programs outside `restore-args`, pane contents (tmux-resurrect's
`capture-pane-contents`).

## Format and location

`<data dir>/tmxr/resurrect/<server>/` (via `hjkl-xdg`: `$XDG_DATA_HOME`, else
`~/.local/share`, on every OS), one directory per socket (`Endpoint::slug`:
`default` for the default Unix socket, `tmxr-<user>-<label>` for a Windows
pipe). Each server restores and prunes only its own saves, so a throwaway `-L`
server never touches the main one's:

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
  0 disables) while any session exists, writing only when the sessions changed
  since the last save (an explicit `prefix C-s` always writes).
- Save when the server exits (last session closed or `kill-server`), again only
  when something changed. On Unix, SIGTERM and SIGHUP end the server through
  that same exit, so a logout or shutdown saves; the detached Windows server
  receives no such signal.
- **Auto-restore**: with `resurrect.restore-on-start = true`, a newly started
  server restores the last save before handling any client. When the restore
  produced sessions, the first bare `tmxr` attaches instead of creating a new
  session.

## Restore rules

- A session that already exists by name is skipped (tmux-resurrect does the
  same), so restore is safe to run twice.
- A directory that no longer exists falls back to `$HOME`, and the restore
  message lists the missing directories.
- A saved program is started by name, with its saved arguments, as the pane's
  command in the pane's directory, so `less app.log` comes back as
  `less app.log` in the right place. Only allowlisted programs keep their
  arguments, because blindly re-running any saved command line is how restore
  runs something destructive twice.
