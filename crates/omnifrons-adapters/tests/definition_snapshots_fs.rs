//! Filesystem tests for definition-file snapshots (ADR-0005 sub-slice 2c):
//! the identity files of a possibly nested agent directory recorded in the
//! existing snapshot store, beside -- never mixed with -- the guidance
//! snapshots that earlier releases wrote.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use omnifrons_adapters::FsSnapshotStore;
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::contract::guidance::{
    InMemorySnapshotStore, definition_snapshot_store_contract, sample_definition_manifest,
    sample_manifest,
};
use omnifrons_app::snapshot_store::{
    DefinitionSnapshotStore, DefinitionSubject, SnapshotId, SnapshotStore, SnapshotStoreError,
};
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::guidance::ManagedFileKind;
use omnifrons_domain::publication::ProjectIdentity;

/// A drop-guard temp directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-definition-snapshots-test-{}-{label}-{n}",
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

fn project() -> ProjectIdentity {
    ProjectIdentity(Sha256Digest([7; 32]))
}

fn work_area(base: &TempDir, project: &TempDir) -> WorkAreaRoot {
    WorkAreaRoot::open(&base.path().join("wa"), &[&project.workspace()]).expect("work area")
}

fn subject(agent: &str, file: &str) -> DefinitionSubject {
    DefinitionSubject::new(agent, file).expect("a valid subject")
}

#[test]
fn the_shared_definition_contract_holds_for_the_filesystem_store() {
    let base = TempDir::new("contract-fs");
    let project_dir = TempDir::new("contract-fs-project");
    let area = work_area(&base, &project_dir);
    definition_snapshot_store_contract(&mut FsSnapshotStore::open(&area));
}

#[test]
fn the_shared_definition_contract_holds_for_the_in_memory_store() {
    definition_snapshot_store_contract(&mut InMemorySnapshotStore::new());
}

#[test]
fn a_definition_snapshot_reads_back_by_id_with_its_subject_and_bytes() {
    let base = TempDir::new("round-trip");
    let project_dir = TempDir::new("round-trip-project");
    let mut store = FsSnapshotStore::open(&work_area(&base, &project_dir));
    let nested = subject("child/grandchild", "AGENTS.md");
    let manifest = sample_definition_manifest(1, 100, &nested, true, 0x11);
    let id = store
        .record_definition(&project(), manifest.clone(), b"one")
        .expect("record");
    let (read_back, bytes) = store.read_definition(&project(), id).expect("read");
    assert_eq!(read_back, manifest);
    assert_eq!(read_back.subject.agent(), "child/grandchild");
    assert_eq!(bytes, b"one");
}

#[test]
fn an_unchanged_file_dedups_within_one_subject_but_never_across_agents() {
    let base = TempDir::new("dedup");
    let project_dir = TempDir::new("dedup-project");
    let mut store = FsSnapshotStore::open(&work_area(&base, &project_dir));
    let root = subject(".", "AGENTS.md");
    let child = subject("child", "AGENTS.md");
    let first = store
        .record_definition(
            &project(),
            sample_definition_manifest(1, 100, &root, true, 0x11),
            b"same",
        )
        .expect("record");
    let again = store
        .record_definition(
            &project(),
            sample_definition_manifest(2, 200, &root, true, 0x11),
            b"same",
        )
        .expect("record");
    assert_eq!(again, first, "the unchanged file stands for the old one");
    let other_agent = store
        .record_definition(
            &project(),
            sample_definition_manifest(3, 300, &child, true, 0x11),
            b"same",
        )
        .expect("record");
    assert_ne!(
        other_agent, first,
        "another agent's file is its own snapshot"
    );
    assert_eq!(
        store
            .list_definitions(&project(), None)
            .expect("list")
            .len(),
        2
    );
}

#[test]
fn a_listing_by_agent_returns_only_that_agent_newest_first() {
    let base = TempDir::new("by-agent");
    let project_dir = TempDir::new("by-agent-project");
    let mut store = FsSnapshotStore::open(&work_area(&base, &project_dir));
    let root_agents = subject(".", "AGENTS.md");
    let root_claude = subject(".", "CLAUDE.md");
    let child_agents = subject("child", "AGENTS.md");
    for (n, secs, subject, byte) in [
        (1, 100, &root_agents, 0x01),
        (2, 200, &root_claude, 0x02),
        (3, 300, &child_agents, 0x03),
        (4, 400, &root_agents, 0x04),
    ] {
        store
            .record_definition(
                &project(),
                sample_definition_manifest(n, secs, subject, true, byte),
                b"x",
            )
            .expect("record");
    }
    let listed = store
        .list_definitions(&project(), Some("."))
        .expect("list by agent");
    let ids: Vec<u64> = listed.iter().map(|m| m.id.0).collect();
    assert_eq!(ids, vec![4, 2, 1]);
    let child = store
        .list_definitions(&project(), Some("child"))
        .expect("list");
    assert_eq!(child.len(), 1);
    assert!(
        store
            .list_definitions(&project(), Some("missing"))
            .expect("list")
            .is_empty()
    );
}

#[test]
fn definition_and_guidance_snapshots_share_the_store_without_touching_each_other() {
    let base = TempDir::new("coexist");
    let project_dir = TempDir::new("coexist-project");
    let area = work_area(&base, &project_dir);
    let mut store = FsSnapshotStore::open(&area);
    let guidance_id = store
        .record(&project(), sample_manifest(1, 100, true, 0x11), b"note")
        .expect("record guidance");
    let definition_id = store
        .record_definition(
            &project(),
            sample_definition_manifest(1, 100, &subject(".", "AGENTS.md"), true, 0x11),
            b"note",
        )
        .expect("record definition");
    let guidance = SnapshotStore::list(&store, &project(), ManagedFileKind::Guidance)
        .expect("the guidance listing stays readable");
    assert_eq!(guidance.len(), 1);
    assert_eq!(guidance[0].id, guidance_id);
    assert_eq!(
        store
            .list_definitions(&project(), None)
            .expect("list")
            .len(),
        1
    );
    // The guidance read never serves a definition snapshot, nor the reverse,
    // even when the ids are equal.
    assert_eq!(guidance_id, definition_id);
    let (manifest, _) = SnapshotStore::read(&store, &project(), guidance_id).expect("read");
    assert_eq!(manifest.kind, ManagedFileKind::Guidance);
    assert_eq!(
        SnapshotStore::read(&store, &project(), SnapshotId(0xdead)),
        Err(SnapshotStoreError::Unknown)
    );
}

#[test]
fn pruning_applies_per_subject() {
    let base = TempDir::new("prune");
    let project_dir = TempDir::new("prune-project");
    let mut store = FsSnapshotStore::open(&work_area(&base, &project_dir));
    let busy = subject(".", "AGENTS.md");
    let quiet = subject("child", "AGENTS.md");
    store
        .record_definition(
            &project(),
            sample_definition_manifest(100, 1, &quiet, true, 0xaa),
            b"q",
        )
        .expect("record");
    for n in 1..=7_u8 {
        store
            .record_definition(
                &project(),
                sample_definition_manifest(u64::from(n), u64::from(n) * 10, &busy, true, n),
                b"b",
            )
            .expect("record");
    }
    assert_eq!(
        store
            .list_definitions(&project(), Some("."))
            .expect("list")
            .len(),
        5
    );
    assert_eq!(
        store
            .list_definitions(&project(), Some("child"))
            .expect("list")
            .len(),
        1
    );
}

#[test]
fn a_manifest_written_by_an_earlier_release_still_parses_and_lists() {
    let base = TempDir::new("legacy");
    let project_dir = TempDir::new("legacy-project");
    let area = work_area(&base, &project_dir);
    let dir = area.snapshots_dir().join(project().to_hex());
    std::fs::create_dir_all(&dir).expect("project dir");
    let legacy = r#"{"schema":1,"id":"0000000000000001","kind":"guidance","file":"AGENTS.md","existed":true,"sha256":"0101010101010101010101010101010101010101010101010101010101010101","size":3,"takenAt":{"secs":1725782401,"nanos":0},"pinned":false}"#;
    std::fs::write(dir.join("0000000000000001.json"), legacy).expect("manifest");
    std::fs::write(dir.join("0000000000000001.bytes"), b"old").expect("bytes");

    let mut store = FsSnapshotStore::open(&area);
    let listed = SnapshotStore::list(&store, &project(), ManagedFileKind::Guidance)
        .expect("the legacy manifest lists");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].file, "AGENTS.md");
    assert_eq!(listed[0].sha256, Sha256Digest([1; 32]));
    assert_eq!(
        listed[0].taken_at,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_401)
    );
    // A definition record beside it leaves it listed, and a legacy record
    // still dedups as before.
    store
        .record_definition(
            &project(),
            sample_definition_manifest(9, 900, &subject(".", "AGENTS.md"), true, 0x01),
            b"old",
        )
        .expect("record definition");
    let deduped = store
        .record(&project(), sample_manifest(5, 5, true, 0x01), b"old")
        .expect("record guidance");
    assert_eq!(deduped, SnapshotId(1));
}

#[test]
fn a_subject_names_an_agent_directory_and_a_plain_file_only() {
    for (agent, file) in [
        ("", "AGENTS.md"),
        ("/abs", "AGENTS.md"),
        ("../up", "AGENTS.md"),
        ("a//b", "AGENTS.md"),
        ("a/./b", "AGENTS.md"),
        ("a\0b", "AGENTS.md"),
        ("a\nb", "AGENTS.md"),
        (".", ""),
        (".", "sub/AGENTS.md"),
        (".", ".."),
        (".", "a\0b.md"),
    ] {
        assert!(
            DefinitionSubject::new(agent, file).is_none(),
            "{agent:?} {file:?} must be refused"
        );
    }
    assert!(DefinitionSubject::new(".", "CLAUDE.md").is_some());
    assert!(DefinitionSubject::new("child/grandchild", "GEMINI.md").is_some());
}

#[cfg(unix)]
#[test]
fn a_backslash_or_colon_in_an_agent_path_is_an_ordinary_name_on_unix() {
    let base = TempDir::new("backslash");
    let project_dir = TempDir::new("backslash-project");
    let mut store = FsSnapshotStore::open(&work_area(&base, &project_dir));
    let odd = subject("a\\b/c:d", "AGENTS.md");
    store
        .record_definition(
            &project(),
            sample_definition_manifest(1, 100, &odd, true, 0x11),
            b"x",
        )
        .expect("record");
    let listed = store
        .list_definitions(&project(), Some("a\\b/c:d"))
        .expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].subject, odd);
}

#[cfg(windows)]
#[test]
fn a_backslash_or_colon_is_refused_on_windows() {
    assert!(DefinitionSubject::new("a\\b", "AGENTS.md").is_none());
    assert!(DefinitionSubject::new("a:b", "AGENTS.md").is_none());
    assert!(DefinitionSubject::new(".", "a\\b.md").is_none());
}
