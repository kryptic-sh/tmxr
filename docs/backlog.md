# Backlog

Work raised and not finished, decisions still open, and gaps in what has been
verified. Delete an entry when it ships; `git log` keeps the history.

## Decisions awaiting the owner

The provisional decisions in
[plan/16-open-questions.md](plan/16-open-questions.md) (index base, `prefix X`
kill-pane, theme, config format, vim navigation, picker mode, Windows current
directory) are defaults the implementation follows until answered.

## Cross-repo work

- **hjkl `$TMXR` fall-through.** `dispatch_tmux_navigate` in
  `hjkl/apps/hjkl/src/app/window.rs` only hands off to `tmux select-pane` when
  `$TMUX` is set. It needs the same branch for `$TMXR` → `tmxr select-pane` for
  the `C-h/j/k/l` navigator to cross from hjkl splits into tmxr panes (milestone
  M4).
- **nvim navigation.** vim-tmux-navigator shells out to `tmux`; a snippet or
  plugin option calling `tmxr select-pane` is needed for plain nvim.

## Known gaps in the design

- **DCS passthrough** (`allow-passthrough on` in the tmux config, used by image
  protocols) is not supported by `vt100`; needs a pre-parser or a vt100 patch.
- **Windows `pane_current_path`** relies on shell integration (OSC 7 / OSC 9;9);
  without it splits open in the pane's start directory.
- **Windows pipe DACL**: whether `interprocess` can set a current-user-only
  security descriptor is unverified; the default DACL is the fallback.

## Not yet verified

- The release pipeline (phase 2 of plan/14) does not exist yet; nothing about
  packaging has been exercised.
