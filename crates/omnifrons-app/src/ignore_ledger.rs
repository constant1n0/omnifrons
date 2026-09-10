//! The `IgnoreLedger` port (spike slice 5d, HAP-001 § Wrong-root
//! detection and remedies, the ignore remedy): "the decision is recorded
//! against the file's identity and digest, and the file is not offered
//! again until its content changes".
//!
//! This is the **only durable fact** the wrong-root surface leaves behind.
//! `misplaced` is a detection condition, not a publication state
//! (HAP-001 § Artifact states), so a finding is recomputed by every scan
//! and never persisted; an ignore decision is a user decision and must
//! survive a restart, so it is.
//!
//! **What "identity" means here.** A finding never carries a device path
//! (RCS-001-R14), and a device/inode pair is neither portable nor stable
//! across every platform this product targets, so the key is the project
//! identity, the project-relative name, and the content digest. Two
//! consequences, both disclosed in `docs/spike-log.md` § Slice 5d: the
//! same bytes moved to another name are offered again, and a file rewritten
//! in place is offered again -- which is exactly what the remedy promises
//! ("until its content changes").
//!
//! Persisted like the publication journal: an append-only JSONL file in
//! the product work area, owner-only, failing closed on a line that does
//! not parse. Implemented by `omnifrons-adapters`' `JsonlIgnoreLedger`.

use std::time::SystemTime;

use omnifrons_domain::outbox::{ArtifactClass, ContentDigest, DetectedType};
use omnifrons_domain::publication::ProjectIdentity;
use omnifrons_domain::wrong_root::MisplacedFinding;

/// One recorded ignore decision: which file, by name and digest, and the
/// facts the surface showed when the user decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreEntry {
    /// The project-relative name the finding was offered under.
    pub name: String,
    /// The content digest the decision binds to.
    pub digest: ContentDigest,
    /// The size the surface showed.
    pub size: u64,
    /// The type the surface showed.
    pub detected_type: DetectedType,
    /// The class the surface showed.
    pub class: ArtifactClass,
    /// When the decision was recorded.
    pub ignored_at: SystemTime,
}

impl IgnoreEntry {
    /// The entry recording `finding` as ignored at `at`.
    #[must_use]
    pub fn of(finding: &MisplacedFinding, at: SystemTime) -> Self {
        Self {
            name: finding.name().to_string(),
            digest: finding.digest,
            size: finding.size,
            detected_type: finding.detected_type,
            class: finding.class,
            ignored_at: at,
        }
    }

    /// Whether this entry is the decision recorded for `name` and
    /// `digest`.
    #[must_use]
    pub fn covers(&self, name: &str, digest: &ContentDigest) -> bool {
        self.name == name && &self.digest == digest
    }
}

/// Why an [`IgnoreLedger`] operation failed. Closed and exhaustive, never
/// a raw `io::Error` whose text can carry a device path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IgnoreLedgerError {
    /// The ledger could not be read.
    #[error("the ignore ledger could not be read")]
    Unreadable,
    /// A line does not parse, or carries another schema: the whole read
    /// fails closed rather than silently skipping a decision, so a
    /// corrupt ledger never causes a file the user ignored to be offered
    /// again as if no decision existed.
    #[error("the ignore ledger's content is corrupt")]
    Corrupt,
    /// A write did not complete.
    #[error("the ignore ledger could not be written")]
    WriteFailed,
}

/// A port over the device's ignore decisions, per project identity.
pub trait IgnoreLedger {
    /// Record `entry` for `project`, durably.
    ///
    /// # Errors
    ///
    /// Returns [`IgnoreLedgerError::WriteFailed`] if the write did not
    /// complete, or a read error if the ledger had to be read first.
    fn record(
        &mut self,
        project: &ProjectIdentity,
        entry: &IgnoreEntry,
    ) -> Result<(), IgnoreLedgerError>;

    /// Every decision recorded for `project`, in the order recorded.
    ///
    /// # Errors
    ///
    /// Returns [`IgnoreLedgerError`] if the ledger cannot be read or is
    /// corrupt.
    fn entries(&self, project: &ProjectIdentity) -> Result<Vec<IgnoreEntry>, IgnoreLedgerError>;

    /// Whether `name` at `digest` was ignored for `project`.
    ///
    /// # Errors
    ///
    /// Returns [`IgnoreLedgerError`] if the ledger cannot be read or is
    /// corrupt. A read that fails is never "not ignored": the caller
    /// propagates it rather than offering a file the user already
    /// dismissed.
    fn is_ignored(
        &self,
        project: &ProjectIdentity,
        name: &str,
        digest: &ContentDigest,
    ) -> Result<bool, IgnoreLedgerError> {
        Ok(self
            .entries(project)?
            .iter()
            .any(|entry| entry.covers(name, digest)))
    }
}

/// Split `findings` into the ones still to offer and a count of the ones
/// `ledger` says were ignored for `project`.
///
/// # Errors
///
/// Returns [`IgnoreLedgerError`] if the ledger cannot be read; nothing is
/// offered on a ledger that cannot be consulted.
pub fn retain_unignored(
    findings: Vec<MisplacedFinding>,
    ledger: &dyn IgnoreLedger,
    project: &ProjectIdentity,
) -> Result<(Vec<MisplacedFinding>, u32), IgnoreLedgerError> {
    let recorded = ledger.entries(project)?;
    let mut kept = Vec::with_capacity(findings.len());
    let mut ignored = 0u32;
    for finding in findings {
        if recorded
            .iter()
            .any(|entry| entry.covers(finding.name(), &finding.digest))
        {
            ignored = ignored.saturating_add(1);
        } else {
            kept.push(finding);
        }
    }
    Ok((kept, ignored))
}
