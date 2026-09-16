//! V4: `record_id` unique across the whole set; `corrects` (when present)
//! resolves to an existing `record_id`.
//! V5: rows sharing `(scenario_id, baseline_id)` must carry distinct
//! `evidence_artifact` values, so a re-run until green is visible as
//! several rows rather than one silently overwriting the same artifact.
//! V6: a partial provenance lint (not a proof) against a value that looks
//! like an absolute local path or a host-shaped token -- an infrastructure
//! detail the Provenance-Clean Publication requirement forbids in a
//! published row.

use std::collections::HashSet;

use crate::record::{Record, Violation};

pub(super) fn check(records: &[Record], violations: &mut Vec<Violation>) {
    check_identifiers(records, violations);
    check_evidence_artifact_reuse(records, violations);
    check_provenance_leak(records, violations);
}

/// V4: `record_id` unique across the whole set; `corrects` (when present)
/// resolves to an existing `record_id`.
fn check_identifiers(records: &[Record], violations: &mut Vec<Violation>) {
    let mut seen_ids: HashSet<&str> = HashSet::new();
    for record in records {
        if let Some(id) = record.record_id().filter(|id| !id.is_empty())
            && !seen_ids.insert(id)
        {
            violations.push(Violation::new(
                Some(id),
                "duplicate-record-id",
                format!("record_id `{id}` is used by more than one record"),
            ));
        }
    }

    for record in records {
        let Some(corrects) = record.get("corrects").filter(|c| !c.is_empty()) else {
            continue;
        };
        let resolves = records.iter().any(|r| r.record_id() == Some(corrects));
        if !resolves {
            violations.push(Violation::new(
                record.record_id(),
                "corrects-unresolved",
                format!("corrects `{corrects}` does not resolve to an existing record_id"),
            ));
        }
    }
}

/// V5: rows sharing `(scenario_id, baseline_id)` must carry distinct
/// `evidence_artifact` values, so a re-run until green is visible as
/// several rows rather than one silently overwriting the same artifact.
fn check_evidence_artifact_reuse(records: &[Record], violations: &mut Vec<Violation>) {
    let mut seen: HashSet<(&str, &str, &str)> = HashSet::new();
    for record in records {
        if record.kind() != Some("scenario") {
            continue;
        }
        let (Some(scenario_id), Some(baseline_id), Some(artifact)) = (
            record.get("scenario_id").filter(|v| !v.is_empty()),
            record.get("baseline_id").filter(|v| !v.is_empty()),
            record.get("evidence_artifact").filter(|v| !v.is_empty()),
        ) else {
            continue;
        };
        let key = (scenario_id, baseline_id, artifact);
        if !seen.insert(key) {
            violations.push(Violation::new(
                record.record_id(),
                "evidence-artifact-reused",
                format!(
                    "evidence_artifact `{artifact}` reused for scenario `{scenario_id}` / baseline `{baseline_id}`"
                ),
            ));
        }
    }
}

/// V6: a partial provenance lint (not a proof) against a value that looks
/// like an absolute local path or a host-shaped token -- an infrastructure
/// detail the Provenance-Clean Publication requirement forbids in a
/// published row.
fn check_provenance_leak(records: &[Record], violations: &mut Vec<Violation>) {
    for record in records {
        for (field, value) in &record.fields {
            if looks_like_infrastructure_detail(value) {
                violations.push(Violation::new(
                    record.record_id(),
                    "provenance-leak",
                    format!("field `{field}` value `{value}` looks like an absolute path or host-shaped token"),
                ));
            }
        }
    }
}

/// Heuristic only (design.md V6: "a lint, not a proof"): absolute Unix or
/// Windows paths, and `user@host`-shaped or bare-IPv4 tokens.
fn looks_like_infrastructure_detail(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if value.starts_with('/') || value.starts_with("~/") {
        return true;
    }
    let bytes = value.as_bytes();
    if bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
        return true; // Windows drive-letter path, e.g. C:\Users\...
    }
    if value.contains('@') && !value.contains(char::is_whitespace) {
        return true; // user@host shape
    }
    is_ipv4_shaped(value)
}

fn is_ipv4_shaped(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| p.parse::<u8>().is_ok())
}
