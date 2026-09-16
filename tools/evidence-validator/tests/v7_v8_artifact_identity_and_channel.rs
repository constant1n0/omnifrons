//! V7: `exercised_artifact_digest` must equal `build_channel_digest`, and
//! `variant_scan` must be `absent` (AV1/AV2 recorded, not asserted).
//! V8: `build_channel` must be the packaged CI channel, and a
//! `build_channel_digest` must never appear with no retained
//! `evidence_artifact`.

use std::fmt::Write as _;

use evidence_validator::record::parse;
use evidence_validator::validate::validate;

#[allow(clippy::too_many_arguments)]
fn scenario(
    build_channel: &str,
    variant_scan: &str,
    build_channel_digest: &str,
    exercised_artifact_digest: &str,
    evidence_artifact: &str,
) -> String {
    let mut rows = String::from(
        "## Scenario\n\n\
         | field | value |\n\
         | --- | --- |\n\
         | `record_id` | VP-001-VP-S6-01 |\n\
         | `kind` | scenario |\n\
         | `scenario_id` | VP-S6 |\n\
         | `baseline_id` | VP-001-BASE-01 |\n\
         | `result` | uncertain |\n\
         | `observed_state` | orphan-risk |\n",
    );
    let _ = writeln!(rows, "| `build_channel` | {build_channel} |");
    if !variant_scan.is_empty() {
        let _ = writeln!(rows, "| `variant_scan` | {variant_scan} |");
    }
    rows.push_str("| `procedure_ref` | docs/evidence/VP-001/procedures/vp-s6-linux.sh |\n");
    if !build_channel_digest.is_empty() {
        let _ = writeln!(rows, "| `build_channel_digest` | {build_channel_digest} |");
    }
    if !exercised_artifact_digest.is_empty() {
        let _ = writeln!(
            rows,
            "| `exercised_artifact_digest` | {exercised_artifact_digest} |"
        );
    }
    if !evidence_artifact.is_empty() {
        let _ = writeln!(rows, "| `evidence_artifact` | {evidence_artifact} |");
    }
    rows
}

fn violated(text: &str, token: &str) -> bool {
    let records = parse(text).expect("well-formed table must parse");
    validate(&records).iter().any(|v| v.token == token)
}

#[test]
fn digest_mismatch_is_rejected() {
    let text = scenario(
        "packaged-ci",
        "absent",
        "sha256:aaa",
        "sha256:bbb",
        "artifact-1",
    );
    assert!(
        violated(&text, "artifact-digest-mismatch"),
        "differing exercised/build digests must be rejected"
    );
}

#[test]
fn matching_digests_are_accepted() {
    let text = scenario(
        "packaged-ci",
        "absent",
        "sha256:aaa",
        "sha256:aaa",
        "artifact-1",
    );
    assert!(
        !violated(&text, "artifact-digest-mismatch"),
        "identical exercised/build digests must be accepted"
    );
}

#[test]
fn variant_scan_found_is_rejected() {
    let text = scenario(
        "packaged-ci",
        "found",
        "sha256:aaa",
        "sha256:aaa",
        "artifact-1",
    );
    assert!(
        violated(&text, "feature-enabled-variant"),
        "a found demo-harness variant must be rejected"
    );
}

#[test]
fn variant_scan_missing_is_rejected_as_field_missing() {
    let text = scenario("packaged-ci", "", "sha256:aaa", "sha256:aaa", "artifact-1");
    assert!(
        violated(&text, "field-missing"),
        "a missing mandatory variant_scan must be rejected under V1"
    );
}

#[test]
fn non_packaged_build_channel_is_rejected() {
    let text = scenario("tauri-dev", "absent", "", "", "");
    assert!(
        violated(&text, "dev-mode-only"),
        "a non-packaged-ci build_channel must be rejected"
    );
}

#[test]
fn digest_with_no_retained_evidence_artifact_is_rejected() {
    let text = scenario("packaged-ci", "absent", "sha256:aaa", "sha256:aaa", "");
    assert!(
        violated(&text, "dev-mode-only"),
        "a build_channel_digest with no retained evidence_artifact must be rejected"
    );
}

#[test]
fn packaged_row_with_matching_digests_and_artifact_is_accepted() {
    let text = scenario(
        "packaged-ci",
        "absent",
        "sha256:aaa",
        "sha256:aaa",
        "artifact-1",
    );
    let records = parse(&text).expect("well-formed table must parse");
    let violations = validate(&records);
    assert!(
        !violations.iter().any(|v| [
            "dev-mode-only",
            "artifact-digest-mismatch",
            "feature-enabled-variant"
        ]
        .contains(&v.token.as_str())),
        "a complete packaged-ci row must not trip V7/V8, got {violations:?}"
    );
}
