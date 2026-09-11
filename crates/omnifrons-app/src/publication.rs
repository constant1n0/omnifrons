//! The publication transaction (spike slice 5b, HAP-001 § Publication
//! transaction, steps 7 through 10 plus the approval validation and the
//! idempotency check), generic over the ports: the work area re-check
//! (HAP-001-R7), the journal replay that decides whether the request is a
//! duplicate or a resume (HAP-001-R23, R29), the identity facts re-taken
//! from the held handle (regular file, link count; HAP-001-R15, R20), the
//! path-still-names-the-handle check with the recovery entry on a
//! mismatch (HAP-001-R18), the copy streamed from the held handle with its
//! digest taken from the same bytes (HAP-001-R16, R17), the published copy
//! re-read and compared (HAP-001-R21), the record registered and the
//! reference issued only then (HAP-001-R24), the entry removed only after
//! both and only while the path still names the handle's file
//! (HAP-001-R28), one journal entry per step (HAP-001-R29), and one
//! `artifact.state` event per transition (HAP-001-R35).
//!
//! Every failure ends the transaction with nothing published (HAP-001-R13)
//! except the one HAP-001 makes recoverable: a registration failure after
//! a verified `published-local` is `registration-pending`, resumed by
//! [`recover`] from the record the journal carries.

use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Seek as _, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use omnifrons_domain::outbox::{Attribution, ContentDigest};
use omnifrons_domain::publication::{
    ApprovalSource, ArtifactApproval, ArtifactState, CatalogId, CatalogRecord, JournalEntry,
    JournalStep, PortableReference, Producer, ProjectIdentity, Provenance, ProviderRecord,
    ProviderState, PublicationIdentity, RECORD_VERSION, StepEntry, StepOutcome,
};

use crate::blob_store::{BlobStoreError, BlobStorePort};
use crate::catalog_store::{CatalogStore, CatalogStoreError};
use crate::clock::Clock;
use crate::content_hasher::ContentHasher;
use crate::harness_adapter::WorkspaceRoot;
use crate::outbox_entry_ops::{EntryIdentity, OutboxEntryOps};
use crate::publication_journal::{JournalError, PublicationJournal, ResumePoint, resume_point};
use crate::run_outbox::DirectoryHandle;
use crate::work_area::{WorkAreaError, WorkAreaRoot};

/// The reason token the local-directory adapter's `pending` carries; the
/// transaction records the adapter's own reason when `confirm` yields
/// nothing.
pub const NO_CONFIRMATION_REASON: &str = "no remote";

/// The fixed cleanup reason a publication of a recovery entry journals
/// (spike slice 5e, HAP-001-R18): the preserved bytes are kept, and this
/// says so rather than leaving a removal that never happened unexplained.
pub const RECOVERY_ENTRY_RETAINED: &str = "recovery-entry-retained";

/// The held handle and where its name lives: the one source of every
/// published byte, and the directory the identity check and the removal
/// run relative to.
#[derive(Debug)]
pub struct CandidateSource {
    /// The directory the entry sits in (the run subdirectory, or the
    /// outbox for a root entry), open.
    pub dir: DirectoryHandle,
    /// That directory's path, for the platforms with no relative open.
    pub dir_path: PathBuf,
    /// The entry's own name within `dir`.
    pub file_name: OsString,
    /// The handle held since validation (HAP-001-R17).
    pub handle: File,
}

/// The ports one transaction runs over.
pub struct PublishPorts<'a> {
    /// SHA-256 over bytes and streams.
    pub hasher: &'a dyn ContentHasher,
    /// The provider adapter for the destination asset root.
    pub provider: &'a dyn BlobStorePort,
    /// The project's Catalog.
    pub catalog: &'a mut dyn CatalogStore,
    /// The device's publication journal.
    pub journal: &'a mut dyn PublicationJournal,
    /// Identity checks and removal at the entry's name.
    pub entry_ops: &'a dyn OutboxEntryOps,
    /// The wall clock.
    pub clock: &'a dyn Clock,
    /// The product work area, re-checked at every use.
    pub work_area: &'a WorkAreaRoot,
    /// Every registered workspace root the work area is checked against.
    pub workspaces: &'a [&'a WorkspaceRoot],
}

/// One `artifact.state` transition (HAP-001-R35): the publication
/// identity, the state entered, and the record's provider state when a
/// record exists. Never a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateEvent {
    /// The publication.
    pub publication_id: PublicationIdentity,
    /// The state entered.
    pub state: ArtifactState,
    /// The record's `provider_state`, once a record exists.
    pub provider_state: Option<ProviderState>,
}

/// Why the published copy's digest did not match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrityReason {
    /// The bytes read from the held handle no longer hash to the approved
    /// digest: the file was rewritten in place after approval.
    SourceDigestChanged,
    /// The committed copy, read back, hashes differently from the bytes
    /// written.
    PublishedCopyDiffers,
}

impl IntegrityReason {
    /// This reason's fixed journal token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::SourceDigestChanged => "source-digest-changed",
            Self::PublishedCopyDiffers => "published-copy-digest-differs",
        }
    }
}

/// How the transaction ended without a registered artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishError {
    /// The work area resolves inside a registered workspace root
    /// (HAP-001-R7); nothing was journaled or published.
    WorkAreaInvalid,
    /// The path no longer names the handle's file, or the handle is not a
    /// regular file (HAP-001-R18, R15); the held bytes are the recovery
    /// entry named by `recovery` when one could be written.
    OutboxEscape {
        /// The recovery entry's digest (its name under `recovery/`).
        recovery: Option<ContentDigest>,
    },
    /// The handle's link count is greater than one (HAP-001-R20).
    OutboxLinked {
        /// The link count read from the handle.
        link_count: u64,
    },
    /// The digest did not verify (HAP-001-R21).
    IntegrityMismatch {
        /// Which comparison failed.
        reason: IntegrityReason,
    },
    /// The publication identity is already registered (HAP-001-R23).
    DuplicatePublication {
        /// The existing record, with the alias applied when one was added
        /// (boxed: a record is several hundred bytes, every other variant
        /// a few).
        existing: Box<CatalogRecord>,
    },
    /// The journal could not be read or written.
    Journal(JournalError),
    /// The Catalog could not be read.
    Catalog(CatalogStoreError),
    /// The provider could not stage or commit.
    Provider(BlobStoreError),
}

impl From<WorkAreaError> for PublishError {
    fn from(_: WorkAreaError) -> Self {
        Self::WorkAreaInvalid
    }
}

impl From<JournalError> for PublishError {
    fn from(error: JournalError) -> Self {
        Self::Journal(error)
    }
}

/// What became of the outbox entry after registration (HAP-001-R28,
/// D19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupOutcome {
    /// The entry was removed: the path still named the handle's file.
    Removed,
    /// The entry was left in place, for this fixed reason token.
    Deferred(String),
}

/// A publication that reached `registered` or `registration-pending`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    /// The record, as written (or as pending registration).
    pub record: CatalogRecord,
    /// `registered`, or `registration-pending`.
    pub state: ArtifactState,
    /// The portable reference, issued only once registered
    /// (HAP-001-R24).
    pub reference: Option<PortableReference>,
    /// What became of the outbox entry.
    pub cleanup: CleanupOutcome,
}

/// A `Read` that copies everything it yields into `sink`, so one pass over
/// the held handle both writes the published copy and digests exactly the
/// bytes written (HAP-001-R16, R17).
struct Tee<'a> {
    source: &'a mut File,
    sink: &'a mut dyn Write,
}

impl Read for Tee<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.source.read(buf)?;
        self.sink.write_all(&buf[..read])?;
        Ok(read)
    }
}

/// The link count from the handle's metadata where `std` exposes it.
#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)]
fn link_count(metadata: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt as _;
    Some(metadata.nlink())
}

#[cfg(not(unix))]
fn link_count(_metadata: &std::fs::Metadata) -> Option<u64> {
    None
}

/// Copy the held handle's bytes into a recovery entry under the work
/// area's `recovery/` directory, named by their digest (HAP-001-R18):
/// written to a temporary name first, owner-only, then renamed once the
/// digest is known. Returns the digest.
///
/// # Errors
///
/// Returns any `io::Error` the copy produces.
pub fn write_recovery_entry(
    work_area: &WorkAreaRoot,
    hasher: &dyn ContentHasher,
    handle: &mut File,
    label: &ContentDigest,
) -> std::io::Result<ContentDigest> {
    let recovery_dir = work_area.recovery_dir();
    let partial = recovery_dir.join(format!(".part-{}", label.to_hex()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut sink = options.open(&partial)?;
    handle.seek(SeekFrom::Start(0))?;
    let digest = {
        let mut tee = Tee {
            source: handle,
            sink: &mut sink,
        };
        hasher.digest_reader(&mut tee)?.0
    };
    sink.sync_data()?;
    drop(sink);
    let final_path = recovery_dir.join(digest.to_hex());
    if final_path.exists() {
        // The same bytes were already recovered: keep the existing entry.
        std::fs::remove_file(&partial)?;
    } else {
        std::fs::rename(&partial, &final_path)?;
    }
    Ok(digest)
}

/// The record for `approval` once its copy is placed at `locator`.
fn build_record(
    approval: &ArtifactApproval,
    provider: &dyn BlobStorePort,
    locator: omnifrons_domain::publication::ProviderLocator,
    at: SystemTime,
) -> CatalogRecord {
    let producer = match &approval.attribution {
        Attribution::Run(run_id) => Producer::Run {
            run_id: run_id.clone(),
            adapter_id: approval.adapter_id.clone(),
            executable_approval: approval.executable_approval,
        },
        // HAP-001-R36: the run subdirectory the entry was found under is a
        // location fact only -- the listing run's, or the one its name
        // carries when it was approved from the whole-outbox inventory.
        Attribution::Unattributed => Producer::Unattributed {
            found_under: approval.found_under(),
        },
    };
    let capabilities = provider.capabilities();
    let mut record = CatalogRecord {
        publication_id: approval.publication_id,
        catalog_id: CatalogId {
            asset_root_id: approval.asset_root_id.clone(),
            publication_id: approval.publication_id,
        },
        class: approval.class,
        detected_type: approval.detected_type,
        size: approval.size,
        digest: approval.digest,
        names: vec![approval.display_name.clone()],
        relationships: Vec::new(),
        provenance: Provenance {
            scope_id: approval.project,
            producer,
            transitions: Vec::new(),
        },
        provider: ProviderRecord {
            adapter_id: capabilities.adapter_id,
            locator: Some(locator),
            confirmation_kind: None,
            state: ProviderState::Pending,
            reason: Some(NO_CONFIRMATION_REASON.to_string()),
        },
        state: ArtifactState::Candidate,
        record_version: RECORD_VERSION,
    };
    record.transition(ArtifactState::PublishedLocal, at);
    record
}

/// What every journal entry and event of one transaction names: the
/// publication, the approval that authorized it, and the approved digest.
#[derive(Debug, Clone, Copy)]
struct Subject {
    publication_id: PublicationIdentity,
    approval_id: omnifrons_domain::publication::ArtifactApprovalId,
    digest: ContentDigest,
}

impl Subject {
    const fn of(approval: &ArtifactApproval) -> Self {
        Self {
            publication_id: approval.publication_id,
            approval_id: approval.approval_id,
            digest: approval.digest,
        }
    }

    /// The subject of a resumed publication: the journaled record and the
    /// approval id its `published-local` step carried.
    const fn of_record(
        record: &CatalogRecord,
        approval_id: omnifrons_domain::publication::ArtifactApprovalId,
    ) -> Self {
        Self {
            publication_id: record.publication_id,
            approval_id,
            digest: record.digest,
        }
    }
}

/// One transaction's mutable context: the ports, the subject, and the
/// event sink.
struct Transaction<'a, 'b> {
    ports: PublishPorts<'a>,
    subject: Subject,
    on_state: &'b mut dyn FnMut(StateEvent),
}

impl Transaction<'_, '_> {
    fn now(&self) -> SystemTime {
        self.ports.clock.now()
    }

    fn emit(&mut self, state: ArtifactState, provider_state: Option<ProviderState>) {
        (self.on_state)(StateEvent {
            publication_id: self.subject.publication_id,
            state,
            provider_state,
        });
    }

    /// Journal one step for this transaction's publication.
    fn journal(
        &mut self,
        step: JournalStep,
        outcome: StepOutcome,
        reason: Option<&str>,
        record: Option<CatalogRecord>,
        recovery: Option<ContentDigest>,
    ) -> Result<(), PublishError> {
        let entry = JournalEntry::Step(Box::new(StepEntry {
            publication_id: self.subject.publication_id,
            approval_id: self.subject.approval_id,
            step,
            outcome,
            digest: self.subject.digest,
            reason: reason.map(str::to_string),
            at: self.now(),
            record,
            recovery,
        }));
        self.ports.journal.append(&entry)?;
        Ok(())
    }

    /// A failure step: journaled, emitted, returned.
    fn fail(
        &mut self,
        step: JournalStep,
        state: ArtifactState,
        reason: &str,
        recovery: Option<ContentDigest>,
        error: PublishError,
    ) -> PublishError {
        if let Err(journal_error) =
            self.journal(step, StepOutcome::Failed, Some(reason), None, recovery)
        {
            return journal_error;
        }
        self.emit(state, None);
        error
    }

    /// HAP-001-R23: the request repeats an existing record. The display
    /// name becomes an alias when it differs; nothing else is written.
    fn duplicate(&mut self, approval: &ArtifactApproval, existing: CatalogRecord) -> PublishError {
        acknowledge_duplicate(
            &mut *self.ports.catalog,
            &mut *self.ports.journal,
            self.ports.clock,
            self.subject,
            approval,
            existing,
            &mut *self.on_state,
        )
    }

    /// Whether the copy at `record`'s locator still verifies against its
    /// digest.
    fn copy_verifies(&self, record: &CatalogRecord) -> bool {
        let Some(locator) = &record.provider.locator else {
            return false;
        };
        let Ok(mut copy) = self.ports.provider.open_published(locator) else {
            return false;
        };
        match self.ports.hasher.digest_reader(&mut *copy) {
            Ok((digest, size)) => digest == record.digest && size == record.size,
            Err(_) => false,
        }
    }

    /// Step 4 re-taken on the held handle: a regular file with a link
    /// count of one, from the handle's own metadata, never the path
    /// (HAP-001-R15, R20).
    fn verify_handle_facts(&mut self, source: &CandidateSource) -> Result<(), PublishError> {
        let metadata = source.handle.metadata().map_err(|_| {
            self.fail(
                JournalStep::OutboxEscape,
                ArtifactState::OutboxEscape,
                "handle-metadata-unreadable",
                None,
                PublishError::OutboxEscape { recovery: None },
            )
        })?;
        if !metadata.file_type().is_file() {
            return Err(self.fail(
                JournalStep::OutboxEscape,
                ArtifactState::OutboxEscape,
                "not-a-regular-file",
                None,
                PublishError::OutboxEscape { recovery: None },
            ));
        }
        if let Some(count) = link_count(&metadata)
            && count > 1
        {
            return Err(self.fail(
                JournalStep::OutboxLinked,
                ArtifactState::OutboxLinked,
                "link-count-above-one",
                None,
                PublishError::OutboxLinked { link_count: count },
            ));
        }
        Ok(())
    }

    /// HAP-001-R18: the path must still name the handle's file before a
    /// byte moves; otherwise the held bytes become a recovery entry and the
    /// outcome is `outbox-escape`. An unverifiable identity (the disclosed
    /// Windows residual) lets the copy proceed from the handle.
    fn verify_path_identity(&mut self, source: &mut CandidateSource) -> Result<(), PublishError> {
        let identity = self.ports.entry_ops.identity(
            &source.dir,
            &source.dir_path,
            &source.file_name,
            &source.handle,
        );
        let reason = match identity {
            EntryIdentity::SameFile | EntryIdentity::Unverifiable => return Ok(()),
            EntryIdentity::Missing => "path-no-longer-exists",
            EntryIdentity::DifferentFile => "path-names-a-different-file",
        };
        let recovery = write_recovery_entry(
            self.ports.work_area,
            self.ports.hasher,
            &mut source.handle,
            &self.subject.digest,
        )
        .ok();
        Err(self.fail(
            JournalStep::OutboxEscape,
            ArtifactState::OutboxEscape,
            reason,
            recovery,
            PublishError::OutboxEscape { recovery },
        ))
    }

    /// Step 7: the copy, streamed from the held handle and digested from
    /// the same bytes (HAP-001-R16, R17), then re-read and compared
    /// (HAP-001-R21). Returns the locator of a verified copy.
    fn copy_from_handle(
        &mut self,
        approval: &ArtifactApproval,
        source: &mut CandidateSource,
    ) -> Result<omnifrons_domain::publication::ProviderLocator, PublishError> {
        let mut staged = self
            .ports
            .provider
            .stage(&self.subject.publication_id)
            .map_err(PublishError::Provider)?;
        source
            .handle
            .seek(SeekFrom::Start(0))
            .map_err(|_| PublishError::Provider(BlobStoreError::ReadFailed))?;
        let streamed = {
            let mut tee = Tee {
                source: &mut source.handle,
                sink: &mut *staged,
            };
            self.ports.hasher.digest_reader(&mut tee)
        };
        let Ok((digest, size)) = streamed else {
            staged.abort();
            return Err(PublishError::Provider(BlobStoreError::WriteFailed));
        };
        if digest != approval.digest || size != approval.size {
            staged.abort();
            return Err(self.fail(
                JournalStep::IntegrityMismatch,
                ArtifactState::IntegrityMismatch,
                IntegrityReason::SourceDigestChanged.as_str(),
                None,
                PublishError::IntegrityMismatch {
                    reason: IntegrityReason::SourceDigestChanged,
                },
            ));
        }
        let locator = staged.commit().map_err(PublishError::Provider)?;

        let verified = self
            .ports
            .provider
            .open_published(&locator)
            .ok()
            .and_then(|mut copy| self.ports.hasher.digest_reader(&mut *copy).ok())
            .is_some_and(|(read_back, read_size)| {
                read_back == approval.digest && read_size == approval.size
            });
        if !verified {
            // Discard the copy; the entry is preserved. A discard failure
            // changes nothing about the outcome.
            let _ = self.ports.provider.discard(&locator);
            return Err(self.fail(
                JournalStep::IntegrityMismatch,
                ArtifactState::IntegrityMismatch,
                IntegrityReason::PublishedCopyDiffers.as_str(),
                None,
                PublishError::IntegrityMismatch {
                    reason: IntegrityReason::PublishedCopyDiffers,
                },
            ));
        }
        Ok(locator)
    }

    /// Steps 4 through 7 on the held handle: identity facts, the
    /// path-still-names-the-handle check, the copy with its digest, and
    /// the read-back. Ends with the `published-local` record journaled.
    fn publish_from_handle(
        &mut self,
        approval: &ArtifactApproval,
        source: &mut CandidateSource,
    ) -> Result<CatalogRecord, PublishError> {
        self.verify_handle_facts(source)?;
        self.verify_path_identity(source)?;
        let locator = self.copy_from_handle(approval, source)?;
        let record = build_record(approval, self.ports.provider, locator, self.now());
        self.journal(
            JournalStep::PublishedLocal,
            StepOutcome::Ok,
            None,
            Some(record.clone()),
            None,
        )?;
        self.emit(ArtifactState::PublishedLocal, None);
        Ok(record)
    }

    /// Step 8: register `record`; a failure is `registration-pending`,
    /// recoverable (HAP-001-R29), never an error.
    fn register(
        &mut self,
        mut record: CatalogRecord,
        reason: Option<&str>,
    ) -> Result<Published, PublishError> {
        // The `registered` transition is written to the Catalog and kept on
        // the returned record only once the write succeeded (R3-002): a
        // record that is `registration-pending` carries no transition to a
        // state it never reached.
        let mut registered_record = record.clone();
        registered_record.transition(ArtifactState::Registered, self.now());
        let registered = match self.ports.catalog.register(registered_record.clone()) {
            // Already there (a resume that raced a completed registration):
            // idempotent, never a second record.
            Ok(()) | Err(CatalogStoreError::Duplicate) => true,
            Err(_) => false,
        };
        if !registered {
            self.journal(
                JournalStep::RegistrationPending,
                StepOutcome::Failed,
                Some("catalog-write-failed"),
                None,
                None,
            )?;
            self.emit(ArtifactState::RegistrationPending, None);
            record.state = ArtifactState::RegistrationPending;
            return Ok(Published {
                record,
                state: ArtifactState::RegistrationPending,
                reference: None,
                cleanup: CleanupOutcome::Deferred("registration-pending".to_string()),
            });
        }
        self.journal(JournalStep::Registered, StepOutcome::Ok, reason, None, None)?;
        self.emit(
            ArtifactState::Registered,
            Some(registered_record.provider.state),
        );
        let reference = registered_record.reference();
        Ok(Published {
            record: registered_record,
            state: ArtifactState::Registered,
            reference,
            cleanup: CleanupOutcome::Deferred("not-attempted".to_string()),
        })
    }

    /// Step 10 for a source this transaction must not remove: the
    /// deferral is journaled with its fixed reason, exactly as
    /// [`Transaction::cleanup`] journals its own.
    fn defer_cleanup(&mut self, reason: &str) -> Result<CleanupOutcome, PublishError> {
        self.journal(
            JournalStep::Cleanup,
            StepOutcome::Deferred,
            Some(reason),
            None,
            None,
        )?;
        Ok(CleanupOutcome::Deferred(reason.to_string()))
    }

    /// Step 10: remove the entry only now that both steps are verified,
    /// and only while the path still names the handle's file
    /// (HAP-001-R28); otherwise defer, journaled, never silent.
    fn cleanup(&mut self, source: &CandidateSource) -> Result<CleanupOutcome, PublishError> {
        let identity = self.ports.entry_ops.identity(
            &source.dir,
            &source.dir_path,
            &source.file_name,
            &source.handle,
        );
        let outcome = match identity {
            EntryIdentity::SameFile => {
                match self
                    .ports
                    .entry_ops
                    .unlink(&source.dir, &source.dir_path, &source.file_name)
                {
                    Ok(()) => CleanupOutcome::Removed,
                    Err(_) => CleanupOutcome::Deferred("unlink-failed".to_string()),
                }
            }
            EntryIdentity::DifferentFile => {
                CleanupOutcome::Deferred("entry-changed-after-publication".to_string())
            }
            EntryIdentity::Missing => CleanupOutcome::Deferred("entry-missing".to_string()),
            EntryIdentity::Unverifiable => {
                CleanupOutcome::Deferred("identity-check-unavailable".to_string())
            }
        };
        match &outcome {
            CleanupOutcome::Removed => {
                self.journal(JournalStep::Cleanup, StepOutcome::Ok, None, None, None)?;
            }
            CleanupOutcome::Deferred(reason) => {
                let reason = reason.clone();
                self.journal(
                    JournalStep::Cleanup,
                    StepOutcome::Deferred,
                    Some(&reason),
                    None,
                    None,
                )?;
            }
        }
        Ok(outcome)
    }
}

/// HAP-001-R23's acknowledgement of an existing record: the display name
/// is added as an alias when it differs, the `duplicate-publication` step
/// is journaled, the state is emitted, and the error the transaction
/// returns for it is built. Shared by the transaction and by
/// [`acknowledge_registered`].
fn acknowledge_duplicate(
    catalog: &mut dyn CatalogStore,
    journal: &mut dyn PublicationJournal,
    clock: &dyn Clock,
    subject: Subject,
    approval: &ArtifactApproval,
    mut existing: CatalogRecord,
    on_state: &mut dyn FnMut(StateEvent),
) -> PublishError {
    if !existing.names.contains(&approval.display_name) {
        if let Err(error) =
            catalog.add_alias(&existing.publication_id, approval.display_name.clone())
        {
            return PublishError::Catalog(error);
        }
        existing.names.push(approval.display_name.clone());
    }
    let entry = JournalEntry::Step(Box::new(StepEntry {
        publication_id: subject.publication_id,
        approval_id: subject.approval_id,
        step: JournalStep::DuplicatePublication,
        outcome: StepOutcome::Ok,
        digest: subject.digest,
        reason: Some("existing-record-acknowledged".to_string()),
        at: clock.now(),
        record: None,
        recovery: None,
    }));
    if let Err(error) = journal.append(&entry) {
        return PublishError::Journal(error);
    }
    on_state(StateEvent {
        publication_id: subject.publication_id,
        state: ArtifactState::DuplicatePublication,
        provider_state: Some(existing.provider.state),
    });
    PublishError::DuplicatePublication {
        existing: Box::new(existing),
    }
}

/// HAP-001-R23 before any entry is touched: when `approval`'s publication
/// identity is already registered in `catalog`, the existing record is
/// acknowledged exactly as the transaction would acknowledge it -- alias,
/// journal step, `duplicate-publication` event -- and the error the
/// transaction would return comes back as `Some`; `None` when nothing is
/// registered yet. A caller runs this before acquiring a handle, so a
/// repeat of a completed publication is a duplicate even when its outbox
/// entry is long gone.
///
/// # Errors
///
/// Returns [`PublishError::Catalog`] if the Catalog cannot be read.
pub fn acknowledge_registered(
    catalog: &mut dyn CatalogStore,
    journal: &mut dyn PublicationJournal,
    clock: &dyn Clock,
    approval: &ArtifactApproval,
    on_state: &mut dyn FnMut(StateEvent),
) -> Result<Option<PublishError>, PublishError> {
    let existing = catalog
        .find(&approval.publication_id)
        .map_err(PublishError::Catalog)?;
    match existing {
        None => Ok(None),
        Some(existing) => Ok(Some(acknowledge_duplicate(
            catalog,
            journal,
            clock,
            Subject::of(approval),
            approval,
            existing,
            on_state,
        ))),
    }
}

/// Run the publication transaction for `approval` over `source`.
///
/// # Errors
///
/// Returns [`PublishError`] for every way the transaction ends with
/// nothing registered; a registration failure after a verified
/// `published-local` is not an error but an `Ok` whose `state` is
/// `registration-pending`.
pub fn publish(
    ports: PublishPorts<'_>,
    approval: &ArtifactApproval,
    mut source: CandidateSource,
    on_state: &mut dyn FnMut(StateEvent),
) -> Result<Published, PublishError> {
    // HAP-001-R7: the work area is re-checked before it is used for
    // anything -- before the journal is even read.
    ports.work_area.check(ports.workspaces)?;
    let entries = ports.journal.replay()?;
    let mut transaction = Transaction {
        ports,
        subject: Subject::of(approval),
        on_state,
    };

    // HAP-001-R23, R29: is this a duplicate, a resume, or a fresh start?
    let existing = transaction
        .ports
        .catalog
        .find(&approval.publication_id)
        .map_err(PublishError::Catalog)?;
    if let Some(existing) = existing {
        return Err(transaction.duplicate(approval, existing));
    }
    let record = match resume_point(&entries, &approval.publication_id) {
        ResumePoint::PublishedLocal(record) if transaction.copy_verifies(&record) => *record,
        // `Registered` in the journal but absent from the Catalog (the
        // project's records were reset), or a copy that no longer
        // verifies: publish again from the held handle.
        ResumePoint::Registered | ResumePoint::PublishedLocal(_) | ResumePoint::NotStarted => {
            transaction.publish_from_handle(approval, &mut source)?
        }
    };

    let mut published = transaction.register(record, None)?;
    if published.state == ArtifactState::Registered {
        published.cleanup = if approval.source == ApprovalSource::Outbox {
            transaction.cleanup(&source)?
        } else {
            // HAP-001-R18, R28: a recovery entry is the preserved copy of
            // what was digested and approved. Registering those bytes does
            // not make the preserved copy disposable, and this contract
            // defines no deletion path for one -- HAP-001-R39 makes
            // MRP-001's tombstone the only way a registered artifact goes
            // away. Deferred, journaled, never silent.
            transaction.defer_cleanup(RECOVERY_ENTRY_RETAINED)?
        };
    }
    Ok(published)
}

/// On start, resume every publication of `project` whose journal shows a
/// verified `published-local` without a registration (HAP-001-R29): the
/// copy is re-verified and the record the journal carries is registered;
/// no second copy is made and no completed step is repeated. The outbox
/// entry is left in place -- no handle survives a restart, so its removal
/// is deferred and journaled, never silent.
///
/// # Errors
///
/// Returns [`PublishError::WorkAreaInvalid`] if the work area fails its
/// re-check, or [`PublishError::Journal`] if the journal cannot be read.
pub fn recover(
    ports: PublishPorts<'_>,
    project: &ProjectIdentity,
    on_state: &mut dyn FnMut(StateEvent),
) -> Result<Vec<Published>, PublishError> {
    ports.work_area.check(ports.workspaces)?;
    let entries = ports.journal.replay()?;
    let pending: Vec<_> = crate::publication_journal::pending_registrations(&entries)
        .into_iter()
        .filter(|(record, _)| record.provenance.scope_id == *project)
        .collect();
    let Some((first, first_approval)) = pending.first() else {
        return Ok(Vec::new());
    };
    // One transaction context for every resumed publication: the ports
    // move in once, and the subject is set per publication below.
    let mut transaction = Transaction {
        ports,
        subject: Subject::of_record(first, *first_approval),
        on_state,
    };
    let mut resumed = Vec::new();
    for (record, approval_id) in pending {
        transaction.subject = Subject::of_record(&record, approval_id);
        if !transaction.copy_verifies(&record) {
            // The retained entry needs a fresh handle and a fresh approval
            // (HAP-001-R29); recorded, not repaired here.
            transaction.journal(
                JournalStep::IntegrityMismatch,
                StepOutcome::Failed,
                Some("published-copy-unverifiable-after-restart"),
                None,
                None,
            )?;
            transaction.emit(ArtifactState::IntegrityMismatch, None);
            continue;
        }
        let mut published = transaction.register(record, Some("resumed-after-restart"))?;
        if published.state == ArtifactState::Registered {
            transaction.journal(
                JournalStep::Cleanup,
                StepOutcome::Deferred,
                Some("no-handle-after-restart"),
                None,
                None,
            )?;
            published.cleanup = CleanupOutcome::Deferred("no-handle-after-restart".to_string());
        }
        resumed.push(published);
    }
    Ok(resumed)
}

/// Journal a failure decided before the transaction could start (a
/// re-opened entry whose facts changed, an entry that cannot be approved
/// in its state): the same step shape as the transaction's own failures.
///
/// # Errors
///
/// Returns [`JournalError`] if the write did not complete.
pub fn journal_failure(
    journal: &mut dyn PublicationJournal,
    approval: &ArtifactApproval,
    step: JournalStep,
    reason: &str,
    at: SystemTime,
) -> Result<(), JournalError> {
    journal.append(&JournalEntry::Step(Box::new(StepEntry {
        publication_id: approval.publication_id,
        approval_id: approval.approval_id,
        step,
        outcome: StepOutcome::Failed,
        digest: approval.digest,
        reason: Some(reason.to_string()),
        at,
        record: None,
        recovery: None,
    })))
}

/// The path of the recovery entry named by `digest` under `work_area`,
/// for a caller that lists or removes recovery entries. Device-local.
#[must_use]
pub fn recovery_entry_path(work_area: &WorkAreaRoot, digest: &ContentDigest) -> PathBuf {
    work_area.recovery_dir().join(digest.to_hex())
}

/// Whether `path` names an existing recovery entry.
#[must_use]
pub fn recovery_entry_exists(path: &Path) -> bool {
    path.is_file()
}
