# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking

- **The client-server protocol is now 6** (the client sends its process id, for
  `#{client_pid}`): after upgrading, a running server must be restarted
  (`tmxr kill-server`); the client says so if it meets one.
- **`break-pane -t` is the window the pane becomes**, as tmux's, not the pane to
  break: that is `-s` now (the current pane without it). `-t` takes an index
  (`-t :5`), `-a` / `-b` place it next to the target window, and `-n` names it
  (automatic-rename off). A window's only pane no longer refuses: the window
  moves, as tmux's.

### Added

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

[Unreleased]: https://github.com/kryptic-sh/tmxr/compare/v0.9.0...HEAD
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
