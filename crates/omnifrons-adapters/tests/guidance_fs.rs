//! The filesystem guidance adapters (spike slice 5c, HAP-001 D18):
//! `FsProjectTextFile` (one no-follow open, a regular file at most 1 MiB
//! of valid UTF-8, an atomic replace through a sibling `.part` file, a
//! removal that never follows a link) and `FsSnapshotStore` (the work
//! area's `snapshots/<project hex>/<id>.json` and `<id>.bytes`, owner-only
//! on unix, a corrupt manifest failing the listing closed). The shared
//! port contracts run first; the adapter-specific cases follow.
//!
//! Platform notes: the symbolic-link fixtures are unix only, gated per
//! test (creating one on Windows needs a privilege the CI runner lacks);
//! modes are asserted on unix only (Windows ACLs are outside `std`, the
//! disclosed residual).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use omnifrons_adapters::{FsProjectTextFile, FsSnapshotStore, Sha2Hasher};
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::content_hasher::{ContentHasher as _, derive_project_identity};
use omnifrons_app::contract::approval_store::FixedClock;
use omnifrons_app::contract::guidance::{
    managed_file_contract, sample_manifest, snapshot_store_contract,
};
use omnifrons_app::guidance::{GuidanceError, GuidancePorts, restore};
use omnifrons_app::managed_file::{ProjectTextFile, ProjectTextFileError};
use omnifrons_app::snapshot_store::{
    SNAPSHOT_SCHEMA_VERSION, SnapshotId, SnapshotManifest, SnapshotStore, SnapshotStoreError,
};
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::guidance::{ManagedFileKind, ManagedFileName, ManagedTarget};
use omnifrons_domain::publication::ProjectIdentity;

/// A drop-guard temp directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-guidance-fs-test-{}-{label}-{n}",
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

fn agents() -> ManagedTarget {
    ManagedTarget::guidance(ManagedFileName::default_guidance())
}

fn project() -> ProjectIdentity {
    ProjectIdentity(Sha256Digest([7; 32]))
}

/// A work area in `base`, checked against `project`.
fn work_area(base: &TempDir, project: &TempDir) -> WorkAreaRoot {
    WorkAreaRoot::open(&base.path().join("wa"), &[&project.workspace()]).expect("work area")
}

// -- the shared contracts --

#[test]
fn fs_project_text_file_satisfies_the_managed_file_contract() {
    let project = TempDir::new("file-contract");
    managed_file_contract(&FsProjectTextFile::new(), &project.workspace());
}

#[test]
fn fs_snapshot_store_satisfies_the_snapshot_store_contract() {
    let base = TempDir::new("snapshot-contract");
    let project = TempDir::new("snapshot-contract-project");
    let work_area = work_area(&base, &project);
    snapshot_store_contract(|| FsSnapshotStore::open(&work_area));
}

// -- the file port --

/// A symbolic link at the managed name is refused by every operation and
/// never followed: the target keeps its content and its existence.
#[cfg(unix)] // a symbolic link fixture
#[test]
fn a_link_at_the_managed_name_is_refused_and_never_followed() {
    let project = TempDir::new("link");
    let elsewhere = TempDir::new("link-target");
    let target = elsewhere.path().join("real.md");
    std::fs::write(&target, b"# real\n").expect("target");
    std::os::unix::fs::symlink(&target, project.path().join("AGENTS.md")).expect("plant");
    let files = FsProjectTextFile::new();
    let workspace = project.workspace();
    assert_eq!(
        files.read(&workspace, &agents()),
        Err(ProjectTextFileError::NotAFile)
    );
    assert_eq!(
        files.replace(&workspace, &agents(), None, b"# replaced\n"),
        Err(ProjectTextFileError::NotAFile)
    );
    assert_eq!(
        files.remove(&workspace, &agents(), None),
        Err(ProjectTextFileError::NotAFile)
    );
    assert_eq!(std::fs::read(&target).expect("still there"), b"# real\n");
    assert!(
        project
            .path()
            .join("AGENTS.md")
            .symlink_metadata()
            .expect("the link stays")
            .file_type()
            .is_symlink()
    );
}

/// A directory at the managed name is not a file: refused on every
/// operation, and left alone.
#[test]
fn a_directory_at_the_managed_name_is_refused() {
    let project = TempDir::new("directory");
    std::fs::create_dir(project.path().join("AGENTS.md")).expect("a directory");
    let files = FsProjectTextFile::new();
    let workspace = project.workspace();
    assert_eq!(
        files.read(&workspace, &agents()),
        Err(ProjectTextFileError::NotAFile)
    );
    assert_eq!(
        files.replace(&workspace, &agents(), None, b"# no\n"),
        Err(ProjectTextFileError::NotAFile)
    );
    assert_eq!(
        files.remove(&workspace, &agents(), None),
        Err(ProjectTextFileError::NotAFile)
    );
    assert!(project.path().join("AGENTS.md").is_dir());
}

/// A replace writes a sibling `.<name>.<pid>-<seq>.part` file and renames
/// it over the target: nothing is left behind, the content is the new
/// one, and on unix the original's mode survives.
#[test]
fn replace_goes_through_a_sibling_part_file_and_keeps_the_originals_mode() {
    let project = TempDir::new("replace");
    let path = project.path().join("AGENTS.md");
    std::fs::write(&path, b"# original\n").expect("original");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).expect("mode");
    }
    let files = FsProjectTextFile::new();
    let workspace = project.workspace();
    files
        .replace(
            &workspace,
            &agents(),
            Some(b"# original\n"),
            b"# replaced\n",
        )
        .expect("replace");
    assert_eq!(std::fs::read(&path).expect("read"), b"# replaced\n");
    let names: Vec<String> = std::fs::read_dir(project.path())
        .expect("list")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, vec!["AGENTS.md".to_string()], "no .part left behind");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o640, "the original's mode is preserved");
    }
}

/// R1-002 (slice 5c risk review): the write is bound to the bytes it was
/// planned from. A file rewritten between the caller's read and the
/// replace is refused from the re-read the adapter does immediately before
/// the rename -- the external content stays, no `.part` file is left
/// behind, and the same binding governs the removal. A file that vanished,
/// or one that appeared where the caller found none, is a change too.
#[test]
fn a_file_rewritten_between_the_read_and_the_write_is_refused_and_left_alone() {
    let project = TempDir::new("stale-write");
    let files = FsProjectTextFile::new();
    let workspace = project.workspace();
    let path = project.path().join("AGENTS.md");
    std::fs::write(&path, b"# read this\n").expect("original");
    let read = files
        .read(&workspace, &agents())
        .expect("read")
        .expect("present");

    // An external editor writes between the read and the write.
    std::fs::write(&path, b"# rewritten elsewhere\n").expect("external rewrite");
    assert_eq!(
        files.replace(&workspace, &agents(), Some(&read), b"# ours\n"),
        Err(ProjectTextFileError::Changed)
    );
    assert_eq!(
        std::fs::read(&path).expect("read"),
        b"# rewritten elsewhere\n",
        "the external write stands"
    );
    assert_eq!(
        files.remove(&workspace, &agents(), Some(&read)),
        Err(ProjectTextFileError::Changed)
    );
    assert!(path.is_file(), "nothing removed");
    let names: Vec<String> = std::fs::read_dir(project.path())
        .expect("list")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        names,
        vec!["AGENTS.md".to_string()],
        "no .part left behind by the refusal"
    );

    // The file the caller read is gone.
    std::fs::remove_file(&path).expect("external removal");
    assert_eq!(
        files.replace(&workspace, &agents(), Some(&read), b"# ours\n"),
        Err(ProjectTextFileError::Changed)
    );
    assert!(!path.exists(), "still nothing there");

    // A file stands where the caller found none.
    std::fs::write(&path, b"# appeared\n").expect("external create");
    assert_eq!(
        files.replace(&workspace, &agents(), None, b"# ours\n"),
        Err(ProjectTextFileError::Changed)
    );
    assert_eq!(
        files.remove(&workspace, &agents(), None),
        Err(ProjectTextFileError::Changed)
    );
    assert_eq!(std::fs::read(&path).expect("read"), b"# appeared\n");

    // Bound to what is actually there, the write goes through.
    files
        .replace(&workspace, &agents(), Some(b"# appeared\n"), b"# ours\n")
        .expect("replace");
    assert_eq!(std::fs::read(&path).expect("read"), b"# ours\n");
    files
        .remove(&workspace, &agents(), Some(b"# ours\n"))
        .expect("remove");
    assert!(!path.exists());
}

// -- the snapshot store --

/// A snapshot lives at `<work area>/snapshots/<project hex>/<id>.json`
/// and `<id>.bytes`, owner-only on unix, the manifest in its documented
/// camelCase shape; an absent file's snapshot has an empty bytes file.
#[test]
fn snapshots_are_owner_only_files_under_the_projects_directory_in_the_documented_shape() {
    let base = TempDir::new("layout");
    let project_dir = TempDir::new("layout-project");
    let work_area = work_area(&base, &project_dir);
    let mut store = FsSnapshotStore::open(&work_area);
    let manifest = sample_manifest(0x0123_4567_89ab_cdef, 1_725_782_401, true, 0xab);
    let id = store
        .record(&project(), manifest.clone(), b"# one\n")
        .expect("record");
    let dir = work_area.snapshots_dir().join(project().to_hex());
    let json_path = dir.join(format!("{}.json", id.to_hex()));
    let bytes_path = dir.join(format!("{}.bytes", id.to_hex()));
    assert_eq!(id.to_hex(), "0123456789abcdef");
    assert_eq!(std::fs::read(&bytes_path).expect("bytes"), b"# one\n");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json_path).expect("manifest"))
            .expect("json");
    assert_eq!(
        json,
        serde_json::json!({
            "schema": 1,
            "id": "0123456789abcdef",
            "kind": "guidance",
            "file": "AGENTS.md",
            "existed": true,
            "sha256": "ab".repeat(32),
            "size": 3,
            "takenAt": {"secs": 1_725_782_401, "nanos": 0},
            "pinned": false,
        })
    );
    let dumped = std::fs::read_to_string(&json_path).expect("manifest");
    assert!(
        !dumped.contains(&project_dir.path().to_string_lossy().to_string())
            && !dumped.contains(&base.path().to_string_lossy().to_string()),
        "a manifest carries no device path"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for (path, expected) in [(&dir, 0o700), (&json_path, 0o600), (&bytes_path, 0o600)] {
            let mode = std::fs::metadata(path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, expected, "{}", path.display());
        }
    }
    let (read_back, bytes) = store.read(&project(), id).expect("read");
    assert_eq!(read_back, manifest);
    assert_eq!(bytes, b"# one\n");

    let absent = sample_manifest(2, 1_725_782_402, false, 0x00);
    let absent_id = store.record(&project(), absent, b"").expect("record");
    let (read_back, bytes) = store.read(&project(), absent_id).expect("read");
    assert!(!read_back.existed);
    assert!(bytes.is_empty());
    assert_eq!(
        std::fs::read(dir.join(format!("{}.bytes", absent_id.to_hex()))).expect("bytes"),
        b""
    );
}

/// A manifest that does not parse fails the listing closed as `Corrupt`
/// and the read of that id -- never a listing with the bad one skipped;
/// another id still reads.
#[test]
fn a_corrupt_manifest_fails_the_listing_closed_and_the_read_of_that_id() {
    let base = TempDir::new("corrupt");
    let project_dir = TempDir::new("corrupt-project");
    let work_area = work_area(&base, &project_dir);
    let mut store = FsSnapshotStore::open(&work_area);
    let good = store
        .record(
            &project(),
            sample_manifest(1, 1_725_782_401, true, 0x11),
            b"one",
        )
        .expect("record");
    let bad = store
        .record(
            &project(),
            sample_manifest(2, 1_725_782_402, true, 0x22),
            b"two",
        )
        .expect("record");
    let dir = work_area.snapshots_dir().join(project().to_hex());
    let bad_path = dir.join(format!("{}.json", bad.to_hex()));
    let text = std::fs::read_to_string(&bad_path).expect("manifest");

    std::fs::write(&bad_path, text.replace("\"schema\":1", "\"schema\":2")).expect("reschema");
    assert_eq!(
        store.list(&project(), ManagedFileKind::Guidance),
        Err(SnapshotStoreError::Corrupt)
    );
    assert_eq!(
        store.read(&project(), bad),
        Err(SnapshotStoreError::Corrupt)
    );
    assert!(
        store.read(&project(), good).is_ok(),
        "the other id still reads"
    );

    std::fs::write(&bad_path, "{ nope").expect("garbage");
    assert_eq!(
        store.list(&project(), ManagedFileKind::Guidance),
        Err(SnapshotStoreError::Corrupt)
    );
    assert_eq!(
        store.record(
            &project(),
            sample_manifest(3, 1_725_782_403, true, 0x33),
            b"three"
        ),
        Err(SnapshotStoreError::Corrupt),
        "a record needs the listing for dedup and prune: fails closed too"
    );

    // A manifest whose file name disagrees with its id is corrupt too.
    std::fs::write(
        &bad_path,
        text.replace(&bad.to_hex(), &SnapshotId(9).to_hex()),
    )
    .expect("id mismatch");
    assert_eq!(
        store.list(&project(), ManagedFileKind::Guidance),
        Err(SnapshotStoreError::Corrupt)
    );
    assert_eq!(
        store.read(&project(), SnapshotId(99)),
        Err(SnapshotStoreError::Unknown)
    );
}

/// R3-001 (slice 5c reliability review): the branch the module documents
/// for "a manifest naming bytes that are not there". The manifest is
/// intact and the `.bytes` sibling is gone -- a half-deleted snapshot, or
/// a crash between the two writes in the other order -- and the read of
/// that id fails closed as `Corrupt`; the listing, which reads manifests
/// only, still lists it, and another id still reads.
#[test]
fn a_manifest_whose_bytes_file_is_missing_is_corrupt_at_read() {
    let base = TempDir::new("no-bytes");
    let project_dir = TempDir::new("no-bytes-project");
    let work_area = work_area(&base, &project_dir);
    let mut store = FsSnapshotStore::open(&work_area);
    let good = store
        .record(
            &project(),
            sample_manifest(1, 1_725_782_401, true, 0x11),
            b"one",
        )
        .expect("record");
    let orphan = store
        .record(
            &project(),
            sample_manifest(2, 1_725_782_402, true, 0x22),
            b"two",
        )
        .expect("record");
    let dir = work_area.snapshots_dir().join(project().to_hex());
    std::fs::remove_file(dir.join(format!("{}.bytes", orphan.to_hex()))).expect("drop the bytes");
    assert_eq!(
        store.read(&project(), orphan),
        Err(SnapshotStoreError::Corrupt)
    );
    assert_eq!(
        store
            .list(&project(), ManagedFileKind::Guidance)
            .expect("the listing reads manifests only")
            .len(),
        2
    );
    assert!(
        store.read(&project(), good).is_ok(),
        "the other id still reads"
    );
}

/// R3-001 and R3-008 (slice 5c reliability review): a bytes file whose
/// content no longer digests to what its manifest records. The store hands
/// the bytes back -- checking them against the manifest is the
/// transaction's job, and the store has no hasher -- and the restore
/// refuses with the snapshot `Corrupt` error before it reads, snapshots,
/// or writes anything: the current file is left exactly as it was.
#[test]
fn a_snapshot_whose_bytes_disagree_with_its_manifest_is_refused_at_restore() {
    let base = TempDir::new("bytes-mismatch");
    let project_dir = TempDir::new("bytes-mismatch-project");
    let work_area = work_area(&base, &project_dir);
    let workspace = project_dir.workspace();
    let hasher = Sha2Hasher::new();
    let identity = derive_project_identity(&hasher, &workspace);
    let recorded: &[u8] = b"# the note as it was\n";
    let mut store = FsSnapshotStore::open(&work_area);
    let id = store
        .record(
            &identity,
            SnapshotManifest {
                schema: SNAPSHOT_SCHEMA_VERSION,
                id: SnapshotId(1),
                kind: ManagedFileKind::Guidance,
                file: "AGENTS.md".to_string(),
                existed: true,
                sha256: hasher.sha256(recorded),
                size: recorded.len() as u64,
                taken_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_401),
                pinned: false,
            },
            recorded,
        )
        .expect("record");

    let dir = work_area.snapshots_dir().join(identity.to_hex());
    std::fs::write(
        dir.join(format!("{}.bytes", id.to_hex())),
        b"# tampered with\n",
    )
    .expect("corrupt the bytes");
    let (read_back, bytes) = store
        .read(&identity, id)
        .expect("the store hands them back");
    assert_ne!(
        hasher.sha256(&bytes),
        read_back.sha256,
        "the bytes no longer digest to the manifest's record"
    );

    let current: &[u8] = b"# what the project carries now\n";
    let path = project_dir.path().join("AGENTS.md");
    std::fs::write(&path, current).expect("current");
    let files = FsProjectTextFile::new();
    let clock = FixedClock::new(SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_500));
    let mut snapshots = FsSnapshotStore::open(&work_area);
    let workspaces = [&workspace];
    let mut ports = GuidancePorts {
        files: &files,
        snapshots: &mut snapshots,
        hasher: &hasher,
        clock: &clock,
        work_area: &work_area,
        workspaces: &workspaces,
    };
    assert_eq!(
        restore(&mut ports, &workspace, id, Some(hasher.sha256(current)))
            .map(|restored| restored.restored),
        Err(GuidanceError::Snapshot(SnapshotStoreError::Corrupt))
    );
    assert_eq!(
        std::fs::read(&path).expect("read"),
        current,
        "the current file is untouched"
    );
    assert_eq!(
        store
            .list(&identity, ManagedFileKind::Guidance)
            .expect("list")
            .len(),
        1,
        "a refused restore snapshots nothing"
    );
}

/// Pruned snapshots are deleted from disk, manifest and bytes alike; a
/// pinned one keeps both files.
#[test]
fn pruned_snapshots_are_deleted_from_disk_and_a_pinned_one_stays() {
    let base = TempDir::new("prune");
    let project_dir = TempDir::new("prune-project");
    let work_area = work_area(&base, &project_dir);
    let mut store = FsSnapshotStore::open(&work_area);
    let first = store
        .record(&project(), sample_manifest(1, 1_725_782_401, true, 1), b"1")
        .expect("record");
    store.pin(&project(), first, true).expect("pin");
    for n in 2..=8 {
        let byte = u8::try_from(n).expect("small");
        store
            .record(
                &project(),
                sample_manifest(n, 1_725_782_401 + n, true, byte),
                &[byte],
            )
            .expect("record");
    }
    let dir = work_area.snapshots_dir().join(project().to_hex());
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("list")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    let expected: Vec<String> = [1u64, 4, 5, 6, 7, 8]
        .iter()
        .flat_map(|n| {
            let id = SnapshotId(*n).to_hex();
            [format!("{id}.bytes"), format!("{id}.json")]
        })
        .collect();
    assert_eq!(names, expected, "the pinned first and the five newest");
    let (manifest, _) = store.read(&project(), first).expect("pinned still reads");
    assert!(manifest.pinned);
    assert_eq!(
        store.read(&project(), SnapshotId(2)),
        Err(SnapshotStoreError::Unknown)
    );
    assert_eq!(
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_401),
        manifest.taken_at
    );
}
