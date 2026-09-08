//! Runs `omnifrons-app`'s reusable publication-port contracts against the
//! in-memory fakes -- the real adapters (`omnifrons-adapters`) run them
//! again against themselves. Green on arrival: the fakes were written
//! alongside the contracts; recorded as a pin, not a red.
//!
//! Only compiled with `--features contract-tests`.

#![cfg(feature = "contract-tests")]

use omnifrons_app::contract::publication::{
    InMemoryBlobStore, InMemoryCatalogStore, InMemoryJournal, blob_store_contract,
    catalog_store_contract, publication_journal_contract,
};

#[test]
fn in_memory_catalog_store_satisfies_the_contract() {
    catalog_store_contract(InMemoryCatalogStore::new);
}

#[test]
fn in_memory_journal_satisfies_the_contract() {
    publication_journal_contract(InMemoryJournal::new);
}

#[test]
fn in_memory_blob_store_satisfies_the_contract() {
    blob_store_contract(|| InMemoryBlobStore::new("fake"));
}
