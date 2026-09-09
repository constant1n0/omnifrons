//! The publication commands (spike slice 5b, HAP-001 § Publication
//! transaction): `artifact_approve`, `artifact_publish`, and
//! `publications_list`, registered next to the outbox commands. Each is a
//! thin wrapper over a pure body tested here without a Tauri runtime --
//! [`approve_candidate`], [`publish_approved`], [`publications_for`] --
//! that composes the application transaction (`omnifrons_app::publication`)
//! with the real adapters: the JSONL journal under the product work area,
//! the JSONL Catalog under the project's `.omnifrons/`, the dev-mode
//! local-directory provider under the device's asset roots, SHA-256, and
//! the by-handle entry ops.
//!
//! The device paths never cross IPC (HAP-001-R5): a command returns
//! logical identities, states, and the portable reference only.

use std::ffi::{OsStr, OsString};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use omnifrons_adapters::{
    FsCandidateProber, FsOutboxEntryOps, JsonlCatalogStore, JsonlPublicationJournal,
    LocalDirBlobStore, Sha2Hasher,
};
use omnifrons_app::blob_store::{BlobStoreError, DestinationError, DeviceAssetPath};
use omnifrons_app::catalog_store::{CatalogStore as _, CatalogStoreError};
use omnifrons_app::content_hasher::{
    derive_artifact_approval_id, derive_project_identity, derive_publication_identity,
};
use omnifrons_app::outbox_policy::OutboxPolicyStore as _;
use omnifrons_app::publication::{
    CandidateSource, PublishError, PublishPorts, Published, StateEvent, acknowledge_registered,
    journal_failure, publish, recover,
};
use omnifrons_app::publication_journal::{
    JournalError, PublicationJournal as _, find_approval, pending_registrations,
};
use omnifrons_app::run_outbox::{
    CandidateProbe, CandidateProber as _, OutboxInventory as _, RunOutboxPreparer as _,
};
use omnifrons_app::work_area::{WorkAreaError, WorkAreaRoot};
use omnifrons_domain::executable::{DeviceLocalUser, Sha256Digest};
use omnifrons_domain::outbox::{ArtifactClass, CandidateState, RunId};
use omnifrons_domain::publication::{
    ArtifactApproval, ArtifactApprovalId, ArtifactState, DisplayName, JournalEntry, JournalStep,
    ProjectIdentity, StepOutcome,
};
use omnifrons_supervisor::TokioProcessSupervisor;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager as _};

use crate::executable_state::SystemClock;
use crate::ipc::commands::active_workspace;
use crate::ipc::dto::{
    ArtifactApprovalDto, ArtifactStateFrame, AvailabilityTag, PublicationDto, ShellError,
    ShellErrorCode, ShellErrorDetail,
};
use crate::outbox_state::{OutboxState, RunTable};
use crate::publication_state::PublicationState;

impl From<WorkAreaError> for ShellError {
    /// HAP-001-R7: a work area that resolves inside a registered workspace
    /// root, or cannot be used, refuses the operation.
    fn from(error: WorkAreaError) -> Self {
        let message = match error {
            WorkAreaError::InsideWorkspace => {
                "the product work area resolves inside a registered workspace root"
            }
            WorkAreaError::Unusable => "the product work area could not be used",
            WorkAreaError::NotOwnerOnly => "the product work area could not be made owner-only",
        };
        Self::new(ShellErrorCode::WorkAreaInvalid, message)
    }
}

impl From<JournalError> for ShellError {
    /// The publication journal is the work area's; a journal that cannot
    /// be read or written is a work area that cannot be used.
    fn from(error: JournalError) -> Self {
        let message = match error {
            JournalError::Unreadable => "the publication journal could not be read",
            JournalError::Corrupt => "the publication journal is corrupt",
            JournalError::WriteFailed => "the publication journal could not be written",
        };
        Self::new(ShellErrorCode::WorkAreaInvalid, message)
    }
}

impl From<DestinationError> for ShellError {
    /// HAP-001-R6, R14: no asset root, or a device asset path that resolves
    /// inside a registered workspace root or cannot be used.
    fn from(error: DestinationError) -> Self {
        let message = match error {
            DestinationError::Unconfigured => "the project declares no asset root",
            DestinationError::InsideWorkspace => {
                "the device asset path resolves inside a registered workspace root"
            }
            DestinationError::Unusable => "the device asset path could not be used",
        };
        Self::new(ShellErrorCode::DestinationInvalid, message)
    }
}

impl From<CatalogStoreError> for ShellError {
    fn from(error: CatalogStoreError) -> Self {
        let message = match error {
            CatalogStoreError::Unreadable => "the catalog could not be read",
            CatalogStoreError::Corrupt => "the catalog is corrupt",
            CatalogStoreError::WriteFailed => "the catalog could not be written",
            CatalogStoreError::Duplicate | CatalogStoreError::Unknown => {
                "the catalog refused the record"
            }
        };
        Self::new(ShellErrorCode::CatalogUnavailable, message)
    }
}

impl From<PublishError> for ShellError {
    /// Maps every way the transaction ends with nothing registered to its
    /// HAP-001 token; the message names the condition, never a path or a
    /// digest (the duplicate's ids travel in `detail`).
    fn from(error: PublishError) -> Self {
        match error {
            PublishError::WorkAreaInvalid => Self::from(WorkAreaError::InsideWorkspace),
            PublishError::OutboxEscape { recovery: Some(_) } => Self::new(
                ShellErrorCode::OutboxEscape,
                "the entry's path no longer names the approved file; the approved bytes were kept \
                 as a recovery entry",
            ),
            PublishError::OutboxEscape { recovery: None } => Self::new(
                ShellErrorCode::OutboxEscape,
                "the entry's path no longer names the approved file and no recovery entry could be \
                 written",
            ),
            PublishError::OutboxLinked { .. } => Self::new(
                ShellErrorCode::OutboxLinked,
                "the entry's link count is greater than one",
            ),
            PublishError::IntegrityMismatch { .. } => Self::new(
                ShellErrorCode::IntegrityMismatch,
                "the published copy did not verify against the approved digest; it was discarded \
                 and the entry preserved",
            ),
            PublishError::DuplicatePublication { existing } => Self::with_detail(
                ShellErrorCode::DuplicatePublication,
                "an artifact with this content is already registered for this project",
                ShellErrorDetail::DuplicatePublication {
                    publication_id: existing.publication_id.to_hex(),
                    catalog_id: existing.catalog_id.to_string(),
                },
            ),
            PublishError::Journal(error) => Self::from(error),
            PublishError::Catalog(error) => Self::from(error),
            PublishError::Provider(error) => Self::new(
                ShellErrorCode::DestinationInvalid,
                match error {
                    BlobStoreError::WriteFailed => {
                        "the published copy could not be written to the device asset path"
                    }
                    BlobStoreError::ReadFailed | BlobStoreError::NotFound => {
                        "the published copy could not be read back from the device asset path"
                    }
                    BlobStoreError::ForeignLocator => {
                        "the record's locator was not issued by this device's provider"
                    }
                },
            ),
        }
    }
}

/// Whether any supervised process is running: the shell's view of the one
/// source `harness_stop` and `harness_observe` consult (renderer risk
/// review R1-001). The publication surface is frozen while it says so -- a
/// spike default the owner may relax (publishing a finished run's
/// candidates while another run streams is a legitimate future option); it
/// exists now so the renderer's frozen surface and the backend agree, and
/// direct IPC cannot bypass what the renderer refuses. TM-001-R1's facts
/// are re-verified by the transaction regardless of this guard.
pub trait RunActivity {
    /// `true` while at least one supervised process is `Running`.
    fn any_running(&self) -> bool;
}

impl RunActivity for TokioProcessSupervisor {
    fn any_running(&self) -> bool {
        !self.running_ids().is_empty()
    }
}

/// The fixed refusal while a run is active.
fn run_active() -> ShellError {
    ShellError::new(
        ShellErrorCode::RunActive,
        "a run is active; approve or publish once it has ended",
    )
}

/// `artifact_approve`'s request: the candidate named by its run, its
/// outbox-relative name as `candidates_list` listed it, and its full
/// digest (HAP-001-R22: the approval binds identity facts, never a name
/// alone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApproveRequest {
    pub run_id: String,
    pub name: String,
    pub sha256: String,
}

fn invalid(message: &str) -> ShellError {
    ShellError::new(ShellErrorCode::InvalidRequest, message)
}

fn refused(message: &str) -> ShellError {
    ShellError::new(ShellErrorCode::Refused, message)
}

/// The entry's own file name within its run subdirectory: the part of the
/// outbox-relative name after `<run id>/`.
fn file_name_of(approval: &ArtifactApproval) -> OsString {
    let prefix = format!("{}/", approval.run_id);
    OsString::from(
        approval
            .name
            .strip_prefix(&prefix)
            .unwrap_or(&approval.name),
    )
}

/// Approve the candidate `request` names for publication (HAP-001-R22,
/// D3): an explicit, per-artifact action bound to the identity facts the
/// run-end inventory took from the entry's handle. The approval is the
/// publication journal's first entry for the publication, written under
/// the work area after its re-check (HAP-001-R7).
///
/// # Errors
///
/// `invalid-request` for a run, name, or digest that names nothing;
/// `refused` for an entry refused at validation, a digest that does not
/// match, or any class but `generated-heavy`; `destination-invalid` when
/// the project's policy declares no asset root; `work-area-invalid` when
/// the work area fails its check or the journal cannot be written.
pub fn approve_candidate(
    outbox: &OutboxState,
    workspace: &omnifrons_app::WorkspaceRoot,
    publication: &PublicationState,
    run_activity: &dyn RunActivity,
    now: SystemTime,
    request: &ApproveRequest,
) -> Result<ArtifactApprovalDto, ShellError> {
    // R1-001: one approve, publish, or list at a time per shell -- the
    // journal's replay-then-append is never interleaved.
    let _surface = publication.lock_surface();
    // The surface is frozen while a run is live (spike default; see
    // `RunActivity`).
    if run_activity.any_running() {
        return Err(run_active());
    }
    let run_id = RunId::new(&request.run_id).map_err(|_| invalid("the run id is not valid"))?;
    let digest = Sha256Digest::from_hex(&request.sha256)
        .ok_or_else(|| invalid("the digest is not 64 hex characters"))?;
    let hasher = Sha2Hasher::new();
    let project = derive_project_identity(&hasher, workspace);

    let approval = {
        let runs = outbox
            .runs
            .lock()
            .expect("run table mutex poisoned by a prior panic");
        let record = runs
            .find_by_run_id(&run_id)
            .ok_or_else(|| invalid("no run with that id is remembered"))?;
        let candidates = record.candidates().ok_or_else(|| {
            invalid("the run has not ended yet, so its subdirectory has not been inventoried")
        })?;
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.entry.name == request.name)
            .ok_or_else(|| invalid("no candidate with that name is listed for the run"))?;
        if candidate.state != CandidateState::Candidate {
            return Err(refused(
                "the entry was refused at validation and cannot be approved",
            ));
        }
        if candidate.entry.digest != digest {
            return Err(refused(
                "the candidate's digest does not match the approval request",
            ));
        }
        if candidate.entry.class != ArtifactClass::GeneratedHeavy {
            return Err(refused(
                "only a generated-heavy candidate can be approved for publication",
            ));
        }
        let asset_root_id = record
            .policy()
            .asset_root_id()
            .cloned()
            .ok_or_else(|| ShellError::from(DestinationError::Unconfigured))?;
        let publication_id = derive_publication_identity(&hasher, &project, &digest);
        let file_name = request
            .name
            .strip_prefix(&format!("{run_id}/"))
            .unwrap_or(&request.name);
        ArtifactApproval {
            approval_id: derive_artifact_approval_id(&hasher, &publication_id, now),
            publication_id,
            project,
            run_id: run_id.clone(),
            name: request.name.clone(),
            display_name: DisplayName::sanitize(file_name),
            digest,
            size: candidate.entry.size,
            detected_type: candidate.entry.detected_type,
            class: candidate.entry.class,
            attribution: candidate.entry.attribution.clone(),
            asset_root_id,
            adapter_id: record.adapter_id().cloned(),
            executable_approval: record.executable_approval(),
            approver: DeviceLocalUser,
            approved_at: now,
        }
    };

    // HAP-001-R7: the work area is checked before anything is recorded.
    let work_area = WorkAreaRoot::open(&publication.work_area, &[workspace])?;
    let mut journal = JsonlPublicationJournal::open(&work_area)?;
    // A derived id is never counted, so a collision -- the same publication
    // approved at the same instant -- is refused explicitly rather than
    // recorded twice, as the slice-2 approval store refuses its own.
    if find_approval(&journal.replay()?, approval.approval_id).is_some() {
        return Err(invalid(
            "an approval with this derived id is already recorded",
        ));
    }
    journal.append(&JournalEntry::Approved(Box::new(approval.clone())))?;
    Ok(ArtifactApprovalDto::from_approval(&approval))
}

/// Where a publication's bytes come from: the handle the run record held
/// since validation, or -- when the record holds none (the D22 cap, an
/// evicted record) -- a fresh open under the single-handle discipline
/// (HAP-001-R15, R17) whose facts must equal the approved ones.
fn acquire_source(
    outbox: &OutboxState,
    workspace: &omnifrons_app::WorkspaceRoot,
    approval: &ArtifactApproval,
    journal: &mut JsonlPublicationJournal,
    now: SystemTime,
    on_state: &mut dyn FnMut(ArtifactStateFrame),
) -> Result<CandidateSource, ShellError> {
    let file_name = file_name_of(approval);
    if let Some(source) = take_held_source(&outbox.runs, approval, &file_name)? {
        return Ok(source);
    }

    // The re-open path: the outbox, then the run subdirectory relative to
    // it, then one no-follow open of the entry.
    let policy = outbox.policy_store.load(workspace)?;
    let opened = outbox
        .preparer
        .open_outbox(workspace, policy.outbox(), false)
        .ok();
    let dir = opened.as_ref().and_then(|opened| {
        outbox
            .inventory
            .open_subdirectory(
                &opened.handle,
                &opened.path,
                OsStr::new(approval.run_id.as_str()),
            )
            .ok()
    });
    let (Some(opened), Some(dir)) = (opened, dir) else {
        return Err(fail_before_publish(
            journal,
            approval,
            JournalStep::Refused,
            ArtifactState::Refused,
            "entry-not-reopenable",
            now,
            on_state,
            refused("the entry could not be re-opened under the outbox"),
        ));
    };
    let dir_path = opened.path.join(approval.run_id.as_str());
    match FsCandidateProber::new().probe(&dir, &dir_path, &file_name) {
        CandidateProbe::Regular(regular)
            if regular.digest == approval.digest
                && regular.size == approval.size
                && regular.detected_type == approval.detected_type =>
        {
            Ok(CandidateSource {
                dir,
                dir_path,
                file_name,
                handle: regular.handle,
            })
        }
        CandidateProbe::Regular(_) => Err(fail_before_publish(
            journal,
            approval,
            JournalStep::Refused,
            ArtifactState::Refused,
            "identity-facts-changed-at-re-open",
            now,
            on_state,
            refused("the entry's identity facts changed since it was approved"),
        )),
        CandidateProbe::Escape(_) => Err(fail_before_publish(
            journal,
            approval,
            JournalStep::OutboxEscape,
            ArtifactState::OutboxEscape,
            "not-a-regular-file-at-re-open",
            now,
            on_state,
            ShellError::new(
                ShellErrorCode::OutboxEscape,
                "the entry is no longer a regular file inside the outbox",
            ),
        )),
        CandidateProbe::Linked { .. } => Err(fail_before_publish(
            journal,
            approval,
            JournalStep::OutboxLinked,
            ArtifactState::OutboxLinked,
            "link-count-above-one-at-re-open",
            now,
            on_state,
            ShellError::new(
                ShellErrorCode::OutboxLinked,
                "the entry's link count is greater than one",
            ),
        )),
        CandidateProbe::Unreadable => Err(fail_before_publish(
            journal,
            approval,
            JournalStep::Refused,
            ArtifactState::Refused,
            "entry-unreadable-at-re-open",
            now,
            on_state,
            refused("the entry could not be re-opened under the outbox"),
        )),
    }
}

/// The held handle from the run record, if the record still holds one for
/// the approved entry, with a duplicate of the run subdirectory handle it
/// is opened relative to.
fn take_held_source(
    runs: &Arc<Mutex<RunTable>>,
    approval: &ArtifactApproval,
    file_name: &OsStr,
) -> Result<Option<CandidateSource>, ShellError> {
    let mut runs = runs
        .lock()
        .expect("run table mutex poisoned by a prior panic");
    let Some(record) = runs.find_by_run_id_mut(&approval.run_id) else {
        return Ok(None);
    };
    let Some(handle) = record.take_candidate_handle(&approval.name) else {
        return Ok(None);
    };
    let subdirectory = record
        .subdirectory_clone()
        .map_err(|_| refused("the run subdirectory handle could not be duplicated"))?;
    Ok(Some(CandidateSource {
        dir: subdirectory.handle,
        dir_path: subdirectory.path,
        file_name: file_name.to_os_string(),
        handle,
    }))
}

/// A failure decided before the transaction started: journaled as its
/// step, emitted as its state, returned as `error`.
#[allow(clippy::too_many_arguments)]
fn fail_before_publish(
    journal: &mut JsonlPublicationJournal,
    approval: &ArtifactApproval,
    step: JournalStep,
    state: ArtifactState,
    reason: &str,
    now: SystemTime,
    on_state: &mut dyn FnMut(ArtifactStateFrame),
    error: ShellError,
) -> ShellError {
    if let Err(journal_error) = journal_failure(journal, approval, step, reason, now) {
        return ShellError::from(journal_error);
    }
    on_state(ArtifactStateFrame::from_event(&StateEvent {
        publication_id: approval.publication_id,
        state,
        provider_state: None,
    }));
    error
}

/// The dev-mode provider for `asset_root_id`: its device asset path under
/// the configured asset roots, re-canonicalized and re-checked against the
/// active workspace at this publication (HAP-001-R14).
fn open_provider(
    publication: &PublicationState,
    workspace: &omnifrons_app::WorkspaceRoot,
    asset_root_id: &omnifrons_domain::publication::AssetRootId,
) -> Result<LocalDirBlobStore, ShellError> {
    let device_path = DeviceAssetPath::open(
        &publication.asset_roots.join(asset_root_id.as_str()),
        &[workspace],
    )?;
    Ok(LocalDirBlobStore::open(&device_path, asset_root_id.clone()))
}

/// Publish the approval `approval_id` names (HAP-001 § Publication
/// transaction, steps 7 to 10 over the held handle), emitting one
/// `artifact.state` frame per transition on `on_state`.
///
/// # Errors
///
/// `invalid-request` for an approval id that is malformed, unrecorded, or
/// another project's; `work-area-invalid`, `destination-invalid`,
/// `refused`, `outbox-escape`, `outbox-linked`, `integrity-mismatch`,
/// `duplicate-publication`, and `catalog-unavailable` as
/// [`From<PublishError>`] and the re-open path map them.
pub fn publish_approved(
    outbox: &OutboxState,
    workspace: &omnifrons_app::WorkspaceRoot,
    publication: &PublicationState,
    run_activity: &dyn RunActivity,
    approval_id: &str,
    on_state: &mut dyn FnMut(ArtifactStateFrame),
) -> Result<PublicationDto, ShellError> {
    // R1-001: the publication surface lock is held for the whole
    // transaction, so two publications of one identity can never both pass
    // the Catalog's duplicate check (HAP-001-R23 under concurrency).
    let _surface = publication.lock_surface();
    // The surface is frozen while a run is live (spike default; see
    // `RunActivity`).
    if run_activity.any_running() {
        return Err(run_active());
    }
    let approval_id = ArtifactApprovalId::from_hex(approval_id)
        .ok_or_else(|| invalid("the approval id is not 16 hex characters"))?;
    let work_area = WorkAreaRoot::open(&publication.work_area, &[workspace])?;
    let mut journal = JsonlPublicationJournal::open(&work_area)?;
    let entries = journal.replay()?;
    let approval = find_approval(&entries, approval_id)
        .cloned()
        .ok_or_else(|| invalid("no approval with that id is recorded"))?;
    let hasher = Sha2Hasher::new();
    if approval.project != derive_project_identity(&hasher, workspace) {
        return Err(invalid("the approval belongs to another project"));
    }
    let clock = SystemClock;
    let now = std::time::SystemTime::now();

    // HAP-001-R23 before the entry is touched: a repeat of a completed
    // publication is acknowledged from the Catalog alone, whether or not
    // its outbox entry still exists (it was removed when it published).
    let mut catalog = JsonlCatalogStore::open(workspace)?;
    {
        let mut early_sink = |event: StateEvent| on_state(ArtifactStateFrame::from_event(&event));
        if let Some(error) = acknowledge_registered(
            &mut catalog,
            &mut journal,
            &clock,
            &approval,
            &mut early_sink,
        )? {
            return Err(ShellError::from(error));
        }
    }

    let source = acquire_source(outbox, workspace, &approval, &mut journal, now, on_state)?;
    let provider = open_provider(publication, workspace, &approval.asset_root_id)?;
    let entry_ops = FsOutboxEntryOps::new();
    let mut sink = |event: StateEvent| on_state(ArtifactStateFrame::from_event(&event));
    let published: Published = publish(
        PublishPorts {
            hasher: &hasher,
            provider: &provider,
            catalog: &mut catalog,
            journal: &mut journal,
            entry_ops: &entry_ops,
            clock: &clock,
            work_area: &work_area,
            workspaces: &[workspace],
        },
        &approval,
        source,
        &mut sink,
    )?;
    // The bytes were verified at this device's asset path just now.
    Ok(PublicationDto::from_published(
        &published,
        AvailabilityTag::Local,
    ))
}

/// This device's availability observation for `publication` (HAP-001-R27):
/// `local` when this device's own journal verified the bytes here.
fn availability_of(
    entries: &[JournalEntry],
    publication: &omnifrons_domain::publication::PublicationIdentity,
) -> AvailabilityTag {
    let verified_here = entries.iter().any(|entry| match entry {
        JournalEntry::Step(step) => {
            &step.publication_id == publication
                && step.step == JournalStep::PublishedLocal
                && step.outcome == StepOutcome::Ok
        }
        JournalEntry::Approved(_) => false,
    });
    if verified_here {
        AvailabilityTag::Local
    } else {
        AvailabilityTag::Unknown
    }
}

/// Every publication of the active project: the Catalog's records, after
/// the journal is replayed and any `published-local` left without a
/// registration is registered (HAP-001-R29 -- the restart replay runs
/// here and in `artifact_publish`, the two commands that touch
/// publications), plus any registration recovery could not complete,
/// listed as `registration-pending`.
///
/// # Errors
///
/// `work-area-invalid`, `destination-invalid`, `outbox-invalid` (the
/// policy could not be loaded), or `catalog-unavailable`.
pub fn publications_for(
    outbox: &OutboxState,
    workspace: &omnifrons_app::WorkspaceRoot,
    publication: &PublicationState,
) -> Result<Vec<PublicationDto>, ShellError> {
    // R1-001: the restart replay registers records; it must not interleave
    // with a publication.
    let _surface = publication.lock_surface();
    let work_area = WorkAreaRoot::open(&publication.work_area, &[workspace])?;
    let mut journal = JsonlPublicationJournal::open(&work_area)?;
    let hasher = Sha2Hasher::new();
    let project: ProjectIdentity = derive_project_identity(&hasher, workspace);
    let mut catalog = JsonlCatalogStore::open(workspace)?;

    // Recovery resumes through the project's asset root; a project without
    // one has nothing to resume through.
    let policy = outbox.policy_store.load(workspace)?;
    if let Some(asset_root_id) = policy.asset_root_id() {
        let provider = open_provider(publication, workspace, asset_root_id)?;
        let entry_ops = FsOutboxEntryOps::new();
        let clock = SystemClock;
        recover(
            PublishPorts {
                hasher: &hasher,
                provider: &provider,
                catalog: &mut catalog,
                journal: &mut journal,
                entry_ops: &entry_ops,
                clock: &clock,
                work_area: &work_area,
                workspaces: &[workspace],
            },
            &project,
            &mut |_| {},
        )?;
    }

    let entries = journal.replay()?;
    let records = catalog.list()?;
    let mut listed: Vec<PublicationDto> = records
        .iter()
        .map(|record| {
            PublicationDto::from_record(record, availability_of(&entries, &record.publication_id))
        })
        .collect();
    for (record, _) in pending_registrations(&entries) {
        if record.provenance.scope_id == project
            && !records
                .iter()
                .any(|registered| registered.publication_id == record.publication_id)
        {
            listed.push(PublicationDto {
                publication_id: record.publication_id.to_hex(),
                state: ArtifactState::RegistrationPending.into(),
                reference: None,
                provider_state: None,
                catalog_id: None,
                names: record
                    .names
                    .iter()
                    .map(|name| name.as_str().to_string())
                    .collect(),
                availability: availability_of(&entries, &record.publication_id),
            });
        }
    }
    Ok(listed)
}

/// Approve a candidate entry of a remembered run for publication
/// (HAP-001-R22, D3).
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`approve_candidate`]'s errors.
#[tauri::command]
pub async fn artifact_approve(
    app: AppHandle,
    run_id: String,
    name: String,
    sha256: String,
) -> Result<ArtifactApprovalDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let outbox = app.state::<OutboxState>();
        let publication = app.state::<PublicationState>();
        let supervisor = app.state::<TokioProcessSupervisor>().inner().clone();
        approve_candidate(
            &outbox,
            &workspace,
            &publication,
            &supervisor,
            SystemTime::now(),
            &ApproveRequest {
                run_id,
                name,
                sha256,
            },
        )
    })
    .await
    .expect("the blocking artifact-approve task panicked")
}

/// Publish an approved candidate (HAP-001 § Publication transaction),
/// streaming one `artifact-state` frame per transition over `on_state`.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`publish_approved`]'s errors.
#[tauri::command]
pub async fn artifact_publish(
    app: AppHandle,
    approval_id: String,
    on_state: Channel<ArtifactStateFrame>,
) -> Result<PublicationDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let outbox = app.state::<OutboxState>();
        let publication = app.state::<PublicationState>();
        let supervisor = app.state::<TokioProcessSupervisor>().inner().clone();
        publish_approved(
            &outbox,
            &workspace,
            &publication,
            &supervisor,
            &approval_id,
            &mut |frame| {
                // A renderer that went away is not a publication failure:
                // the transaction and its journal are the record.
                let _ = on_state.send(frame);
            },
        )
    })
    .await
    .expect("the blocking artifact-publish task panicked")
}

/// Every publication of the active project, after the restart replay
/// (HAP-001-R29).
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`publications_for`]'s errors.
#[tauri::command]
pub async fn publications_list(app: AppHandle) -> Result<Vec<PublicationDto>, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let outbox = app.state::<OutboxState>();
        let publication = app.state::<PublicationState>();
        publications_for(&outbox, &workspace, &publication)
    })
    .await
    .expect("the blocking publications-list task panicked")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime};

    use omnifrons_adapters::Sha2Hasher;
    use omnifrons_app::content_hasher::ContentHasher as _;
    use omnifrons_app::outbox_policy::POLICY_FILE_PATH;
    use omnifrons_app::run_outbox::{
        OutboxInventory as _, RunOutboxPreparer as _, assemble_run_candidates, cap_held_handles,
    };
    use omnifrons_app::{ProcessId, WorkspaceRoot};
    use omnifrons_domain::adapter::AdapterId;
    use omnifrons_domain::executable::{ApprovalId, Sha256Digest};
    use omnifrons_domain::outbox::{OutboxPath, ProposedEntry, PublishProposal, RunId};

    use super::{
        ApproveRequest, RunActivity, approve_candidate, publications_for, publish_approved,
    };

    /// No supervised process is running.
    struct Idle;

    impl RunActivity for Idle {
        fn any_running(&self) -> bool {
            false
        }
    }

    /// A supervised process is running.
    struct Busy;

    impl RunActivity for Busy {
        fn any_running(&self) -> bool {
            true
        }
    }
    use crate::ipc::dto::{
        ArtifactApprovalDto, ArtifactStateFrame, ArtifactStateTag, AttributionDto, AvailabilityTag,
        ProviderStateTag, PublicationDto, ShellError, ShellErrorCode, ShellErrorDetail,
    };
    use crate::outbox_state::{OutboxState, RunRecord};
    use crate::publication_state::PublicationState;

    const PDF: &[u8] = b"%PDF-1.7\nomnifrons shell publication\n";
    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];

    /// A drop-guard temp directory.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "omnifrons-shell-publication-test-{}-{label}-{n}",
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

    /// The shell has no hashing dependency of its own; the adapters' real
    /// hasher is what every identity comes from anyway.
    fn hex(bytes: &[u8]) -> String {
        Sha2Hasher::new().sha256(bytes).to_hex()
    }

    /// The fixture: a project whose policy names asset root `main`, a
    /// device directory holding the work area and the asset roots, the
    /// shell's outbox state with one ended run remembered (its candidates
    /// inventoried and stored, `report.pdf` attributed by the run's own
    /// proposal, `stray.png` and `notes.md` unattributed), and the
    /// publication state pointing at the device directory.
    struct Fixture {
        project: TempDir,
        device: TempDir,
        outbox: OutboxState,
        publication: PublicationState,
        run_id: RunId,
        run_dir: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            Self::with_policy(label, r#"{"schema": 1, "assetRootId": "main"}"#)
        }

        fn with_policy(label: &str, policy_json: &str) -> Self {
            let project = TempDir::new(label);
            let device = TempDir::new(&format!("{label}-device"));
            let policy_path = project.path().join(POLICY_FILE_PATH);
            std::fs::create_dir_all(policy_path.parent().expect("parent")).expect("mkdir");
            std::fs::write(&policy_path, policy_json).expect("policy");
            let outbox = OutboxState::new();
            let publication = PublicationState::under(device.path());
            let run_id = RunId::new("run-1").expect("valid");
            let workspace = project.workspace();
            let policy = omnifrons_app::outbox_policy::OutboxPolicyStore::load(
                &outbox.policy_store,
                &workspace,
            )
            .expect("policy loads");
            let prepared = outbox
                .preparer
                .prepare(&workspace, &OutboxPath::default_path(), &run_id)
                .expect("prepare");
            let run_dir = prepared.path.clone();
            std::fs::write(run_dir.join("report.pdf"), PDF).expect("report");
            std::fs::write(run_dir.join("stray.png"), PNG).expect("stray");
            std::fs::write(run_dir.join("notes.md"), b"# notes\n").expect("notes");
            let entries = outbox
                .inventory
                .inventory(&prepared.handle, &prepared.path)
                .expect("inventory");
            let proposal = PublishProposal {
                entries: vec![ProposedEntry {
                    name: "report.pdf".to_string(),
                    sha256: Sha256Digest::from_hex(&hex(PDF)).expect("hex"),
                }],
            };
            let assembled = assemble_run_candidates(&run_id, entries, &[proposal], &policy);
            let mut runs = outbox.runs.lock().expect("table");
            runs.insert(
                ProcessId(7),
                RunRecord::with_provenance(
                    prepared,
                    policy,
                    Some(AdapterId::claude_code()),
                    Some(ApprovalId(42)),
                ),
            );
            runs.store_candidates(ProcessId(7), assembled.candidates)
                .expect("remembered");
            drop(runs);
            Self {
                project,
                device,
                outbox,
                publication,
                run_id,
                run_dir,
            }
        }

        fn workspace(&self) -> WorkspaceRoot {
            self.project.workspace()
        }

        fn now() -> SystemTime {
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_401)
        }

        fn approve(&self, name: &str, bytes: &[u8]) -> Result<ArtifactApprovalDto, ShellError> {
            self.approve_at(name, bytes, Self::now())
        }

        fn approve_at(
            &self,
            name: &str,
            bytes: &[u8],
            now: SystemTime,
        ) -> Result<ArtifactApprovalDto, ShellError> {
            approve_candidate(
                &self.outbox,
                &self.workspace(),
                &self.publication,
                &Idle,
                now,
                &ApproveRequest {
                    run_id: self.run_id.as_str().to_string(),
                    name: format!("{}/{name}", self.run_id),
                    sha256: hex(bytes),
                },
            )
        }

        fn publish(
            &self,
            approval_id: &str,
        ) -> (Result<PublicationDto, ShellError>, Vec<ArtifactStateFrame>) {
            let mut frames = Vec::new();
            let result = publish_approved(
                &self.outbox,
                &self.workspace(),
                &self.publication,
                &Idle,
                approval_id,
                &mut |frame| frames.push(frame),
            );
            (result, frames)
        }

        /// Re-inventory the remembered run and store the candidates, with
        /// every held handle released when `release_handles` (the D22
        /// cap's effect).
        fn restore_candidates(&self, release_handles: bool) {
            let mut runs = self.outbox.runs.lock().expect("table");
            let record = runs.get(ProcessId(7)).expect("record");
            let subdirectory = record.subdirectory_clone().expect("clone");
            let policy = record.policy().clone();
            let entries = self
                .outbox
                .inventory
                .inventory(&subdirectory.handle, &subdirectory.path)
                .expect("inventory");
            let mut assembled = assemble_run_candidates(&self.run_id, entries, &[], &policy);
            if release_handles {
                let released = cap_held_handles(&mut assembled.candidates, 0);
                assert!(released > 0, "the fixture releases every handle");
            }
            runs.store_candidates(ProcessId(7), assembled.candidates)
                .expect("remembered");
            if release_handles {
                assert_eq!(runs.held_handles(), 0);
            }
        }

        fn journal_text(&self) -> String {
            std::fs::read_to_string(
                self.publication
                    .work_area
                    .join("journal")
                    .join("publications.jsonl"),
            )
            .unwrap_or_default()
        }

        fn catalog_text(&self) -> String {
            std::fs::read_to_string(self.project.path().join(".omnifrons/catalog.jsonl"))
                .unwrap_or_default()
        }

        fn copies(&self) -> usize {
            std::fs::read_dir(self.device.path().join("asset-roots/main"))
                .map_or(0, Iterator::count)
        }
    }

    fn states(frames: &[ArtifactStateFrame]) -> Vec<ArtifactStateTag> {
        frames
            .iter()
            .map(|frame| match frame {
                ArtifactStateFrame::ArtifactState { state, .. } => *state,
            })
            .collect()
    }

    // -- approval (HAP-001-R22, D3) --

    /// An explicit approval naming the candidate by run id, name, and
    /// digest records the identity-bound facts under the work area and
    /// returns them with the act-as identity; the approval id is derived,
    /// and the publication identity is the project's and the content's.
    #[test]
    fn approving_an_attributed_candidate_records_and_returns_its_facts() {
        let fixture = Fixture::new("approve");
        let dto = fixture.approve("report.pdf", PDF).expect("approved");
        assert_eq!(dto.run_id, "run-1");
        assert_eq!(dto.name, "run-1/report.pdf");
        assert_eq!(dto.display_name, "report.pdf");
        assert_eq!(dto.sha256_short, hex(PDF)[..8]);
        assert_eq!(dto.size, PDF.len() as u64);
        assert_eq!(dto.detected_type, "pdf");
        assert_eq!(dto.class, "generated-heavy");
        assert_eq!(
            dto.attribution,
            AttributionDto::Run {
                run_id: "run-1".to_string()
            }
        );
        assert_eq!(dto.asset_root_id, "main");
        assert_eq!(dto.approval_id.len(), 16, "a 16-hex derived id");
        assert_eq!(dto.publication_id.len(), 64);
        assert_eq!(dto.approved_at, 1_725_782_401_000);
        let journal = fixture.journal_text();
        assert_eq!(journal.lines().count(), 1, "one approved line");
        let line: serde_json::Value =
            serde_json::from_str(journal.lines().next().expect("line")).expect("json");
        assert_eq!(line["event"], "approved");
        assert_eq!(line["approvalId"], dto.approval_id);
        assert_eq!(line["adapterId"], "claude-code");
        assert_eq!(line["executableApprovalId"], 42);
        assert!(
            !journal.contains(&fixture.project.path().to_string_lossy().to_string()),
            "the journal carries no device path"
        );
    }

    /// An unattributed entry needs the same explicit approval and gets it;
    /// its approval records the unattributed fact.
    #[test]
    fn approving_an_unattributed_candidate_records_the_unattributed_fact() {
        let fixture = Fixture::new("approve-unattributed");
        let dto = fixture.approve("stray.png", PNG).expect("approved");
        assert_eq!(dto.attribution, AttributionDto::Unattributed);
        assert_eq!(dto.detected_type, "png");
        let line: serde_json::Value =
            serde_json::from_str(fixture.journal_text().lines().next().expect("line"))
                .expect("json");
        assert_eq!(
            line["attribution"],
            serde_json::json!({"kind": "unattributed"})
        );
    }

    /// A digest that does not match the candidate's is `refused`: the
    /// approval binds identity facts, never a name alone (HAP-001-R22).
    #[test]
    fn approving_with_a_mismatched_digest_is_refused() {
        let fixture = Fixture::new("approve-digest");
        let error = fixture
            .approve("report.pdf", b"other bytes")
            .expect_err("refused");
        assert_eq!(error.code, ShellErrorCode::Refused);
        assert!(!error.message.contains('/'));
        assert!(fixture.journal_text().is_empty(), "nothing recorded");
    }

    /// A Markdown note (`portable-text`) is never approved for publication
    /// (HAP-001-R2), and nor is any class but `generated-heavy`.
    #[test]
    fn approving_a_non_heavy_entry_is_refused() {
        let fixture = Fixture::new("approve-class");
        let error = fixture
            .approve("notes.md", b"# notes\n")
            .expect_err("refused");
        assert_eq!(error.code, ShellErrorCode::Refused);
    }

    /// An entry refused at validation (`outbox-escape`) cannot be approved.
    #[cfg(unix)] // a symbolic link fixture
    #[test]
    fn approving_an_entry_refused_at_validation_is_refused() {
        let project = TempDir::new("approve-escape");
        let device = TempDir::new("approve-escape-device");
        let policy_path = project.path().join(POLICY_FILE_PATH);
        std::fs::create_dir_all(policy_path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&policy_path, r#"{"schema": 1, "assetRootId": "main"}"#).expect("policy");
        let outbox = OutboxState::new();
        let run_id = RunId::new("run-2").expect("valid");
        let workspace = project.workspace();
        let policy =
            omnifrons_app::outbox_policy::OutboxPolicyStore::load(&outbox.policy_store, &workspace)
                .expect("policy");
        let prepared = outbox
            .preparer
            .prepare(&workspace, &OutboxPath::default_path(), &run_id)
            .expect("prepare");
        std::os::unix::fs::symlink(
            project.path().join("elsewhere"),
            prepared.path.join("link.pdf"),
        )
        .expect("plant a link");
        let entries = outbox
            .inventory
            .inventory(&prepared.handle, &prepared.path)
            .expect("inventory");
        let assembled = assemble_run_candidates(&run_id, entries, &[], &policy);
        {
            let mut runs = outbox.runs.lock().expect("table");
            runs.insert(
                ProcessId(8),
                RunRecord::with_provenance(prepared, policy, None, None),
            );
            runs.store_candidates(ProcessId(8), assembled.candidates)
                .expect("remembered");
        }
        let error = approve_candidate(
            &outbox,
            &workspace,
            &PublicationState::under(device.path()),
            &Idle,
            Fixture::now(),
            &ApproveRequest {
                run_id: "run-2".to_string(),
                name: "run-2/link.pdf".to_string(),
                sha256: "00".repeat(32),
            },
        )
        .expect_err("refused");
        assert_eq!(error.code, ShellErrorCode::Refused);
    }

    /// A policy that declares no asset root has no destination:
    /// `destination-invalid`, never a fallback (HAP-001-R6).
    #[test]
    fn approving_without_a_declared_asset_root_is_destination_invalid() {
        let fixture = Fixture::with_policy("approve-no-root", r#"{"schema": 1}"#);
        let error = fixture
            .approve("report.pdf", PDF)
            .expect_err("no destination");
        assert_eq!(error.code, ShellErrorCode::DestinationInvalid);
    }

    /// An unknown run, an unknown name, or a malformed digest is
    /// `invalid-request`.
    #[test]
    fn approving_an_unknown_run_name_or_digest_is_invalid_request() {
        let fixture = Fixture::new("approve-unknown");
        let cases = [
            ("run-9", "run-9/report.pdf", hex(PDF)),
            ("run-1", "run-1/missing.pdf", hex(PDF)),
            ("run-1", "run-1/report.pdf", "zz".to_string()),
            ("not a run id", "x", hex(PDF)),
        ];
        for (run_id, name, sha256) in cases {
            let error = approve_candidate(
                &fixture.outbox,
                &fixture.workspace(),
                &fixture.publication,
                &Idle,
                Fixture::now(),
                &ApproveRequest {
                    run_id: run_id.to_string(),
                    name: name.to_string(),
                    sha256,
                },
            )
            .expect_err("invalid");
            assert_eq!(
                error.code,
                ShellErrorCode::InvalidRequest,
                "{run_id} {name}"
            );
        }
    }

    /// A derived approval id already on record -- the same publication
    /// approved at the same instant -- is refused rather than recorded
    /// twice, the slice-2 store's own duplicate discipline.
    #[test]
    fn approving_the_same_publication_at_the_same_instant_is_refused_as_a_duplicate_id() {
        let fixture = Fixture::new("approve-twice");
        let first = fixture.approve("report.pdf", PDF).expect("approved");
        let error = fixture
            .approve("report.pdf", PDF)
            .expect_err("the derived id is already recorded");
        assert_eq!(error.code, ShellErrorCode::InvalidRequest);
        assert_eq!(fixture.journal_text().lines().count(), 1, "recorded once");
        // One second, not one nanosecond: Windows keeps `SystemTime` in
        // 100 ns steps, so a 1 ns later instant is the same instant there.
        let later = fixture
            .approve_at("report.pdf", PDF, Fixture::now() + Duration::from_secs(1))
            .expect("a later instant is another approval");
        assert_ne!(later.approval_id, first.approval_id);
    }

    /// HAP-001-R7 at approval: a work area configured inside the active
    /// workspace refuses the approval with `work-area-invalid`, and
    /// nothing is written anywhere.
    #[test]
    fn a_work_area_inside_the_workspace_refuses_the_approval() {
        let mut fixture = Fixture::new("approve-work-area");
        fixture.publication.work_area = fixture.project.path().join(".omnifrons/work-area");
        let error = fixture.approve("report.pdf", PDF).expect_err("refused");
        assert_eq!(error.code, ShellErrorCode::WorkAreaInvalid);
        assert!(!fixture.project.path().join(".omnifrons/work-area").exists());
    }

    // -- publication (HAP-001 § Publication transaction) --

    /// The whole transaction through the shell's composition: the approval
    /// is looked up in the journal, the bytes come from the run record's
    /// held handle, the copy lands under the device's asset roots, the
    /// record under the project's `.omnifrons/catalog.jsonl`, the entry is
    /// removed only at the end, and the `artifact.state` frames arrive in
    /// order with the reference issued only at `registered`.
    #[test]
    fn publishing_an_approval_registers_the_artifact_and_emits_the_transitions() {
        let fixture = Fixture::new("publish");
        let approval = fixture.approve("report.pdf", PDF).expect("approved");
        let (result, frames) = fixture.publish(&approval.approval_id);
        let dto = result.expect("registered");

        assert_eq!(dto.state, ArtifactStateTag::Registered);
        assert_eq!(dto.publication_id, approval.publication_id);
        assert_eq!(dto.provider_state, Some(ProviderStateTag::Pending));
        assert_eq!(
            dto.catalog_id.as_deref(),
            Some(format!("main/{}", approval.publication_id).as_str())
        );
        let reference = dto.reference.expect("a reference once registered");
        assert_eq!(reference.id, approval.publication_id);
        assert_eq!(
            reference.locator,
            format!("main/{}", approval.publication_id)
        );
        assert_eq!(dto.names, vec!["report.pdf".to_string()]);
        assert_eq!(dto.availability, AvailabilityTag::Local);
        assert_eq!(
            states(&frames),
            vec![
                ArtifactStateTag::PublishedLocal,
                ArtifactStateTag::Registered
            ]
        );

        let copy = fixture
            .device
            .path()
            .join("asset-roots/main")
            .join(&approval.publication_id);
        assert_eq!(std::fs::read(&copy).expect("the copy"), PDF);
        assert_eq!(fixture.catalog_text().lines().count(), 1, "one record");
        // Unix removes the entry through the directory handle; elsewhere the
        // identity check is unverifiable and the entry stays (disclosed).
        #[cfg(unix)]
        assert!(!fixture.run_dir.join("report.pdf").exists());
        #[cfg(not(unix))]
        assert!(fixture.run_dir.join("report.pdf").exists());
        assert!(
            fixture.run_dir.join("stray.png").exists(),
            "other entries untouched"
        );
        let journal = fixture.journal_text();
        assert_eq!(
            journal.lines().count(),
            4,
            "approved, published-local, registered, cleanup"
        );
        assert!(!journal.contains(&fixture.device.path().to_string_lossy().to_string()));
    }

    /// The same bytes approved again under another name: the second
    /// publication is `duplicate-publication`, the existing record is
    /// acknowledged in `detail`, the new name is an alias, and no second
    /// copy exists.
    #[test]
    fn publishing_the_same_bytes_again_is_duplicate_publication_with_the_existing_ids() {
        let fixture = Fixture::new("duplicate");
        let first = fixture.approve("report.pdf", PDF).expect("approved");
        fixture.publish(&first.approval_id).0.expect("registered");
        std::fs::write(fixture.run_dir.join("report-copy.pdf"), PDF).expect("second file");
        // The second entry was written after the run ended: re-inventory
        // the run so the table knows it (the whole-outbox inventory would
        // list it too).
        fixture.restore_candidates(false);
        let second = fixture
            .approve_at(
                "report-copy.pdf",
                PDF,
                Fixture::now() + Duration::from_secs(1),
            )
            .expect("approved");
        assert_eq!(
            second.publication_id, first.publication_id,
            "same bytes, same identity"
        );
        assert_ne!(
            second.approval_id, first.approval_id,
            "a later instant, another approval"
        );
        let (result, frames) = fixture.publish(&second.approval_id);
        let error = result.expect_err("duplicate");
        assert_eq!(error.code, ShellErrorCode::DuplicatePublication);
        assert_eq!(
            error.detail,
            Some(ShellErrorDetail::DuplicatePublication {
                publication_id: first.publication_id.clone(),
                catalog_id: format!("main/{}", first.publication_id),
            })
        );
        assert_eq!(
            states(&frames),
            vec![ArtifactStateTag::DuplicatePublication]
        );
        assert_eq!(fixture.copies(), 1, "no second copy");
        let listed = publications_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].names,
            vec!["report.pdf".to_string(), "report-copy.pdf".to_string()]
        );
    }

    /// HAP-001-R17, D22: a candidate remembered without a held handle (the
    /// cap released it) is re-opened under the single-handle discipline
    /// when its turn comes, and publishes.
    #[test]
    fn a_candidate_beyond_the_handle_cap_is_re_opened_when_its_turn_comes() {
        let fixture = Fixture::new("cap-reopen");
        fixture.restore_candidates(true);
        let approval = fixture
            .approve("report.pdf", PDF)
            .expect("approved without a handle");
        let (result, frames) = fixture.publish(&approval.approval_id);
        let dto = result.expect("re-opened and registered");
        assert_eq!(dto.state, ArtifactStateTag::Registered);
        assert_eq!(
            states(&frames),
            vec![
                ArtifactStateTag::PublishedLocal,
                ArtifactStateTag::Registered
            ]
        );
    }

    /// A re-opened entry whose identity facts changed since approval is
    /// `refused`, journaled, and nothing is published.
    #[test]
    fn a_re_opened_entry_whose_facts_changed_is_refused() {
        let fixture = Fixture::new("cap-changed");
        fixture.restore_candidates(true);
        let approval = fixture.approve("report.pdf", PDF).expect("approved");
        std::fs::write(fixture.run_dir.join("report.pdf"), b"%PDF-1.7\nchanged\n")
            .expect("rewrite");
        let (result, frames) = fixture.publish(&approval.approval_id);
        let error = result.expect_err("refused");
        assert_eq!(error.code, ShellErrorCode::Refused);
        assert_eq!(states(&frames), vec![ArtifactStateTag::Refused]);
        assert_eq!(fixture.copies(), 0, "nothing published");
        assert!(fixture.catalog_text().is_empty());
        assert!(fixture.journal_text().contains("\"step\":\"refused\""));
    }

    /// HAP-001-R14 at publication: a device asset path that would resolve
    /// inside the active workspace is `destination-invalid`; nothing is
    /// copied and the entry stays.
    #[test]
    fn a_device_asset_path_inside_the_workspace_is_destination_invalid() {
        let mut fixture = Fixture::new("destination");
        let approval = fixture.approve("report.pdf", PDF).expect("approved");
        fixture.publication.asset_roots = fixture.project.path().join("assets");
        let (result, frames) = fixture.publish(&approval.approval_id);
        let error = result.expect_err("refused");
        assert_eq!(error.code, ShellErrorCode::DestinationInvalid);
        assert!(frames.is_empty());
        assert!(fixture.run_dir.join("report.pdf").exists());
        assert!(!fixture.project.path().join("assets").exists());
    }

    /// An approval id nobody recorded is `invalid-request`.
    #[test]
    fn publishing_an_unknown_approval_is_invalid_request() {
        let fixture = Fixture::new("unknown-approval");
        let (result, _) = fixture.publish("0123456789abcdef");
        assert_eq!(
            result.expect_err("unknown").code,
            ShellErrorCode::InvalidRequest
        );
        let (result, _) = fixture.publish("not-hex");
        assert_eq!(
            result.expect_err("malformed").code,
            ShellErrorCode::InvalidRequest
        );
    }

    // -- the list and the restart replay (HAP-001-R29) --

    /// `publications_list` replays the journal first: a `published-local`
    /// step with no registration -- the crash between publish and
    /// register -- is registered from the journaled record, once, and
    /// listed as `registered`; a second list repeats nothing.
    #[test]
    fn publications_list_resumes_a_pending_registration_from_the_journal() {
        let fixture = Fixture::new("resume");
        let approval = fixture.approve("report.pdf", PDF).expect("approved");
        // Simulate the crash: publish, then erase the registration and the
        // record while keeping the journal's published-local step.
        fixture
            .publish(&approval.approval_id)
            .0
            .expect("registered");
        let journal_path = fixture
            .publication
            .work_area
            .join("journal/publications.jsonl");
        let journal = std::fs::read_to_string(&journal_path).expect("journal");
        let truncated: Vec<&str> = journal
            .lines()
            .filter(|line| {
                !line.contains("\"step\":\"registered\"") && !line.contains("\"step\":\"cleanup\"")
            })
            .collect();
        std::fs::write(&journal_path, format!("{}\n", truncated.join("\n"))).expect("truncate");
        std::fs::remove_file(fixture.project.path().join(".omnifrons/catalog.jsonl"))
            .expect("erase the record");

        let listed = publications_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].state, ArtifactStateTag::Registered);
        assert_eq!(listed[0].publication_id, approval.publication_id);
        assert!(listed[0].reference.is_some());
        assert_eq!(
            fixture.catalog_text().lines().count(),
            1,
            "exactly one record"
        );
        assert_eq!(fixture.copies(), 1, "exactly one copy");
        let again = publications_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("list");
        assert_eq!(again.len(), 1);
        assert_eq!(
            fixture.catalog_text().lines().count(),
            1,
            "nothing repeated"
        );
    }

    /// An empty project lists nothing, and creates nothing in the project.
    #[test]
    fn publications_list_of_a_fresh_project_is_empty_and_writes_nothing() {
        let fixture = Fixture::new("fresh");
        let listed = publications_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("list");
        assert!(listed.is_empty());
        assert!(
            !fixture
                .project
                .path()
                .join(".omnifrons/catalog.jsonl")
                .exists()
        );
    }

    /// Every publication error maps to a fixed, slash-free catalogue
    /// message under its own code.
    #[test]
    fn publish_errors_map_to_their_codes_with_slash_free_messages() {
        use omnifrons_app::publication::{IntegrityReason, PublishError};
        let cases = [
            (
                PublishError::WorkAreaInvalid,
                ShellErrorCode::WorkAreaInvalid,
            ),
            (
                PublishError::OutboxEscape { recovery: None },
                ShellErrorCode::OutboxEscape,
            ),
            (
                PublishError::OutboxLinked { link_count: 2 },
                ShellErrorCode::OutboxLinked,
            ),
            (
                PublishError::IntegrityMismatch {
                    reason: IntegrityReason::PublishedCopyDiffers,
                },
                ShellErrorCode::IntegrityMismatch,
            ),
            (
                PublishError::Journal(omnifrons_app::publication_journal::JournalError::Corrupt),
                ShellErrorCode::WorkAreaInvalid,
            ),
            (
                PublishError::Catalog(omnifrons_app::catalog_store::CatalogStoreError::Corrupt),
                ShellErrorCode::CatalogUnavailable,
            ),
            (
                PublishError::Provider(omnifrons_app::blob_store::BlobStoreError::WriteFailed),
                ShellErrorCode::DestinationInvalid,
            ),
        ];
        for (error, code) in cases {
            let mapped = ShellError::from(error);
            assert_eq!(mapped.code, code);
            assert!(!mapped.message.contains('/'), "{}", mapped.message);
        }
    }

    // -- concurrency (R1-001) and the other project's approval (R3-006) --

    /// Split concurrent results into how many registered and how many were
    /// acknowledged as duplicates.
    fn outcomes(results: &[Result<PublicationDto, ShellError>]) -> (usize, usize) {
        let registered = results
            .iter()
            .filter(|result| matches!(result, Ok(dto) if dto.state == ArtifactStateTag::Registered))
            .count();
        let duplicates = results
            .iter()
            .filter(|result| {
                matches!(result, Err(error) if error.code == ShellErrorCode::DuplicatePublication)
            })
            .count();
        (registered, duplicates)
    }

    /// R1-001: the publication surface is serialized per project, so two
    /// publications of the same approval racing each other yield exactly
    /// one `registered` and one `duplicate-publication`, one record, one
    /// copy, and a catalog that still reads (HAP-001-R23 under
    /// concurrency).
    #[test]
    fn concurrent_publications_of_one_approval_yield_one_record_and_a_readable_catalog() {
        let fixture = Fixture::new("concurrent-same");
        let approval = fixture.approve("report.pdf", PDF).expect("approved");
        let results: Vec<Result<PublicationDto, ShellError>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..2)
                .map(|_| scope.spawn(|| fixture.publish(&approval.approval_id).0))
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("a publishing thread panicked"))
                .collect()
        });
        assert_eq!(outcomes(&results), (1, 1), "got {results:?}");
        let listed = publications_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("the catalog still reads");
        assert_eq!(listed.len(), 1);
        assert_eq!(fixture.copies(), 1, "one copy");
        assert_eq!(fixture.catalog_text().lines().count(), 1, "one record line");
    }

    /// R1-001: the same bytes approved under two names and published
    /// concurrently: one record, one alias, one copy.
    #[test]
    fn concurrent_publications_of_the_same_bytes_under_two_names_yield_one_record_and_one_alias() {
        let fixture = Fixture::new("concurrent-alias");
        std::fs::write(fixture.run_dir.join("report-copy.pdf"), PDF).expect("second file");
        fixture.restore_candidates(false);
        let first = fixture.approve("report.pdf", PDF).expect("approved");
        let second = fixture
            .approve_at(
                "report-copy.pdf",
                PDF,
                Fixture::now() + Duration::from_secs(1),
            )
            .expect("approved");
        let results: Vec<Result<PublicationDto, ShellError>> = std::thread::scope(|scope| {
            let a = scope.spawn(|| fixture.publish(&first.approval_id).0);
            let b = scope.spawn(|| fixture.publish(&second.approval_id).0);
            vec![
                a.join().expect("thread a panicked"),
                b.join().expect("thread b panicked"),
            ]
        });
        assert_eq!(outcomes(&results), (1, 1), "got {results:?}");
        let listed = publications_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("list");
        assert_eq!(listed.len(), 1, "one record");
        let mut names = listed[0].names.clone();
        names.sort();
        assert_eq!(
            names,
            vec!["report-copy.pdf".to_string(), "report.pdf".to_string()],
            "the loser's name is the alias"
        );
        assert_eq!(fixture.copies(), 1, "one copy");
        assert_eq!(
            fixture.catalog_text().lines().count(),
            2,
            "one record line and one alias line"
        );
    }

    /// R1-001 (renderer risk review): the publication surface is frozen
    /// while any supervised process runs -- a spike default mirroring the
    /// renderer's own guard so direct IPC cannot bypass it -- with nothing
    /// recorded, and open again once none does.
    #[test]
    fn approving_and_publishing_are_refused_while_a_run_is_active() {
        let fixture = Fixture::new("run-active");
        let request = ApproveRequest {
            run_id: "run-1".to_string(),
            name: "run-1/report.pdf".to_string(),
            sha256: hex(PDF),
        };
        let error = approve_candidate(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Busy,
            Fixture::now(),
            &request,
        )
        .expect_err("frozen while a run is active");
        assert_eq!(error.code, ShellErrorCode::RunActive);
        assert!(!error.message.contains('/'));
        assert!(
            fixture.journal_text().is_empty(),
            "nothing recorded while frozen"
        );

        let approval = fixture
            .approve("report.pdf", PDF)
            .expect("open again once no run is active");
        let mut frames = Vec::new();
        let error = publish_approved(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Busy,
            &approval.approval_id,
            &mut |frame| frames.push(frame),
        )
        .expect_err("frozen while a run is active");
        assert_eq!(error.code, ShellErrorCode::RunActive);
        assert!(frames.is_empty());
        assert_eq!(fixture.copies(), 0);
        assert!(fixture.run_dir.join("report.pdf").exists());
        assert_eq!(
            fixture.journal_text().lines().count(),
            1,
            "only the approval is recorded"
        );

        let dto = fixture
            .publish(&approval.approval_id)
            .0
            .expect("published once the run ended");
        assert_eq!(dto.state, ArtifactStateTag::Registered);
    }

    /// Unix: the same guard against a real supervised child -- `sleep`,
    /// spawned and stopped through the supervisor, the source
    /// `harness_stop` and `harness_observe` consult -- refused while it
    /// runs, allowed after its terminal state. (The Windows runner has no
    /// `sleep`; the fake-driven test above covers the logic there.)
    #[cfg(unix)]
    #[test]
    fn a_live_supervised_process_freezes_the_surface_until_it_ends() {
        use omnifrons_app::{ProcessSpec, ProcessSupervisor as _};
        let fixture = Fixture::new("run-active-live");
        let mut supervisor = omnifrons_supervisor::TokioProcessSupervisor::with_demo_launcher(
            std::path::PathBuf::from("unused-launcher"),
        );
        let id = supervisor
            .spawn(ProcessSpec::new("sleep").with_args(["30"]))
            .expect("spawn a live child");
        let request = ApproveRequest {
            run_id: "run-1".to_string(),
            name: "run-1/report.pdf".to_string(),
            sha256: hex(PDF),
        };
        let error = approve_candidate(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &supervisor,
            Fixture::now(),
            &request,
        )
        .expect_err("frozen while the child runs");
        assert_eq!(error.code, ShellErrorCode::RunActive);

        supervisor
            .stop(id, Duration::from_secs(5))
            .expect("the child stops within the deadline");
        let approval = approve_candidate(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &supervisor,
            Fixture::now(),
            &request,
        )
        .expect("open once the child reached its terminal state");
        let mut frames = Vec::new();
        let dto = publish_approved(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &supervisor,
            &approval.approval_id,
            &mut |frame| frames.push(frame),
        )
        .expect("published with no run active");
        assert_eq!(dto.state, ArtifactStateTag::Registered);
    }

    /// R3-006: an approval recorded under project A cannot publish while
    /// project B is active -- the journal is device-wide, the approval is
    /// not -- and nothing of B's is touched.
    #[test]
    fn publishing_an_approval_under_another_project_is_invalid_request_and_touches_nothing() {
        let fixture = Fixture::new("other-project");
        let approval = fixture
            .approve("report.pdf", PDF)
            .expect("approved under project A");
        let other = TempDir::new("other-project-b");
        let policy_path = other.path().join(POLICY_FILE_PATH);
        std::fs::create_dir_all(policy_path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&policy_path, r#"{"schema": 1, "assetRootId": "main"}"#).expect("policy");
        let mut frames = Vec::new();
        let error = publish_approved(
            &fixture.outbox,
            &other.workspace(),
            &fixture.publication,
            &Idle,
            &approval.approval_id,
            &mut |frame| frames.push(frame),
        )
        .expect_err("another project's approval");
        assert_eq!(error.code, ShellErrorCode::InvalidRequest);
        assert!(frames.is_empty());
        assert!(
            !other.path().join(".omnifrons/catalog.jsonl").exists(),
            "nothing written to B's catalog"
        );
        assert!(
            fixture.run_dir.join("report.pdf").exists(),
            "A's entry untouched"
        );
        assert_eq!(fixture.copies(), 0, "nothing copied");
    }
}
