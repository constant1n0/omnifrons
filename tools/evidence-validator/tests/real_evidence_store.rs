//! Integration coverage over the REAL `docs/evidence/VP-001/{baselines,
//! records}.md` files -- not the synthetic fixtures under `tests/fixtures/`
//! (`fixtures_parse_and_validate.rs`). design.md D2 requires that a
//! malformed or incomplete real record fail the repository's own
//! `cargo test --workspace` run with no workflow change; this test is that
//! enforcement.

use std::fs;

use evidence_validator::record::parse;
use evidence_validator::validate::validate;

fn read_real(name: &str) -> String {
    let path = format!(
        "{}/../../docs/evidence/VP-001/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"))
}

fn real_store_text() -> String {
    format!("{}\n{}", read_real("baselines.md"), read_real("records.md"))
}

#[test]
fn real_evidence_store_parses_and_validates_clean() {
    let records = parse(&real_store_text())
        .expect("the real evidence store must parse as well-formed records");
    let violations = validate(&records);
    assert!(
        violations.is_empty(),
        "the real VP-001 evidence store must validate clean, got {violations:?}"
    );
}

#[test]
fn real_evidence_store_carries_a_baseline_and_its_vp_s6_row() {
    let records = parse(&real_store_text())
        .expect("the real evidence store must parse as well-formed records");

    assert!(
        records.iter().any(|r| r.kind() == Some("baseline")),
        "expected at least one filed baseline record"
    );
    assert!(
        records
            .iter()
            .any(|r| r.kind() == Some("scenario") && r.get("scenario_id") == Some("VP-S6")),
        "expected at least one filed VP-S6 scenario row"
    );
}
