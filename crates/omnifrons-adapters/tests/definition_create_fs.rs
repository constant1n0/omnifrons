//! A create (ADR-0005 decision 6, sub-slice 3c) end to end over the real
//! adapters on temporary directories: the filesystem tree, writer,
//! snapshot store and apply-record store, with the real SHA-256 hasher.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use omnifrons_adapters::{
    FsApplyRecordStore, FsDefinitionTree, FsDefinitionWriter, FsSnapshotStore, Sha2Hasher,
};
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::apply_record::ApplyRecordStore;
use omnifrons_app::clock::Clock;
use omnifrons_app::content_hasher::derive_project_identity;
use omnifrons_app::contract::definition_writer::CrashingWriter;
use omnifrons_app::definition::{DefinitionError, DefinitionPorts, verify};
use omnifrons_app::definition_apply::{ApplyOutcome, ApplyPorts, apply_create};
use omnifrons_app::definition_plan::{PlanOutcome, plan_create};
use omnifrons_app::definition_tree::DefinitionTreeError;
use omnifrons_app::definition_writer::DefinitionWriter;
use omnifrons_app::snapshot_store::DefinitionSnapshotStore;
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::definition::WRAPPER;
use omnifrons_domain::definition_create::{Answers, CreateContext, RawValue, render};
use omnifrons_domain::definition_plan::{Operation, PlanId, PlanOptions, PlanRefusal};
use omnifrons_domain::publication::ProjectIdentity;

const SKILL: &[u8] = b"---\nname: triage\ndescription: Sorts incoming requests.\n---\n";

/// A drop-guard temp directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-definition-create-fs-test-{}-{label}-{n}",
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

struct StepClock;

impl Clock for StepClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(5_000)
    }
}

fn text(value: &str) -> RawValue {
    RawValue::Text(value.to_string())
}

fn item(body: &str) -> RawValue {
    RawValue::Object(vec![
        ("text".to_string(), text(body)),
        ("said".to_string(), text("Said.")),
        ("status".to_string(), text("confirmed")),
    ])
}

fn answers_pairs() -> Vec<(String, RawValue)> {
    answers_pairs_with("a scheduling assistant")
}

fn answers_pairs_with(role: &str) -> Vec<(String, RawValue)> {
    let pairs = vec![
        ("name", item("Harbor")),
        ("role", item(role)),
        ("deliverables", RawValue::List(vec![item("Propose slots.")])),
        (
            "authority",
            RawValue::List(vec![item("Draft and advise only.")]),
        ),
        ("work_language", item("the language of each user")),
    ];
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn answers_with(role: &str) -> Answers {
    Answers::validate(&RawValue::Object(answers_pairs_with(role))).expect("valid answers")
}

fn answers() -> Answers {
    answers_with("a scheduling assistant")
}

fn body() -> String {
    render(
        &answers(),
        &CreateContext::new(Vec::new(), false).expect("context"),
    )
}

/// A workspace and a work area outside it, with the real stores.
struct Env {
    _workspace_dir: TempDir,
    _base: TempDir,
    root: PathBuf,
    workspace: WorkspaceRoot,
    snapshots: FsSnapshotStore,
    records: FsApplyRecordStore,
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

    /// Every entry of the workspace, as sorted relative paths.
    fn listing(&self) -> Vec<String> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("read dir").flatten() {
                let path = entry.path();
                // Joined with `/` on every platform, so the listings the
                // tests pin read the same on Windows.
                out.push(
                    path.strip_prefix(base)
                        .unwrap()
                        .components()
                        .map(|component| component.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/"),
                );
                if entry.file_type().expect("type").is_dir() {
                    walk(&path, base, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &self.root, &mut out);
        out.sort();
        out
    }

    fn project(&self) -> ProjectIdentity {
        derive_project_identity(&Sha2Hasher, &self.workspace)
    }

    fn plan(&self, target: &str, answers: &Answers) -> PlanOutcome {
        self.plan_under(&FsDefinitionTree::new(None), target, answers)
    }

    fn plan_under(&self, tree: &FsDefinitionTree, target: &str, answers: &Answers) -> PlanOutcome {
        let ports = DefinitionPorts {
            tree,
            hasher: &Sha2Hasher,
        };
        let options = PlanOptions::default();
        plan_create(&ports, &self.workspace, Some(target), &options, answers).expect("plan")
    }

    fn plan_id(&self, target: &str, answers: &Answers) -> PlanId {
        match self.plan(target, answers) {
            PlanOutcome::Planned(plan) => plan.id,
            PlanOutcome::Refused(refusal) => panic!("expected a plan, got {refusal}"),
        }
    }

    fn apply_through(
        &mut self,
        writer: &dyn DefinitionWriter,
        target: &str,
        answers: &Answers,
        id: &PlanId,
    ) -> Result<ApplyOutcome, DefinitionError> {
        let tree = FsDefinitionTree::new(None);
        let mut ports = ApplyPorts {
            tree: &tree,
            hasher: &Sha2Hasher,
            writer,
            snapshots: &mut self.snapshots,
            records: &self.records,
            clock: &StepClock,
        };
        let options = PlanOptions::default();
        apply_create(
            &mut ports,
            &self.workspace,
            Some(target),
            &options,
            answers,
            id,
        )
    }

    /// Plans from the disk as it is, then applies that plan.
    fn create(&mut self, target: &str, answers: &Answers) -> ApplyOutcome {
        let id = self.plan_id(target, answers);
        self.apply_through(&FsDefinitionWriter, target, answers, &id)
            .expect("apply")
    }

    fn record_count(&self) -> usize {
        self.records.list(&self.project()).expect("records").len()
    }

    fn snapshot_count(&self) -> usize {
        self.snapshots
            .list_definitions(&self.project(), None)
            .expect("snapshots")
            .len()
    }

    fn assert_canonical(&self, target: &str) {
        let tree = FsDefinitionTree::new(None);
        let ports = DefinitionPorts {
            tree: &tree,
            hasher: &Sha2Hasher,
        };
        let report = verify(&ports, &self.workspace, target).expect("verify");
        assert!(report.canonical, "{:?}", report.problems);
    }
}

fn applied_files(outcome: &ApplyOutcome) -> Vec<String> {
    match outcome {
        ApplyOutcome::Applied { files, .. } => files.iter().map(|f| f.path.clone()).collect(),
        other => panic!("expected Applied, got {other:?}"),
    }
}

fn refused(outcome: PlanOutcome) -> PlanRefusal {
    match outcome {
        PlanOutcome::Refused(refusal) => refusal,
        PlanOutcome::Planned(plan) => panic!("planned: {plan:?}"),
    }
}

#[test]
fn a_create_writes_exactly_agents_md_and_the_wrapper() {
    let mut env = Env::new("plain");
    let outcome = env.create(".", &answers());
    assert_eq!(applied_files(&outcome), ["AGENTS.md", "CLAUDE.md"]);
    assert_eq!(env.listing(), ["AGENTS.md", "CLAUDE.md"]);
    assert_eq!(env.read("AGENTS.md").as_deref(), Some(body().as_bytes()));
    assert_eq!(env.read("CLAUDE.md").as_deref(), Some(WRAPPER));
    assert_eq!((env.record_count(), env.snapshot_count()), (1, 0));
    env.assert_canonical(".");
}

#[test]
fn a_folder_with_a_skill_gets_the_body_then_the_current_block() {
    let mut env = Env::new("skill");
    env.write("skills/triage/SKILL.md", SKILL);
    env.create(".", &answers());
    let agents = String::from_utf8(env.read("AGENTS.md").expect("AGENTS.md")).expect("utf-8");
    let rest = agents.strip_prefix(&body()).expect("the body comes first");
    assert!(rest.starts_with("\n<!-- omnifrons:begin skills "), "{rest}");
    assert!(
        rest.contains("| triage | Sorts incoming requests. |"),
        "{rest}"
    );
    assert!(rest.ends_with("<!-- omnifrons:end skills -->\n"), "{rest}");
    assert_eq!(
        env.listing(),
        [
            "AGENTS.md",
            "CLAUDE.md",
            "skills",
            "skills/triage",
            "skills/triage/SKILL.md"
        ]
    );
    env.assert_canonical(".");
}

#[test]
fn a_create_in_a_subfolder_leaves_the_rest_alone() {
    let mut env = Env::new("sub");
    env.write("notes/keep.md", b"keep\n");
    std::fs::create_dir_all(env.root.join("team")).expect("team dir");
    env.create("team", &answers());
    assert_eq!(
        env.read("team/AGENTS.md").as_deref(),
        Some(body().as_bytes())
    );
    assert_eq!(env.read("team/CLAUDE.md").as_deref(), Some(WRAPPER));
    assert_eq!(env.read("notes/keep.md").as_deref(), Some(&b"keep\n"[..]));
}

#[test]
fn a_dry_run_writes_nothing_and_saves_nothing() {
    let env = Env::new("dry");
    env.write("skills/triage/SKILL.md", SKILL);
    let before = env.listing();
    let PlanOutcome::Planned(plan) = env.plan(".", &answers()) else {
        panic!("expected a plan");
    };
    assert!(matches!(
        plan.operations[0],
        Operation::WriteGenerated { .. }
    ));
    assert_eq!(env.listing(), before);
    assert_eq!((env.record_count(), env.snapshot_count()), (0, 0));
    // The same plan twice is the same plan: nothing was remembered.
    assert_eq!(plan.id, env.plan_id(".", &answers()));
}

#[test]
fn an_apply_needs_the_current_plan_id() {
    let mut env = Env::new("stale-id");
    let wrong = PlanId([7; 32]);
    let result = env.apply_through(&FsDefinitionWriter, ".", &answers(), &wrong);
    assert!(
        matches!(result, Err(DefinitionError::StalePlan { .. })),
        "{result:?}"
    );
    assert!(env.listing().is_empty());
    assert_eq!(env.record_count(), 0);
}

#[test]
fn a_rerun_with_the_same_answers_is_a_no_op() {
    let mut env = Env::new("rerun");
    env.create(".", &answers());
    let outcome = env.create(".", &answers());
    assert!(matches!(outcome, ApplyOutcome::NoOp { .. }), "{outcome:?}");
    assert_eq!(env.record_count(), 1);
    assert_eq!(env.read("AGENTS.md").as_deref(), Some(body().as_bytes()));
}

#[test]
fn a_rerun_with_different_answers_is_an_existing_agent() {
    let mut env = Env::new("different");
    env.create(".", &answers());
    let other = answers_with("a different assistant");
    let refusal = refused(env.plan(".", &other));
    assert_eq!(refusal.code(), "identity_exists");
    assert!(refusal.message().contains("render"), "{refusal}");
    let id = env.plan_id(".", &answers());
    let applied = env
        .apply_through(&FsDefinitionWriter, ".", &other, &id)
        .expect("a refusal is an outcome");
    assert!(
        matches!(
            applied,
            ApplyOutcome::Refused(PlanRefusal::IdentityExists(_))
        ),
        "{applied:?}"
    );
    assert_eq!(env.read("AGENTS.md").as_deref(), Some(body().as_bytes()));
    assert_eq!(env.record_count(), 1);
}

#[test]
fn a_crash_after_agents_md_converges_on_the_rerun() {
    let mut env = Env::new("crash");
    let id = env.plan_id(".", &answers());
    let writer = CrashingWriter::crashing_before(
        FsDefinitionWriter,
        NonZeroUsize::new(2).expect("non-zero"),
    );
    let result = env.apply_through(&writer, ".", &answers(), &id);
    assert!(
        matches!(result, Err(DefinitionError::Write(_))),
        "{result:?}"
    );
    assert!(writer.crashed());
    assert_eq!(env.listing(), ["AGENTS.md"]);
    assert_eq!(env.record_count(), 0);

    // The rerun finds this writer's own file and plans the wrapper only.
    let PlanOutcome::Planned(plan) = env.plan(".", &answers()) else {
        panic!("expected a plan");
    };
    assert_eq!(plan.operations.len(), 1);
    assert!(matches!(plan.operations[0], Operation::WriteWrapper { .. }));
    let outcome = env.create(".", &answers());
    assert_eq!(applied_files(&outcome), ["CLAUDE.md"]);
    assert_eq!(env.read("AGENTS.md").as_deref(), Some(body().as_bytes()));
    assert_eq!(env.read("CLAUDE.md").as_deref(), Some(WRAPPER));
    assert_eq!((env.record_count(), env.snapshot_count()), (1, 0));
    env.assert_canonical(".");
}

#[test]
fn files_appearing_between_plan_and_apply_change_nothing() {
    let mut env = Env::new("appeared");
    let id = env.plan_id(".", &answers());
    env.write("AGENTS.md", body().as_bytes());
    let result = env.apply_through(&FsDefinitionWriter, ".", &answers(), &id);
    // The plan is recomputed from the disk as it is now: its id differs.
    assert!(
        matches!(result, Err(DefinitionError::StalePlan { .. })),
        "{result:?}"
    );
    assert_eq!(env.listing(), ["AGENTS.md"]);
    assert_eq!(env.record_count(), 0);

    // A file of someone else's makes the folder an existing agent.
    env.write("CLAUDE.md", b"# Theirs\n");
    let result = env.apply_through(&FsDefinitionWriter, ".", &answers(), &id);
    assert!(
        matches!(
            result,
            Ok(ApplyOutcome::Refused(PlanRefusal::IdentityExists(_)))
        ),
        "{result:?}"
    );
    assert_eq!(env.listing(), ["AGENTS.md", "CLAUDE.md"]);
    assert_eq!(env.read("CLAUDE.md").as_deref(), Some(&b"# Theirs\n"[..]));
    assert_eq!(env.record_count(), 0);
}

#[test]
fn a_symlinked_target_is_refused_and_nothing_is_written() {
    let env = Env::new("symlink");
    std::fs::create_dir_all(env.root.join("real")).expect("real dir");
    #[cfg(unix)]
    std::os::unix::fs::symlink("real", env.root.join("link")).expect("symlink");
    #[cfg(unix)]
    {
        let before = env.listing();
        let refusal = refused(env.plan("link", &answers()));
        assert_eq!(refusal.code(), "unsafe_target");
        assert!(refusal.message().contains("symlink"), "{refusal}");
        assert_eq!(env.listing(), before);
    }
}

#[test]
fn a_host_configuration_target_is_refused_at_any_depth() {
    let env = Env::new("host");
    for target in [".claude", ".claude/inner", "work/.cursor/rules"] {
        std::fs::create_dir_all(env.root.join(target)).expect("dir");
        let refusal = refused(env.plan(target, &answers()));
        assert_eq!(refusal.code(), "unsafe_target", "{target}");
        assert!(
            refusal.message().contains("host configuration folder"),
            "{target}: {refusal}"
        );
    }
}

#[test]
fn a_target_below_a_pruned_directory_is_refused_and_nothing_is_written() {
    let env = Env::new("pruned");
    std::fs::create_dir_all(env.root.join(".git/hooks")).expect("dir");
    let before = env.listing();
    let refusal = refused(env.plan(".git/hooks", &answers()));
    assert_eq!(refusal.code(), "unsafe_target");
    assert!(refusal.message().contains(".git/"), "{refusal}");
    assert_eq!(env.listing(), before);
    assert!(!env.root.join(".git/hooks/AGENTS.md").exists());
}

#[cfg(unix)]
#[test]
fn a_target_through_a_symlinked_folder_is_refused_and_nothing_is_written() {
    let env = Env::new("linked-parent");
    std::fs::create_dir_all(env.root.join("real/child")).expect("dirs");
    std::os::unix::fs::symlink("real", env.root.join("alias")).expect("symlink");
    let before = env.listing();
    let refusal = refused(env.plan("alias/child", &answers()));
    assert_eq!(refusal.code(), "unsafe_target");
    assert!(refusal.message().contains("alias"), "{refusal}");
    assert_eq!(env.listing(), before);
    // The same folder by its real path still plans.
    let PlanOutcome::Planned(_) = env.plan("real/child", &answers()) else {
        panic!("the real path plans");
    };
}

#[test]
fn the_home_directory_is_refused() {
    let env = Env::new("home");
    std::fs::create_dir_all(env.root.join("me")).expect("home dir");
    // Home is given by a path that is not canonical: through a symlink and
    // with a `.` component. The adapter canonicalises it when it learns it.
    #[cfg(unix)]
    std::os::unix::fs::symlink("me", env.root.join("alias")).expect("symlink");
    #[cfg(unix)]
    let home = env.root.join("alias").join(".");
    #[cfg(not(unix))]
    let home = env.root.join("me");
    let tree = FsDefinitionTree::new(Some(home));
    let refusal = refused(env.plan_under(&tree, "me", &answers()));
    assert_eq!(refusal.code(), "unsafe_target");
    assert!(refusal.message().contains("home directory"), "{refusal}");
    // Another folder is fine under the same home.
    std::fs::create_dir_all(env.root.join("me/project")).expect("project dir");
    let PlanOutcome::Planned(_) = env.plan_under(&tree, "me/project", &answers()) else {
        panic!("a folder below home is allowed");
    };
}

#[test]
fn a_nonexistent_target_is_a_missing_target_and_nothing_is_written() {
    let env = Env::new("missing");
    let tree = FsDefinitionTree::new(None);
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &Sha2Hasher,
    };
    let result = plan_create(
        &ports,
        &env.workspace,
        Some("nowhere"),
        &PlanOptions::default(),
        &answers(),
    );
    assert_eq!(
        result,
        Err(DefinitionError::Tree(DefinitionTreeError::TargetMissing))
    );
    assert!(env.listing().is_empty());
}

#[test]
fn a_source_that_leaves_the_workspace_is_refused_though_it_exists() {
    let base = TempDir::new("sibling-base");
    let outside = base.path().join("outside.md");
    std::fs::write(&outside, b"secret").expect("sibling file");
    let workspace_dir = base.path().join("ws");
    std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
    let workspace = WorkspaceRoot::new(&workspace_dir).expect("workspace");
    let tree = FsDefinitionTree::new(None);
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &Sha2Hasher,
    };
    let mut pairs = answers_pairs();
    pairs.push((
        "sources".to_string(),
        RawValue::List(vec![item("../outside.md")]),
    ));
    let answers = Answers::validate(&RawValue::Object(pairs)).expect("valid answers");
    let outcome =
        plan_create(&ports, &workspace, None, &PlanOptions::default(), &answers).expect("plan");
    let refusal = refused(outcome);
    assert_eq!(refusal.code(), "invalid_source");
    assert!(refusal.message().contains("../outside.md"), "{refusal}");
}
