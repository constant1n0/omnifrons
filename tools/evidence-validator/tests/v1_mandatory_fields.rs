//! V1: every mandatory schema field present and non-empty, per record kind
//! (design.md validator contract). A scenario row's `result` and
//! `observed_state` are two distinct mandatory fields -- neither substitutes
//! for the other.

use std::fmt::Write as _;

use evidence_validator::record::parse;
use evidence_validator::validate::validate;

const BASELINE_FIELDS: &[(&str, &str)] = &[
    ("record_id", "VP-001-BASE-01"),
    ("kind", "baseline"),
    ("os_build", "Ubuntu 24.04.1 LTS"),
    ("architecture", "x86_64"),
    ("webview_runtime", "WebKitGTK 2.44.1"),
    ("packaging_substrate", "appimage"),
    (
        "assistive_technology",
        "none exercised - VP-S19 out of scope",
    ),
    ("test_date", "2026-09-16"),
];

const SCENARIO_FIELDS: &[(&str, &str)] = &[
    ("record_id", "VP-001-VP-S6-01"),
    ("kind", "scenario"),
    ("scenario_id", "VP-S6"),
    ("baseline_id", "VP-001-BASE-01"),
    ("result", "uncertain"),
    ("observed_state", "orphan-risk"),
    ("build_channel", "packaged-ci"),
    ("variant_scan", "absent"),
    (
        "procedure_ref",
        "docs/evidence/VP-001/procedures/vp-s6-linux.sh",
    ),
];

fn record_section(fields: &[(&str, &str)]) -> String {
    let mut section = String::from("## Record\n\n| field | value |\n| --- | --- |\n");
    for (field, value) in fields {
        let _ = writeln!(section, "| `{field}` | {value} |");
    }
    section
}

fn table_omitting(fields: &[(&str, &str)], omit: &str) -> String {
    record_section(
        &fields
            .iter()
            .copied()
            .filter(|(field, _)| *field != omit)
            .collect::<Vec<_>>(),
    )
}

fn assert_field_missing(text: &str, expected_field: &str) {
    let records = parse(text).expect("well-formed table must parse");
    let violations = validate(&records);
    assert!(
        violations
            .iter()
            .any(|v| v.token == "field-missing" && v.detail.contains(expected_field)),
        "expected a field-missing violation naming `{expected_field}`, got {violations:?}"
    );
}

#[test]
fn baseline_missing_each_mandatory_field_in_turn_is_rejected() {
    for (field, _) in BASELINE_FIELDS {
        assert_field_missing(&table_omitting(BASELINE_FIELDS, field), field);
    }
}

#[test]
fn scenario_missing_each_mandatory_field_in_turn_is_rejected() {
    for (field, _) in SCENARIO_FIELDS {
        assert_field_missing(&table_omitting(SCENARIO_FIELDS, field), field);
    }
}

#[test]
fn scenario_result_and_observed_state_are_two_distinct_mandatory_fields() {
    // Omitting only `observed_state` (leaving `result` intact) must still be
    // rejected: one field can never stand in for the other.
    let missing_state = table_omitting(SCENARIO_FIELDS, "observed_state");
    assert_field_missing(&missing_state, "observed_state");

    let missing_state_records = parse(&missing_state).unwrap();
    let missing_state_violations = validate(&missing_state_records);
    assert!(
        !missing_state_violations
            .iter()
            .any(|v| v.token == "field-missing" && v.detail.contains("`result`")),
        "result is present, so it must not also be reported missing"
    );
}

#[test]
fn complete_baseline_and_scenario_pass_v1() {
    let text = format!(
        "{}\n{}",
        record_section(BASELINE_FIELDS),
        record_section(SCENARIO_FIELDS)
    );
    let records = parse(&text).expect("well-formed table must parse");
    let violations = validate(&records);
    assert!(
        !violations.iter().any(|v| v.token == "field-missing"),
        "a complete baseline and scenario must not report field-missing, got {violations:?}"
    );
}
