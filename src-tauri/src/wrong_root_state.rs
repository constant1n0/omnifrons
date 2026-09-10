//! Managed state for the wrong-root surface (spike slice 5d, HAP-001 §
//! Wrong-root detection and remedies, D16): the findings of the most
//! recent scan for the active workspace, and nothing else.
//!
//! **`misplaced` is recomputed, never persisted.** The contract calls it a
//! detection condition, not a publication state (HAP-001 § Artifact
//! states), so this table lives only as long as the process and is cleared
//! when the active workspace changes -- one project's findings never speak
//! for the next. The one durable fact the surface leaves behind is the
//! ignore decision, which `JsonlIgnoreLedger` keeps in the product work
//! area.
//!
//! Nothing here holds a file handle: a scan opens, reads, and closes each
//! file within the walk. The 32-handle cap HAP-001 D22 sets is shared with
//! the approval surface, and a scan that held handles could starve it.

use std::sync::Mutex;

use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::publication::ProjectIdentity;
use omnifrons_domain::wrong_root::MisplacedFinding;

/// One completed scan's result, and **which project it walked**.
///
/// The project is part of the record and not an assumption about whichever
/// workspace happens to be active when it is read (spike slice 5d, R1-005).
/// One process holds one table; a run in project A can finish after the
/// active workspace has moved to B, and a reader that assumed otherwise
/// would let A's findings answer for B -- which, with identical bytes at
/// the same relative name (a shared fixture, a copied artifact), means the
/// quarantine remedy deleting a file in a project the user never scanned.
#[derive(Debug)]
pub struct ScanRecord {
    /// The project this scan walked. Every read is keyed to it.
    pub project: ProjectIdentity,
    /// The findings still standing after the ignore ledger was consulted,
    /// in walk order.
    pub findings: Vec<MisplacedFinding>,
    /// How many files the walk examined.
    pub scanned: u32,
    /// How many entries an exclusion skipped.
    pub excluded: u32,
    /// How many findings the ignore ledger withheld.
    pub ignored: u32,
    /// How many entries could not be opened or read.
    pub unreadable: u32,
    /// Whether the walk stopped at its bound.
    pub truncated: bool,
}

/// This shell's managed state for the wrong-root surface.
#[derive(Debug, Default)]
pub struct WrongRootState {
    scan: Mutex<Option<ScanRecord>>,
}

impl WrongRootState {
    /// Empty: no scan has run in this session.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the remembered scan with `record`.
    ///
    /// # Panics
    ///
    /// Panics if the lock was poisoned by a prior panic while held.
    pub fn record(&self, record: ScanRecord) {
        *self
            .scan
            .lock()
            .expect("wrong-root scan mutex poisoned by a prior panic") = Some(record);
    }

    /// Whether a scan has run **for `project`**, and how many findings it
    /// left standing. `None` -- no project is active -- is answered exactly
    /// as a project no scan ran for.
    ///
    /// # Panics
    ///
    /// Panics if the lock was poisoned by a prior panic while held.
    #[must_use]
    pub fn summary(&self, project: Option<&ProjectIdentity>) -> (bool, u32) {
        let scan = self
            .scan
            .lock()
            .expect("wrong-root scan mutex poisoned by a prior panic");
        scan.as_ref()
            .filter(|record| project == Some(&record.project))
            .map_or((false, 0), |record| {
                (
                    true,
                    u32::try_from(record.findings.len()).unwrap_or(u32::MAX),
                )
            })
    }

    /// Every finding the last scan **of `project`** left standing, in walk
    /// order. Empty for any other project, and for none.
    ///
    /// # Panics
    ///
    /// Panics if the lock was poisoned by a prior panic while held.
    #[must_use]
    pub fn findings(&self, project: Option<&ProjectIdentity>) -> Vec<MisplacedFinding> {
        self.scan
            .lock()
            .expect("wrong-root scan mutex poisoned by a prior panic")
            .as_ref()
            .filter(|record| project == Some(&record.project))
            .map(|record| record.findings.clone())
            .unwrap_or_default()
    }

    /// The finding the last scan **of `project`** reported at `name` with
    /// `digest` -- the pair a remedy request binds to, exactly as
    /// `artifact_approve` binds a candidate by name and digest
    /// (HAP-001-R22, R32). `None` for a name this scan never reported, for
    /// one reported at other bytes, and for a record that belongs to
    /// another project entirely.
    ///
    /// # Panics
    ///
    /// Panics if the lock was poisoned by a prior panic while held.
    #[must_use]
    pub fn find(
        &self,
        project: &ProjectIdentity,
        name: &str,
        digest: &Sha256Digest,
    ) -> Option<MisplacedFinding> {
        self.scan
            .lock()
            .expect("wrong-root scan mutex poisoned by a prior panic")
            .as_ref()
            .filter(|record| &record.project == project)
            .and_then(|record| {
                record
                    .findings
                    .iter()
                    .find(|finding| finding.is_named_by(name, digest))
                    .cloned()
            })
    }

    /// Forget the finding at `name` with `digest`, once a remedy has run
    /// over it: the file is no longer where the scan found it (quarantine),
    /// or the user has dismissed it (ignore), or it has been offered to the
    /// outbox (publish). Returns whether one was removed.
    ///
    /// # Panics
    ///
    /// Panics if the lock was poisoned by a prior panic while held.
    pub fn forget(&self, project: &ProjectIdentity, name: &str, digest: &Sha256Digest) -> bool {
        let mut scan = self
            .scan
            .lock()
            .expect("wrong-root scan mutex poisoned by a prior panic");
        let Some(record) = scan.as_mut().filter(|record| &record.project == project) else {
            return false;
        };
        let before = record.findings.len();
        record
            .findings
            .retain(|finding| !finding.is_named_by(name, digest));
        record.findings.len() != before
    }

    /// The active workspace changed: forget the scan entirely. Returns
    /// whether one was remembered.
    ///
    /// # Panics
    ///
    /// Panics if the lock was poisoned by a prior panic while held.
    pub fn clear(&self) -> bool {
        self.scan
            .lock()
            .expect("wrong-root scan mutex poisoned by a prior panic")
            .take()
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{ScanRecord, WrongRootState};
    use omnifrons_domain::executable::Sha256Digest;
    use omnifrons_domain::outbox::{ArtifactClass, DetectedType};
    use omnifrons_domain::publication::ProjectIdentity;
    use omnifrons_domain::wrong_root::{MisplacedFinding, WrongRootReason};

    fn finding(name: &str, digest: u8) -> MisplacedFinding {
        MisplacedFinding::new(
            name,
            10,
            Sha256Digest([digest; 32]),
            DetectedType::Pdf,
            ArtifactClass::GeneratedHeavy,
            WrongRootReason::InProjectOutsideOutbox,
        )
        .expect("a project-relative name")
    }

    fn project(seed: u8) -> ProjectIdentity {
        ProjectIdentity(Sha256Digest([seed; 32]))
    }

    fn record(seed: u8, findings: Vec<MisplacedFinding>) -> ScanRecord {
        ScanRecord {
            project: project(seed),
            findings,
            scanned: 2,
            excluded: 0,
            ignored: 0,
            unreadable: 0,
            truncated: false,
        }
    }

    #[test]
    fn a_remedy_request_is_bound_to_a_name_and_a_digest_together() {
        let state = WrongRootState::new();
        assert_eq!(state.summary(Some(&project(1))), (false, 0));
        state.record(record(
            1,
            vec![finding("docs/a.pdf", 1), finding("docs/b.pdf", 2)],
        ));
        assert_eq!(state.summary(Some(&project(1))), (true, 2));
        assert!(
            state
                .find(&project(1), "docs/a.pdf", &Sha256Digest([1; 32]))
                .is_some()
        );
        assert!(
            state
                .find(&project(1), "docs/a.pdf", &Sha256Digest([2; 32]))
                .is_none(),
            "the digest binds the request as tightly as the name"
        );
        assert!(
            state
                .find(&project(1), "docs/c.pdf", &Sha256Digest([1; 32]))
                .is_none()
        );
    }

    /// R1-005: the record belongs to the project it walked. One process
    /// holds one table and a run can finish after the active workspace has
    /// moved on, so a reader that did not name its project could take a
    /// finding from a project it never scanned -- and the quarantine remedy
    /// deletes the file a finding names.
    #[test]
    fn a_record_answers_only_for_the_project_it_walked() {
        let state = WrongRootState::new();
        state.record(record(1, vec![finding("docs/a.pdf", 1)]));

        assert_eq!(state.summary(Some(&project(2))), (false, 0));
        assert!(state.findings(Some(&project(2))).is_empty());
        assert!(
            state
                .find(&project(2), "docs/a.pdf", &Sha256Digest([1; 32]))
                .is_none(),
            "another project's finding never answers for this one"
        );
        assert!(!state.forget(&project(2), "docs/a.pdf", &Sha256Digest([1; 32])));

        // No project active at all is answered exactly the same way.
        assert_eq!(state.summary(None), (false, 0));
        assert!(state.findings(None).is_empty());

        // And the record itself is untouched by any of that.
        assert_eq!(state.summary(Some(&project(1))), (true, 1));
    }

    #[test]
    fn a_remedied_finding_is_forgotten_and_a_workspace_change_clears_the_scan() {
        let state = WrongRootState::new();
        state.record(record(
            1,
            vec![finding("docs/a.pdf", 1), finding("docs/b.pdf", 2)],
        ));
        assert!(state.forget(&project(1), "docs/a.pdf", &Sha256Digest([1; 32])));
        assert!(!state.forget(&project(1), "docs/a.pdf", &Sha256Digest([1; 32])));
        assert_eq!(state.summary(Some(&project(1))), (true, 1));
        assert!(state.clear());
        assert_eq!(
            state.summary(Some(&project(1))),
            (false, 0),
            "a workspace change leaves no findings behind"
        );
        assert!(!state.clear());
    }
}
