//! V1: every mandatory schema field present and non-empty, per record kind
//! (design.md validator contract). A scenario row's `result` and
//! `observed_state` are two distinct mandatory fields -- neither substitutes
//! for the other.

use crate::record::{Record, Violation};

/// Mandatory fields for a `kind: baseline` record: the store-level identity
/// pair plus every VP-001-R1 field (verification-evidence-store spec,
/// Baseline-Before-Scenario Admission).
const BASELINE_MANDATORY: &[&str] = &[
    "record_id",
    "kind",
    "os_build",
    "architecture",
    "webview_runtime",
    "packaging_substrate",
    "assistive_technology",
    "test_date",
];

/// Mandatory fields for a `kind: scenario` record. `result` and
/// `observed_state` are listed separately -- V1 rejects either one missing
/// on its own, never accepting one as a stand-in for the other.
const SCENARIO_MANDATORY: &[&str] = &[
    "record_id",
    "kind",
    "scenario_id",
    "baseline_id",
    "result",
    "observed_state",
    "build_channel",
    "variant_scan",
    "procedure_ref",
];

pub(super) fn check(records: &[Record], violations: &mut Vec<Violation>) {
    for record in records {
        let mandatory: &[&str] = match record.kind() {
            Some("baseline") => BASELINE_MANDATORY,
            Some("scenario") => SCENARIO_MANDATORY,
            // An unrecognized or absent `kind` cannot select a per-kind
            // mandatory set; `kind` itself is still checked below.
            _ => &["kind"],
        };
        for field in mandatory {
            let present = record.get(field).is_some_and(|v| !v.is_empty());
            if !present {
                violations.push(Violation::new(
                    record.record_id(),
                    "field-missing",
                    format!("mandatory field `{field}` is missing or empty"),
                ));
            }
        }
    }
}
