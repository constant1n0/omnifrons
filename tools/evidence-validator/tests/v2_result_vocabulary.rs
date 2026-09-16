//! V2: `result` must be exactly `pass`, `fail`, or `uncertain`. A compound
//! value such as `orphan-risk/uncertain` is rejected -- that state belongs
//! in the separate `observed_state` field, never folded into `result`.

use evidence_validator::record::parse;
use evidence_validator::validate::validate;

fn scenario_with_result(result: &str) -> String {
    format!(
        "## Record\n\n\
         | field | value |\n\
         | --- | --- |\n\
         | `record_id` | VP-001-VP-S6-01 |\n\
         | `kind` | scenario |\n\
         | `scenario_id` | VP-S6 |\n\
         | `baseline_id` | VP-001-BASE-01 |\n\
         | `result` | {result} |\n\
         | `observed_state` | orphan-risk |\n\
         | `build_channel` | packaged-ci |\n\
         | `variant_scan` | absent |\n\
         | `procedure_ref` | docs/evidence/VP-001/procedures/vp-s6-linux.sh |\n"
    )
}

#[test]
fn out_of_vocabulary_result_is_rejected() {
    for bad_result in ["orphan-risk/uncertain", "PASS", "cleanly-stopped", ""] {
        // An empty `result` is already caught by V1 (field-missing); the
        // other three must be caught here, as result-out-of-vocabulary.
        let text = scenario_with_result(bad_result);
        let records = parse(&text).expect("well-formed table must parse");
        let violations = validate(&records);
        if bad_result.is_empty() {
            continue;
        }
        assert!(
            violations
                .iter()
                .any(|v| v.token == "result-out-of-vocabulary"),
            "result `{bad_result}` must be rejected as result-out-of-vocabulary, got {violations:?}"
        );
    }
}

#[test]
fn each_closed_vocabulary_value_is_accepted() {
    for good_result in ["pass", "fail", "uncertain"] {
        let text = scenario_with_result(good_result);
        let records = parse(&text).expect("well-formed table must parse");
        let violations = validate(&records);
        assert!(
            !violations
                .iter()
                .any(|v| v.token == "result-out-of-vocabulary"),
            "result `{good_result}` must be accepted, got {violations:?}"
        );
    }
}
