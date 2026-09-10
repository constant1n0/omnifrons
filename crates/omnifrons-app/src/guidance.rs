//! The guidance-note installer (spike slice 5c, HAP-001 D18 at its
//! default: the system proposes, the user disposes, and every write is an
//! approval-mediated workspace write), under the installer discipline D18
//! names -- a snapshot before any write, identical snapshots deduplicated,
//! a bounded number kept and pinnable, a restore, a removal that deletes
//! only the lines Omnifrons owns, and an idempotent re-apply -- generic
//! over the ports: the managed text file, the snapshot store in the
//! product work area (re-checked at every use, HAP-001-R7), the hasher,
//! and the clock.
//!
//! Every operation runs in a fixed order: the work area re-check, the
//! current file read through one no-follow open, the binding of the
//! request to the digest the surface showed (`FileChanged` otherwise), the
//! block refusals (`BlockModified`, `BlockMalformed`: neither is replaced
//! or removed; the user resolves by hand or restores), the pure plan, no
//! snapshot for a no-op, the snapshot (deduplicated and pruned by the
//! store), the atomic write -- carrying the bytes it was planned from, so
//! the port re-checks the target immediately before the rename and refuses
//! `FileChanged` rather than discarding a rewrite that landed since
//! (R1-002) -- and the read-back that verifies the digest.
//! Nothing here ever reads the note as an instruction: the note is content
//! the product proposes, never authority (HAP-001-R22, R42).

use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::guidance::{
    ApplyAction, IGNORE_FILE, LineEnding, ManagedBlock, ManagedFileError, ManagedFileKind,
    ManagedFileName, ManagedStatus, ManagedTarget, plan_apply, plan_remove, status as block_status,
};
use omnifrons_domain::outbox::OutboxPath;
use omnifrons_domain::publication::ProjectIdentity;

use crate::clock::Clock;
use crate::content_hasher::{ContentHasher, derive_project_identity, derive_snapshot_id};
use crate::harness_adapter::WorkspaceRoot;
use crate::managed_file::{ProjectTextFile, ProjectTextFileError};
use crate::snapshot_store::{
    SNAPSHOT_SCHEMA_VERSION, SnapshotId, SnapshotManifest, SnapshotStore, SnapshotStoreError,
};
use crate::work_area::WorkAreaRoot;

/// The ports one installer operation runs over.
pub struct GuidancePorts<'a> {
    /// The managed text file at the workspace root.
    pub files: &'a dyn ProjectTextFile,
    /// The snapshot store under the product work area.
    pub snapshots: &'a mut dyn SnapshotStore,
    /// SHA-256 over bytes: the file digests and the sentinel digests.
    pub hasher: &'a dyn ContentHasher,
    /// The wall clock, for the snapshot instants.
    pub clock: &'a dyn Clock,
    /// The product work area, re-checked at every use.
    pub work_area: &'a WorkAreaRoot,
    /// Every registered workspace root the work area is checked against.
    pub workspaces: &'a [&'a WorkspaceRoot],
}

/// Why an installer operation was refused or failed. Every variant has a
/// fixed reason token ([`Self::reason`]); no message carries a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GuidanceError {
    /// The work area resolves inside a registered workspace root or cannot
    /// be used (HAP-001-R7).
    #[error("the product work area resolves inside a registered workspace root or cannot be used")]
    WorkAreaInvalid,
    /// The managed file cannot be read, written, or removed.
    #[error("the managed file cannot be used: {0}")]
    FileInvalid(ProjectTextFileError),
    /// The file's digest (or its absence) differs from what the surface
    /// showed: the request binds to that state and nothing is written.
    #[error("the managed file changed since it was shown")]
    FileChanged,
    /// The block was edited inside its sentinels.
    #[error("the managed block was modified inside its sentinels")]
    BlockModified,
    /// The sentinels are not one intact pair.
    #[error("the managed block is malformed")]
    BlockMalformed,
    /// A removal found no block to remove.
    #[error("the file carries no managed block")]
    Unmanaged,
    /// The snapshot store cannot be used, or names no such snapshot.
    #[error("the snapshot store cannot be used: {0}")]
    Snapshot(SnapshotStoreError),
    /// The file read back after the write does not carry the planned
    /// digest.
    #[error("the written file does not read back as planned")]
    VerifyFailed,
}

impl GuidanceError {
    /// This error's fixed reason token.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::WorkAreaInvalid => "work-area-invalid",
            Self::FileInvalid(error) => error.reason(),
            Self::FileChanged => "file-changed",
            Self::BlockModified => "block-modified",
            Self::BlockMalformed => "block-malformed",
            Self::Unmanaged => "unmanaged",
            Self::Snapshot(error) => error.reason(),
            Self::VerifyFailed => "verify-failed",
        }
    }
}

/// A managed file's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceStatus {
    /// The file's name at the workspace root.
    pub file: String,
    /// Whether the file exists.
    pub exists: bool,
    /// The block's state in it.
    pub managed: ManagedStatus,
    /// The file's digest, when it exists: what a request binds to.
    pub file_sha256: Option<Sha256Digest>,
    /// How many snapshots of this file the store holds.
    pub snapshots: usize,
    /// How many of them are pinned.
    pub pinned: usize,
}

/// What an apply would write, before it is approved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidancePreview {
    /// The file's name at the workspace root.
    pub file: String,
    /// What the apply would do.
    pub action: ApplyAction,
    /// The block as it would be written, in the file's own line ending.
    pub proposed_block: String,
    /// The file's digest now, when it exists: what the apply must bind to.
    pub file_sha256: Option<Sha256Digest>,
    /// The digest of the whole file afterwards.
    pub result_sha256: Sha256Digest,
}

/// What an apply did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceApplied {
    /// What was done.
    pub action: ApplyAction,
    /// The snapshot taken before the write; `None` for a no-op, which
    /// writes nothing and snapshots nothing.
    pub snapshot_id: Option<SnapshotId>,
    /// The digest of the whole file afterwards, verified by a read-back.
    pub result_sha256: Sha256Digest,
}

/// What a removal did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceRemoved {
    /// The snapshot taken before the write.
    pub snapshot_id: SnapshotId,
    /// The digest of the whole file afterwards, or `None` when the file
    /// -- created by Omnifrons and left empty -- was removed.
    pub result_sha256: Option<Sha256Digest>,
}

/// What a restore did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceRestored {
    /// The file the snapshot recorded and the restore wrote.
    pub target: ManagedTarget,
    /// The snapshot taken of the state before the restore.
    pub snapshot_id: SnapshotId,
    /// The snapshot brought back.
    pub restored: SnapshotId,
    /// The digest of the whole file afterwards, or `None` when the
    /// snapshot recorded an absent file and the file was removed.
    pub result_sha256: Option<Sha256Digest>,
}

/// The managed file as read: its bytes, when it exists, and their digest.
struct Current {
    bytes: Option<Vec<u8>>,
    digest: Option<Sha256Digest>,
}

impl Current {
    /// The content as text; the port refused invalid UTF-8 already, so a
    /// failure here is the port's own contract broken.
    fn text(&self) -> Result<Option<&str>, GuidanceError> {
        self.bytes
            .as_deref()
            .map(|bytes| {
                std::str::from_utf8(bytes)
                    .map_err(|_| GuidanceError::FileInvalid(ProjectTextFileError::NotUtf8))
            })
            .transpose()
    }
}

/// HAP-001-R7: the work area is re-checked before anything else, at
/// every operation.
fn check_work_area(ports: &GuidancePorts<'_>) -> Result<(), GuidanceError> {
    ports
        .work_area
        .check(ports.workspaces)
        .map_err(|_| GuidanceError::WorkAreaInvalid)
}

/// Read the managed file through the port and digest it.
fn read_current(
    ports: &GuidancePorts<'_>,
    workspace: &WorkspaceRoot,
    target: &ManagedTarget,
) -> Result<Current, GuidanceError> {
    let bytes = ports
        .files
        .read(workspace, target)
        .map_err(GuidanceError::FileInvalid)?;
    let digest = bytes.as_deref().map(|bytes| ports.hasher.sha256(bytes));
    Ok(Current { bytes, digest })
}

/// The request binds to the digest the surface showed, or to the file's
/// absence.
fn bind(current: &Current, expected: Option<Sha256Digest>) -> Result<(), GuidanceError> {
    if current.digest == expected {
        Ok(())
    } else {
        Err(GuidanceError::FileChanged)
    }
}

/// The domain's block refusals, as this module names them.
fn block_error(error: ManagedFileError) -> GuidanceError {
    match error {
        ManagedFileError::Modified => GuidanceError::BlockModified,
        ManagedFileError::Malformed => GuidanceError::BlockMalformed,
    }
}

/// The write ports' refusals, as this module names them. The port's own
/// "the target is not what you read" (R1-002) is the same refusal as the
/// digest binding's: the surface reads the status again and retries.
fn write_error(error: ProjectTextFileError) -> GuidanceError {
    match error {
        ProjectTextFileError::Changed => GuidanceError::FileChanged,
        other => GuidanceError::FileInvalid(other),
    }
}

/// The sentinel digest function over the hasher.
fn digest_fn(hasher: &dyn ContentHasher) -> impl Fn(&[u8]) -> Sha256Digest + '_ {
    move |bytes| hasher.sha256(bytes)
}

/// Snapshot the file as it is -- its bytes, or its absence -- before a
/// write; the store deduplicates an identical state and prunes.
fn snapshot_current(
    ports: &mut GuidancePorts<'_>,
    project: &ProjectIdentity,
    target: &ManagedTarget,
    current: &Current,
) -> Result<SnapshotId, GuidanceError> {
    let bytes = current.bytes.as_deref().unwrap_or(&[]);
    let digest = current.digest.unwrap_or_else(|| ports.hasher.sha256(&[]));
    let now = ports.clock.now();
    let manifest = SnapshotManifest {
        schema: SNAPSHOT_SCHEMA_VERSION,
        id: derive_snapshot_id(ports.hasher, project, &digest, now),
        kind: target.kind(),
        file: target.file_name().to_string(),
        existed: current.bytes.is_some(),
        sha256: digest,
        size: bytes.len() as u64,
        taken_at: now,
        pinned: false,
    };
    ports
        .snapshots
        .record(project, manifest, bytes)
        .map_err(GuidanceError::Snapshot)
}

/// Write `content` atomically and read it back: the digest of what is on
/// disk must be the digest of what was planned. `expected` is the content
/// the plan was made from, re-checked by the port immediately before the
/// rename (R1-002).
fn write_and_verify(
    ports: &GuidancePorts<'_>,
    workspace: &WorkspaceRoot,
    target: &ManagedTarget,
    expected: Option<&[u8]>,
    content: &[u8],
) -> Result<Sha256Digest, GuidanceError> {
    ports
        .files
        .replace(workspace, target, expected, content)
        .map_err(write_error)?;
    let expected = ports.hasher.sha256(content);
    let written = ports
        .files
        .read(workspace, target)
        .map_err(GuidanceError::FileInvalid)?;
    match written {
        Some(bytes) if ports.hasher.sha256(&bytes) == expected => Ok(expected),
        _ => Err(GuidanceError::VerifyFailed),
    }
}

/// Remove the file and read back its absence. `expected` is the content
/// the plan was made from, re-checked by the port immediately before the
/// removal (R1-002).
fn remove_and_verify(
    ports: &GuidancePorts<'_>,
    workspace: &WorkspaceRoot,
    target: &ManagedTarget,
    expected: Option<&[u8]>,
) -> Result<(), GuidanceError> {
    ports
        .files
        .remove(workspace, target, expected)
        .map_err(write_error)?;
    match ports
        .files
        .read(workspace, target)
        .map_err(GuidanceError::FileInvalid)?
    {
        None => Ok(()),
        Some(_) => Err(GuidanceError::VerifyFailed),
    }
}

/// The managed file `target`'s state with respect to the block its kind
/// manages for `outbox`.
///
/// # Errors
///
/// Returns [`GuidanceError::WorkAreaInvalid`], [`GuidanceError::FileInvalid`],
/// or [`GuidanceError::Snapshot`].
pub fn status(
    ports: &mut GuidancePorts<'_>,
    workspace: &WorkspaceRoot,
    target: &ManagedTarget,
    outbox: &OutboxPath,
) -> Result<GuidanceStatus, GuidanceError> {
    check_work_area(ports)?;
    let project = derive_project_identity(ports.hasher, workspace);
    let current = read_current(ports, workspace, target)?;
    let hasher = ports.hasher;
    let digest = digest_fn(hasher);
    let block = ManagedBlock::for_kind(target.kind(), outbox, &digest);
    let managed = block_status(current.text()?, &block, &digest);
    let listed = ports
        .snapshots
        .list(&project, target.kind())
        .map_err(GuidanceError::Snapshot)?;
    let of_file = listed
        .iter()
        .filter(|manifest| manifest.file == target.file_name());
    let (snapshots, pinned) = of_file.fold((0, 0), |(count, pinned), manifest| {
        (count + 1, pinned + usize::from(manifest.pinned))
    });
    Ok(GuidanceStatus {
        file: target.file_name().to_string(),
        exists: current.bytes.is_some(),
        managed,
        file_sha256: current.digest,
        snapshots,
        pinned,
    })
}

/// What applying the block for `outbox` to `target` would write: the
/// proposal the user disposes of (D18). Writes nothing and snapshots
/// nothing.
///
/// # Errors
///
/// Returns [`GuidanceError::WorkAreaInvalid`], [`GuidanceError::FileInvalid`],
/// [`GuidanceError::BlockModified`], or [`GuidanceError::BlockMalformed`].
pub fn preview(
    ports: &mut GuidancePorts<'_>,
    workspace: &WorkspaceRoot,
    target: &ManagedTarget,
    outbox: &OutboxPath,
) -> Result<GuidancePreview, GuidanceError> {
    check_work_area(ports)?;
    let current = read_current(ports, workspace, target)?;
    let hasher = ports.hasher;
    let digest = digest_fn(hasher);
    let block = ManagedBlock::for_kind(target.kind(), outbox, &digest);
    let text = current.text()?;
    let plan = plan_apply(text, &block, &digest).map_err(block_error)?;
    let ending = text.map_or(LineEnding::Lf, LineEnding::dominant);
    Ok(GuidancePreview {
        file: target.file_name().to_string(),
        action: plan.action,
        proposed_block: block.render(ending),
        file_sha256: current.digest,
        result_sha256: hasher.sha256(plan.result.as_bytes()),
    })
}

/// Apply the block for `outbox` to `target` -- the user's approved write
/// (D18) -- bound to `expected_file_sha256`, the file digest the surface
/// showed (`None` for an absent file): the snapshot, the write, the
/// read-back, in that order; a no-op writes and snapshots nothing.
///
/// # Errors
///
/// Returns [`GuidanceError`] naming the step that refused or failed.
pub fn apply(
    ports: &mut GuidancePorts<'_>,
    workspace: &WorkspaceRoot,
    target: &ManagedTarget,
    outbox: &OutboxPath,
    expected_file_sha256: Option<Sha256Digest>,
) -> Result<GuidanceApplied, GuidanceError> {
    check_work_area(ports)?;
    let project = derive_project_identity(ports.hasher, workspace);
    let current = read_current(ports, workspace, target)?;
    bind(&current, expected_file_sha256)?;
    let hasher = ports.hasher;
    let digest = digest_fn(hasher);
    let block = ManagedBlock::for_kind(target.kind(), outbox, &digest);
    let plan = plan_apply(current.text()?, &block, &digest).map_err(block_error)?;
    if plan.action == ApplyAction::NoOp {
        return Ok(GuidanceApplied {
            action: ApplyAction::NoOp,
            snapshot_id: None,
            result_sha256: current
                .digest
                .unwrap_or_else(|| hasher.sha256(plan.result.as_bytes())),
        });
    }
    let snapshot_id = snapshot_current(ports, &project, target, &current)?;
    let result_sha256 = write_and_verify(
        ports,
        workspace,
        target,
        current.bytes.as_deref(),
        plan.result.as_bytes(),
    )?;
    Ok(GuidanceApplied {
        action: plan.action,
        snapshot_id: Some(snapshot_id),
        result_sha256,
    })
}

/// Remove the block from `target` -- exactly the lines Omnifrons owns and
/// one adjacent blank line -- bound to `expected_file_sha256`; a file
/// Omnifrons created (its oldest snapshot recorded no file) that would be
/// left empty is removed. A modified or malformed block is refused: the
/// user resolves it by hand or restores a snapshot.
///
/// # Errors
///
/// Returns [`GuidanceError::Unmanaged`] when the file or the block is
/// absent, and every other variant as the step that refused or failed.
pub fn remove(
    ports: &mut GuidancePorts<'_>,
    workspace: &WorkspaceRoot,
    target: &ManagedTarget,
    expected_file_sha256: Option<Sha256Digest>,
) -> Result<GuidanceRemoved, GuidanceError> {
    check_work_area(ports)?;
    let project = derive_project_identity(ports.hasher, workspace);
    let current = read_current(ports, workspace, target)?;
    bind(&current, expected_file_sha256)?;
    let Some(text) = current.text()? else {
        return Err(GuidanceError::Unmanaged);
    };
    let hasher = ports.hasher;
    let digest = digest_fn(hasher);
    let located = ManagedBlock::locate(target.kind(), text, &digest)
        .map_err(block_error)?
        .ok_or(GuidanceError::Unmanaged)?;
    // Newest first, so the file's oldest snapshot is the last of its own:
    // the state before Omnifrons's first write.
    let created_by_us = ports
        .snapshots
        .list(&project, target.kind())
        .map_err(GuidanceError::Snapshot)?
        .iter()
        .rev()
        .find(|manifest| manifest.file == target.file_name())
        .is_some_and(|oldest| !oldest.existed);
    let plan = plan_remove(text, &located, created_by_us);
    let snapshot_id = snapshot_current(ports, &project, target, &current)?;
    let result_sha256 = if let Some(result) = plan.result {
        Some(write_and_verify(
            ports,
            workspace,
            target,
            current.bytes.as_deref(),
            result.as_bytes(),
        )?)
    } else {
        remove_and_verify(ports, workspace, target, current.bytes.as_deref())?;
        None
    };
    Ok(GuidanceRemoved {
        snapshot_id,
        result_sha256,
    })
}

/// The target a snapshot manifest names, or `Corrupt` when the manifest
/// names a file its kind could not manage.
fn target_of(manifest: &SnapshotManifest) -> Result<ManagedTarget, GuidanceError> {
    match manifest.kind {
        ManagedFileKind::Guidance => ManagedFileName::guidance(&manifest.file)
            .map(ManagedTarget::guidance)
            .map_err(|_| GuidanceError::Snapshot(SnapshotStoreError::Corrupt)),
        ManagedFileKind::Ignore if manifest.file == IGNORE_FILE => Ok(ManagedTarget::ignore()),
        ManagedFileKind::Ignore => Err(GuidanceError::Snapshot(SnapshotStoreError::Corrupt)),
    }
}

/// Restore the snapshot `id`, bound to `expected_file_sha256`: the state
/// before the restore is snapshotted first (deduplicated), then the
/// snapshot's bytes are written back, or the file is removed when the
/// snapshot recorded an absent file.
///
/// # Errors
///
/// Returns [`GuidanceError::Snapshot`] with
/// [`SnapshotStoreError::Unknown`] for an id the store does not hold, and
/// every other variant as the step that refused or failed.
pub fn restore(
    ports: &mut GuidancePorts<'_>,
    workspace: &WorkspaceRoot,
    id: SnapshotId,
    expected_file_sha256: Option<Sha256Digest>,
) -> Result<GuidanceRestored, GuidanceError> {
    check_work_area(ports)?;
    let project = derive_project_identity(ports.hasher, workspace);
    let (manifest, bytes) = ports
        .snapshots
        .read(&project, id)
        .map_err(GuidanceError::Snapshot)?;
    if ports.hasher.sha256(&bytes) != manifest.sha256 {
        return Err(GuidanceError::Snapshot(SnapshotStoreError::Corrupt));
    }
    let target = target_of(&manifest)?;
    let current = read_current(ports, workspace, &target)?;
    bind(&current, expected_file_sha256)?;
    let snapshot_id = snapshot_current(ports, &project, &target, &current)?;
    let result_sha256 = if manifest.existed {
        Some(write_and_verify(
            ports,
            workspace,
            &target,
            current.bytes.as_deref(),
            &bytes,
        )?)
    } else {
        remove_and_verify(ports, workspace, &target, current.bytes.as_deref())?;
        None
    };
    Ok(GuidanceRestored {
        target,
        snapshot_id,
        restored: id,
        result_sha256,
    })
}

/// Every snapshot of `kind` for the workspace's project, newest first.
///
/// # Errors
///
/// Returns [`GuidanceError::WorkAreaInvalid`] or [`GuidanceError::Snapshot`].
pub fn snapshots(
    ports: &mut GuidancePorts<'_>,
    workspace: &WorkspaceRoot,
    kind: ManagedFileKind,
) -> Result<Vec<SnapshotManifest>, GuidanceError> {
    check_work_area(ports)?;
    let project = derive_project_identity(ports.hasher, workspace);
    ports
        .snapshots
        .list(&project, kind)
        .map_err(GuidanceError::Snapshot)
}

/// Pin or unpin the snapshot `id` of the workspace's project: a pinned
/// snapshot is never pruned.
///
/// # Errors
///
/// Returns [`GuidanceError::WorkAreaInvalid`] or [`GuidanceError::Snapshot`].
pub fn pin(
    ports: &mut GuidancePorts<'_>,
    workspace: &WorkspaceRoot,
    id: SnapshotId,
    pinned: bool,
) -> Result<(), GuidanceError> {
    check_work_area(ports)?;
    let project = derive_project_identity(ports.hasher, workspace);
    ports
        .snapshots
        .pin(&project, id, pinned)
        .map_err(GuidanceError::Snapshot)
}
