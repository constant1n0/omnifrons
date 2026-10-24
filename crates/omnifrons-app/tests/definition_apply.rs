//! The `definition::apply` service (ADR-0005 sub-slice 2d) against the
//! in-memory doubles: the order of writes, the permission rule each one
//! asks for, the snapshots and the record it leaves, and what stops it.
//!
//! Only compiled with `--features contract-tests`: the doubles live
//! behind that feature.

#![cfg(feature = "contract-tests")]

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::apply_record::ApplyRecordStore;
use omnifrons_app::content_hasher::{ContentHasher, derive_project_identity};
use omnifrons_app::contract::apply_record::InMemoryApplyRecordStore;
use omnifrons_app::contract::approval_store::FixedClock;
use omnifrons_app::contract::definition::{InMemoryDefinitionTree, TreeSnapshotBuilder};
use omnifrons_app::contract::definition_writer::{FakeEntry, InMemoryDefinitionWriter};
use omnifrons_app::contract::guidance::InMemorySnapshotStore;
use omnifrons_app::contract::publication::FakeHasher;
use omnifrons_app::definition::DefinitionError;
use omnifrons_app::definition::DefinitionPorts;
use omnifrons_app::definition_apply::{ApplyOutcome, ApplyPorts, apply};
use omnifrons_app::definition_plan::{PlanOutcome, plan};
use omnifrons_app::definition_tree::{DefinitionTree, DefinitionTreeError, TreeSnapshot};
use omnifrons_app::definition_writer::{DefinitionWriteError, ModePolicy};
use omnifrons_app::snapshot_store::DefinitionSnapshotStore;
use omnifrons_domain::definition::{FileShape, IdentityFile, IdentityKind, WRAPPER};
use omnifrons_domain::definition_plan::{InputFingerprint, PlanId, PlanOptions};

const TEXT: &[u8] = b"# Rules\n\nBe kind.\n";
const OTHER: &[u8] = b"# Other\n\nBe brief.\n";

/// A drop-guard temp directory standing in for a workspace.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-definition-apply-test-{}-{label}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("fixture dir");
        Self(dir)
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

fn regular(kind: IdentityKind, bytes: &[u8]) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: bytes.to_vec(),
        shape: FileShape::Regular,
    }
}

fn fixed_tree(workspace: &WorkspaceRoot, files: Vec<IdentityFile>) -> InMemoryDefinitionTree {
    let snapshot = TreeSnapshotBuilder::new()
        .directory(".", files, Vec::new(), Vec::new())
        .build();
    InMemoryDefinitionTree::new().with_snapshot(workspace, snapshot)
}

/// A tree that reports whatever identity files the writer double holds at
/// the workspace root, so a write is visible to the next read. An optional
/// hook runs just before the `n`th read (1-based).
struct WriterTree<'a> {
    writer: &'a InMemoryDefinitionWriter,
    reads: Cell<usize>,
    hook: Option<(usize, Box<dyn Fn() + 'a>)>,
}

impl<'a> WriterTree<'a> {
    fn new(writer: &'a InMemoryDefinitionWriter) -> Self {
        Self {
            writer,
            reads: Cell::new(0),
            hook: None,
        }
    }

    fn before_read(mut self, n: usize, hook: impl Fn() + 'a) -> Self {
        self.hook = Some((n, Box::new(hook)));
        self
    }
}

impl DefinitionTree for WriterTree<'_> {
    fn snapshot(&self, _root: &WorkspaceRoot) -> Result<TreeSnapshot, DefinitionTreeError> {
        self.reads.set(self.reads.get() + 1);
        if let Some((n, hook)) = &self.hook
            && *n == self.reads.get()
        {
            hook();
        }
        let files = [
            IdentityKind::Agents,
            IdentityKind::Claude,
            IdentityKind::Gemini,
        ]
        .into_iter()
        .filter_map(|kind| {
            self.writer
                .bytes(kind.file_name())
                .map(|bytes| regular(kind, &bytes))
        })
        .collect();
        Ok(TreeSnapshotBuilder::new()
            .directory(".", files, Vec::new(), Vec::new())
            .build())
    }
}

struct Fixture {
    snapshots: InMemorySnapshotStore,
    records: InMemoryApplyRecordStore,
    clock: FixedClock,
}

impl Fixture {
    fn new() -> Self {
        Self {
            snapshots: InMemorySnapshotStore::new(),
            records: InMemoryApplyRecordStore::new(),
            clock: FixedClock::new(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000)),
        }
    }

    fn apply(
        &mut self,
        tree: &dyn DefinitionTree,
        writer: &InMemoryDefinitionWriter,
        workspace: &WorkspaceRoot,
        target: Option<&str>,
        options: PlanOptions,
        expected: &PlanId,
    ) -> Result<ApplyOutcome, DefinitionError> {
        let mut ports = ApplyPorts {
            tree,
            hasher: &FakeHasher,
            writer,
            snapshots: &mut self.snapshots,
            records: &self.records,
            clock: &self.clock,
        };
        apply(&mut ports, workspace, target, &options, expected)
    }
}

/// The id the current plan would carry, or a panic when it is refused.
fn plan_id(
    tree: &dyn DefinitionTree,
    workspace: &WorkspaceRoot,
    target: Option<&str>,
    options: PlanOptions,
) -> PlanId {
    let ports = DefinitionPorts {
        tree,
        hasher: &FakeHasher,
    };
    match plan(&ports, workspace, target, &options).expect("plan") {
        PlanOutcome::Planned(plan) => plan.id,
        PlanOutcome::Refused(refusal) => panic!("expected a plan, got {refusal}"),
    }
}

fn digest(bytes: &[u8]) -> omnifrons_domain::executable::Sha256Digest {
    FakeHasher.sha256(bytes)
}

#[test]
fn a_refused_root_returns_refused_and_writes_nothing() {
    let dir = TempDir::new("refused");
    let workspace = dir.workspace();
    let tree = fixed_tree(
        &workspace,
        vec![
            regular(IdentityKind::Agents, TEXT),
            regular(IdentityKind::Claude, OTHER),
        ],
    );
    let writer = InMemoryDefinitionWriter::new()
        .with_file("AGENTS.md", TEXT)
        .with_file("CLAUDE.md", OTHER);
    let mut fixture = Fixture::new();
    let outcome = fixture
        .apply(
            &tree,
            &writer,
            &workspace,
            None,
            PlanOptions::default(),
            &PlanId([9; 32]),
        )
        .expect("a refusal is an outcome");
    match outcome {
        ApplyOutcome::Refused(refusal) => assert_eq!(refusal.code(), "conflict"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(writer.writes().is_empty());
    let project = derive_project_identity(&FakeHasher, &workspace);
    assert!(
        fixture
            .snapshots
            .list_definitions(&project, None)
            .unwrap()
            .is_empty()
    );
    assert!(fixture.records.list(&project).unwrap().is_empty());
}

#[test]
fn an_id_mismatch_is_stale_and_writes_nothing() {
    let dir = TempDir::new("mismatch");
    let workspace = dir.workspace();
    let tree = fixed_tree(&workspace, vec![regular(IdentityKind::Claude, TEXT)]);
    let writer = InMemoryDefinitionWriter::new().with_file("CLAUDE.md", TEXT);
    let actual = plan_id(&tree, &workspace, None, PlanOptions::default());
    let mut fixture = Fixture::new();
    let error = fixture
        .apply(
            &tree,
            &writer,
            &workspace,
            None,
            PlanOptions::default(),
            &PlanId([9; 32]),
        )
        .expect_err("a different id is stale");
    assert_eq!(error.reason(), "stale-plan");
    assert_eq!(
        error,
        DefinitionError::StalePlan {
            expected: PlanId([9; 32]),
            actual: Some(actual),
        }
    );
    assert!(writer.writes().is_empty());
    let project = derive_project_identity(&FakeHasher, &workspace);
    assert!(
        fixture
            .snapshots
            .list_definitions(&project, None)
            .unwrap()
            .is_empty()
    );
    assert!(fixture.records.list(&project).unwrap().is_empty());
}

#[test]
fn a_canonical_root_is_a_no_op_with_no_snapshot_and_no_record() {
    let dir = TempDir::new("canonical");
    let workspace = dir.workspace();
    let tree = fixed_tree(
        &workspace,
        vec![
            regular(IdentityKind::Agents, TEXT),
            regular(IdentityKind::Claude, WRAPPER),
        ],
    );
    let writer = InMemoryDefinitionWriter::new()
        .with_file("AGENTS.md", TEXT)
        .with_file("CLAUDE.md", WRAPPER);
    let options = PlanOptions::default();
    let id = plan_id(&tree, &workspace, None, options);
    let mut fixture = Fixture::new();
    let outcome = fixture
        .apply(&tree, &writer, &workspace, None, options, &id)
        .expect("apply");
    assert_eq!(outcome, ApplyOutcome::NoOp { plan_id: id });
    assert!(writer.writes().is_empty());
    let project = derive_project_identity(&FakeHasher, &workspace);
    assert!(
        fixture
            .snapshots
            .list_definitions(&project, None)
            .unwrap()
            .is_empty()
    );
    assert!(fixture.records.list(&project).unwrap().is_empty());
}

#[test]
fn a_claude_only_root_applies_in_order_snapshots_what_existed_and_records_every_file() {
    let dir = TempDir::new("claude-only");
    let workspace = dir.workspace();
    let writer = InMemoryDefinitionWriter::new().with_file("CLAUDE.md", TEXT);
    let tree = WriterTree::new(&writer);
    let options = PlanOptions::default();
    let id = plan_id(&tree, &workspace, None, options);
    let mut fixture = Fixture::new();

    let outcome = fixture
        .apply(&tree, &writer, &workspace, None, options, &id)
        .expect("apply");

    let log = writer.writes();
    assert_eq!(
        log.iter().map(|w| w.path.as_str()).collect::<Vec<_>>(),
        ["AGENTS.md", "CLAUDE.md"]
    );
    assert_eq!(log[0].bytes, TEXT);
    assert_eq!(log[0].expected, InputFingerprint::Absent);
    assert_eq!(
        log[0].mode,
        ModePolicy::NoWiderThan {
            source: "CLAUDE.md".to_string()
        }
    );
    assert_eq!(log[1].bytes, WRAPPER);
    assert_eq!(log[1].expected, InputFingerprint::Sha256(digest(TEXT)));
    assert_eq!(
        log[1].mode,
        ModePolicy::NoWiderThan {
            source: "AGENTS.md".to_string()
        }
    );

    let project = derive_project_identity(&FakeHasher, &workspace);
    let snapshots = fixture
        .snapshots
        .list_definitions(&project, Some("."))
        .unwrap();
    assert_eq!(
        snapshots.len(),
        1,
        "AGENTS.md did not exist, so only CLAUDE.md"
    );
    assert_eq!(snapshots[0].subject.file(), "CLAUDE.md");
    assert_eq!(snapshots[0].sha256, digest(TEXT));
    let (_, kept) = fixture
        .snapshots
        .read_definition(&project, snapshots[0].id)
        .unwrap();
    assert_eq!(kept, TEXT);

    let ApplyOutcome::Applied {
        plan_id,
        record,
        files,
    } = outcome
    else {
        panic!("expected Applied, got {outcome:?}");
    };
    assert_eq!(plan_id, id);
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path, "AGENTS.md");
    assert_eq!(files[0].before, InputFingerprint::Absent);
    assert_eq!(files[0].after, Some(digest(TEXT)));
    assert_eq!(files[0].snapshot, None);
    assert_eq!(files[1].path, "CLAUDE.md");
    assert_eq!(files[1].before, InputFingerprint::Sha256(digest(TEXT)));
    assert_eq!(files[1].after, Some(digest(WRAPPER)));
    assert_eq!(files[1].snapshot, Some(snapshots[0].id));

    let stored = fixture.records.list(&project).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].id, record);
    assert_eq!(stored[0].record.plan_id, id);
    assert_eq!(stored[0].record.agent, ".");
    assert_eq!(stored[0].record.files, files);
    assert_eq!(stored[0].record.operations.len(), 2);
    assert_eq!(
        stored[0].record.taken_at,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000)
    );
}

#[test]
fn a_second_plan_after_an_apply_is_a_no_op_and_the_old_id_is_stale() {
    let dir = TempDir::new("second");
    let workspace = dir.workspace();
    let writer = InMemoryDefinitionWriter::new().with_file("CLAUDE.md", TEXT);
    let tree = WriterTree::new(&writer);
    let options = PlanOptions::default();
    let first = plan_id(&tree, &workspace, None, options);
    let mut fixture = Fixture::new();
    fixture
        .apply(&tree, &writer, &workspace, None, options, &first)
        .expect("first apply");

    let second = plan_id(&tree, &workspace, None, options);
    let outcome = fixture
        .apply(&tree, &writer, &workspace, None, options, &second)
        .expect("second apply");
    assert_eq!(outcome, ApplyOutcome::NoOp { plan_id: second });
    assert_eq!(writer.writes().len(), 2, "the second run wrote nothing");

    let replay = fixture
        .apply(&tree, &writer, &workspace, None, options, &first)
        .expect_err("the consumed plan is stale");
    assert_eq!(replay.reason(), "stale-plan");
    let project = derive_project_identity(&FakeHasher, &workspace);
    assert_eq!(fixture.records.list(&project).unwrap().len(), 1);
}

#[test]
fn a_target_the_writer_reports_stale_stops_the_run_with_no_record() {
    let dir = TempDir::new("writer-stale");
    let workspace = dir.workspace();
    // The tree shows what the plan saw; the disk has moved on.
    let tree = fixed_tree(&workspace, vec![regular(IdentityKind::Claude, TEXT)]);
    let writer = InMemoryDefinitionWriter::new().with_file("CLAUDE.md", TEXT);
    let options = PlanOptions::default();
    let id = plan_id(&tree, &workspace, None, options);
    writer.put("AGENTS.md", FakeEntry::File(b"someone else\n".to_vec()));

    let mut fixture = Fixture::new();
    let error = fixture
        .apply(&tree, &writer, &workspace, None, options, &id)
        .expect_err("the writer refuses");
    assert_eq!(error.reason(), "stale-plan");
    assert!(matches!(
        error,
        DefinitionError::StalePlan { actual: None, .. }
    ));
    assert!(writer.writes().is_empty());
    assert_eq!(
        writer.bytes("AGENTS.md").as_deref(),
        Some(&b"someone else\n"[..])
    );
    let project = derive_project_identity(&FakeHasher, &workspace);
    assert!(fixture.records.list(&project).unwrap().is_empty());
}

#[test]
fn the_writers_other_errors_surface_as_write() {
    let dir = TempDir::new("writer-error");
    let workspace = dir.workspace();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![regular(IdentityKind::Agents, TEXT)],
            Vec::new(),
            Vec::new(),
        )
        .directory(
            "child",
            vec![regular(IdentityKind::Claude, OTHER)],
            Vec::new(),
            Vec::new(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    // The writer double has no `child` directory.
    let writer = InMemoryDefinitionWriter::new();
    let options = PlanOptions {
        nested_consent: true,
        ..PlanOptions::default()
    };
    let id = plan_id(&tree, &workspace, Some("child"), options);

    let mut fixture = Fixture::new();
    let error = fixture
        .apply(&tree, &writer, &workspace, Some("child"), options, &id)
        .expect_err("the parent is missing");
    assert_eq!(error.reason(), "parent-missing");
    assert!(matches!(
        error,
        DefinitionError::Write(DefinitionWriteError::ParentMissing { .. })
    ));
    let project = derive_project_identity(&FakeHasher, &workspace);
    assert!(fixture.records.list(&project).unwrap().is_empty());
}

#[test]
fn an_input_that_changes_between_two_writes_stops_the_run_as_stale() {
    let dir = TempDir::new("recheck");
    let workspace = dir.workspace();
    let writer = InMemoryDefinitionWriter::new()
        .with_file("CLAUDE.md", TEXT)
        .with_file("GEMINI.md", TEXT);
    let options = PlanOptions {
        with_gemini: true,
        ..PlanOptions::default()
    };
    let id = plan_id(&WriterTree::new(&writer), &workspace, None, options);
    // Reads: 1 plans, 2 checks before the first write, 3 checks after
    // it, 4 checks before the second. A concurrent edit of GEMINI.md,
    // which the first write does not touch, lands before read 4.
    let tree = WriterTree::new(&writer).before_read(4, || {
        writer.put("GEMINI.md", FakeEntry::File(b"edited meanwhile\n".to_vec()));
    });

    let mut fixture = Fixture::new();
    let error = fixture
        .apply(&tree, &writer, &workspace, None, options, &id)
        .expect_err("the edit is noticed");
    assert_eq!(error.reason(), "stale-plan");
    assert_eq!(writer.writes().len(), 1, "only AGENTS.md was written");
    assert_eq!(writer.bytes("CLAUDE.md").as_deref(), Some(TEXT));
    assert_eq!(
        writer.bytes("GEMINI.md").as_deref(),
        Some(&b"edited meanwhile\n"[..])
    );
    let project = derive_project_identity(&FakeHasher, &workspace);
    assert!(fixture.records.list(&project).unwrap().is_empty());
}
