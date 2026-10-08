# 15 — Milestones

Build order. Each milestone ends green on all three platforms
(`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`) with its tests in place, and is committed before the
next starts. A milestone's plan sections are trimmed to what is still open once
it lands.

| #   | Milestone                       | Delivers                                                                                                                                                                   | Exit check                                                                                     |
| --- | ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| M0  | Scaffold                        | Workspace with every crate stubbed honestly, CI checks matrix, repo meta, this plan. Pushed as the repo's initial commit.                                                  | CI green on ubuntu/macos/windows                                                               |
| M1  | Protocol + server + single pane | `tmxr-proto` framing/handshake; server auto-spawn + socket security; one session with one pane running a shell; attach, detach (`prefix d`), reattach, `ls`, `kill-server` | integration test: attach → type → see echo → detach → reattach → output still there            |
| M2  | Windows, panes, layout, status  | splits, `select-pane`, kill/exit reflow, resize, zoom, windows + indices + renumber, borders, Tokyo Night status line in the catppuccin layout                             | status-line snapshot test; layout op tests; split/navigate integration test                    |
| M3  | Commands, keys, config          | `tmxr-command` parser + formats; key tables, prefix, repeat; embedded defaults = tmux.conf port; `tmxr <command>` CLI; command prompt; `source-file`                       | defaults-parity test; parser/format tests; every default bind exercised by an integration test |
| M4  | Navigator                       | `navigate-pane` + process inspection on all platforms; `TMXR`/`TMXR_PANE`; hjkl `$TMXR` fall-through (hjkl repo change)                                                    | both navigator branches tested with a fixture named `hjkl` and one not                         |
| M5  | Session picker                  | `prefix s` / `prefix w` pickers on hjkl-picker; switch-client; MRU order                                                                                                   | picker integration test: open, filter, `j`/`k`, Enter switches                                 |
| M6  | Copy mode + clipboard + mouse   | vi copy mode with the config's `v` / `C-v` / `y`; paste buffers; OSC 52 + local clipboard; mouse select/resize/wheel                                                       | copy → buffer → paste integration test; encoder tests                                          |
| M7  | Resurrect (stretch)             | save/restore, auto-save, restore-on-start                                                                                                                                  | round-trip test; restore-on-start integration test                                             |
| M8  | Release                         | completions/man, release jobs + `pkg/`, README install docs, site entry                                                                                                    | first tag's CI run fully green and artifacts landed in every channel                           |

M1–M6 is the MVP. M7 is the stretch. M8 is cut when the user decides the MVP is
good enough to be a daily driver.
