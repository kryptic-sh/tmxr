# 07 — Rendering, status line, theme

## Compositor

For each attached client the server keeps a `ratatui::Terminal` over a custom
`Backend` (`MemoryBackend`) whose size is the client's size and whose writes go
into a `Vec<u8>` instead of a tty. The backend is ~100 lines: it stores the
size, and implements `draw` / cursor / clear by emitting the same crossterm
commands `CrosstermBackend` would, into the buffer. ratatui's double buffering
then gives us **diffed frames for free** — each `Output` sent to a client is
only the cells that changed since the last frame that client received.

A frame is drawn as:

1. **Panes** — for each `(pane, rect)` from `window_rects` (or the zoomed pane
   full-size), copy the `vt100::Screen` cells (or the copy-mode view) into the
   buffer: character (wide chars occupy two cells), fg/bg (indexed and RGB),
   bold/dim/italic/underline/inverse. A `PaneWidget` does this; it is small and
   tested against vt100 directly.
2. **Borders** — one-cell separators between panes, drawn with tmux's
   box-drawing junctions; the active pane's border segments use the active
   style. `pane-border-lines single`.
3. **Status line** — bottom row (`status-position bottom`, one line).
4. **Overlays** — session/window picker, command prompt, confirm prompt,
   `display-message`, `display-panes` numbers, copy-mode position indicator
   (`[12/340]` top-right of the pane, as tmux).
5. **Cursor** — the active pane's cursor (hidden if the app hid it), or the
   copy-mode cursor, or the prompt cursor; cursor shape is passed through
   (`DECSCUSR`).

Out-of-band bytes are appended to the frame's `Output` after the diff: OSC 52
clipboard writes, the bell, the client title (`set-titles`), and the client's
focus/mouse/keyboard mode switches.

Frame rate: dirty-driven. The server renders after draining every queued event,
so output that arrives together becomes one frame; there is no fixed frame
interval capping a pane that keeps spewing output.

## Theme: Tokyo Night

The colours come from the user's desktop (alacritty, fish, bat, nvim all use
folke's Tokyo Night "Night"). The canonical palette as hjkl ships it
(`hjkl/apps/hjkl/themes/tokyonight.toml`):

| Role           | Hex       | Role             | Hex       |
| -------------- | --------- | ---------------- | --------- |
| `bg`           | `#1a1b26` | `blue`           | `#7aa2f7` |
| `bg_dark`      | `#16161e` | `cyan`           | `#7dcfff` |
| `bg_highlight` | `#292e42` | `magenta`        | `#bb9af7` |
| `fg`           | `#c0caf5` | `purple`         | `#9d7cd8` |
| `fg_dark`      | `#a9b1d6` | `green`          | `#9ece6a` |
| `fg_gutter`    | `#3b4261` | `yellow`         | `#e0af68` |
| `comment`      | `#565f89` | `orange`         | `#ff9e64` |
| `dark5`        | `#737aa2` | `red`            | `#f7768e` |
| `selection`    | `#283457` | `terminal_black` | `#414868` |

The palette lives as `@thm_*` user options in the embedded defaults
(`crates/tmxr-config/defaults.toml`), named after catppuccin's roles, and the
status, border and mode styles reference them with `#{@thm_*}` — the same way
the catppuccin tmux plugin works. Another palette is a config file overriding
those options; there is no separate theme file or `theme` key. Colours are
always emitted as 24-bit RGB (the tmux config forces
`terminal-overrides ",*:RGB"`); there is no 256-colour fallback.

### UI roles

catppuccin's roles mapped onto the Tokyo Night palette, so the layout of the
current status line is kept and only the colours change:

| catppuccin role | Used for                                      | Tokyo Night                                   |
| --------------- | --------------------------------------------- | --------------------------------------------- |
| `mantle`        | status line background                        | `bg_dark` #16161e                             |
| `crust`         | text on coloured blocks                       | `bg_dark` #16161e                             |
| `surface0`      | window text block, module text block          | `bg_highlight`                                |
| `surface1`      | current window text block                     | `fg_gutter`                                   |
| `overlay2`      | window number block                           | `dark5`                                       |
| `mauve`         | current window number block, host icon block  | `magenta`                                     |
| `green`         | session icon block                            | `green`                                       |
| `red`           | session icon block while the prefix is active | `red`                                         |
| `text`          | text                                          | `fg`                                          |
| —               | pane border / active pane border              | `fg_gutter` / `blue`                          |
| —               | copy-mode selection, picker selection         | `selection`                                   |
| —               | messages / prompt                             | `fg` on `bg_highlight`, `yellow` for warnings |

## Status line layout

Reproduces the config's catppuccin setup:

```tmux
set -g @catppuccin_status_left_separator  "█"
set -g @catppuccin_status_right_separator "█"
set -g @catppuccin_window_current_text    " #W"
set -g @catppuccin_window_status_style    "basic"
set -g @catppuccin_window_text            " #W"
set -g  status-left ""
set -g status-right "#{E:@catppuccin_status_session}#{E:@catppuccin_status_host}"
```

```
 0   zsh  1   hjkl  2   cargo                                   █ main█ █󰒋 hostname█
└─┬─┘└──┬──┘                                                    └──── session ────┘└── host ──┘
number  text      (window list, left-justified, status-left empty)
```

- **Window list** (catppuccin `basic`): per window a number block (`#I`, fg
  `crust` on `overlay2`; current window on `mauve`) followed by a text block
  (`" #W"` + padding, fg `text` on `surface0`; current window on `surface1`).
  Window flags (zoom `Z`, synchronize, bell, activity) appear after the name.
- **Right side**: two modules, each `█` (fg = block colour) + icon block (fg
  `crust` on the module colour) + ` text` block (fg `text` on `surface0`) + `█`
  (fg `surface0`):
  - session: icon ``, text `#S`, colour `green` — `red` while the prefix key is
    pending (catppuccin's `client_prefix` cue, which is the user's current
    "prefix pressed" indicator);
  - host: icon `󰒋`, text `#H` (short hostname), colour `magenta`.
- Icons are Nerd Font glyphs, as in catppuccin; the user's terminals already use
  a Nerd Font.

The status line is built from `status-left`, `status-right`,
`window-status-format`, `window-status-current-format` in the **tmux format
language** (`#{…}`, `#[fg=…,bg=…]`, `#{?cond,a,b}`, `#S #W #I #H #F`), and the
defaults above are expressed in that language rather than hard-coded, so users
can restyle it. The format engine is in `tmxr-command` (see
[10](10-config-and-commands.md)).

`hjkl-statusline` was considered for this: its `Bar`/`Segment` model is an
editor status line (mode, file, cursor) without per-segment `#[style]` runs, so
it is used only where it fits (truncation/width math) rather than as the status
line model. Decided at implementation time; recorded either way.
