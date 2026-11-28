//! Every committed seed and regression input through its target, on
//! stable, so a crash a fuzzer found stays fixed without a fuzzer.

use std::path::Path;

#[test]
fn every_seed_and_regression_input_passes_its_target() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut ran = 0;
    for (name, target) in tmxr_fuzz::TARGETS {
        for dir in ["seeds", "regressions"] {
            let Ok(entries) = std::fs::read_dir(root.join(dir).join(name)) else {
                continue;
            };
            for entry in entries {
                let path = entry.unwrap().path();
                let data = std::fs::read(&path).unwrap();
                eprintln!("{}", path.display());
                target(&data);
                ran += 1;
            }
        }
    }
    // Every target has seeds, so a run that found none read the wrong place.
    assert!(ran >= tmxr_fuzz::TARGETS.len(), "ran {ran} inputs");
}

#[test]
fn every_target_has_a_binary_and_seeds() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (name, _) in tmxr_fuzz::TARGETS {
        assert!(
            root.join("fuzz_targets")
                .join(format!("{name}.rs"))
                .is_file(),
            "{name}"
        );
        let seeds = std::fs::read_dir(root.join("seeds").join(name)).map_or(0, Iterator::count);
        assert!(seeds > 0, "{name} has no seeds");
    }
}
