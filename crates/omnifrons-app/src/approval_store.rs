//! The `ApprovalStore` port: durable (or in-memory, for tests) storage of
//! executable-approval records.

use std::time::SystemTime;

pub use omnifrons_domain::executable::{
    ApprovalId, ApprovalRecord, DeviceLocalUser, ExecutableIdentity,
};

/// Why an [`ApprovalStore`] operation failed.
///
/// Closed and exhaustive: every adapter-specific failure (a missing file, a
/// permissions error, a torn write, a malformed line, a colliding derived
/// id) reduces to one of these four, never a raw `io::Error` whose text
/// can carry a real filesystem path.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ApprovalStoreError {
    /// The store could not be read at all.
    #[error("the approval store could not be read")]
    Unreadable,
    /// The store's content could not be parsed as valid records.
    #[error("the approval store's content is corrupt")]
    Corrupt,
    /// A write to the store did not complete.
    #[error("the approval store could not be written")]
    WriteFailed,
    /// `record` derived an [`ApprovalId`] that already has a record on
    /// file. A conformant [`ApprovalStore`] must reject this rather than
    /// silently overwrite or merge with the existing record -- see
    /// `docs/spike-log.md` § Slice 2 on why an id is derived, not counted.
    #[error("the derived approval id already has a record on file")]
    Duplicate,
}

/// A port for recording, listing, and revoking executable approvals.
///
/// `record`/`revoke` take `&mut self`: a conformant implementation may
/// need exclusive access to append to (or otherwise mutate) durable
/// storage. `list`/`find_active` take `&self`: reading never needs
/// exclusive access.
pub trait ApprovalStore {
    /// List every approval on record, in an implementation-defined but
    /// stable order.
    ///
    /// # Errors
    ///
    /// Returns [`ApprovalStoreError`] if the store could not be read or its
    /// content is corrupt.
    fn list(&self) -> Result<Vec<ApprovalRecord>, ApprovalStoreError>;

    /// Record a fresh approval for `identity`, minting a new
    /// [`ApprovalId`] distinct from every id this store has ever issued
    /// (including a previously revoked one for the same identity: a
    /// re-approval after revocation is a new record, never a resurrection
    /// of the old one).
    ///
    /// # Errors
    ///
    /// Returns [`ApprovalStoreError`] if the write did not complete.
    fn record(
        &mut self,
        identity: ExecutableIdentity,
        approver: DeviceLocalUser,
        approved_at: SystemTime,
    ) -> Result<ApprovalRecord, ApprovalStoreError>;

    /// Revoke the approval identified by `id`, effective `at`.
    ///
    /// A no-op (not an error) if `id` is not on record at all: revoking
    /// something that was never approved has nothing to undo.
    ///
    /// # Errors
    ///
    /// Returns [`ApprovalStoreError`] if the write did not complete.
    fn revoke(&mut self, id: ApprovalId, at: SystemTime) -> Result<(), ApprovalStoreError>;

    /// Find the active (not revoked) approval on record for `identity`, if
    /// any, comparing by [`ExecutableIdentity`]'s own equality (canonical
    /// path, size, and digest only).
    ///
    /// # Errors
    ///
    /// Returns [`ApprovalStoreError`] if the store could not be read or its
    /// content is corrupt.
    fn find_active(
        &self,
        identity: &ExecutableIdentity,
    ) -> Result<Option<ApprovalRecord>, ApprovalStoreError>;
}
