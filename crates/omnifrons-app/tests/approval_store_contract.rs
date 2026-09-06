//! Runs `omnifrons-app`'s reusable `ApprovalStore` contract against the
//! in-memory fake -- the real adapter (`JsonlApprovalStore`,
//! `omnifrons-adapters`) runs it again against itself.
//!
//! Only compiled with `--features contract-tests` (see `omnifrons-app`'s
//! `contract-tests` feature).

#![cfg(feature = "contract-tests")]

use omnifrons_app::contract::approval_store::{InMemoryApprovalStore, approval_store_contract};

#[test]
fn in_memory_store_satisfies_approval_store_contract() {
    approval_store_contract(InMemoryApprovalStore::new);
}
