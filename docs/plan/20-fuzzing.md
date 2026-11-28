# 20 — Fuzzing

Asked for by the owner (2026-11-28): proper fuzzing for the project, and the
pane holders ([19-pane-holders.md](19-pane-holders.md)) fully covered by tests
and fuzzing as they are built. Decided with the owner: cargo-fuzz (libFuzzer), a
short run of every target on each push to `main` and a long run nightly, and
vt1k and ttyr fuzzed in their own repositories as well as through tmxr.

Until now the only fuzz-like coverage was two property-test files
(`crates/tmxr-command/tests/props.rs`, `crates/tmxr-proto/tests/props.rs`):
random, not coverage-guided, and only over the command parser, format expansion
and the frame codec. They stay; fuzzing adds to them.

## Layout

- **`fuzz/`**, a crate of its own outside the workspace (its own `[workspace]`
  and `Cargo.lock`), so the normal build, `cargo test` and the stable toolchain
  are untouched by `libfuzzer-sys`.
- **`fuzz/src/lib.rs`** holds one function per target, taking `&[u8]`; each
  `fuzz/fuzz_targets/<name>.rs` only calls its function. The functions build on
  stable, so the replay test below runs without nightly.
- **`fuzz/seeds/<target>/`**: small hand-picked inputs, committed, that start
  each target somewhere useful (real frames, real layouts, real configs, real
  terminal output).
- **`fuzz/regressions/<target>/`**: every input a fuzzer ever crashed on,
  minimized, committed with the fix.
- **`fuzz/tests/replay.rs`**: runs every seed and regression through its
  target's function, on stable, in CI's test job, so a fixed crash stays fixed
  without a fuzzer.
- The grown corpus is not committed (it churns); CI keeps it in the Actions
  cache between runs.

## Invariants, not just "no panic"

A target that only checks for panics finds crashes; one that also checks an
invariant finds wrong answers. Each target states which it checks:

- **round trip:** what parses, printed and parsed again, gives the same value
  (layouts, key names, styles, commands);
- **chunking:** bytes fed in one call and the same bytes split at any points
  give the same screen, modes and replies (the emulator and its splitters,
  ttyr's reader);
- **bounds:** nothing allocates past a stated limit (frames), nothing indexes
  past the screen (the emulator, layout geometry);
- **clean refusal:** bad input is an error value, never a panic, a hang or an
  abort.

## Targets in tmxr

| Target          | Entry point                                                   | Checks                                                 |
| --------------- | ------------------------------------------------------------- | ------------------------------------------------------ |
| `proto_frame`   | `tmxr-proto` `read_msg` for both directions                   | clean refusal, bounds                                  |
| `tmxr_env`      | `tmxr-proto` `parse_tmxr_env`                                 | clean refusal                                          |
| `command_line`  | `tmxr-command` `tokenize_lines`, `table::parse_line`          | clean refusal; escaped words round trip                |
| `format_expand` | `tmxr-command` format expansion over a fake context           | clean refusal, no hang (bounded nesting)               |
| `key_names`     | `Key` and `BindKey` from strings                              | round trip through their printed form                  |
| `styles`        | `format::parse_colour`, `colour::parse_x11`, `format::styled` | clean refusal                                          |
| `layout`        | `tmxr-server` `layout::parse` (string and JSON)               | round trip through `layout::dump`; rects in the window |
| `config_toml`   | `tmxr-config` loading a config text                           | clean refusal                                          |
| `tmux_conf`     | `tmxr-config` `from_tmux_conf`                                | clean refusal                                          |
| `resurrect`     | a resurrect save file as `resurrect::Save`                    | clean refusal                                          |
| `emulator`      | `tmxr-term` `Emulator::process`                               | bounds; chunking (one call against split calls)        |
| `splitters`     | `DcsSplitter::split`, `ModeSplitter::split`                   | chunking: pieces concatenate back to the input         |
| `encode`        | `encode_key`, `encode_mouse`, `encode_paste` over any modes   | clean refusal; paste never contains its end marker     |
| `menu`          | `tmxr-server` `menu` parsing                                  | clean refusal                                          |

The pane-holder targets are listed in
[19-pane-holders.md](19-pane-holders.md#tests-and-fuzzing) and land with each
holder phase.

Not fuzzed, and why: `resurrect::apply` and anything else that starts processes
(a fuzzer would fork-bomb the runner); the server loop as a whole (state too
large for a byte-driven fuzzer to reach usefully; its parsers are fuzzed one by
one instead).

## In vt1k and ttyr

Each gets the same layout (`fuzz/`, seeds, regressions, the replay test) and the
same two CI runs:

- **vt1k:** the parser over any bytes (bounds: the cursor and every cell stay
  inside the screen at any size, including resizes mid-stream), chunking, and
  `capture` of any screen.
- **ttyr:** the input reader over any bytes (clean refusal, chunking: an escape
  sequence split across reads gives the same events), and the reports it parses
  (cursor position, device attributes, colours).

A crash in either is fixed and released there, then picked up in tmxr with a
dependency update; the tmxr targets that reach them keep the regression input
too.

## CI

- **`.github/workflows/fuzz.yml`**, one job per target in a matrix, on nightly
  with cargo-fuzz:
  - on each push to `main`: every target for a short run from the cached corpus
    and the seeds;
  - nightly (a schedule) and by hand: every target for a long run, the grown
    corpus saved back to the cache.
- A crash fails the job and uploads the crashing input as an artifact; the fix
  commits the minimized input under `fuzz/regressions/<target>/`.
- The run lengths are workflow variables, not numbers scattered through steps.
- The replay test runs in the normal CI test job, on stable.

## Phases

1. tmxr: the `fuzz/` crate, every target above, the seeds, the replay test, the
   workflow; every crash it finds fixed, with its regression input. (Done
   2026-11-28: it found the paste-marker injection and, through vt1k, vte's
   split UTF-8 bug.)
2. vt1k: the same, released if a fix is needed. (Done 2026-11-28: three targets,
   four fixes and one change to follow tmux, released as 0.1.1.)
3. ttyr: the same, released if a fix is needed. (Done 2026-11-28: four targets
   and one decoder fix, not released yet.)
4. The pane holders' targets, with each holder phase.

Each target is shown to find a planted bug before it is trusted (a panic or a
broken invariant inserted by hand, found, removed).
