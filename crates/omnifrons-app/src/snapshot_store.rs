//! The `SnapshotStore` port (spike slice 5c, HAP-001 D18's installer
//! discipline): before any write to a managed file, the file as it was --
//! its bytes and a manifest saying whether it existed, its digest, its
//! size, and when it was taken -- is recorded in the product work area;
//! identical states are deduplicated, a bounded number of unpinned
//! snapshots is kept per file, the user can pin one, and a restore recovers
//! one. Device-local, never a roaming payload (the same discipline as the
//! publication journal's). Persisted by `omnifrons-adapters`'
//! `FsSnapshotStore`; the pure rules every implementation applies --
//! newest-first order, dedup against the most recent snapshot of the same
//! file, and the prune set -- live here so the fake and the real store
//! cannot drift.

use std::time::{Duration, SystemTime};

use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::guidance::ManagedFileKind;
use omnifrons_domain::publication::ProjectIdentity;

/// The manifest schema integer every snapshot this version writes carries.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// How many unpinned snapshots are kept per managed file (HAP-001 D18's
/// "a bounded number kept"; the gentle-ai precedent keeps five). Pinned
/// snapshots are never pruned.
pub const MAX_UNPINNED_SNAPSHOTS: usize = 5;

/// The domain tag mixed into a snapshot id's preimage, so the id can never
/// collide with any other hash computed in this codebase.
pub const SNAPSHOT_ID_DOMAIN: &[u8] = b"omnifrons-guidance-snapshot-v1";

/// A snapshot's identifier: the first 8 bytes of the SHA-256 of
/// [`snapshot_id_preimage`], carried on the wire as 16 hex characters (a
/// `u64` does not survive a JSON number's 53-bit mantissa).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SnapshotId(pub u64);

impl SnapshotId {
    /// The 16-character lowercase hex form.
    #[must_use]
    pub fn to_hex(&self) -> String {
        format!("{:016x}", self.0)
    }

    /// Parse the hex form, or `None` if `text` is not 16 hex characters.
    #[must_use]
    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != 16 {
            return None;
        }
        u64::from_str_radix(text, 16).ok().map(Self)
    }
}

/// The exact bytes a snapshot id is the SHA-256 of: [`SNAPSHOT_ID_DOMAIN`]
/// ‖ the project identity's 32 bytes ‖ the file digest's 32 bytes ‖ the
/// instant as nanoseconds since the Unix epoch, big-endian `u128`.
#[must_use]
pub fn snapshot_id_preimage(
    project: &ProjectIdentity,
    digest: &Sha256Digest,
    taken_at: SystemTime,
) -> Vec<u8> {
    let nanos: u128 = taken_at
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    let mut preimage = SNAPSHOT_ID_DOMAIN.to_vec();
    preimage.extend_from_slice(&project.0.0);
    preimage.extend_from_slice(&digest.0);
    preimage.extend_from_slice(&nanos.to_be_bytes());
    preimage
}

/// One snapshot's manifest: the state of a managed file at one instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotManifest {
    /// [`SNAPSHOT_SCHEMA_VERSION`].
    pub schema: u32,
    /// The snapshot's id.
    pub id: SnapshotId,
    /// The kind of managed file.
    pub kind: ManagedFileKind,
    /// The file's name at the workspace root.
    pub file: String,
    /// Whether the file existed; a snapshot of an absent file carries no
    /// bytes, and restoring it removes the file.
    pub existed: bool,
    /// The digest of the bytes recorded (of no bytes, for an absent file).
    pub sha256: Sha256Digest,
    /// How many bytes were recorded.
    pub size: u64,
    /// When the snapshot was taken.
    pub taken_at: SystemTime,
    /// Whether the user pinned it: never pruned while pinned.
    pub pinned: bool,
}

impl SnapshotManifest {
    /// Whether `other` records the same state of the same file: the same
    /// kind and name, the same existence, and the same digest -- what
    /// dedup compares.
    #[must_use]
    pub fn same_state_as(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.file == other.file
            && self.existed == other.existed
            && self.sha256 == other.sha256
    }

    /// Whether this manifest is of `kind` and `file`.
    #[must_use]
    pub fn is_of(&self, kind: ManagedFileKind, file: &str) -> bool {
        self.kind == kind && self.file == file
    }
}

/// Why a [`SnapshotStore`] operation failed. Closed and exhaustive, never
/// a raw `io::Error` whose text can carry a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotStoreError {
    /// The store could not be read.
    #[error("the snapshot store could not be read")]
    Unreadable,
    /// A manifest does not parse: fails closed for the whole listing,
    /// never silently skipped.
    #[error("a snapshot manifest is corrupt")]
    Corrupt,
    /// A write did not complete.
    #[error("the snapshot store could not be written")]
    WriteFailed,
    /// No snapshot with that id exists for the project.
    #[error("no snapshot with that id exists")]
    Unknown,
}

impl SnapshotStoreError {
    /// This error's fixed reason token.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::Unreadable => "snapshot-store-unreadable",
            Self::Corrupt => "snapshot-store-corrupt",
            Self::WriteFailed => "snapshot-write-failed",
            Self::Unknown => "no-such-snapshot",
        }
    }
}

/// A port over the device's snapshots, per project identity.
pub trait SnapshotStore {
    /// Record `bytes` under `manifest` for `project` -- unless the most
    /// recent snapshot of the same kind and file records the same state
    /// (the same `existed` and `sha256`), in which case nothing is written
    /// and that snapshot's id comes back -- then prune: the
    /// [`MAX_UNPINNED_SNAPSHOTS`] newest unpinned snapshots of that kind
    /// and file are kept, pinned ones always. Returns the id of the
    /// snapshot that stands for the state.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotStoreError`] if the store cannot be read, is
    /// corrupt, or cannot be written.
    fn record(
        &mut self,
        project: &ProjectIdentity,
        manifest: SnapshotManifest,
        bytes: &[u8],
    ) -> Result<SnapshotId, SnapshotStoreError>;

    /// Every snapshot of `kind` for `project`, newest first.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotStoreError`] if the store cannot be read or any
    /// manifest is corrupt.
    fn list(
        &self,
        project: &ProjectIdentity,
        kind: ManagedFileKind,
    ) -> Result<Vec<SnapshotManifest>, SnapshotStoreError>;

    /// The snapshot `id` of `project`: its manifest and bytes.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotStoreError::Unknown`] if no such snapshot exists,
    /// or [`SnapshotStoreError`] if it cannot be read or is corrupt.
    fn read(
        &self,
        project: &ProjectIdentity,
        id: SnapshotId,
    ) -> Result<(SnapshotManifest, Vec<u8>), SnapshotStoreError>;

    /// Pin or unpin the snapshot `id` of `project`.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotStoreError::Unknown`] if no such snapshot exists,
    /// or [`SnapshotStoreError`] if the manifest cannot be rewritten.
    fn pin(
        &mut self,
        project: &ProjectIdentity,
        id: SnapshotId,
        pinned: bool,
    ) -> Result<(), SnapshotStoreError>;
}

/// Order `manifests` newest first: by `taken_at` descending, then by id
/// descending so two snapshots of one instant keep a stable order.
pub fn sort_newest_first(manifests: &mut [SnapshotManifest]) {
    manifests.sort_by(|a, b| b.taken_at.cmp(&a.taken_at).then_with(|| b.id.cmp(&a.id)));
}

/// The id an identical state dedups to: the most recent snapshot of
/// `manifest`'s kind and file, when it records the same state.
#[must_use]
pub fn dedup_target(
    newest_first: &[SnapshotManifest],
    manifest: &SnapshotManifest,
) -> Option<SnapshotId> {
    newest_first
        .iter()
        .find(|existing| existing.is_of(manifest.kind, &manifest.file))
        .filter(|existing| existing.same_state_as(manifest))
        .map(|existing| existing.id)
}

/// The snapshots to prune from a newest-first list: every unpinned
/// snapshot of `kind` and `file` beyond the [`MAX_UNPINNED_SNAPSHOTS`]
/// newest ones. Pinned snapshots are never listed.
#[must_use]
pub fn prune_targets(
    newest_first: &[SnapshotManifest],
    kind: ManagedFileKind,
    file: &str,
) -> Vec<SnapshotId> {
    newest_first
        .iter()
        .filter(|manifest| manifest.is_of(kind, file) && !manifest.pinned)
        .skip(MAX_UNPINNED_SNAPSHOTS)
        .map(|manifest| manifest.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_UNPINNED_SNAPSHOTS, SnapshotId, SnapshotManifest, dedup_target, prune_targets,
        sort_newest_first,
    };
    use omnifrons_domain::executable::Sha256Digest;
    use omnifrons_domain::guidance::ManagedFileKind;
    use std::time::{Duration, SystemTime};

    fn manifest(n: u64, secs: u64, pinned: bool) -> SnapshotManifest {
        SnapshotManifest {
            schema: 1,
            id: SnapshotId(n),
            kind: ManagedFileKind::Guidance,
            file: "AGENTS.md".to_string(),
            existed: true,
            sha256: Sha256Digest([u8::try_from(n).expect("small"); 32]),
            size: 1,
            taken_at: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
            pinned,
        }
    }

    #[test]
    fn newest_first_orders_by_instant_then_id() {
        let mut manifests = vec![
            manifest(1, 10, false),
            manifest(3, 20, false),
            manifest(2, 20, false),
        ];
        sort_newest_first(&mut manifests);
        let ids: Vec<u64> = manifests.iter().map(|m| m.id.0).collect();
        assert_eq!(ids, vec![3, 2, 1]);
    }

    #[test]
    fn dedup_compares_against_the_most_recent_of_the_same_file_only() {
        let newest_first = vec![manifest(2, 20, false), manifest(1, 10, false)];
        let same_as_newest = SnapshotManifest {
            id: SnapshotId(9),
            ..manifest(2, 30, false)
        };
        assert_eq!(
            dedup_target(&newest_first, &same_as_newest),
            Some(SnapshotId(2))
        );
        let same_as_older = SnapshotManifest {
            id: SnapshotId(9),
            ..manifest(1, 30, false)
        };
        assert_eq!(dedup_target(&newest_first, &same_as_older), None);
        let other_file = SnapshotManifest {
            file: "CLAUDE.md".to_string(),
            ..same_as_newest
        };
        assert_eq!(dedup_target(&newest_first, &other_file), None);
    }

    #[test]
    fn prune_keeps_the_newest_unpinned_and_every_pinned() {
        let mut manifests: Vec<SnapshotManifest> =
            (1..=8).map(|n| manifest(n, n * 10, n == 2)).collect();
        sort_newest_first(&mut manifests);
        let pruned = prune_targets(&manifests, ManagedFileKind::Guidance, "AGENTS.md");
        // Unpinned newest first: 8, 7, 6, 5, 4, 3, 1 -> keep five, prune 3 and 1.
        assert_eq!(pruned, vec![SnapshotId(3), SnapshotId(1)]);
        assert_eq!(MAX_UNPINNED_SNAPSHOTS, 5);
        assert!(prune_targets(&manifests, ManagedFileKind::Ignore, ".gitignore").is_empty());
    }
}
