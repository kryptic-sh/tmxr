# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.12.2] - 2026-10-29

### Fixed

- **Copy mode's `#{top_line_time}` follows tmux's line times through a resize**:
  a line a taller pane takes back from the history keeps its time, where it gave
  0, and a line a shorter pane pushes into the history takes none, where it took
  the time of the resize. Only output scrolling a line in stamps it, as tmux's.

## [0.12.1] - 2026-10-29

### Changed

- **Dependencies updated to their latest patch releases**, among them
  unicode-width 0.2.3 (the widths the `w:` format modifier counts), rustix 1.1.6
  and pest 2.9.5.

### Fixed

- **A client that detaches or goes away no longer holds up a pane's answers**: a
  palette colour asked of its terminal is dropped with it, as tmux drops it, so
  the program's other answers (DA1, say) reach it at once rather than after the
  2-second wait for an answer that cannot come.
- **An option index that is not a number is refused as tmux refuses it**:
  `set -g update-environment[x] …` and `show -g status-left[]` say
  `ambiguous option`, where tmxr said `invalid option`, and `-q` silences it.
- **Copy-mode searches match and count as tmux's**: a text search's matches no
  longer overlap (`aa` in `aaaa` is two, not three), in the highlighting and in
  `#{search_count}`; a match can run past a wrapped line's end onto the next
  row, where tmxr found only matches within a row; and under vi keys a search
  down starts after the match the cursor is on, even from inside it, rather than
  one cell on.

## [0.12.0] - 2026-10-26

### Breaking

- **The client-server protocol is now 13** (the client sends its terminal's
  answers to colour queries): after upgrading, a running server must be
  restarted (`tmxr kill-server`); the client says so if it meets one.

### Added

- **A palette colour a program asks for and has not set is asked of the
  terminal**, as tmux asks it: a pane's `OSC 4 ; n ; ?` for an entry it never
  set goes to the terminal of the client on its window that was used last, and
  the answer is passed on; the program's other answers wait behind it, for up to
  2 seconds if the terminal never answers. Where no such client can be asked (a
  Windows client, or none attached), the query goes unanswered as before.
- **The terminal's colours are asked again when it changes theme**: when a
  client's terminal reports a switch between light and dark, and on a resize at
  most every 30 seconds, it is asked for its foreground and background again, as
  tmux asks, so a program's OSC 10 / 11 query gets the colours it shows now
  rather than those from attach. Seen in foot 1.28, switched by `SIGUSR2` and
  `SIGUSR1`.

### Changed

- **Dependencies updated to their latest releases**, among them the hjkl crates
  (0.43: the session and window pickers, layouts, config and XDG paths) and
  interprocess 2.4.7, whose split socket halves tmxr no longer uses (deprecated
  there) in favour of sharing one stream between its reading and writing
  threads.

### Fixed

- **A program's answers come in the order it asked**: answers to OSC 10, 11 and
  12 colour queries and to `CSI ? 996 n` went after every other answer to the
  same output, so a program that asks for DA1 last, to know when the terminal
  has answered, took them as missing.

## [0.11.0] - 2026-10-26

### Breaking

- **The client-server protocol is now 12** (the client sends its terminal's
  color scheme): after upgrading, a running server must be restarted
  (`tmxr kill-server`); the client says so if it meets one.

### Added

- **Blink, hidden and struck-through text**: a program's SGR 5 / 6, 8 and 9 are
  drawn on the client's terminal, as tmux draws them, and `capture-pane -e`
  writes them and the overline (SGR 53, written `5:3` as tmux writes it). tmxr
  dropped all four before. Like tmux, tmxr keeps the overline but does not draw
  it.

- **`#{top_line_time}`**: when the copy-mode view's top line went into the
  history, as tmux keeps a time for each line of history, so the default
  position indicator shows it (`16:49 [25/25]`) as tmux's does. A line keeps its
  time through a rewrap; rows a rewrap makes have none, as tmux's.

- **`copy-mode-position-format`, `copy-mode-position-style`,
  `copy-mode-selection-style` and `copy-mode-mark-style`**, tmux's options: the
  position indicator is drawn from its format (tmux's default adds the last
  search's result count), the selection in its own style, and the `set-mark`
  line in the mark style with the marked cell's colours swapped.

- **Copy mode's search formats**: `#{search_present}` while a search's matches
  are marked, `#{search_count}` over every line, `#{search_match}` (the match at
  the cursor), `#{search_count_partial}` and `#{search_timed_out}`; and
  `#{rectangle_toggle}`, `#{copy_cursor_line}` and `#{copy_cursor_hyperlink}`,
  as tmux's.
- **`#{buffer_name}`, `#{buffer_sample}`, `#{buffer_size}`, `#{buffer_full}` and
  `#{buffer_created}` outside a buffer list**, as the newest buffer's, as
  tmux's.

- **`#{selection_start_x}`, `#{selection_start_y}`, `#{selection_end_x}`,
  `#{selection_end_y}` and `#{selection_active}`**, as tmux's; and
  `#{selection_present}` is 0 while the selection's ends are the same cell, as
  tmux's.

- **Lines rewrap when a pane's width changes**, as tmux's reflow: a wrapped line
  joins and splits again at the new width, its history too, and the cursor keeps
  its place in its line. Under a program on the alternate screen, the normal
  screen is rewrapped as the program leaves it, and copy mode rewraps what it
  shows, its cursor keeping its place. Before, rows were cut or padded where
  they stood.
- **`clear-history -H` clears the pane's hyperlinks**, those on its screen
  included, as tmux's does.
- **The client's theme follows its terminal**: where the terminal reports its
  color scheme (mode 2031, as foot does), `#{client_theme}` is `dark` or `light`
  and the `client-dark-theme` / `client-light-theme` hooks run as it changes, as
  tmux's do. A program's own `CSI ? 996 n` is answered with its pane's scheme:
  light or dark by its background's brightness, else its clients' theme.

### Changed

- **A `#()` job in a format is stopped with SIGTERM**, as tmux stops it, where
  tmxr killed it outright (SIGKILL) and waited for it; a job that traps SIGTERM
  can now tidy up, and one that ignores it no longer holds up the server.

- **Copy mode takes far less memory**: its copy of a pane's lines keeps each
  line as its text and runs of styles instead of 48 bytes a cell, so entering
  copy mode over a full 50000-line history of 80-column lines adds 8–15 MB to
  the server where it added 190 MB (measured over plain and over coloured
  lines).

- **Each pane's terminal emulator is
  [vt1k](https://github.com/kryptic-sh/vt1k)**, kryptic-sh's own, in place of
  vt100: it behaves as vt100 with tmxr's fork did (checked by a differential
  test), and packs the rows of history, so a pane's full 50000-line history
  takes about a seventh of the memory (21 MB against 155 MB for 80-column
  lines). Where vt100 panicked on some input it does not.

- **A config with a bad setting keeps the rest**: an unknown option or a value
  of the wrong kind leaves out only that setting, and the first client to attach
  is told which, as tmux keeps the config lines that work; tmxr used to start
  with the defaults instead. A file that is not TOML still gives the defaults.
  `source-file` of a `.toml` likewise applies its good settings and names the
  bad ones.
- **tmxr is no longer published to crates.io**: it builds against a fork of
  crossterm that a published crate cannot take, so `tmxr-cli` on crates.io stays
  at 0.9.0. Install from a release or package, or with
  `cargo install --locked --git https://github.com/kryptic-sh/tmxr tmxr-cli`.

### Fixed

- **The client keeps up with input that does not stop** (on Linux and macOS): a
  burst longer than one read is read a buffer at a time, its keys handed on as
  they are parsed, where the client's reader parsed all of it first, waiting
  past its timeout and holding the keys back.

- **A program's full reset (`ESC c`) no longer stops its pane rewrapping** on a
  width change; a full reset now also stops focus reports, as tmux's does.
- **A program's `ESC 7` cursor survives its use of the alternate screen**:
  switching with mode 1049 saved the cursor over it, where tmux keeps the two
  apart.

- **A terminal resized while its client attaches gets the session at its new
  size**: a resize between the client's start and its first read of input, as a
  tiling window manager such as Hyprland makes when it places a new window, was
  lost and the session kept the size the terminal had at start.

- **Insert mode and autowrap turned off draw as tmux's**: a program's `CSI 4 h`
  makes its text push the rest of the row right, and `CSI ? 7 l` keeps text at
  the last column instead of wrapping. tmxr showed both in `#{insert_flag}` and
  `#{wrap_flag}` but drew as with insert off and wrap on.

- **A copy-mode cursor past the last column shows a `$` at the row's edge**, as
  tmux's does when a rectangle selection's end of line takes it there.

- **Underline styles and colours are kept and drawn**, as tmux's: double, curly
  (Neovim's undercurl), dotted and dashed underlines (`CSI 4:n m`, `CSI 21 m`)
  and the underline's colour (`CSI 58 … m`, `59`) reach a terminal that draws
  them (`TERM` naming Alacritty, foot, WezTerm, VTE, tmux, kitty or Ghostty),
  and a plain underline any other; `capture-pane -e` writes them as tmux's does.
  Before, an SGR holding any of them was dropped whole, so such text had no
  underline at all, and `CSI 4;58;2;255;0;0 m` lost its underline too. RGB
  colours in their colon form (`38:2::r:g:b`) are read as well. A pane's history
  takes about an eighth more memory (each cell 36 bytes, from 32: 138 to 155 MB
  for 50000 full lines of 80 columns, release build), and copy mode's copy of it
  a fifth more.

- **The server's thread reading a client no longer wakes for each frame it sends
  that client** (Linux, Unix sockets): it waited in `read`, which Linux wakes
  whenever the client takes data off the socket; it now waits in `poll` for
  input. An idle attached session wakes the server a third less often.

- **A copy-mode selection is highlighted as tmux's**: a line selection up to its
  last line's text, not across the pane; under emacs keys without the cell at
  its end; and an upward selection begun at a line's start leaves that line out,
  as tmux's `screen_check_selection` draws them.

- **The current search match is drawn as tmux's**: the match the cursor is
  anywhere in, or under emacs keys after a search down the one just before the
  cursor; tmxr drew it only with the cursor on its first character.

- **A line or word selection moves by lines as tmux's**: after `select-line` or
  `select-word` a move up or down puts the cursor at its line's end, or at its
  start once the selection has turned round at its first line, and `history-top`
  / `-bottom` turn it round as tmux's do. The selection's start and end keep the
  words and lines they snapped to through `other-end` and `stop-selection`, and
  `select-line` with a count selects that many lines.

- **Copy mode's searches place the cursor and view as tmux's**: a match off the
  screen is shown a quarter of the screen up from the bottom (also for bracket
  and paragraph moves), `jump-to-mark` puts the mark's line at the top, and
  under emacs keys a search down starts at the cursor and leaves it just past
  the match. Searching the same text again keeps the marks and count it had when
  it finds nothing, as tmux's.

- **Copy mode's word moves and bracket matching follow tmux's, by `mode-keys`**:
  under emacs keys `next-word-end` and `next-space-end` stop just past the word
  and `previous-word` stops at a line ending in a blank; `select-word` takes a
  one-letter word with what follows it as tmux's does;
  `previous-matching-bracket` under vi keys moves only from a closing bracket,
  and under emacs keys both bracket matches try the cell beside the cursor, then
  move by a word, as tmux's. tmxr searched along the line for any bracket.

- **Copied text follows `mode-keys`, as tmux's**: under emacs keys the character
  under the cursor is left out; under vi keys a selection reaching past its last
  line's text keeps that line's newline, so `copy-line` copies the line with its
  newline. A wrapped line copied to its end keeps its trailing blanks.
  `copy-line` and `copy-end-of-line` clear the selection after, where tmxr kept
  it.

- **A search's highlights are cleared by the commands that clear tmux's**: any
  command but a search, a move under vi keys, `toggle-position` and the
  `-no-clear` copies; before, they stayed until the next search.
- **An incremental search whose text does not start with `=`, `+` or `-`
  searches nothing**, as tmux's; tmxr took its first character as the prefix and
  searched for the rest.

- **Copy mode's `scroll-up` and `scroll-down` move the cursor with the view**,
  keeping its row on the screen and its column as `cursor-up` / `-down` do,
  where the cursor stayed on its line until the view left it behind; and
  `history-bottom` goes to the last line's end, as tmux's.
- **`other-end` keeps where the selection began**: it moves the cursor between
  the selection's ends without swapping them, so `other-end` after
  `stop-selection` goes to the end tmux's does.

- **`top-line`, `middle-line` and `bottom-line` go to the row's start** in copy
  mode, as tmux's do, and `middle-line` to tmux's middle row on an even number
  of rows; `#{copy_cursor_word}` follows a word across a wrapped line, as
  tmux's.

## [0.10.0] - 2026-10-23

### Breaking

- **A client whose session is destroyed detaches**, as tmux's does by default:
  tmxr always moved it to the session used last. The new `detach-on-destroy`
  session option takes tmux's values (`on`, `off`, `no-detached`, `previous`,
  `next`); `set -g detach-on-destroy off` keeps the old behaviour, and the
  client flag `no-detach-on-destroy` (`attach -f`, `refresh-client -f`) keeps
  one client attached where `on` would detach it.
- **The Unix socket directory is `/tmp/tmxr-<uid>`** (or under `$TMXR_TMPDIR` or
  `$TMPDIR`), no longer under `$XDG_RUNTIME_DIR`: logind deletes that directory
  when the user's last session ends, so a server left running across a logout
  could no longer be reached. A running server renews its socket's status time
  each hour, as tmux does, so a cleaner of old files in `/tmp` (systemd-tmpfiles
  on Arch, after 10 days) leaves it. A server started by an earlier tmxr is
  reached with `TMXR_TMPDIR=$XDG_RUNTIME_DIR tmxr kill-server`.
- **`customize-mode` refuses `-a` and `-F`**, which it accepted and ignored:
  tmux's has no `-a`, and its `-F` formats tree lines tmxr's picker does not
  have.
- **The client-server protocol is now 11** (the client sends its process id, for
  `#{client_pid}`, its terminal's device, for `#{client_tty}`, its terminal's
  colours, and the input events it reads together as one message; the server
  tells it whether to ask for focus reports, `focus-events`, and what to run in
  its place, `detach-client -E`): after upgrading, a running server must be
  restarted (`tmxr kill-server`); the client says so if it meets one.
- **`break-pane -t` is the window the pane becomes**, as tmux's, not the pane to
  break: that is `-s` now (the current pane without it). `-t` takes an index
  (`-t :5`), `-a` / `-b` place it next to the target window, and `-n` names it
  (automatic-rename off). A window's only pane no longer refuses: the window
  moves, as tmux's.

### Added

- **Pane, session and user ranges in the status line**: `status-format`'s
  `#[range=pane|%N]`, `#[range=session|$N]` and `#[range=user|text]` take clicks
  as tmux's do, and `#{mouse_status_range}` names the range clicked (a user
  range's text), for status-line buttons of a config's own.
- **A program's palette is its pane's**: the colours of the 256 a program sets
  with OSC 4 (base16-shell and the like) are drawn as it set them in its pane,
  and its OSC 4 queries for them answered, as tmux does; OSC 104 unsets them.
- **A program's default colours are its pane's**: the foreground and background
  a program sets with OSC 10 / 11 (reset by OSC 110 / 111) are the pane's, drawn
  and shown by `#{pane_fg}` / `#{pane_bg}`, as tmux's, under `window-style` in
  an inactive pane and over it in the active one.
- **Colour queries are answered**: a program asking for the default foreground,
  background or cursor colour (`OSC 10`, `11`, `12` with `?`, as editors do to
  pick a light or dark theme) gets the colour `window-style` /
  `window-active-style` or the cursor's colour gives, else the client terminal's
  own (the client asks its terminal at attach, as tmux does), as tmux answers,
  ended as its query was.
- **A program's cursor colour reaches the terminal**: the cursor colour the
  active pane's program sets with OSC 12 (read as tmux reads X11 colour
  specifications, and reset by OSC 112) is given to the client's terminal, as
  tmux does, and reset when the client leaves; the new `cursor-colour` pane
  option gives one where the program sets none. `#{cursor_colour}` shows it, and
  `#{cursor_very_visible}` the very visible cursor of `CSI 34 l`.
- **`#{alternate_saved_x}` and `#{alternate_saved_y}`**: where the cursor was
  when the pane's program last switched to the alternate screen with
  `CSI ? 1049 h`, as tmux's (4294967295 until it has).
- **`capture-pane -a`** captures the normal screen a program on the alternate
  screen hides (an editor, a pager), as tmux's; without an alternate screen it
  fails with `no alternate screen`, or with `-q` captures nothing.
- **Hyperlinks (OSC 8)**: a link a program prints (`ls --hyperlink`, a
  compiler's diagnostics) reaches the terminal through tmxr, as through tmux,
  and `#{mouse_hyperlink}` gives the one under the mouse, so the pane menu's
  "Type" / "Copy" link items show; copy mode keeps them too. The `vt100` tmxr
  builds against keeps them (doy/vt100-rust#51).
- **Copy mode's `next-prompt` / `previous-prompt`**, as tmux's: with shell
  integration (OSC 133), move to the start of the next or previous prompt, or
  with `-o` of a command's output, throughout the history.
- **`source-file` runs tmux command files**: a file not named `.toml` (a
  `.tmux.conf`) is read as tmux's: each line's commands, `{ … }` blocks over
  several lines, `\` line continuations, `NAME=VALUE`, `%hidden` and `%if` /
  `%elif` / `%else` / `%endif`, the whole file read before any of it runs. A
  command that does not parse stops the file with `path:line: error`, as tmux's.
- **`source-file -v`** shows what a file set, as the commands that set it
  (`set-option -g`, `bind-key` / `unbind-key`, `set-hook -g`), each after the
  file's path, as tmux's `-v` shows a file's commands.
- **`source-file` takes several paths and globs**, as tmux's:
  `source-file ~/.config/tmxr/conf.d/*.toml extra.toml` applies each file in
  turn, a later one winning; a path matching nothing is an error unless `-q`.
- **X11's colour names**, as tmux's: `#{c:orchid}`, and styles such as
  `fg=NavyBlue` or `bg=grey50`, take the 578 names tmux 3.6 knows, ignoring
  case. Styles also take `grey` / `gray` with a percentage, which only `c:` took
  before.
- **The `w:` format modifier**, as tmux's: the width the expanded text takes on
  screen (`#{w:#{pane_title}}`), where `n:` gives its length in bytes. Styles
  take no width, `##` counts as one `#`, and a wide character as two.
- **`#{session_bell_flag}`, `#{session_activity_flag}` and
  `#{session_silence_flag}`**: 1 when any of the session's windows has that
  alert for it. tmux 3.6 has them but looks at one window only (the first for a
  bell, the format's own window for the others), so it can say 0 where a later
  window is alerted; tmxr looks at every window.
- **Synchronized updates** (Linux and macOS): a client asks its terminal at
  attach whether it takes DEC mode 2026 (DECRQM) and, where it does, writes each
  frame as one synchronized update, so the terminal never shows a frame half
  drawn. tmux sends them only where terminfo or `terminal-features` says so.
- **SIGUSR1 binds the server's socket again** (Unix), as tmux's does: a server
  whose socket file was removed is reached again after `kill -USR1 <pid>`, its
  sessions kept.
- **`#{cursor_shape}` and `#{cursor_blinking}`**, as tmux's: the cursor the
  pane's program set (`block`, `underline`, `bar` or `default`) and whether it
  blinks.
- **`#{pane_tabs}`**, as tmux's: the columns of the pane's tab stops.
- **`#{scroll_region_upper}` and `#{scroll_region_lower}`**, as tmux's: the
  pane's scroll region.
- **`#{insert_flag}`, `#{wrap_flag}` and `#{origin_flag}`**, as tmux's: insert
  mode, autowrap and origin mode as the pane's program set them.
- **`#{window_cell_width}` and `#{window_cell_height}`**, tmux's 16 and 32: the
  pixel size tmux gives a cell when the terminal reports none.
- **`#{command}`**, as tmux's: the name of the command whose formats are
  expanding (`display-message`, a hook's `set-option`).
- **`#{pane_fg}`, `#{pane_bg}` and `#{sixel_support}`**: a pane's default
  colours from `window-style` and, for the active pane, `window-active-style`,
  named as tmux's; `sixel_support` is 0, as tmxr draws no sixel images.
- **`#{pane_unseen_changes}`**, as tmux's: 1 once the pane prints while in copy
  mode, through `refresh-from-pane`, until copy mode ends.
- **The `editor` server option and `choose-buffer`'s `e`**, as tmux's: the
  highlighted buffer opens in `editor` (`$VISUAL`, else `$EDITOR`, else
  `/usr/bin/vi`; Notepad on Windows) in a popup, and what the editor saves is
  the buffer once it exits successfully; the picker is back after, its rows
  showing the edit.
- **`escape-time`** takes effect, as tmux's: set above 0, an Escape with a key
  pressed within it is Meta and that key, and a lone Escape goes once it is up.
  tmxr's default stays 0.
- **The command prompt's Tab opens a menu of the names a word could be**, as
  tmux's, when they share no longer start: under the word, picked with Tab, the
  arrows, a digit or Enter, Escape keeping the word; after `-t` / `-s` and in a
  `target` prompt, a menu of the sessions; after `-tname:` and in a
  `window-target` prompt, the session's windows, one completed at once. A menu
  shows at most ten names, or what fits.
- **strftime's week numbers and the C locale's date and time** in formats: `%U`,
  `%W`, ISO `%V` / `%G` / `%g`, `%x`, `%X` and the zone's offset `%z`, as tmux's
  (glibc's), where they were kept as written.
- **A pane keeps its last copy-mode search**, as tmux's: `#{pane_search_string}`
  shows it after copy mode ends, and in the next copy mode `search-again` (`n`)
  repeats it upwards.
- **`#{tree_mode_format}`, `#{buffer_mode_format}` and
  `#{client_mode_format}`**, tmux 3.6's default `choose-*` formats, so
  `#{E:tree_mode_format}` expands as tmux's.
- **`#{pane_format}`, `#{window_format}` and `#{session_format}`**, as tmux's: 1
  for the kind of thing a format is expanded for, so a `#{W:…}` row or a
  `list-windows` line is a window though it carries the window's active pane.
- **`#{pane_tty}`**, as tmux's: the pane's terminal device on Unix, which
  scripts hand to `ps -t` (vim-tmux-navigator's `is_vim` check among them);
  empty on Windows, where a ConPTY has none.
- **`#{client_tty}`**, as tmux's: the client's terminal device on Unix
  (`/dev/pts/3`), empty on Windows, whose console has none.
- **The `strikethrough`, `blink` and `hidden` style attributes**, as tmux's, and
  its underline styles drawn as a plain underline.
- **Keys for the `choose-*` rows and `-K`**, as tmux's: each row of
  `choose-tree`, `choose-client`, `choose-buffer` and `find-window` shows a key
  (0 to 9, then M-a to M-z, or what `-K` gives it), and that key chooses the row
  while the query is not being typed.
- **`#{buffer_full}` and `#{buffer_created}`**, as tmux's.
- **`#(command)` in formats**, as tmux's: the command runs in the background
  with the session's shell and directory (again at most once a second while a
  format names it, at once when its expanded text changes, the run before
  stopped), and the format shows the last line it printed: in the status line,
  the line once it has printed one. A job no format has started for an hour is
  dropped, and the server's exit stops every run.
- **`window-status-bell-style` and `window-status-activity-style`**, as tmux's:
  a window's entry with a bell alert takes the bell style, else with an activity
  or silence alert the activity style, unless `default` (tmxr's defaults, where
  tmux's are `reverse`).
- **`message-line` and styled messages**, as tmux's: a message or the prompt
  takes the status line `message-line` names (0 to 4, the last there is when
  fewer), and `display-message`'s text is laid out by its `#[…]` styles rather
  than showing them; other messages stay plain text.
- **Formats read option values**, as tmux's: `#{status-justify}`,
  `#{E:pane-status-style}` and any other option name give the option's value for
  the pane, window, session or server. `pane-status-style`,
  `pane-status-current-style`, `session-status-style` and
  `session-status-current-style` are added, which tmux's second and third status
  lines use.
- **`status 2` to `5` and `status-format`**, as tmux's: `status` takes `off`,
  `on` or 2 to 5 lines, and each line after the first is drawn from its
  `status-format` item (tmux's defaults: the panes, then the sessions), laid out
  by `align`, `fill` and the `default` styles; a `status-format[0]` set apart
  from tmux's default is drawn the same way, a click reaching its left, right
  and window ranges. The config file's `[status] status` takes those names
  (still `true` / `false`) and gains `format`.
- **Tab completion in the command prompt**, as tmux's: the word at the cursor
  completes to a command name or alias (and after the first word an option name
  or layout) with a space after when one matches, or as far as the matches
  agree; `-t` / `-s` words complete session names as `-tname:`, and a
  `command-prompt -T target` prompt completes any word as a session.
- **Empty panes, `split-window -I` and `display-message -I`**, as tmux's:
  `split-window ''` or `split-window -I` makes a pane with no program (no pid,
  `#{pane_dead}` 1, the cursor hidden, typed keys going nowhere), and `-I` shows
  what was piped into the command in it, each newline starting a line;
  `display-message -I` shows more there and refuses a pane running a program.
- **`display-message -a`**, as tmux's: every format variable with a value, as
  `name=value` lines sorted by name.
- **`capture-pane -T`**, and `-N` and `-J` as tmux's: each line runs to what
  tmux allocates for it (a quarter, half or all of the width) or, with `-T` or
  `-J`, to its last written cell, where `-N` padded every line to the pane's
  width and `-J` cut written trailing spaces; a never-written line is empty.
- **`choose-tree`, `choose-client` and `choose-buffer` `-F`, `-f`, `-O` and
  `-r`**, as tmux's: `-F` sets each row's text, `-f` filters them (`choose-tree`
  keeps the sessions and windows with a pane that passes), `-O` sorts them by
  the orders each takes in tmux (`index`, `name`, `time`; `name`, `size`,
  `creation`, `activity`; `time`, `name`, `size`), and `-r` turns the order
  round; a sorted tree opens on the current item.
- **Session groups**, as tmux's: `new-session -t target` makes a session that
  shares the target's windows (or joins the group of that name), named `group-N`
  unless `-s` names it; a window made, killed or moved in one member reaches the
  others, each keeping its own current window, and `swap-window`, `move-window`
  and `link-window` refuse to move one between members. `#{session_group}`,
  `#{session_grouped}`, `#{session_group_size}`, `#{session_group_list}`,
  `#{session_group_attached}`, `#{session_group_attached_list}` and
  `#{session_group_many_attached}`, and `list-sessions` shows `(group name)`.
- **The `c:` format modifier**, as tmux's: `#{c:colour110}` gives the colour's
  RGB as six hex digits (`87afd7`), for `#rrggbb`, `colourN`, the eight colour
  names and their `bright` forms, and `greyN`; X11's colour names are not read
  yet.
- **`capture-pane -e`**, as tmux's: each change of colour or attribute in the
  capture is written as the SGR escape tmux writes for it (with `-C`, as
  `\033`).
- **`choose-tree`, `choose-client` and `choose-buffer` `-N` and `-y`**, as
  tmux's: `-N` opens the picker without its preview, `-y` kills a session with
  `C-x` without asking first.
- **`choose-tree`, `choose-client` and `choose-buffer` templates**, as tmux's: a
  command given after the flags runs for the chosen item, its `%%` the item's
  target (`=session:`, `=session:index.`, the client, or the buffer's name).
- **Copy mode's word and line selection, and `selection-mode`**, as tmux's:
  after `select-word` or `select-line` (or `selection-mode word` / `line`),
  moving the cursor grows the selection by whole words or lines, keeping the
  first one on whichever side the cursor is not.
- **The `pane-border-status` and `pane-border-format` options**, as tmux's: each
  pane gets a status line on its top or bottom border, two cells in, in the
  border style (the active pane's in `pane-active-border-style`), and a pane on
  the window's top or bottom edge gives up a row for it.
- **A client away or out of focus stops sizing windows** (tmxr's, past tmux):
  with several clients of different sizes, a window takes the smallest of those
  in use, and a client whose terminal reports focus out, or that has had no
  input for `client-away-time` seconds (a new server option, 300 by default, 0
  never), gives it up until it is used or focused again. With none in use, all
  count.
- **The `fill-character` window option and a smaller window's edge**, as tmux's:
  on a larger client, a window gets a border along its right and bottom edges
  and the rest is padded with `fill-character` (a dot when empty).
- **The `status-keys` and `message-command-style` options, and tmux's prompt
  keys**: the command prompt takes tmux's emacs keys (`C-b` / `C-f`, `C-d`,
  `C-p` / `C-n`, word moves on `M-b` / `M-f` and `C-Left` / `C-Right` by
  `word-separators`, `C-w` cutting a word that `C-y` puts back before the newest
  buffer, `C-t`, `C-v`), or with `status-keys vi` vi's insert and command modes,
  the command mode drawn in `message-command-style`.
- **The `pane-border-indicators` option**, as tmux's: `arrows` and `both` draw
  arrows on the active pane's borders, pointing at it.
- **The `pane-border-lines` option**, as tmux's: pane borders in `single`,
  `double` or `heavy` lines, `simple` ASCII, `spaces`, or `number`, each border
  cell its pane's index.
- **`copy-mode -s src-pane`**, as tmux's: the target pane enters copy mode
  showing another pane's screen and history, which `refresh-from-pane` reads
  again.
- **`send-keys -R`**, as tmux's: the pane's terminal is reset before any keys
  go: a sequence half received is dropped, the modes and scroll region go back
  to the defaults, and the screen is cleared with the cursor at the top; the
  history stays.
- **The `aggressive-resize` option**, as tmux's: a window is sized only by the
  clients showing it, not every client whose session holds it.
- **The `alternate-screen` option**, as tmux's: off, a program's switches to the
  alternate screen (modes 47, 1047 and 1049) are ignored, so what it draws stays
  on the main screen and in the history.
- **The `window-style` and `window-active-style` options**, as tmux's: the
  colours a pane's cells take where its program sets none, the window's active
  pane taking `window-active-style`'s where it sets one; in copy mode too.
- **`list-keys -a` and `-P`**, as tmux's: with `-N`, `-a` lists unnoted binds
  too (their command for a note) and `-P` gives the text keys follow.
- **`detach-client -E shell-command`**, as tmux's: each client it detaches runs
  the command, with its session's `default-shell`, in its place and prints no
  `[detached]` line. On Unix the client becomes the command; Windows cannot
  replace a running process, so there the client runs it and exits with its
  status.
- **`set-buffer -t` and `load-buffer -t`**, as tmux's: the client whose
  clipboard `-w` sets; one that does not exist is no error.
- **The `history-file` server option**, as tmux's: the prompts' history is read
  from it when the server starts and written to it when the server exits, one
  `type:entry` line each. It takes an absolute path or one starting `~/`; empty,
  the default, keeps no file.
- **The `assume-paste-time` session option**, as tmux's (1 millisecond by
  default): once a key comes that soon after the one before, each further fast
  key is pasted to the pane as it is, past the binds and the prefix, so text
  pasted into a terminal without bracketed paste does not run binds. A pane in a
  mode drops them; 0 turns it off.
- **The `focus-events` server option**, as tmux's, on by default as
  tmux-sensible sets it: off, the client's terminal is not asked for focus
  reports, and a pane made active or a window made current is not told of focus
  (`CSI I` / `CSI O`, `pane-focus-in` / `-out`); attaching, a popup or menu
  opening or closing, and a focus report still are.
- **The `remain-on-exit-format` option and `#{pane_dead_signal}`**, as tmux's: a
  pane `remain-on-exit` keeps gets the format's line written into its screen
  after a linefeed at the bottom (tmux's default reads
  `Pane is dead (status 3, <time>)`), and a pane a signal ended shows its number
  in `#{pane_dead_signal}`. The line is written once the program's output is
  done, and the pane reads nothing after it; an empty format writes nothing.
- **The `status-fg` and `status-bg` options**, as tmux's: a colour other than
  `default` replaces that side of `status-style` (`fg` / `bg` under `[status]`
  in the config).
- **The `status-position` option**, as tmux's: `top` puts the status line, and
  messages and prompts, on the client's first row and the windows under it;
  mouse clicks, `display-panes` labels, pickers and the `S` / `W` places of
  `display-menu` and `display-popup` follow it.
- **The `default-size` option**, as tmux's: the size a detached `new-session`
  makes its windows (`80x24` by default), `-x` or `-y` taking either side.
- **The `status` option**, as tmux's: `set -g status off` drops the status line
  and gives its row to the windows (`status = false` under `[status]` in the
  config); a message or prompt still shows, over the window's last row. tmux's
  `2` to `5` rows are not taken.
- **`command-prompt` asks several prompts**, as tmux's: `-p` and `-I` take
  comma-separated lists (`-l` keeps one prompt, commas and all), each answer
  fills `%1` … `%9` and the next `%%`, and with no `-p` the template, in
  brackets, is the prompt. `-1` takes one character as the answer, `-N` only
  digits (any other key answers), `-F` expands the template as a format first,
  and `-b` is accepted, as tmxr never waits for a prompt.
- **`customize-mode -t`**, as tmux's: the target's session, window and pane
  options are listed first, each edited back at its own scope.
- **The `client-active` hook**, as tmux's: it runs when a client becomes its
  window's latest by typing or resizing there after another client did, and when
  the latest client leaves and the window passes to another; attaching or
  switching makes a client the latest without it.
- **The `pane-set-clipboard` hook**, as tmux's: it runs when a program's copy is
  taken, and when copy mode copies to the clipboard.
- **`capture-pane -C` and `-q`**, as tmux's: `-C` doubles the backslashes in the
  text; `-q` is accepted (in tmux it only quiets `-a`).
- **`display-message -N` and `-C`**, as tmux's: with `-N` a key leaves the
  message up until its time is out; `-C` is accepted, as tmxr never freezes the
  panes behind a message.
- **`source-file -n` and `-F`**, as tmux's: `-n` only checks that the file (else
  the config's own) loads, changing nothing, and `-F` expands the path as a
  format.
- **`send-keys -K` and `-c`**, as tmux's: the keys go to a client (`-c`, else
  the best one) as if typed there, so its prefix and binds act on them.
- **`display-panes -d`, `-N`, `-b` and a template**, as tmux's: `-d` sets how
  long the numbers show (0 until a key), `-N` lets keys go on to the pane while
  they show, the template is the command list a chosen pane runs (`%%` its id;
  `select-pane -t "%%%"` by default), and `a` to `z` choose panes 10 and on.
  `-b` is accepted: tmxr never waits for the overlay.
- **Copy mode's `stop-selection`**, as tmux's: the selection stays where it is
  while the cursor moves on, and `other-end` takes the cursor back to its end,
  which follows the cursor again.
- **Copy mode's `pipe`, `pipe-no-clear`, `pipe-and-cancel`,
  `copy-selection-no-clear` and `copy-pipe-no-clear`**, as tmux's: the `pipe`
  commands hand the selection to a command (else `copy-command`) with no buffer
  or clipboard, and the `-no-clear` ones keep the selection.
- **The `command-error` hook**, as tmux's: it runs once for each command that
  fails as it runs (not one that does not parse), against the command's target.
- **`refresh-client -f` / `-F`** sets a client's flags, as tmux's, and `!flag`
  turns one off (`-f '!ignore-size'`) there and in `attach-session -f` /
  `new-session -f`; turning `ignore-size` on or off resizes the windows at once.
- **`update-environment` is a session option, an array as tmux's**:
  `set -g update-environment 'A B'` replaces it (words split at spaces and
  commas), `-a` adds words at the first free indexes, `update-environment[3]`
  sets one item (`-a` appends to it), `-u update-environment[3]` removes one,
  and `show` lists `update-environment[i] value` lines (an empty array as its
  bare name). A session may have its own, which attaching to it reads; a new
  session reads the global one. It was a config-file value only.
- **The terminal's title** (`set-titles`, `set-titles-string`, as tmux's): on by
  default, the client's terminal shows the title the active pane's program sets,
  or `session:window` when it sets none. Control characters in a title are
  dropped before it reaches the terminal.
- **tmux's notification hooks**: `session-renamed`, `session-window-changed`,
  `window-linked`, `window-unlinked`, `window-renamed` (by hand or automatic),
  `window-pane-changed`, `window-layout-changed`, `window-resized`,
  `pane-mode-changed`, `pane-title-changed`, `client-session-changed`,
  `client-resized` and `pane-died`, fired for the same actions as tmux 3.6's and
  kept with the same options (the window and pane ones with `-wg`).
- **A config file the server could not use is shown to the first client**:
  starting with the defaults because of an error in the file was told only in
  `show-messages`; the first client to attach now sees it as a message too, as
  tmux shows its config errors.
- **More of tmux's format modifiers**: `b:` and `d:` (a path's last part and the
  rest), `s/pattern/replacement/flags:` (regular expression, `\1` groups, `i`),
  `l:` (literal), `n:` (length), `pN:` / `p-N:` (padding), chained with `;` and
  applied in tmux's order (`#{=3;b:path}` cuts the basename), and
  `#{?c1,v1,c2,v2,else}` with any number of conditions.
- **Window list styles, as tmux's**: `window-status-style`,
  `window-status-current-style` (the current window's, unless `default`),
  `window-status-last-style` (added for the last window) and
  `window-status-separator` (between entries), with `status-left-style` and
  `status-right-style` under the two sides; `[status]` takes them as
  `window-style`, `window-current-style`, `window-last-style`,
  `window-separator`, `left-style` and `right-style`. The separator defaults to
  empty, where tmux's is a space, so the list looks as before. Also
  `#{window_last_flag}`.
- **`set-buffer -n`, `unbind-key -q` and `kill-session -C`**, as tmux's: `-n`
  renames a buffer (the newest without `-b`; one already of that name goes),
  `-q` makes a key that is not bound or not known no error, and `-C` clears a
  session's alert flags instead of killing it. `set-buffer` with no data says
  so, and with empty data sets nothing.
- **`display-message -d`, `-c`, `-l` and `-F`**, as tmux's: how long the message
  shows (0 until a key), which client shows it, the text as given without
  expanding it, and the format given as a flag.
- **`exit-empty`, `exit-unattached` and `destroy-unattached`**, as tmux's: the
  server can stay with no sessions (`exit-empty off`) or go once no client is
  attached (`exit-unattached on`), and a session can go once no client is
  attached to it (`destroy-unattached on`, checked when a client attaches,
  detaches or switches, as tmux checks; `keep-last` and `keep-group` act as
  tmux's do for a session in no group).
- **`default-command`**, as tmux's: what a new window or pane runs, by the
  shell, when given no command (empty, the shell itself). A respawned pane keeps
  its own command.
- **`automatic-rename-format`, `monitor-bell` and `remain-on-exit failed`**, as
  tmux's: the format an automatic rename names a window with (tmux's default,
  the active pane's command, or `[tmux]` in a mode), whether a bell flags a
  window, and keeping a pane only when its program failed. `remain-on-exit` is
  `off` / `on` / `failed` now; the config file still takes `true` / `false`.
- **`word-separators` and `wrap-search`**, as tmux's: the characters that end a
  word in copy mode (its word motions, the word under the cursor or mouse), and
  whether a copy-mode search goes round from the other end. Word motions now
  class characters as tmux does, so a non-ASCII symbol like `→` is part of a
  word.
- **`prefix2` and `key-table`**, as tmux's: a second prefix key (`None` by
  default), and the key table a client's keys start in (`root` by default). A
  key not bound in the table a client is in gets a second try in that default
  table, and is not passed to the pane, as tmux does.
- **`clock-mode-style`**, as tmux's: `12`, `24` (the default), `12-with-seconds`
  or `24-with-seconds`, the 12-hour clock with `AM` / `PM` in tmux's big
  letters.
- **`window-size largest` and `smallest`**, as tmux's: a window as wide and as
  tall as the session's widest and tallest clients, or its narrowest and
  shortest, each way.
- **`-P` and `-F` on `new-window`, `split-window`, `new-session` and
  `break-pane`**, as tmux's: print the new window or pane
  (`#{session_name}:#{window_index}.#{pane_index}`) or session
  (`#{session_name}:`), or `-F`'s format for it.
- **Cursor shapes**: a program's `CSI Ps SP q` (vim's bar cursor in insert mode,
  say) reaches the client's terminal for the active pane instead of being
  dropped, and **`cursor-style`**, as tmux's, gives the shape for a pane whose
  program has not asked for one.
- **`prefix-timeout`**, as tmux's: a server option in milliseconds; a key
  pressed longer than that after the prefix (or `switch-client -T prefix`) goes
  to the root table instead (unless it repeats a repeatable binding). 0, the
  default, waits forever.
- **Format variables** `active_window_index`, `last_window_index`,
  `window_start_flag`, `window_end_flag`, `window_active_sessions` and
  `window_active_sessions_list`, `window_linked_sessions` and
  `window_linked_sessions_list`, `window_marked_flag`, `session_marked`,
  `session_many_attached`, `server_sessions`, `history_limit`, `cursor_flag`,
  `keypad_flag`, `keypad_cursor_flag`, `pane_start_command` (the command as
  given, quoted as tmux quotes it), `pane_start_path`, `client_last_session`,
  `client_pid`, `session_attached_list`, `session_path`, `window_active_clients`
  and `window_active_clients_list`, `pane_last`, `session_alert` and
  `session_alerts`, the mouse modes `mouse_standard_flag`, `mouse_button_flag`,
  `mouse_all_flag` and `mouse_sgr_flag`, `cursor_character`, the users `uid`,
  `user`, `client_uid` and `client_user` (on Windows the ids are SIDs),
  `next_session_id`, `config_files` (the TOML file the config was last read
  from, empty for the defaults alone), and the times `start_time`,
  `session_created`, `session_activity`, `session_last_attached` (empty until a
  client attaches), `window_activity`, `client_created`, `client_activity` and
  `pane_dead_time` (when a pane `remain-on-exit` kept died) (seconds since the
  epoch), as tmux's.
- **Layout strings**, as tmux's: `#{window_layout}` and
  `#{window_visible_layout}` write the window's layout as tmux does (checksum
  and all), and `select-layout` takes such a string back, laying its cells over
  the window's panes in order and sizing the window to it.
- **Copy-mode commands `copy-line`, `copy-pipe-line` (and their `-and-cancel`),
  `scroll-top`, `scroll-bottom`, `rectangle-on`, `rectangle-off`, and
  `scroll-down-and-cancel`, `page-down-and-cancel`, `halfpage-down-and-cancel`
  and `cursor-down-and-cancel`**, as tmux's: whole lines copied (a count for
  more), the cursor's line taken to the top or bottom of the view when there is
  room, and copy mode left once a motion down reaches the bottom.
- **Times in formats**, as tmux's: strftime sequences (`%H:%M`, `%d-%b-%y`) in
  `status-left`, `status-right`, the window entries, `set-titles-string` and
  `display-message` show the time now, `T:` turns that on for a value, and the
  `t:` modifier shows a time variable as a date (`#{t:session_created}`), as
  short as its age allows (`t/p`) or by a strftime format (`t/f/%H:%M`).
- **`status-justify`**, as tmux's: the window list after `status-left` (`left`,
  the default), centred between the two sides (`centre`), against `status-right`
  (`right`), or centred on the whole line (`absolute-centre`).
- **Format modifiers `e|op|` and `a:`**, as tmux's: arithmetic and numeric
  comparison (`#{e|+|:#{window_index},1}`, `#{e|/|f|4:7,2}` for decimals) and
  the character for a code (`#{a:65}` is `A`).
- **The `N:`, `C:`, `R:`, `!:` and `!!:` format modifiers**, as tmux's:
  `#{N:name}` (or `N/w`) is 1 when a window of the session is called `name`,
  `#{N/s:name}` when a session is; `#{C:text}` is the number of the first line
  on the pane's screen holding `text` (a glob, `C/r` a regular expression, `i`
  ignoring case), 0 for none; `#{R:text,n}` repeats `text` n times; `#{!:#{x}}`
  / `#{!!:#{x}}` are 1 when x is false / true.
- **Format loops**, as tmux's: `#{S:fmt}`, `#{W:fmt}`, `#{P:fmt}` and `#{L:fmt}`
  expand `fmt` for each session, window of the session, pane of the window or
  client; `S`, `W` and `P` take a second format after a comma for the current
  one (`#{W:#I ,*#I }`), `/n` sorts by name, `/t` by activity and `/r` reverses,
  and `#{loop_last_flag}` is 1 for the last item.
- **`split-window -p`**, tmux's older percentage (`-p 30` is `-l 30%`), and
  **`join-pane -f` / `move-pane -f`**, the pane spanning the whole window as
  `split-window -f` does.
- **`confirm-before -c`, `-y`, `-b` and `-t`**, as tmux's: `-c` picks the key
  that confirms (the prompt shows it), `-y` makes Enter confirm, `-t` names the
  client to ask, and `-b` is accepted (tmxr's prompt never holds up the commands
  after it). Only the confirm key itself confirms now, not its capital (`Y`), as
  in tmux.
- **`~` and `{marked}` targets**, as tmux's: the marked pane (`select-pane -m`),
  its window and its session, wherever a target is taken.
- **`new-window -k` and `-S`**, as tmux's: `-k` replaces a window already at the
  target index, and `-S` with `-n` (and no index) selects a window of that name
  if there is one instead of making another.
- **`-Z` on `last-pane`, `rotate-window`, `swap-pane` and `split-window`**, as
  tmux's: a zoomed window stays zoomed, on the pane that is active after
  (`split-window -Z` zooms the new pane whether the window was zoomed or not).
- **`select-pane -d` / `-e` and `last-pane -d` / `-e`**, as tmux's: a pane's
  input off (no keys, pastes or mouse reach its program, nor its synchronized
  siblings through it) and back on, shown by `#{pane_input_off}`.
- **`send-keys -H`**, each key a byte in hex (`send-keys -H 1b 5b 41`), and
  **`-N` for keys**, sending them that many times; both as tmux's (`-N` worked
  only with `-X` before).
- **`capture-pane -S` / `-E`, `-N` and `-b`**, as tmux's: the lines to capture,
  0 being the screen's first row, negative ones in the history and `-` either
  end (`capture-pane -p -S -` prints the whole history), formats expanded
  (`-S -#{history_size}`); `-N` keeps trailing blanks and `-b` names the buffer.
- **`switch-client -t session:window.pane` (and `attach-session -t`), `-Z` and
  `-r`**, as tmux's: the window and pane named become the session's current ones
  (`-Z` keeping a zoomed window zoomed), and `-r` toggles the client read-only.
- **`switch-client -c target-client`**, and from a command client (a script,
  `run-shell`) without `-c` it acts on the attached client used most recently,
  as tmux's, where it failed with "no current client".
- **`allow-rename`**, as tmux's: on, a program names its window with
  `ESC k name ESC \` (and stops automatic renaming; an empty name gives it
  back). Off by default; either way the string is no longer printed. On Windows
  this needs the bundled ConPTY, as passthrough does.
- **`allow-set-title`**, as tmux's: off, a program's OSC 0 / 2 no longer titles
  its pane (`select-pane -T` still does); on by default.
- **`#{client_flags}` and `#{client_readonly}`**: the client's `attached`,
  `focused`, `ignore-size` and `read-only` flags, in tmux's order, and whether
  it is read-only. A client is focused from attaching until its terminal reports
  focus out, as tmux's.
- **Focus reports reach panes**: a program that asks (mode 1004) gets `CSI I`
  when its pane becomes the focused one (the active pane of a window a focused
  client shows) and `CSI O` when it stops, as tmux sends them, so editors see
  focus come and go. `pane-focus-in`, `pane-focus-out`, `client-focus-in` and
  `client-focus-out` hooks fire.

### Changed

- **A click on the status line runs `switch-client -t =`**, tmux's default
  `MouseDown1Status`, instead of `select-window -t =`: a window still selects
  it, and a session range now switches to its session.
- **Windows: one process snapshot a refresh for every pane**, where naming each
  pane's foreground command took two snapshots of every process a pane (about 5
  ms each), on the server's thread.
- **`source-file` applies a file over what is running, as tmux's**: what the
  file sets (options, user options, binds, hooks) takes its value, and
  everything else keeps its own, so a `set -g`, `bind` or `set-hook` made at run
  time survives `prefix R`. It used to replace the whole config, dropping them.
  A file setting an option to its default value still sets it. Sourcing a file
  no longer makes it the config `prefix R` reloads: that stays the file the
  server started with, as in tmux.
- **Pane output no longer redraws the status line**, as in tmux: the status line
  is expanded on its `status-interval` timer and on the events that change it (a
  window renamed, flagged, added or selected, a key table, a message, a title),
  and a frame drawn only for a pane's output shows it as last drawn. A format
  that changes with nothing else happening (a clock in a session with
  `status-interval 0`) shows the change at the next of those.
- **A pane given no command runs its shell as a login shell** (Unix), as tmux's
  does: `-` and the shell's name as `argv[0]`, and `SHELL` set to it, so the
  shell reads `~/.profile`, `~/.bash_profile` or `~/.zprofile` as it would in
  tmux. A pane given a command, and a popup, run it as before. On macOS the
  login profile's `path_helper` puts the system's directories first in `PATH`,
  as it does in tmux; set `default-command` to the shell to start panes as
  before.
- **A `{ … }` block is kept as tmux keeps it**: its commands printed back by
  their full names and requoted, a nested block in braces, `;;` between its
  lines (`set @z { display -p A }` shows `display-message -p A`, as tmux's). A
  block that does not parse is kept as written.
- **`show-hooks` and `show-options -H` list hooks as tmux's**: in tmux's order,
  a global table every hook (the unset ones as a bare name), each command
  printed re-parsed (`display-message hi` for `display hi`).
- **Format syntax errors end the expansion, as tmux's**: a comparison, `m:` or
  `R:` without its comma (`A#{==:a}B`) now stops the format there (`A`), where
  it compared against an empty side; a comparison's right side is all the text
  after its first comma, and `&&:` / `||:` take any number of parts.
- **`command-prompt -i` runs its template as tmux's**: once as it opens, then
  after every edit, the text prefixed `=`, or `-` / `+` after `C-r` / `C-s`
  (which on an empty input bring back the `-I` text, the prompt opening empty);
  Enter runs nothing more. `search-backward-incremental` and
  `search-forward-incremental` read the prefix for their direction and search
  again from where they started when the text changes. A copy command that takes
  no flags is skipped when its first argument looks like one, as tmux's.
- **`list-keys` prints commands as tmux does**: full command names, flags
  grouped and quoted as tmux's, commands joined by `\;`.
- **`window-size` is `smallest` by default**, so every client sees the whole
  window; it was `latest`.
- **`C-u` clears the whole command prompt**, as tmux's, not only what is before
  the cursor.
- **Two panes colour half their shared border**, as tmux's default
  `pane-border-indicators colour`: only the half nearer the active pane takes
  `pane-active-border-style`. `pane-border-lines number` gives a border cell the
  index of the pane tmux gives it, looking from the active pane.
- **A window is sized by every client whose session holds it**, as tmux's
  `recalculate_size`: one linked into several sessions takes all their clients
  into account, not only one session's; `window-size latest` takes the window's
  latest client when more than one has it.
- **`list-keys` is laid out and ordered as tmux's**: keys in tmux's key-code
  order (characters, then special keys, then `M-`, `C-`, `C-M-` …), `bind-key`
  lines padded to the widest table and key with a `-r` column, and `-N` the root
  table then the prefix table, each key after the prefix key. A key nothing
  binds is an error (`unknown key: F5`), as is a table that does not exist.
- **`set-buffer -w` and `load-buffer -w` go to one client**, as tmux's: the OSC
  52 copy reaches the command's client (from a shell, the most recently used
  one) or `-t`'s, where it reached every attached client.
- **A dead pane's line is part of the pane**, as tmux's: it scrolls the pane up
  a row and stays in its history and `capture-pane`, where it used to be drawn
  over the last row. A pane a signal ended has an empty `#{pane_dead_status}`,
  not `1`.
- **`display-message` from a shell shows on a client**, as tmux's: with no `-c`
  or `-p` it takes the most recently used attached client and prints nothing,
  and with no client attached it shows nowhere; it used to print the text.
- **A menu may cover the status line**, as tmux's: `display-menu` is kept inside
  the whole client, not the rows above the status line.
- **A detached `new-session` is sized as tmux's**: `default-size` (80x24), not
  the terminal it was typed in, with `-x` and `-y` each taking one side (`-` the
  terminal's); an attaching one takes its terminal's size, `-x` / `-y` aside,
  and with `-x` or `-y` the session keeps its size as its own `default-size`.
  Sessions `resurrect` restores get `default-size` too.
- **Copy mode's `search-forward` / `search-backward` take a regular
  expression**, as tmux's: a search holding one of `^$*+()?[].\` is an extended
  regular expression (so the `/` and `?` prompts' searches are), one without is
  text; the new `search-forward-text` / `search-backward-text` always search for
  the text itself, and `search-again` repeats the kind of search it follows.
- **`tmxr_term::localtime::hour_minute` is
  `tmxr_command::localtime::hour_minute_second`**: the module moved to the
  format crate, which formats times, and the function returns the second too,
  for clock mode's clocks with seconds.
- **Pane targets `{left}`, `{right}`, `{top}` and `{bottom}` mean the pane at
  that edge of the window, as tmux's**: `{left}` and `{right}` were the
  neighbouring pane, which is now `{left-of}` / `{right-of}` / `{up-of}` /
  `{down-of}`; `{up}` and `{down}` are gone, as tmux has neither. The corners
  (`{top-left}` and the like) and the bare words (`bottom-right`) work too, as
  do `+N` / `-N` from the current pane and a window's `{next}` / `{previous}`.
- **`tmxr_proto::socket::format_tmxr_env` takes the session as an `Option`, and
  `parse_tmxr_env` returns it as one**: a job run with no session gets
  `TMXR=path,pid,-1`, as tmux writes `TMUX`.

### Fixed

- **A server nothing reaches no longer runs for ever**: one started for a client
  that gave up waiting for it (a slow start under load), or by a `start-server`
  that nothing followed, has no session and no client, and now goes after 15 s
  (with `exit-empty`, as tmux's server goes when it has no sessions).

- **Binds of the root table run in copy mode** when copy mode's table has none
  for the key or mouse event, as tmux's do: a `bind -n` key, the right-click
  menu (with its "Go To Top" and "Go To Bottom" items) and anything else bound
  only there did nothing in copy mode.
- **A tapped or quick right click keeps the pane menu open**, as tmux's: its
  release, read with the press, closed the menu the press had just opened, so a
  touchpad tap showed no menu. Input the terminal sends together is now taken
  together, as tmux takes one read: a menu, popup or `display-panes` open when
  it arrives takes it, and one opened by an event in it does not see the rest.
- **A comment in a command string spanning lines ended the whole string**: the
  commands on the lines after a `#` comment were dropped; a comment now runs to
  the end of its line, as in tmux.
- **The clipboard helper sleeps while it holds a copy** (Linux): hjkl-clipboard
  0.43.4, which tmxr now builds with, no longer polls the Wayland socket or the
  X11 connection every 50 ms; measured on Hyprland, 19.9 wakeups a second while
  holding a copy before, 0.0 after.
- **A window's new name or flag shows in every session listing it**: a window
  linked into another session, where it was not the current window, kept its old
  name and activity or bell flag in that session's window list until something
  else redrew it.
- **`#{pane_current_command}` names a program as soon as it starts** (Linux,
  macOS): it read the command tmxr keeps for automatic renames, refreshed once
  per `status-interval`, so a script asking just after starting a program (a key
  binding checking for vim) got the shell's name for up to 5 seconds. It is now
  read when asked, as tmux does. Windows still reads the kept copy, as a lookup
  there snapshots every process.
- **A pane printing fast costs about a quarter of the CPU it did**: each line
  that scrolled into history cost the screen emulator (the `vt100` crate) an
  allocation, a reallocation and a comparison per cell. tmxr now builds against
  a `vt100` with that fixed (offered upstream as doy/vt100-rust#47). On a
  laptop, the server's CPU for a pane printing `seq 1 2000000` went from about 2
  s to about 0.55 s, where tmux 3.8 spends about 0.6 s.
- **A pane's history takes about a tenth of the memory**: with 50000 lines of
  history the server held about 193 MiB, and entering copy mode once took it to
  about 600 MiB for good; now about 20 MiB, and about 30 MiB after copy mode,
  where tmux 3.8 holds about 23 MiB. The `vt100` tmxr builds against keeps a
  line of history only as long as its text (doy/vt100-rust#47) and narrows a
  line again once scrolled past (#49).
- **Copy mode starts about sixty times faster on a long history**: entering it
  copies the pane's history, which on a pane with 50000 lines took about 330 ms
  of the server's CPU, every client waiting meanwhile; now about 5 ms, where
  tmux 3.8 takes 4. The copy reads each line of history straight from the
  `vt100` tmxr builds against (doy/vt100-rust#50) instead of scrolling the pane
  through it a cell at a time, and keeps a line's cells only up to the last that
  is not blank.
- **`capture-pane` reads only the lines it prints**: it copied the pane's whole
  history first, so a capture of the screen took about 370 ms of the server's
  CPU on a pane with 50000 lines of history, and a script capturing in a loop
  kept the server busy. Now about 1 ms.
- **Command lines split as tmux's do** (found by new property tests): only
  spaces and tabs part words, where tmxr also split on Unicode's other spaces (a
  no-break space in a name); a line break ends a command as `;` does, where tmxr
  read it as a space, so a multi-line command string runs each line; `$0` and
  `$1x` stay as written, as a variable name starts with a letter or `_`.
- **Quoted words read back as they were**: a word starting with `{` (a session
  named `{x` from the session picker) is quoted instead of opening a block, and
  `show-options` writes a NUL in a value as `\000`, which reads back, where `\0`
  did not.
- **An idle tmxr no longer wakes the CPU several times a second** (the client on
  Linux and macOS, the server everywhere): the client's input thread polled the
  terminal every 50 ms to see whether to pause for a lock command, and the
  server ticked every 250 ms. The client now waits for input, a pause waking it,
  and the server sleeps until its next timed duty (the status line's
  `status-interval`, a message's expiry and the like), checking panes' commands
  for automatic renames in the same wakeup as a status redraw. Measured on a
  laptop, an attached, idle session went from about 25 wakeups a second to about
  1, which matters on battery.
- **Copies reach the desktop the copying client is on** (Linux and the BSDs):
  the local clipboard was set with the server's own display, so a server started
  from a TTY, or left running across a logout or a compositor restart, copied
  only through OSC 52. A copy now takes the client's `WAYLAND_DISPLAY` /
  `DISPLAY`, and a `tmxr __clipboard` helper holds it until the next copy. The
  SSH check follows the client too: a client over SSH gets OSC 52 alone, a local
  one the desktop clipboard, whichever way the server was started. On Linux the
  helper sleeps until the server is gone (the kernel tells it), costing no
  wakeups of its own.
- **A program's own tab stops are kept**: HTS (`ESC H`) sets one and TBC
  (`CSI g`) clears them, and tabs and back tabs move by them, as tmux's, where
  tmxr kept fixed stops every 8 columns. A width change, `ESC c` and
  `send-keys -R` put back the defaults.
- **The scroll region is kept as tmux keeps it**: inserting or deleting lines
  (`CSI L` / `CSI M`) with the cursor outside it now moves the rows from the
  cursor to the bottom, where it did nothing; a region set on one screen is kept
  across a switch to the alternate screen and back; and a region of fewer than
  two rows is ignored, where it reset the region and moved the cursor.
- **A terminal that reports a size of 0 is taken as 80x24**, as tmux takes it,
  where tmxr made it a window of one cell (a pty nobody sized, some CI runners).
- **A pane one row tall no longer takes the server down**: text that wrapped in
  it made the terminal emulator (`vt100`, now 0.18) panic, and the server with
  it, in release builds too.
- **A pane made taller gets back the rows a shrink pushed into its history**, as
  tmux's does, where tmxr added blank rows at the bottom: a split then closed
  leaves the screen as it was.
- **Keys typed faster than the client reads them all reach the pane** (Linux and
  macOS): once a read of the terminal filled crossterm's buffer, the rest of the
  input waited for the next key press, so a burst of more than 1024 bytes (a
  large unbracketed paste) arrived cut short.
- **A resize made as a key arrives reaches the server** (Linux and macOS): the
  client's terminal reader (crossterm, patched) dropped a SIGWINCH that became
  ready in the same wakeup as input, so the window kept the old size until the
  next resize. A tiling window manager (Hyprland) hits this when closing a
  window grows and focuses its neighbour: the terminal's focus report comes with
  the resize.
- **btop draws its boxes in place**: a pane now takes the cursor sequences tmux
  takes and its emulator (`vt100`) ignored, so a program using them no longer
  draws everything as one run of text. HVP (`CSI row;col f`, btop's cursor move)
  and HPA (`` CSI col ` ``) move the cursor, `CSI s` / `CSI u` save and restore
  it, REP (`CSI n b`) repeats the character just printed and CBT (`CSI n Z`)
  moves back tab stops, each as tmux 3.8 draws them.
- **`load-buffer -` and replies past 16 MiB go whole**: the client's standard
  input and a command's output are sent in pieces, where input that large failed
  on the client and a reply that large (`show-buffer` of a big buffer) came back
  as an error.
- **`assume-paste-time` counts a mouse event as input and is kept per session**,
  as tmux's: a key right after a click (or attaching) is timed from it,
  whichever of the session's clients it came from, where it was timed from the
  same client's key before.
- **The first status line is drawn from `status-format[0]` always**, as tmux's:
  `align=`, `fill=`, `list=` and ranges in `status-left`, `status-right` and the
  window formats now take effect there, where tmxr drew that line by its own
  rules unless `status-format[0]` was changed; `pane-border-format` is laid out
  the same way, over the border line.
- **`=` and `p` take their count from a format, and `=` cuts around styles**, as
  tmux's: `#{=/#{status-left-length}:status-left}` expands the length, where it
  stayed as text, and a `#[…]` style in the value takes no room in the count.
- **Resurrect restores an empty pane empty** (`split-window -I`), with no
  program, where it came back running the shell.
- **`list-keys` prints a bind's `{ … }` blocks in braces**, as tmux's, where it
  printed them as quoted strings; `show-hooks` prints a hook's commands joined
  by `;`, as tmux's, and takes a hook's name (or `name[i]`) to show that hook
  alone.
- **A status-format list too long for its room is cut as tmux's**: around its
  focused item (`#[list=focus]`), with its `<` / `>` markers, the text after it
  (`#[nolist]`) beside it; the parts are trimmed in tmux's order and the centre
  placed as tmux places it. A list given no alignment is not drawn, as tmux's.
- **`extended-keys off` ignores a program's request for extended keys**, as
  tmux's (xterm modifyOtherKeys, `CSI > 4 ; n m`), where tmxr honoured it; a
  level asked for before stays. The kitty keyboard protocol, which tmux does not
  have, is not affected. `#{pane_key_mode}` shows the level, as tmux's (`VT10x`,
  `Ext 1`, `Ext 2`; a pane started under `always` has `Ext 1`).
- **A session remembers every window visited, as tmux's stack**: killing the
  current window goes back to the one before it and `last-window` still has the
  one before that, where it said `no last window`; with none, the previous index
  is taken (wrapping to the highest), as tmux's, not the next.
  `#{session_stack}` and `#{window_stack_index}` show the stack.
- **The Homebrew formula loads on Homebrew 7.** Homebrew now loads every formula
  in a tap for each OS and architecture when it taps it, and this one only had
  its URLs under `on_macos`, so `brew tap kryptic-sh/tap` failed with "invalid
  syntax in tap" and no kryptic-sh tool could be installed. The URLs moved out
  of `on_macos` and `depends_on :macos` keeps the formula macOS-only.
- **The status line draws runs of `#` as tmux's**: `##` is one `#`, and `##[` a
  literal `#[` rather than a style, so a window named `a##b` shows `a#b`.
  `#{window_flags}` escapes the activity flag as `##`, as tmux's, and
  `#{window_raw_flags}` is added for the flags as they are.
- **A command run from a shell expands `client_*` for the best client**, as
  tmux's: `display-message`, `set -F` and the other formats a command expands
  took no client, so `#{client_flags}` or `#{client_session}` came out empty.
- **`#{client_flags}` ends with `UTF-8`**, as tmux's does for a UTF-8 client.
- **Copy mode's word moves and `select-word` take wide characters as tmux's**: a
  wide character's second cell counts as part of the word, where it split the
  word as a blank (`select-word` on `日本語xy` took one character).
- **`select-word` takes a word across a wrapped line**, as tmux's: the whole
  word, where it took only the part on the cursor's row.
- **Paste buffers as tmux's**: `#{buffer_sample}` is the first 200 bytes with
  control characters escaped (`\n`, `\t`, octal), cut with `...`, where it was
  50 characters with newlines as spaces; and setting a named buffer that exists
  makes it the newest, where it kept its place in the list.
- **A `default` status part style keeps the status line's colours**, as tmux's:
  `status-left-style`, `status-right-style` and `window-status-style` set to
  `default` drew their part in the terminal's colours instead of
  `status-style`'s.
- **Mouse modes as tmux's**: a program's `ESC [ ? 9 h` (X10) is ignored,
  resetting any of modes 1000 to 1003 clears the one set, and the UTF-8 (1005)
  and SGR (1006) encodings are kept apart, SGR winning for the events sent;
  `ESC c` clears them. `#{mouse_utf8_flag}` is added.
- **The alternate screen keeps the cursor where it was**, as tmux's: a program
  switching with `ESC [ ? 1049 h` now prints where its cursor stood, not at the
  top left; mode 1047 switches screens (it was ignored) and 47 clears the
  alternate screen as tmux's do, and switching off with 47 or 1047 leaves the
  cursor where it got to.
- **A window made from a shell has a latest client**, as tmux's: `new-window`
  and `break-pane` run from a shell give it the best attached client, so typing
  in it does not run `client-active` as if a client had just come to it.
- **`\;` on the command line**: an argument ending in `\;` is a literal `;`, and
  one ending in a bare `;` ends its command, as tmux's
  `cmd_parse_from_arguments`; `bind-key x cmd1 \; cmd2` binds both commands,
  where tmxr bound one broken command.
- **`send-keys -M` in a panned view** sent the mouse's place in the client, not
  in the window, to a program in a window larger than its client.
- **Templates are filled as tmux's** (`command-prompt`, `display-panes`): an
  answer goes into every `%N` of its number and only the next `%%`, and a `%`
  after either (`%1%`, `%%%`) escapes `"`, `\`, `$`, `;` and `~` in it.
- **`choose-tree -t` and `find-window -t` start from their target**, as tmux's:
  its session (and window) is the current one the picker marks and lists first,
  where `-t` was accepted and ignored.
- **A program's clipboard copy (OSC 52) is taken as tmux's**: only with
  `set-clipboard on` (`external`, tmux's default, leaves programs' copies
  alone), and it also becomes a paste buffer.
- **`run-shell` shows a command's output as tmux's**: its error output only with
  the new `-E`, a line saying how it failed (`'cmd' returned 3`,
  `'cmd' terminated by signal 9`) whenever it did, not only when it printed
  nothing, and a command client exits with the command's code (128 and the
  signal for a signal).
- **Copy mode's cursor moves across lines as tmux's**: `end-of-line` puts the
  cursor one past the last character, `cursor-right` there goes on to the next
  line and `cursor-left` at a line's start to the previous one's end, and up,
  down and the page moves remember the column, putting the cursor at a shorter
  line's end and back again (tmux's `lastcx`).
- **Copy mode's copy commands take tmux's arguments**: `copy-selection`,
  `copy-line` and `copy-end-of-line` take a prefix for the buffer's name
  (`copy-selection mine` makes `mine0`, numbered as tmux's buffers are), as does
  `copy-pipe` after its command; `-P` adds no buffer and `-C` sets no clipboard;
  and the prefix and copy-pipe's command are expanded as formats. Plain
  `append-selection` appends and clears the selection, staying in copy mode.
- **Copy mode's rectangle is the mode's, as tmux's**: toggled on, it holds for
  the next `begin-selection` and past `clear-selection`, until it is toggled off
  or a line or word is selected; and `select-line` puts the cursor at the line's
  end.
- **The `E`, `T`, `q` and `m` format modifiers chain with others** by `;`, as
  tmux's (`#{E;=5:@f}`, `#{q;=3:@x}`, `#{=1;m:a*,abc}`), where such a chain
  expanded to nothing; and `q` quotes only a variable's value, so `#{q:#{@x}}`
  is no longer quoted, as in tmux.
- **Copy mode's `page-up` / `page-down`** move `rows - 2` lines, as tmux's, not
  a whole screen, and at the top or bottom of the history they move the cursor
  on by the page as tmux's do.
- **A program that reads nothing no longer freezes the server.** Input for a
  pane (keys, pastes) was written on the server's own thread, so pasting into a
  raw-mode program that was not reading stopped every session and command (seen
  on Linux); each pane now has a writer thread.
- **A client that fell behind** (its frames dropped while it was not reading) is
  repainted on the server's next pass instead of waiting for another event; with
  `status-interval 0` it stayed stale until then.
- **`set-environment -h` / `show-environment -h`** were accepted and ignored;
  `-h` now sets a hidden variable, left out of new panes and jobs and shown only
  by `show-environment -h`, as tmux's.
- **`select-layout -o`** was accepted and ignored; it now goes back to the
  layout before the last `select-layout`, as tmux's (its cells to the panes in
  order, the last cells dropped when there are fewer panes).
- **`even-horizontal` / `even-vertical`, and the other panes of the `main-*`
  layouts**, give the spare cells to the first panes, as tmux's (30 rows for two
  panes are 15 and 14, not 14 and 15).
- **`source-file -q`** was accepted and ignored; a missing file is now no error
  with it, as tmux's. And a file that does not load no longer becomes the config
  to reload: a plain `source-file` after it reloads the old one.
- **`resize-pane -x` / `-y`** were accepted and ignored; they now set the pane's
  width or height, in cells or as a percentage of the window's (`-x 25%`), as
  tmux's `layout_resize_pane_to`.
- **`split-window -f`** was accepted and ignored; the new pane now spans the
  whole window (full width or height, sized against it), first or last in the
  pane list, as tmux's.
- **`confirm-before` and `command-prompt` from a script** (a command client) ask
  the attached client used most recently instead of failing with "no current
  client", their prompts show at once rather than at the next status-line
  redraw, and `command-prompt -t` names the client, as tmux's.
- **The active pane after `rotate-window` and `swap-pane`** is tmux's: the pane
  after the active one in the turned list (before it with `-D`), and for
  `swap-pane` the target pane (each window's arriving pane across windows); both
  unzoom a zoomed window unless `-Z`.
- **`respawn-window -t` respawns the window named**: it took the window's index
  for its id, so on a window whose index and id differ it failed with "no
  window" or respawned another window.
- **`display-time 0` keeps a message until a key is pressed**, as tmux's; the
  message went at once.
- **A reply too large to send is an error**: a command whose output was over the
  protocol's 16 MiB frame (`show-buffer` of a huge buffer) left its client
  waiting for good, as the server's writer stopped; it now gets an error saying
  the reply was over the limit, and the connection carries on.
- **A pane id names its window and session**, as tmux's: `kill-window -t %3` and
  `rename-window -t $TMXR_PANE` failed with "can't find window", and a session
  target now takes a window or pane id too (`list-windows -t %3`).
- **`copy-pipe` with a command that writes before it reads, or reads only part
  of its input**: the selection was written whole before the command's error
  output was read, so a command filling that output first never got the rest and
  never finished; and one that stopped reading early (`head -1`) was logged as a
  failure. The input now goes in alongside, and a command that stops reading is
  not a failure.
- **A pane kept by `remain-on-exit` fires `pane-died`, not `pane-exited`**, as
  tmux's; `pane-exited` is for a pane that goes away.
- **`capture-pane` gives one line per row, as tmux's**: a wrapped line came out
  joined and trailing blank rows were dropped; now each row is a line, blank
  ones kept and trailing spaces dropped, and `-J` joins wrapped rows and keeps
  trailing spaces.
- **A pane's kitty keyboard flag stack is capped**: a program pushing flags
  (`CSI > n u`) without end grew the server's memory; a push to a full stack now
  drops the oldest entry, as the kitty protocol asks.
- **The paste buffer limit drops only automatic buffers**, as tmux's: a buffer
  named with `-b` was dropped like any other once 50 were kept; now only the
  oldest `bufferN` makes way, and named buffers stay until deleted. A buffer set
  by name (`set-buffer -b buffer3`) counts as named from then on, as in tmux.
- **A `#[` style with no `]` ends the text there**, as tmux stops drawing: the
  status line showed the text before it twice (`ab#[fg` drew `abab#[fg`).
- **`run-shell`, `if-shell`, `copy-pipe` and `pipe-pane` commands run in their
  session's environment, with `TMXR`**, as tmux's jobs do: they had only the
  server's own environment, so a `tmxr` command in one talked to the default
  server instead of the one running it, and `set-environment` did not reach
  them. A `run-shell -t` job takes its target's session.
- **An environment variable that is not valid Unicode no longer crashes tmxr**:
  the client's hello and `show-environment -g` read the environment with
  `std::env::vars`, which panics on one, ending the client or the whole server;
  such variables are left out now (panes still inherit them).
- **`move-window -a` / `-b` and `link-window -a` / `-b`** put the window after
  (or before) the target and move the later windows up, as tmux's
  `winlink_shuffle_up`; `move-window` refused both flags, `link-window -a` took
  the next free index, and `-b` did not exist. `new-window -a` moves the later
  windows up too, and takes `-b`. A destination like `-t session:name` now finds
  the window of that name; it was read as no index at all.
- **`-t =name` finds only that exact name**, as tmux's: `has-session -t =foo`
  found a session `foobar` by its prefix, and `-t session:=window` found no
  window at all. A session or window name may also be an `fnmatch` pattern
  (`-t 'f*r'`) when no name starts with it, as in tmux.
- **A title holding `;` reaches the pane**: a program's OSC 0 / 2 title with a
  semicolon in it (`vim; a.rs`) was dropped, leaving the old `#{pane_title}`;
  everything after the first `;` is the title now, as xterm reads it.

## [0.9.0] - 2026-10-17

### Changed

- **Pickers open in normal mode**: in the session, window, buffer, client,
  `find-window` and `customize-mode` pickers, `j` / `k` move (as do the arrows
  and `C-n` / `C-p`), `Enter` picks and `q` or `Escape` closes. `i`, `I`, `a`,
  `A` or `/` start typing to filter, and `Escape` goes back to normal mode,
  keeping the query. Letters typed straight away no longer filter, and `Escape`
  while typing no longer closes the picker.

## [0.8.0] - 2026-10-17

### Added

- **`show-options -H`** adds the hooks to a list of options, as tmux's:
  `name[index] command` for each hook set at that scope, `-A` the inherited ones
  too, marked `*`. The global hooks show where tmux keeps them: `pane-exited`
  with `-wg`, the rest with `-g`.
- **`attach-session -f ignore-size`** (and `new-session -f`), as tmux's: the
  client's size sizes no window while a client without the flag is attached.

### Changed

- **Backslash escapes are read as tmux reads them** in commands, binds, hooks
  and imported tmux.conf lines: outside single quotes `\n`, `\t`, `\e`, `\s` and
  the other C escapes, `\ooo` octal and `\uXXXX` / `\UXXXXXXXX` give the
  character, and any other `\x` gives `x`, inside double quotes too (`"\q"` was
  kept as `\q`). A bad octal or `\u` escape, or a trailing backslash, is now an
  error, as in tmux.
- **`show-options` quotes values as tmux's** (`args_escape`): `@x "a b"`, not
  `@x 'a b'`, with tmux's escapes (`\t`, `\033`, `\~`, `\$`), and only for the
  options tmux types as strings, so a colour shows as `#ff0000` and the prefix
  as `C-a`. Window options in the global list and in `customize-mode` were
  printed unquoted, so a format with a space or a leading `#` did not read back;
  they are quoted now too. `set-option -a` appends to `default-shell`, a string
  in tmux, and no longer to `prefix` or the `*-colour` options.
- **A new session marks the `update-environment` variables its client lacks
  removed**, as tmux's (`show-environment` lists `-DISPLAY`), so its panes do
  not inherit the server's value. A session made by a key's command (`prefix :`
  then `new-session`) now takes the variables of the client the key came from;
  it took none.
- **Each client's status line keeps its own session's `status-interval`**, as
  tmux's: a session set to 60 no longer redraws every second because another is
  set to 1, and `status-interval 0` stops the timed redraws (it redrew each
  second). Panes' commands and automatic window names still refresh at the
  shortest interval, at least each second.
- **`set -g @x` and `set -wg @x` are two options**, as tmux's global session and
  global window options: `show -g @x` and `show -wg @x` each show their own, a
  window inherits the `-wg` one (`show -wA`), and a format finds the pane's, the
  window's, the global window one, the session's, then the global session one.
  They were one table, which `-wg` overwrote.
- **Alerts flag each session's link to a window**, as tmux's winlinks: a window
  linked into two sessions and in sight in one gets its bell, activity or
  silence flag in the other, where no flag showed. `display-message -t` expands
  in the target's session, so `display -p -t other:5 '#{window_flags}'` shows
  that session's link.

## [0.7.0] - 2026-10-14

### Added

- **Session options per session, as tmux's**: a session option set without `-g`
  (`set -t work status-left "W "`, `set prefix C-a`) is the target session's
  own, over the global value, for every session option tmxr has: `prefix`,
  `mouse`, `base-index`, `renumber-windows`, `history-limit`, the `status-*`
  options, `message-style`, `display-time`, `display-panes-*`, `repeat-time`,
  `default-shell`, `lock-command`, `lock-after-time` and the alert options. `-u`
  gives one back, and `show-options -t session` lists a session's own values
  (`-A` the inherited ones too, marked `*`). `display-panes-colour`,
  `display-panes-active-colour` and `display-panes-time` can now be set at run
  time.
- **`set-option -q` and `show-options -q`**: an unknown option (or an unset user
  option) is no error. `set-option -o` works for session options too.
- **`show-options -s`** lists the server options.
- **`detach-client -P`, `attach-session -x` and `new-session -X`**, as tmux's:
  the detached clients send their parent process (the shell they ran from)
  SIGHUP, which usually closes their terminal. A Windows client has no SIGHUP,
  so these are refused for one, detaching nobody.
- **`tmxr --import-tmux-conf FILE`** prints a tmux.conf as a tmxr config file:
  its global options (typed as tmxr's, `status-left` under `[status]`), binds,
  unbinds and `set-hook -g` hooks, with the lines it leaves out (plugins,
  `run-shell`, options tmxr does not have) listed at the top with why.
- **`update-environment` entries are patterns**, as tmux's: `SSH_*` takes every
  variable it matches (`*`, `?` and `[...]`, as `fnmatch`).
- **`load-buffer -`** reads the buffer from the command's standard input, as
  tmux's (`xclip -o | tmxr loadb -`). The client sends its input to the server,
  so this needs protocol 5: a running server from an older tmxr must be
  restarted (`tmxr kill-server`).
- **A 16-colour fallback**: `rgb-colour 16` sends only the 16 ANSI colours (RGB
  and 256-colour styles mapped to the nearest, by tmux's `colour_256to16`
  table), and `rgb-colour auto` uses it for a client whose `TERM` has no 256
  colours (`xterm`, `linux`, `screen`), as tmux does.
- **`attach-session -c`, `-E` and `-f read-only`, and `new-session -D`, `-E` and
  `-f`**, as tmux's: `-c` sets the session's working directory, `-E` leaves its
  environment alone, `-f read-only` attaches read-only (as `-r`; tmux's other
  client flags are refused), and `new-session -A -D` detaches the session's
  other clients as `attach-session -d` does.

### Changed

- **Breaking: the client-server protocol is now 5** (for `load-buffer -` and
  `detach-client -P`): after upgrading, a server still running from 0.6 must be
  restarted (`tmxr kill-server`); the client says so if it meets one.
- **Attaching updates the session's environment**, as tmux's: the
  `update-environment` variables of a client that attaches (`attach-session`,
  `new-session -A`) or switches to it (`switch-client`) replace the session's,
  and one the client does not have is marked removed, so new panes get that
  terminal's `DISPLAY`, `SSH_AUTH_SOCK` and the rest. `-E` (now also on
  `switch-client`) keeps the old values.
- **Breaking: `show-options` without `-g` reads the target session's own
  values**, as tmux's: `show -v status-left` prints nothing unless the session
  sets it (use `-g` for the global value), and `show-options` alone lists only
  what the session sets, where it listed every option.
- **Breaking: a user option set without `-w` or `-p` is the session's**
  (`set @x 1`), as tmux's, where it was global: another session no longer sees
  it. `set -g @x 1` sets a global one.
- **Breaking (library): `tmxr_config::Config`'s session options are now one
  table, `Config::session` (`SessionOptions`)**, in place of the `prefix`,
  `mouse`, `base_index`, `status` and other fields. The config file's keys are
  unchanged.

### Fixed

- **No console windows flash up on Windows**: the server, which runs with no
  console, gave every shell command it started (`run-shell`, `if-shell`,
  `copy-pipe`, `pipe-pane`, and the binds that use them) a console window of its
  own. They now run without one.
- **C-h and C-j reach the program in a pane on Windows**: hjkl, nvim and other
  programs in a tmxr pane got C-h as Ctrl+Backspace and C-j as Ctrl+Enter (the
  pane's ConPTY read their bytes that way), so their own C-h / C-j binds, such
  as moving between editor windows, did nothing. tmxr now sends those two keys
  to a Windows pane as win32-input-mode key events.
- **`synchronize-panes` no longer types into panes that cannot take it**: a
  sibling in copy or clock mode, a dead pane, or one hidden by a zoomed window
  is skipped, as tmux's.
- **`display-message -t` with a target that names nothing** fails with
  `can't find …`, as tmux's; it showed pane `%0`'s values instead.

## [0.6.0] - 2026-10-12

### Added

- **Session names may hold dots** (`new-session -s my.app`, `rename-session`),
  where tmux's turn a dot into `_`. Targets find them: `my.app`, `my.app:1` and
  `my.app:1.0`; a target without `:` that is exactly a session's name is that
  session, any other dotted word still `window.pane`. A `:` is still refused.
- **A bare `tmxr` opens the session named after the current directory**: `tmxr`
  in `~/src/my.app` attaches to session `my.app`, or makes it there (tmux makes
  a numbered session each time). A `:` in the directory's name becomes `_`; `/`
  and drive roots still get a numbered session.
- **`new-session -A`**, as tmux's: attach to the session named by `-s` when it
  exists (even with `-d`), else make it.
- **`[hooks]` in the config**: tmux's global hooks (`set-hook -g`), each a
  command or a list run in order, e.g.
  `session-created = ["new-window -d", "new-window -d"]` for three windows in
  every new session. An unknown hook name is reported; reloading the config
  replaces the global hooks.

### Changed

- **The first bare `tmxr` after a restore** attaches to its directory's restored
  session, or makes that one; it attached to whichever session was restored.

### Fixed

- **C-j and C-h move between panes on Windows in Alacritty, VS Code and other
  terminals that send keys as bytes**: the Windows console reported them as
  Ctrl+Enter and Ctrl+Backspace, so their binds never ran (C-k and C-l did).
  Outside Windows Terminal, those two chords are now read as C-j and C-h.

## [0.5.0] - 2026-10-12

### Added

- **Window and pane options, as tmux's**: a window can hold its own window
  options (`set-option -w`, or a window option set without `-g`) and a pane its
  own pane options (`set-option -p`: `allow-passthrough`,
  `pane-scrollbars-style`, `remain-on-exit` and `synchronize-panes`), over the
  global ones. `-u` gives one back to what it inherits (and with `-g` resets the
  global one to its default), `-U` clears a window's panes' too, and `-o`
  refuses one already set. `show-options -w` / `-p` (and `show-window-options`)
  list a window's or pane's own values; `-A` adds the inherited ones, marked
  `*`. User options (`@name`) can be set on a window or pane too, and formats
  find them pane, window, then global.
- **`automatic-rename`, `synchronize-panes` and `window-size` are options** with
  global values (and config keys), as tmux's: `rename-window` and
  `new-window -n` set a window's own `automatic-rename off`, and `resize-window`
  its own `window-size manual`.
- **`display-popup -k`, `-N`, and `display-popup` on an open popup**, as tmux's:
  `-k` lets any key close a popup whose command has exited, and a second
  `display-popup` changes the open popup (title, styles, border and, with `-N`,
  `-E` or `-k`, how it closes) instead of replacing it.
- **`paste-buffer -r` and `-s separator`**, as tmux's.
- **`menu-style`, `menu-selected-style`, `menu-border-style`,
  `menu-border-lines`, `popup-style`, `popup-border-style` and
  `popup-border-lines`**, as tmux's: a menu's or popup's look when its command
  gives no `-s`, `-H`, `-S` or `-b`. The defaults keep tmxr's look (borders as
  the active pane's, the selected item as copy mode's selection).
- **`display-menu -b` and `display-popup -b` take `simple` and `padded`**, as
  tmux's: ASCII lines, or blank ones that keep the box's size.
- **Menus take the mouse, as tmux 3.6's**: one the mouse opened (or
  `display-menu -M`) highlights the item under the pointer and runs the one the
  button is released on, closing on a release elsewhere; with `-O` a click runs
  an item and the menu stays open. A menu the keyboard opened ignores the left
  button and closes on the others. A right-click let go in place closes the
  menu, as in tmux: hold the button and release over an item.
- **`main-pane-height`, `main-pane-width`, `other-pane-height` and
  `other-pane-width`**, in cells or `N%`, as tmux's.
- **`alternate_on`, `scroll_position` and `selection_present`** format
  variables, as tmux's.
- **A tmux compatibility oracle** (`crates/tmxr-compat-oracle`): the same steps
  run in tmux 3.6 and tmxr, their results compared, over a corpus of layouts,
  formats, list commands, popups, menus and the mouse; CI runs it against tmux
  built from its release.
- **`set-option -F` and `set-window-option -F`** expand the value as a format
  for the target (else the current pane) before setting it, as tmux's.
- **Pane scrollbars, as tmux 3.5's**: `pane-scrollbars` (`off`, `modal` for copy
  mode only, or `on`), `pane-scrollbars-position` (`right` or `left`) and
  `pane-scrollbars-style` (with `width=` and `pad=`). A pane gives its bar the
  columns it takes, hides it on the alternate screen, and its slider shows where
  the view is; clicking above or below the slider pages through the history and
  dragging it scrolls (`copy-mode -S`), bound as tmux binds them. Letting go of
  the slider sends `MouseDragEnd1ScrollbarSlider`.

### Changed

- **Window options without `-g` are the target window's, as tmux's**:
  `set mode-keys emacs` now changes the current window only (it changed every
  window), and `show-options` without `-g` shows that window's own value, empty
  when it has none. Use `-g` for the global value. `show-options` with an
  unknown name fails (`invalid option: …`), as tmux's.
- **`synchronize-panes` is per pane, as tmux's**: typing in a pane reaches the
  window's other panes whose option is on, so `set -p synchronize-panes off`
  leaves one out.
- **Window options are one typed table**, as tmux's: `set-option -a` appends to
  the string ones (styles, formats, sizes), a bad value's error names the option
  (`remain-on-exit: bad value: x`), and a config file whose `mode-keys`,
  `menu-border-lines` or `popup-border-lines` is not one of tmux's names fails
  to load, as other bad values do.
- **Menus and popups are boxed as tmux's**: the title sits on the top border
  from its third cell, in the border's style and without padding; a menu's
  separators join its border (`├───┤`); and a menu with `-b none` keeps its
  border's cells and title. An unknown `-b` fails as tmux's does
  (`menu-border-lines unknown value: …`).
- **Popups close on tmux's keys**: a popup whose command has exited closes on
  Escape or C-c (any key with `-k`), not on any key; and a running popup without
  `-E` closes on Escape or C-c, as tmux's does.
- **`paste-buffer` sends what tmux's sends**: each newline as a carriage return
  (unless `-r` or `-s`), bracketed only with `-p` (it bracketed whenever the
  program asked), and to the target pane alone, not every synchronized pane.
  tmxr's binds all pass `-p`, as tmux's do.
- **The mouse binds are tmux 3.6's**: the wheel over a pane enters copy mode
  without scrolling (the next tick scrolls), and the wheel, drags and double /
  triple clicks pass to a program on the alternate screen or in copy mode, as
  tmux's `#{||:#{alternate_on},#{pane_in_mode},#{mouse_any_flag}}` binds do.
  Found by the oracle.

### Fixed

- **`send-keys` text reaches synchronized panes**: only key names (`Enter`) did,
  so the other panes got a bare Enter; tmux sends both.
- **A pane made shorter keeps its cursor's row**, as tmux's: rows below the
  cursor go first, then rows off the top into the history. The bottom rows were
  cut, so splitting a full pane lost the prompt line and left the cursor on old
  output. Found by the oracle.
- **A border drag ends with a `MouseDragEnd` key**, as tmux's: where the button
  is let go (`MouseDragEnd1Border`, `MouseDragEnd1Pane`, …), so it can be bound;
  the release was dropped.
- **Alt+Escape reaches panes and binds on Linux and macOS**: the client read it
  as a plain Escape (crossterm 0.29's input parser), so `M-Escape` binds never
  matched and programs got one ESC. The release binaries and packages build
  against a patched crossterm until the fix (crossterm-rs/crossterm#1143) is
  released; `cargo install tmxr` builds without it. Found by the oracle.
- **Panes keep their numbers in mirrored layouts**, as tmux's: the main pane
  stays pane 0 though laid out last, and `split-window`, `rotate-window` and
  `swap-pane` then follow tmux's pane list; resurrect restores them so. Found by
  the oracle.
- **`main-horizontal` and `main-vertical` size the main pane as tmux does**: 24
  rows or 80 columns by default (tmxr gave it 60%). Found by the oracle.
- **Sessions list in tmux's order**, by name: `list-sessions`,
  `list-windows -a`, `list-panes -a` and `switch-client -n` / `-p` went by
  creation order. Found by the oracle.

## [0.4.0] - 2026-10-11

### Added

- **Panning, as tmux's**: a window larger than the client (from `resize-window`)
  follows the active pane's cursor, and `refresh-client -L` / `-R` / `-U` /
  `-D [n]` move the view, `-c` goes back to following the cursor; bound to
  `prefix S-arrows` and `prefix DC`. Clicks, border drags and boxes placed at
  the mouse or a pane follow the view. Before, only the window's top-left corner
  showed. `refresh-client` from a command client acts on the current client.
- **`prefix C`** opens `customize-mode`, as tmux's.
- **tmux's menus** on `prefix <`, `prefix >` and the right button over a pane, a
  window's name or the status line's left part, bound as tmux 3.6 binds them. A
  menu item runs against the menu's target and the mouse event that opened it,
  as tmux's: a right-click's items act on the pane clicked.
- **`mouse_word`, `mouse_line`, `mouse_x`, `mouse_y` and `pane_mode`** for
  commands a mouse event runs, and **`copy-mode -q`, `-d` and `-H`**.
- **More of tmux's format language**: `#{<:…}`, `#{>:…}`, `#{<=:…}`, `#{>=:…}`
  (string comparisons, as tmux's), `#{m:glob,text}` and `#{m/r:regex,text}`
  (with `i` for any case), `#{q:…}` and `#{q/e:…}` quoting, and
  `#{=/N/marker:…}` truncation, each checked against tmux 3.6's output.
- **`main-horizontal-mirrored` and `main-vertical-mirrored` layouts** (main pane
  below or to the right), in `next-layout`'s order and on `prefix M-6` / `M-7`,
  as tmux 3.5 added them.
- **vi `#` / `*` in copy mode** search back and on for the word at the cursor,
  as tmux's, through the new `send-keys -F` (arguments are formats) and the
  `copy_cursor_word`, `copy_cursor_x` and `copy_cursor_y` variables. A copy
  command's arguments may start with `--`, as in tmux's binds.
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

- **Splits size panes as tmux does**: the new pane gets the smaller half (a
  100-column split is 50 + 49, not 49 + 50), and `-l` / `-b -l` sizes and their
  limits follow tmux 3.6's `layout_split_pane`; tmxr also kept a split between
  5% and 95%, which tmux does not.
- **`-t =` off a pane or window** (a click on the status line) means the
  window's active pane, or the session's current window, as tmux's does; it
  failed with "no pane under the mouse".
- **`display-menu` follows tmux's item rules**: an item whose name expands to
  nothing is left out (tmxr showed a separator), and a separator at the top or
  after another is dropped. `#[…]` styles in a menu's title and items are
  applied, and `#[align=centre]` centres the title, instead of being shown as
  text.
- **`swap-window`, `join-pane` and `move-pane` without `-s`** use the current
  window or pane when none is marked, as tmux's do; they failed with "needs -s
  or a marked pane".
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

[Unreleased]: https://github.com/kryptic-sh/tmxr/compare/v0.12.2...HEAD
[0.12.2]: https://github.com/kryptic-sh/tmxr/compare/v0.12.1...v0.12.2
[0.12.1]: https://github.com/kryptic-sh/tmxr/compare/v0.12.0...v0.12.1
[0.12.0]: https://github.com/kryptic-sh/tmxr/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/kryptic-sh/tmxr/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/kryptic-sh/tmxr/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/kryptic-sh/tmxr/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/kryptic-sh/tmxr/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/kryptic-sh/tmxr/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/kryptic-sh/tmxr/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/kryptic-sh/tmxr/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/kryptic-sh/tmxr/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/kryptic-sh/tmxr/compare/v0.2.4...v0.3.0
[0.2.4]: https://github.com/kryptic-sh/tmxr/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/kryptic-sh/tmxr/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/kryptic-sh/tmxr/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/kryptic-sh/tmxr/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/kryptic-sh/tmxr/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/kryptic-sh/tmxr/releases/tag/v0.1.0
