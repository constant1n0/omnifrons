//! Integration coverage (design.md Testing Strategy): parses and validates a
//! realistic multi-record fixture set the same way `cargo test --workspace`
//! will exercise the real `docs/evidence/VP-001/{baselines,records}.md` once
//! Slice 4 files them. Everything under `tests/fixtures/` is synthetic,
//! never real evidence.

use std::fs;

use evidence_validator::record::parse;
use evidence_validator::validate::validate;

fn read_fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"))
}

#[test]
fn synthetic_baseline_and_scenario_fixtures_parse_and_validate_clean() {
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

    let violations = validate(&records);
    assert!(
        violations.is_empty(),
        "the fixture set must validate clean, got {violations:?}"
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
