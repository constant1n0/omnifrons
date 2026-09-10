//! The guidance-note installer (spike slice 5c, HAP-001 D18 at its
//! default with the installer discipline) against the in-memory fakes:
//! status, preview, apply in its fixed order (work area, read, the digest
//! binding, the block refusals, the plan, no snapshot for a no-op, the
//! snapshot deduplicated, the write, the read-back), remove deleting only
//! the lines Omnifrons owns, restore snapshotting first, and the fixed
//! reason tokens.
//!
//! Only compiled with `--features contract-tests`: the fakes live behind
//! that feature.

#![cfg(feature = "contract-tests")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use omnifrons_app::content_hasher::ContentHasher;
use omnifrons_app::contract::approval_store::FixedClock;
use omnifrons_app::contract::guidance::{
    InMemoryProjectTextFile, InMemorySnapshotStore, sample_manifest,
};
use omnifrons_app::contract::publication::FakeHasher;
use omnifrons_app::guidance::{
    GuidanceError, GuidancePorts, apply, pin, preview, remove, restore, snapshots, status,
};
use omnifrons_app::managed_file::ProjectTextFileError;
use omnifrons_app::snapshot_store::SnapshotStoreError;
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_app::{Clock, WorkspaceRoot};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::guidance::{
    ApplyAction, LineEnding, ManagedBlock, ManagedFileKind, ManagedFileName, ManagedStatus,
    ManagedTarget,
};
use omnifrons_domain::outbox::OutboxPath;

/// A drop-guard temp directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-guidance-test-{}-{label}-{n}",
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

/// The fixture: a project, a work area beside it, the in-memory file port
/// and snapshot store, the fake hasher, and a fixed clock the tests step.
struct Fixture {
    project: TempDir,
    work_base: TempDir,
    work_area: WorkAreaRoot,
    files: InMemoryProjectTextFile,
    snapshots: InMemorySnapshotStore,
    hasher: FakeHasher,
    clock: FixedClock,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let project = TempDir::new(label);
        let work_base = TempDir::new(&format!("{label}-work"));
        let work_area = WorkAreaRoot::open(&work_base.path().join("wa"), &[&project.workspace()])
            .expect("the work area opens outside the project");
        Self {
            project,
            work_base,
            work_area,
            files: InMemoryProjectTextFile::new(),
            snapshots: InMemorySnapshotStore::new(),
            hasher: FakeHasher,
            clock: FixedClock::new(SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_401)),
        }
    }

    fn workspace(&self) -> WorkspaceRoot {
        self.project.workspace()
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

    /// Step the clock by a whole second (Windows keeps `SystemTime` in
    /// 100 ns steps, so a nanosecond would not tell two instants apart).
    fn tick(&mut self) {
        self.clock = FixedClock::new(self.clock.now() + Duration::from_secs(1));
    }

    fn digest_of(&self, bytes: &[u8]) -> Sha256Digest {
        self.hasher.sha256(bytes)
    }

    fn file_digest(&self, name: &str) -> Option<Sha256Digest> {
        self.files
            .bytes(name)
            .map(|bytes| self.hasher.sha256(&bytes))
    }

    fn text(&self, name: &str) -> Option<String> {
        self.files
            .bytes(name)
            .map(|bytes| String::from_utf8(bytes).expect("utf-8"))
    }

    fn block(&self, kind: ManagedFileKind) -> ManagedBlock {
        ManagedBlock::for_kind(kind, &outbox(), &|bytes| self.hasher.sha256(bytes))
    }
}

fn outbox() -> OutboxPath {
    OutboxPath::default_path()
}

fn agents() -> ManagedTarget {
    ManagedTarget::guidance(ManagedFileName::default_guidance())
}

// -- status and preview --

#[test]
fn status_of_an_absent_file_is_absent_with_no_digest_and_no_snapshots() {
    let mut fixture = Fixture::new("status-absent");
    let workspace = fixture.workspace();
    let status = status(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
    )
    .expect("status");
    assert_eq!(status.file, "AGENTS.md");
    assert!(!status.exists);
    assert_eq!(status.managed, ManagedStatus::Absent);
    assert_eq!(status.file_sha256, None);
    assert_eq!(status.snapshots, 0);
    assert_eq!(status.pinned, 0);
}

#[test]
fn preview_shows_the_block_and_the_result_digest_without_writing() {
    let mut fixture = Fixture::new("preview");
    let workspace = fixture.workspace();
    fixture.files.plant("AGENTS.md", b"# Title\r\n");
    let block = fixture.block(ManagedFileKind::Guidance);
    let preview = preview(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
    )
    .expect("preview");
    assert_eq!(preview.file, "AGENTS.md");
    assert_eq!(preview.action, ApplyAction::Insert);
    assert_eq!(
        preview.proposed_block,
        block.render(LineEnding::CrLf),
        "the block as it would be written, in the file's own ending"
    );
    assert_eq!(preview.file_sha256, fixture.file_digest("AGENTS.md"));
    let expected = format!("# Title\r\n\r\n{}\r\n", block.render(LineEnding::CrLf));
    assert_eq!(
        preview.result_sha256,
        fixture.digest_of(expected.as_bytes())
    );
    assert_eq!(
        fixture.text("AGENTS.md").as_deref(),
        Some("# Title\r\n"),
        "nothing written"
    );
    assert!(
        fixture
            .snapshots
            .manifests(&project_of(&fixture))
            .is_empty(),
        "nothing snapshotted"
    );
}

fn project_of(fixture: &Fixture) -> omnifrons_domain::publication::ProjectIdentity {
    omnifrons_app::content_hasher::derive_project_identity(&fixture.hasher, &fixture.workspace())
}

// -- apply --

#[test]
fn apply_inserts_the_note_into_an_absent_file_after_snapshotting_its_absence() {
    let mut fixture = Fixture::new("apply-absent");
    let workspace = fixture.workspace();
    let block = fixture.block(ManagedFileKind::Guidance);
    let applied = apply(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
        None,
    )
    .expect("applied");
    assert_eq!(applied.action, ApplyAction::Insert);
    let expected = format!("{}\n", block.render(LineEnding::Lf));
    assert_eq!(
        fixture.text("AGENTS.md").as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(
        applied.result_sha256,
        fixture.digest_of(expected.as_bytes())
    );
    let snapshot_id = applied.snapshot_id.expect("a snapshot of the absence");
    let project = project_of(&fixture);
    let manifests = fixture.snapshots.manifests(&project);
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].id, snapshot_id);
    assert!(!manifests[0].existed, "the file did not exist");
    assert_eq!(manifests[0].size, 0);
    assert_eq!(manifests[0].file, "AGENTS.md");
    assert_eq!(manifests[0].kind, ManagedFileKind::Guidance);
    assert!(!manifests[0].pinned);

    let status = status(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
    )
    .expect("status");
    assert!(status.exists);
    assert_eq!(status.managed, ManagedStatus::Current);
    assert_eq!(status.file_sha256, Some(applied.result_sha256));
    assert_eq!(status.snapshots, 1);
}

#[test]
fn apply_binds_to_the_digest_the_surface_showed() {
    let mut fixture = Fixture::new("apply-binding");
    let workspace = fixture.workspace();
    fixture.files.plant("AGENTS.md", b"# Title\n");
    let shown = fixture.file_digest("AGENTS.md");
    assert_eq!(
        apply(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            &agents(),
            &outbox(),
            None
        )
        .map(|applied| applied.action),
        Err(GuidanceError::FileChanged),
        "the surface showed no file; there is one now"
    );
    assert_eq!(
        apply(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            &agents(),
            &outbox(),
            Some(Sha256Digest([9; 32])),
        )
        .map(|applied| applied.action),
        Err(GuidanceError::FileChanged)
    );
    assert!(
        fixture
            .snapshots
            .manifests(&project_of(&fixture))
            .is_empty(),
        "a refused apply snapshots nothing"
    );
    let applied = apply(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
        shown,
    )
    .expect("applied");
    assert_eq!(applied.action, ApplyAction::Insert);
    let block = fixture.block(ManagedFileKind::Guidance);
    assert_eq!(
        fixture.text("AGENTS.md"),
        Some(format!("# Title\n\n{}\n", block.render(LineEnding::Lf)))
    );
    let manifests = fixture.snapshots.manifests(&project_of(&fixture));
    assert_eq!(manifests.len(), 1);
    assert!(manifests[0].existed);
    assert_eq!(manifests[0].sha256, shown.expect("digest"));
    assert_eq!(manifests[0].size, 8);
}

#[test]
fn apply_is_idempotent_and_a_no_op_takes_no_snapshot() {
    let mut fixture = Fixture::new("apply-idempotent");
    let workspace = fixture.workspace();
    let first = apply(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
        None,
    )
    .expect("applied");
    fixture.tick();
    let again = apply(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
        Some(first.result_sha256),
    )
    .expect("applied again");
    assert_eq!(again.action, ApplyAction::NoOp);
    assert_eq!(again.snapshot_id, None);
    assert_eq!(again.result_sha256, first.result_sha256);
    assert_eq!(fixture.snapshots.manifests(&project_of(&fixture)).len(), 1);
}

#[test]
fn apply_replaces_an_outdated_block_keeping_the_rest_of_the_file() {
    let mut fixture = Fixture::new("apply-replace");
    let workspace = fixture.workspace();
    let older = ManagedBlock::guidance(&OutboxPath::new("old/outbox").expect("valid"), &|bytes| {
        fixture.hasher.sha256(bytes)
    });
    let planted = format!(
        "# Title\n\n{}\n\nKeep this.\n",
        older.render(LineEnding::Lf)
    );
    fixture.files.plant("AGENTS.md", planted.as_bytes());
    let shown = fixture.file_digest("AGENTS.md");
    let before = status(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
    )
    .expect("status");
    assert_eq!(
        before.managed,
        ManagedStatus::Outdated {
            version: "hap-001-guidance-v1".to_string()
        }
    );
    let applied = apply(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
        shown,
    )
    .expect("applied");
    assert_eq!(applied.action, ApplyAction::Replace);
    let block = fixture.block(ManagedFileKind::Guidance);
    assert_eq!(
        fixture.text("AGENTS.md"),
        Some(format!(
            "# Title\n\n{}\n\nKeep this.\n",
            block.render(LineEnding::Lf)
        ))
    );
}

#[test]
fn apply_refuses_a_modified_or_malformed_block_without_snapshotting() {
    let mut fixture = Fixture::new("apply-refusals");
    let workspace = fixture.workspace();
    let block = fixture.block(ManagedFileKind::Guidance);
    let current = format!("{}\n", block.render(LineEnding::Lf));
    fixture.files.plant(
        "AGENTS.md",
        current
            .replace("Create the directory", "Create the folder")
            .as_bytes(),
    );
    let shown = fixture.file_digest("AGENTS.md");
    assert_eq!(
        apply(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            &agents(),
            &outbox(),
            shown
        )
        .map(|applied| applied.action),
        Err(GuidanceError::BlockModified)
    );
    assert_eq!(
        status(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            &agents(),
            &outbox()
        )
        .expect("status")
        .managed,
        ManagedStatus::Modified
    );
    fixture.files.plant(
        "AGENTS.md",
        current
            .replace("<!-- omnifrons:end guidance -->", "")
            .as_bytes(),
    );
    let shown = fixture.file_digest("AGENTS.md");
    assert_eq!(
        apply(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            &agents(),
            &outbox(),
            shown
        )
        .map(|applied| applied.action),
        Err(GuidanceError::BlockMalformed)
    );
    assert_eq!(
        preview(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            &agents(),
            &outbox()
        )
        .map(|preview| preview.action),
        Err(GuidanceError::BlockMalformed)
    );
    assert!(
        fixture
            .snapshots
            .manifests(&project_of(&fixture))
            .is_empty()
    );
}

/// A write that fails leaves the snapshot taken before it; the retry
/// deduplicates against that snapshot -- the same state, the same id --
/// and then writes.
#[test]
fn a_failed_write_keeps_its_snapshot_and_the_retry_dedups_against_it() {
    let mut fixture = Fixture::new("apply-retry");
    let workspace = fixture.workspace();
    fixture.files.plant("AGENTS.md", b"# Title\n");
    let shown = fixture.file_digest("AGENTS.md");
    fixture.files.fail_next_write();
    let error = apply(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
        shown,
    )
    .expect_err("the write fails");
    assert_eq!(
        error,
        GuidanceError::FileInvalid(ProjectTextFileError::WriteFailed)
    );
    assert_eq!(
        fixture.text("AGENTS.md").as_deref(),
        Some("# Title\n"),
        "untouched"
    );
    let manifests = fixture.snapshots.manifests(&project_of(&fixture));
    assert_eq!(manifests.len(), 1, "the snapshot stands");
    fixture.tick();
    let applied = apply(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
        shown,
    )
    .expect("the retry writes");
    assert_eq!(applied.snapshot_id, Some(manifests[0].id), "deduplicated");
    assert_eq!(fixture.snapshots.manifests(&project_of(&fixture)).len(), 1);
}

#[test]
fn apply_manages_the_ignore_rule_in_the_gitignore_file() {
    let mut fixture = Fixture::new("apply-ignore");
    let workspace = fixture.workspace();
    fixture.files.plant(".gitignore", b"/target\n*.log\n");
    let shown = fixture.file_digest(".gitignore");
    let applied = apply(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &ManagedTarget::ignore(),
        &outbox(),
        shown,
    )
    .expect("applied");
    assert_eq!(applied.action, ApplyAction::Insert);
    let block = fixture.block(ManagedFileKind::Ignore);
    assert_eq!(
        fixture.text(".gitignore"),
        Some(format!(
            "/target\n*.log\n\n{}\n",
            block.render(LineEnding::Lf)
        ))
    );
    assert!(
        fixture
            .text(".gitignore")
            .expect("text")
            .contains("\n/.omnifrons/outbox/\n")
    );
    let status = status(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &ManagedTarget::ignore(),
        &outbox(),
    )
    .expect("status");
    assert_eq!(status.file, ".gitignore");
    assert_eq!(status.managed, ManagedStatus::Current);
    assert_eq!(status.snapshots, 1);
    assert_eq!(
        snapshots(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            ManagedFileKind::Guidance
        )
        .expect("list")
        .len(),
        0,
        "the guidance kind has its own snapshots"
    );
}

// -- remove --

#[test]
fn remove_deletes_only_the_lines_omnifrons_owns_and_the_file_it_created() {
    let mut fixture = Fixture::new("remove");
    let workspace = fixture.workspace();
    // A file Omnifrons created is removed again.
    let applied = apply(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
        None,
    )
    .expect("applied");
    fixture.tick();
    let removed = remove(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        Some(applied.result_sha256),
    )
    .expect("removed");
    assert_eq!(removed.result_sha256, None, "the file is gone");
    assert_eq!(fixture.text("AGENTS.md"), None);
    let manifests = fixture.snapshots.manifests(&project_of(&fixture));
    assert_eq!(manifests.len(), 2, "the absence, then the applied state");
    assert_eq!(manifests[0].id, removed.snapshot_id);
    assert!(manifests[0].existed);

    // A file the user had keeps everything but the block.
    fixture.files.plant("CLAUDE.md", b"# Mine\n\nKeep.\n");
    let claude = ManagedTarget::guidance(ManagedFileName::guidance("CLAUDE.md").expect("valid"));
    let shown = fixture.file_digest("CLAUDE.md");
    fixture.tick();
    let applied = apply(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &claude,
        &outbox(),
        shown,
    )
    .expect("applied");
    fixture.tick();
    let removed = remove(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &claude,
        Some(applied.result_sha256),
    )
    .expect("removed");
    assert_eq!(
        fixture.text("CLAUDE.md").as_deref(),
        Some("# Mine\n\nKeep.\n")
    );
    assert_eq!(
        removed.result_sha256,
        fixture.file_digest("CLAUDE.md"),
        "the digest of what is left"
    );
}

#[test]
fn remove_refuses_an_unmanaged_file_a_changed_file_and_a_modified_block() {
    let mut fixture = Fixture::new("remove-refusals");
    let workspace = fixture.workspace();
    assert_eq!(
        remove(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            &agents(),
            None
        )
        .map(|removed| removed.snapshot_id),
        Err(GuidanceError::Unmanaged),
        "no file"
    );
    fixture.files.plant("AGENTS.md", b"# Title\n");
    let shown = fixture.file_digest("AGENTS.md");
    assert_eq!(
        remove(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            &agents(),
            shown
        )
        .map(|removed| removed.snapshot_id),
        Err(GuidanceError::Unmanaged),
        "no block"
    );
    assert_eq!(
        remove(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            &agents(),
            None
        )
        .map(|removed| removed.snapshot_id),
        Err(GuidanceError::FileChanged)
    );
    let block = fixture.block(ManagedFileKind::Guidance);
    let edited = format!("{}\n", block.render(LineEnding::Lf))
        .replace("Create the directory", "Create the folder");
    fixture.files.plant("AGENTS.md", edited.as_bytes());
    let shown = fixture.file_digest("AGENTS.md");
    assert_eq!(
        remove(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            &agents(),
            shown
        )
        .map(|removed| removed.snapshot_id),
        Err(GuidanceError::BlockModified),
        "a modified block is left to the user"
    );
    assert!(
        fixture
            .snapshots
            .manifests(&project_of(&fixture))
            .is_empty()
    );
}

// -- restore, pin, list --

#[test]
fn restore_snapshots_the_current_state_then_brings_the_snapshot_back() {
    let mut fixture = Fixture::new("restore");
    let workspace = fixture.workspace();
    fixture.files.plant("AGENTS.md", b"# Title\n");
    let shown = fixture.file_digest("AGENTS.md");
    let applied = apply(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
        shown,
    )
    .expect("applied");
    let original = applied.snapshot_id.expect("snapshot");
    fixture.tick();
    let restored = restore(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        original,
        Some(applied.result_sha256),
    )
    .expect("restored");
    assert_eq!(fixture.text("AGENTS.md").as_deref(), Some("# Title\n"));
    assert_eq!(restored.restored, original);
    assert_eq!(restored.result_sha256, shown);
    let manifests = fixture.snapshots.manifests(&project_of(&fixture));
    assert_eq!(manifests.len(), 2, "the state before the restore was kept");
    assert_eq!(manifests[0].id, restored.snapshot_id);
    assert_eq!(manifests[0].sha256, applied.result_sha256);

    // Restoring the pre-restore snapshot brings the note back; restoring a
    // snapshot of an absent file removes the file.
    fixture.tick();
    restore(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        restored.snapshot_id,
        shown,
    )
    .expect("restored the note");
    assert_eq!(
        fixture.file_digest("AGENTS.md"),
        Some(applied.result_sha256)
    );
    let absent = {
        let mut absent = fixture.snapshots.manifests(&project_of(&fixture))[0].clone();
        absent.id = omnifrons_app::snapshot_store::SnapshotId(77);
        absent.existed = false;
        absent.sha256 = fixture.digest_of(b"");
        absent.size = 0;
        absent.taken_at = SystemTime::UNIX_EPOCH;
        absent
    };
    let absent_id = absent.id;
    let project = project_of(&fixture);
    omnifrons_app::snapshot_store::SnapshotStore::record(
        &mut fixture.snapshots,
        &project,
        absent,
        b"",
    )
    .expect("record");
    fixture.tick();
    let restored = restore(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        absent_id,
        Some(applied.result_sha256),
    )
    .expect("restored the absence");
    assert_eq!(restored.result_sha256, None);
    assert_eq!(fixture.text("AGENTS.md"), None, "the file is removed");
}

#[test]
fn restore_refuses_an_unknown_snapshot_and_a_changed_file() {
    let mut fixture = Fixture::new("restore-refusals");
    let workspace = fixture.workspace();
    assert_eq!(
        restore(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            omnifrons_app::snapshot_store::SnapshotId(1),
            None,
        )
        .map(|restored| restored.restored),
        Err(GuidanceError::Snapshot(SnapshotStoreError::Unknown))
    );
    let applied = apply(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
        None,
    )
    .expect("applied");
    assert_eq!(
        restore(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            applied.snapshot_id.expect("snapshot"),
            None,
        )
        .map(|restored| restored.restored),
        Err(GuidanceError::FileChanged)
    );
}

/// R3-008 (slice 5c reliability review): a snapshot whose bytes no longer
/// digest to what its manifest records is refused as `Corrupt` before the
/// restore reads the file, snapshots it, or writes anything -- the current
/// file is left exactly as it was and the store gains nothing.
#[test]
fn restore_refuses_a_snapshot_whose_bytes_do_not_match_its_manifest() {
    let mut fixture = Fixture::new("restore-corrupt");
    let workspace = fixture.workspace();
    fixture.files.plant("AGENTS.md", b"# Title\n");
    let shown = fixture.file_digest("AGENTS.md");
    let project = project_of(&fixture);
    // A manifest recording `[0x11; 32]` beside bytes that digest to
    // something else: a half-written or tampered-with snapshot.
    let manifest = sample_manifest(1, 1_725_782_401, true, 0x11);
    assert_ne!(fixture.digest_of(b"tampered"), manifest.sha256);
    let corrupt = omnifrons_app::snapshot_store::SnapshotStore::record(
        &mut fixture.snapshots,
        &project,
        manifest,
        b"tampered",
    )
    .expect("record");
    assert_eq!(
        restore(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            corrupt,
            shown,
        )
        .map(|restored| restored.restored),
        Err(GuidanceError::Snapshot(SnapshotStoreError::Corrupt))
    );
    assert_eq!(
        fixture.text("AGENTS.md").as_deref(),
        Some("# Title\n"),
        "the current file is untouched"
    );
    assert_eq!(
        fixture.snapshots.manifests(&project_of(&fixture)).len(),
        1,
        "a refused restore snapshots nothing"
    );
}

#[test]
fn snapshots_list_newest_first_and_pins_are_counted() {
    let mut fixture = Fixture::new("pins");
    let workspace = fixture.workspace();
    let first = apply(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
        None,
    )
    .expect("applied");
    fixture.tick();
    fixture.files.plant("AGENTS.md", b"# Replaced by hand\n");
    let shown = fixture.file_digest("AGENTS.md");
    let second = apply(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
        shown,
    )
    .expect("applied");
    let listed = snapshots(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        ManagedFileKind::Guidance,
    )
    .expect("list");
    assert_eq!(
        listed.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![
            second.snapshot_id.expect("snapshot"),
            first.snapshot_id.expect("snapshot")
        ]
    );
    pin(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        first.snapshot_id.expect("snapshot"),
        true,
    )
    .expect("pinned");
    let status = status(
        &mut fixture.ports(&[&workspace]),
        &workspace,
        &agents(),
        &outbox(),
    )
    .expect("status");
    assert_eq!(status.snapshots, 2);
    assert_eq!(status.pinned, 1);
    assert_eq!(
        pin(
            &mut fixture.ports(&[&workspace]),
            &workspace,
            omnifrons_app::snapshot_store::SnapshotId(99),
            true,
        ),
        Err(GuidanceError::Snapshot(SnapshotStoreError::Unknown))
    );
}

// -- the work area (HAP-001-R7) and the reason tokens --

#[test]
fn a_work_area_inside_the_workspace_refuses_every_operation() {
    let mut fixture = Fixture::new("work-area");
    let workspace = fixture.workspace();
    // The base directory the work area sits in is registered as a
    // workspace afterwards: every use re-checks and refuses.
    let over = WorkspaceRoot::new(fixture.work_base.path()).expect("valid");
    let workspaces = [&workspace, &over];
    assert_eq!(
        status(
            &mut fixture.ports(&workspaces),
            &workspace,
            &agents(),
            &outbox()
        )
        .map(|status| status.managed),
        Err(GuidanceError::WorkAreaInvalid)
    );
    assert_eq!(
        apply(
            &mut fixture.ports(&workspaces),
            &workspace,
            &agents(),
            &outbox(),
            None
        )
        .map(|applied| applied.action),
        Err(GuidanceError::WorkAreaInvalid)
    );
    assert_eq!(
        remove(&mut fixture.ports(&workspaces), &workspace, &agents(), None)
            .map(|removed| removed.snapshot_id),
        Err(GuidanceError::WorkAreaInvalid)
    );
    assert_eq!(
        restore(
            &mut fixture.ports(&workspaces),
            &workspace,
            omnifrons_app::snapshot_store::SnapshotId(1),
            None,
        )
        .map(|restored| restored.restored),
        Err(GuidanceError::WorkAreaInvalid)
    );
    // R3-007: the read-only three refuse as well -- every operation
    // re-checks the work area before anything else (HAP-001-R7).
    assert_eq!(
        preview(
            &mut fixture.ports(&workspaces),
            &workspace,
            &agents(),
            &outbox()
        )
        .map(|preview| preview.action),
        Err(GuidanceError::WorkAreaInvalid)
    );
    assert_eq!(
        snapshots(
            &mut fixture.ports(&workspaces),
            &workspace,
            ManagedFileKind::Guidance
        )
        .map(|listed| listed.len()),
        Err(GuidanceError::WorkAreaInvalid)
    );
    assert_eq!(
        pin(
            &mut fixture.ports(&workspaces),
            &workspace,
            omnifrons_app::snapshot_store::SnapshotId(1),
            true,
        ),
        Err(GuidanceError::WorkAreaInvalid)
    );
    assert_eq!(fixture.text("AGENTS.md"), None, "nothing written");
    assert!(
        fixture
            .snapshots
            .manifests(&project_of(&fixture))
            .is_empty(),
        "nothing snapshotted"
    );
}

#[test]
fn every_error_has_a_fixed_reason_token() {
    let cases = [
        (GuidanceError::WorkAreaInvalid, "work-area-invalid"),
        (
            GuidanceError::FileInvalid(ProjectTextFileError::NotAFile),
            "not-a-regular-file",
        ),
        (
            GuidanceError::FileInvalid(ProjectTextFileError::TooLarge),
            "too-large",
        ),
        (GuidanceError::FileChanged, "file-changed"),
        (GuidanceError::BlockModified, "block-modified"),
        (GuidanceError::BlockMalformed, "block-malformed"),
        (GuidanceError::Unmanaged, "unmanaged"),
        (
            GuidanceError::Snapshot(SnapshotStoreError::Corrupt),
            "snapshot-store-corrupt",
        ),
        (
            GuidanceError::Snapshot(SnapshotStoreError::Unknown),
            "no-such-snapshot",
        ),
        (GuidanceError::VerifyFailed, "verify-failed"),
    ];
    for (error, reason) in cases {
        assert_eq!(error.reason(), reason);
        assert!(!error.reason().contains('/'));
        assert!(!error.to_string().contains('/'));
    }
}
