//! `FsSnapshotStore`: the real
//! `omnifrons_app::snapshot_store::SnapshotStore` (spike slice 5c,
//! HAP-001 D18's installer discipline) under the product work area's
//! `snapshots/<project identity hex>/`: one `<id>.json` manifest
//! (`camelCase`, `schema: 1`, `deny_unknown_fields`, the id, kind, file,
//! existence, digest, size, instant, and pin) beside one `<id>.bytes`
//! file, both owner-only on unix (`0o600`, the directory `0o700`), each
//! written to a sibling temporary name and renamed into place. The bytes
//! are written before the manifest, so a manifest never names bytes that
//! are not there. A manifest that does not parse, carries another schema,
//! or sits under a file name that disagrees with its id fails the whole
//! listing closed as `Corrupt` -- and with it every record, which needs
//! the listing for dedup and prune -- never a listing with the bad one
//! skipped; the read of that id fails the same way, and another id still
//! reads. Device-local, never a roaming payload.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_app::snapshot_store::{
    SNAPSHOT_SCHEMA_VERSION, SnapshotId, SnapshotManifest, SnapshotStore, SnapshotStoreError,
    dedup_target, prune_targets, sort_newest_first,
};
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::guidance::ManagedFileKind;
use omnifrons_domain::publication::ProjectIdentity;
use serde::{Deserialize, Serialize};

use crate::catalog_record_dto::TimestampDto;

/// One manifest file's shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestDto {
    schema: u32,
    id: String,
    kind: String,
    file: String,
    existed: bool,
    sha256: String,
    size: u64,
    taken_at: TimestampDto,
    pinned: bool,
}

impl From<&SnapshotManifest> for ManifestDto {
    fn from(manifest: &SnapshotManifest) -> Self {
        Self {
            schema: manifest.schema,
            id: manifest.id.to_hex(),
            kind: manifest.kind.as_str().to_string(),
            file: manifest.file.clone(),
            existed: manifest.existed,
            sha256: manifest.sha256.to_hex(),
            size: manifest.size,
            taken_at: manifest.taken_at.into(),
            pinned: manifest.pinned,
        }
    }
}

impl TryFrom<ManifestDto> for SnapshotManifest {
    type Error = SnapshotStoreError;

    fn try_from(dto: ManifestDto) -> Result<Self, SnapshotStoreError> {
        if dto.schema != SNAPSHOT_SCHEMA_VERSION {
            return Err(SnapshotStoreError::Corrupt);
        }
        Ok(Self {
            schema: dto.schema,
            id: SnapshotId::from_hex(&dto.id).ok_or(SnapshotStoreError::Corrupt)?,
            kind: ManagedFileKind::parse(&dto.kind).ok_or(SnapshotStoreError::Corrupt)?,
            file: dto.file,
            existed: dto.existed,
            sha256: Sha256Digest::from_hex(&dto.sha256).ok_or(SnapshotStoreError::Corrupt)?,
            size: dto.size,
            taken_at: dto.taken_at.into(),
            pinned: dto.pinned,
        })
    }
}

/// The process-wide sequence behind the temporary names, so two writes of
/// one manifest never share a temporary file.
static PART_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Create `dir` if missing and make it owner-only where the platform
/// expresses that (unix `0o700`, set explicitly so a permissive umask
/// cannot widen it).
fn create_owner_only_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Write `bytes` to `final_path` through a sibling temporary file created
/// exclusively (owner-only on unix), synced, and renamed into place.
fn write_owner_only(final_path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = final_path.parent().unwrap_or_else(|| Path::new("."));
    let name = final_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sequence = PART_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".{name}.{}-{sequence}.part", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_data());
    drop(file);
    let placed = written.and_then(|()| std::fs::rename(&temp, final_path));
    if placed.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    placed
}

/// The JSONL-free, file-per-snapshot [`SnapshotStore`] of one work area.
/// Stateless beyond its root: every call re-reads the directory.
#[derive(Debug, Clone)]
pub struct FsSnapshotStore {
    root: PathBuf,
}

impl FsSnapshotStore {
    /// The manifest file's extension.
    pub const MANIFEST_EXTENSION: &'static str = "json";

    /// The bytes file's extension.
    pub const BYTES_EXTENSION: &'static str = "bytes";

    /// The store under `work_area`'s snapshots directory.
    #[must_use]
    pub fn open(work_area: &WorkAreaRoot) -> Self {
        Self {
            root: work_area.snapshots_dir(),
        }
    }

    fn project_dir(&self, project: &ProjectIdentity) -> PathBuf {
        self.root.join(project.to_hex())
    }

    fn manifest_path(dir: &Path, id: SnapshotId) -> PathBuf {
        dir.join(format!("{}.{}", id.to_hex(), Self::MANIFEST_EXTENSION))
    }

    fn bytes_path(dir: &Path, id: SnapshotId) -> PathBuf {
        dir.join(format!("{}.{}", id.to_hex(), Self::BYTES_EXTENSION))
    }

    /// Read and check the manifest at `path`, which must carry `id`.
    fn read_manifest(path: &Path, id: SnapshotId) -> Result<SnapshotManifest, SnapshotStoreError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(SnapshotStoreError::Unknown);
            }
            Err(_) => return Err(SnapshotStoreError::Unreadable),
        };
        let dto: ManifestDto =
            serde_json::from_str(&text).map_err(|_| SnapshotStoreError::Corrupt)?;
        let manifest = SnapshotManifest::try_from(dto)?;
        if manifest.id != id {
            return Err(SnapshotStoreError::Corrupt);
        }
        Ok(manifest)
    }

    /// Every manifest of `project`, newest first; a missing directory is
    /// an empty store, and any manifest that does not parse fails the
    /// whole listing.
    fn manifests(
        &self,
        project: &ProjectIdentity,
    ) -> Result<Vec<SnapshotManifest>, SnapshotStoreError> {
        let dir = self.project_dir(project);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(SnapshotStoreError::Unreadable),
        };
        let mut manifests = Vec::new();
        for entry in entries {
            let path = entry.map_err(|_| SnapshotStoreError::Unreadable)?.path();
            if path.extension().and_then(|extension| extension.to_str())
                != Some(Self::MANIFEST_EXTENSION)
            {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(SnapshotId::from_hex)
                .ok_or(SnapshotStoreError::Corrupt)?;
            manifests.push(Self::read_manifest(&path, id)?);
        }
        sort_newest_first(&mut manifests);
        Ok(manifests)
    }

    fn write_manifest(dir: &Path, manifest: &SnapshotManifest) -> Result<(), SnapshotStoreError> {
        let text = serde_json::to_string(&ManifestDto::from(manifest))
            .map_err(|_| SnapshotStoreError::WriteFailed)?;
        write_owner_only(&Self::manifest_path(dir, manifest.id), text.as_bytes())
            .map_err(|_| SnapshotStoreError::WriteFailed)
    }

    /// Delete the snapshot `id`'s files; nothing to delete is nothing.
    fn delete(dir: &Path, id: SnapshotId) -> Result<(), SnapshotStoreError> {
        for path in [Self::manifest_path(dir, id), Self::bytes_path(dir, id)] {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(SnapshotStoreError::WriteFailed),
            }
        }
        Ok(())
    }
}

impl SnapshotStore for FsSnapshotStore {
    fn record(
        &mut self,
        project: &ProjectIdentity,
        manifest: SnapshotManifest,
        bytes: &[u8],
    ) -> Result<SnapshotId, SnapshotStoreError> {
        let mut newest_first = self.manifests(project)?;
        if let Some(id) = dedup_target(&newest_first, &manifest) {
            return Ok(id);
        }
        let dir = self.project_dir(project);
        create_owner_only_dir(&dir).map_err(|_| SnapshotStoreError::WriteFailed)?;
        let id = manifest.id;
        // The bytes first: a manifest never names bytes that are not there.
        write_owner_only(&Self::bytes_path(&dir, id), bytes)
            .map_err(|_| SnapshotStoreError::WriteFailed)?;
        Self::write_manifest(&dir, &manifest)?;
        let (kind, file) = (manifest.kind, manifest.file.clone());
        newest_first.push(manifest);
        sort_newest_first(&mut newest_first);
        for pruned in prune_targets(&newest_first, kind, &file) {
            Self::delete(&dir, pruned)?;
        }
        Ok(id)
    }

    fn list(
        &self,
        project: &ProjectIdentity,
        kind: ManagedFileKind,
    ) -> Result<Vec<SnapshotManifest>, SnapshotStoreError> {
        Ok(self
            .manifests(project)?
            .into_iter()
            .filter(|manifest| manifest.kind == kind)
            .collect())
    }

    fn read(
        &self,
        project: &ProjectIdentity,
        id: SnapshotId,
    ) -> Result<(SnapshotManifest, Vec<u8>), SnapshotStoreError> {
        let dir = self.project_dir(project);
        let manifest = Self::read_manifest(&Self::manifest_path(&dir, id), id)?;
        let bytes = match std::fs::read(Self::bytes_path(&dir, id)) {
            Ok(bytes) => bytes,
            // A manifest naming bytes that are not there is corrupt.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(SnapshotStoreError::Corrupt);
            }
            Err(_) => return Err(SnapshotStoreError::Unreadable),
        };
        Ok((manifest, bytes))
    }

    fn pin(
        &mut self,
        project: &ProjectIdentity,
        id: SnapshotId,
        pinned: bool,
    ) -> Result<(), SnapshotStoreError> {
        let dir = self.project_dir(project);
        let mut manifest = Self::read_manifest(&Self::manifest_path(&dir, id), id)?;
        manifest.pinned = pinned;
        Self::write_manifest(&dir, &manifest)
    }
}
