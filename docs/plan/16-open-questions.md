# 16 — Open questions

Decisions the plan made provisionally that the owner may want to overrule. Each
has the default the implementation follows until told otherwise.

1. **`base-index`.** The tmux config comments "Start windows and panes at 1, not
   0" but sets `base-index 0` / `pane-base-index 0`. tmxr follows the values
   (0). Flip both defaults to 1 if the comment is what was meant.
2. **`prefix x` / kill-pane.** The config rebinds `x` to synchronize-panes,
   which removes tmux's only kill-pane bind. tmxr adds `prefix X` →
   `confirm-before kill-pane` (a new bind, not from the config). Drop it if
   unwanted.
3. **Theme.** The tmux config uses catppuccin mocha; the request says Tokyo
   Night "just like my config" — which matches the rest of the dotfiles
   (alacritty, fish, bat, nvim). tmxr uses Tokyo Night colours with the
   catppuccin status-line layout. Role mapping in
   [07](07-rendering-status-theme.md#ui-roles).
4. **Config format.** TOML (house style, `hjkl-config`) with tmux command
   strings for binds, not a `tmux.conf` reader. A `tmxr import-tmux-conf`
   converter is possible later.
5. **vim (not hjkl) navigation.** vim-tmux-navigator in nvim calls `tmux`, so it
   does not hand off to tmxr. Supported route: hjkl (which `vim` is aliased to).
   An nvim snippet calling `tmxr select-pane` is a backlog item.
6. **Session picker opening mode.** Opens in query (insert) mode so typing
   filters immediately; `j`/`k` move after `Escape`, arrows and `C-j`/`C-k` move
   in both modes. The alternative is opening in normal mode (`j`/`k`
   immediately, `/` to filter).
7. **Windows current directory.** Without shell integration (OSC 7 / OSC 9;9)
   `#{pane_current_path}` on Windows is the pane's start directory, so
   split-in-current-dir uses the directory the pane started in. Accept, or
   invest in PEB reading.
