//! The guidance-note installer commands (spike slice 5c, HAP-001 D18 at
//! its default: the system proposes, the user disposes): `guidance_status`,
//! `guidance_preview`, `guidance_apply`, `guidance_remove`,
//! `guidance_snapshots`, `guidance_pin`, and `guidance_restore`, registered
//! next to the publication commands. Each is a thin wrapper over a pure
//! body tested here without a Tauri runtime, composing the application
//! transaction (`omnifrons_app::guidance`) with the real adapters: the
//! no-follow file port at the workspace root, the snapshot store under the
//! product work area, SHA-256, and the wall clock.
//!
//! The three writing commands -- `apply`, `remove`, `restore` -- and `pin`
//! hold the publication surface lock; the writing three are also frozen
//! while a run is active, like the publication commands. Every payload is
//! a logical value -- a name at the workspace root, tokens, digests, ids
//! -- never a device path (HAP-001-R5), and nothing here reads the note as
//! an instruction (HAP-001-R22).

use omnifrons_adapters::{FsProjectTextFile, FsSnapshotStore, Sha2Hasher};
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::guidance::{
    GuidanceError, GuidancePorts, apply, pin, preview, remove, restore, snapshots, status,
};
use omnifrons_app::managed_file::ProjectTextFileError;
use omnifrons_app::outbox_policy::OutboxPolicyStore as _;
use omnifrons_app::snapshot_store::{SnapshotId, SnapshotStoreError};
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::guidance::{
    DEFAULT_GUIDANCE_FILE, ManagedFileKind, ManagedFileName, ManagedFileNameError, ManagedTarget,
};
use omnifrons_domain::outbox::OutboxPath;
use omnifrons_supervisor::TokioProcessSupervisor;
use tauri::{AppHandle, Manager as _};

use crate::executable_state::SystemClock;
use crate::ipc::commands::active_workspace;
use crate::ipc::dto::{
    GuidanceActionTag, GuidanceAppliedDto, GuidancePreviewDto, GuidanceStatusDto,
    ManagedFileKindDto, ShellError, ShellErrorCode, SnapshotDto,
};
use crate::ipc::publication::{RunActivity, run_active};
use crate::outbox_state::OutboxState;
use crate::publication_state::PublicationState;

impl From<GuidanceError> for ShellError {
    /// Maps every installer refusal and failure to its closed code with a
    /// fixed, path-free message.
    fn from(error: GuidanceError) -> Self {
        match error {
            GuidanceError::WorkAreaInvalid => Self::new(
                ShellErrorCode::WorkAreaInvalid,
                "the product work area resolves inside a registered workspace root or cannot be \
                 used",
            ),
            GuidanceError::FileInvalid(error) => Self::new(
                ShellErrorCode::GuidanceFileInvalid,
                match error {
                    ProjectTextFileError::NotAFile => "the managed file is not a regular file",
                    ProjectTextFileError::TooLarge => "the managed file exceeds the size limit",
                    ProjectTextFileError::NotUtf8 => "the managed file is not valid UTF-8",
                    ProjectTextFileError::Unreadable => "the managed file could not be read",
                    ProjectTextFileError::WriteFailed => "the managed file could not be written",
                    // The port's own re-check before the rename (R1-002) is
                    // the same refusal as the digest binding's; the
                    // transaction already maps it there, and this arm keeps
                    // the two spellings on one code and one message.
                    ProjectTextFileError::Changed => {
                        return Self::from(GuidanceError::FileChanged);
                    }
                },
            ),
            GuidanceError::FileChanged => Self::new(
                ShellErrorCode::GuidanceFileChanged,
                "the managed file changed since it was shown; read its status again and retry",
            ),
            GuidanceError::BlockModified => Self::new(
                ShellErrorCode::GuidanceBlockModified,
                "the managed block was modified inside its sentinels; resolve it by hand or \
                 restore a snapshot",
            ),
            GuidanceError::BlockMalformed => Self::new(
                ShellErrorCode::GuidanceBlockMalformed,
                "the managed block's sentinels are not one intact pair; resolve it by hand or \
                 restore a snapshot",
            ),
            GuidanceError::Unmanaged => Self::new(
                ShellErrorCode::GuidanceUnmanaged,
                "the file carries no managed block",
            ),
            GuidanceError::Snapshot(SnapshotStoreError::Unknown) => Self::new(
                ShellErrorCode::GuidanceUnmanaged,
                "no snapshot with that id is recorded for this project",
            ),
            GuidanceError::Snapshot(SnapshotStoreError::Unreadable) => Self::new(
                ShellErrorCode::SnapshotUnavailable,
                "the snapshot store could not be read",
            ),
            GuidanceError::Snapshot(SnapshotStoreError::Corrupt) => Self::new(
                ShellErrorCode::SnapshotUnavailable,
                "a snapshot manifest is corrupt",
            ),
            GuidanceError::Snapshot(SnapshotStoreError::WriteFailed) => Self::new(
                ShellErrorCode::SnapshotUnavailable,
                "the snapshot store could not be written",
            ),
            GuidanceError::VerifyFailed => Self::new(
                ShellErrorCode::GuidanceFileInvalid,
                "the managed file did not read back as written",
            ),
        }
    }
}

impl From<ManagedFileNameError> for ShellError {
    /// A guidance file name outside RCS-001's file-name rule: the rule's
    /// own fixed message, never the name itself.
    fn from(error: ManagedFileNameError) -> Self {
        Self::new(ShellErrorCode::GuidanceFileInvalid, &error.to_string())
    }
}

fn invalid(message: &str) -> ShellError {
    ShellError::new(ShellErrorCode::InvalidRequest, message)
}

/// A guidance command's target: the kind and, for the guidance kind, the
/// file the user named at the workspace root (default `AGENTS.md`);
/// ignored for the ignore kind, which always manages `.gitignore`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceRequest {
    pub kind: ManagedFileKind,
    pub file: Option<String>,
}

impl GuidanceRequest {
    /// The managed target, with the guidance file name validated.
    fn target(&self) -> Result<ManagedTarget, ShellError> {
        match self.kind {
            ManagedFileKind::Guidance => {
                let name = self.file.as_deref().unwrap_or(DEFAULT_GUIDANCE_FILE);
                Ok(ManagedTarget::guidance(ManagedFileName::guidance(name)?))
            }
            ManagedFileKind::Ignore => Ok(ManagedTarget::ignore()),
        }
    }
}

/// The digest a write binds to: `null` for a file the surface showed as
/// absent, else its full 64-hex digest.
fn parse_expected(file_sha256: Option<&str>) -> Result<Option<Sha256Digest>, ShellError> {
    file_sha256
        .map(|hex| {
            Sha256Digest::from_hex(hex)
                .ok_or_else(|| invalid("the file digest is not 64 hex characters"))
        })
        .transpose()
}

fn parse_snapshot_id(id: &str) -> Result<SnapshotId, ShellError> {
    SnapshotId::from_hex(id).ok_or_else(|| invalid("the snapshot id is not 16 hex characters"))
}

/// The policy's declared outbox path: what the note and the ignore rule
/// name.
fn declared_outbox(
    outbox: &OutboxState,
    workspace: &WorkspaceRoot,
) -> Result<OutboxPath, ShellError> {
    Ok(outbox.policy_store.load(workspace)?.outbox().clone())
}

/// The real adapters over the work area, opened for one operation
/// (HAP-001-R7: the work area is checked before anything else).
struct Installer {
    work_area: WorkAreaRoot,
    snapshots: FsSnapshotStore,
    files: FsProjectTextFile,
    hasher: Sha2Hasher,
    clock: SystemClock,
}

impl Installer {
    fn open(publication: &PublicationState, workspace: &WorkspaceRoot) -> Result<Self, ShellError> {
        let work_area = WorkAreaRoot::open(&publication.work_area, &[workspace])?;
        Ok(Self {
            snapshots: FsSnapshotStore::open(&work_area),
            work_area,
            files: FsProjectTextFile::new(),
            hasher: Sha2Hasher::new(),
            clock: SystemClock,
        })
    }

    fn ports<'a>(&'a mut self, workspaces: &'a [&'a WorkspaceRoot]) -> GuidancePorts<'a> {
        GuidancePorts {
            files: &self.files,
            snapshots: &mut self.snapshots,
            hasher: &self.hasher,
            clock: &self.clock,
            work_area: &self.work_area,
            workspaces,
        }
    }
}

/// The managed file's state (`guidance_status`).
///
/// # Errors
///
/// `guidance-file-invalid` for a name outside the file-name rule or a
/// file that is not a regular UTF-8 file within the bound;
/// `outbox-invalid` when the policy cannot be loaded; `work-area-invalid`;
/// `snapshot-unavailable`.
pub fn guidance_status_for(
    outbox: &OutboxState,
    workspace: &WorkspaceRoot,
    publication: &PublicationState,
    request: &GuidanceRequest,
) -> Result<GuidanceStatusDto, ShellError> {
    let target = request.target()?;
    let declared = declared_outbox(outbox, workspace)?;
    let mut installer = Installer::open(publication, workspace)?;
    let status = status(
        &mut installer.ports(&[workspace]),
        workspace,
        &target,
        &declared,
    )?;
    Ok(GuidanceStatusDto::from_status(target.kind(), &status))
}

/// The proposal (`guidance_preview`): what an apply would write, and the
/// digest it must bind to. Writes nothing.
///
/// # Errors
///
/// As [`guidance_status_for`], plus `guidance-block-modified` and
/// `guidance-block-malformed`.
pub fn guidance_preview_for(
    outbox: &OutboxState,
    workspace: &WorkspaceRoot,
    publication: &PublicationState,
    request: &GuidanceRequest,
) -> Result<GuidancePreviewDto, ShellError> {
    let target = request.target()?;
    let declared = declared_outbox(outbox, workspace)?;
    let mut installer = Installer::open(publication, workspace)?;
    let preview = preview(
        &mut installer.ports(&[workspace]),
        workspace,
        &target,
        &declared,
    )?;
    Ok(GuidancePreviewDto::from_preview(target.kind(), &preview))
}

/// The approved write (`guidance_apply`), bound to `file_sha256`.
///
/// # Errors
///
/// `run-active` while a run is live; `invalid-request` for a malformed
/// digest; `guidance-file-changed` when the file differs from the digest;
/// and [`guidance_preview_for`]'s errors.
pub fn guidance_apply_for(
    outbox: &OutboxState,
    workspace: &WorkspaceRoot,
    publication: &PublicationState,
    run_activity: &dyn RunActivity,
    request: &GuidanceRequest,
    file_sha256: Option<&str>,
) -> Result<GuidanceAppliedDto, ShellError> {
    let _surface = publication.lock_surface();
    if run_activity.any_running() {
        return Err(run_active());
    }
    let target = request.target()?;
    let expected = parse_expected(file_sha256)?;
    let declared = declared_outbox(outbox, workspace)?;
    let mut installer = Installer::open(publication, workspace)?;
    let applied = apply(
        &mut installer.ports(&[workspace]),
        workspace,
        &target,
        &declared,
        expected,
    )?;
    Ok(GuidanceAppliedDto {
        kind: target.kind().into(),
        file: target.file_name().to_string(),
        action: applied.action.into(),
        snapshot_id: applied.snapshot_id.map(|id| id.to_hex()),
        result_sha256_short: Some(applied.result_sha256.short_hex()),
    })
}

/// The removal (`guidance_remove`), bound to `file_sha256`: only the lines
/// Omnifrons owns go, and the file with them when Omnifrons created it.
///
/// # Errors
///
/// `run-active`; `invalid-request`; `guidance-unmanaged` when no block is
/// there; `guidance-file-changed`; `guidance-block-modified`;
/// `guidance-block-malformed`; `guidance-file-invalid`;
/// `work-area-invalid`; `snapshot-unavailable`.
pub fn guidance_remove_for(
    workspace: &WorkspaceRoot,
    publication: &PublicationState,
    run_activity: &dyn RunActivity,
    request: &GuidanceRequest,
    file_sha256: Option<&str>,
) -> Result<GuidanceAppliedDto, ShellError> {
    let _surface = publication.lock_surface();
    if run_activity.any_running() {
        return Err(run_active());
    }
    let target = request.target()?;
    let expected = parse_expected(file_sha256)?;
    let mut installer = Installer::open(publication, workspace)?;
    let removed = remove(
        &mut installer.ports(&[workspace]),
        workspace,
        &target,
        expected,
    )?;
    Ok(GuidanceAppliedDto {
        kind: target.kind().into(),
        file: target.file_name().to_string(),
        action: GuidanceActionTag::Remove,
        snapshot_id: Some(removed.snapshot_id.to_hex()),
        result_sha256_short: removed.result_sha256.map(|digest| digest.short_hex()),
    })
}

/// Every snapshot of `kind` for the active project, newest first
/// (`guidance_snapshots`).
///
/// # Errors
///
/// `work-area-invalid` or `snapshot-unavailable`.
pub fn guidance_snapshots_for(
    workspace: &WorkspaceRoot,
    publication: &PublicationState,
    kind: ManagedFileKind,
) -> Result<Vec<SnapshotDto>, ShellError> {
    let mut installer = Installer::open(publication, workspace)?;
    let listed = snapshots(&mut installer.ports(&[workspace]), workspace, kind)?;
    Ok(listed.iter().map(SnapshotDto::from_manifest).collect())
}

/// Pin or unpin the snapshot `id` (`guidance_pin`); a pinned snapshot is
/// never pruned. Returns the snapshot as it now stands.
///
/// # Errors
///
/// `invalid-request` for a malformed id; `guidance-unmanaged` for an id
/// this project does not hold; `work-area-invalid`; `snapshot-unavailable`.
pub fn guidance_pin_for(
    workspace: &WorkspaceRoot,
    publication: &PublicationState,
    id: &str,
    pinned: bool,
) -> Result<SnapshotDto, ShellError> {
    let _surface = publication.lock_surface();
    let id = parse_snapshot_id(id)?;
    let mut installer = Installer::open(publication, workspace)?;
    pin(&mut installer.ports(&[workspace]), workspace, id, pinned)?;
    // The manifest as rewritten: the store knows the id, not its kind, so
    // both kinds' listings are searched.
    for kind in ManagedFileKind::ALL {
        if let Some(manifest) = snapshots(&mut installer.ports(&[workspace]), workspace, kind)?
            .iter()
            .find(|manifest| manifest.id == id)
        {
            return Ok(SnapshotDto::from_manifest(manifest));
        }
    }
    Err(ShellError::from(GuidanceError::Snapshot(
        SnapshotStoreError::Unknown,
    )))
}

/// The restore (`guidance_restore`), bound to `file_sha256`: the state
/// before it is snapshotted first, then the snapshot is brought back --
/// its bytes, or the file's absence.
///
/// # Errors
///
/// `run-active`; `invalid-request`; `guidance-unmanaged` for an id this
/// project does not hold; `guidance-file-changed`; `guidance-file-invalid`;
/// `work-area-invalid`; `snapshot-unavailable`.
pub fn guidance_restore_for(
    workspace: &WorkspaceRoot,
    publication: &PublicationState,
    run_activity: &dyn RunActivity,
    id: &str,
    file_sha256: Option<&str>,
) -> Result<GuidanceAppliedDto, ShellError> {
    let _surface = publication.lock_surface();
    if run_activity.any_running() {
        return Err(run_active());
    }
    let id = parse_snapshot_id(id)?;
    let expected = parse_expected(file_sha256)?;
    let mut installer = Installer::open(publication, workspace)?;
    let restored = restore(&mut installer.ports(&[workspace]), workspace, id, expected)?;
    Ok(GuidanceAppliedDto {
        kind: restored.target.kind().into(),
        file: restored.target.file_name().to_string(),
        action: GuidanceActionTag::Restore,
        snapshot_id: Some(restored.snapshot_id.to_hex()),
        result_sha256_short: restored.result_sha256.map(|digest| digest.short_hex()),
    })
}

/// The managed file's state for the active workspace.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`guidance_status_for`]'s errors.
#[tauri::command]
pub async fn guidance_status(
    app: AppHandle,
    kind: ManagedFileKindDto,
    file: Option<String>,
) -> Result<GuidanceStatusDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let outbox = app.state::<OutboxState>();
        let publication = app.state::<PublicationState>();
        guidance_status_for(
            &outbox,
            &workspace,
            &publication,
            &GuidanceRequest {
                kind: kind.into(),
                file,
            },
        )
    })
    .await
    .expect("the blocking guidance-status task panicked")
}

/// The proposal: what `guidance_apply` would write (HAP-001 D18).
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`guidance_preview_for`]'s errors.
#[tauri::command]
pub async fn guidance_preview(
    app: AppHandle,
    kind: ManagedFileKindDto,
    file: Option<String>,
) -> Result<GuidancePreviewDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let outbox = app.state::<OutboxState>();
        let publication = app.state::<PublicationState>();
        guidance_preview_for(
            &outbox,
            &workspace,
            &publication,
            &GuidanceRequest {
                kind: kind.into(),
                file,
            },
        )
    })
    .await
    .expect("the blocking guidance-preview task panicked")
}

/// The approved write (HAP-001 D18, R42), bound to `fileSha256`.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`guidance_apply_for`]'s errors.
#[tauri::command]
pub async fn guidance_apply(
    app: AppHandle,
    kind: ManagedFileKindDto,
    file: Option<String>,
    file_sha256: Option<String>,
) -> Result<GuidanceAppliedDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let outbox = app.state::<OutboxState>();
        let publication = app.state::<PublicationState>();
        let supervisor = app.state::<TokioProcessSupervisor>().inner().clone();
        guidance_apply_for(
            &outbox,
            &workspace,
            &publication,
            &supervisor,
            &GuidanceRequest {
                kind: kind.into(),
                file,
            },
            file_sha256.as_deref(),
        )
    })
    .await
    .expect("the blocking guidance-apply task panicked")
}

/// The removal of the lines Omnifrons owns, bound to `fileSha256`.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`guidance_remove_for`]'s errors.
#[tauri::command]
pub async fn guidance_remove(
    app: AppHandle,
    kind: ManagedFileKindDto,
    file: Option<String>,
    file_sha256: Option<String>,
) -> Result<GuidanceAppliedDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let publication = app.state::<PublicationState>();
        let supervisor = app.state::<TokioProcessSupervisor>().inner().clone();
        guidance_remove_for(
            &workspace,
            &publication,
            &supervisor,
            &GuidanceRequest {
                kind: kind.into(),
                file,
            },
            file_sha256.as_deref(),
        )
    })
    .await
    .expect("the blocking guidance-remove task panicked")
}

/// Every snapshot of `kind` for the active project, newest first.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`guidance_snapshots_for`]'s errors.
#[tauri::command]
pub async fn guidance_snapshots(
    app: AppHandle,
    kind: ManagedFileKindDto,
) -> Result<Vec<SnapshotDto>, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let publication = app.state::<PublicationState>();
        guidance_snapshots_for(&workspace, &publication, kind.into())
    })
    .await
    .expect("the blocking guidance-snapshots task panicked")
}

/// Pin or unpin a snapshot.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`guidance_pin_for`]'s errors.
#[tauri::command]
pub async fn guidance_pin(
    app: AppHandle,
    id: String,
    pinned: bool,
) -> Result<SnapshotDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let publication = app.state::<PublicationState>();
        guidance_pin_for(&workspace, &publication, &id, pinned)
    })
    .await
    .expect("the blocking guidance-pin task panicked")
}

/// Restore a snapshot, bound to `fileSha256`.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`guidance_restore_for`]'s errors.
#[tauri::command]
pub async fn guidance_restore(
    app: AppHandle,
    id: String,
    file_sha256: Option<String>,
) -> Result<GuidanceAppliedDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let publication = app.state::<PublicationState>();
        let supervisor = app.state::<TokioProcessSupervisor>().inner().clone();
        guidance_restore_for(
            &workspace,
            &publication,
            &supervisor,
            &id,
            file_sha256.as_deref(),
        )
    })
    .await
    .expect("the blocking guidance-restore task panicked")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use omnifrons_adapters::Sha2Hasher;
    use omnifrons_app::WorkspaceRoot;
    use omnifrons_app::content_hasher::ContentHasher as _;
    use omnifrons_domain::guidance::{GuidanceNote, ManagedFileKind};
    use omnifrons_domain::outbox::OutboxPath;

    use super::{
        GuidanceRequest, guidance_apply_for, guidance_pin_for, guidance_preview_for,
        guidance_remove_for, guidance_restore_for, guidance_snapshots_for, guidance_status_for,
    };
    use crate::ipc::dto::{
        GuidanceActionTag, ManagedFileKindDto, ManagedStatusTag, ShellError, ShellErrorCode,
    };
    use crate::ipc::publication::RunActivity;
    use crate::outbox_state::OutboxState;
    use crate::publication_state::PublicationState;

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
                "omnifrons-shell-guidance-test-{}-{label}-{n}",
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
    /// hasher is what every digest comes from anyway.
    fn hex(bytes: &[u8]) -> String {
        Sha2Hasher::new().sha256(bytes).to_hex()
    }

    /// The fixture: a project with no policy file (the default policy, the
    /// default outbox), a device directory holding the work area, the
    /// shell's outbox and publication states.
    struct Fixture {
        project: TempDir,
        device: TempDir,
        outbox: OutboxState,
        publication: PublicationState,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let project = TempDir::new(label);
            let device = TempDir::new(&format!("{label}-device"));
            let publication = PublicationState::under(device.path());
            Self {
                project,
                device,
                outbox: OutboxState::new(),
                publication,
            }
        }

        fn workspace(&self) -> WorkspaceRoot {
            self.project.workspace()
        }

        fn request(kind: ManagedFileKind, file: Option<&str>) -> GuidanceRequest {
            GuidanceRequest {
                kind,
                file: file.map(str::to_string),
            }
        }

        fn agents() -> GuidanceRequest {
            Self::request(ManagedFileKind::Guidance, None)
        }

        fn text(&self, name: &str) -> Option<String> {
            std::fs::read_to_string(self.project.path().join(name)).ok()
        }

        fn file_digest(&self, name: &str) -> Option<String> {
            std::fs::read(self.project.path().join(name))
                .ok()
                .map(|bytes| hex(&bytes))
        }

        fn status(
            &self,
            request: &GuidanceRequest,
        ) -> Result<crate::ipc::dto::GuidanceStatusDto, ShellError> {
            guidance_status_for(&self.outbox, &self.workspace(), &self.publication, request)
        }

        fn apply(
            &self,
            request: &GuidanceRequest,
            file_sha256: Option<&str>,
        ) -> Result<crate::ipc::dto::GuidanceAppliedDto, ShellError> {
            guidance_apply_for(
                &self.outbox,
                &self.workspace(),
                &self.publication,
                &Idle,
                request,
                file_sha256,
            )
        }

        fn remove(
            &self,
            request: &GuidanceRequest,
            file_sha256: Option<&str>,
        ) -> Result<crate::ipc::dto::GuidanceAppliedDto, ShellError> {
            guidance_remove_for(
                &self.workspace(),
                &self.publication,
                &Idle,
                request,
                file_sha256,
            )
        }

        fn restore(
            &self,
            id: &str,
            file_sha256: Option<&str>,
        ) -> Result<crate::ipc::dto::GuidanceAppliedDto, ShellError> {
            guidance_restore_for(&self.workspace(), &self.publication, &Idle, id, file_sha256)
        }

        fn snapshots_dir_entries(&self) -> usize {
            std::fs::read_dir(self.device.path().join("work-area/snapshots")).map_or(0, |entries| {
                entries
                    .flatten()
                    .map(|entry| std::fs::read_dir(entry.path()).map_or(0, Iterator::count))
                    .sum()
            })
        }
    }

    // -- status and preview (D18: the system proposes) --

    #[test]
    fn status_of_a_fresh_project_is_absent_with_the_template_version() {
        let fixture = Fixture::new("status");
        let dto = fixture.status(&Fixture::agents()).expect("status");
        assert_eq!(dto.kind, ManagedFileKindDto::Guidance);
        assert_eq!(dto.file, "AGENTS.md");
        assert!(!dto.exists);
        assert_eq!(dto.managed, ManagedStatusTag::Absent);
        assert_eq!(dto.template_version, "hap-001-guidance-v1");
        assert_eq!(dto.file_sha256, None);
        assert_eq!(dto.file_sha256_short, None);
        assert_eq!(dto.snapshots, 0);
        assert_eq!(dto.pinned, 0);
        assert!(
            !fixture.project.path().join("AGENTS.md").exists(),
            "a status creates nothing"
        );
    }

    /// VP-S28's "the offered note equals the template with only the path
    /// substituted": the preview's proposed block is the template for the
    /// policy's outbox between the sentinels, and nothing is written.
    #[test]
    fn preview_offers_the_template_with_only_the_path_substituted_and_writes_nothing() {
        let fixture = Fixture::new("preview");
        let dto = guidance_preview_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Fixture::agents(),
        )
        .expect("preview");
        assert_eq!(dto.kind, ManagedFileKindDto::Guidance);
        assert_eq!(dto.file, "AGENTS.md");
        assert_eq!(dto.action, GuidanceActionTag::Insert);
        let note = GuidanceNote::render(&OutboxPath::default_path());
        let body = note.trim_end_matches('\n');
        assert!(
            dto.proposed
                .starts_with("<!-- omnifrons:begin guidance hap-001-guidance-v1 sha256:"),
            "{}",
            dto.proposed
        );
        assert!(dto.proposed.ends_with("\n<!-- omnifrons:end guidance -->"));
        assert!(
            dto.proposed.contains(&format!("\n{body}\n")),
            "the note verbatim"
        );
        assert_eq!(dto.file_sha256, None);
        assert_eq!(dto.file_sha256_short, None);
        assert_eq!(dto.result_sha256_short.len(), 8);
        assert!(!fixture.project.path().join("AGENTS.md").exists());
        assert_eq!(fixture.snapshots_dir_entries(), 0, "nothing snapshotted");
    }

    // -- apply (D18: the user disposes) --

    #[test]
    fn apply_writes_the_note_after_a_snapshot_and_a_second_apply_is_a_no_op() {
        let fixture = Fixture::new("apply");
        let applied = fixture.apply(&Fixture::agents(), None).expect("applied");
        assert_eq!(applied.kind, ManagedFileKindDto::Guidance);
        assert_eq!(applied.file, "AGENTS.md");
        assert_eq!(applied.action, GuidanceActionTag::Insert);
        let snapshot_id = applied
            .snapshot_id
            .clone()
            .expect("a snapshot of the absence");
        assert_eq!(snapshot_id.len(), 16, "16 hex");
        let result_short = applied.result_sha256_short.clone().expect("written");
        assert_eq!(result_short.len(), 8);
        let text = fixture.text("AGENTS.md").expect("written");
        assert!(text.starts_with("<!-- omnifrons:begin guidance hap-001-guidance-v1 sha256:"));
        assert!(text.contains("\n## Generated files\n"));
        assert!(text.ends_with("<!-- omnifrons:end guidance -->\n"));
        assert_eq!(
            &fixture.file_digest("AGENTS.md").expect("digest")[..8],
            result_short
        );
        assert_eq!(
            fixture.snapshots_dir_entries(),
            2,
            "one manifest and one bytes file"
        );

        let status = fixture.status(&Fixture::agents()).expect("status");
        assert!(status.exists);
        assert_eq!(status.managed, ManagedStatusTag::Current);
        assert_eq!(status.file_sha256, fixture.file_digest("AGENTS.md"));
        assert_eq!(status.snapshots, 1);

        let shown = fixture.file_digest("AGENTS.md");
        let again = fixture
            .apply(&Fixture::agents(), shown.as_deref())
            .expect("applied again");
        assert_eq!(again.action, GuidanceActionTag::NoOp);
        assert_eq!(again.snapshot_id, None);
        assert_eq!(
            fixture.snapshots_dir_entries(),
            2,
            "a no-op snapshots nothing"
        );
    }

    /// The write binds to the digest the surface showed (`fileSha256`):
    /// an absent file shown as present, or a stale digest, is refused with
    /// `guidance-file-changed` and nothing is written.
    #[test]
    fn apply_refuses_a_stale_file_digest() {
        let fixture = Fixture::new("apply-stale");
        std::fs::write(fixture.project.path().join("AGENTS.md"), b"# Title\n").expect("file");
        let error = fixture
            .apply(&Fixture::agents(), None)
            .expect_err("the surface showed no file");
        assert_eq!(error.code, ShellErrorCode::GuidanceFileChanged);
        assert!(!error.message.contains('/'), "{}", error.message);
        let error = fixture
            .apply(&Fixture::agents(), Some(&"ab".repeat(32)))
            .expect_err("stale");
        assert_eq!(error.code, ShellErrorCode::GuidanceFileChanged);
        let error = fixture
            .apply(&Fixture::agents(), Some("not-hex"))
            .expect_err("malformed");
        assert_eq!(error.code, ShellErrorCode::InvalidRequest);
        assert_eq!(fixture.text("AGENTS.md").as_deref(), Some("# Title\n"));
        assert_eq!(fixture.snapshots_dir_entries(), 0);
        let shown = fixture.file_digest("AGENTS.md");
        let applied = fixture
            .apply(&Fixture::agents(), shown.as_deref())
            .expect("applied");
        assert_eq!(applied.action, GuidanceActionTag::Insert);
        assert!(
            fixture
                .text("AGENTS.md")
                .expect("text")
                .starts_with("# Title\n\n<!-- omnifrons:begin")
        );
    }

    #[test]
    fn apply_takes_a_user_named_guidance_file_and_manages_the_ignore_file() {
        let fixture = Fixture::new("apply-names");
        let claude = Fixture::request(ManagedFileKind::Guidance, Some("CLAUDE.md"));
        let applied = fixture.apply(&claude, None).expect("applied");
        assert_eq!(applied.file, "CLAUDE.md");
        assert!(fixture.project.path().join("CLAUDE.md").is_file());
        assert!(!fixture.project.path().join("AGENTS.md").exists());

        std::fs::write(fixture.project.path().join(".gitignore"), b"/target\n").expect("rules");
        let shown = fixture.file_digest(".gitignore");
        // `file` is ignored for the ignore kind: it always manages .gitignore.
        let ignore = Fixture::request(ManagedFileKind::Ignore, Some("elsewhere.md"));
        let applied = fixture.apply(&ignore, shown.as_deref()).expect("applied");
        assert_eq!(applied.kind, ManagedFileKindDto::Ignore);
        assert_eq!(applied.file, ".gitignore");
        assert_eq!(
            fixture.text(".gitignore").expect("rules"),
            format!(
                "/target\n\n# omnifrons:begin ignore hap-001-guidance-v1 sha256:{}\n/.omnifrons/outbox/\n# omnifrons:end ignore\n",
                hex(b"/.omnifrons/outbox/")
            )
        );
        let status = fixture.status(&ignore).expect("status");
        assert_eq!(status.file, ".gitignore");
        assert_eq!(status.managed, ManagedStatusTag::Current);
    }

    /// A guidance file name outside RCS-001's file-name rule -- a path, a
    /// reserved device name, not Markdown -- is `guidance-file-invalid`
    /// before anything is read.
    #[test]
    fn an_invalid_guidance_file_name_is_guidance_file_invalid() {
        let fixture = Fixture::new("names");
        for name in [
            "docs/AGENTS.md",
            "..",
            "CON.md",
            "AGENTS.txt",
            "AGENTS\u{202E}.md",
            "",
        ] {
            let request = Fixture::request(ManagedFileKind::Guidance, Some(name));
            let error = fixture.status(&request).expect_err(name);
            assert_eq!(error.code, ShellErrorCode::GuidanceFileInvalid, "{name:?}");
            assert!(!error.message.contains('/'), "{}", error.message);
            // The message is the rule's own fixed text, never the name
            // (`..` and the empty name are words of their own rules).
            if !matches!(name, "" | "..") {
                assert!(
                    !error.message.contains(name),
                    "the name never rides the message: {}",
                    error.message
                );
            }
        }
    }

    // -- remove, restore, snapshots, pin (the installer discipline) --

    #[test]
    fn remove_takes_the_note_back_and_removes_the_file_omnifrons_created() {
        let fixture = Fixture::new("remove");
        fixture.apply(&Fixture::agents(), None).expect("applied");
        let shown = fixture.file_digest("AGENTS.md");
        let removed = fixture
            .remove(&Fixture::agents(), shown.as_deref())
            .expect("removed");
        assert_eq!(removed.action, GuidanceActionTag::Remove);
        assert!(removed.snapshot_id.is_some());
        assert_eq!(removed.result_sha256_short, None, "the file is gone");
        assert!(!fixture.project.path().join("AGENTS.md").exists());
        let error = fixture
            .remove(&Fixture::agents(), None)
            .expect_err("nothing managed");
        assert_eq!(error.code, ShellErrorCode::GuidanceUnmanaged);

        std::fs::write(fixture.project.path().join("CLAUDE.md"), b"# Mine\n").expect("file");
        let claude = Fixture::request(ManagedFileKind::Guidance, Some("CLAUDE.md"));
        let shown = fixture.file_digest("CLAUDE.md");
        fixture.apply(&claude, shown.as_deref()).expect("applied");
        let shown = fixture.file_digest("CLAUDE.md");
        let removed = fixture.remove(&claude, shown.as_deref()).expect("removed");
        assert_eq!(fixture.text("CLAUDE.md").as_deref(), Some("# Mine\n"));
        assert_eq!(
            removed.result_sha256_short.as_deref(),
            Some(&fixture.file_digest("CLAUDE.md").expect("digest")[..8])
        );
    }

    #[test]
    fn snapshots_are_listed_newest_first_pinned_and_restored() {
        let fixture = Fixture::new("snapshots");
        let applied = fixture.apply(&Fixture::agents(), None).expect("applied");
        let first = applied.snapshot_id.expect("snapshot");
        let listed = guidance_snapshots_for(
            &fixture.workspace(),
            &fixture.publication,
            ManagedFileKind::Guidance,
        )
        .expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, first);
        assert_eq!(listed[0].kind, ManagedFileKindDto::Guidance);
        assert_eq!(listed[0].file, "AGENTS.md");
        assert!(!listed[0].existed);
        assert_eq!(listed[0].size, 0);
        assert_eq!(listed[0].sha256_short, hex(b"")[..8]);
        assert!(!listed[0].pinned);
        assert!(listed[0].taken_at > 1_700_000_000_000, "ms since the epoch");

        let pinned = guidance_pin_for(&fixture.workspace(), &fixture.publication, &first, true)
            .expect("pinned");
        assert!(pinned.pinned);
        assert_eq!(
            fixture.status(&Fixture::agents()).expect("status").pinned,
            1
        );

        let shown = fixture.file_digest("AGENTS.md");
        let restored = fixture.restore(&first, shown.as_deref()).expect("restored");
        assert_eq!(restored.action, GuidanceActionTag::Restore);
        assert_eq!(restored.result_sha256_short, None, "the absence is back");
        assert!(!fixture.project.path().join("AGENTS.md").exists());
        let listed = guidance_snapshots_for(
            &fixture.workspace(),
            &fixture.publication,
            ManagedFileKind::Guidance,
        )
        .expect("list");
        assert_eq!(listed.len(), 2, "the state before the restore was kept");
        assert_eq!(
            listed[1].id, first,
            "newest first: the pre-restore snapshot precedes the original"
        );
        assert!(listed[0].existed);

        let error = fixture
            .restore("0000000000000000", None)
            .expect_err("no such snapshot");
        assert_eq!(error.code, ShellErrorCode::GuidanceUnmanaged);
        let error = fixture.restore("nope", None).expect_err("malformed");
        assert_eq!(error.code, ShellErrorCode::InvalidRequest);
        let error = guidance_pin_for(
            &fixture.workspace(),
            &fixture.publication,
            "0000000000000000",
            false,
        )
        .expect_err("no such snapshot");
        assert_eq!(error.code, ShellErrorCode::GuidanceUnmanaged);
    }

    /// A block edited inside its sentinels, or with a sentinel missing, is
    /// refused with its own code by apply and by remove; the user resolves
    /// it by hand or restores.
    #[test]
    fn modified_and_malformed_blocks_are_refused_with_their_codes() {
        let fixture = Fixture::new("blocks");
        fixture.apply(&Fixture::agents(), None).expect("applied");
        let path = fixture.project.path().join("AGENTS.md");
        let current = fixture.text("AGENTS.md").expect("text");
        std::fs::write(
            &path,
            current.replace("Create the directory", "Create the folder"),
        )
        .expect("edit");
        let shown = fixture.file_digest("AGENTS.md");
        assert_eq!(
            fixture.status(&Fixture::agents()).expect("status").managed,
            ManagedStatusTag::Modified
        );
        let error = fixture
            .apply(&Fixture::agents(), shown.as_deref())
            .expect_err("modified");
        assert_eq!(error.code, ShellErrorCode::GuidanceBlockModified);
        let error = fixture
            .remove(&Fixture::agents(), shown.as_deref())
            .expect_err("modified");
        assert_eq!(error.code, ShellErrorCode::GuidanceBlockModified);

        std::fs::write(
            &path,
            current.replace("<!-- omnifrons:end guidance -->", ""),
        )
        .expect("break");
        let shown = fixture.file_digest("AGENTS.md");
        assert_eq!(
            fixture.status(&Fixture::agents()).expect("status").managed,
            ManagedStatusTag::Malformed
        );
        let error = fixture
            .apply(&Fixture::agents(), shown.as_deref())
            .expect_err("malformed");
        assert_eq!(error.code, ShellErrorCode::GuidanceBlockMalformed);
        assert!(!error.message.contains('/'), "{}", error.message);
    }

    /// The three writing commands are frozen while a run is active, like
    /// the publication surface; nothing is written.
    #[test]
    fn apply_remove_and_restore_are_refused_while_a_run_is_active() {
        let fixture = Fixture::new("run-active");
        let error = guidance_apply_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Busy,
            &Fixture::agents(),
            None,
        )
        .expect_err("frozen");
        assert_eq!(error.code, ShellErrorCode::RunActive);
        assert!(!fixture.project.path().join("AGENTS.md").exists());
        let error = guidance_remove_for(
            &fixture.workspace(),
            &fixture.publication,
            &Busy,
            &Fixture::agents(),
            None,
        )
        .expect_err("frozen");
        assert_eq!(error.code, ShellErrorCode::RunActive);
        let error = guidance_restore_for(
            &fixture.workspace(),
            &fixture.publication,
            &Busy,
            "0000000000000000",
            None,
        )
        .expect_err("frozen");
        assert_eq!(error.code, ShellErrorCode::RunActive);
        assert_eq!(fixture.snapshots_dir_entries(), 0);
    }

    /// HAP-001-R7: a work area configured inside the workspace refuses
    /// every guidance command with `work-area-invalid`. R3-007 (slice 5c
    /// reliability review): all seven, not only the two that write.
    #[test]
    fn a_work_area_inside_the_workspace_refuses_the_guidance_commands() {
        let mut fixture = Fixture::new("work-area");
        fixture.publication.work_area = fixture.project.path().join(".omnifrons/work-area");
        let id = "0000000000000000";
        let refusals: [(&str, ShellError); 7] = [
            (
                "status",
                fixture.status(&Fixture::agents()).expect_err("refused"),
            ),
            (
                "preview",
                guidance_preview_for(
                    &fixture.outbox,
                    &fixture.workspace(),
                    &fixture.publication,
                    &Fixture::agents(),
                )
                .expect_err("refused"),
            ),
            (
                "apply",
                fixture
                    .apply(&Fixture::agents(), None)
                    .expect_err("refused"),
            ),
            (
                "remove",
                fixture
                    .remove(&Fixture::agents(), None)
                    .expect_err("refused"),
            ),
            (
                "snapshots",
                guidance_snapshots_for(
                    &fixture.workspace(),
                    &fixture.publication,
                    ManagedFileKind::Guidance,
                )
                .expect_err("refused"),
            ),
            (
                "pin",
                guidance_pin_for(&fixture.workspace(), &fixture.publication, id, true)
                    .expect_err("refused"),
            ),
            ("restore", fixture.restore(id, None).expect_err("refused")),
        ];
        for (command, error) in refusals {
            assert_eq!(error.code, ShellErrorCode::WorkAreaInvalid, "{command}");
            assert!(!error.message.contains('/'), "{command}: {}", error.message);
        }
        assert!(!fixture.project.path().join("AGENTS.md").exists());
        assert!(!fixture.project.path().join(".omnifrons/work-area").exists());
        assert_eq!(fixture.snapshots_dir_entries(), 0, "nothing written");
    }

    /// A link planted at the guidance name is never followed: refused as
    /// `guidance-file-invalid`, the link and its target untouched.
    #[cfg(unix)] // a symbolic link fixture
    #[test]
    fn a_link_at_the_guidance_name_is_guidance_file_invalid() {
        let fixture = Fixture::new("link");
        let elsewhere = TempDir::new("link-target");
        std::fs::write(elsewhere.path().join("real.md"), b"# real\n").expect("target");
        std::os::unix::fs::symlink(
            elsewhere.path().join("real.md"),
            fixture.project.path().join("AGENTS.md"),
        )
        .expect("plant");
        let error = fixture.status(&Fixture::agents()).expect_err("refused");
        assert_eq!(error.code, ShellErrorCode::GuidanceFileInvalid);
        let error = fixture
            .apply(&Fixture::agents(), None)
            .expect_err("refused");
        assert_eq!(error.code, ShellErrorCode::GuidanceFileInvalid);
        assert_eq!(
            std::fs::read(elsewhere.path().join("real.md")).expect("target"),
            b"# real\n"
        );
    }

    /// Every installer error maps to a fixed, slash-free catalogue message
    /// under its own code.
    #[test]
    fn guidance_errors_map_to_their_codes_with_slash_free_messages() {
        use omnifrons_app::guidance::GuidanceError;
        use omnifrons_app::managed_file::ProjectTextFileError;
        use omnifrons_app::snapshot_store::SnapshotStoreError;
        let cases = [
            (
                GuidanceError::WorkAreaInvalid,
                ShellErrorCode::WorkAreaInvalid,
            ),
            (
                GuidanceError::FileInvalid(ProjectTextFileError::NotAFile),
                ShellErrorCode::GuidanceFileInvalid,
            ),
            (
                GuidanceError::FileInvalid(ProjectTextFileError::TooLarge),
                ShellErrorCode::GuidanceFileInvalid,
            ),
            (
                GuidanceError::FileInvalid(ProjectTextFileError::NotUtf8),
                ShellErrorCode::GuidanceFileInvalid,
            ),
            (
                GuidanceError::FileInvalid(ProjectTextFileError::Unreadable),
                ShellErrorCode::GuidanceFileInvalid,
            ),
            (
                GuidanceError::FileInvalid(ProjectTextFileError::WriteFailed),
                ShellErrorCode::GuidanceFileInvalid,
            ),
            (
                GuidanceError::FileChanged,
                ShellErrorCode::GuidanceFileChanged,
            ),
            (
                GuidanceError::BlockModified,
                ShellErrorCode::GuidanceBlockModified,
            ),
            (
                GuidanceError::BlockMalformed,
                ShellErrorCode::GuidanceBlockMalformed,
            ),
            (GuidanceError::Unmanaged, ShellErrorCode::GuidanceUnmanaged),
            (
                GuidanceError::Snapshot(SnapshotStoreError::Unknown),
                ShellErrorCode::GuidanceUnmanaged,
            ),
            (
                GuidanceError::Snapshot(SnapshotStoreError::Corrupt),
                ShellErrorCode::SnapshotUnavailable,
            ),
            (
                GuidanceError::Snapshot(SnapshotStoreError::Unreadable),
                ShellErrorCode::SnapshotUnavailable,
            ),
            (
                GuidanceError::Snapshot(SnapshotStoreError::WriteFailed),
                ShellErrorCode::SnapshotUnavailable,
            ),
            (
                GuidanceError::VerifyFailed,
                ShellErrorCode::GuidanceFileInvalid,
            ),
        ];
        for (error, code) in cases {
            let mapped = ShellError::from(error);
            assert_eq!(mapped.code, code, "{error:?}");
            assert!(!mapped.message.contains('/'), "{}", mapped.message);
            assert!(mapped.detail.is_none());
        }
    }
}
