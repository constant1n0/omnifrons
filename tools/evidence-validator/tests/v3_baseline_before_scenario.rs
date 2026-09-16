//! V3: a scenario row's `baseline_id` must resolve to a baseline record
//! carrying every VP-001-R1 field (Baseline-Before-Scenario Admission).

use evidence_validator::record::parse;
use evidence_validator::validate::validate;

const COMPLETE_BASELINE: &str = "## Baseline\n\n\
    | field | value |\n\
    | --- | --- |\n\
    | `record_id` | VP-001-BASE-01 |\n\
    | `kind` | baseline |\n\
    | `os_build` | Ubuntu 24.04.1 LTS |\n\
    | `architecture` | x86_64 |\n\
    | `webview_runtime` | WebKitGTK 2.44.1 |\n\
    | `packaging_substrate` | appimage |\n\
    | `assistive_technology` | none exercised - VP-S19 out of scope |\n\
    | `test_date` | 2026-09-16 |\n";

fn scenario_row(baseline_id: &str) -> String {
    format!(
        "## Scenario\n\n\
         | field | value |\n\
         | --- | --- |\n\
         | `record_id` | VP-001-VP-S6-01 |\n\
         | `kind` | scenario |\n\
         | `scenario_id` | VP-S6 |\n\
         | `baseline_id` | {baseline_id} |\n\
         | `result` | uncertain |\n\
         | `observed_state` | orphan-risk |\n\
         | `build_channel` | packaged-ci |\n\
         | `variant_scan` | absent |\n\
         | `procedure_ref` | docs/evidence/VP-001/procedures/vp-s6-linux.sh |\n"
    )
}

#[test]
fn scenario_row_with_no_matching_baseline_is_rejected() {
    let text = scenario_row("VP-001-BASE-DOES-NOT-EXIST");
    let records = parse(&text).expect("well-formed table must parse");
    let violations = validate(&records);
    assert!(
        violations.iter().any(|v| v.token == "baseline-unpinned"),
        "an unresolved baseline_id must be rejected as baseline-unpinned, got {violations:?}"
    );
}

#[test]
fn baseline_missing_a_vp_001_r1_field_blocks_the_scenario_row() {
    // A baseline record present but missing `webview_runtime`.
    let incomplete_baseline = "## Baseline\n\n\
        | field | value |\n\
        | --- | --- |\n\
        | `record_id` | VP-001-BASE-01 |\n\
        | `kind` | baseline |\n\
        | `os_build` | Ubuntu 24.04.1 LTS |\n\
        | `architecture` | x86_64 |\n\
        | `packaging_substrate` | appimage |\n\
        | `assistive_technology` | none exercised - VP-S19 out of scope |\n\
        | `test_date` | 2026-09-16 |\n";
    let text = format!("{incomplete_baseline}\n{}", scenario_row("VP-001-BASE-01"));
    let records = parse(&text).expect("well-formed table must parse");
    let violations = validate(&records);
    assert!(
        violations
            .iter()
            .any(|v| v.token == "baseline-unpinned" && v.detail.contains("webview_runtime")),
        "a baseline missing a VP-001-R1 field must block the scenario row, got {violations:?}"
    );
}

#[test]
fn scenario_row_with_complete_matching_baseline_is_admitted() {
    let text = format!("{COMPLETE_BASELINE}\n{}", scenario_row("VP-001-BASE-01"));
    let records = parse(&text).expect("well-formed table must parse");
    let violations = validate(&records);
    assert!(
        !violations.iter().any(|v| v.token == "baseline-unpinned"),
        "a complete matching baseline must admit the scenario row, got {violations:?}"
    );
}
