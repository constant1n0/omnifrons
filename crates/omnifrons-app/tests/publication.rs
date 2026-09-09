//! The publication transaction (spike slice 5b, HAP-001 § Publication
//! transaction) against the in-memory fakes: the fixed step order, the
//! bytes taken from the held handle (never the path), the recovery entry
//! on an identity mismatch, the published copy re-verified, idempotent
//! retry, registration resumed from the journal after an interruption,
//! the entry removed only after both publication and registration, and the
//! `artifact.state` transitions in order.
//!
//! Only compiled with `--features contract-tests`: the fakes live behind
//! that feature.

#![cfg(feature = "contract-tests")]

use std::ffi::OsString;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::blob_store::BlobStorePort;
use omnifrons_app::catalog_store::CatalogStore;
use omnifrons_app::content_hasher::{
    ContentHasher, derive_artifact_approval_id, derive_publication_identity,
};
use omnifrons_app::contract::approval_store::FixedClock;
use omnifrons_app::contract::publication::{
    FakeHasher, InMemoryBlobStore, InMemoryCatalogStore, InMemoryJournal, ScriptedEntryOps,
};
use omnifrons_app::outbox_entry_ops::EntryIdentity;
use omnifrons_app::publication::{
    CandidateSource, CleanupOutcome, PublishError, PublishPorts, StateEvent, publish, recover,
};
use omnifrons_app::publication_journal::{JournalError, PublicationJournal};
use omnifrons_app::run_outbox::DirectoryHandle;
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::executable::{DeviceLocalUser, Sha256Digest};
use omnifrons_domain::outbox::{ArtifactClass, Attribution, DetectedType, RunId};
use omnifrons_domain::publication::{
    ArtifactApproval, ArtifactState, AssetRootId, DisplayName, JournalEntry, JournalStep,
    PortableReference, Producer, ProjectIdentity, ProviderState, RECORD_VERSION, StepOutcome,
};

/// Open a directory handle for a fixture. Windows refuses a plain
/// `File::open` on a directory: `FILE_FLAG_BACKUP_SEMANTICS` is required.
#[cfg(unix)]
fn open_directory(path: &Path) -> File {
    File::open(path).expect("open the directory")
}

#[cfg(windows)]
fn open_directory(path: &Path) -> File {
    use std::os::windows::fs::OpenOptionsExt as _;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .expect("open the directory")
}

/// A drop-guard temp directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-publication-test-{}-{label}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("fixture dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn workspace(&self) -> WorkspaceRoot {
        WorkspaceRoot::new(&self.0).expect("valid workspace")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const PDF: &[u8] = b"%PDF-1.7\nomnifrons fake artifact\n";

/// The fixture: a project with a run subdirectory holding one PDF, a
/// work area beside it, and the ports.
struct Fixture {
    project: TempDir,
    work_base: TempDir,
    work_area: WorkAreaRoot,
    run_dir: PathBuf,
    hasher: FakeHasher,
    provider: InMemoryBlobStore,
    catalog: InMemoryCatalogStore,
    journal: InMemoryJournal,
    clock: FixedClock,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let project = TempDir::new(label);
        let work_base = TempDir::new(&format!("{label}-work"));
        let work_area = WorkAreaRoot::open(&work_base.path().join("wa"), &[&project.workspace()])
            .expect("the work area opens outside the project");
        let run_dir = project.path().join(".omnifrons/outbox/run-1");
        std::fs::create_dir_all(&run_dir).expect("run dir");
        std::fs::write(run_dir.join("report.pdf"), PDF).expect("fixture file");
        Self {
            project,
            work_base,
            work_area,
            run_dir,
            hasher: FakeHasher,
            provider: InMemoryBlobStore::new("fake"),
            catalog: InMemoryCatalogStore::new(),
            journal: InMemoryJournal::new(),
            clock: FixedClock::new(SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_401)),
        }
    }

    fn workspace(&self) -> WorkspaceRoot {
        self.project.workspace()
    }

    /// Open the held handle for `name` in the run subdirectory: the one
    /// handle every fact and every published byte comes from.
    fn source(&self, name: &str) -> CandidateSource {
        let dir = open_directory(&self.run_dir);
        let handle = File::open(self.run_dir.join(name)).expect("open the entry");
        CandidateSource {
            dir: DirectoryHandle::new(dir),
            dir_path: self.run_dir.clone(),
            file_name: OsString::from(name),
            handle,
        }
    }

    fn digest_of(&self, bytes: &[u8]) -> Sha256Digest {
        self.hasher.sha256(bytes)
    }

    /// An explicit per-artifact approval for `name` with `bytes` as the
    /// approved content (HAP-001-R22).
    fn approval(&self, name: &str, bytes: &[u8]) -> ArtifactApproval {
        let project = ProjectIdentity(Sha256Digest([7; 32]));
        let digest = self.digest_of(bytes);
        let publication_id = derive_publication_identity(&self.hasher, &project, &digest);
        let approved_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_399);
        let run_id = RunId::new("run-1").expect("valid");
        ArtifactApproval {
            approval_id: derive_artifact_approval_id(&self.hasher, &publication_id, approved_at),
            publication_id,
            project,
            run_id: run_id.clone(),
            name: format!("run-1/{name}"),
            display_name: DisplayName::sanitize(name),
            digest,
            size: bytes.len() as u64,
            detected_type: DetectedType::Pdf,
            class: ArtifactClass::GeneratedHeavy,
            attribution: Attribution::Run(run_id),
            asset_root_id: AssetRootId::new("main").expect("valid"),
            adapter_id: None,
            executable_approval: None,
            approver: DeviceLocalUser,
            approved_at,
        }
    }

    fn ports<'a>(
        &'a mut self,
        entry_ops: &'a ScriptedEntryOps,
        workspaces: &'a [&'a WorkspaceRoot],
    ) -> PublishPorts<'a> {
        PublishPorts {
            hasher: &self.hasher,
            provider: &self.provider,
            catalog: &mut self.catalog,
            journal: &mut self.journal,
            entry_ops,
            clock: &self.clock,
            work_area: &self.work_area,
            workspaces,
        }
    }

    fn steps(&self) -> Vec<(JournalStep, StepOutcome)> {
        self.journal
            .replay()
            .expect("in-memory replay")
            .into_iter()
            .filter_map(|entry| match entry {
                JournalEntry::Step(step) => Some((step.step, step.outcome)),
                JournalEntry::Approved(_) => None,
            })
            .collect()
    }
}

fn states(events: &[StateEvent]) -> Vec<ArtifactState> {
    events.iter().map(|event| event.state).collect()
}

/// The happy path: approved bytes copied from the held handle into the
/// provider, re-verified, registered, the reference issued only then, the
/// entry removed only after both, and the transitions in HAP-001's order.
#[test]
fn a_publication_copies_verifies_registers_and_removes_the_entry_in_order() {
    let mut fixture = Fixture::new("happy");
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    let entry_ops = ScriptedEntryOps::always(EntryIdentity::SameFile);
    let workspace = fixture.workspace();
    let mut events = Vec::new();

    let published = publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        source,
        &mut |event| events.push(event),
    )
    .expect("the publication succeeds");

    assert_eq!(published.state, ArtifactState::Registered);
    let record = &published.record;
    assert_eq!(record.publication_id, approval.publication_id);
    assert_eq!(
        record.catalog_id.to_string(),
        format!("main/{}", approval.publication_id)
    );
    assert_eq!(record.digest, approval.digest);
    assert_eq!(record.size, PDF.len() as u64);
    assert_eq!(record.class, ArtifactClass::GeneratedHeavy);
    assert_eq!(record.detected_type, DetectedType::Pdf);
    assert_eq!(record.names, vec![DisplayName::sanitize("report.pdf")]);
    assert_eq!(record.provider.adapter_id, "fake");
    assert_eq!(record.provider.state, ProviderState::Pending);
    assert_eq!(record.record_version, RECORD_VERSION);
    assert_eq!(
        record.provenance.producer,
        Producer::Run {
            run_id: RunId::new("run-1").expect("valid"),
            adapter_id: None,
            executable_approval: None,
        }
    );
    assert_eq!(
        record
            .provenance
            .transitions
            .iter()
            .map(|transition| transition.state)
            .collect::<Vec<_>>(),
        vec![ArtifactState::PublishedLocal, ArtifactState::Registered]
    );
    assert_eq!(
        published.reference,
        Some(PortableReference::new(record.catalog_id.clone()))
    );
    assert_eq!(published.cleanup, CleanupOutcome::Removed);

    // The provider holds exactly the approved bytes, once.
    assert_eq!(fixture.provider.stage_count(), 1);
    let locator = record
        .provider
        .locator
        .clone()
        .expect("a locator once placed");
    let mut copy = Vec::new();
    fixture
        .provider
        .open_published(&locator)
        .expect("the copy exists")
        .read_to_end(&mut copy)
        .expect("read");
    assert_eq!(copy, PDF);

    // The Catalog holds one record; the entry is gone from the outbox.
    assert_eq!(fixture.catalog.list().expect("list").len(), 1);
    assert_eq!(entry_ops.unlinked(), vec![OsString::from("report.pdf")]);

    // HAP-001-R29: one journal entry per step, in order.
    assert_eq!(
        fixture.steps(),
        vec![
            (JournalStep::PublishedLocal, StepOutcome::Ok),
            (JournalStep::Registered, StepOutcome::Ok),
            (JournalStep::Cleanup, StepOutcome::Ok),
        ]
    );

    // HAP-001-R35: `artifact.state` on every transition, in order.
    assert_eq!(
        states(&events),
        vec![ArtifactState::PublishedLocal, ArtifactState::Registered]
    );
    assert_eq!(events[0].provider_state, None);
    assert_eq!(events[1].provider_state, Some(ProviderState::Pending));
    assert!(
        events
            .iter()
            .all(|event| event.publication_id == approval.publication_id)
    );
}

/// HAP-001-R17: the published bytes come from the held handle. With the
/// identity check unverifiable (the Windows residual, scripted here), a
/// file swapped at the path after approval changes nothing: the copy still
/// equals the approved digest, because the path is never re-read.
#[test]
fn the_copy_comes_from_the_held_handle_even_when_the_path_was_swapped() {
    let mut fixture = Fixture::new("swap-unverifiable");
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    // Swap: a different file now sits at the path the handle was opened at.
    std::fs::remove_file(fixture.run_dir.join("report.pdf")).expect("unlink");
    std::fs::write(
        fixture.run_dir.join("report.pdf"),
        b"%PDF-1.7\nsomething else\n",
    )
    .expect("swap in");
    let entry_ops = ScriptedEntryOps::always(EntryIdentity::Unverifiable);
    let workspace = fixture.workspace();

    let published = publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        source,
        &mut |_| {},
    )
    .expect("the publication succeeds from the handle");

    assert_eq!(published.record.digest, approval.digest);
    let locator = published.record.provider.locator.expect("placed");
    let mut copy = Vec::new();
    fixture
        .provider
        .open_published(&locator)
        .expect("copy")
        .read_to_end(&mut copy)
        .expect("read");
    assert_eq!(copy, PDF, "the approved bytes, never the swapped file's");
    // Without an identity check the entry is never unlinked: cleanup is
    // deferred and says why.
    assert_eq!(
        published.cleanup,
        CleanupOutcome::Deferred("identity-check-unavailable".to_string())
    );
    assert!(entry_ops.unlinked().is_empty());
}

/// HAP-001-R18: when the path no longer names the handle's file at
/// publish time, the outcome is `outbox-escape`, nothing is published, and
/// the held handle's bytes survive as a recovery entry in the work area,
/// named by their digest.
#[test]
fn a_path_that_no_longer_names_the_handle_is_outbox_escape_with_a_recovery_entry() {
    let mut fixture = Fixture::new("escape");
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    let entry_ops = ScriptedEntryOps::always(EntryIdentity::DifferentFile);
    let workspace = fixture.workspace();
    let mut events = Vec::new();

    let error = publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        source,
        &mut |event| events.push(event),
    )
    .expect_err("an identity mismatch ends the transaction");

    let recovery = match error {
        PublishError::OutboxEscape { recovery } => recovery.expect("a recovery entry was written"),
        other => panic!("expected outbox-escape, got {other:?}"),
    };
    assert_eq!(recovery, approval.digest);
    let recovered = fixture.work_area.recovery_dir().join(recovery.to_hex());
    assert_eq!(std::fs::read(&recovered).expect("recovery entry"), PDF);
    assert_eq!(fixture.provider.stage_count(), 0, "nothing was published");
    assert!(fixture.catalog.list().expect("list").is_empty());
    assert!(entry_ops.unlinked().is_empty());
    assert_eq!(states(&events), vec![ArtifactState::OutboxEscape]);
    assert_eq!(
        fixture.steps(),
        vec![(JournalStep::OutboxEscape, StepOutcome::Failed)]
    );
}

/// A recovery entry is owner-only on unix (the work area's discipline).
#[cfg(unix)]
#[test]
fn a_recovery_entry_is_owner_only_on_unix() {
    use std::os::unix::fs::PermissionsExt as _;
    let mut fixture = Fixture::new("escape-mode");
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    let entry_ops = ScriptedEntryOps::always(EntryIdentity::Missing);
    let workspace = fixture.workspace();
    let error = publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        source,
        &mut |_| {},
    )
    .expect_err("missing at the path");
    let PublishError::OutboxEscape {
        recovery: Some(digest),
    } = error
    else {
        panic!("expected outbox-escape with a recovery entry, got {error:?}");
    };
    let mode = std::fs::metadata(fixture.work_area.recovery_dir().join(digest.to_hex()))
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

/// HAP-001-R21: the published copy is re-read and its digest compared;
/// a copy the provider corrupted renders `integrity-mismatch`, the copy is
/// discarded, no record is written, and the entry is preserved.
#[test]
fn a_corrupted_published_copy_is_integrity_mismatch_and_discarded() {
    let mut fixture = Fixture::new("corrupt");
    fixture.provider.corrupt_next_commit();
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    let entry_ops = ScriptedEntryOps::always(EntryIdentity::SameFile);
    let workspace = fixture.workspace();
    let mut events = Vec::new();

    let error = publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        source,
        &mut |event| events.push(event),
    )
    .expect_err("a corrupt copy ends the transaction");

    assert!(
        matches!(error, PublishError::IntegrityMismatch { .. }),
        "got {error:?}"
    );
    assert_eq!(
        fixture.provider.discarded().len(),
        1,
        "the copy is discarded"
    );
    assert!(
        fixture.provider.objects().is_empty(),
        "nothing remains in the provider"
    );
    assert!(
        fixture.catalog.list().expect("list").is_empty(),
        "no record"
    );
    assert!(entry_ops.unlinked().is_empty(), "the entry is preserved");
    assert!(fixture.run_dir.join("report.pdf").is_file());
    assert_eq!(states(&events), vec![ArtifactState::IntegrityMismatch]);
    assert_eq!(
        fixture.steps(),
        vec![(JournalStep::IntegrityMismatch, StepOutcome::Failed)]
    );
}

/// The digest is re-verified on the bytes leaving the held handle: a file
/// rewritten in place after approval (same inode, new content) renders
/// `integrity-mismatch` before anything is committed.
#[test]
fn a_source_rewritten_in_place_after_approval_is_integrity_mismatch() {
    let mut fixture = Fixture::new("rewritten");
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    {
        // Rewrite in place through a second handle: the inode is the same,
        // the bytes are not.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(fixture.run_dir.join("report.pdf"))
            .expect("reopen for writing");
        file.write_all(b"%PDF-1.7\nrewritten\n").expect("rewrite");
    }
    let entry_ops = ScriptedEntryOps::always(EntryIdentity::SameFile);
    let workspace = fixture.workspace();

    let error = publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        source,
        &mut |_| {},
    )
    .expect_err("the rewritten source ends the transaction");

    assert!(
        matches!(error, PublishError::IntegrityMismatch { .. }),
        "got {error:?}"
    );
    assert_eq!(
        fixture.provider.aborted(),
        1,
        "the staged copy is aborted, never committed"
    );
    assert!(fixture.provider.objects().is_empty());
    assert!(fixture.catalog.list().expect("list").is_empty());
}

/// HAP-001-R23: a second request for the same publication identity is
/// `duplicate-publication`: the existing record is acknowledged, the new
/// display name becomes an alias, and neither a second copy nor a second
/// record is written.
#[test]
fn a_repeated_publication_is_duplicate_publication_with_an_alias_and_no_second_copy() {
    let mut fixture = Fixture::new("duplicate");
    let approval = fixture.approval("report.pdf", PDF);
    let entry_ops = ScriptedEntryOps::always(EntryIdentity::SameFile);
    let workspace = fixture.workspace();
    let source = fixture.source("report.pdf");
    publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        source,
        &mut |_| {},
    )
    .expect("first publication");

    // The same bytes again under another name (a second run, say).
    std::fs::write(fixture.run_dir.join("report-copy.pdf"), PDF).expect("second file");
    let mut again = fixture.approval("report-copy.pdf", PDF);
    again.approved_at += Duration::from_secs(1);
    again.approval_id =
        derive_artifact_approval_id(&fixture.hasher, &again.publication_id, again.approved_at);
    let source = fixture.source("report-copy.pdf");
    let mut events = Vec::new();
    let error = publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &again,
        source,
        &mut |event| events.push(event),
    )
    .expect_err("a duplicate ends the transaction with nothing new published");

    let existing = match error {
        PublishError::DuplicatePublication { existing } => existing,
        other => panic!("expected duplicate-publication, got {other:?}"),
    };
    assert_eq!(existing.publication_id, approval.publication_id);
    assert_eq!(fixture.provider.stage_count(), 1, "no second copy");
    let records = fixture.catalog.list().expect("list");
    assert_eq!(records.len(), 1, "no second record");
    assert_eq!(
        records[0].names,
        vec![
            DisplayName::sanitize("report.pdf"),
            DisplayName::sanitize("report-copy.pdf")
        ],
        "the new display name is an alias"
    );
    assert_eq!(states(&events), vec![ArtifactState::DuplicatePublication]);
    assert_eq!(events[0].provider_state, Some(ProviderState::Pending));
    assert_eq!(
        entry_ops.unlinked(),
        vec![OsString::from("report.pdf")],
        "the duplicate's entry is not removed by this slice"
    );
}

/// HAP-001-R29: a failure between `published-local` and registration
/// leaves `registration-pending` with no reference; `recover` replays the
/// journal and registers from the record it carries -- one copy, one
/// record, no second publish.
#[test]
fn registration_resumes_from_the_journal_after_a_failure_with_no_second_copy() {
    let mut fixture = Fixture::new("resume");
    fixture.catalog.fail_next_register();
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    let entry_ops = ScriptedEntryOps::always(EntryIdentity::SameFile);
    let workspace = fixture.workspace();
    let mut events = Vec::new();

    let pending = publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        source,
        &mut |event| events.push(event),
    )
    .expect("a registration failure is a recoverable state, not an error");
    assert_eq!(pending.state, ArtifactState::RegistrationPending);
    assert_eq!(pending.reference, None, "no reference before registration");
    assert_eq!(
        pending.cleanup,
        CleanupOutcome::Deferred("registration-pending".to_string()),
        "the entry stays until both steps are verified (HAP-001-R28)"
    );
    assert!(entry_ops.unlinked().is_empty());
    assert_eq!(
        states(&events),
        vec![
            ArtifactState::PublishedLocal,
            ArtifactState::RegistrationPending
        ]
    );
    assert_eq!(
        fixture.steps(),
        vec![
            (JournalStep::PublishedLocal, StepOutcome::Ok),
            (JournalStep::RegistrationPending, StepOutcome::Failed),
        ]
    );

    // "Restart": replay the journal and resume.
    let mut recovered_events = Vec::new();
    let recovered = recover(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval.project,
        &mut |event| recovered_events.push(event),
    )
    .expect("recovery runs");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].state, ArtifactState::Registered);
    assert!(recovered[0].reference.is_some());
    assert_eq!(
        recovered[0].cleanup,
        CleanupOutcome::Deferred("no-handle-after-restart".to_string())
    );
    assert_eq!(fixture.provider.stage_count(), 1, "no second copy");
    assert_eq!(
        fixture.catalog.list().expect("list").len(),
        1,
        "exactly one record"
    );
    assert_eq!(states(&recovered_events), vec![ArtifactState::Registered]);

    // A second recovery finds nothing left to resume.
    let again = recover(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval.project,
        &mut |_| {},
    )
    .expect("recovery runs");
    assert!(again.is_empty(), "a completed step is never repeated");
}

/// `recover` resumes only the active project's publications: another
/// project's pending registration is left alone.
#[test]
fn recovery_leaves_another_projects_pending_publication_alone() {
    let mut fixture = Fixture::new("resume-other");
    fixture.catalog.fail_next_register();
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    let entry_ops = ScriptedEntryOps::always(EntryIdentity::SameFile);
    let workspace = fixture.workspace();
    publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        source,
        &mut |_| {},
    )
    .expect("pending");
    let other = ProjectIdentity(Sha256Digest([0xee; 32]));
    let recovered = recover(
        fixture.ports(&entry_ops, &[&workspace]),
        &other,
        &mut |_| {},
    )
    .expect("recovery runs");
    assert!(recovered.is_empty());
    assert!(fixture.catalog.list().expect("list").is_empty());
}

/// HAP-001-R28: the entry is removed only after publication *and*
/// registration are verified, and only while the path still names the
/// handle's file: an entry swapped after registration is left in place
/// and the deferral is journaled, never silent.
#[test]
fn cleanup_is_deferred_when_the_entry_changed_after_registration() {
    let mut fixture = Fixture::new("cleanup-deferred");
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    // Same file at publish time, a different one when cleanup looks.
    let entry_ops =
        ScriptedEntryOps::scripted([EntryIdentity::SameFile, EntryIdentity::DifferentFile]);
    let workspace = fixture.workspace();

    let published = publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        source,
        &mut |_| {},
    )
    .expect("registered");
    assert_eq!(published.state, ArtifactState::Registered);
    assert_eq!(
        published.cleanup,
        CleanupOutcome::Deferred("entry-changed-after-publication".to_string())
    );
    assert!(entry_ops.unlinked().is_empty());
    assert_eq!(
        fixture.steps().last(),
        Some(&(JournalStep::Cleanup, StepOutcome::Deferred))
    );
}

/// HAP-001-R7: the work area is re-checked at every use -- a workspace
/// registered over it makes the publication refuse with
/// `work-area-invalid` before any journal write, and nothing is published.
#[test]
fn a_work_area_inside_a_registered_workspace_refuses_the_publication() {
    let mut fixture = Fixture::new("work-area-invalid");
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    let entry_ops = ScriptedEntryOps::always(EntryIdentity::SameFile);
    let workspace = fixture.workspace();
    let over = WorkspaceRoot::new(fixture.work_base.path()).expect("valid workspace");

    let error = publish(
        fixture.ports(&entry_ops, &[&workspace, &over]),
        &approval,
        source,
        &mut |_| {},
    )
    .expect_err("refused");
    assert!(
        matches!(error, PublishError::WorkAreaInvalid),
        "got {error:?}"
    );
    assert_eq!(fixture.provider.stage_count(), 0);
    assert!(
        fixture.journal.replay().expect("replay").is_empty(),
        "nothing journaled"
    );
}

/// HAP-001-R20 at publish time: a handle whose link count rose above one
/// since validation is refused as `outbox-linked`, never
/// `integrity-mismatch`.
#[cfg(unix)] // a hard link fixture; the link count is not read on Windows (disclosed)
#[test]
fn a_hard_link_added_after_approval_is_outbox_linked() {
    let mut fixture = Fixture::new("linked");
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    std::fs::hard_link(
        fixture.run_dir.join("report.pdf"),
        fixture.run_dir.join("report-link.pdf"),
    )
    .expect("hard link");
    let entry_ops = ScriptedEntryOps::always(EntryIdentity::SameFile);
    let workspace = fixture.workspace();
    let mut events = Vec::new();
    let error = publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        source,
        &mut |event| events.push(event),
    )
    .expect_err("refused");
    assert!(
        matches!(error, PublishError::OutboxLinked { link_count: 2 }),
        "got {error:?}"
    );
    assert_eq!(fixture.provider.stage_count(), 0);
    assert_eq!(states(&events), vec![ArtifactState::OutboxLinked]);
}

/// R3-002: a registration that fails leaves no `registered` transition on
/// the returned record -- the record says `registration-pending` and its
/// provenance ends at `published-local`.
#[test]
fn a_failed_registration_leaves_no_registered_transition_on_the_record() {
    let mut fixture = Fixture::new("pending-record");
    fixture.catalog.fail_next_register();
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    let entry_ops = ScriptedEntryOps::always(EntryIdentity::SameFile);
    let workspace = fixture.workspace();
    let pending = publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        source,
        &mut |_| {},
    )
    .expect("a recoverable state");
    assert_eq!(pending.state, ArtifactState::RegistrationPending);
    assert_eq!(pending.record.state, ArtifactState::RegistrationPending);
    assert_eq!(
        pending
            .record
            .provenance
            .transitions
            .iter()
            .map(|transition| transition.state)
            .collect::<Vec<_>>(),
        vec![ArtifactState::PublishedLocal],
        "no transition to a state that was never reached"
    );
}

/// R3-003: the crash window between `commit` and the `published-local`
/// journal line. The copy exists, nothing is journaled, and a retry with a
/// fresh handle stages the same identity again -- the store replaces the
/// copy, so one object and one record result, never two.
#[test]
fn a_crash_between_commit_and_the_journal_line_is_recovered_by_a_retry_with_one_copy() {
    let mut fixture = Fixture::new("crash-after-commit");
    fixture.journal.fail_next_append();
    let approval = fixture.approval("report.pdf", PDF);
    let source = fixture.source("report.pdf");
    let entry_ops = ScriptedEntryOps::always(EntryIdentity::SameFile);
    let workspace = fixture.workspace();

    let error = publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        source,
        &mut |_| {},
    )
    .expect_err("the journal write after the commit failed");
    assert!(
        matches!(error, PublishError::Journal(JournalError::WriteFailed)),
        "got {error:?}"
    );
    assert_eq!(fixture.provider.stage_count(), 1);
    assert_eq!(
        fixture.provider.objects().len(),
        1,
        "the copy was committed before the journal line"
    );
    assert!(fixture.catalog.list().expect("list").is_empty());
    assert!(
        fixture.journal.replay().expect("replay").is_empty(),
        "nothing journaled: the resume point is the start"
    );

    let retry_source = fixture.source("report.pdf");
    let retried = publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        retry_source,
        &mut |_| {},
    )
    .expect("the retry registers");
    assert_eq!(retried.state, ArtifactState::Registered);
    assert_eq!(
        fixture.provider.stage_count(),
        2,
        "the same identity is staged again"
    );
    assert_eq!(
        fixture.provider.objects().len(),
        1,
        "the re-staged copy replaced the orphan: one object"
    );
    assert_eq!(fixture.catalog.list().expect("list").len(), 1, "one record");
}

/// R3-007: a held handle that is not a regular file -- a directory here --
/// is refused as `outbox-escape` before any byte moves, with no recovery
/// entry (there are no regular bytes to preserve).
#[test]
fn a_held_handle_that_is_not_a_regular_file_is_outbox_escape_with_nothing_staged() {
    let mut fixture = Fixture::new("not-regular");
    let approval = fixture.approval("report.pdf", PDF);
    let mut source = fixture.source("report.pdf");
    source.handle = open_directory(&fixture.run_dir);
    let entry_ops = ScriptedEntryOps::always(EntryIdentity::SameFile);
    let workspace = fixture.workspace();
    let mut events = Vec::new();
    let error = publish(
        fixture.ports(&entry_ops, &[&workspace]),
        &approval,
        source,
        &mut |event| events.push(event),
    )
    .expect_err("refused");
    assert!(
        matches!(error, PublishError::OutboxEscape { recovery: None }),
        "got {error:?}"
    );
    assert_eq!(fixture.provider.stage_count(), 0);
    assert_eq!(states(&events), vec![ArtifactState::OutboxEscape]);
    assert_eq!(
        fixture.steps(),
        vec![(JournalStep::OutboxEscape, StepOutcome::Failed)]
    );
    assert!(
        std::fs::read_dir(fixture.work_area.recovery_dir())
            .expect("list")
            .next()
            .is_none(),
        "no recovery entry"
    );
}
