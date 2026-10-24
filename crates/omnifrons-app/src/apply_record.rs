//! The `ApplyRecordStore` port (ADR-0005 decision 3, sub-slice 2c): each
//! apply of a definition plan leaves one record in the product work area
//! -- the plan id, the agent it was for, every file's digest before and
//! after, the snapshot taken of it, and the operations that ran. The
//! record is device-local, like the snapshots it names, and is never
//! written into the definition itself. Persisted by `omnifrons-adapters`'
//! `FsApplyRecordStore`.

use std::fmt;
use std::time::SystemTime;

use omnifrons_domain::definition_plan::{InputFingerprint, Operation, PlanId};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::publication::ProjectIdentity;

use crate::snapshot_store::SnapshotId;

/// The schema integer every apply record this version writes carries.
pub const APPLY_RECORD_SCHEMA_VERSION: u32 = 1;

/// One file an apply considered: what it was, what it became, and the
/// snapshot that holds what it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedFile {
    /// The file's path relative to the agent directory.
    pub path: String,
    /// What the file looked like before the apply.
    pub before: InputFingerprint,
    /// The digest of the file after the apply, or `None` when the apply
    /// left no file there.
    pub after: Option<Sha256Digest>,
    /// The snapshot taken of the file before it was written, or `None`
    /// when there was nothing to snapshot.
    pub snapshot: Option<SnapshotId>,
}

/// One apply of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyRecord {
    /// The id of the plan that was applied.
    pub plan_id: PlanId,
    /// The workspace root as display text (the plan's own).
    pub workspace: String,
    /// The agent directory the plan was for, relative to the workspace.
    pub agent: String,
    /// When the apply finished.
    pub taken_at: SystemTime,
    /// Every file the apply considered.
    pub files: Vec<AppliedFile>,
    /// The operations the plan carried.
    pub operations: Vec<Operation>,
}

/// The stored name of one apply record: the plan id, the instant, and a
/// sequence number that tells apart two records of one plan at one
/// instant. Opaque to callers; ordered newest first by the store.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ApplyRecordId(pub String);

impl fmt::Display for ApplyRecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A record as listed: its stored name and its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredApplyRecord {
    /// The stored name.
    pub id: ApplyRecordId,
    /// The record.
    pub record: ApplyRecord,
}

/// Why an [`ApplyRecordStore`] operation failed. Closed and exhaustive,
/// never a raw `io::Error` whose text can carry a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ApplyRecordError {
    /// The records could not be read.
    #[error("the apply records could not be read")]
    Unreadable,
    /// A record does not parse: fails closed for the whole listing,
    /// never silently skipped.
    #[error("an apply record is corrupt")]
    Corrupt,
    /// A write did not complete; nothing was left behind.
    #[error("the apply record could not be written")]
    WriteFailed,
}

impl ApplyRecordError {
    /// This error's fixed reason token.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::Unreadable => "apply-record-unreadable",
            Self::Corrupt => "apply-record-corrupt",
            Self::WriteFailed => "apply-record-write-failed",
        }
    }
}

/// A port over the device's apply records, per project identity.
pub trait ApplyRecordStore {
    /// Store `record` for `project` under a name no earlier record has,
    /// even when the plan id and the instant repeat. Returns that name.
    ///
    /// # Errors
    ///
    /// Returns [`ApplyRecordError::WriteFailed`] if the record could not
    /// be written; nothing is left behind.
    fn record(
        &self,
        project: &ProjectIdentity,
        record: &ApplyRecord,
    ) -> Result<ApplyRecordId, ApplyRecordError>;

    /// Every record of `project`, newest first: by instant descending,
    /// then by stored name descending.
    ///
    /// # Errors
    ///
    /// Returns [`ApplyRecordError`] if the records cannot be read or any
    /// one is corrupt.
    fn list(&self, project: &ProjectIdentity) -> Result<Vec<StoredApplyRecord>, ApplyRecordError>;
}

/// Order `records` newest first.
pub fn sort_records_newest_first(records: &mut [StoredApplyRecord]) {
    records.sort_by(|a, b| {
        b.record
            .taken_at
            .cmp(&a.record.taken_at)
            .then_with(|| b.id.cmp(&a.id))
    });
}

/// The stored name for a record of `plan_id` taken at `taken_at`, the
/// `sequence`th attempt: `<plan hex>-<nanoseconds since the epoch,
/// 20 digits>-<sequence, 4 digits>`. The fixed widths make the names sort
/// the way the records do.
#[must_use]
pub fn apply_record_name(plan_id: &PlanId, taken_at: SystemTime, sequence: u32) -> ApplyRecordId {
    let nanos = taken_at
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    ApplyRecordId(format!("{plan_id}-{nanos:020}-{sequence:04}"))
}

/// The parts of a stored record name, as [`parse_apply_record_name`]
/// reads them back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedApplyRecordName {
    /// The plan id the name carries.
    pub plan_id: PlanId,
    /// The instant the name carries, nanosecond-exact.
    pub taken_at: SystemTime,
    /// The attempt number the name carries.
    pub sequence: u32,
}

/// Reads a stored name written by [`apply_record_name`]:
/// `<64 hex plan id>-<20 digit nanoseconds>-<4 digit sequence>`. Returns
/// `None` for any other shape, including different widths, signs, or
/// uppercase hex.
#[must_use]
pub fn parse_apply_record_name(name: &str) -> Option<ParsedApplyRecordName> {
    let mut parts = name.split('-');
    let (plan, nanos, sequence) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || plan.len() != 64 || nanos.len() != 20 || sequence.len() != 4 {
        return None;
    }
    let all_digits = |text: &str| text.bytes().all(|b| b.is_ascii_digit());
    let lower_hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
    if !all_digits(nanos) || !all_digits(sequence) || !plan.bytes().all(lower_hex) {
        return None;
    }
    let plan_id = PlanId(Sha256Digest::from_hex(plan)?.0);
    let nanos: u128 = nanos.parse().ok()?;
    let secs = u64::try_from(nanos / 1_000_000_000).ok()?;
    let subsec = u32::try_from(nanos % 1_000_000_000).ok()?;
    let taken_at = SystemTime::UNIX_EPOCH.checked_add(std::time::Duration::new(secs, subsec))?;
    Some(ParsedApplyRecordName {
        plan_id,
        taken_at,
        sequence: sequence.parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_generated_name_parses_back_to_its_parts() {
        let plan_id = PlanId([0xab; 32]);
        let taken_at = SystemTime::UNIX_EPOCH + Duration::new(1_725_782_401, 500);
        let name = apply_record_name(&plan_id, taken_at, 7);
        let parsed = parse_apply_record_name(&name.0).expect("parses");
        assert_eq!(parsed.plan_id, plan_id);
        assert_eq!(parsed.taken_at, taken_at);
        assert_eq!(parsed.sequence, 7);
    }

    #[test]
    fn names_of_any_other_shape_do_not_parse() {
        let good = apply_record_name(&PlanId([0xab; 32]), SystemTime::UNIX_EPOCH, 0).0;
        assert!(parse_apply_record_name(&good).is_some());
        let (plan, rest) = good.split_at(64);
        let bad_names = [
            "junk".to_string(),
            String::new(),
            good[1..].to_string(),
            format!("{good}0"),
            format!("{good}-0001"),
            format!("{plan}-0000000000000000000x-0000"),
            format!("{plan}-+0000000000000000000-0000"),
            format!("{plan}-00000000000000000000-000x"),
            format!("{plan}-00000000000000000000-+000"),
            format!("{}{rest}", "+b".repeat(32)),
            good.to_uppercase(),
        ];
        for bad in &bad_names {
            assert_eq!(parse_apply_record_name(bad), None, "{bad}");
        }
    }
}
