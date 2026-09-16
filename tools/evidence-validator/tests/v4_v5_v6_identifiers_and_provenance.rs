//! V4: `record_id` unique; `corrects` resolves to an existing `record_id`.
//! V5: rows sharing `(scenario_id, baseline_id)` carry distinct
//! `evidence_artifact` values.
//! V6: a lint against absolute-path or host-shaped values (provenance).

use evidence_validator::record::parse;
use evidence_validator::validate::validate;

fn baseline(record_id: &str) -> String {
    format!(
        "## Baseline\n\n\
         | field | value |\n\
         | --- | --- |\n\
         | `record_id` | {record_id} |\n\
         | `kind` | baseline |\n\
         | `os_build` | Ubuntu 24.04.1 LTS |\n\
         | `architecture` | x86_64 |\n\
         | `webview_runtime` | WebKitGTK 2.44.1 |\n\
         | `packaging_substrate` | appimage |\n\
         | `assistive_technology` | none exercised - VP-S19 out of scope |\n\
         | `test_date` | 2026-09-16 |\n"
    )
}

fn scenario(record_id: &str, evidence_artifact: &str, corrects: Option<&str>) -> String {
    let corrects_row = corrects.map_or(String::new(), |c| format!("| `corrects` | {c} |\n"));
    format!(
        "## Scenario\n\n\
         | field | value |\n\
         | --- | --- |\n\
         | `record_id` | {record_id} |\n\
         | `kind` | scenario |\n\
         | `scenario_id` | VP-S6 |\n\
         | `baseline_id` | VP-001-BASE-01 |\n\
         | `result` | uncertain |\n\
         | `observed_state` | orphan-risk |\n\
         | `build_channel` | packaged-ci |\n\
         | `variant_scan` | absent |\n\
         | `procedure_ref` | docs/evidence/VP-001/procedures/vp-s6-linux.sh |\n\
         | `evidence_artifact` | {evidence_artifact} |\n\
         {corrects_row}"
    )
}

#[test]
fn duplicate_record_id_is_rejected() {
    let text = format!(
        "{}\n{}\n{}",
        baseline("VP-001-BASE-01"),
        scenario("DUP-01", "artifact-a", None),
        scenario("DUP-01", "artifact-b", None)
    );
    let records = parse(&text).expect("well-formed table must parse");
    let violations = validate(&records);
    assert!(
        violations.iter().any(|v| v.token == "duplicate-record-id"),
        "a reused record_id must be rejected, got {violations:?}"
    );
}

#[test]
fn dangling_corrects_is_rejected() {
    let text = format!(
        "{}\n{}",
        baseline("VP-001-BASE-01"),
        scenario(
            "VP-001-VP-S6-02",
            "artifact-a",
            Some("VP-001-VP-S6-DOES-NOT-EXIST")
        )
    );
    let records = parse(&text).expect("well-formed table must parse");
    let violations = validate(&records);
    assert!(
        violations.iter().any(|v| v.token == "corrects-unresolved"),
        "an unresolvable corrects target must be rejected, got {violations:?}"
    );
}

#[test]
fn resolvable_corrects_is_accepted() {
    let text = format!(
        "{}\n{}\n{}",
        baseline("VP-001-BASE-01"),
        scenario("VP-001-VP-S6-01", "artifact-a", None),
        scenario("VP-001-VP-S6-02", "artifact-b", Some("VP-001-VP-S6-01"))
    );
    let records = parse(&text).expect("well-formed table must parse");
    let violations = validate(&records);
    assert!(
        !violations.iter().any(|v| v.token == "corrects-unresolved"),
        "a corrects target that exists must be accepted, got {violations:?}"
    );
}

#[test]
fn reused_evidence_artifact_for_same_scenario_and_baseline_is_rejected() {
    let text = format!(
        "{}\n{}\n{}",
        baseline("VP-001-BASE-01"),
        scenario("VP-001-VP-S6-01", "same-artifact", None),
        scenario("VP-001-VP-S6-02", "same-artifact", None)
    );
    let records = parse(&text).expect("well-formed table must parse");
    let violations = validate(&records);
    assert!(
        violations
            .iter()
            .any(|v| v.token == "evidence-artifact-reused"),
        "a reused evidence_artifact for the same (scenario_id, baseline_id) must be rejected, got {violations:?}"
    );
}

#[test]
fn distinct_evidence_artifact_per_row_is_accepted() {
    let text = format!(
        "{}\n{}\n{}",
        baseline("VP-001-BASE-01"),
        scenario("VP-001-VP-S6-01", "artifact-a", None),
        scenario("VP-001-VP-S6-02", "artifact-b", None)
    );
    let records = parse(&text).expect("well-formed table must parse");
    let violations = validate(&records);
    assert!(
        !violations
            .iter()
            .any(|v| v.token == "evidence-artifact-reused"),
        "distinct evidence_artifact values must be accepted, got {violations:?}"
    );
}

#[test]
fn absolute_path_value_is_a_provenance_leak() {
    let text = format!(
        "{}\n{}",
        baseline("VP-001-BASE-01"),
        scenario(
            "VP-001-VP-S6-01",
            "/home/operator/artifacts/build.AppImage",
            None
        )
    );
    let records = parse(&text).expect("well-formed table must parse");
    let violations = validate(&records);
    assert!(
        violations.iter().any(|v| v.token == "provenance-leak"),
        "an absolute local path must be flagged as a provenance-leak, got {violations:?}"
    );
}

#[test]
fn host_shaped_value_is_a_provenance_leak() {
    let text = format!(
        "{}\n{}",
        baseline("VP-001-BASE-01"),
        scenario("VP-001-VP-S6-01", "operator@runner-host-01", None)
    );
    let records = parse(&text).expect("well-formed table must parse");
    let violations = validate(&records);
    assert!(
        violations.iter().any(|v| v.token == "provenance-leak"),
        "a host-shaped value must be flagged as a provenance-leak, got {violations:?}"
    );
}

#[test]
fn repo_relative_procedure_ref_is_not_a_provenance_leak() {
    let text = format!(
        "{}\n{}",
        baseline("VP-001-BASE-01"),
        scenario("VP-001-VP-S6-01", "artifact-a", None)
    );
    let records = parse(&text).expect("well-formed table must parse");
    let violations = validate(&records);
    assert!(
        !violations.iter().any(|v| v.token == "provenance-leak"),
        "an ordinary repo-relative value must not be flagged, got {violations:?}"
    );
}
