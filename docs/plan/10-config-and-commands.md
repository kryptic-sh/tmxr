# 10 — Config, commands, CLI

## Config file

House style: TOML loaded with `hjkl-config` (`load_layered`), path from
`hjkl-xdg`:

- `~/.config/tmxr/config.toml` on every platform (hjkl-xdg deliberately uses
  `~/.config` on Windows and macOS too), overridable with `-f <file>`.
- The **defaults are an embedded TOML file**
  (`crates/tmxr-config/defaults.toml`) that is the tmux config port; the user
  file is deep-merged over it. No user file at all = the tmux setup described in
  [06](06-keys-and-bindings.md).
- Errors carry file positions (hjkl-config does this) and are shown on attach as
  a message, never a crash: a bad config falls back to defaults.

Shape (illustrative, finalised in M3):

```toml
prefix = "C-b"
mouse = true
base-index = 0
pane-base-index = 0
renumber-windows = true
mode-keys = "vi"
history-limit = 50000
escape-time = 0
default-terminal = "tmux-256color"
extended-keys = "always"
update-environment = ["WAYLAND_DISPLAY", "XDG_SESSION_TYPE", "XDG_CURRENT_DESKTOP", "XDG_RUNTIME_DIR"]
theme = "tokyonight"

[navigator]
pattern = '(\S+/)?g?\.?(view|l?n?vim?x?|fzf|sqeel|hjkl)(diff)?(-wrapped)?'
disable-when-zoomed = false

[status]
left = ""
right = "#{E:@status_session}#{E:@status_host}"

[resurrect]
auto-save-minutes = 15
restore-on-start = true

# Binds: table → key → command (tmux command language) + note.
[keys.root]
"M-h" = { cmd = "previous-window", note = "Previous window" }

[keys.prefix]
"'" = { cmd = 'split-window -v -c "#{pane_current_path}"', note = "Split pane top/bottom in current dir" }
"x" = { cmd = "set-window-option synchronize-panes", note = "Toggle synchronized input to all panes" }

# Remove a default bind:
# "x" = false
```

Option names are tmux's, so docs and muscle memory carry over. `@user-options`
are allowed (formats reference them via `#{@name}` / `#{E:@name}`).

`source-file` (and `prefix R`) re-reads the file and applies it to the running
server.

## Command language (`tmxr-command`)

A tmux-compatible parser shared by config binds, the command prompt
(`prefix :`), `tmxr <command>` on the CLI, and `run-shell`/`if-shell` bodies:

- Tokenizer: tmux quoting rules (single quotes literal, double quotes with `\`
  escapes and `~`/`$VAR` expansion, `\;` and bare `;` as command separators,
  `{ … }` blocks).
- Command table: name, aliases (`splitw`, `neww`, `selectp`, …), flag spec
  (getopt-style, e.g. `"bc:dfhIl:Pt:vZ"`), min/max positional args. Unique
  prefixes resolve (`split` → `split-window`), as in tmux.
- Formats: `#{name}`, `#{?cond,a,b}`, `#{E:…}`, `#{=N:…}` (trim), `#{s/a/b/:…}`,
  `#{==:a,b}` / `#{!=:…}` / `#{||:…}` / `#{&&:…}`, short aliases
  `#S #W #I #P #H #h #F #D #T`, style runs `#[fg=…,bg=…,bold]`. Variables come
  from a context trait implemented by the server (session/window/pane/client).

### MVP command set

`new-session`, `attach-session`, `detach-client`, `kill-session`, `kill-server`,
`list-sessions`, `rename-session`, `switch-client`, `has-session`; `new-window`,
`kill-window`, `select-window`, `next-window`, `previous-window`, `last-window`,
`rename-window`, `list-windows`, `move-window`, `swap-window`; `split-window`,
`kill-pane`, `select-pane`, `last-pane`, `resize-pane`, `swap-pane`,
`break-pane`, `join-pane`, `rotate-window`, `select-layout`, `next-layout`,
`list-panes`, `display-panes`, `capture-pane`, `respawn-pane`; `send-keys`,
`send-prefix`, `bind-key`, `unbind-key`, `list-keys`, `switch-client -T`;
`copy-mode`, `paste-buffer`, `set-buffer`, `show-buffer`, `list-buffers`,
`delete-buffer`, `save-buffer`, `load-buffer`; `set-option`,
`set-window-option`, `show-options`; `source-file`, `command-prompt`,
`confirm-before`, `display-message`, `show-messages`, `refresh-client`,
`run-shell`, `if-shell`, `choose-tree` (→ picker), `list-commands`; plus tmxr's
own `navigate-pane`, `resurrect-save`, `resurrect-restore`.

Not implemented yet (unknown commands to the parser): `move-window`,
`swap-window`, `join-pane`, `respawn-pane`.

`if-shell` always runs its shell command in the background, as tmux does with
`-b`: the command client gets its reply at once, and the chosen command runs
when the shell exits (errors go to the attached client's status line and
`show-messages`). `-F` decides immediately, so its command runs as part of the
same command list. `load-buffer -` (standard input) is not supported, since
command clients do not forward their input; relative `save-buffer` /
`load-buffer` paths are relative to the command client's directory, or the home
directory for a key bind.

One deliberate difference from tmux: `set-buffer` format-expands its data, so
`prefix Y` is `set-buffer -w "#{pane_current_path}"` (tmux-yank's copy of the
pane's directory) without a helper script. tmux's `set-buffer` stores the text
literally.

## CLI (`apps/tmxr`)

clap derive, following hjkl/krypt
(`#[command(name, version, about, after_help)]`, tests via `CommandFactory`):

```
tmxr [-L label | -S socket] [-f config] [-v] [command [flags] [args]] [\; command …]
```

- No command = `new-session` (tmux behaviour). `tmxr a` / `attach` / `at`,
  `tmxr new -s name`, `tmxr ls`, `tmxr kill-server` as in tmux.
- Global flags are clap fields; **everything after them is passed to the
  server's command parser verbatim** (`allow_external_subcommands` / trailing
  var-arg), so every server command is available on the CLI without duplicating
  flag specs in clap. `tmxr --help` lists commands from the same command table.
- `-V` / `--version` prints `tmxr <version>`.
- Exit status: the command's status (1 on error, with tmux-style
  `no server running on <socket>` / `can't find session: x` messages on stderr).
- Hidden: `__server` (internal, the spawned daemon).
