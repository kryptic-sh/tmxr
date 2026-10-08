# tmxr plan — index

tmxr is a Rust terminal multiplexer: a tmux alternative built on the kryptic-sh
house crates, client/server like tmux, on Linux, macOS and Windows, shipping the
owner's tmux config as its defaults.

Section numbers are document identity. Build order is
[15-milestones.md](15-milestones.md).

| #   | Section                                                        | Covers                                                                        |
| --- | -------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| 00  | [00-index.md](00-index.md)                                     | This list                                                                     |
| 01  | [01-overview.md](01-overview.md)                               | Goals, baseline, MVP scope, non-goals, decisions, naming                      |
| 02  | [02-architecture.md](02-architecture.md)                       | Processes, workspace crates, server/client internals, dependencies            |
| 03  | [03-protocol.md](03-protocol.md)                               | Transport, access control, framing, handshake, messages, versioning           |
| 04  | [04-panes-and-terminal.md](04-panes-and-terminal.md)           | PTYs, vt100 emulation, key/mouse encoding, process inspection, pane lifecycle |
| 05  | [05-sessions-windows-layout.md](05-sessions-windows-layout.md) | Model, ids/indices, targets, hjkl-layout mapping, sync panes, multi-client    |
| 06  | [06-keys-and-bindings.md](06-keys-and-bindings.md)             | Key tables, the full default bind table, vim/hjkl navigation                  |
| 07  | [07-rendering-status-theme.md](07-rendering-status-theme.md)   | Compositor, Tokyo Night theme, status line layout                             |
| 08  | [08-copy-mode.md](08-copy-mode.md)                             | vi copy mode, engine choice, clipboard                                        |
| 09  | [09-session-picker.md](09-session-picker.md)                   | Fuzzy session/window picker on hjkl-picker                                    |
| 10  | [10-config-and-commands.md](10-config-and-commands.md)         | TOML config, tmux command language, formats, CLI                              |
| 11  | [11-resurrect.md](11-resurrect.md)                             | Save/restore sessions, auto-save, restore on start                            |
| 12  | [12-platforms.md](12-platforms.md)                             | Per-platform behaviour table                                                  |
| 13  | [13-testing.md](13-testing.md)                                 | Unit, in-process integration, PTY end-to-end                                  |
| 14  | [14-ci-and-release.md](14-ci-and-release.md)                   | CI checks now, release pipeline later                                         |
| 15  | [15-milestones.md](15-milestones.md)                           | Build order and exit checks                                                   |
| 16  | [16-open-questions.md](16-open-questions.md)                   | Provisional decisions awaiting the owner's call                               |
