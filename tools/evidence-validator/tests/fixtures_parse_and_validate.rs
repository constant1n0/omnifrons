//! Integration coverage (design.md Testing Strategy): parses a realistic
//! multi-record fixture set the same way `cargo test --workspace` will
//! exercise the real `docs/evidence/VP-001/{baselines,records}.md` once
//! Slice 4 files them. Everything under `tests/fixtures/` is synthetic,
//! never real evidence.
//!
//! The full-validation assertion (`validate` must report zero violations
//! against this fixture set) is added once every V1-V8 check lands, later in
//! this delivery split; this file exercises `parse` alone until then.

use std::fs;

use evidence_validator::record::parse;

fn read_fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"))
}

#[test]
fn synthetic_baseline_and_scenario_fixtures_parse_into_three_records() {
    let text = format!(
        "{}\n{}",
        read_fixture("synthetic-baselines.md"),
        read_fixture("synthetic-records.md")
    );
    let records = parse(&text).expect("fixture files must parse as well-formed records");
    assert_eq!(
        records.len(),
        3,
        "one baseline plus an original and a correcting scenario row"
    );
}

#[test]
fn correction_row_resolves_via_corrects_and_the_original_stays_unchanged() {
    let text = format!(
        "{}\n{}",
        read_fixture("synthetic-baselines.md"),
        read_fixture("synthetic-records.md")
    );
    let records = parse(&text).expect("fixture files must parse as well-formed records");

    let original = records
        .iter()
        .find(|r| r.record_id() == Some("VP-001-SYNTHETIC-VP-S6-01"))
        .expect("the original row must still be present, never rewritten");
    assert_eq!(original.get("result"), Some("uncertain"));

    let correction = records
        .iter()
        .find(|r| r.get("corrects") == Some("VP-001-SYNTHETIC-VP-S6-01"))
        .expect("a correction row referencing the original must be present");
    assert_eq!(correction.get("result"), Some("fail"));
}
