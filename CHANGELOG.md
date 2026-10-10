# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Copy-mode actions from tmux**: `other-end` (vi `o`), `scroll-middle` (vi
  `z`), `append-selection-and-cancel` (vi `A`), `toggle-position` (`P`) and
  `cursor-centre-vertical` / `cursor-centre-horizontal` (emacs `C-l` / `M-l`),
  bound as tmux 3.6 binds them.
- **More of tmux's default binds**: the wheel over a window's name on the status
  line switches windows, the middle button pastes the latest buffer, and vi copy
  mode has `BSpace`, `Home`, `End`, `J`, `K`, `C-Up`, `C-Down` and `C-c`, as
  tmux 3.6 binds them.
- **`{ … }` command blocks**, as tmux's: a block is one argument holding the
  commands inside (one per line or `;`), so tmux binds such as
  `if-shell -F 1 { cmd ; cmd }` work in tmxr's commands and binds. Unlike tmux,
  the commands in a block are checked when they run, not when parsed.
- **`-F` and `-f` on the list commands**: `list-sessions`, `list-windows`,
  `list-panes`, `list-clients` and `list-buffers` print each item with a format
  and keep only those a filter is true for, as tmux's do; `list-panes -s` lists
  a session's panes. New variables: `pane_left`, `pane_top`, `pane_right`,
  `pane_bottom`, `pane_at_left` / `_right` / `_top` / `_bottom`, `client_name`,
  `client_width`, `client_height`, `client_session`, `client_termname`, and in
  `list-buffers` `buffer_name`, `buffer_size`, `buffer_sample`.
- **Alert actions**: `bell-action`, `activity-action` and `silence-action`
  (`any`, `none`, `current`, `other`) choose which windows' alerts act, and
  `visual-bell`, `visual-activity` and `visual-silence` (`off`, `on`, `both`)
  whether they ring the client's terminal bell, show "Bell in window 1" (or "in
  current window"), or both, with tmux's defaults; `alert-bell`,
  `alert-activity` and `alert-silence` hooks run when one acts. Alerts used to
  set window flags only, so a bell never reached the terminal.
- **Other users**: `server-access` (`-a` / `-d` user, `-r` / `-w`, `-l`) admits
  other local users to a server, read-only or not, checked on every connection
  by the user its process runs as. The new `socket-access = "users"` config
  option, read when the server starts, opens the socket file (`0666`) or pipe
  (authenticated users) for them; on Unix give the server a `-S` path they can
  reach, as with tmux.
- `attach -r`: a read-only client, whose keys reach no pane and whose commands
  are limited to attaching, detaching, switching and looking.
- **`prefix y`** copies the shell's command line to the clipboard, as
  tmux-yank's does, through the new `copy-command-line` command: tmxr moves the
  shell's cursor to the line's start and end itself (`Home` / `End` for
  PowerShell and cmd, `C-a` / `C-e` elsewhere) and waits for the pane to settle
  rather than for a fixed time. A shell that sends OSC 133 prompt marks needs no
  keys at all: the copy starts where its prompt said the input does.

- `lock-after-time`: lock a client after that many idle seconds (0, the default,
  never does).

- `-e NAME=VALUE`, as often as needed, on `new-window`, `split-window`,
  `respawn-pane` and `respawn-window` (the new pane's environment) and
  `new-session` (the session's); `display-popup -e` may now be repeated too.

- `pipe-pane -I`: the command's output is typed into the pane (`-IO` for both
  directions); closing such a pipe stops its command.

- `display-menu` and `display-popup` follow tmux's style flags: `-s` (the box),
  `-S` (its border), `-H` (a menu's selected item) and `-b` (border lines:
  single, rounded, double, heavy or none). They were accepted and ignored.

- `display-popup -x` / `-y` take tmux's position letters (`R` the right edge,
  `S` above the status line, `P` by the pane, `M` at the mouse, `W` under the
  window's name) and expand formats, and a popup or menu with no `-t` takes its
  formats from the client's current pane.

- Window and pane hooks: `set-hook -w` / `-p` (and `show-hooks -w` / `-p`). An
  event runs its pane's hooks, else its window's, its session's, the global
  ones; a respawned pane keeps its hooks.

- `select-pane -T title` titles a pane (`#{pane_title}`), and resurrect saves
  and restores pane titles.

### Changed

- **Popup and menu placement is tmux's**: `-x` / `-y` are worked out as tmux 3.6
  does, checked against a running tmux. A number for `-y` is now the box's
  bottom edge, not its top; `C` centres as tmux does; `-x R` is the target
  pane's right edge, not the client's; `M` centres the box on the mouse; `-y W`
  works; tmux's `popup_*` variables (`#{popup_pane_right}` and the rest) can be
  used; a letter with nothing to place by (`M` with no mouse event) is 0 instead
  of an error. `display-menu` now follows `-x` / `-y` too; it was always
  centred.
- `wait-for` works anywhere a command list runs, as in tmux: in the middle of a
  list (the rest waits) and from binds, hooks and the prompt. It had to end its
  list and come from a script.

- A `-S` socket's directory is no longer required to be private to its owner, as
  in tmux; a `-L` label's directory still is.

### Fixed

- **`new-session -d -x W -y H` makes a W x H window**, as tmux does; a row was
  taken off for the status line.
- **`kill-server` could fail on Linux and macOS** with "server closed the
  connection": the server exited before its reply went out, and a command run
  straight after could reach the server as it closed. The server now removes its
  socket as soon as it starts to exit and sends what it owes its clients before
  it goes.
- **Alert flags follow tmux**: a window is flagged for a bell, activity or
  silence unless a client is looking at it. A bell in the window on screen used
  to flag it anyway, and the current window of a session nobody was attached to
  was never flagged for activity or silence.
- **Passthrough lands where it was printed**: a pane's `tmux;` passthrough (an
  inline sixel, say) is now sent to the terminal from the cell the program's
  cursor was on, inside the pane, as tmux does. It used to go from wherever the
  previous frame left the terminal's cursor, which is stale when the program
  printed text just before the image.
- Resurrect keeps a window's `resize-window` size; it was restored following its
  clients again.

- Resurrect saved a window linked into several sessions once per session, so a
  restore made separate copies of it; it is now saved once and linked again.

- A `display-popup` box stays inside its client when the client is resized:
  moved in, and shrunk when it no longer fits. It used to keep its place and
  size, running off the screen.

- Windows: a pane's program, and the directory splits open in, could be read
  from an unrelated process. Windows keeps a dead parent's pid in its children
  and reuses pids, so an orphan of an older process could pass for the pane
  program's child; only processes started after the pane's program now count.

## [0.3.0] - 2026-10-10

### Added

- tmux's `clear-history` (`clearhist`), which drops a pane's history and keeps
  its screen, and `respawn-window` (`respawnw`), which takes a window back to
  its first pane and restarts it (`-k` when a program still runs). The
  `#{history_size}` format reports a pane's lines of history.
- `pipe-pane` (`pipep`): copy a pane's output to a shell command
  (`pipep 'cat >> ~/pane.log'`), with `-o` to toggle and `#{pane_pipe}` to show
  it. `-I` (the command's output into the pane) is not supported.
- **Prompt history**: Up / Down in a prompt recall earlier entries of its type
  (`command-prompt -T`; the copy-mode search binds use `search`), kept up to
  `prompt-history-limit`; `show-prompt-history` (`showphist`) and
  `clear-prompt-history` (`clearphist`) list and clear them.
- `wait-for` (`wait`): a command client blocks until another signals the channel
  (`-S`), or takes turns at its lock (`-L` / `-U`), for scripts that synchronise
  with tmxr.
- **Hooks**: `set-hook` and `show-hooks`, global (`-g`) or per session, with
  `-a` to append, `-u` to remove and `-R` to run now. `after-<command>` fires
  for every command, in the session it targeted, and `client-attached`,
  `client-detached`, `pane-exited`, `session-closed` and `session-created` for
  those events. Pane and window hooks (`-p`, `-w`) are not supported.
- `display-menu` (`menu`): a menu of commands, picked with the arrows and Enter
  or by an item's key; empty names are separators and `-` names are shown
  disabled. tmxr centres it; tmux's placement and style flags are accepted and
  not followed.
- `lock-client` (`lockc`), `lock-session` (`locks`) and `lock-server` (`lock`):
  the client hands its terminal to the new `lock-command` option (tmux's
  `lock -np` by default) and takes it back when that exits. The client reads no
  keys meanwhile, so a password goes to the lock command, not a pane. A lock
  command that fails or exits non-zero is reported in the status line.
- `resize-window` (`resizew`): give a window a size of its own (`-x` / `-y`,
  `-L` / `-R` / `-U` / `-D` by an amount, `-A` / `-a` for the largest or
  smallest client) that client resizes leave alone, until
  `set-window-option window-size latest`. The rest of the screen is filled with
  `·`. The `#{window_width}` and `#{window_height}` formats are new.
- `link-window` (`linkw`) and `unlink-window` (`unlinkw`): one window shown in
  several sessions, with `-k` to replace the window at the target index and
  `unlink-window -k` to kill a window's last link. A linked window outlives a
  killed session that held it, and `#{window_linked}` says whether it is linked.
- `display-popup` (`popup`): run a command in a box over the panes, which takes
  the client's keys. `-E` closes it when the command exits (`-EE` only on
  success); otherwise it stays until a key. `-w` / `-h` in cells or percent,
  `-x` / `-y`, `-T` title, `-B` no border, `-d` directory (default: the pane's
  current one), `-e` one variable, `-C` to close.
- `customize-mode`: a picker over every option and key bind, each shown as the
  command that sets it; Enter puts that command in the prompt to edit and run.
  `-f` opens it filtered.
- **Activity and silence alerts**: `monitor-activity` flags a window that prints
  while out of sight (`#`), and `monitor-silence N` one quiet for N seconds
  (`~`), globally with `-g` or per window. `M-n` / `M-p` (`next-window -a` /
  `previous-window -a`) now stop at these as well as bells;
  `#{window_activity_flag}` and `#{window_silence_flag}` are new.
- Mouse keys for the status line's parts, as in tmux: `…StatusLeft` and
  `…StatusRight` for `status-left` and `status-right`, `…StatusDefault` for the
  rest outside the window list (`MouseDown3StatusLeft`, `WheelUpStatusDefault`,
  …). `…Status` is now only a click on a window in the list.
- `rgb-colour` option: `on` (the default) sends 24-bit colour as before, `off`
  maps it to the nearest of the 256 colours (tmux's own mapping) for terminals
  without true colour, such as macOS Terminal.app, and `auto` decides per client
  from `COLORTERM`, a `-direct` `TERM` or Windows Terminal.
- `resurrect.processes` takes tmux-resurrect's `~text` (restore a program whose
  command line holds `text`, with its arguments) and `match->command` (restore a
  command line starting with `match` as `command`) forms.
- `run-shell -d delay` (seconds, before it runs; with no command, only the
  wait), `-C` (run a tmux command instead of a shell one) and `-c directory`.
- `SecondClick1Pane` (and the other `SecondClick` keys): the second press of a
  double click.

### Changed

- `run-shell` and `if-shell` wait, as tmux's do unless `-b`: the commands after
  them in their list run once they finish (for `if-shell`, after the command it
  chose, and a `run-shell` in that command is waited for too), and run from a
  script its output goes to the script's standard output (it used to be shown on
  the attached client, with the rest of the list already run). `-b` keeps them
  in the background.
- `DoubleClick` keys are sent once the click time passes after a second press,
  and not at all when a third press follows, as in tmux: a triple click used to
  fire the double click's bind as well.
- The default double- and triple-click binds show the selection for 0.3 s before
  copying it, as tmux's do.
- `find-window` (`prefix f`) matches as tmux's does: the text in window names,
  pane titles and visible pane contents (`-N` / `-T` / `-C` to pick, `-i` to
  ignore case, `-r` for a regular expression), and lists only the windows that
  match. It used to open every window with the text as a fuzzy filter on names.
- `choose-tree`, `choose-buffer`, `choose-client` and `find-window` run from a
  script open on the attached client typed into last, as in tmux, rather than
  failing with `no current client`.
- The client/server protocol is now version 4, for locking: after upgrading,
  restart a running server (`tmxr kill-server`) before attaching.

### Fixed

- Shell commands (`new-window 'cmd'`, `run-shell`, `if-shell`, `copy-pipe`,
  `pipe-pane`, popups) now run with `default-shell` when it is set, as in tmux,
  each with that shell's own flag (`/c` for cmd, `-Command` for PowerShell, `-c`
  for the rest). They used `$SHELL` on Unix and the first PowerShell or cmd
  found on Windows, whatever `default-shell` said.
- `move-window -k` onto the only window of a session ended that session first,
  so the move then failed with `no such session`; the window is now replaced in
  place, as in tmux.

## [0.2.4] - 2026-10-09

### Added

- **Package channels**: releases now publish to the AUR (`tmxr-bin`), the
  kryptic-sh Homebrew tap and Scoop bucket, Alpine (`.apk` on the release) and
  crates.io, as `tmxr-cli` (the name `tmxr` is another project's there); the
  binary is still `tmxr`.
- tmux's `move-pane` (`movep`), `previous-layout` (`prevl`),
  `show-window-options` (`showw`) and `start-server` (`start`), so their names
  and shorthands work as in tmux.
- `set-environment` (`setenv`) and `show-environment` (`showenv`): a global
  environment (`-g`) over the server's own and one per session, with `-u` to
  forget a value and `-r` to keep a variable out of new panes; `-s` prints shell
  commands.

### Fixed

- The server no longer keeps open what the program that started it inherited. A
  script reading the output of something that ran `tmxr new -d` waited until the
  tmxr server exited; on Windows the server got every inheritable handle, and on
  Unix every descriptor not marked close-on-exec.
- Windows: a 32-bit program's current directory was read from the WOW64 layer's
  64-bit process block, which named another directory (`C:\WINDOWS` for a 32-bit
  `cmd`), so splits from it opened there. It is now read from the program's own
  32-bit block.

## [0.2.3] - 2026-10-09

### Added

- **Inline images on Windows**: the Windows release bundles Microsoft's ConPTY
  (`conpty.dll`, `OpenConsole.exe`, MIT), which keeps the end of a program's
  tmux passthrough that Windows' own ConPTY drops; tmxr turns passthrough on
  when they sit beside `tmxr.exe`.
- **Windows: sessions are saved at logoff and shutdown**, as SIGTERM / SIGHUP
  already did on Unix. The server keeps a hidden window for Windows' session-end
  message; before, only the periodic auto-save protected them.

## [0.2.2] - 2026-10-09

### Added

- `select-pane -Z` keeps a zoomed window zoomed on the pane it selects, as in
  tmux (tmxr-navigator.nvim's `preserve_zoom` uses it).

## [0.2.1] - 2026-10-09

### Fixed

- Windows panes keep the environment of the shell that started tmxr. They used
  to get the registry's variables in its place, so a `PATH` extended before
  starting tmxr (a venv, a toolchain, a dev build) was lost in every pane.

## [0.2.0] - 2026-10-09

### Added

- **Mouse keys**: `MouseDown1Pane`, `MouseDrag1Border`, `MouseDragEnd1Pane`,
  `WheelUpPane`, `MouseDown1Status` and the rest of tmux's mouse key names can
  be bound and unbound in any key table, with `-t =` for the pane or window
  under the mouse and the `-M` flags of `send-keys`, `copy-mode` and
  `resize-pane`. The mouse behaviour that was built in is now default binds, so
  `unbind -n WheelUpPane` (for example) turns one part of it off.
- `copy-mode -e`: scrolling back to the bottom leaves copy mode, as the wheel
  does. The flag was accepted before but did nothing.
- `#{mouse_any_flag}`: whether the pane's program asked for the mouse.
- **Emacs copy mode**: `mode-keys emacs` switches copy mode to tmux's emacs
  table (`copy-mode`: `C-Space`, `M-w`, `C-s` / `C-r`, `M-1`… counts, …).
  `mode-keys` now rejects values other than `vi` and `emacs`.
- **Resurrect restores arguments** for programs in the new
  `resurrect.restore-args` list (tmux-resurrect's defaults and `hjkl`), so
  `less app.log` comes back as `less app.log`, not `less`.
- Copy-mode commands `previous-matching-bracket`, `next-paragraph` /
  `previous-paragraph`, `goto-line` and `copy-end-of-line` (with its
  `-and-cancel` and `copy-pipe-` forms), bound as in tmux: `{` `}` `:` `D` in vi
  mode, `C-M-b` `M-{` `M-}` `g` `C-k` in emacs mode.
- `DoubleClick1Pane` / `TripleClick1Pane` mouse keys, bound by default to copy
  the word or line clicked (the copy-mode `select-word` command is new too).
- **Windows: splits and new windows open in cmd's current directory** without
  any shell integration: tmxr reads the directory from the program's process
  (its PEB). What a shell announces (OSC 7 / OSC 9;9) still comes first, which
  PowerShell needs.
- **Incremental search**: emacs copy mode's `C-s` / `C-r` move to the match as
  you type, through `command-prompt -i` and the `search-forward-incremental` /
  `search-backward-incremental` copy commands.
- nvim support for `C-h/j/k/l` lives in the new
  [tmxr-navigator.nvim](https://github.com/kryptic-sh/tmxr-navigator.nvim)
  plugin, tmxr's vim-tmux-navigator.

### Changed

- **Pickers work like hjkl's**: one mode, where typing always filters, arrows
  and `C-n` / `C-p` move and `Escape` closes. The vi-style normal mode (`j` /
  `k`, `g` / `G`, `C-j` / `C-k`) is gone.

### Fixed

- A pane printing faster than the server can parse (`seq` on Linux) no longer
  delays key presses by seconds or grows the server's memory without bound: each
  pane's output waits in a bounded queue that slows the program down, and the
  server repaints during a flood.
- `C-Space` binds now fire on Windows, where the terminal reports the key (byte
  0x00) as `C-2`; `C-@` names the same key, as in tmux.
- A save made right after a program started (`prefix C-s`) now records it; saves
  used to read a list of foreground programs refreshed only on the status tick.

## [0.1.0] - 2026-10-09

### Added

- **First implementation of the tmxr client/server.** A detached server is
  started on demand and owns sessions, windows and panes (openpty / ConPTY,
  `vt100` emulation); clients attach over a Unix socket or an owner-only named
  pipe, or run one command (`tmxr ls`, `tmxr split-window -h`, …) in tmux's
  command language.
- **Built-in defaults port the owner's tmux config**: vim pane selection, splits
  and new windows in the current directory, `M-h`/`M-l`, synchronize panes on
  `prefix x`, vi copy mode with `v`/`C-v`/`y`, tmux-sensible and tmux-yank
  binds, tmux's own default binds (`display-panes` numbers on `prefix q`, alert
  windows on `M-n`/`M-p`, even spread on `prefix E`, …), vim/hjkl-aware
  `C-h/j/k/l` navigation, Tokyo Night status line in the catppuccin layout.
  Overridable from `~/.config/tmxr/config.toml`.
- **The tmux command set of the plan's MVP**, including moving windows and panes
  between sessions (`move-window`, `swap-window`, `join-pane`, `swap-pane -s`),
  `respawn-pane`, `if-shell` and buffers to and from files.
- **Fuzzy pickers** on `hjkl-picker` for sessions (`prefix s`), windows
  (`prefix w`, and `prefix f` to find one) and paste buffers (`prefix =`),
  moving with `j`/`k` or arrows.
- **Mouse** (`mouse on`): click to focus, drag borders, wheel into copy mode,
  drag to copy. With `mouse off` the client leaves the mouse to the terminal.
- **tmux passthrough** (`allow-passthrough`, on by default as in the tmux
  config) forwards programs' `ESC P tmux; …` sequences, such as inline images,
  to the outer terminal. Unix only: ConPTY drops the sequence's terminator.
- **Save and restore of sessions** (`prefix C-s` / `C-r`), auto-save and restore
  when the server starts. Each server (`-L` label or `-S` socket) keeps its own
  saves.
- **Release binaries** for Linux (glibc 2.28 and musl, x86_64 and aarch64, plus
  `.deb` and `.rpm`), macOS (Apple silicon and Intel) and Windows (x86_64),
  published to GitHub Releases with `.sha256` files, and shell completions and a
  man page from the hidden `tmxr --completions <shell>` and `tmxr --man`.

[Unreleased]: https://github.com/kryptic-sh/tmxr/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/kryptic-sh/tmxr/compare/v0.2.4...v0.3.0
[0.2.4]: https://github.com/kryptic-sh/tmxr/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/kryptic-sh/tmxr/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/kryptic-sh/tmxr/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/kryptic-sh/tmxr/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/kryptic-sh/tmxr/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/kryptic-sh/tmxr/releases/tag/v0.1.0
