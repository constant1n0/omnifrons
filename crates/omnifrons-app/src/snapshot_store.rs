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

// -- definition-file snapshots (ADR-0005 decisions 2 and 3) --

/// The domain tag mixed into a definition snapshot's id preimage, apart
/// from [`SNAPSHOT_ID_DOMAIN`] so the two id spaces can never collide.
pub const DEFINITION_SNAPSHOT_ID_DOMAIN: &[u8] = b"omnifrons-definition-snapshot-v1";

/// What a definition snapshot is of: one identity file of one agent
/// directory. The agent is the directory's path relative to the
/// workspace -- `.` for the workspace itself, otherwise `/`-separated
/// plain components -- and the file is a plain file name with no
/// separator. Validated on construction, so nothing that reads one back
/// from disk can smuggle a traversal in.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DefinitionSubject {
    agent: String,
    file: String,
}

impl DefinitionSubject {
    /// The subject for `file` of `agent`, or `None` when `agent` is not
    /// `.` or a relative `/`-separated path of plain components, or
    /// `file` is not a plain file name.
    #[must_use]
    pub fn new(agent: &str, file: &str) -> Option<Self> {
        let agent_ok = agent == "." || agent.split('/').all(is_plain_component);
        (agent_ok && !file.contains('/') && is_plain_component(file)).then(|| Self {
            agent: agent.to_string(),
            file: file.to_string(),
        })
    }

    /// The agent directory, relative to the workspace.
    #[must_use]
    pub fn agent(&self) -> &str {
        &self.agent
    }

    /// The file name inside the agent directory.
    #[must_use]
    pub fn file(&self) -> &str {
        &self.file
    }
}

/// A path component that is a real name: not empty, not `.` or `..`, and
/// free of NUL and control characters everywhere, and of `\` and `:`
/// on Windows only -- the rule the definition plan and writer apply,
/// since on Unix those are ordinary name characters.
fn is_plain_component(component: &str) -> bool {
    !component.is_empty()
        && component != "."
        && component != ".."
        && !(cfg!(windows) && component.contains(['\\', ':']))
        && !component.chars().any(char::is_control)
}

/// The exact bytes a definition snapshot id is the SHA-256 of:
/// [`DEFINITION_SNAPSHOT_ID_DOMAIN`] ‖ the project identity's 32 bytes ‖
/// the agent's length (big-endian `u32`) and bytes ‖ the file's length
/// and bytes ‖ the file digest's 32 bytes ‖ the instant as nanoseconds
/// since the Unix epoch, big-endian `u128`. The subject is part of the
/// preimage because the identical wrapper bytes of two agents, taken in
/// one instant, must not share an id.
#[must_use]
pub fn definition_snapshot_id_preimage(
    project: &ProjectIdentity,
    subject: &DefinitionSubject,
    digest: &Sha256Digest,
    taken_at: SystemTime,
) -> Vec<u8> {
    let nanos: u128 = taken_at
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    let mut preimage = DEFINITION_SNAPSHOT_ID_DOMAIN.to_vec();
    preimage.extend_from_slice(&project.0.0);
    for part in [subject.agent(), subject.file()] {
        let length = u32::try_from(part.len()).unwrap_or(u32::MAX);
        preimage.extend_from_slice(&length.to_be_bytes());
        preimage.extend_from_slice(part.as_bytes());
    }
    preimage.extend_from_slice(&digest.0);
    preimage.extend_from_slice(&nanos.to_be_bytes());
    preimage
}

/// One definition snapshot's manifest: the state of one identity file of
/// one agent directory at one instant. Shaped like [`SnapshotManifest`],
/// with a [`DefinitionSubject`] where that one has a kind and a root file
/// name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionSnapshotManifest {
    /// [`SNAPSHOT_SCHEMA_VERSION`].
    pub schema: u32,
    /// The snapshot's id.
    pub id: SnapshotId,
    /// The file this is a snapshot of.
    pub subject: DefinitionSubject,
    /// Whether the file existed; a snapshot of an absent file carries no
    /// bytes.
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

impl DefinitionSnapshotManifest {
    /// Whether `other` records the same state of the same file: the same
    /// subject, existence and digest -- what dedup compares.
    #[must_use]
    pub fn same_state_as(&self, other: &Self) -> bool {
        self.subject == other.subject
            && self.existed == other.existed
            && self.sha256 == other.sha256
    }
}

/// A port over the device's definition-file snapshots, per project
/// identity, kept beside the [`SnapshotStore`] snapshots in the same
/// store and under the same rules: dedup against the most recent snapshot
/// of the same subject, pruning to [`MAX_UNPINNED_SNAPSHOTS`] unpinned
/// per subject, pinned ones always kept.
pub trait DefinitionSnapshotStore {
    /// Record `bytes` under `manifest` for `project` -- unless the most
    /// recent snapshot of the same subject records the same state, in
    /// which case nothing is written and that snapshot's id comes back --
    /// then prune that subject. Returns the id that stands for the state.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotStoreError`] if the store cannot be read, is
    /// corrupt, or cannot be written.
    fn record_definition(
        &mut self,
        project: &ProjectIdentity,
        manifest: DefinitionSnapshotManifest,
        bytes: &[u8],
    ) -> Result<SnapshotId, SnapshotStoreError>;

    /// The definition snapshots of `project`, newest first: those of
    /// `agent` when given, else every agent's.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotStoreError`] if the store cannot be read or any
    /// manifest is corrupt.
    fn list_definitions(
        &self,
        project: &ProjectIdentity,
        agent: Option<&str>,
    ) -> Result<Vec<DefinitionSnapshotManifest>, SnapshotStoreError>;

    /// The definition snapshot `id` of `project`: its manifest and bytes.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotStoreError::Unknown`] if no such snapshot exists,
    /// or [`SnapshotStoreError`] if it cannot be read or is corrupt.
    fn read_definition(
        &self,
        project: &ProjectIdentity,
        id: SnapshotId,
    ) -> Result<(DefinitionSnapshotManifest, Vec<u8>), SnapshotStoreError>;

    /// Pin or unpin the definition snapshot `id` of `project`.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotStoreError::Unknown`] if no such snapshot exists,
    /// or [`SnapshotStoreError`] if the manifest cannot be rewritten.
    fn pin_definition(
        &mut self,
        project: &ProjectIdentity,
        id: SnapshotId,
        pinned: bool,
    ) -> Result<(), SnapshotStoreError>;
}

/// Order `manifests` newest first, as [`sort_newest_first`] does.
pub fn sort_definitions_newest_first(manifests: &mut [DefinitionSnapshotManifest]) {
    manifests.sort_by(|a, b| b.taken_at.cmp(&a.taken_at).then_with(|| b.id.cmp(&a.id)));
}

/// The id an identical state dedups to: the most recent snapshot of
/// `manifest`'s subject, when it records the same state.
#[must_use]
pub fn definition_dedup_target(
    newest_first: &[DefinitionSnapshotManifest],
    manifest: &DefinitionSnapshotManifest,
) -> Option<SnapshotId> {
    newest_first
        .iter()
        .find(|existing| existing.subject == manifest.subject)
        .filter(|existing| existing.same_state_as(manifest))
        .map(|existing| existing.id)
}

/// The definition snapshots to prune from a newest-first list: every
/// unpinned snapshot of `subject` beyond the [`MAX_UNPINNED_SNAPSHOTS`]
/// newest ones. Pinned snapshots are never listed.
#[must_use]
pub fn definition_prune_targets(
    newest_first: &[DefinitionSnapshotManifest],
    subject: &DefinitionSubject,
) -> Vec<SnapshotId> {
    newest_first
        .iter()
        .filter(|manifest| &manifest.subject == subject && !manifest.pinned)
        .skip(MAX_UNPINNED_SNAPSHOTS)
        .map(|manifest| manifest.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        DEFINITION_SNAPSHOT_ID_DOMAIN, DefinitionSubject, MAX_UNPINNED_SNAPSHOTS,
        SNAPSHOT_ID_DOMAIN, SnapshotId, SnapshotManifest, dedup_target,
        definition_snapshot_id_preimage, prune_targets, snapshot_id_preimage, sort_newest_first,
    };
    use omnifrons_domain::executable::Sha256Digest;
    use omnifrons_domain::guidance::ManagedFileKind;
    use omnifrons_domain::publication::ProjectIdentity;
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

    fn subject(agent: &str, file: &str) -> DefinitionSubject {
        DefinitionSubject::new(agent, file).expect("a valid subject")
    }

    const PROJECT: ProjectIdentity = ProjectIdentity(Sha256Digest([1; 32]));

    fn instant() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::new(1, 2)
    }

    #[test]
    fn the_definition_preimage_has_the_documented_byte_layout() {
        let digest = Sha256Digest([2; 32]);
        let mut expected = DEFINITION_SNAPSHOT_ID_DOMAIN.to_vec();
        expected.extend_from_slice(&[1; 32]);
        expected.extend_from_slice(&3_u32.to_be_bytes());
        expected.extend_from_slice(b"a/b");
        expected.extend_from_slice(&4_u32.to_be_bytes());
        expected.extend_from_slice(b"C.md");
        expected.extend_from_slice(&[2; 32]);
        expected.extend_from_slice(&1_000_000_002_u128.to_be_bytes());
        assert_eq!(
            definition_snapshot_id_preimage(&PROJECT, &subject("a/b", "C.md"), &digest, instant()),
            expected
        );
    }

    #[test]
    fn the_definition_preimage_depends_on_agent_and_file_with_length_framing() {
        let digest = Sha256Digest([2; 32]);
        let preimage = |agent: &str, file: &str| {
            definition_snapshot_id_preimage(&PROJECT, &subject(agent, file), &digest, instant())
        };
        let base = preimage("a/b", "C.md");
        assert_ne!(base, preimage("a/c", "C.md"));
        assert_ne!(base, preimage("a/b", "D.md"));
        // The length prefixes keep a boundary shift from aliasing.
        assert_ne!(preimage("ab", "c.md"), preimage("a", "bc.md"));
    }

    #[test]
    fn a_definition_preimage_never_equals_a_guidance_one() {
        let digest = Sha256Digest([2; 32]);
        let guidance = snapshot_id_preimage(&PROJECT, &digest, instant());
        let definition = definition_snapshot_id_preimage(
            &PROJECT,
            &subject(".", "AGENTS.md"),
            &digest,
            instant(),
        );
        assert_ne!(guidance, definition);
        assert!(guidance.starts_with(SNAPSHOT_ID_DOMAIN));
        assert!(definition.starts_with(DEFINITION_SNAPSHOT_ID_DOMAIN));
        assert!(!definition.starts_with(SNAPSHOT_ID_DOMAIN));
        assert!(!guidance.starts_with(DEFINITION_SNAPSHOT_ID_DOMAIN));
    }
}
