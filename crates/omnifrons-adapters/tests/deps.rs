//! Compile-time-adjacent honesty check: `omnifrons-adapters` must not
//! depend on Tokio or Tauri (docs/repository-layout.md § Crate map:
//! `omnifrons-adapters` sits above `omnifrons-domain` and `omnifrons-app`
//! only), and must actually declare the two dependencies it does need.
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
    let mut in_dependencies = false;
    let mut keys = BTreeSet::new();
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dependencies = trimmed == "[dependencies]";
            continue;
        }
        if !in_dependencies || trimmed.is_empty() || trimmed.starts_with('#') {
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
fn dependencies_table_is_exactly_domain_and_app() {
    let manifest = manifest();
    let keys = dependency_keys(&manifest);
    let expected: BTreeSet<String> = ["omnifrons-domain", "omnifrons-app"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(
        keys, expected,
        "omnifrons-adapters' [dependencies] table must be exactly \
         {{omnifrons-domain, omnifrons-app}}, found {keys:?} in:\n{manifest}"
    );
}
