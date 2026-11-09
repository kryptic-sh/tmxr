# 18 — The kitty keyboard protocol

The owner asked (2026-11-09) for two things of equal weight:

1. **Full kitty keyboard protocol support**: every flag, both towards the
   terminal a client runs in and towards the programs in panes. The client and
   server change the protocol's flags whenever what is focused wants something
   else. Kitty-style binds are added beside tmux's.
2. **A middle tier for xterm-style extended keys** (decided 2026-11-09), as tmux
   already supports: xterm's modifyOtherKeys towards terminals that have it but
   not the kitty protocol.
3. **Full backwards compatibility for plain input**: a client in a terminal
   without the protocol sends keys as a VT does (ASCII control bytes and escape
   sequences, with typed text as UTF-8) and loses nothing it has today. tmux's
   key names and binds keep working unchanged on either kind of client.

tmux has no kitty protocol, so the first goes beyond tmux and the compatibility
oracle cannot check it. The other two are tmux's behaviour.

## Where it stands

Read in the tree on 2026-11-09, after step 1 (`bde1ed4`):

- **Client → terminal.** `Guard::enter` (`tmxr-client/src/terminal.rs`) pushes
  flags 1 and 4 (`DISAMBIGUATE_ESCAPE_CODES | REPORT_ALTERNATE_KEYS`) once,
  where crossterm's `supports_keyboard_enhancement` probe says the terminal
  speaks the protocol, records that in `KITTY_KEYS` for the input thread, and
  pops them on exit. Nothing changes them while attached. The input thread's
  `kitty_key` reads Ctrl with an uppercase letter as the letter with Shift,
  undoing crossterm's swap for letters (`unshifted_letter`, in `encode.rs`).
- **Client → server.** Keys travel as crossterm's own events,
  `ClientMsg::Input(Vec<crossterm::event::Event>)` (`tmxr-proto`), so the server
  sees only what crossterm keeps. crossterm 0.29 parses flag 4 by swapping in
  the shifted character and dropping the unshifted key and the base-layout key,
  and has no flag 16 at all (`REPORT_ASSOCIATED_TEXT` is commented out).
  `Hello.terminal` (`TerminalInfo`) says nothing about the keyboard.
- **Server → client.** There is a pattern to follow: `ServerMsg::Mouse(bool)`
  and `ServerMsg::FocusEvents(bool)` already switch a client's terminal modes.
- **Panes.** The emulator (`tmxr-term/src/emulator.rs`) keeps one stack of kitty
  flags per pane (`CSI > u` push, `CSI < u` pop, `CSI = u` set, `CSI ? u` query;
  cleared by `ESC c`). `encode_key` (`tmxr-term/src/encode.rs`) acts on flag 1
  only, and the `CSI ? u` reply is masked to `KITTY_FLAGS_ENCODED`, which each
  later step widens.
- **Binds.** `Key::normalise` (`tmxr-command/src/keys.rs`) folds Shift into a
  character and lowercases a character held with Ctrl, so key tables know only
  tmux's `C-` / `M-` / `S-` model.

## Design

### Capabilities, both ways

- **A client says what its terminal speaks.** `TerminalInfo` gains a keyboard
  field with three tiers, the best the terminal has:
  - `Kitty`: the probe found the kitty protocol. The protocol's query reports
    the current flags, not the supported ones, so the client assumes every flag;
    a terminal that ignores one just never sends what it adds.
  - `Xterm`: the terminal has xterm's modifyOtherKeys (tmux's `extkeys`
    feature).
  - `Legacy`: bytes as a VT sends them (ASCII control bytes and escape
    sequences, typed text as UTF-8).

- **The server says what a client should ask for.** A new
  `ServerMsg::KeyboardFlags(u8)`, beside `Mouse` and `FocusEvents`. The client
  applies it with `CSI = flags ; 1 u` and ignores it on a legacy terminal.
- **Which flags.** For each client, the flags of the pane its keys go to,
  unioned with flag 1 and flag 4, which tmxr itself needs to tell keys apart.
  When tmxr reads the keys itself (copy mode, the command prompt, menus,
  choose-tree and the other modes, the session picker, a held `prefix` table),
  only tmxr's own flags. A popup is a pane with flags of its own.
- **When the server sends it.** On every change of the pane a client's keys go
  to: select-pane, a click, select-window, a pane or window closing, a popup
  opening or closing, entering or leaving a mode or a prompt, switch-client,
  attach. And when the focused pane's program pushes, pops or sets flags or
  switches screens. One choke point that compares with what the client last got
  and sends only on change, rather than a send at each site.

### The xterm tier, as tmux

Read in tmux 3.6's source (`tty-features.c`, `tty-keys.c`, `tty.c`) on
2026-11-09:

- **Which terminals.** tmux gives a terminal the `extkeys` feature from its name
  in the XTVERSION or secondary DA reply (`XTerm(`, `foot(`, `mintty `,
  `iTerm2 `; not `tmux `), or from the `terminal-features` option. tmxr picks
  terminal features by `TERM` (`Server::client_underline_styles`) and has no
  `terminal-features` option, so this tier needs both: the client asks for
  XTVERSION and reports the name, and the option is added with at least
  `extkeys`.
- **When.** tmux sends `CSI > 4 ; 2 m` (modifyOtherKeys level 2, its `Eneks`) to
  such a terminal when the `extended-keys` option is not `off`, and `CSI > 4 m`
  (`Dseks`) when it stops. One mode for the whole attach: unlike the kitty tier
  it does not follow focus, so `ServerMsg::KeyboardFlags` is for kitty clients
  alone.
- **What comes back.** Modified keys as `CSI 27 ; mods ; key ~`, or
  `CSI key ; mods u` under `formatOtherKeys`; tmux reads both
  (`tty_keys_extended_key`). crossterm reads the `u` form but not the `27 ;…~`
  one, so this needs the own parser below.
- **What it gives.** Modifiers on any key (C-S-h apart from C-h, C-i apart from
  Tab, Alt keys without a bare Escape in front). No releases, no alternate or
  base-layout keys, no Super or Hyper, and a bare Escape stays ambiguous.
- **When a terminal has both,** the kitty tier wins.

### Keys as tmxr's own type

- A key event type in `tmxr-proto` carrying all the protocol reports: the key
  (unshifted), the shifted key, the base-layout key, every modifier (Shift, Alt,
  Ctrl, Super, Hyper, Meta, Caps Lock, Num Lock), the event type (press, repeat,
  release) and the text the key produces.
- The client parses `CSI … u` (and the protocol's legacy-compatible forms)
  itself into that type, and maps crossterm's events into it for everything
  else, so the loss in crossterm stops at the client.
- This changes the client–server protocol: bump `Hello.protocol`, and say so
  under Breaking.
- **The decoder and key type come from `ttyr`** (decided by the owner,
  2026-11-09): a new kryptic-sh crate replacing crossterm entirely in tmxr, hjkl
  and the other house projects, since crossterm is the bottleneck for both
  (hjkl's plan `docs/plan/01-kitty-keyboard.md`, phase 3, needs the same
  decoder, and hjkl has the same Alt+Shift bug). ttyr owns input (its own event
  types, the full kitty, xterm `CSI 27 ;…~`, legacy, SGR mouse, paste, focus and
  terminal-report decoder, a Unix tty and Windows console reader, blocking and
  async), raw mode, size, output commands, and a ratatui backend for the
  projects that draw with ratatui. It takes over every fix in tmxr's crossterm
  fork (`[patch.crates-io]` in `Cargo.toml`), which goes once nothing uses
  crossterm. ttyr keeps its own plan in its own repo; this step waits on its
  input half.
- hjkl's phase 6 (Super/Meta, F13–F35, keypad and media keys) changes
  `hjkl-picker-tui::handle_key`, which tmxr's session picker calls; the version
  bump is coordinated with hjkl.

### Options

Decided by the owner (2026-11-09):

- **`extended-keys` stays exactly tmux's** (`off`, `on`, `always`, with
  `extended-keys-format`) and covers the xterm tier alone: whether a client asks
  its terminal for modifyOtherKeys, and what panes get, as tmux does. A tmux
  config means the same in tmxr, and the oracle keeps comparing it.
- **`kitty-keys`, a new server option, `on` or `off`, default `on`**, beside
  tmxr's other own server options (`rgb-colour`, `socket-access`) in
  `SERVER_OPTIONS`. It covers the kitty tier in both directions. `off`: no
  client asks its terminal for the protocol (each falls to the xterm tier or
  legacy), and the emulator ignores a pane's pushes and answers the query with
  no flags, so programs fall back themselves. No `always`: the protocol is one a
  program asks for, and kitty encoding sent unasked breaks a program. `on` keeps
  today's behaviour, where the client already pushes flag 1.
- **The two are independent.** With `extended-keys off` and `kitty-keys on`, a
  kitty terminal still gets the protocol, an xterm-style one goes legacy, and a
  pane that pushes kitty flags still gets them. A tmux config with
  `extended-keys off` therefore still sees kitty keys in tmxr, as it does today;
  `kitty-keys off` is the switch for that.
- Changing either while clients are attached takes effect at once: the server
  sends each client its new tier's request (`ServerMsg::KeyboardFlags`, or
  `CSI > 4 ; 2 m` / `CSI > 4 m`).

### Keys into panes

- `encode_key` honours flags 1, 2, 4, 8 and 16 per the spec, from the pane's
  stack.
- **Fallback.** Keys from a legacy client carry no releases, no alternate keys
  and no base-layout key; the encoder sends what it can (a press, the key as
  typed) and nothing invented. A pane that asked for releases from a legacy
  client gets none, as from a terminal that ignores flag 2.
- **Keys tmxr synthesises:** a release for every key held when focus moves away
  from a pane that asked for releases, so no program sees a key stuck down;
  `send-keys` keys, which have no release.
- **Separate stacks for the main and alternate screens.** The emulator keeps
  one, so leaving the alternate screen keeps a program's flags; the protocol
  keeps one per screen (_reported_ by an earlier review, moved here from the
  backlog; to confirm against the spec).

### Binds

Decided by the owner (2026-11-09): tmux-style and kitty-style key names are both
accepted, anywhere a key is named, and either can be used.

- **tmux's names keep matching.** Both encodings of a key normalise to the same
  `Key`: `M-A` from `ESC A` and from `CSI 97;4u`, `C-h` from `0x08` and from
  `CSI 104;5u`.
- **tmux's names grow** to cover what the protocol reports and tmux cannot name:
  `Super-`, `Hyper-` and `Meta-` prefixes, `C-S-h` kept apart from `C-h`, keypad
  keys apart from the main keys (tmux's own `KP*` names), the lock-state
  modifiers.
- **kitty's names are accepted as written in kitty's docs**: `+`-joined
  modifiers (`ctrl`, `shift`, `alt`/`opt`/`option`, `super`/`cmd`/`command`,
  `hyper`, `meta`, `caps_lock`, `num_lock`) and kitty's key names (`f1`,
  `enter`, `kp_enter`, `left`, `page_up`, …), so `ctrl+shift+h` and `super+k`
  bind. The exact list is transcribed from the spec's key table, with a test per
  name.
- **One key, two spellings.** `M-H` and `alt+shift+h` parse to the same `Key`:
  binding one replaces a bind on the other, and `unbind` takes either.
- **Printing.** `list-keys` and `#{key}`-style formats print a key in tmux's
  form where tmux has a name for it, so output the oracle compares stays tmux's,
  and in the extended tmux form otherwise (`Super-k`). Not printing in the
  spelling the bind was written in is deliberate: a `Key` holds no spelling.
- **Sequences stay tmux's.** kitty's `ctrl+a>x` multi-key maps are not key
  names; tmux's key tables (`bind -T`, `switch-client -T`) already do that.
- A release never runs a bind; a repeat runs it as a press, as an auto-repeating
  legacy terminal already does.
- `send-keys`, `list-keys`, `bind`, `unbind` and the config take both forms.
- **The hjkl `<…>` form follows hjkl, not kitty** (hjkl's reply, 2026-11-09):
  vim/neovim notation, no `alt+shift+h` inside `<…>`. `parse_bracketed`
  (`keys.rs`) already takes `C-`, `S-`, `A-` and `M-` in any case and order; it
  gains neovim's `D-` (Super/Cmd) and `T-` (Meta), and neovim's key names as
  hjkl adds them (`<F13>`, `<kEnter>`). A shifted letter is its uppercase
  (`<M-A>`), `<M-S-a>` is the same key, and `<C-S-h>` is apart from `<C-h>`.
- **A bind the client cannot use is listed as such** (decided 2026-11-09). A
  bind on a key the client's tier cannot send (C-S-h from a legacy terminal,
  Super from an xterm-tier one) never fires there, and `list-keys` says so with
  a trailing comment naming the tier it needs, `# needs kitty keys` or
  `# needs extended keys`, which keeps each line a valid command. "The client"
  is the one `list-keys` runs for: the attached client that ran it from a bind
  or the prompt, or the client attached to the pane it was typed in. A command
  client with none prints no marks. Every default bind is sendable from a legacy
  terminal, so the oracle's `list-keys` cases see no marks.

### Fallback guarantees

What a legacy client keeps, each one a test; an xterm-tier client keeps all of
it too, and gains the modified keys its tier reports:

- Every key it can send today reaches panes as the same bytes, under every
  `extended-keys` setting, as the oracle already checks.
- Every tmux bind it can fire today still fires; no default bind needs the kitty
  protocol.
- A pane that asked for kitty flags still gets kitty encoding of the keys a
  legacy client sends (the encoder works from tmxr's key type, not from the
  client's terminal), minus what a legacy terminal never reports: releases,
  alternate keys, the base-layout key, Super and Hyper.
- Clients of both kinds on one session each work: the server never assumes a
  client's keyboard from another's.
- A legacy client is never sent `ServerMsg::KeyboardFlags`, so nothing new is
  written to its terminal.

### Lifecycle

- Detach, suspend (`ServerMsg::Suspend`), lock and a client crash already pop
  the flags; resume and unlock must apply the current ones again.
- Keys typed while a change of flags is in flight arrive in the old encoding.
  The parser reads every encoding whatever was asked, so only releases can go
  missing; the synthesised releases above cover a key held across the change.

### Platforms

- **Windows client.** The console hands over input records, not bytes
  (`keys_from_console_bytes`); under Windows Terminal's win32-input-mode they
  carry key-up events, which could feed flag 2. ConPTY behind another terminal
  is legacy.
- **Windows panes.** Programs behind ConPTY read console input records, and
  `encode_key`'s ConPTY path (`conpty_key`) translates for them; whether a kitty
  sequence survives ConPTY at all is not known. Expect panes on Windows to stay
  at flag 1 or below.

## Order

1. Done in `bde1ed4` (the Alt+Shift fix and the honest query reply); the line
   stays so later steps keep the numbers ttyr's and hjkl's plans cite.
2. The emulator's per-screen stacks.
3. Adopt `ttyr`'s key type and parser (blocked on ttyr's input half); the
   protocol bump; `TerminalInfo`'s keyboard field. No behaviour change yet
   beyond what the parser fixes, which includes the backlog's real Ctrl+4 …
   Ctrl+7 that crossterm reports as the legacy control byte (`tmux_key` at level
   2), and what step 1 left (in the CHANGELOG's Changed entry): a kitty-aware
   pane gets Alt/Ctrl+Shift with a digit or symbol as the shifted character
   (`CSI 33;3u` for Alt+Shift+1, where the protocol says `CSI 49;4u`), and a
   letter's unshifted key is guessed from Unicode case (`unshifted_letter`),
   wrong on layouts like Turkish. With the unshifted key from the terminal,
   `kitty_key` and `unshifted_letter` go, and the pinned test
   `kitty_sends_a_shifted_symbol_as_itself_without_shift` changes.
4. `ServerMsg::KeyboardFlags`, the choke point that follows focus, and the
   `kitty-keys` option.
5. The xterm tier: XTVERSION in the client's probe, `terminal-features`,
   `CSI > 4 ; 2 m` under `extended-keys`.
6. `encode_key` for flags 2, 4, 8 and 16, with the fallback and the synthesised
   releases.
7. Kitty-style key names and the extended tmux names, in every place a key is
   named; `list-keys` marking the binds the client cannot use.
8. Windows.

## Testing

- The oracle cannot help: tmux has no kitty protocol.
- Unit tests for the parser and the encoder against the spec's own examples.
- e2e: the harness's terminal answers the probe as a kitty terminal and sends
  `CSI … u` keys; assert what a pane's program reads (a small reader that pushes
  flags and echoes its input) under each flag, and what the client is told to
  ask for as focus moves.
- The xterm tier is tmux's behaviour, so where the oracle can drive a client's
  terminal input it compares tmxr with tmux; whether it can is to be checked.
- e2e for the xterm tier: a harness terminal answering XTVERSION as `XTerm(…)`
  must receive `CSI > 4 ; 2 m`, and its `CSI 27 ;…~` keys must reach binds and
  panes.
- e2e for the options: `kitty-keys off` sends a kitty client no flags and a
  pane's query answers none; `extended-keys off` stops `CSI > 4 ; 2 m` and
  leaves the kitty tier alone; each takes effect on attached clients.
- e2e for `list-keys`: a bind on `C-S-h` is marked for a legacy client and not
  for a kitty one; a command client prints no marks.
- e2e for the fallback: the same assertions from a harness terminal that answers
  no kitty probe, and one of each kind attached to one session.
- By hand: Alacritty, kitty, foot, Ghostty and WezTerm, plus one legacy terminal
  (xterm), plus tmxr inside tmxr and tmxr inside tmux (legacy, since tmux
  answers no query).
