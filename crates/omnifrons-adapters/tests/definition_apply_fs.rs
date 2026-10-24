//! `definition::apply` (ADR-0005 sub-slice 2d) end to end over the real
//! adapters on temporary directories: the filesystem tree, writer,
//! snapshot store, and apply-record store, with the real SHA-256 hasher.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use omnifrons_adapters::{
    FsApplyRecordStore, FsDefinitionTree, FsDefinitionWriter, FsSnapshotStore, Sha2Hasher,
};
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::apply_record::ApplyRecordStore;
use omnifrons_app::content_hasher::{ContentHasher, derive_project_identity};
use omnifrons_app::contract::approval_store::FixedClock;
use omnifrons_app::definition::{DefinitionError, DefinitionPorts};
use omnifrons_app::definition_apply::{ApplyOutcome, ApplyPorts, apply};
use omnifrons_app::definition_plan::{PlanOutcome, plan};
use omnifrons_app::definition_tree::{DefinitionTree, DefinitionTreeError, TreeSnapshot};
use omnifrons_app::snapshot_store::DefinitionSnapshotStore;
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::definition::{IdentityKind, WRAPPER};
use omnifrons_domain::definition_plan::{PlanId, PlanOptions};
use omnifrons_domain::publication::ProjectIdentity;

const TEXT: &[u8] = b"# Rules\n\nBe kind.\n";
const OTHER: &[u8] = b"# Other\n\nBe brief.\n";

/// A drop-guard temp directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-definition-apply-fs-test-{}-{label}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("fixture dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A tree that runs a hook just before its `n`th read (1-based), to
/// stand in for a concurrent editor.
struct HookedTree<'a> {
    inner: FsDefinitionTree,
    reads: Cell<usize>,
    hook: Option<(usize, Box<dyn Fn() + 'a>)>,
}

impl DefinitionTree for HookedTree<'_> {
    fn snapshot(&self, root: &WorkspaceRoot) -> Result<TreeSnapshot, DefinitionTreeError> {
        self.reads.set(self.reads.get() + 1);
        if let Some((n, hook)) = &self.hook
            && *n == self.reads.get()
        {
            hook();
        }
        self.inner.snapshot(root)
    }
}

/// A workspace and a work area, outside it, with the real stores.
struct Env {
    _workspace_dir: TempDir,
    _base: TempDir,
    root: PathBuf,
    workspace: WorkspaceRoot,
    snapshots: FsSnapshotStore,
    records: FsApplyRecordStore,
    clock: FixedClock,
}

impl Env {
    fn new(label: &str) -> Self {
        let workspace_dir = TempDir::new(label);
        let base = TempDir::new(&format!("{label}-base"));
        let workspace = WorkspaceRoot::new(workspace_dir.path()).expect("workspace");
        let work_area =
            WorkAreaRoot::open(&base.path().join("wa"), &[&workspace]).expect("work area");
        Self {
            root: workspace.path().to_path_buf(),
            snapshots: FsSnapshotStore::open(&work_area),
            records: FsApplyRecordStore::open(&work_area),
            clock: FixedClock::new(SystemTime::UNIX_EPOCH + Duration::from_secs(5_000)),
            workspace,
            _workspace_dir: workspace_dir,
            _base: base,
        }
    }

    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("parent dir");
        std::fs::write(path, bytes).expect("fixture file");
    }

    fn read(&self, relative: &str) -> Option<Vec<u8>> {
        std::fs::read(self.root.join(relative)).ok()
    }

    fn project(&self) -> ProjectIdentity {
        derive_project_identity(&Sha2Hasher, &self.workspace)
    }

    fn plan_id(&self, target: Option<&str>, options: PlanOptions) -> PlanId {
        let tree = FsDefinitionTree::new(None);
        let ports = DefinitionPorts {
            tree: &tree,
            hasher: &Sha2Hasher,
        };
        match plan(&ports, &self.workspace, target, &options).expect("plan") {
            PlanOutcome::Planned(plan) => plan.id,
            PlanOutcome::Refused(refusal) => panic!("expected a plan, got {refusal}"),
        }
    }

    fn apply_with(
        &mut self,
        tree: &dyn DefinitionTree,
        target: Option<&str>,
        options: PlanOptions,
        expected: &PlanId,
    ) -> Result<ApplyOutcome, DefinitionError> {
        let mut ports = ApplyPorts {
            tree,
            hasher: &Sha2Hasher,
            writer: &FsDefinitionWriter,
            snapshots: &mut self.snapshots,
            records: &self.records,
            clock: &self.clock,
        };
        apply(&mut ports, &self.workspace, target, &options, expected)
    }

    /// Plans, then applies that plan.
    fn apply(&mut self, target: Option<&str>, options: PlanOptions) -> ApplyOutcome {
        let id = self.plan_id(target, options);
        self.apply_with(&FsDefinitionTree::new(None), target, options, &id)
            .expect("apply")
    }

    fn snapshot_count(&self) -> usize {
        self.snapshots
            .list_definitions(&self.project(), None)
            .expect("list snapshots")
            .len()
    }

    fn record_count(&self) -> usize {
        self.records
            .list(&self.project())
            .expect("list records")
            .len()
    }

    /// The bytes kept by the newest snapshot of `agent`'s `file`.
    fn snapshot_bytes(&self, agent: &str, file: &str) -> Option<Vec<u8>> {
        let project = self.project();
        let manifest = self
            .snapshots
            .list_definitions(&project, Some(agent))
            .expect("list snapshots")
            .into_iter()
            .find(|m| m.subject.file() == file)?;
        let (_, bytes) = self
            .snapshots
            .read_definition(&project, manifest.id)
            .expect("read snapshot");
        Some(bytes)
    }
}

/// The `(name, bytes)` files a fixture root starts with.
type Files = Vec<(&'static str, &'static [u8])>;

fn applied(outcome: &ApplyOutcome) -> usize {
    match outcome {
        ApplyOutcome::Applied { files, .. } => files.len(),
        other => panic!("expected Applied, got {other:?}"),
    }
}

#[test]
fn a_second_run_is_a_no_op_and_a_restored_file_adds_no_snapshot() {
    let mut env = Env::new("second-run");
    env.write("CLAUDE.md", TEXT);
    let options = PlanOptions::default();
    assert_eq!(applied(&env.apply(None, options)), 2);
    assert_eq!(env.read("AGENTS.md").as_deref(), Some(TEXT));
    assert_eq!(env.read("CLAUDE.md").as_deref(), Some(WRAPPER));
    assert_eq!((env.snapshot_count(), env.record_count()), (1, 1));

    let outcome = env.apply(None, options);
    assert!(matches!(outcome, ApplyOutcome::NoOp { .. }), "{outcome:?}");
    assert_eq!(env.read("AGENTS.md").as_deref(), Some(TEXT));
    assert_eq!(env.read("CLAUDE.md").as_deref(), Some(WRAPPER));
    assert_eq!((env.snapshot_count(), env.record_count()), (1, 1));

    // The user puts the original CLAUDE.md back and removes AGENTS.md:
    // the same file state is snapshotted again, and the store keeps one.
    env.write("CLAUDE.md", TEXT);
    std::fs::remove_file(env.root.join("AGENTS.md")).expect("remove");
    assert_eq!(applied(&env.apply(None, options)), 2);
    assert_eq!((env.snapshot_count(), env.record_count()), (1, 2));
}

#[test]
fn a_stale_plan_is_refused_before_any_write() {
    let mut env = Env::new("stale");
    env.write("CLAUDE.md", TEXT);
    let options = PlanOptions::default();
    let id = env.plan_id(None, options);
    env.write("CLAUDE.md", OTHER);

    let error = env
        .apply_with(&FsDefinitionTree::new(None), None, options, &id)
        .expect_err("the plan is stale");
    assert_eq!(error.reason(), "stale-plan");
    assert_eq!(env.read("CLAUDE.md").as_deref(), Some(OTHER));
    assert_eq!(env.read("AGENTS.md"), None);
    assert_eq!((env.snapshot_count(), env.record_count()), (0, 0));
}

#[cfg(unix)]
#[test]
fn an_unsafe_input_is_refused_and_nothing_changes() {
    let mut env = Env::new("unsafe");
    env.write("elsewhere.md", TEXT);
    env.write("CLAUDE.md", OTHER);
    std::os::unix::fs::symlink("elsewhere.md", env.root.join("AGENTS.md")).expect("symlink");
    let id = PlanId([0; 32]);

    let outcome = env
        .apply_with(
            &FsDefinitionTree::new(None),
            None,
            PlanOptions::default(),
            &id,
        )
        .expect("a refusal is an outcome");
    let ApplyOutcome::Refused(refusal) = outcome else {
        panic!("expected a refusal, got {outcome:?}");
    };
    assert_eq!(refusal.code(), "unsafe");
    assert_eq!(env.read("CLAUDE.md").as_deref(), Some(OTHER));
    assert_eq!(env.read("elsewhere.md").as_deref(), Some(TEXT));
    assert!(
        std::fs::symlink_metadata(env.root.join("AGENTS.md"))
            .expect("link")
            .is_symlink()
    );
    assert_eq!((env.snapshot_count(), env.record_count()), (0, 0));
}

#[test]
fn canonical_from_snapshots_the_losing_file_before_overwriting_it() {
    let mut env = Env::new("canonical-from");
    env.write("AGENTS.md", TEXT);
    env.write("CLAUDE.md", OTHER);
    let options = PlanOptions {
        canonical_from: Some(IdentityKind::Claude),
        ..PlanOptions::default()
    };
    assert_eq!(applied(&env.apply(None, options)), 2);
    assert_eq!(env.read("AGENTS.md").as_deref(), Some(OTHER));
    assert_eq!(env.read("CLAUDE.md").as_deref(), Some(WRAPPER));
    assert_eq!(env.snapshot_bytes(".", "AGENTS.md").as_deref(), Some(TEXT));
    assert_eq!(env.snapshot_bytes(".", "CLAUDE.md").as_deref(), Some(OTHER));
    assert_eq!(env.snapshot_count(), 2);
}

#[test]
fn each_root_state_ends_canonical_with_exact_bytes() {
    let cases: [(&str, Files); 4] = [
        ("claude-only", vec![("CLAUDE.md", TEXT)]),
        ("agents-only", vec![("AGENTS.md", TEXT)]),
        ("byte-copy", vec![("AGENTS.md", TEXT), ("CLAUDE.md", TEXT)]),
        (
            "wrapper-already",
            vec![("AGENTS.md", TEXT), ("CLAUDE.md", WRAPPER)],
        ),
    ];
    for (label, files) in cases {
        let mut env = Env::new(label);
        for (name, bytes) in &files {
            env.write(name, bytes);
        }
        env.apply(None, PlanOptions::default());
        assert_eq!(env.read("AGENTS.md").as_deref(), Some(TEXT), "{label}");
        assert_eq!(env.read("CLAUDE.md").as_deref(), Some(WRAPPER), "{label}");
        assert_eq!(env.read("GEMINI.md"), None, "{label}: no opt-in, no file");
        let again = env.apply(None, PlanOptions::default());
        assert!(matches!(again, ApplyOutcome::NoOp { .. }), "{label}");
    }
}

#[test]
fn the_gemini_opt_in_writes_the_wrapper_and_only_then() {
    let mut env = Env::new("gemini");
    env.write("CLAUDE.md", TEXT);
    env.write("GEMINI.md", TEXT);
    let options = PlanOptions {
        with_gemini: true,
        ..PlanOptions::default()
    };
    assert_eq!(applied(&env.apply(None, options)), 3);
    assert_eq!(env.read("AGENTS.md").as_deref(), Some(TEXT));
    assert_eq!(env.read("CLAUDE.md").as_deref(), Some(WRAPPER));
    assert_eq!(env.read("GEMINI.md").as_deref(), Some(WRAPPER));
    let again = env.apply(None, options);
    assert!(matches!(again, ApplyOutcome::NoOp { .. }));
}

#[test]
fn an_input_edited_between_two_writes_stops_the_run_as_stale() {
    let mut env = Env::new("between-writes");
    env.write("CLAUDE.md", TEXT);
    env.write("GEMINI.md", TEXT);
    let options = PlanOptions {
        with_gemini: true,
        ..PlanOptions::default()
    };
    let id = env.plan_id(None, options);
    let gemini = env.root.join("GEMINI.md");
    // Reads: 1 plans, 2 checks before the first write, 3 checks after
    // it, 4 checks before the second. The edit lands before read 4.
    let tree = HookedTree {
        inner: FsDefinitionTree::new(None),
        reads: Cell::new(0),
        hook: Some((
            4,
            Box::new(move || std::fs::write(&gemini, b"edited meanwhile\n").expect("edit")),
        )),
    };

    let error = env
        .apply_with(&tree, None, options, &id)
        .expect_err("the edit is noticed");
    assert_eq!(error.reason(), "stale-plan");
    assert_eq!(
        env.read("AGENTS.md").as_deref(),
        Some(TEXT),
        "first write stays"
    );
    assert_eq!(
        env.read("CLAUDE.md").as_deref(),
        Some(TEXT),
        "second never ran"
    );
    assert_eq!(
        env.read("GEMINI.md").as_deref(),
        Some(&b"edited meanwhile\n"[..])
    );
    assert_eq!(env.record_count(), 0);

    // The edit is new content nobody chose between: a rerun refuses it
    // as a conflict instead of overwriting it, and the written file stays.
    let rerun = env
        .apply_with(
            &FsDefinitionTree::new(None),
            None,
            options,
            &PlanId([0; 32]),
        )
        .expect("a refusal is an outcome");
    assert!(matches!(rerun, ApplyOutcome::Refused(_)), "{rerun:?}");
    assert_eq!(
        env.read("GEMINI.md").as_deref(),
        Some(&b"edited meanwhile\n"[..])
    );
}

#[test]
fn a_parents_apply_leaves_the_child_byte_identical_and_unsnapshotted() {
    let mut env = Env::new("nested");
    env.write("CLAUDE.md", TEXT);
    env.write("child/CLAUDE.md", b"child notes\n");
    assert_eq!(applied(&env.apply(Some("."), PlanOptions::default())), 2);
    assert_eq!(env.read("child/AGENTS.md"), None);
    assert_eq!(
        env.read("child/CLAUDE.md").as_deref(),
        Some(&b"child notes\n"[..])
    );
    let project = env.project();
    let manifests = env
        .snapshots
        .list_definitions(&project, None)
        .expect("list");
    assert!(manifests.iter().all(|m| m.subject.agent() == "."));

    // The child is adapted separately, with its own consent, and its
    // snapshots are filed under it.
    let consent = PlanOptions {
        nested_consent: true,
        ..PlanOptions::default()
    };
    assert_eq!(applied(&env.apply(Some("child"), consent)), 2);
    assert_eq!(env.read("child/CLAUDE.md").as_deref(), Some(WRAPPER));
    assert_eq!(
        env.snapshot_bytes("child", "CLAUDE.md").as_deref(),
        Some(&b"child notes\n"[..])
    );
    assert_eq!(env.read("CLAUDE.md").as_deref(), Some(WRAPPER));
}

#[test]
fn a_migration_keeps_the_text_outside_the_block_and_records_a_snapshot_and_a_record() {
    let mut env = Env::new("migration");
    let body = "## Local skill index\n\n| Skill | Use when | Path |\n| --- | --- | --- |\n\
                | sample | Sample skill. | [sample](skills/sample/SKILL.md) |\n";
    let legacy = format!(
        "<!-- portable-agent:skills:begin sha256={} -->\n{body}<!-- portable-agent:skills:end -->\n",
        Sha2Hasher.sha256(body.as_bytes()).to_hex()
    );
    let outside = "# Rules\r\n\r\nBe kind.\r\n\r\nSee notes.\n";
    let before = format!("{outside}\n{legacy}");
    env.write("AGENTS.md", before.as_bytes());
    env.write("CLAUDE.md", WRAPPER);
    env.write(
        "skills/sample/SKILL.md",
        b"---\nname: sample\ndescription: Sample skill.\n---\n",
    );

    let outcome = env.apply(None, PlanOptions::default());
    assert_eq!(applied(&outcome), 1);
    let after = String::from_utf8(env.read("AGENTS.md").expect("AGENTS.md")).expect("utf-8");
    assert!(after.starts_with(&format!("{outside}\n")), "{after:?}");
    assert!(!after.contains("portable-agent:"), "{after:?}");
    assert!(
        after.contains("[sample](skills/sample/SKILL.md)"),
        "{after:?}"
    );
    assert_eq!(
        env.snapshot_bytes(".", "AGENTS.md").as_deref(),
        Some(before.as_bytes())
    );
    assert_eq!((env.snapshot_count(), env.record_count()), (1, 1));
    assert_eq!(env.read("CLAUDE.md").as_deref(), Some(WRAPPER));
    let again = env.apply(None, PlanOptions::default());
    assert!(matches!(again, ApplyOutcome::NoOp { .. }), "{again:?}");
}

#[cfg(unix)]
#[test]
fn file_modes_are_never_widened() {
    use std::os::unix::fs::PermissionsExt;
    let mut env = Env::new("modes");
    env.write("CLAUDE.md", TEXT);
    std::fs::set_permissions(
        env.root.join("CLAUDE.md"),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("chmod");
    env.apply(None, PlanOptions::default());
    let mode = |name: &str| {
        std::fs::metadata(env.root.join(name))
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(
        mode("AGENTS.md"),
        0o600,
        "a created file takes its source's bits"
    );
    assert_eq!(mode("CLAUDE.md") & !0o600, 0, "the wrapper is not wider");
}
