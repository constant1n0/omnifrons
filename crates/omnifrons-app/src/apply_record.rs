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
