//! V3: a scenario row's `baseline_id` must resolve to a baseline record
//! carrying every VP-001-R1 field. An unresolved id, or a resolved baseline
//! missing one of those fields, is rejected as `baseline-unpinned`.

use crate::record::{Record, Violation};

/// The VP-001-R1 fields a baseline must carry in full before any scenario
/// row may reference it (Baseline-Before-Scenario Admission requirement).
const VP001_R1_FIELDS: &[&str] = &[
    "os_build",
    "architecture",
    "webview_runtime",
    "packaging_substrate",
    "assistive_technology",
    "test_date",
];

pub(super) fn check(records: &[Record], violations: &mut Vec<Violation>) {
    for record in records {
        if record.kind() != Some("scenario") {
            continue;
        }
        let Some(baseline_id) = record.get("baseline_id").filter(|id| !id.is_empty()) else {
            continue; // V1 already reports a missing baseline_id.
        };

        let baseline = records
            .iter()
            .find(|r| r.kind() == Some("baseline") && r.record_id() == Some(baseline_id));

        match baseline {
            None => violations.push(Violation::new(
                record.record_id(),
                "baseline-unpinned",
                format!("baseline_id `{baseline_id}` does not resolve to a baseline record"),
            )),
            Some(baseline) => {
                let missing: Vec<&str> = VP001_R1_FIELDS
                    .iter()
                    .copied()
                    .filter(|field| baseline.get(field).is_none_or(str::is_empty))
                    .collect();
                if !missing.is_empty() {
                    violations.push(Violation::new(
                        record.record_id(),
                        "baseline-unpinned",
                        format!(
                            "baseline `{baseline_id}` is missing VP-001-R1 field(s): {missing:?}"
                        ),
                    ));
                }
            }
        }
    }
}
