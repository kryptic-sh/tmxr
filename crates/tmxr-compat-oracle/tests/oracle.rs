//! The corpus, each file through the oracle. Every case must match tmux (and
//! its expected values), or skip where it cannot run. With
//! `ORACLE_REQUIRE_TMUX` set, as CI sets it, a missing tmux fails instead of
//! skipping.

use std::path::PathBuf;

use tmxr_compat_oracle::{Status, load_corpus, run_corpus, tmux_program, tmxr_program};

fn run(file: &str) -> Vec<tmxr_compat_oracle::CaseResult> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("corpus")
        .join(file);
    let corpus = load_corpus(&path).unwrap();
    let tmux = tmux_program();
    assert!(
        tmux.is_some() || std::env::var_os("ORACLE_REQUIRE_TMUX").is_none(),
        "ORACLE_REQUIRE_TMUX is set but TMUX_BIN does not name a tmux that runs"
    );
    let tmxr = tmxr_program();
    assert!(
        tmxr.is_file(),
        "no tmxr at {}: build it (cargo build -p tmxr-cli) or set TMXR_BIN",
        tmxr.display()
    );
    let results = run_corpus(&corpus, tmux.as_deref(), &tmxr);
    for r in &results {
        eprintln!("{file}: {}: {:?}", r.name, r.status);
    }
    results
}

fn assert_passes(file: &str) {
    let results = run(file);
    let failures: Vec<_> = results
        .iter()
        .filter(|r| !matches!(r.status, Status::Pass | Status::Skipped(_)))
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn layout_matches_tmux() {
    assert_passes("layout.toml");
}

#[test]
fn formats_match_tmux() {
    assert_passes("formats.toml");
}

#[test]
fn list_commands_match_tmux() {
    assert_passes("lists.toml");
}

#[test]
fn popups_and_menus_match_tmux() {
    assert_passes("popups.toml");
}

#[test]
fn the_mouse_matches_tmux() {
    assert_passes("mouse.toml");
}

#[test]
fn pasting_matches_tmux() {
    assert_passes("paste.toml");
}

/// Differences kept on purpose or not fixed yet: reported, never failing.
#[test]
fn known_divergences_report() {
    for r in run("known_divergences.toml") {
        if r.status == Status::Pass {
            eprintln!(
                "known divergence {} now matches tmux: move it to a corpus file",
                r.name
            );
        }
    }
}
