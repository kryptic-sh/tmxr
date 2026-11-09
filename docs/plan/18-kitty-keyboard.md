# 18 — The kitty keyboard protocol

The owner asked (2026-11-09) for two things of equal weight:

1. **Full kitty keyboard protocol support**: every flag, both towards the
   terminal a client runs in and towards the programs in panes. The client and
   server change the protocol's flags whenever what is focused wants something
   else. Kitty-style binds are added beside tmux's.
2. **Full backwards compatibility for plain input**: a client in a terminal
   without the protocol sends keys as a VT does (ASCII control bytes and escape
   sequences, with typed text as UTF-8) and loses nothing it has today. tmux's
   key names and binds keep working unchanged on either kind of client.

tmux has no kitty protocol, so the first half goes beyond tmux and the
compatibility oracle cannot check it. The second half the oracle keeps checking
as it does now.

## Where it stands

Read in the tree on 2026-11-09:

- **Client → terminal.** `Guard::enter` (`tmxr-client/src/terminal.rs`) pushes
  `DISAMBIGUATE_ESCAPE_CODES` (flag 1) once, where crossterm's
  `supports_keyboard_enhancement` probe says the terminal speaks the protocol,
  and pops it on exit. Nothing changes it while attached.
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
  only (`modes.kitty_flags & 1`).
- **Binds.** `Key::normalise` (`tmxr-command/src/keys.rs`) folds Shift into a
  character and lowercases a character held with Ctrl, so key tables know only
  tmux's `C-` / `M-` / `S-` model.

## Known bugs this plan fixes first

- **Alt+Shift loses its Shift under a kitty-protocol terminal.** The terminal
  sends Alt+Shift+a as `CSI 97;4u` (the unshifted key plus Shift, as flag 1
  says); crossterm reports `'a'` with Shift and Alt, and `normalise` and
  `encode_key` drop the Shift. In Alacritty, kitty, foot, Ghostty and WezTerm,
  M-A reaches a program as M-a and `bind -n M-H` never fires (`M-h` runs
  instead). _Reproduced_ by the 2026-11-02 reviewer. Fix: also push flag 4
  (alternate keys), so the terminal sends the layout's shifted character, and,
  since crossterm then reports Ctrl+Shift+h as `'H'` with Ctrl (which
  `normalise` lowercases to C-h), read an uppercase ASCII letter held with Ctrl
  as the lowercase letter plus Shift. Ctrl+Shift+1 arrives as `C-!`, the same
  key tmux names. Superseded by phase 3's own parser, but small enough to ship
  first.
- **The query reply claims flags tmxr ignores.** `CSI ? u` answers the top of
  the stack as pushed, so a program pushing flag 8 is told it is on and then
  gets legacy keys. Reply with the pushed flags masked to those tmxr encodes,
  widening the mask as each flag lands. Whether kitty itself masks this way is
  to be confirmed against the spec.

## Design

### Capabilities, both ways

- **A client says what its terminal speaks.** `TerminalInfo` gains a keyboard
  field: `Legacy` (bytes as a VT sends them: UTF-8 text and escape sequences,
  not ASCII only), or `Kitty`. The protocol's query reports the current flags,
  not the supported ones, so a client learns support from the probe and assumes
  every flag on a kitty terminal; a terminal that ignores one just never sends
  what it adds. A middle tier, xterm's modifyOtherKeys, which tmux asks the
  outer terminal for where terminfo or `terminal-features` says `extkeys`, is a
  decision below.
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

- tmux's names keep matching. Both encodings of a key normalise to the same
  `Key`: `M-A` from `ESC A` and from `CSI 97;4u`, `C-h` from `0x08` and from
  `CSI 104;5u`.
- Kitty-style binds are added for what tmux cannot name: Super, Hyper, Meta,
  keypad keys apart from the main keys, the lock-state modifiers, Shift on a
  letter held with Ctrl. A release or repeat never runs a bind; a repeat runs it
  as a press, as an auto-repeating legacy terminal already does.
- `send-keys`, `list-keys` and the hjkl `<…>` names learn the new names.
- A bind on a key a legacy terminal cannot send (C-S-h, Super) just never fires
  there; `list-keys` could mark it.

### Fallback guarantees

What a legacy client keeps, each one a test:

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

## Decisions for the owner

1. **Kitty-style bind syntax.** Options: extend tmux's prefixes (`Super-k`,
   `Hyper-k`, `C-S-h` kept apart from `C-h`, `KP_Enter`); or take kitty's own
   (`ctrl+shift+h`, `super+k`); or both, normalised to one `Key`. The first
   keeps one syntax in a config; the second matches kitty's docs.
2. **modifyOtherKeys as a middle tier** for xterm and terminals like it: ask the
   outer terminal for it, as tmux does, or leave those terminals legacy.
3. **`extended-keys off`**: whether it also stops the kitty protocol for panes
   (today the kitty path bypasses the option).
4. **Bind on a key the client cannot send**: silent, or marked in `list-keys`.

## Order

1. The Alt+Shift fix and the honest query reply (above). Shippable alone.
2. The emulator's per-screen stacks.
3. tmxr's own key type and parser; the protocol bump; `TerminalInfo`'s keyboard
   field. No behaviour change yet beyond what the parser fixes, which includes
   the backlog's real Ctrl+4 … Ctrl+7 that crossterm reports as the legacy
   control byte (`tmux_key` at level 2).
4. `ServerMsg::KeyboardFlags` and the choke point that follows focus.
5. `encode_key` for flags 2, 4, 8 and 16, with the fallback and the synthesised
   releases.
6. Kitty-style binds, `send-keys` and `list-keys`, after decision 1.
7. Windows.

## Testing

- The oracle cannot help: tmux has no kitty protocol.
- Unit tests for the parser and the encoder against the spec's own examples.
- e2e: the harness's terminal answers the probe as a kitty terminal and sends
  `CSI … u` keys; assert what a pane's program reads (a small reader that pushes
  flags and echoes its input) under each flag, and what the client is told to
  ask for as focus moves.
- e2e for the fallback: the same assertions from a harness terminal that answers
  no kitty probe, and one of each kind attached to one session.
- By hand: Alacritty, kitty, foot, Ghostty and WezTerm, plus one legacy terminal
  (xterm), plus tmxr inside tmxr and tmxr inside tmux (legacy, since tmux
  answers no query).
