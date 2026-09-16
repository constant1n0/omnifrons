//! `validate(records) -> Vec<Violation>`: the closed V1-V8 admission
//! contract from design.md. Every check runs over the whole parsed set so
//! cross-record rules (a scenario's baseline, duplicate ids, reused
//! artifacts) see the complete picture. Each check lives in its own module,
//! named after the design.md validator-contract row(s) it implements; they
//! are wired in incrementally across this delivery split's commits.

mod v1_mandatory_fields;
mod v2_result_vocabulary;
mod v3_baseline_before_scenario;
mod v4_v5_v6_identifiers_and_provenance;

use crate::record::{Record, Violation};

/// Runs every check wired so far and returns every violation found (never
/// stops at the first one, so a caller sees the complete rejection set).
#[must_use]
pub fn validate(records: &[Record]) -> Vec<Violation> {
    let mut violations = Vec::new();
    v1_mandatory_fields::check(records, &mut violations);
    v2_result_vocabulary::check(records, &mut violations);
    v3_baseline_before_scenario::check(records, &mut violations);
    v4_v5_v6_identifiers_and_provenance::check(records, &mut violations);
    violations
}
