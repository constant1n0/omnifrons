//! The wrong-root commands (spike slice 5d, HAP-001 § Wrong-root
//! detection and remedies, D16 at the post-run-scan half of its default):
//! `wrongroot_status`, `wrongroot_scan`, `misplaced_list`, and
//! `misplaced_remedy`, registered next to the guidance and publication
//! commands. Each is a thin wrapper over a pure body tested here without a
//! Tauri runtime, composing the application services
//! (`omnifrons_app::wrong_root`, `omnifrons_app::quarantine`,
//! `omnifrons_app::ignore_ledger`) with the real adapters.
//!
//! **A scan observes; a remedy acts.** The scan holds no handle, takes no
//! lock, and runs while a process is live -- the run-end scan is exactly
//! that. Every remedy takes the publication surface lock and refuses with
//! `run-active` while any supervised process runs, the same guard the
//! approval, publication, and guidance-write surfaces hold, so direct IPC
//! cannot bypass what the renderer refuses.
//!
//! **A remedy names a finding by name and digest, never by a path**
//! (RCS-001-R14), and the digest binds the request exactly as
//! `artifact_approve` binds a candidate (HAP-001-R22): a name whose bytes
//! changed since the surface showed them is `misplaced-unknown`, and
//! nothing moves.

use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use omnifrons_adapters::{
    FsCandidateProber, FsOutboxEntryOps, FsQuarantineStore, FsWrongRootScanner,
    JsonOutboxPolicyStore, JsonlIgnoreLedger, Sha2Hasher,
};
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::content_hasher::derive_project_identity;
use omnifrons_app::harness_adapter::AdapterDescriptor;
use omnifrons_app::ignore_ledger::{
    IgnoreEntry, IgnoreLedger as _, IgnoreLedgerError, retain_unignored,
};
use omnifrons_app::outbox_policy::OutboxPolicyStore as _;
use omnifrons_app::quarantine::{
    QuarantineError, QuarantineMove, QuarantinePorts, QuarantineRoot, quarantine,
};
use omnifrons_app::run_outbox::{PrepareError, RunOutboxPreparer as _};
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_app::wrong_root::{
    CopyInError, CopyInPorts, OpenMisplacedError, ScanError, ScanRequest, WrongRootScanner as _,
    copy_in_from_misplaced,
};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::publication::{DisplayName, ProjectIdentity};
use omnifrons_domain::scope::ScopeMode;
use omnifrons_domain::wrong_root::{DeclaredWriteSet, MisplacedFinding, OutputDiscipline, Remedy};
use omnifrons_supervisor::TokioProcessSupervisor;
use tauri::{AppHandle, Manager as _};

use crate::adapter_state::AdapterState;
use crate::ipc::commands::active_workspace;
use crate::ipc::dto::{
    MisplacedDto, MisplacedRemedyDto, MisplacedScanDto, RemedyOutcomeTag, RemedyTag, ShellError,
    ShellErrorCode, WrongRootStatusDto,
};
use crate::ipc::publication::{RunActivity, run_active};
use crate::outbox_state::OutboxState;
use crate::publication_state::PublicationState;
use crate::wrong_root_state::{ScanRecord, WrongRootState};

impl From<ScanError> for ShellError {
    fn from(error: ScanError) -> Self {
        let message = match error {
            ScanError::Unreadable => "the project could not be scanned",
        };
        Self::new(ShellErrorCode::ScanFailed, message)
    }
}

impl From<IgnoreLedgerError> for ShellError {
    /// The ignore ledger is the work area's, exactly as the publication
    /// journal is: a ledger that cannot be read or written is a work area
    /// that cannot be used.
    fn from(error: IgnoreLedgerError) -> Self {
        let message = match error {
            IgnoreLedgerError::Unreadable => "the ignore ledger could not be read",
            IgnoreLedgerError::Corrupt => "the ignore ledger is corrupt",
            IgnoreLedgerError::WriteFailed => "the ignore ledger could not be written",
        };
        Self::new(ShellErrorCode::WorkAreaInvalid, message)
    }
}

impl From<QuarantineError> for ShellError {
    /// Every containment and write failure of the quarantine directory is
    /// `quarantine-unavailable`; a fact about the *file* that no longer
    /// holds -- not a regular file, a name that no longer holds it,
    /// content that changed -- is `refused`, the same code the approval
    /// surface uses when a bound fact stops holding.
    fn from(error: QuarantineError) -> Self {
        match error {
            QuarantineError::Unusable => Self::new(
                ShellErrorCode::QuarantineUnavailable,
                "the quarantine directory could not be used",
            ),
            QuarantineError::InsideWorkspace => Self::new(
                ShellErrorCode::QuarantineUnavailable,
                "the quarantine directory resolves inside a registered workspace root",
            ),
            QuarantineError::NotOwnerOnly => Self::new(
                ShellErrorCode::QuarantineUnavailable,
                "the quarantine directory could not be made owner-only",
            ),
            QuarantineError::CopyFailed => Self::new(
                ShellErrorCode::QuarantineUnavailable,
                "the file could not be moved into quarantine",
            ),
            // A destination answering to a second name is a fact about the
            // quarantine directory, not about the file the user asked
            // about, so it is `quarantine-unavailable` beside the other
            // write failures rather than `refused` (R1-022). The count
            // itself stays out of the message: a payload carries logical
            // values, and nothing about the producer's other name is one.
            QuarantineError::DestinationLinked { .. } => Self::new(
                ShellErrorCode::QuarantineUnavailable,
                "the quarantine destination has more than one name",
            ),
            QuarantineError::NotRegularFile => {
                Self::new(ShellErrorCode::Refused, "the file is not a regular file")
            }
            QuarantineError::PathChanged => Self::new(
                ShellErrorCode::Refused,
                "the name no longer holds the file that was opened",
            ),
            QuarantineError::DigestChanged => Self::new(
                ShellErrorCode::Refused,
                "the file's content changed since it was found",
            ),
        }
    }
}

impl From<CopyInError> for ShellError {
    fn from(error: CopyInError) -> Self {
        match error {
            CopyInError::NotRegularFile => {
                Self::new(ShellErrorCode::Refused, "the file is not a regular file")
            }
            CopyInError::PathChanged => Self::new(
                ShellErrorCode::Refused,
                "the name no longer holds the file that was opened",
            ),
            CopyInError::DigestChanged => Self::new(
                ShellErrorCode::Refused,
                "the file's content changed since it was found",
            ),
            CopyInError::CopyFailed => Self::new(
                ShellErrorCode::OutboxUnavailable,
                "the file could not be copied into the outbox",
            ),
            CopyInError::EntryExists => Self::new(
                ShellErrorCode::OutboxUnavailable,
                "an outbox entry of that name already exists with other content",
            ),
            // HAP-001-R20 applied with the misplaced file as the candidate
            // (HAP-001-R32): the same code the outbox path uses for the
            // same fact, so one link-count refusal reads the same
            // everywhere.
            CopyInError::Linked { .. } => Self::new(
                ShellErrorCode::OutboxLinked,
                "the file's link count is greater than one",
            ),
        }
    }
}

impl From<OpenMisplacedError> for ShellError {
    fn from(error: OpenMisplacedError) -> Self {
        match error {
            OpenMisplacedError::InvalidName => Self::new(
                ShellErrorCode::InvalidRequest,
                "the name is not a project-relative path",
            ),
            OpenMisplacedError::NotFound => Self::new(
                ShellErrorCode::MisplacedUnknown,
                "nothing sits at that name any more; scan again",
            ),
            OpenMisplacedError::NotRegularFile => Self::new(
                ShellErrorCode::Refused,
                "that name does not hold a regular file",
            ),
            OpenMisplacedError::Unreadable => {
                Self::new(ShellErrorCode::Refused, "that file could not be read")
            }
            OpenMisplacedError::Linked { .. } => Self::new(
                ShellErrorCode::OutboxLinked,
                "the file's link count is greater than one",
            ),
        }
    }
}

/// The project identity of `workspace` -- the key every read of the scan
/// table is taken under (spike slice 5d, R1-005). A logical value derived
/// from the canonical path, never the path (HAP-001-R5, RCS-001-R14).
#[must_use]
pub fn project_identity_of(workspace: &WorkspaceRoot) -> ProjectIdentity {
    derive_project_identity(&Sha2Hasher::new(), workspace)
}

fn invalid(message: &str) -> ShellError {
    ShellError::new(ShellErrorCode::InvalidRequest, message)
}

fn unknown_finding() -> ShellError {
    ShellError::new(
        ShellErrorCode::MisplacedUnknown,
        "no scan reported that file at that digest; scan again and retry",
    )
}

/// `misplaced_remedy`'s request: the finding named by the name and full
/// digest the last scan reported, and which of the three remedies to run.
/// Never a path (RCS-001-R14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemedyRequest {
    pub name: String,
    pub sha256: String,
    pub remedy: RemedyTag,
}

/// The scope mode this project's output discipline is reported under: the
/// weakest mode any registered adapter declares, `advisory` for an empty
/// catalog.
///
/// A spike default, and a deliberately conservative one: the product
/// cannot claim `enforced` because *some* adapter is sandboxed while
/// another is not, and every built-in adapter of this slice declares
/// `advisory` anyway (`AdapterDescriptor::scope_mode`).
#[must_use]
pub fn reported_scope_mode(descriptors: &[AdapterDescriptor]) -> ScopeMode {
    let mut weakest = ScopeMode::SandboxEnforced;
    for descriptor in descriptors {
        weakest = match (weakest, descriptor.scope_mode) {
            (ScopeMode::Advisory, _) | (_, ScopeMode::Advisory) => ScopeMode::Advisory,
            (ScopeMode::HarnessEnforced, _) | (_, ScopeMode::HarnessEnforced) => {
                ScopeMode::HarnessEnforced
            }
            _ => ScopeMode::SandboxEnforced,
        };
    }
    if descriptors.is_empty() {
        ScopeMode::Advisory
    } else {
        weakest
    }
}

/// The output-discipline report and whether a scan has run
/// (`wrongroot_status`; HAP-001-R33, R34).
///
/// The declared write set is the project root: a launch plan sets the
/// harness's `cwd` to the active workspace root and declares its output
/// directory inside it (`validate_cwd_within_workspace`,
/// `OUTPUT_DIR_ENV_KEY`), and nothing wider is ever declared. Whether that
/// write set is *enforced* is exactly what the scope mode decides, which
/// is why HAP-001-R33 needs both facts.
#[must_use]
pub fn wrong_root_status_for(
    descriptors: &[AdapterDescriptor],
    wrong_root: &WrongRootState,
    project: Option<&ProjectIdentity>,
) -> WrongRootStatusDto {
    let mode = reported_scope_mode(descriptors);
    let discipline = OutputDiscipline::for_scope(mode, DeclaredWriteSet::ProjectRoot);
    let (scanned, findings) = wrong_root.summary(project);
    WrongRootStatusDto::of(discipline, mode, scanned, findings)
}

/// Scan the active workspace root for misplaced files (`wrongroot_scan`;
/// HAP-001-R32, R43, R44), filter what the ignore ledger already covers,
/// and remember the rest for `misplaced_list`.
///
/// Observation only: no handle is held, no lock is taken, and a live run
/// does not freeze it -- the run-end scan is this same body.
///
/// # Errors
///
/// `outbox-invalid` when the project's classification policy cannot be
/// loaded; `scan-failed` when the project root cannot be walked;
/// `work-area-invalid` when the ignore ledger cannot be opened or read.
pub fn wrong_root_scan_for(
    outbox: &OutboxState,
    workspace: &WorkspaceRoot,
    publication: &PublicationState,
    wrong_root: &WrongRootState,
) -> Result<MisplacedScanDto, ShellError> {
    scan_project(
        outbox.policy_store,
        workspace,
        &publication.work_area,
        wrong_root,
    )
}

/// The scan itself, over the pieces it needs rather than the managed
/// states: the run-end tracker captures exactly these when its forwarder
/// thread starts, so `RunEndTracker::finish` never touches Tauri state
/// (spike slice 5d).
///
/// # Errors
///
/// As [`wrong_root_scan_for`].
pub fn scan_project(
    policy_store: JsonOutboxPolicyStore,
    workspace: &WorkspaceRoot,
    work_area_path: &Path,
    wrong_root: &WrongRootState,
) -> Result<MisplacedScanDto, ShellError> {
    // HAP-001-R7 **before the walk**, not after it (spike slice 5d,
    // R3-014). The work area is excluded from the scan by construction --
    // it never resolves inside a registered workspace root -- and a work
    // area that does is one this walk would descend. Deciding that
    // afterwards means it already has, and the exclusion was never real.
    let work_area = WorkAreaRoot::open(work_area_path, &[workspace])?;
    let ledger = JsonlIgnoreLedger::open(&work_area)?;
    let project = project_identity_of(workspace);

    let policy = policy_store.load(workspace)?;
    let report = FsWrongRootScanner::new().scan(&ScanRequest {
        project: workspace,
        outbox: policy.outbox(),
        classifier: &policy,
    })?;

    let (findings, ignored) = retain_unignored(report.findings, &ledger, &project)?;

    let record = ScanRecord {
        project,
        findings,
        scanned: report.scanned,
        excluded: report.excluded,
        ignored,
        unreadable: report.unreadable,
        truncated: report.truncated,
    };
    let summary = MisplacedScanDto {
        scanned: record.scanned,
        findings: u32::try_from(record.findings.len()).unwrap_or(u32::MAX),
        ignored: record.ignored,
        excluded: record.excluded,
        unreadable: record.unreadable,
        truncated: record.truncated,
    };
    wrong_root.record(record);
    Ok(summary)
}

/// Every finding the last scan left standing (`misplaced_list`), in walk
/// order. Empty when no scan has run.
#[must_use]
pub fn misplaced_list_for(
    wrong_root: &WrongRootState,
    project: Option<&ProjectIdentity>,
) -> Vec<MisplacedDto> {
    wrong_root
        .findings(project)
        .iter()
        .map(MisplacedDto::from_finding)
        .collect()
}

/// Run one of HAP-001-R32's three remedies over the finding `request`
/// names (`misplaced_remedy`). Nothing happens without this explicit
/// choice, and nothing else is ever offered.
///
/// # Errors
///
/// `run-active` while a supervised process runs; `invalid-request` for a
/// digest that is not 64 hex characters; `misplaced-unknown` for a name
/// and digest the last scan did not report; `refused` when a fact bound at
/// scan time no longer holds; `quarantine-unavailable`,
/// `outbox-unavailable`, `outbox-invalid`, and `work-area-invalid` as the
/// mappings above spell them.
pub fn misplaced_remedy_for(
    outbox: &OutboxState,
    workspace: &WorkspaceRoot,
    publication: &PublicationState,
    wrong_root: &WrongRootState,
    run_activity: &dyn RunActivity,
    now: SystemTime,
    request: &RemedyRequest,
) -> Result<MisplacedRemedyDto, ShellError> {
    // The same surface lock and the same live-run guard the approval,
    // publication, and guidance-write commands hold (spike slice 5b,
    // renderer risk review R1-001).
    let _surface = publication.lock_surface();
    if run_activity.any_running() {
        return Err(run_active());
    }
    let digest = Sha256Digest::from_hex(&request.sha256)
        .ok_or_else(|| invalid("the digest is not 64 hex characters"))?;
    // The record must be *this* project's: the remedy resolves the name
    // against the active workspace, so a finding from any other project
    // would act on a file this session never scanned (R1-005).
    let project = project_identity_of(workspace);
    let finding = wrong_root
        .find(&project, &request.name, &digest)
        .ok_or_else(unknown_finding)?;

    let remedy = Remedy::from(request.remedy);
    let dto = match remedy {
        Remedy::Ignore => {
            record_ignore(workspace, publication, &project, &finding, now)?;
            MisplacedRemedyDto {
                remedy: request.remedy,
                outcome: RemedyOutcomeTag::Ignored,
                name: None,
                sha256: Some(finding.digest.to_hex()),
                original_kept: Some(true),
                detail: None,
            }
        }
        Remedy::Quarantine => run_quarantine(workspace, publication, &finding)?,
        Remedy::Publish => run_publish(outbox, workspace, &finding)?,
    };
    wrong_root.forget(&project, &request.name, &digest);
    Ok(dto)
}

/// The ignore remedy: the decision recorded against the finding's name and
/// digest in the work area's ledger. The file is left exactly where it is.
fn record_ignore(
    workspace: &WorkspaceRoot,
    publication: &PublicationState,
    project: &ProjectIdentity,
    finding: &MisplacedFinding,
    now: SystemTime,
) -> Result<(), ShellError> {
    let work_area = WorkAreaRoot::open(&publication.work_area, &[workspace])?;
    let mut ledger = JsonlIgnoreLedger::open(&work_area)?;
    ledger.record(project, &IgnoreEntry::of(finding, now))?;
    Ok(())
}

/// The display name a remedy sanitizes for the file it is acting on: the
/// last component of the finding's project-relative name.
fn display_name_of(finding: &MisplacedFinding) -> DisplayName {
    let last = finding
        .name()
        .rsplit('/')
        .next()
        .unwrap_or_else(|| finding.name());
    DisplayName::sanitize(last)
}

/// The quarantine remedy: the file is moved into the product's quarantine
/// directory, outside any workspace (RCS-001-R10). The one remedy that
/// removes the original.
fn run_quarantine(
    workspace: &WorkspaceRoot,
    publication: &PublicationState,
    finding: &MisplacedFinding,
) -> Result<MisplacedRemedyDto, ShellError> {
    let root = QuarantineRoot::open(&publication.quarantine, &[workspace])?;
    let mut source = FsWrongRootScanner::new().open_misplaced(workspace, finding.name())?;
    let hasher = Sha2Hasher::new();
    let quarantined = quarantine(
        &QuarantinePorts {
            store: &FsQuarantineStore::new(),
            hasher: &hasher,
            entry_ops: &FsOutboxEntryOps::new(),
            root: &root,
            workspaces: &[workspace],
        },
        &mut source,
        &finding.digest,
        &display_name_of(finding),
    )?;
    let (original_kept, detail) = quarantine_detail(&quarantined.outcome);
    Ok(MisplacedRemedyDto {
        remedy: RemedyTag::Quarantine,
        outcome: RemedyOutcomeTag::Quarantined,
        name: Some(quarantined.name),
        sha256: Some(quarantined.digest.to_hex()),
        original_kept: Some(original_kept),
        detail: Some(detail.to_string()),
    })
}

/// What one completed quarantine move puts on the wire: whether the
/// original is still where it was, and the fixed `detail` token naming how
/// the move went. Never free text and never a path (HAP-001-R5,
/// RCS-001-R14).
///
/// Pure and exhaustive over [`QuarantineMove`], so every token the wire
/// fixes has one producer and can be pinned by one test rather than only
/// by whichever branch a platform happens to take (spike slice 5d).
#[must_use]
pub fn quarantine_detail(outcome: &QuarantineMove) -> (bool, &'static str) {
    match outcome {
        QuarantineMove::Renamed => (false, "renamed"),
        // The rename ran, so the original name no longer holds the file;
        // only the destination verification did not complete.
        // `originalKept: false` is the fact, and the token says which half
        // failed.
        QuarantineMove::RenamedUnverified => (false, "renamed-unverified"),
        QuarantineMove::CopiedAndUnlinked { fallback } => (false, fallback.as_str()),
        QuarantineMove::CopiedOriginalKept { reason, .. } => (true, reason),
    }
}

/// The publish remedy: the bytes are copied from the one held handle into
/// the outbox root as a new unattributed entry, and the original is left
/// exactly where it was found (HAP-001-R28).
///
/// Nothing is published here. The new entry is an ordinary outbox entry
/// that the existing whole-outbox path takes over: `candidates_list
/// { runId: null }` lists it, `artifact_approve` takes its own explicit
/// per-artifact approval (HAP-001-R22, D3), and `artifact_publish` runs
/// the transaction -- over an entry the product itself created, which is
/// what lets that transaction's cleanup unlink it (HAP-001-R28).
fn run_publish(
    outbox: &OutboxState,
    workspace: &WorkspaceRoot,
    finding: &MisplacedFinding,
) -> Result<MisplacedRemedyDto, ShellError> {
    let policy = outbox.policy_store.load(workspace)?;
    let opened = match outbox
        .preparer
        .open_outbox(workspace, policy.outbox(), true)
    {
        Ok(opened) => opened,
        Err(PrepareError::Declaration(error)) => return Err(ShellError::from(error)),
        Err(error) => return Err(ShellError::from(error)),
    };
    let mut source = FsWrongRootScanner::new().open_misplaced(workspace, finding.name())?;
    let hasher = Sha2Hasher::new();
    let copied = copy_in_from_misplaced(
        &CopyInPorts {
            hasher: &hasher,
            entry_ops: &FsOutboxEntryOps::new(),
            prober: &FsCandidateProber::new(),
        },
        &mut source,
        &opened,
        &finding.digest,
        &display_name_of(finding),
    )?;
    let dto = MisplacedRemedyDto {
        remedy: RemedyTag::Publish,
        outcome: RemedyOutcomeTag::CopiedToOutbox,
        name: Some(copied.name),
        sha256: Some(copied.digest.to_hex()),
        original_kept: Some(true),
        detail: None,
    };
    // The copy's own handle is released here rather than held: the entry
    // is approved and published through the existing whole-outbox path,
    // which re-opens it under HAP-001-R15 when its turn comes, so the
    // remedy costs nothing against HAP-001 D22's 32-handle cap.
    drop(copied.source);
    Ok(dto)
}

/// The output-discipline report and whether a scan has run.
///
/// # Errors
///
/// Never fails; the `Result` matches every other command's shape.
#[tauri::command]
pub async fn wrongroot_status(app: AppHandle) -> Result<WrongRootStatusDto, ShellError> {
    let descriptors = app.state::<AdapterState>().catalog.descriptors();
    let project = active_project(&app);
    let wrong_root = app.state::<Arc<WrongRootState>>();
    Ok(wrong_root_status_for(
        &descriptors,
        &wrong_root,
        project.as_ref(),
    ))
}

/// The active workspace's project identity, or `None` when none is active.
///
/// Every read of the scan table is taken under it, so a table this process
/// shares between projects can never answer for one with another's scan
/// (spike slice 5d, R1-005). Neither status nor listing *fails* without a
/// workspace: they answer exactly as they would for a project no scan has
/// run for.
fn active_project(app: &AppHandle) -> Option<ProjectIdentity> {
    active_workspace(app)
        .ok()
        .map(|workspace| project_identity_of(&workspace))
}

/// Scan the active workspace root for misplaced files.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`wrong_root_scan_for`]'s errors.
#[tauri::command]
pub async fn wrongroot_scan(app: AppHandle) -> Result<MisplacedScanDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let outbox = app.state::<OutboxState>();
        let publication = app.state::<PublicationState>();
        let wrong_root = app.state::<Arc<WrongRootState>>();
        wrong_root_scan_for(&outbox, &workspace, &publication, &wrong_root)
    })
    .await
    .expect("the blocking wrong-root scan task panicked")
}

/// Every finding the last scan left standing.
///
/// # Errors
///
/// Never fails; the `Result` matches every other command's shape.
#[tauri::command]
pub async fn misplaced_list(app: AppHandle) -> Result<Vec<MisplacedDto>, ShellError> {
    let project = active_project(&app);
    Ok(misplaced_list_for(
        &app.state::<Arc<WrongRootState>>(),
        project.as_ref(),
    ))
}

/// Run one of the three remedies over the finding named by `name` and
/// `sha256`.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`misplaced_remedy_for`]'s errors.
#[tauri::command]
pub async fn misplaced_remedy(
    app: AppHandle,
    name: String,
    sha256: String,
    remedy: RemedyTag,
) -> Result<MisplacedRemedyDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let outbox = app.state::<OutboxState>();
        let publication = app.state::<PublicationState>();
        let wrong_root = app.state::<Arc<WrongRootState>>();
        let supervisor = app.state::<TokioProcessSupervisor>().inner().clone();
        misplaced_remedy_for(
            &outbox,
            &workspace,
            &publication,
            &wrong_root,
            &supervisor,
            SystemTime::now(),
            &RemedyRequest {
                name,
                sha256,
                remedy,
            },
        )
    })
    .await
    .expect("the blocking wrong-root remedy task panicked")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime};

    use omnifrons_adapters::Sha2Hasher;
    use omnifrons_app::WorkspaceRoot;
    use omnifrons_app::content_hasher::ContentHasher as _;
    use omnifrons_domain::scope::ScopeMode;

    use super::{
        RemedyRequest, misplaced_list_for, misplaced_remedy_for, reported_scope_mode,
        wrong_root_scan_for, wrong_root_status_for,
    };
    use crate::ipc::dto::{RemedyOutcomeTag, RemedyTag, ShellErrorCode};
    use crate::ipc::publication::RunActivity;
    use crate::outbox_state::OutboxState;
    use crate::publication_state::PublicationState;
    use crate::wrong_root_state::WrongRootState;

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

    /// A drop-guard temp directory.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "omnifrons-shell-wrong-root-{}-{label}-{n}",
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

    fn pdf_bytes() -> Vec<u8> {
        let mut bytes = b"%PDF-1.7\n".to_vec();
        bytes.extend_from_slice(&[9u8; 96]);
        bytes
    }

    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }

    /// A project with a heavy file at `docs/report.pdf`, the device
    /// directory holding the work area and the quarantine, and the shell's
    /// three states.
    struct Fixture {
        project: TempDir,
        device: TempDir,
        identity: omnifrons_domain::publication::ProjectIdentity,
        outbox: OutboxState,
        publication: PublicationState,
        wrong_root: WrongRootState,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let project = TempDir::new(label);
            let device = TempDir::new(&format!("{label}-device"));
            std::fs::create_dir_all(project.path().join("docs")).expect("fixture dir");
            std::fs::write(project.path().join("docs/report.pdf"), pdf_bytes())
                .expect("fixture file");
            let publication = PublicationState::under(device.path());
            let identity = super::project_identity_of(&project.workspace());
            Self {
                project,
                device,
                identity,
                outbox: OutboxState::new(),
                publication,
                wrong_root: WrongRootState::new(),
            }
        }

        fn workspace(&self) -> WorkspaceRoot {
            self.project.workspace()
        }

        fn scan(&self) -> crate::ipc::dto::MisplacedScanDto {
            wrong_root_scan_for(
                &self.outbox,
                &self.workspace(),
                &self.publication,
                &self.wrong_root,
            )
            .expect("a real worktree scans")
        }

        fn digest() -> String {
            Sha2Hasher::new().sha256(&pdf_bytes()).to_hex()
        }

        /// The identity every read of the scan table is keyed to
        /// (R1-005). `Option`, because that is the shape every reader
        /// takes: a session with no active workspace has none.
        #[allow(clippy::unnecessary_wraps)]
        fn project_identity(&self) -> Option<&omnifrons_domain::publication::ProjectIdentity> {
            Some(&self.identity)
        }
    }

    /// HAP-001-R33 and R34: every built-in adapter of this slice declares
    /// `advisory` scope, so the reported discipline is `advisory` and both
    /// disclosures ride the payload; the status also says whether a scan
    /// has run.
    #[test]
    fn the_status_reports_the_discipline_its_disclosures_and_whether_a_scan_has_run() {
        let fixture = Fixture::new("status");
        let descriptors = omnifrons_adapters::catalog()
            .iter()
            .map(|adapter| adapter.describe())
            .collect::<Vec<_>>();
        assert_eq!(
            reported_scope_mode(&descriptors),
            ScopeMode::Advisory,
            "the built-in adapters of this slice are all advisory"
        );
        let status = wrong_root_status_for(
            &descriptors,
            &fixture.wrong_root,
            fixture.project_identity(),
        );
        assert_eq!(status.disclosures.len(), 2);
        assert!(!status.scanned);
        assert_eq!(status.findings, 0);

        fixture.scan();
        let after = wrong_root_status_for(
            &descriptors,
            &fixture.wrong_root,
            fixture.project_identity(),
        );
        assert!(after.scanned);
        assert_eq!(after.findings, 1);
    }

    /// HAP-001-R32 with R43: the heavy file outside the outbox is listed
    /// with its digest and the three remedies, by a project-relative name
    /// and never a device path (RCS-001-R14).
    #[test]
    fn a_scan_lists_the_misplaced_file_by_a_relative_name_and_never_a_path() {
        let fixture = Fixture::new("list");
        let summary = fixture.scan();
        assert_eq!(summary.findings, 1);
        assert!(!summary.truncated);

        let rows = misplaced_list_for(&fixture.wrong_root, fixture.project_identity());
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.name, "docs/report.pdf");
        assert_eq!(row.sha256, Fixture::digest());
        assert_eq!(row.class, "generated-heavy");
        assert_eq!(row.reason, "in-project-outside-outbox");
        assert_eq!(row.remedies, vec!["quarantine", "publish", "ignore"]);
        let json = serde_json::to_string(&rows).expect("serializable");
        let device = fixture.project.path().to_string_lossy().into_owned();
        assert!(
            !json.contains(&device),
            "no device path may cross the wire in a finding"
        );
    }

    /// The ignore remedy: the decision is recorded against the name and
    /// digest, and the next scan withholds the file until its content
    /// changes.
    #[test]
    fn the_ignore_remedy_withholds_the_file_from_the_next_scan() {
        let fixture = Fixture::new("ignore");
        fixture.scan();
        let remedied = misplaced_remedy_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &fixture.wrong_root,
            &Idle,
            now(),
            &RemedyRequest {
                name: "docs/report.pdf".to_string(),
                sha256: Fixture::digest(),
                remedy: RemedyTag::Ignore,
            },
        )
        .expect("the ignore remedy records the decision");
        assert_eq!(remedied.outcome, RemedyOutcomeTag::Ignored);
        assert!(misplaced_list_for(&fixture.wrong_root, fixture.project_identity()).is_empty());

        let again = fixture.scan();
        assert_eq!(again.findings, 0, "an ignored file is not offered again");
        assert_eq!(again.ignored, 1, "and the count says why");

        // Its content changes: offered again.
        let mut changed = pdf_bytes();
        changed.push(1);
        std::fs::write(fixture.project.path().join("docs/report.pdf"), &changed)
            .expect("rewrite the fixture");
        let after_change = fixture.scan();
        assert_eq!(
            after_change.findings, 1,
            "the decision binds to the digest, so changed content is offered again"
        );
    }

    /// The quarantine remedy: the one remedy that removes the original.
    #[test]
    fn the_quarantine_remedy_moves_the_file_out_of_the_project() {
        let fixture = Fixture::new("quarantine");
        fixture.scan();
        let remedied = misplaced_remedy_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &fixture.wrong_root,
            &Idle,
            now(),
            &RemedyRequest {
                name: "docs/report.pdf".to_string(),
                sha256: Fixture::digest(),
                remedy: RemedyTag::Quarantine,
            },
        )
        .expect("the quarantine remedy moves the file");
        assert_eq!(remedied.outcome, RemedyOutcomeTag::Quarantined);
        let name = remedied.name.expect("a quarantined name");
        assert!(!name.contains('/') && !name.contains('\\'));
        assert_eq!(remedied.sha256.as_deref(), Some(Fixture::digest().as_str()));
        assert!(
            fixture
                .device
                .path()
                .join(crate::publication_state::QUARANTINE_DIR)
                .join(&name)
                .is_file(),
            "the file now sits in the product's quarantine directory"
        );
        #[cfg(unix)]
        assert!(
            !fixture.project.path().join("docs/report.pdf").exists(),
            "quarantine is the one remedy that removes the original"
        );
        #[cfg(not(unix))]
        assert_eq!(
            remedied.original_kept,
            Some(true),
            "without a by-handle unlink the original is kept and the payload says so"
        );
        assert!(misplaced_list_for(&fixture.wrong_root, fixture.project_identity()).is_empty());
    }

    /// The publish remedy: the bytes reach the outbox as a new entry and
    /// **the original stays exactly where it was found** (HAP-001-R28).
    /// Nothing is published by the remedy itself: the new entry goes
    /// through the existing whole-outbox approval path.
    #[test]
    fn the_publish_remedy_copies_into_the_outbox_and_leaves_the_original() {
        let fixture = Fixture::new("publish");
        fixture.scan();
        let remedied = misplaced_remedy_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &fixture.wrong_root,
            &Idle,
            now(),
            &RemedyRequest {
                name: "docs/report.pdf".to_string(),
                sha256: Fixture::digest(),
                remedy: RemedyTag::Publish,
            },
        )
        .expect("the publish remedy copies into the outbox");
        assert_eq!(remedied.outcome, RemedyOutcomeTag::CopiedToOutbox);
        let name = remedied.name.expect("the new entry's name");
        assert!(!name.contains('/'), "one component at the outbox root");
        assert_eq!(
            std::fs::read(fixture.project.path().join(".omnifrons/outbox").join(&name))
                .expect("the new outbox entry"),
            pdf_bytes()
        );
        assert!(
            fixture.project.path().join("docs/report.pdf").exists(),
            "the publish remedy never removes the original"
        );
    }

    /// The remedy binds to a name *and* a digest the last scan reported;
    /// anything else is `misplaced-unknown`, and nothing happens.
    #[test]
    fn a_remedy_for_a_finding_this_scan_never_reported_is_refused() {
        let fixture = Fixture::new("unknown");
        fixture.scan();
        let cases = [
            ("docs/other.pdf", Fixture::digest()),
            ("docs/report.pdf", "ab".repeat(32)),
        ];
        for (name, sha256) in cases {
            let error = misplaced_remedy_for(
                &fixture.outbox,
                &fixture.workspace(),
                &fixture.publication,
                &fixture.wrong_root,
                &Idle,
                now(),
                &RemedyRequest {
                    name: name.to_string(),
                    sha256,
                    remedy: RemedyTag::Quarantine,
                },
            )
            .expect_err("a finding this scan never reported is refused");
            assert_eq!(error.code, ShellErrorCode::MisplacedUnknown);
        }
        assert!(fixture.project.path().join("docs/report.pdf").exists());

        let malformed = misplaced_remedy_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &fixture.wrong_root,
            &Idle,
            now(),
            &RemedyRequest {
                name: "docs/report.pdf".to_string(),
                sha256: "not-hex".to_string(),
                remedy: RemedyTag::Ignore,
            },
        )
        .expect_err("a malformed digest is an invalid request");
        assert_eq!(malformed.code, ShellErrorCode::InvalidRequest);
    }

    /// Every remedy takes the publication surface lock and refuses with
    /// `run-active` while a supervised process runs -- the same guard the
    /// approval, publication, and guidance-write surfaces hold.
    #[test]
    fn a_remedy_is_refused_while_a_run_is_active() {
        let fixture = Fixture::new("run-active");
        fixture.scan();
        for remedy in [RemedyTag::Quarantine, RemedyTag::Publish, RemedyTag::Ignore] {
            let error = misplaced_remedy_for(
                &fixture.outbox,
                &fixture.workspace(),
                &fixture.publication,
                &fixture.wrong_root,
                &Busy,
                now(),
                &RemedyRequest {
                    name: "docs/report.pdf".to_string(),
                    sha256: Fixture::digest(),
                    remedy,
                },
            )
            .expect_err("a live run freezes the remedy surface");
            assert_eq!(error.code, ShellErrorCode::RunActive);
        }
        assert!(fixture.project.path().join("docs/report.pdf").exists());
    }

    /// A scan is observation, not mutation: it takes **no** lock, so it is
    /// not frozen by the surface every remedy holds -- which is what lets
    /// the run-end scan run while a process is finishing.
    ///
    /// Proven by holding the publication surface lock across the whole
    /// scan. The previous version of this test started no process and held
    /// nothing, so its name claimed a concurrency it never arranged
    /// (R3-007).
    #[test]
    fn a_scan_takes_no_surface_lock_and_runs_while_the_remedy_surface_is_held() {
        let fixture = Fixture::new("scan-live");
        let held = fixture.publication.lock_surface();
        assert_eq!(
            fixture.scan().findings,
            1,
            "a scan is not serialized against the remedy surface"
        );
        drop(held);
    }

    /// R1-005: the scan table is keyed to the project that was walked. One
    /// process holds one table, a run in project A can finish after the
    /// active workspace has moved to B, and a remedy then takes its
    /// *workspace* from B and its *finding* from A -- so with identical
    /// bytes at the same relative name (a shared fixture, a copied
    /// artifact) the quarantine remedy would delete a file in a project the
    /// user never scanned.
    #[test]
    fn a_finding_from_another_project_never_answers_for_the_active_one() {
        let a = Fixture::new("cross-a");
        a.scan();
        assert_eq!(
            misplaced_list_for(&a.wrong_root, a.project_identity()).len(),
            1
        );

        // Project B: the same relative name carrying the same bytes.
        let b = TempDir::new("cross-b");
        std::fs::create_dir_all(b.path().join("docs")).expect("fixture dir");
        std::fs::write(b.path().join("docs/report.pdf"), pdf_bytes()).expect("fixture file");
        let b_workspace = b.workspace();
        let b_identity = super::project_identity_of(&b_workspace);

        assert!(
            misplaced_list_for(&a.wrong_root, Some(&b_identity)).is_empty(),
            "another project's scan never lists its findings under this one"
        );
        let error = misplaced_remedy_for(
            &a.outbox,
            &b_workspace,
            &a.publication,
            &a.wrong_root,
            &Idle,
            now(),
            &RemedyRequest {
                name: "docs/report.pdf".to_string(),
                sha256: Fixture::digest(),
                remedy: RemedyTag::Quarantine,
            },
        )
        .expect_err("a finding another project's scan reported is not this project's");
        assert_eq!(error.code, ShellErrorCode::MisplacedUnknown);
        assert!(
            b.path().join("docs/report.pdf").exists(),
            "no file in a project this session never scanned is ever removed"
        );
    }

    /// R3-014: the work area's containment (HAP-001-R7) is checked
    /// **before** the walk, not after it. A work area that resolves inside
    /// the project is a work area the walk would descend, and deciding that
    /// afterwards means it already has. Proven where the two orders differ:
    /// a project root that cannot be listed at all, whose work area is also
    /// invalid -- the refusal names the check that runs first.
    #[cfg(unix)]
    #[test]
    fn the_work_area_is_checked_before_the_project_is_walked() {
        use std::os::unix::fs::PermissionsExt as _;
        let project = TempDir::new("work-area-first");
        std::fs::create_dir_all(project.path().join("docs")).expect("fixture dir");
        std::fs::write(project.path().join("docs/report.pdf"), pdf_bytes()).expect("fixture");
        // The work area configured *inside* the project: exactly what
        // HAP-001-R7 refuses.
        let publication = PublicationState::under(&project.path().join("device"));
        let wrong_root = WrongRootState::new();

        std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o000))
            .expect("chmod");
        let unlistable_as_this_user = std::fs::read_dir(project.path()).is_err();
        let outcome = wrong_root_scan_for(
            &OutboxState::new(),
            &project.workspace(),
            &publication,
            &wrong_root,
        );
        std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o700))
            .expect("restore");

        if !unlistable_as_this_user {
            // Running as root: no permission bit stops the walk, so the two
            // orders are indistinguishable here and nothing is asserted.
            return;
        }
        assert_eq!(
            outcome
                .expect_err("an invalid work area refuses the scan")
                .code,
            ShellErrorCode::WorkAreaInvalid,
            "the work area is checked before anything is walked, so its refusal is the one \
             reported"
        );
        assert_eq!(
            wrong_root.summary(None),
            (false, 0),
            "a refused scan records nothing"
        );
    }

    /// R3-002: every fixed `detail` token the wire promises, pinned at the
    /// one place that produces it. Exhaustive over [`QuarantineMove`], so a
    /// variant added later cannot reach the wire without a token being
    /// chosen for it here.
    #[test]
    fn every_quarantine_detail_token_the_wire_fixes_is_pinned() {
        use omnifrons_app::quarantine::{FallbackReason, QuarantineMove};
        let cases = [
            (QuarantineMove::Renamed, false, "renamed"),
            (
                QuarantineMove::RenamedUnverified,
                false,
                "renamed-unverified",
            ),
            (
                QuarantineMove::CopiedAndUnlinked {
                    fallback: FallbackReason::DifferentVolume,
                },
                false,
                "different-volume",
            ),
            (
                QuarantineMove::CopiedAndUnlinked {
                    fallback: FallbackReason::Unsupported,
                },
                false,
                "rename-unsupported",
            ),
            (
                QuarantineMove::CopiedAndUnlinked {
                    fallback: FallbackReason::Failed,
                },
                false,
                "rename-failed",
            ),
            (
                QuarantineMove::CopiedOriginalKept {
                    fallback: FallbackReason::DifferentVolume,
                    reason: "unlink-failed",
                },
                true,
                "unlink-failed",
            ),
            (
                QuarantineMove::CopiedOriginalKept {
                    fallback: FallbackReason::DifferentVolume,
                    reason: "original-already-gone",
                },
                true,
                "original-already-gone",
            ),
            (
                QuarantineMove::CopiedOriginalKept {
                    fallback: FallbackReason::DifferentVolume,
                    reason: "original-changed-during-the-move",
                },
                true,
                "original-changed-during-the-move",
            ),
            (
                QuarantineMove::CopiedOriginalKept {
                    fallback: FallbackReason::Unsupported,
                    reason: "identity-check-unavailable",
                },
                true,
                "identity-check-unavailable",
            ),
        ];
        for (outcome, original_kept, token) in cases {
            assert_eq!(
                super::quarantine_detail(&outcome),
                (original_kept, token),
                "{outcome:?}"
            );
        }
    }

    /// The quarantine remedy's own payload carries the token, not just the
    /// outcome tag: the end-to-end run pins the branch this platform takes.
    #[test]
    fn the_quarantine_remedy_reports_its_detail_token() {
        let fixture = Fixture::new("detail-e2e");
        fixture.scan();
        let remedied = misplaced_remedy_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &fixture.wrong_root,
            &Idle,
            now(),
            &RemedyRequest {
                name: "docs/report.pdf".to_string(),
                sha256: Fixture::digest(),
                remedy: RemedyTag::Quarantine,
            },
        )
        .expect("the quarantine remedy moves the file");
        #[cfg(unix)]
        assert_eq!(
            remedied.detail.as_deref(),
            Some("renamed"),
            "one volume takes the handle-anchored rename"
        );
        #[cfg(not(unix))]
        assert_eq!(
            remedied.detail.as_deref(),
            Some("identity-check-unavailable"),
            "no by-handle unlink here, so the original is kept and the token says why"
        );
    }

    /// R3-006: what the publish remedy's design decision actually claims.
    /// The remedy publishes nothing -- it hands the outbox a new,
    /// **unattributed** entry that the existing whole-outbox path then
    /// takes over, so the entry is listed by `candidates_list { runId:
    /// null }`, classified `generated-heavy`, and **nothing is approved and
    /// nothing is journalled** by the remedy itself (HAP-001-R22, D3).
    #[test]
    fn the_publish_remedy_leaves_an_unattributed_candidate_and_approves_nothing() {
        let fixture = Fixture::new("publish-listed");
        fixture.scan();
        let remedied = misplaced_remedy_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &fixture.wrong_root,
            &Idle,
            now(),
            &RemedyRequest {
                name: "docs/report.pdf".to_string(),
                sha256: Fixture::digest(),
                remedy: RemedyTag::Publish,
            },
        )
        .expect("the publish remedy copies into the outbox");
        let name = remedied.name.expect("the new entry's name");

        let listed = crate::ipc::commands::inventory_whole_outbox(
            &fixture.workspace(),
            fixture.outbox.policy_store,
            fixture.outbox.preparer,
            fixture.outbox.inventory,
        )
        .expect("the whole-outbox inventory lists the outbox root");
        let row = listed
            .iter()
            .find(|row| row.name == name)
            .expect("the new entry is listed by candidates_list { runId: null }");
        assert_eq!(
            row.attribution,
            crate::ipc::dto::AttributionDto::Unattributed,
            "the remedy's entry belongs to no run"
        );
        assert_eq!(row.class.as_deref(), Some("generated-heavy"));
        assert_eq!(row.sha256.as_deref(), Some(Fixture::digest().as_str()));

        // Nothing was approved and nothing was published: no handle is held
        // for an approval, and the work area carries no publication
        // journal.
        assert_eq!(
            fixture
                .outbox
                .runs
                .lock()
                .expect("the run table is not poisoned")
                .held_handles(),
            0,
            "the remedy holds no approval handle: the entry is approved later, explicitly"
        );
        let journal = fixture
            .publication
            .work_area
            .join("journal")
            .join("publications.jsonl");
        assert!(
            !journal.exists(),
            "the remedy writes no publication journal entry"
        );
    }

    /// R3-010: `quarantine-unavailable` is produced by a behavior test and
    /// not only by a mapping. A quarantine directory that resolves inside
    /// the active workspace is exactly what RCS-001-R10 refuses.
    #[test]
    fn a_quarantine_directory_inside_the_workspace_refuses_the_remedy() {
        let mut fixture = Fixture::new("quarantine-inside");
        fixture.publication.quarantine = fixture.project.path().join("inside-quarantine");
        fixture.scan();
        let error = misplaced_remedy_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &fixture.wrong_root,
            &Idle,
            now(),
            &RemedyRequest {
                name: "docs/report.pdf".to_string(),
                sha256: Fixture::digest(),
                remedy: RemedyTag::Quarantine,
            },
        )
        .expect_err("a quarantine directory inside the workspace is refused");
        assert_eq!(error.code, ShellErrorCode::QuarantineUnavailable);
        assert!(
            fixture.project.path().join("docs/report.pdf").exists(),
            "a refused remedy moves nothing"
        );
        assert!(
            !fixture.publication.quarantine.exists(),
            "and creates nothing"
        );
    }

    /// R1-030: the arm `DestinationLinked` reaches has a behavior test of
    /// its own, and not only a mapping over a different variant.
    ///
    /// A same-user producer plants a hard link at the destination name,
    /// pointing at a file *outside* the quarantine directory that carries
    /// the very bytes the request bound to (HAP-001-R20 at this remedy's
    /// own final name, R1-022). Digest and size cannot tell that apart
    /// from this remedy's own earlier result; the link count can. The
    /// refusal is `quarantine-unavailable` -- a fact about the quarantine
    /// directory, not about the file the user asked about -- and leaves
    /// both names holding exactly what they held.
    ///
    /// Unix-gated because `link_count` yields `None` on Windows, where the
    /// by-handle accessor is unstable and this repository adds no Windows
    /// API dependency, so the refusal cannot be made there at all: the
    /// adapters suite's
    /// `a_platform_without_a_by_handle_link_count_reuses_a_linked_entry`
    /// is that half, and the residual is disclosed under HAP-001-R19
    /// rather than closed.
    #[cfg(unix)]
    #[test]
    fn a_quarantine_destination_with_a_second_name_refuses_the_remedy() {
        use omnifrons_app::quarantine::quarantine_name;
        use omnifrons_domain::publication::DisplayName;

        let fixture = Fixture::new("quarantine-linked");
        fixture.scan();

        // Beside the quarantine directory rather than inside it, which is
        // the whole point: accepting the destination would unlink the
        // project's copy in favour of an inode the producer still holds
        // another name for, and can rewrite afterwards.
        let alias = fixture.device.path().join("alias.pdf");
        std::fs::write(&alias, pdf_bytes()).expect("the producer's own file");
        std::fs::create_dir_all(&fixture.publication.quarantine).expect("the quarantine directory");
        let planted = fixture.publication.quarantine.join(quarantine_name(
            &DisplayName::sanitize("report.pdf"),
            &Sha2Hasher::new().sha256(&pdf_bytes()),
        ));
        std::fs::hard_link(&alias, &planted).expect("the planted second name");

        let error = misplaced_remedy_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &fixture.wrong_root,
            &Idle,
            now(),
            &RemedyRequest {
                name: "docs/report.pdf".to_string(),
                sha256: Fixture::digest(),
                remedy: RemedyTag::Quarantine,
            },
        )
        .expect_err("a destination answering to a second name is refused");
        assert_eq!(error.code, ShellErrorCode::QuarantineUnavailable);
        assert_eq!(
            error.message, "the quarantine destination has more than one name",
            "the count itself stays out of the message"
        );
        assert!(
            fixture.project.path().join("docs/report.pdf").exists(),
            "a refused remedy moves nothing"
        );
        assert_eq!(
            std::fs::read(&planted).expect("read"),
            pdf_bytes(),
            "and leaves the destination name holding exactly what it held"
        );
        assert!(
            alias.exists(),
            "including the producer's own other name for it"
        );
        assert_eq!(
            std::fs::read_dir(&fixture.publication.quarantine)
                .expect("list")
                .count(),
            1,
            "and no staging sibling is left behind"
        );
    }

    /// R3-010: `scan-failed` is produced by a behavior test too -- a
    /// project root this process cannot list at all, with a valid work
    /// area, so the refusal is the walk's own.
    #[cfg(unix)]
    #[test]
    fn a_project_root_that_cannot_be_listed_is_scan_failed() {
        use std::os::unix::fs::PermissionsExt as _;
        let fixture = Fixture::new("scan-failed");
        // Traversable but not listable (`--x`): the project's policy still
        // loads by path, so the refusal that follows is the walk's own and
        // not the policy load's.
        std::fs::set_permissions(
            fixture.project.path(),
            std::fs::Permissions::from_mode(0o111),
        )
        .expect("chmod");
        let unlistable = std::fs::read_dir(fixture.project.path()).is_err();
        let outcome = wrong_root_scan_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &fixture.wrong_root,
        );
        std::fs::set_permissions(
            fixture.project.path(),
            std::fs::Permissions::from_mode(0o700),
        )
        .expect("restore");
        if !unlistable {
            // Running as root: no permission bit stops the walk.
            return;
        }
        assert_eq!(
            outcome
                .expect_err("an unlistable project root refuses")
                .code,
            ShellErrorCode::ScanFailed
        );
    }

    /// R3-009's consequence: a corrupt ignore ledger fails the **scan**
    /// with `work-area-invalid` rather than quietly re-offering a file the
    /// user already dismissed.
    #[test]
    fn a_corrupt_ignore_ledger_fails_the_scan_rather_than_re_offering_a_dismissed_file() {
        let fixture = Fixture::new("ledger-corrupt");
        fixture.scan();
        misplaced_remedy_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &fixture.wrong_root,
            &Idle,
            now(),
            &RemedyRequest {
                name: "docs/report.pdf".to_string(),
                sha256: Fixture::digest(),
                remedy: RemedyTag::Ignore,
            },
        )
        .expect("the file is dismissed");
        assert_eq!(fixture.scan().findings, 0, "and stays dismissed");

        let ledger = fixture
            .publication
            .work_area
            .join("ignore")
            .join("decisions.jsonl");
        let mut content = std::fs::read_to_string(&ledger).expect("read");
        content.push_str("{\"schema\":99}\n");
        std::fs::write(&ledger, content).expect("write");

        let error = wrong_root_scan_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &fixture.wrong_root,
        )
        .expect_err("a corrupt ledger fails the scan closed");
        assert_eq!(error.code, ShellErrorCode::WorkAreaInvalid);
        assert_eq!(
            misplaced_list_for(&fixture.wrong_root, fixture.project_identity()).len(),
            0,
            "and the dismissed file is never re-offered by a scan that could not read the ledger"
        );
    }

    /// R3-013: HAP-001-R33's `enforced` branch, run through the real
    /// derivation rather than asserted at `OutputDiscipline::for_scope`.
    /// The reported mode is the **weakest** any registered adapter
    /// declares, so `enforced` needs every descriptor to be
    /// `sandbox-enforced` -- one advisory adapter is enough to take it
    /// away, which is the property the report exists to have.
    #[test]
    fn the_enforced_branch_runs_through_the_real_scope_mode_derivation() {
        let descriptor = |mode: ScopeMode| omnifrons_app::harness_adapter::AdapterDescriptor {
            id: omnifrons_domain::adapter::AdapterId::stream_json_cli(),
            display_name: "fixture".to_string(),
            transport_class: omnifrons_domain::adapter::TransportClass::StructuredStreamingCli,
            prompt_channel: omnifrons_domain::adapter::PromptChannel::StdinThenClose,
            argv_template: Vec::new(),
            declared_env: Vec::new(),
            scope_mode: mode,
            notes: String::new(),
        };
        let fixture = Fixture::new("enforced");

        let sandboxed = [descriptor(ScopeMode::SandboxEnforced)];
        assert_eq!(
            reported_scope_mode(&sandboxed),
            ScopeMode::SandboxEnforced,
            "every adapter sandboxed is the only way the product may say so"
        );
        let status =
            wrong_root_status_for(&sandboxed, &fixture.wrong_root, fixture.project_identity());
        assert_eq!(
            status.output_discipline,
            crate::ipc::dto::OutputDisciplineTag::Enforced
        );
        assert_eq!(
            status.scope_mode,
            crate::ipc::dto::ScopeModeDto::SandboxEnforced
        );
        // The tokens themselves, since the payload is what the renderer
        // states verbatim.
        let json = serde_json::to_value(&status).expect("serializable");
        assert_eq!(json["outputDiscipline"], "enforced");
        assert_eq!(json["scopeMode"], "sandbox-enforced");
        assert_eq!(
            status.disclosures,
            vec![omnifrons_domain::wrong_root::OutputDiscipline::IN_PROJECT_DISCLOSURE.to_string()],
            "HAP-001-R34's disclosure rides even an enforced report -- one entry, never zero"
        );

        let mixed = [
            descriptor(ScopeMode::SandboxEnforced),
            descriptor(ScopeMode::Advisory),
        ];
        assert_eq!(
            reported_scope_mode(&mixed),
            ScopeMode::Advisory,
            "one adapter that is not sandboxed takes the claim away from all of them"
        );
        assert_eq!(
            wrong_root_status_for(&mixed, &fixture.wrong_root, fixture.project_identity())
                .output_discipline,
            crate::ipc::dto::OutputDisciplineTag::Advisory
        );
        let harness_only = [descriptor(ScopeMode::HarnessEnforced)];
        assert_eq!(
            reported_scope_mode(&harness_only),
            ScopeMode::HarnessEnforced,
            "a harness-enforced catalog is neither sandboxed nor advisory"
        );
        assert_eq!(
            wrong_root_status_for(
                &harness_only,
                &fixture.wrong_root,
                fixture.project_identity()
            )
            .output_discipline,
            crate::ipc::dto::OutputDisciplineTag::Advisory,
            "and only an OS sandbox over the project root may read enforced"
        );
    }
}
