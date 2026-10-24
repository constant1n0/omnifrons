//! Filesystem tests for `FsApplyRecordStore` (ADR-0005 sub-slice 2c): one
//! JSON file per apply under the work area's `applies/<project hex>/`,
//! written atomically and never over an earlier record.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use omnifrons_adapters::FsApplyRecordStore;
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::apply_record::{
    AppliedFile, ApplyRecord, ApplyRecordError, ApplyRecordStore, apply_record_name,
};
use omnifrons_app::contract::apply_record::{
    InMemoryApplyRecordStore, apply_record_store_contract,
};
use omnifrons_app::snapshot_store::SnapshotId;
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::definition::IdentityKind;
use omnifrons_domain::definition_plan::{BlockMode, InputFingerprint, Operation, PlanId};
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
            "omnifrons-apply-record-test-{}-{label}-{n}",
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

fn sample_record(plan_byte: u8, secs: u64) -> ApplyRecord {
    let after = Sha256Digest([0x22; 32]);
    ApplyRecord {
        plan_id: PlanId([plan_byte; 32]),
        workspace: "/work/space".to_string(),
        agent: "child/grandchild".to_string(),
        taken_at: SystemTime::UNIX_EPOCH + Duration::new(secs, 5),
        files: vec![
            AppliedFile {
                path: "AGENTS.md".to_string(),
                before: InputFingerprint::Sha256(Sha256Digest([0x11; 32])),
                after: Some(after),
                snapshot: Some(SnapshotId(0xabc)),
            },
            AppliedFile {
                path: "CLAUDE.md".to_string(),
                before: InputFingerprint::Absent,
                after: Some(Sha256Digest([0x33; 32])),
                snapshot: None,
            },
        ],
        operations: vec![
            Operation::WriteAgents {
                source: IdentityKind::Claude,
                after,
                before: InputFingerprint::Sha256(Sha256Digest([0x11; 32])),
            },
            Operation::WriteWrapper {
                kind: IdentityKind::Claude,
                after: Sha256Digest([0x33; 32]),
                before: InputFingerprint::Symlink("elsewhere.md".to_string()),
            },
            Operation::WriteBlock {
                kind: ManagedFileKind::Skills,
                mode: BlockMode::MigrateLegacy,
                body: Sha256Digest([0x44; 32]),
                after: Sha256Digest([0x55; 32]),
                before: InputFingerprint::Irregular,
            },
        ],
    }
}

#[test]
fn the_shared_contract_holds_for_the_filesystem_store() {
    let base = TempDir::new("contract-fs");
    let project_dir = TempDir::new("contract-fs-project");
    let area = work_area(&base, &project_dir);
    apply_record_store_contract(&FsApplyRecordStore::open(&area));
}

#[test]
fn the_shared_contract_holds_for_the_in_memory_store() {
    apply_record_store_contract(&InMemoryApplyRecordStore::new());
}

#[test]
fn a_recorded_apply_lists_back_equal() {
    let base = TempDir::new("round-trip");
    let project_dir = TempDir::new("round-trip-project");
    let store = FsApplyRecordStore::open(&work_area(&base, &project_dir));
    let record = sample_record(1, 1_725_782_401);
    let id = store.record(&project(), &record).expect("record");
    let listed = store.list(&project()).expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
    assert_eq!(listed[0].record, record);
}

#[test]
fn two_records_for_one_plan_id_both_survive_under_distinct_names() {
    let base = TempDir::new("rerun");
    let project_dir = TempDir::new("rerun-project");
    let area = work_area(&base, &project_dir);
    let store = FsApplyRecordStore::open(&area);
    // The same plan id and the same instant: only the name's sequence
    // tells them apart.
    let first = store
        .record(&project(), &sample_record(1, 100))
        .expect("first");
    let second = store
        .record(&project(), &sample_record(1, 100))
        .expect("second");
    assert_ne!(first, second);
    let later = store
        .record(&project(), &sample_record(1, 200))
        .expect("later");
    let listed = store.list(&project()).expect("list");
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].id, later, "newest first");
    let dir = area.applies_dir().join(project().to_hex());
    let names: Vec<String> = std::fs::read_dir(&dir)
        .expect("applies dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        names.len(),
        3,
        "no temporary file is left behind: {names:?}"
    );
    assert!(
        names
            .iter()
            .all(|name| name.starts_with(&PlanId([1; 32]).to_string()))
    );
}

#[test]
fn the_stored_json_shape_is_stable() {
    let base = TempDir::new("shape");
    let project_dir = TempDir::new("shape-project");
    let area = work_area(&base, &project_dir);
    let store = FsApplyRecordStore::open(&area);
    store
        .record(&project(), &sample_record(1, 1_725_782_401))
        .expect("record");
    let dir = area.applies_dir().join(project().to_hex());
    let entry = std::fs::read_dir(&dir)
        .expect("applies dir")
        .next()
        .expect("one file")
        .expect("entry")
        .path();
    let text = std::fs::read_to_string(entry).expect("read");
    let expected = concat!(
        r#"{"schema":1,"planId":"0101010101010101010101010101010101010101010101010101010101010101","#,
        r#""workspace":"/work/space","agent":"child/grandchild","#,
        r#""takenAt":{"secs":1725782401,"nanos":5},"files":["#,
        r#"{"path":"AGENTS.md","before":{"kind":"sha256","hex":"1111111111111111111111111111111111111111111111111111111111111111"},"#,
        r#""after":"2222222222222222222222222222222222222222222222222222222222222222","snapshot":"0000000000000abc"},"#,
        r#"{"path":"CLAUDE.md","before":{"kind":"absent"},"#,
        r#""after":"3333333333333333333333333333333333333333333333333333333333333333","snapshot":null}],"#,
        r#""operations":["#,
        r#"{"op":"write-agents","source":"CLAUDE.md","after":"2222222222222222222222222222222222222222222222222222222222222222","before":{"kind":"sha256","hex":"1111111111111111111111111111111111111111111111111111111111111111"}},"#,
        r#"{"op":"write-wrapper","path":"CLAUDE.md","after":"3333333333333333333333333333333333333333333333333333333333333333","before":{"kind":"symlink","target":"elsewhere.md"}},"#,
        r#"{"op":"write-block","blockKind":"skills","mode":"migrate-legacy","body":"4444444444444444444444444444444444444444444444444444444444444444","after":"5555555555555555555555555555555555555555555555555555555555555555","before":{"kind":"irregular"}}]}"#,
    );
    assert_eq!(text, expected);
}

#[test]
fn a_file_that_does_not_match_the_shape_fails_the_listing_closed() {
    let base = TempDir::new("corrupt");
    let project_dir = TempDir::new("corrupt-project");
    let area = work_area(&base, &project_dir);
    let store = FsApplyRecordStore::open(&area);
    store
        .record(&project(), &sample_record(1, 100))
        .expect("record");
    let dir = area.applies_dir().join(project().to_hex());
    std::fs::write(dir.join("junk.json"), r#"{"schema":2}"#).expect("junk");
    assert_eq!(store.list(&project()), Err(ApplyRecordError::Corrupt));
}

#[test]
fn an_unwritable_work_area_errors_with_its_token() {
    let base = TempDir::new("unwritable");
    let project_dir = TempDir::new("unwritable-project");
    let area = work_area(&base, &project_dir);
    let store = FsApplyRecordStore::open(&area);
    // A regular file where the applies directory should be.
    let _ = std::fs::remove_dir_all(area.applies_dir());
    std::fs::write(area.applies_dir(), b"not a directory").expect("plant a file");
    let failure = store
        .record(&project(), &sample_record(1, 100))
        .expect_err("cannot write");
    assert_eq!(failure, ApplyRecordError::WriteFailed);
    assert_eq!(failure.reason(), "apply-record-write-failed");
    let listing = store.list(&project()).expect_err("cannot read");
    assert_eq!(listing.reason(), "apply-record-unreadable");
}

#[test]
fn a_missing_applies_directory_lists_empty_and_is_created_on_record() {
    let base = TempDir::new("missing");
    let project_dir = TempDir::new("missing-project");
    let area = work_area(&base, &project_dir);
    let store = FsApplyRecordStore::open(&area);
    let _ = std::fs::remove_dir_all(area.applies_dir());
    assert_eq!(store.list(&project()), Ok(Vec::new()));
    store
        .record(&project(), &sample_record(1, 100))
        .expect("record");
    assert_eq!(store.list(&project()).expect("list").len(), 1);
}

#[test]
fn a_file_already_at_the_first_name_is_left_intact_and_the_record_takes_the_next() {
    let base = TempDir::new("preexisting");
    let project_dir = TempDir::new("preexisting-project");
    let area = work_area(&base, &project_dir);
    let store = FsApplyRecordStore::open(&area);
    let record = sample_record(1, 100);
    let dir = area.applies_dir().join(project().to_hex());
    std::fs::create_dir_all(&dir).expect("project dir");
    let first = apply_record_name(&record.plan_id, record.taken_at, 0);
    let planted = dir.join(format!("{first}.json"));
    std::fs::write(&planted, b"planted").expect("plant");
    let id = store.record(&project(), &record).expect("record");
    assert_eq!(id, apply_record_name(&record.plan_id, record.taken_at, 1));
    assert_eq!(std::fs::read(&planted).expect("read"), b"planted");
    assert!(dir.join(format!("{id}.json")).is_file());
}

#[test]
fn an_empty_reservation_left_by_a_crash_is_skipped_by_the_listing() {
    let base = TempDir::new("reservation");
    let project_dir = TempDir::new("reservation-project");
    let area = work_area(&base, &project_dir);
    let store = FsApplyRecordStore::open(&area);
    let record = sample_record(1, 100);
    store.record(&project(), &record).expect("record");
    let dir = area.applies_dir().join(project().to_hex());
    std::fs::write(dir.join("crashed.json"), b"").expect("empty reservation");
    assert_eq!(store.list(&project()).expect("list").len(), 1);
}
