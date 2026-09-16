//! V7: `exercised_artifact_digest` must equal `build_channel_digest` when
//! both are present, and `variant_scan` must be exactly `absent` -- AV1 and
//! AV2 are recorded facts, never merely asserted.
//! V8: `build_channel` must be the packaged CI channel, and a
//! `build_channel_digest` must never appear on a row with no retained
//! `evidence_artifact`.

use crate::record::{Record, Violation};

pub(super) fn check(records: &[Record], violations: &mut Vec<Violation>) {
    check_artifact_identity(records, violations);
    check_packaged_channel(records, violations);
}

/// V7: `exercised_artifact_digest` must equal `build_channel_digest` when
/// both are present, and `variant_scan` must be exactly `absent` -- AV1 and
/// AV2 are recorded facts, never merely asserted.
fn check_artifact_identity(records: &[Record], violations: &mut Vec<Violation>) {
    for record in records {
        if record.kind() != Some("scenario") {
            continue;
        }
        if let (Some(exercised), Some(channel)) = (
            record
                .get("exercised_artifact_digest")
                .filter(|v| !v.is_empty()),
            record.get("build_channel_digest").filter(|v| !v.is_empty()),
        ) && exercised != channel
        {
            violations.push(Violation::new(
                record.record_id(),
                "artifact-digest-mismatch",
                "exercised_artifact_digest does not equal build_channel_digest".to_string(),
            ));
        }
        if let Some(variant_scan) = record.get("variant_scan").filter(|v| !v.is_empty())
            && variant_scan != "absent"
        {
            violations.push(Violation::new(
                record.record_id(),
                "feature-enabled-variant",
                format!("variant_scan `{variant_scan}` must be `absent`"),
            ));
        }
    }
}

/// V8: `build_channel` must be the packaged CI channel, and a
/// `build_channel_digest` must never appear on a row with no retained
/// `evidence_artifact`.
fn check_packaged_channel(records: &[Record], violations: &mut Vec<Violation>) {
    for record in records {
        if record.kind() != Some("scenario") {
            continue;
        }
        if let Some(channel) = record.get("build_channel").filter(|v| !v.is_empty())
            && channel != "packaged-ci"
        {
            violations.push(Violation::new(
                record.record_id(),
                "dev-mode-only",
                format!("build_channel `{channel}` is not the packaged CI channel"),
            ));
            continue;
        }
        let has_digest = record
            .get("build_channel_digest")
            .is_some_and(|v| !v.is_empty());
        let has_artifact = record
            .get("evidence_artifact")
            .is_some_and(|v| !v.is_empty());
        if has_digest && !has_artifact {
            violations.push(Violation::new(
                record.record_id(),
                "dev-mode-only",
                "build_channel_digest is recorded with no retained evidence_artifact".to_string(),
            ));
        }
    }
}
