//! Runs `omnifrons-app`'s reusable guidance-port contracts (spike slice
//! 5c) against the in-memory fakes -- the real adapters
//! (`omnifrons-adapters`) run them again against themselves. Green on
//! arrival: the fakes were written alongside the contracts; recorded as a
//! pin, not a red.
//!
//! Only compiled with `--features contract-tests`.

#![cfg(feature = "contract-tests")]

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::contract::guidance::{
    InMemoryProjectTextFile, InMemorySnapshotStore, managed_file_contract, snapshot_store_contract,
};

#[test]
fn in_memory_snapshot_store_satisfies_the_contract() {
    snapshot_store_contract(InMemorySnapshotStore::new);
}

#[test]
fn in_memory_project_text_file_satisfies_the_contract() {
    let root = WorkspaceRoot::new(std::env::temp_dir()).expect("the temp dir is a workspace");
    managed_file_contract(&InMemoryProjectTextFile::new(), &root);
}
