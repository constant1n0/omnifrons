//! The `PublicationJournal` port (spike slice 5b, HAP-001-R29): the
//! device-local, append-only record in the product work area of every
//! publication step and its outcome, replayed on restart to resume an
//! interrupted publication from its last verified step. The approval that
//! authorizes a publication (step 6) is the journal's first entry for it,
//! so the work area holds `journal/` and `recovery/` and nothing else.
//!
//! The pure queries below read a replayed journal; the port itself only
//! appends and replays.

use omnifrons_domain::publication::{
    ArtifactApproval, ArtifactApprovalId, CatalogRecord, JournalEntry, JournalStep,
    PublicationIdentity, StepEntry, StepOutcome,
};

/// Why a [`PublicationJournal`] operation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JournalError {
    /// The journal could not be read.
    #[error("the publication journal could not be read")]
    Unreadable,
    /// The journal's content is not a valid entry log.
    #[error("the publication journal's content is corrupt")]
    Corrupt,
    /// A write did not complete.
    #[error("the publication journal could not be written")]
    WriteFailed,
}

/// A port over the device's publication journal.
pub trait PublicationJournal {
    /// Append `entry`, durably, before the next step runs.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::WriteFailed`] if the write did not complete.
    fn append(&mut self, entry: &JournalEntry) -> Result<(), JournalError>;

    /// Every entry, in order.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError`] if the journal cannot be read or is
    /// corrupt.
    fn replay(&self) -> Result<Vec<JournalEntry>, JournalError>;
}

/// The approval recorded under `id`, if any.
#[must_use]
pub fn find_approval(
    entries: &[JournalEntry],
    id: ArtifactApprovalId,
) -> Option<&ArtifactApproval> {
    entries.iter().find_map(|entry| match entry {
        JournalEntry::Approved(approval) if approval.approval_id == id => Some(approval.as_ref()),
        _ => None,
    })
}

/// Every step entry for `publication`, in order.
#[must_use]
pub fn steps_for<'a>(
    entries: &'a [JournalEntry],
    publication: &PublicationIdentity,
) -> Vec<&'a StepEntry> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            JournalEntry::Step(step) if &step.publication_id == publication => Some(step.as_ref()),
            _ => None,
        })
        .collect()
}

/// Where a publication stands according to its journal: the point a
/// retry or a restart resumes from (HAP-001-R29).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumePoint {
    /// No verified step yet: the transaction runs from the start.
    NotStarted,
    /// `published-local` was verified and journaled with this record, and
    /// registration has not completed: resume there.
    PublishedLocal(Box<CatalogRecord>),
    /// Registration completed: a repeated request is a duplicate.
    Registered,
}

/// The resume point of `publication`: the latest `published-local` or
/// `registered` step decides; a later failed step of another attempt does
/// not undo a verified `published-local`.
#[must_use]
pub fn resume_point(entries: &[JournalEntry], publication: &PublicationIdentity) -> ResumePoint {
    let mut point = ResumePoint::NotStarted;
    for step in steps_for(entries, publication) {
        match (step.step, step.outcome) {
            (JournalStep::Registered, StepOutcome::Ok) => point = ResumePoint::Registered,
            (JournalStep::PublishedLocal, StepOutcome::Ok) => {
                if let Some(record) = &step.record
                    && point != ResumePoint::Registered
                {
                    point = ResumePoint::PublishedLocal(Box::new(record.clone()));
                }
            }
            _ => {}
        }
    }
    point
}

/// Every publication whose resume point is `published-local`, with the
/// approval id its `published-local` step recorded, in journal order --
/// what a restart resumes (HAP-001-R29). The step entry is
/// self-sufficient: recovery never needs the approval line itself.
#[must_use]
pub fn pending_registrations(entries: &[JournalEntry]) -> Vec<(CatalogRecord, ArtifactApprovalId)> {
    let mut seen: Vec<PublicationIdentity> = Vec::new();
    let mut pending = Vec::new();
    for entry in entries {
        let publication = *entry.publication_id();
        if seen.contains(&publication) {
            continue;
        }
        seen.push(publication);
        if let ResumePoint::PublishedLocal(record) = resume_point(entries, &publication) {
            let approval_id = steps_for(entries, &publication)
                .into_iter()
                .rev()
                .find(|step| {
                    step.step == JournalStep::PublishedLocal && step.outcome == StepOutcome::Ok
                })
                .map(|step| step.approval_id);
            if let Some(approval_id) = approval_id {
                pending.push((*record, approval_id));
            }
        }
    }
    pending
}
