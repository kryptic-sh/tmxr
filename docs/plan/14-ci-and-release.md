# 14 — CI and release

## Template

hrdr's pipeline is the org's newest and most complete
(`hrdr/.github/workflows/ci.yml`, which itself "mirrors gpur's release
process"). tmxr copies its shape. The sibling repos differ only in which
packaging channels they ship; hrdr's set is the superset for a CLI app.

## Phase 1 — now (scaffold)

Checks only, no deploy steps. `.github/workflows/ci.yml`, triggered on
`pull_request`, `push` to `main` and `v*` tags, and `workflow_dispatch`:

| Job       | Runs on                  | What                                                                                                                    |
| --------- | ------------------------ | ----------------------------------------------------------------------------------------------------------------------- |
| `fmt`     | ubuntu                   | `cargo fmt --all --check`                                                                                               |
| `clippy`  | ubuntu / macos / windows | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`                                         |
| `test`    | ubuntu / macos / windows | `cargo nextest run --workspace --all-features --locked --no-fail-fast` + doctests                                       |
| `smoke`   | ubuntu / macos / windows | build, `tmxr --version`, `tmxr --help`; later, the e2e "client attaches in a real pty" test against the release profile |
| `machete` | ubuntu                   | unused dependencies                                                                                                     |
| `deny`    | ubuntu                   | `cargo deny --all-features check` (licenses, bans, sources, advisories)                                                 |
| `audit`   | ubuntu                   | `cargo audit`                                                                                                           |

Conventions carried over from hrdr: `RUSTFLAGS=-D warnings`,
`CARGO_TERM_COLOR=always`, `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24`, concurrency
group cancelling superseded non-tag runs, `fail-fast: false` matrices,
`actions/checkout@v7`, `actions-rust-lang/setup-rust-toolchain` with
`toolchain: stable` + `rustup update`, `Swatinem/rust-cache@v2`,
`taiki-e/install-action` for nextest / machete / deny / audit.

Also: `.github/workflows/cron.yml` (Monday 06:00 UTC `cargo deny` — the sibling
repos' schedule), `.github/dependabot.yml` (the standard cargo + actions form),
`CODEOWNERS`, the PR template and issue templates from krypt/hjkl,
`.config/nextest.toml` with the `pty-e2e` serial group.

## Phase 2 — first release (M8)

Add hrdr's release jobs verbatim in shape, renamed for tmxr:

- `build` — 7 targets: `x86_64`/`aarch64` `unknown-linux-gnu` (zigbuild, glibc
  2.28), `x86_64`/`aarch64` `unknown-linux-musl`, `x86_64-pc-windows-msvc`,
  `aarch64-apple-darwin` (11.0), `x86_64-apple-darwin` (10.15); tag ↔
  `Cargo.toml` version check; `.tar.gz`/`.zip` + `.sha256`; `.deb` / `.rpm` for
  gnu targets.
- `publish-github-release`, `publish-crates` (topological `ORDER` with the drift
  check), `aur-bin`, `brew-tap`, `scoop-bucket`, `alpine`, and the
  `tag-release-status` aggregate that fails the tag run if any job did not
  succeed.
- `pkg/` templates: `aur/PKGBUILD-bin.in`, `homebrew/tmxr.rb.in`,
  `scoop/tmxr.json.in`, `alpine/APKBUILD.in`.
- The binary gains `--completions <shell>` and `--man` (`clap_complete`,
  `clap_complete_nushell`, `clap_mangen`), which every package template uses.

Before the first tag:

- tmxr must be added to the allowlists of the org secrets `AUR_SSH_KEY`,
  `BREW_SSH_KEY`, `SCOOP_SSH_KEY` (they are scoped to selected repos) and have
  `CARGO_REGISTRY_TOKEN`.
- Check the `tmxr` / `tmxr-*` names are free on crates.io and the AUR.
- Site entry on kryptic-sh.github.io (`content/tmxr.html`, `build.py`
  `SIBLINGS`, `pages.yml` sanity loop, the index card, `sitemap.xml`).

## Phase 3 — later

- `leak-guard`: hrdr runs the suite with `$HOME`/XDG pointed at an empty
  sentinel dir and fails if anything lands there. tmxr tests create sockets,
  configs and resurrect files, so this guard is worth adopting once the
  integration tests exist; until then every test sets `TMXR_TMPDIR` and the XDG
  roots to a temp dir explicitly.
- Weekly cron extras (fuzzing the command parser and the frame codec).
