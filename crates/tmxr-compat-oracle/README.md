# tmxr-compat-oracle

Runs the same steps in tmux and in tmxr and compares what each reports, for
tmux-compatibility regression testing, as `hjkl-compat-oracle` does for hjkl
against neovim. Workspace-only (`publish = false`).

Each case starts a session in both multiplexers, optionally attaches a client on
a pseudo-terminal, runs its steps (commands, keys, mouse events at cells or at
text on the client's screen), then compares its checks: a format, a command's
output, whether text is on the screen, or where it is. A check may also carry
the value it must have.

## Running

```sh
cargo build -p tmxr-cli
TMUX_BIN=$(command -v tmux) cargo test -p tmxr-compat-oracle -- --nocapture
```

`TMUX_BIN` names the tmux to compare against; without it the oracle checks tmxr
alone against the cases' expected values, and skips cases that have none or need
a Unix shell (as on Windows). The corpus is measured against tmux 3.6: another
version differs for its own reasons. `TMXR_BIN` names the tmxr under test
(default: the workspace's debug build). With `ORACLE_REQUIRE_TMUX` set, as CI
sets it, a missing tmux fails instead of skipping.

The tmux side runs with `corpus/tmux.conf`, the owner's tmux config without its
plugins, whose settings tmxr's defaults carry; the tmxr side with its defaults.
Both run `/bin/sh`, with their own sockets and directories.

## Adding a case

Cases live in `corpus/*.toml` (one test per file in `tests/oracle.rs`):

```toml
[[cases]]
name = "a_drag_copies_the_selection"
attach = true                               # a client, for keys and the mouse
size = [100, 30]                            # default
program = "printf 'alpha beta\\n'; exec sleep 1000" # by /bin/sh -c; Unix only
steps = [
  { wait_text = "alpha beta" },
  { mouse = { action = "drag", at = [0, 0], to = [4, 0] } },
  { run = "split-window -h -t o" },         # the session is `o`
  { spawn = "display-menu -T t aaa a ''" }, # for commands that wait
  { keys = "\u0002%" },
  { sleep = 0.5 },
]
checks = [
  { command = "show-buffer" },              # must match tmux
  { format = "#{pane_in_mode}", expect = "0" }, # and this value
  { screen_contains = "alpha", expect = true },
  { screen_find = "┌", expect = "3,8" },     # col,row or none
]
```

Mouse actions: `click`, `double-click`, `triple-click`, `down`, `up`, `drag`
(`at` to `to`), `wheel-up`, `wheel-down`; `button` 1, 2 or 3. `at` is
`[col, row]` or `{ text = "first", row = -1, dx = 0 }` (where the text shows;
row -1 is the last).

## Known divergences

`corpus/known_divergences.toml` holds cases tmxr does not match yet or on
purpose. They run and are reported, never failing; move one to its corpus file
once it matches.
