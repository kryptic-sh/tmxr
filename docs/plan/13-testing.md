# 13 — Testing

Three layers, all run on Linux, macOS and Windows in CI. Every test is shown to
fail against a broken implementation before it is trusted (break, see red,
restore).

## 1. Unit tests (pure functions, most of the coverage)

| Area               | Crate          | What                                                                               |
| ------------------ | -------------- | ---------------------------------------------------------------------------------- |
| Tokenizer / parser | `tmxr-command` | tmux quoting, `;` / `\;`, `{}` blocks, flags, prefix-matched names, error messages |
| Key names          | `tmxr-command` | `C-\`, `M-h`, `'`, `"`, `;`, `BSpace`, round-trip parse ↔ print                    |
| Formats            | `tmxr-command` | every supported `#{…}` form against a fake context; `#[style]` runs                |
| Defaults parity    | `tmxr-config`  | the embedded defaults produce **exactly** the bind table in 06 (table-driven)      |
| Key encoding       | `tmxr-term`    | table of `KeyEvent` × pane modes → expected bytes (xterm ctlseqs)                  |
| Mouse encoding     | `tmxr-term`    | X10 / SGR / UTF-8 encodings with pane-relative offsets                             |
| Layout ops         | `tmxr-server`  | split / kill / resize / zoom / swap / presets, `window_rects` sums to the window   |
| Navigation         | `tmxr-server`  | `select-pane` direction + wrap, navigator pattern on real process names            |
| Status line        | `tmxr-server`  | rendered status row (text + colours per cell) for representative states            |
| Pane widget        | `tmxr-server`  | vt100 screen → ratatui buffer cells (wide chars, colours, attributes)              |
| Protocol           | `tmxr-proto`   | round-trip every message; **golden bytes** so wire changes need a version bump     |
| Picker source      | `tmxr-server`  | MRU order, fuzzy filtering, selection → action                                     |
| Resurrect          | `tmxr-server`  | save → JSON → restore round-trip of a model; skip-existing; allowlist              |

## 2. In-process integration tests (`tmxr-server/tests`)

Start a server on a temp socket inside the test process, connect a test client
over the real transport, and drive it with `ClientMsg::Input`. The client side
feeds received `Output` bytes into its own `vt100::Parser`, so assertions are on
**what the user would see**: "the status line reads `0  zsh`", "pane 1 is to the
right of pane 0", "the border of the active pane is blue".

Panes run a deterministic fixture program instead of a real shell — a small test
binary (`tmxr-testbin`, built as a `[[bin]]` in a dev-only crate) that can echo
input, print markers, report its cwd, enter alt-screen, and exit on command.
That works the same on all three platforms, unlike `sh`/`cmd`.

Covers: attach/detach/reattach keeps panes alive; splits and focus; prefix
binds; `C-h` navigator in both modes (fixture named `hjkl` vs not); session
picker switch; copy-mode copy → buffer; resize; `kill-server` cleanup.

## 3. End-to-end PTY tests (`apps/tmxr/tests`)

The hjkl/hrdr pattern (`hjkl/apps/hjkl/tests/pty_harness`): spawn the real
`tmxr` binary inside a `portable-pty`, send keystrokes as bytes, parse its
output with `vt100`, poll the screen until an expected state appears (with a
timeout), and dump the screen on failure. A few high-value flows only — full
attach with the default config, split + navigate, detach + reattach, server
auto-spawn — because these are the slowest and most timing-sensitive tests.

Configured in `.config/nextest.toml` like hjkl: a serial `pty-e2e` test group,
generous slow-timeout, retries for PTY tests only.

## Not covered by automated tests (stated as a gap)

- Rendering on real terminal emulators (alacritty, Windows Terminal, iTerm2,
  kitty) — manual checklist per release in `docs/`.
- hjkl's side of the navigator handoff is tested in the hjkl repo.
