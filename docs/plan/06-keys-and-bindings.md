# 06 — Keys and bindings

## Key tables

As in tmux: named tables, each a map from key to a command list plus an optional
note (`bind -N`).

- `root` — consulted for every key.
- `prefix` — entered after the prefix key (`C-b`); returns to `root` after one
  key unless the bind was made with `-r` (repeatable within `repeat-time`, 500
  ms).
- `copy-mode-vi` — while a pane is in copy mode (`mode-keys vi`).
- Overlay tables: `picker`, `prompt` (handled by the picker/prompt widgets, not
  user-rebindable in the MVP).

Lookup order for a key: the client's current table; if unbound there and the
table is `root`, the key goes to the active pane. Unbound keys in `prefix` are
swallowed (tmux behaviour). `switch-client -T <table>` is supported so custom
tables work.

Key names are tmux's: `C-x`, `M-x`, `S-x`, `C-M-x`, `Enter`, `Escape`, `Tab`,
`BTab`, `Space`, `BSpace`, `Up`, `PPage`/`PageUp`, `F1`…`F24`, and literal
punctuation (`'`, `"`, `;`, `%`, `\`). hjkl's `<C-x>` notation is not accepted.

Mouse events are not keys in tables: `mouse on` behaviour (click to focus, drag
a border, wheel and drag into copy mode, drag-end copy, status-line clicks) is
built into the server (`crates/tmxr-server/src/mouse.rs`) and matches tmux's
default mouse binds plus tmux-yank's `MouseDragEnd1Pane`; it cannot be rebound.

`list-keys -N` (bound to `prefix ?`) shows every bind that has a note — the
default config gives every bind a note, matching what `plugin-notes.sh` does for
the tmux plugins.

## Default bindings

This is the tmux config, its plugins and `plugin-notes.sh`, resolved into one
table. **(cfg)** = set explicitly in `tmux.conf`; **(nav)** =
vim-tmux-navigator; **(sens)** = tmux-sensible; **(yank)** = tmux-yank;
**(res)** = tmux-resurrect; **(tmux)** = tmux built-in default kept as is.

### `root`

| Key   | Command                                        | Note                                                   | From  |
| ----- | ---------------------------------------------- | ------------------------------------------------------ | ----- |
| `C-h` | `select-pane -L` unless navigator app in front | Focus pane left, or pass the key to vim/fzf-like apps  | (nav) |
| `C-j` | `select-pane -D` unless navigator app in front | Focus pane down, or pass the key to vim/fzf-like apps  | (nav) |
| `C-k` | `select-pane -U` unless navigator app in front | Focus pane up, or pass the key to vim/fzf-like apps    | (nav) |
| `C-l` | `select-pane -R` unless navigator app in front | Focus pane right, or pass the key to vim/fzf-like apps | (nav) |
| `C-\` | `select-pane -l` unless navigator app in front | Focus last pane, or pass the key to vim/fzf-like apps  | (nav) |
| `M-h` | `previous-window`                              | Previous window                                        | (cfg) |
| `M-l` | `next-window`                                  | Next window                                            | (cfg) |
| mouse | tmux's default mouse binds (`mouse on`)        | click focus, drag-resize borders, wheel → copy mode    | (cfg) |

### `prefix` (after `C-b`)

| Key                     | Command                                              | Note                                               | From                  |
| ----------------------- | ---------------------------------------------------- | -------------------------------------------------- | --------------------- |
| `h` `j` `k` `l`         | `select-pane -L/-D/-U/-R`                            | Focus pane left/down/up/right                      | (cfg)                 |
| `'` `"`                 | `split-window -v -c "#{pane_current_path}"`          | Split pane top/bottom in current dir               | (cfg)                 |
| `;` `%`                 | `split-window -h -c "#{pane_current_path}"`          | Split pane left/right in current dir               | (cfg)                 |
| `c`                     | `new-window -c "#{pane_current_path}"`               | New window in current dir                          | (cfg)                 |
| `x`                     | `set-window-option synchronize-panes`                | Toggle synchronized input to all panes             | (cfg)                 |
| `C-l`                   | `send-keys C-l`                                      | Clear screen (CTRL + l is taken by navigation)     | (nav)                 |
| `b`                     | `last-window`                                        | Last window                                        | (sens)                |
| `C-n` / `n`             | `next-window`                                        | Next window                                        | (sens)/(tmux)         |
| `C-p` / `p`             | `previous-window`                                    | Previous window                                    | (sens)/(tmux)         |
| `q`                     | `display-panes` (numbers over panes; a digit picks)  | Show pane numbers                                  | (tmux)                |
| `#` / `-`               | `list-buffers` / `delete-buffer`                     | List / delete paste buffers                        | (tmux)                |
| `.`                     | `move-window` prompt                                 | Move window                                        | (tmux)                |
| `M-o`                   | `rotate-window -D`                                   | Rotate panes down                                  | (tmux)                |
| `M-n` / `M-p`           | `next-window -a` / `previous-window -a`              | Next / previous window with an alert (a bell)      | (tmux)                |
| `E`                     | `select-layout -E`                                   | Spread panes out evenly                            | (tmux)                |
| `R`                     | `source-file` (reload config)                        | Reload config                                      | (sens)                |
| `C-b`                   | `send-prefix`                                        | Send the prefix key                                | (tmux)                |
| `s`                     | session picker ([09](09-session-picker.md))          | Switch session                                     | (tmux, reimplemented) |
| `w`                     | window picker (same widget, windows of all sessions) | Switch window                                      | (tmux, reimplemented) |
| `d`                     | `detach-client`                                      | Detach                                             | (tmux)                |
| `$` / `,`               | `rename-session` / `rename-window` prompt            | Rename session / window                            | (tmux)                |
| `0`–`9`                 | `select-window -t :=N`                               | Select window N                                    | (tmux)                |
| `:`                     | `command-prompt`                                     | Command prompt                                     | (tmux)                |
| `?`                     | `list-keys -N`                                       | List key binds                                     | (tmux)                |
| `[` / `PageUp`          | `copy-mode` / `copy-mode -u`                         | Copy mode                                          | (tmux)                |
| `]`                     | `paste-buffer -p`                                    | Paste the most recent buffer                       | (tmux)                |
| `z`                     | `resize-pane -Z`                                     | Zoom pane                                          | (tmux)                |
| `!`                     | `break-pane`                                         | Break pane into a window                           | (tmux)                |
| `{` / `}`               | `swap-pane -U` / `swap-pane -D`                      | Swap pane up/down                                  | (tmux)                |
| `o` / `C-o`             | `select-pane -t :.+` / `rotate-window`               | Next pane / rotate panes                           | (tmux)                |
| `Space`                 | `next-layout`                                        | Next layout                                        | (tmux)                |
| `M-1`…`M-5`             | `select-layout` presets                              | Layout presets                                     | (tmux)                |
| arrows                  | `select-pane -L/-D/-U/-R`                            | Focus pane                                         | (tmux)                |
| `C-`arrows / `M-`arrows | `resize-pane` by 1 / 5 (repeatable)                  | Resize pane                                        | (tmux)                |
| `(` / `)` / `L`         | `switch-client -p` / `-n` / `-l`                     | Previous / next / last session                     | (tmux)                |
| `&`                     | `confirm-before kill-window`                         | Kill window                                        | (tmux)                |
| `X`                     | `confirm-before kill-pane`                           | Kill pane (tmux's `x`; moved because `x` is sync)  | new                   |
| `i` / `~`               | `display-message` / `show-messages`                  | Pane info / message log                            | (tmux)                |
| `r`                     | `refresh-client`                                     | Redraw                                             | (tmux)                |
| `y`                     | copy the pane's current command line to clipboard    | Copy the command line to the clipboard             | (yank) stretch        |
| `Y`                     | copy `#{pane_current_path}` to clipboard             | Copy the pane's working directory to the clipboard | (yank)                |
| `C-s` / `C-r`           | resurrect save / restore                             | Save sessions / Restore saved sessions             | (res) stretch         |

Dropped from tmux's defaults because the config reuses the key: `'`
(select-window prompt), `;` (last-pane — `C-\` covers it), `l` (last-window —
`b` covers it), `x` (kill-pane — moved to `X`; **new**, flagged in
[16-open-questions.md](16-open-questions.md)). `I`/`U`/`M-u` (tpm) have nothing
to do and are not bound. tmux defaults with no tmxr command yet (`t`, `f`, `m`,
`M`, `D`, `=`, `/`, `C-z`) are unbound until the commands exist; the backlog
tracks them.

### `copy-mode-vi`

The core of tmux's default `copy-mode-vi` table (motions
`h j k l w b e W B E 0 ^ $ g G H M L` and the arrows, `C-u C-d C-b C-f C-y C-e`,
`PPage`/`NPage`, search `/ ? n N`, `Space` begin selection, `V` select line,
`Enter` copy and cancel, `q` / `Escape` cancel), plus the binds below. tmux's
`f F t T ; ,`, counts, marks and `%` are not implemented
([08](08-copy-mode.md#engine)).

| Key                 | Command                                          | From         |
| ------------------- | ------------------------------------------------ | ------------ |
| `v`                 | `begin-selection`                                | (cfg)        |
| `C-v`               | `rectangle-toggle`                               | (cfg)        |
| `y`                 | `copy-selection-and-cancel` → buffer + clipboard | (cfg)/(yank) |
| `Y`                 | copy, then `paste-buffer -p` into the pane       | (yank)       |
| `M-y`               | copy to clipboard and paste                      | (yank)       |
| `!`                 | copy without newlines                            | (yank)       |
| `MouseDragEnd1Pane` | copy mouse selection when the drag ends          | (yank)       |
| `C-h/j/k/l`, `C-\`  | `select-pane -L/-D/-U/-R/-l`                     | (nav)        |

## vim / hjkl navigation

This replaces vim-tmux-navigator end to end.

**tmxr side.** `C-h/j/k/l` and `C-\` in `root` run the built-in
`navigate-pane -L|-D|-U|-R|-l` command:

1. If the active pane's foreground command matches `navigator-pattern`, send the
   key to the pane.
2. Otherwise `select-pane` in that direction.

`navigator-pattern` defaults to the config's `@vim_navigator_pattern`:

```
(\S+/)?g?\.?(view|l?n?vim?x?|fzf|sqeel|hjkl)(diff)?(-wrapped)?
```

matched (anchored, like the plugin's `grep -iqE "^…$"`) against the foreground
process name from [04](04-panes-and-terminal.md#process-inspection).
`@tmux_navigator_disable_when_zoomed` is honoured as
`navigator-disable-when-zoomed` (default off).

**Editor side.** When vim/hjkl is at the edge of its own splits it must hand the
move back:

- **hjkl** already does this for tmux (`dispatch_tmux_navigate` in
  `apps/hjkl/src/app/window.rs` runs `tmux select-pane -L` when `$TMUX` is set).
  It gains the same fall-through for `$TMXR` → `tmxr select-pane -L`. That is a
  small change in the hjkl repo, tracked as milestone M4.
- **vim/nvim** with vim-tmux-navigator: the plugin shells out to `tmux` with
  `$TMUX`. tmxr does not set `TMUX` (it would point real tmux at a socket it
  cannot speak). The supported route is hjkl (`vim` is aliased to hjkl in the
  user's fish config). A small nvim snippet that calls `tmxr select-pane` is a
  backlog item.
