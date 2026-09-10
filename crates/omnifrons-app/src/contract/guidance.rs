//! Fakes and reusable contracts for the guidance installer's ports (spike
//! slice 5c): an `InMemoryProjectTextFile` that applies the read bound
//! and the UTF-8 rule to what it holds, an `InMemorySnapshotStore` that
//! applies the shared dedup and prune rules, and the contracts
//! `omnifrons-adapters`' real file port and snapshot store run against
//! themselves.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::guidance::{ManagedFileKind, ManagedFileName, ManagedTarget};
use omnifrons_domain::publication::ProjectIdentity;

use crate::harness_adapter::WorkspaceRoot;
use crate::managed_file::{MAX_MANAGED_FILE_BYTES, ProjectTextFile, ProjectTextFileError};
use crate::snapshot_store::{
    MAX_UNPINNED_SNAPSHOTS, SNAPSHOT_SCHEMA_VERSION, SnapshotId, SnapshotManifest, SnapshotStore,
    SnapshotStoreError, dedup_target, prune_targets, sort_newest_first,
};

/// An in-memory [`ProjectTextFile`] test double: files live in a map by
/// name, the read bound and the UTF-8 rule apply to what is held, and a
/// knob fails the next write.
#[derive(Debug, Default)]
pub struct InMemoryProjectTextFile {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    fail_next_write: AtomicBool,
}

impl InMemoryProjectTextFile {
    /// An empty workspace.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fail the next `replace` or `remove` with `WriteFailed`, changing
    /// nothing.
    pub fn fail_next_write(&self) {
        self.fail_next_write.store(true, Ordering::Release);
    }

    /// Put `bytes` at `name` directly, as an external editor would.
    ///
    /// # Panics
    ///
    /// Panics if the double's mutex was poisoned by a prior panic.
    pub fn plant(&self, name: &str, bytes: &[u8]) {
        self.files
            .lock()
            .expect("files mutex poisoned")
            .insert(name.to_string(), bytes.to_vec());
    }

    /// The bytes at `name`, if any.
    ///
    /// # Panics
    ///
    /// Panics if the double's mutex was poisoned by a prior panic.
    #[must_use]
    pub fn bytes(&self, name: &str) -> Option<Vec<u8>> {
        self.files
            .lock()
            .expect("files mutex poisoned")
            .get(name)
            .cloned()
    }

    fn take_write_failure(&self) -> Result<(), ProjectTextFileError> {
        if self.fail_next_write.swap(false, Ordering::AcqRel) {
            return Err(ProjectTextFileError::WriteFailed);
        }
        Ok(())
    }
}

impl ProjectTextFile for InMemoryProjectTextFile {
    fn read(
        &self,
        _root: &WorkspaceRoot,
        target: &ManagedTarget,
    ) -> Result<Option<Vec<u8>>, ProjectTextFileError> {
        let files = self.files.lock().expect("files mutex poisoned");
        let Some(bytes) = files.get(target.file_name()) else {
            return Ok(None);
        };
        if bytes.len() as u64 > MAX_MANAGED_FILE_BYTES {
            return Err(ProjectTextFileError::TooLarge);
        }
        if std::str::from_utf8(bytes).is_err() {
            return Err(ProjectTextFileError::NotUtf8);
        }
        Ok(Some(bytes.clone()))
    }

    fn replace(
        &self,
        _root: &WorkspaceRoot,
        target: &ManagedTarget,
        expected: Option<&[u8]>,
        bytes: &[u8],
    ) -> Result<(), ProjectTextFileError> {
        self.take_write_failure()?;
        let mut files = self.files.lock().expect("files mutex poisoned");
        if files.get(target.file_name()).map(Vec::as_slice) != expected {
            return Err(ProjectTextFileError::Changed);
        }
        files.insert(target.file_name().to_string(), bytes.to_vec());
        Ok(())
    }

    fn remove(
        &self,
        _root: &WorkspaceRoot,
        target: &ManagedTarget,
        expected: Option<&[u8]>,
    ) -> Result<(), ProjectTextFileError> {
        self.take_write_failure()?;
        let mut files = self.files.lock().expect("files mutex poisoned");
        if files.get(target.file_name()).map(Vec::as_slice) != expected {
            return Err(ProjectTextFileError::Changed);
        }
        files.remove(target.file_name());
        Ok(())
    }
}

/// An in-memory [`SnapshotStore`] test double applying the shared dedup
/// and prune rules, with a knob that fails the next `record`.
#[derive(Debug, Default)]
pub struct InMemorySnapshotStore {
    snapshots: HashMap<ProjectIdentity, Vec<(SnapshotManifest, Vec<u8>)>>,
    fail_next_record: bool,
}

impl InMemorySnapshotStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fail the next `record` with `WriteFailed`, recording nothing.
    pub const fn fail_next_record(&mut self) {
        self.fail_next_record = true;
    }

    /// Every manifest held for `project`, newest first, whichever kind.
    #[must_use]
    pub fn manifests(&self, project: &ProjectIdentity) -> Vec<SnapshotManifest> {
        let mut manifests: Vec<SnapshotManifest> = self
            .snapshots
            .get(project)
            .map(|entries| entries.iter().map(|(m, _)| m.clone()).collect())
            .unwrap_or_default();
        sort_newest_first(&mut manifests);
        manifests
    }
}

impl SnapshotStore for InMemorySnapshotStore {
    fn record(
        &mut self,
        project: &ProjectIdentity,
        manifest: SnapshotManifest,
        bytes: &[u8],
    ) -> Result<SnapshotId, SnapshotStoreError> {
        if self.fail_next_record {
            self.fail_next_record = false;
            return Err(SnapshotStoreError::WriteFailed);
        }
        let newest_first = self.manifests(project);
        if let Some(id) = dedup_target(&newest_first, &manifest) {
            return Ok(id);
        }
        let id = manifest.id;
        let (kind, file) = (manifest.kind, manifest.file.clone());
        let entries = self.snapshots.entry(*project).or_default();
        entries.push((manifest, bytes.to_vec()));
        let pruned = prune_targets(&self.manifests(project), kind, &file);
        if let Some(entries) = self.snapshots.get_mut(project) {
            entries.retain(|(manifest, _)| !pruned.contains(&manifest.id));
        }
        Ok(id)
    }

    fn list(
        &self,
        project: &ProjectIdentity,
        kind: ManagedFileKind,
    ) -> Result<Vec<SnapshotManifest>, SnapshotStoreError> {
        Ok(self
            .manifests(project)
            .into_iter()
            .filter(|manifest| manifest.kind == kind)
            .collect())
    }

    fn read(
        &self,
        project: &ProjectIdentity,
        id: SnapshotId,
    ) -> Result<(SnapshotManifest, Vec<u8>), SnapshotStoreError> {
        self.snapshots
            .get(project)
            .and_then(|entries| entries.iter().find(|(manifest, _)| manifest.id == id))
            .cloned()
            .ok_or(SnapshotStoreError::Unknown)
    }

    fn pin(
        &mut self,
        project: &ProjectIdentity,
        id: SnapshotId,
        pinned: bool,
    ) -> Result<(), SnapshotStoreError> {
        let entry = self
            .snapshots
            .get_mut(project)
            .and_then(|entries| entries.iter_mut().find(|(manifest, _)| manifest.id == id))
            .ok_or(SnapshotStoreError::Unknown)?;
        entry.0.pinned = pinned;
        Ok(())
    }
}

// -- fixtures --

/// A manifest for the guidance file `AGENTS.md`: id `n`, taken at `secs`
/// past the epoch, digest `[byte; 32]`.
#[must_use]
pub fn sample_manifest(n: u64, secs: u64, existed: bool, byte: u8) -> SnapshotManifest {
    SnapshotManifest {
        schema: SNAPSHOT_SCHEMA_VERSION,
        id: SnapshotId(n),
        kind: ManagedFileKind::Guidance,
        file: "AGENTS.md".to_string(),
        existed,
        sha256: Sha256Digest([byte; 32]),
        size: 3,
        taken_at: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
        pinned: false,
    }
}

// -- contracts --

/// The [`SnapshotStore`] contract: an empty listing; a read round trip; an
/// identical state deduplicated against the most recent snapshot of the
/// same file only; newest-first order; another kind and another project
/// kept apart; pruning to the five newest unpinned per file with a pinned
/// one always kept; unknown ids refused.
///
/// # Panics
///
/// Panics on any contract violation.
#[allow(clippy::too_many_lines)]
pub fn snapshot_store_contract<S: SnapshotStore>(make: impl Fn() -> S) {
    let mut store = make();
    let project = ProjectIdentity(Sha256Digest([7; 32]));
    let other = ProjectIdentity(Sha256Digest([8; 32]));
    let base = 1_725_782_400;
    assert!(
        store
            .list(&project, ManagedFileKind::Guidance)
            .expect("an empty store lists")
            .is_empty()
    );

    // A read round trip.
    let first = store
        .record(&project, sample_manifest(1, base, true, 0x11), b"one")
        .expect("record");
    assert_eq!(first, SnapshotId(1));
    let (manifest, bytes) = store.read(&project, first).expect("read");
    assert_eq!(bytes, b"one");
    assert_eq!(manifest, sample_manifest(1, base, true, 0x11));

    // The same state again is deduplicated: nothing new, the same id.
    let again = store
        .record(&project, sample_manifest(2, base + 1, true, 0x11), b"one")
        .expect("record");
    assert_eq!(again, first, "dedup returns the most recent snapshot's id");
    assert_eq!(
        store
            .list(&project, ManagedFileKind::Guidance)
            .expect("list")
            .len(),
        1
    );

    // A different state is recorded; the listing is newest first.
    let third = store
        .record(&project, sample_manifest(3, base + 2, true, 0x33), b"three")
        .expect("record");
    assert_eq!(third, SnapshotId(3));
    let ids: Vec<SnapshotId> = store
        .list(&project, ManagedFileKind::Guidance)
        .expect("list")
        .iter()
        .map(|manifest| manifest.id)
        .collect();
    assert_eq!(ids, vec![SnapshotId(3), SnapshotId(1)]);

    // Dedup compares against the most recent only: the old state again is
    // a new snapshot.
    let fourth = store
        .record(&project, sample_manifest(4, base + 3, true, 0x11), b"one")
        .expect("record");
    assert_eq!(fourth, SnapshotId(4));

    // Another kind is kept apart; an absent file's snapshot carries no bytes.
    let ignore = SnapshotManifest {
        kind: ManagedFileKind::Ignore,
        file: ".gitignore".to_string(),
        size: 0,
        ..sample_manifest(5, base + 4, false, 0x00)
    };
    assert_eq!(
        store.record(&project, ignore.clone(), b"").expect("record"),
        SnapshotId(5)
    );
    assert_eq!(
        store
            .list(&project, ManagedFileKind::Guidance)
            .expect("list")
            .len(),
        3
    );
    let listed = store.list(&project, ManagedFileKind::Ignore).expect("list");
    assert_eq!(listed, vec![ignore]);
    let (manifest, bytes) = store.read(&project, SnapshotId(5)).expect("read");
    assert!(!manifest.existed);
    assert!(bytes.is_empty());

    // Pruning: pin the oldest, record six more distinct states; the five
    // newest unpinned stay, the pinned one stays, the rest go.
    store.pin(&project, first, true).expect("pin");
    for n in 6..=11 {
        let byte = u8::try_from(n).expect("small");
        store
            .record(&project, sample_manifest(n, base + n, true, byte), &[byte])
            .expect("record");
    }
    let listed = store
        .list(&project, ManagedFileKind::Guidance)
        .expect("list");
    let ids: Vec<SnapshotId> = listed.iter().map(|manifest| manifest.id).collect();
    assert_eq!(
        ids,
        [11, 10, 9, 8, 7, 1].map(SnapshotId).to_vec(),
        "five newest unpinned plus the pinned one, newest first"
    );
    assert_eq!(MAX_UNPINNED_SNAPSHOTS, 5);
    assert!(listed[5].pinned);
    assert_eq!(
        store.read(&project, third),
        Err(SnapshotStoreError::Unknown)
    );
    assert!(store.read(&project, first).is_ok(), "pinned: never pruned");

    // Unpinned again, the next record prunes it with the rest.
    store.pin(&project, first, false).expect("unpin");
    store
        .record(&project, sample_manifest(12, base + 12, true, 12), b"12")
        .expect("record");
    let ids: Vec<SnapshotId> = store
        .list(&project, ManagedFileKind::Guidance)
        .expect("list")
        .iter()
        .map(|manifest| manifest.id)
        .collect();
    assert_eq!(ids, [12, 11, 10, 9, 8].map(SnapshotId).to_vec());
    assert_eq!(
        store.read(&project, first),
        Err(SnapshotStoreError::Unknown)
    );

    // Another project sees nothing of this one.
    assert!(
        store
            .list(&other, ManagedFileKind::Guidance)
            .expect("list")
            .is_empty()
    );
    assert_eq!(
        store.read(&other, SnapshotId(12)),
        Err(SnapshotStoreError::Unknown)
    );
    assert_eq!(
        store.pin(&project, SnapshotId(99), true),
        Err(SnapshotStoreError::Unknown)
    );
}

/// The [`ProjectTextFile`] contract over `root`: an absent file reads as
/// `None`; a replaced file reads back byte for byte, CRLF included; a
/// write binds to the bytes the caller read and refuses a target that is
/// no longer them (R1-002); a file that is not valid UTF-8 or exceeds the
/// bound is refused on read while one exactly at the bound reads; a
/// removed file is absent and removing it again is nothing; the two kinds'
/// files are independent.
///
/// # Panics
///
/// Panics on any contract violation.
pub fn managed_file_contract<F: ProjectTextFile>(files: &F, root: &WorkspaceRoot) {
    let target = ManagedTarget::guidance(ManagedFileName::guidance("CONTRACT.md").expect("valid"));
    assert_eq!(files.read(root, &target), Ok(None));
    files
        .replace(root, &target, None, b"# one\n")
        .expect("replace creates");
    assert_eq!(files.read(root, &target), Ok(Some(b"# one\n".to_vec())));
    files
        .replace(root, &target, Some(b"# one\n"), b"# two\r\n")
        .expect("replace overwrites");
    assert_eq!(files.read(root, &target), Ok(Some(b"# two\r\n".to_vec())));

    // R1-002: the write binds to the bytes the caller read. A target that
    // carries other bytes, or exists where the caller found none, is
    // `Changed`, and nothing is written or removed.
    assert_eq!(
        files.replace(root, &target, Some(b"# one\n"), b"# three\n"),
        Err(ProjectTextFileError::Changed),
        "stale expected bytes"
    );
    assert_eq!(
        files.replace(root, &target, None, b"# three\n"),
        Err(ProjectTextFileError::Changed),
        "a file stands where the caller found none"
    );
    assert_eq!(
        files.remove(root, &target, Some(b"# one\n")),
        Err(ProjectTextFileError::Changed),
        "a removal binds the same way"
    );
    assert_eq!(
        files.read(root, &target),
        Ok(Some(b"# two\r\n".to_vec())),
        "a refused write changes nothing"
    );

    files
        .replace(root, &target, Some(b"# two\r\n"), &[0xff, 0xfe, b'x'])
        .expect("bytes are bytes");
    assert_eq!(
        files.read(root, &target),
        Err(ProjectTextFileError::NotUtf8)
    );
    let at_bound = vec![b'a'; usize::try_from(MAX_MANAGED_FILE_BYTES).expect("fits")];
    files
        .replace(root, &target, Some(&[0xff, 0xfe, b'x']), &at_bound)
        .expect("replace");
    assert_eq!(files.read(root, &target), Ok(Some(at_bound.clone())));
    let mut over = at_bound.clone();
    over.push(b'a');
    files
        .replace(root, &target, Some(&at_bound), &over)
        .expect("replace");
    assert_eq!(
        files.read(root, &target),
        Err(ProjectTextFileError::TooLarge)
    );

    files.remove(root, &target, Some(&over)).expect("remove");
    assert_eq!(files.read(root, &target), Ok(None));
    files
        .remove(root, &target, None)
        .expect("removing an absent file is nothing");
    assert_eq!(
        files.remove(root, &target, Some(b"# one\n")),
        Err(ProjectTextFileError::Changed),
        "the file the caller read is gone"
    );

    let ignore = ManagedTarget::ignore();
    assert_eq!(files.read(root, &ignore), Ok(None));
    files
        .replace(root, &ignore, None, b"/target\n")
        .expect("replace");
    assert_eq!(files.read(root, &ignore), Ok(Some(b"/target\n".to_vec())));
    assert_eq!(files.read(root, &target), Ok(None), "kinds are independent");
    files
        .remove(root, &ignore, Some(b"/target\n"))
        .expect("remove");
}
