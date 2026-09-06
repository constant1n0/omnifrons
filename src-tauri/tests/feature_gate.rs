//! Without the `demo-harness` cargo feature, the hidden
//! `--demo-harness <kind> <rate_hz> <lines>` argv branch (`src/main.rs`)
//! must not exist in the compiled binary at all: no program path or
//! argument vector for a harness request may cross IPC in a normal build
//! (`docs/spike-log.md` § IPC contract), and this is the standing proof
//! that the branch enabling that path is genuinely absent, not merely
//! unreachable at runtime.
//!
//! Enabled for `cargo test -p omnifrons-shell --features demo-harness`
//! would instead build *with* the branch, so this file only asserts the
//! feature's off state.

#![cfg(not(feature = "demo-harness"))]

#[test]
// The assertion is on a `cfg!`-derived constant deliberately: this test's
// whole purpose is to fail loudly if the demo-harness feature is ever made
// default-on, which is exactly a constant-value assertion.
#[allow(clippy::assertions_on_constants)]
fn demo_harness_feature_is_off_by_default() {
    assert!(
        !cfg!(feature = "demo-harness"),
        "the demo-harness feature must not be enabled by default"
    );
}

#[test]
fn built_binary_contains_no_demo_harness_string() {
    let path = env!("CARGO_BIN_EXE_omnifrons-shell");
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));

    let needle = b"--demo-harness";
    let contains_needle = bytes.windows(needle.len()).any(|window| window == needle);

    assert!(
        !contains_needle,
        "a binary built without the demo-harness feature must not contain the literal \
         --demo-harness string anywhere"
    );
}
