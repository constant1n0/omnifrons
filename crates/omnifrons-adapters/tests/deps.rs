//! Compile-time-adjacent honesty check: `omnifrons-adapters` must not
//! depend on Tokio or Tauri (docs/repository-layout.md § Crate map:
//! `omnifrons-adapters` sits above `omnifrons-domain` and `omnifrons-app`,
//! plus the spike-slice-2 additions `sha2`, `serde`, `serde_json`, and,
//! unix only, `nix`), and must actually declare exactly the dependencies
//! it needs -- no more, no less, in either its always-present
//! `[dependencies]` table or its unix-only
//! `[target.'cfg(unix)'.dependencies]` table.
//!
//! This reads the crate's own `Cargo.toml` rather than `cargo tree`, so it
//! fails the moment someone adds a disallowed dependency, without needing
//! a network fetch or a lockfile resolve to run.

use std::collections::BTreeSet;
use std::fs;

fn manifest() -> String {
    fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("omnifrons-adapters/Cargo.toml must be readable")
}

/// Extract the set of dependency keys declared under the manifest's own
/// `[dependencies]` table, ignoring every other table (`[package]`,
/// `[lints]`, `[dev-dependencies]`, ...). Section-aware line parsing is
/// sufficient here without a `toml` dependency: every key in this crate's
/// `[dependencies]` table is a bare `name = ...` line, never an inline
/// table spanning multiple lines.
fn dependency_keys(manifest: &str) -> BTreeSet<String> {
    section_dependency_keys(manifest, "[dependencies]")
}

/// Extract the set of dependency keys declared under `section` (e.g.
/// `[target.'cfg(unix)'.dependencies]`), ignoring every other table.
fn section_dependency_keys(manifest: &str, section: &str) -> BTreeSet<String> {
    let mut in_section = false;
    let mut keys = BTreeSet::new();
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == section;
            continue;
        }
        if !in_section || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = trimmed.split_once('=') {
            keys.insert(key.trim().to_string());
        }
    }
    keys
}

#[test]
fn does_not_depend_on_tokio_or_tauri() {
    let manifest = manifest();
    assert!(
        !manifest.to_lowercase().contains("tokio"),
        "omnifrons-adapters must stay framework-independent; found a tokio reference:\n{manifest}"
    );
    assert!(
        !manifest.to_lowercase().contains("tauri"),
        "omnifrons-adapters must stay framework-independent; found a tauri reference:\n{manifest}"
    );
}

#[test]
fn dependencies_table_is_exactly_domain_app_sha2_serde_and_serde_json() {
    let manifest = manifest();
    let keys = dependency_keys(&manifest);
    let expected: BTreeSet<String> = [
        "omnifrons-domain",
        "omnifrons-app",
        "sha2",
        "serde",
        "serde_json",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(
        keys, expected,
        "omnifrons-adapters' [dependencies] table must be exactly \
         {{omnifrons-domain, omnifrons-app, sha2, serde, serde_json}} -- sha2 for content \
         hashing (FsExecutableProber) and serde/serde_json for the JSONL approval-store \
         record shape (JsonlApprovalStore), found {keys:?} in:\n{manifest}"
    );
}

/// `nix` is unix-only, so it must live under
/// `[target.'cfg(unix)'.dependencies]`, never the always-present
/// `[dependencies]` table above (which this test's sibling asserts stays
/// free of it) -- `fs_prober`'s `O_NOFOLLOW` open and Linux memfd-sealing
/// path are the reason it is here at all (`src/fs_prober.rs`'s own doc
/// comment).
#[test]
fn unix_only_dependencies_table_is_exactly_nix() {
    let manifest = manifest();
    let keys = section_dependency_keys(&manifest, "[target.'cfg(unix)'.dependencies]");
    let expected: BTreeSet<String> = ["nix"].into_iter().map(String::from).collect();
    assert_eq!(
        keys, expected,
        "omnifrons-adapters' [target.'cfg(unix)'.dependencies] table must be exactly {{nix}}, \
         found {keys:?} in:\n{manifest}"
    );
}
