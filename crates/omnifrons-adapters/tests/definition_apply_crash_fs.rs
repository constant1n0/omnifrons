//! Crash-point convergence for `definition::apply` (ADR-0005 sub-slice
//! 2e) over the real adapters on temporary directories.
//!
//! The apply is not one transaction: it publishes each planned file
//! atomically, one after another, and stores its record last. These
//! tests stop a run before every write in turn (a `CrashingWriter`
//! models a process death between write N-1 and write N) and prove two
//! things for each crash point: the directory is left in a recognised
//! intermediate state, with no record and exactly the snapshots taken so
//! far; and planning again and applying that new plan converges to the
//! bytes an uncrashed run produces.

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
use omnifrons_app::definition::{DefinitionError, DefinitionPorts, VerifyReport, verify};
use omnifrons_app::definition_apply::{ApplyOutcome, ApplyPorts, apply};
use omnifrons_app::definition_plan::{PlanOutcome, plan};
use omnifrons_app::definition_tree::DefinitionTree;
use omnifrons_app::definition_writer::{DefinitionWriteError, DefinitionWriter};
use omnifrons_app::snapshot_store::DefinitionSnapshotStore;
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::definition::{IdentityKind, RootState, WRAPPER};
use omnifrons_domain::definition_plan::{PlanId, PlanOptions};
use omnifrons_domain::publication::ProjectIdentity;

const TEXT: &[u8] = b"# Rules\n\nBe kind.\n";
const OTHER: &[u8] = b"# Other\n\nBe brief.\n";
const SKILL: &[u8] = b"---\nname: sample\ndescription: Sample skill.\n---\n";

/// A drop-guard temp directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-definition-apply-crash-test-{}-{label}-{n}",
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

/// A clock that never moves: every snapshot shares one instant.
struct StepClock(u64);

impl Clock for StepClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(self.0)
    }
}

/// A workspace and a work area outside it, with the real stores.
struct Env {
    _workspace_dir: TempDir,
    _base: TempDir,
    root: PathBuf,
    workspace: WorkspaceRoot,
    snapshots: FsSnapshotStore,
    records: FsApplyRecordStore,
    clock: StepClock,
}

impl Env {
    fn new(label: &str, files: &[(&str, &[u8])]) -> Self {
        let workspace_dir = TempDir::new(label);
        let base = TempDir::new(&format!("{label}-base"));
        let workspace = WorkspaceRoot::new(workspace_dir.path()).expect("workspace");
        let work_area =
            WorkAreaRoot::open(&base.path().join("wa"), &[&workspace]).expect("work area");
        let env = Self {
            root: workspace.path().to_path_buf(),
            snapshots: FsSnapshotStore::open(&work_area),
            records: FsApplyRecordStore::open(&work_area),
            clock: StepClock(5_000),
            workspace,
            _workspace_dir: workspace_dir,
            _base: base,
        };
        for (name, bytes) in files {
            let path = env.root.join(name);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("parent dir");
            std::fs::write(path, bytes).expect("fixture file");
        }
        env
    }

    fn read(&self, relative: &str) -> Option<Vec<u8>> {
        std::fs::read(self.root.join(relative)).ok()
    }

    fn project(&self) -> ProjectIdentity {
        derive_project_identity(&Sha2Hasher, &self.workspace)
    }

    /// The id the plan from the disk as it is now carries.
    fn plan_id(&self, options: PlanOptions) -> PlanId {
        let tree = FsDefinitionTree::new(None);
        let ports = DefinitionPorts {
            tree: &tree,
            hasher: &Sha2Hasher,
        };
        match plan(&ports, &self.workspace, None, &options).expect("plan") {
            PlanOutcome::Planned(plan) => plan.id,
            PlanOutcome::Refused(refusal) => panic!("expected a plan, got {refusal}"),
        }
    }

    /// Plans from the disk as it is, then applies that plan through
    /// `writer`.
    fn apply_through(
        &mut self,
        writer: &dyn DefinitionWriter,
        options: PlanOptions,
    ) -> Result<ApplyOutcome, DefinitionError> {
        let id = self.plan_id(options);
        let tree = FsDefinitionTree::new(None);
        let mut ports = ApplyPorts {
            tree: &tree,
            hasher: &Sha2Hasher,
            writer,
            snapshots: &mut self.snapshots,
            records: &self.records,
            clock: &self.clock,
        };
        apply(&mut ports, &self.workspace, None, &options, &id)
    }

    /// The scan root's verification, read from the disk as it is.
    fn verify(&self) -> VerifyReport {
        let tree: &dyn DefinitionTree = &FsDefinitionTree::new(None);
        let ports = DefinitionPorts {
            tree,
            hasher: &Sha2Hasher,
        };
        verify(&ports, &self.workspace, ".").expect("verify")
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
}

/// What one fixture is expected to show at each crash point.
struct Expect {
    /// The state after a crash before write 1, 2, ... in turn.
    states: &'static [RootState],
    /// Whether the verification reports a pending block after that crash
    /// (a block fixture reports it at every crash point: it is written
    /// last).
    block_pending: &'static [bool],
    /// The snapshots on disk after that crash: a snapshot precedes each
    /// write of a file that exists, so the crashed write's own snapshot
    /// is included, and a file that was absent at plan time has none.
    snapshots: &'static [usize],
    /// The snapshots after the rerun converged.
    snapshots_after_rerun: &'static [usize],
}

/// What an uncrashed run of a fixture produced.
struct Reference {
    writes: usize,
    agents: Vec<u8>,
    claude: Vec<u8>,
    snapshots: usize,
}

fn uncrashed(label: &str, files: &[(&str, &[u8])], options: PlanOptions) -> Reference {
    let mut env = Env::new(&format!("{label}-reference"), files);
    let writer = CrashingWriter::counting(FsDefinitionWriter);
    let outcome = env.apply_through(&writer, options).expect("apply");
    assert!(
        matches!(outcome, ApplyOutcome::Applied { .. }),
        "{outcome:?}"
    );
    assert!(!writer.crashed());
    let report = env.verify();
    assert!(report.canonical, "{:?}", report.problems);
    assert_eq!(env.record_count(), 1);
    Reference {
        writes: writer.writes_performed(),
        agents: env.read("AGENTS.md").expect("AGENTS.md"),
        claude: env.read("CLAUDE.md").expect("CLAUDE.md"),
        snapshots: env.snapshot_count(),
    }
}

fn has_pending_block(report: &VerifyReport) -> bool {
    report
        .problems
        .iter()
        .any(|problem| problem.starts_with("pending:"))
}

/// Crashes the apply of `files` before every write in turn and checks
/// each intermediate state and the rerun that converges from it. Returns
/// the reference run's write count.
fn crash_before_every_write(
    label: &str,
    files: &[(&str, &[u8])],
    options: PlanOptions,
    expect: &Expect,
    observed: &mut Vec<RootState>,
) -> usize {
    let reference = uncrashed(label, files, options);
    for (name, len) in [
        ("states", expect.states.len()),
        ("block_pending", expect.block_pending.len()),
        ("snapshots", expect.snapshots.len()),
        ("snapshots_after_rerun", expect.snapshots_after_rerun.len()),
    ] {
        assert_eq!(len, reference.writes, "{label}: {name} table size");
    }
    for crash_point in 1..=reference.writes {
        let at = format!("{label}: crash before write {crash_point}");
        let mut env = Env::new(&format!("{label}-{crash_point}"), files);
        let crashing = CrashingWriter::crashing_before(
            FsDefinitionWriter,
            NonZeroUsize::new(crash_point).expect("crash points start at 1"),
        );
        let error = env
            .apply_through(&crashing, options)
            .expect_err("the simulated crash stops the run");
        assert!(
            matches!(
                error,
                DefinitionError::Write(DefinitionWriteError::Io { .. })
            ),
            "{at}: {error:?}"
        );
        assert!(crashing.crashed(), "{at}");
        assert_eq!(crashing.writes_performed(), crash_point - 1, "{at}");

        // A recognised intermediate state, no record, the snapshots so far.
        let report = env.verify();
        assert_eq!(report.state, expect.states[crash_point - 1], "{at}");
        assert_eq!(
            has_pending_block(&report),
            expect.block_pending[crash_point - 1],
            "{at}: {:?}",
            report.problems
        );
        observed.push(report.state);
        assert_eq!(env.record_count(), 0, "{at}");
        assert_eq!(
            env.snapshot_count(),
            expect.snapshots[crash_point - 1],
            "{at}"
        );

        // A rerun plans from what is on disk and converges. The caller
        // names the winning side only while the files still disagree:
        // the planner refuses `canonical_from` for any other state.
        let rerun = if report.state == RootState::Conflict {
            options
        } else {
            PlanOptions::default()
        };
        let outcome = env
            .apply_through(&FsDefinitionWriter, rerun)
            .unwrap_or_else(|error| panic!("{at}: rerun failed: {error:?}"));
        assert!(
            matches!(outcome, ApplyOutcome::Applied { .. }),
            "{at}: {outcome:?}"
        );
        let report = env.verify();
        assert_eq!(report.state, RootState::Canonical, "{at}");
        assert!(report.canonical, "{at}: {:?}", report.problems);
        assert_eq!(
            env.read("AGENTS.md"),
            Some(reference.agents.clone()),
            "{at}"
        );
        assert_eq!(
            env.read("CLAUDE.md"),
            Some(reference.claude.clone()),
            "{at}"
        );
        assert_eq!(env.record_count(), 1, "{at}: only the rerun is recorded");
        assert_eq!(
            env.snapshot_count(),
            expect.snapshots_after_rerun[crash_point - 1],
            "{at}"
        );
        // Converged: a further apply has nothing to do.
        let again = env
            .apply_through(&FsDefinitionWriter, PlanOptions::default())
            .expect("apply again");
        assert!(
            matches!(again, ApplyOutcome::NoOp { .. }),
            "{at}: {again:?}"
        );
    }
    assert!(reference.snapshots > 0);
    reference.writes
}

fn canonical_from_claude() -> PlanOptions {
    PlanOptions {
        canonical_from: Some(IdentityKind::Claude),
        ..PlanOptions::default()
    }
}

#[test]
fn every_crash_point_of_a_claude_only_and_a_conflict_apply_converges() {
    let mut observed = Vec::new();
    let claude_only = crash_before_every_write(
        "claude-only",
        &[("CLAUDE.md", TEXT)],
        PlanOptions::default(),
        &Expect {
            states: &[RootState::ClaudeOnly, RootState::IdenticalCopies],
            block_pending: &[false, false],
            snapshots: &[0, 1],
            snapshots_after_rerun: &[1, 1],
        },
        &mut observed,
    );
    assert_eq!(claude_only, 2);

    let conflict = crash_before_every_write(
        "conflict",
        &[("AGENTS.md", TEXT), ("CLAUDE.md", OTHER)],
        canonical_from_claude(),
        &Expect {
            states: &[RootState::Conflict, RootState::IdenticalCopies],
            block_pending: &[false, false],
            snapshots: &[1, 2],
            snapshots_after_rerun: &[2, 2],
        },
        &mut observed,
    );
    assert_eq!(conflict, 2);

    // The prototype's assertions: at least three writes in all, and the
    // identical-copies state is seen at least once.
    assert!(claude_only + conflict >= 3);
    assert!(observed.contains(&RootState::IdenticalCopies));
}

#[test]
fn a_crash_before_the_block_write_leaves_the_root_canonical_with_the_block_pending() {
    let files: &[(&str, &[u8])] = &[("CLAUDE.md", TEXT), ("skills/sample/SKILL.md", SKILL)];
    let mut observed = Vec::new();
    let writes = crash_before_every_write(
        "block",
        files,
        PlanOptions::default(),
        &Expect {
            states: &[
                RootState::ClaudeOnly,
                RootState::IdenticalCopies,
                RootState::Canonical,
            ],
            block_pending: &[true, true, true],
            snapshots: &[0, 1, 1],
            snapshots_after_rerun: &[1, 2, 2],
        },
        &mut observed,
    );
    // AGENTS.md, the CLAUDE.md wrapper, then the block: the last write.
    assert_eq!(writes, 3);

    // After the last crash the identity files are canonical and the
    // block is the only thing left to do; the rerun plans exactly that.
    let mut env = Env::new("block-rerun", files);
    let crashing = CrashingWriter::crashing_before(
        FsDefinitionWriter,
        NonZeroUsize::new(writes).expect("at least one write"),
    );
    env.apply_through(&crashing, PlanOptions::default())
        .expect_err("crash before the block");
    assert_eq!(env.read("CLAUDE.md").as_deref(), Some(WRAPPER));
    assert_eq!(env.read("AGENTS.md").as_deref(), Some(TEXT));
    let report = env.verify();
    assert_eq!(report.state, RootState::Canonical);
    assert_eq!(report.problems.len(), 1, "{:?}", report.problems);
    assert!(report.problems[0].starts_with("pending:"));
    let counting = CrashingWriter::counting(FsDefinitionWriter);
    env.apply_through(&counting, PlanOptions::default())
        .expect("rerun");
    assert_eq!(counting.writes_performed(), 1);
    let agents = String::from_utf8(env.read("AGENTS.md").expect("AGENTS.md")).expect("utf-8");
    assert!(agents.contains("skills/sample/SKILL.md"), "{agents:?}");
}
