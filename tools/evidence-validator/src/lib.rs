//! Non-product evidence-record tooling for VP-001 (design.md D2): parses the
//! append-only `docs/evidence/VP-001/{baselines,records}.md` record grammar.
//! Validation (`validate`) and outcome derivation (`derive`) land in later
//! commits of this delivery split. No product crate depends on this crate
//! (`cargo test --workspace` runs it, but it never links into the shipped
//! binary).

pub mod record;
pub mod validate;
