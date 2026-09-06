//! `demo-harness` must be exercised by two separate `cargo test`
//! invocations, not one: `cargo test -p omnifrons-shell --features
//! demo-harness` (or `--all-features`) exercises the hidden argv branch
//! itself, while `cargo test -p omnifrons-shell --test feature_gate`
//! (deliberately with *neither* flag) is the only invocation that observes
//! the feature's off state -- `tests/feature_gate.rs` is entirely
//! `#![cfg(not(feature = "demo-harness"))]`, so a CI job that always builds
//! with every feature enabled would report zero tests for that file,
//! forever, and could never catch a regression that made the feature
//! default-on. `.github/workflows/ci.yml` and `.github/workflows/tauri-build.yml`
//! each carry a dedicated step running exactly `cargo test -p omnifrons-shell
//! --test feature_gate`, after their `--all-features` step, for this reason.
//!
//! Unlike `tests/feature_gate.rs`, this file carries no `cfg` gate on
//! itself: it compiles and its assertions run identically in both
//! invocations, so it stays counted in `cargo test`'s summary either way --
//! a purely `cfg(not(...))`-gated file could otherwise silently vanish from
//! a misconfigured run (one that always passes `--all-features`) with
//! nothing in the test output to say so.

use std::fs;

#[test]
fn the_demo_harness_feature_is_named_correctly_in_the_manifest() {
    let path = format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
    let manifest =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));

    assert!(
        manifest.contains("demo-harness = []"),
        "the omnifrons-shell manifest must declare a `demo-harness` feature exactly named \
         that; if it is ever renamed, update tests/feature_gate.rs, this file, and the \
         dedicated CI step in .github/workflows/ci.yml and tauri-build.yml \
         (`cargo test -p omnifrons-shell --test feature_gate`) together"
    );
}
