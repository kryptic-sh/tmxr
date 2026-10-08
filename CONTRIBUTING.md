# Contributing

Thanks for taking a look. The project is in early development — the plan in
[docs/plan/](docs/plan/00-index.md) and its milestones
([15-milestones.md](docs/plan/15-milestones.md)) show where help fits.

## Commit style

This repo follows
**[Conventional Commits](https://www.conventionalcommits.org/)**.

```
type(scope): short summary

Longer body explaining the WHY, not the WHAT.

Closes #123
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`, `perf`,
`ci`, `build`. Scope is optional; use the crate without its prefix
(`feat(server): …`, `fix(term): …`).

Breaking changes: append `!` after type (`feat(proto)!: ...`) and explain in the
body.

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same on Linux, macOS and Windows, plus `cargo nextest`,
`cargo deny`, `cargo audit` and `cargo machete`. Platform-gated code
(`#[cfg(windows)]`, `#[cfg(unix)]`) is only compiled on its platform, so a green
local run does not cover the others — CI does.

MSRV is **Rust 1.95** (`rust-version` in the workspace `Cargo.toml`). The
workspace is `edition = "2024"`, `resolver = "3"`.

## Repo layout

```
apps/
└── tmxr/           # bin: `tmxr`
crates/
├── tmxr-proto/     # lib: wire protocol
├── tmxr-command/   # lib: tmux command language
├── tmxr-config/    # lib: TOML config + default binds
├── tmxr-term/      # lib: pane terminal (pty, vt100, encoding)
├── tmxr-server/    # lib: the server
└── tmxr-client/    # lib: the attach client
```

The binary stays thin; logic belongs in a library crate where it can be tested
without a terminal.

## Issues + PRs

- Pick an open issue, comment that you're starting.
- Branch off `main`, name like `feat/session-picker` or `fix/split-cwd`.
- Squash on merge unless commits are individually meaningful.

## License

By contributing, you agree your changes are MIT-licensed.
