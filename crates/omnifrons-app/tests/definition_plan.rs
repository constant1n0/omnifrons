//! The read-only `plan` service (ADR-0005 sub-slice 2a) against the
//! in-memory `DefinitionTree` fake: the plan id, the nested-agent guard,
//! the conflict and unresolved-file refusals, unsafe inputs, and the
//! identity-file operations of each root state.
//!
//! Only compiled with `--features contract-tests`: the fake lives behind
//! that feature.

#![cfg(feature = "contract-tests")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::content_hasher::ContentHasher;
use omnifrons_app::contract::definition::{InMemoryDefinitionTree, TreeSnapshotBuilder};
use omnifrons_app::contract::publication::FakeHasher;
use omnifrons_app::definition::{DefinitionError, DefinitionPorts};
use omnifrons_app::definition_plan::{PlanOutcome, plan};
use omnifrons_app::definition_tree::TreeSnapshot;
use omnifrons_domain::definition::{FileShape, IdentityFile, IdentityKind, WRAPPER};
use omnifrons_domain::definition_plan::{
    InputFingerprint, Operation, Plan, PlanOptions, PlanRefusal,
};
use omnifrons_domain::guidance::{LineEnding, ManagedBlock};

/// A drop-guard temp directory standing in for a workspace.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-definition-plan-test-{}-{label}-{n}",
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

const AGENTS_TEXT: &[u8] = b"# Sample\n\nYou are Sample, a fixture agent.\n";
const OTHER_TEXT: &[u8] = b"# Sample\n\nYou are a different Sample.\n";

fn identity(kind: IdentityKind, bytes: &[u8]) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: bytes.to_vec(),
        shape: FileShape::Regular,
    }
}

fn symlinked(kind: IdentityKind, target: &str) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: Vec::new(),
        shape: FileShape::Symlink {
            target: target.to_string(),
        },
    }
}

fn irregular(kind: IdentityKind) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: Vec::new(),
        shape: FileShape::Irregular,
    }
}

/// A snapshot with one root directory `.` holding `files`.
fn single_root(files: Vec<IdentityFile>) -> TreeSnapshot {
    TreeSnapshotBuilder::new()
        .directory(".", files, Vec::new(), Vec::new())
        .build()
}

/// Plans `target` over `snapshot` with `options`.
fn run(
    snapshot: TreeSnapshot,
    target: Option<&str>,
    options: PlanOptions,
) -> Result<PlanOutcome, DefinitionError> {
    let project = TempDir::new("run");
    let workspace = project.workspace();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };
    plan(&ports, &workspace, target, &options)
}

fn planned(outcome: Result<PlanOutcome, DefinitionError>) -> Plan {
    match outcome.expect("plan succeeds") {
        PlanOutcome::Planned(plan) => plan,
        PlanOutcome::Refused(refusal) => panic!("expected a plan, got {refusal}"),
    }
}

fn refused(outcome: Result<PlanOutcome, DefinitionError>) -> PlanRefusal {
    match outcome.expect("plan succeeds") {
        PlanOutcome::Refused(refusal) => refusal,
        PlanOutcome::Planned(plan) => panic!("expected a refusal, got {:?}", plan.operations),
    }
}

fn digest_of(bytes: &[u8]) -> InputFingerprint {
    InputFingerprint::Sha256(FakeHasher.sha256(bytes))
}

fn paths(plan: &Plan) -> Vec<&'static str> {
    plan.operations.iter().map(Operation::path).collect()
}

fn claude_only() -> TreeSnapshot {
    single_root(vec![identity(IdentityKind::Claude, AGENTS_TEXT)])
}

#[test]
fn plan_id_is_stable_across_runs_and_tracks_inputs_and_options() {
    let first = planned(run(claude_only(), None, PlanOptions::default()));
    let second = planned(run(claude_only(), None, PlanOptions::default()));
    assert_eq!(first.id, second.id);
    assert_eq!(first.id.to_string().len(), 64);

    let edited = single_root(vec![identity(IdentityKind::Claude, OTHER_TEXT)]);
    let edited = planned(run(edited, None, PlanOptions::default()));
    assert_ne!(first.id, edited.id);

    let with_gemini = PlanOptions {
        with_gemini: true,
        ..PlanOptions::default()
    };
    let gemini = planned(run(claude_only(), None, with_gemini));
    assert_ne!(first.id, gemini.id);
}

#[test]
fn plan_id_changes_when_an_untouched_input_changes() {
    let base = single_root(vec![
        identity(IdentityKind::Agents, AGENTS_TEXT),
        identity(IdentityKind::Claude, WRAPPER),
        identity(IdentityKind::Gemini, b"@AGENTS.md\n"),
    ]);
    let changed = single_root(vec![
        identity(IdentityKind::Agents, AGENTS_TEXT),
        identity(IdentityKind::Claude, WRAPPER),
        identity(IdentityKind::Gemini, b"@AGENTS.md  \n"),
    ]);
    let a = planned(run(base, None, PlanOptions::default()));
    let b = planned(run(changed, None, PlanOptions::default()));
    assert!(a.operations.is_empty() && b.operations.is_empty());
    assert_ne!(a.id, b.id);
}

#[test]
fn a_directory_without_identity_files_plans_nothing() {
    let outcome = planned(run(single_root(Vec::new()), None, PlanOptions::default()));
    assert!(outcome.operations.is_empty());
    assert!(outcome.inputs.is_empty());
    assert_eq!(outcome.agent, ".");
}

fn nested_snapshot() -> TreeSnapshot {
    TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![identity(IdentityKind::Agents, AGENTS_TEXT)],
            Vec::new(),
            Vec::new(),
        )
        .directory(
            "child",
            vec![identity(IdentityKind::Agents, OTHER_TEXT)],
            Vec::new(),
            Vec::new(),
        )
        .build()
}

#[test]
fn a_nested_agent_is_refused_without_its_own_consent() {
    let refusal = refused(run(
        nested_snapshot(),
        Some("child"),
        PlanOptions::default(),
    ));
    assert_eq!(refusal.code(), "nested_agent");
}

#[test]
fn a_nested_agent_is_planned_with_consent_and_the_parent_plan_never_carries_it() {
    let consent = PlanOptions {
        nested_consent: true,
        ..PlanOptions::default()
    };
    let child = planned(run(nested_snapshot(), Some("child"), consent));
    assert_eq!(child.agent, "child");
    assert_eq!(paths(&child), vec!["CLAUDE.md"]);

    let parent = planned(run(nested_snapshot(), Some("."), PlanOptions::default()));
    assert_eq!(parent.agent, ".");
    assert_eq!(paths(&parent), vec!["CLAUDE.md"]);
    assert_eq!(
        parent.inputs.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["AGENTS.md"]
    );
    assert!(
        !parent
            .inputs
            .values()
            .any(|fingerprint| *fingerprint == digest_of(OTHER_TEXT))
    );
}

#[test]
fn an_agent_below_an_outside_ancestor_needs_consent() {
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![identity(IdentityKind::Agents, AGENTS_TEXT)],
            Vec::new(),
            Vec::new(),
        )
        .ancestor("/outside", vec!["claude".to_string()], Vec::new())
        .build();
    let refusal = refused(run(snapshot, None, PlanOptions::default()));
    assert_eq!(refusal.code(), "nested_agent");
}

fn conflict() -> TreeSnapshot {
    single_root(vec![
        identity(IdentityKind::Agents, AGENTS_TEXT),
        identity(IdentityKind::Claude, OTHER_TEXT),
    ])
}

#[test]
fn a_conflict_is_refused_until_a_canonical_file_is_chosen() {
    let refusal = refused(run(conflict(), None, PlanOptions::default()));
    assert_eq!(refusal.code(), "conflict");
    assert!(refusal.message().contains("--canonical-from"));
}

#[test]
fn canonical_from_plans_the_copy_from_the_chosen_file_and_records_the_loser() {
    let choose_claude = PlanOptions {
        canonical_from: Some(IdentityKind::Claude),
        ..PlanOptions::default()
    };
    let plan = planned(run(conflict(), None, choose_claude));
    assert_eq!(paths(&plan), vec!["AGENTS.md", "CLAUDE.md"]);
    match &plan.operations[0] {
        Operation::WriteAgents {
            source,
            after,
            before,
        } => {
            assert_eq!(*source, IdentityKind::Claude);
            assert_eq!(InputFingerprint::Sha256(*after), digest_of(OTHER_TEXT));
            assert_eq!(*before, digest_of(AGENTS_TEXT));
        }
        other @ Operation::WriteWrapper { .. } => {
            panic!("expected the AGENTS.md copy, got {other:?}")
        }
    }
    assert_eq!(plan.inputs["AGENTS.md"], digest_of(AGENTS_TEXT));
    assert_eq!(plan.inputs["CLAUDE.md"], digest_of(OTHER_TEXT));

    let choose_agents = PlanOptions {
        canonical_from: Some(IdentityKind::Agents),
        ..PlanOptions::default()
    };
    let other = planned(run(conflict(), None, choose_agents));
    assert_eq!(paths(&other), vec!["CLAUDE.md"]);
    assert_ne!(plan.id, other.id);
}

#[test]
fn canonical_from_that_does_not_apply_is_an_error_not_a_refusal() {
    let choose_claude = PlanOptions {
        canonical_from: Some(IdentityKind::Claude),
        ..PlanOptions::default()
    };
    let not_a_conflict = single_root(vec![identity(IdentityKind::Agents, AGENTS_TEXT)]);
    let error = run(not_a_conflict, None, choose_claude).expect_err("not a conflict");
    assert_eq!(error.reason(), "invalid-canonical-choice");
    assert!(matches!(
        error,
        DefinitionError::InvalidCanonicalChoice { .. }
    ));

    let choose_gemini = PlanOptions {
        canonical_from: Some(IdentityKind::Gemini),
        ..PlanOptions::default()
    };
    assert_eq!(
        run(conflict(), None, choose_gemini)
            .expect_err("GEMINI.md is not a canonical candidate")
            .reason(),
        "invalid-canonical-choice"
    );

    let wrapper_choice = single_root(vec![
        identity(IdentityKind::Agents, AGENTS_TEXT),
        identity(IdentityKind::Claude, OTHER_TEXT),
    ]);
    let choose_wrapper_less = PlanOptions {
        canonical_from: Some(IdentityKind::Agents),
        ..PlanOptions::default()
    };
    assert!(matches!(
        run(wrapper_choice, None, choose_wrapper_less),
        Ok(PlanOutcome::Planned(_))
    ));
}

fn with_gemini_copy(bytes: &[u8]) -> TreeSnapshot {
    single_root(vec![
        identity(IdentityKind::Agents, AGENTS_TEXT),
        identity(IdentityKind::Claude, WRAPPER),
        identity(IdentityKind::Gemini, bytes),
    ])
}

#[test]
fn a_secondary_file_that_would_still_differ_is_unresolved() {
    // Without a chosen canonical file the disagreement is a conflict.
    let refusal = refused(run(
        with_gemini_copy(OTHER_TEXT),
        None,
        PlanOptions::default(),
    ));
    assert_eq!(refusal.code(), "conflict");

    let choose_agents = PlanOptions {
        canonical_from: Some(IdentityKind::Agents),
        ..PlanOptions::default()
    };
    let refusal = refused(run(with_gemini_copy(OTHER_TEXT), None, choose_agents));
    assert_eq!(refusal.code(), "unresolved_file");
    assert!(refusal.message().contains("GEMINI.md"));
    assert!(refusal.message().contains("--with-gemini"));

    let qwen = single_root(vec![
        identity(IdentityKind::Agents, AGENTS_TEXT),
        identity(IdentityKind::Claude, WRAPPER),
        identity(IdentityKind::Qwen, OTHER_TEXT),
    ]);
    let with_gemini = PlanOptions {
        with_gemini: true,
        canonical_from: Some(IdentityKind::Agents),
        ..PlanOptions::default()
    };
    let refusal = refused(run(qwen, None, with_gemini));
    assert_eq!(refusal.code(), "unresolved_file");
    assert!(refusal.message().contains("QWEN.md"));
    assert!(!refusal.message().contains("--with-gemini"));
}

#[test]
fn with_gemini_resolves_a_differing_gemini_file_by_replacing_it() {
    let with_gemini = PlanOptions {
        with_gemini: true,
        canonical_from: Some(IdentityKind::Agents),
        ..PlanOptions::default()
    };
    let plan = planned(run(with_gemini_copy(OTHER_TEXT), None, with_gemini));
    assert_eq!(paths(&plan), vec!["GEMINI.md"]);
    assert_eq!(*plan.operations[0].before(), digest_of(OTHER_TEXT));
}

#[test]
fn a_secondary_byte_copy_of_the_source_is_planned() {
    let plan = planned(run(
        with_gemini_copy(AGENTS_TEXT),
        None,
        PlanOptions::default(),
    ));
    assert!(plan.operations.is_empty());
    assert_eq!(plan.inputs["GEMINI.md"], digest_of(AGENTS_TEXT));
}

#[test]
fn a_symlinked_or_irregular_identity_file_is_refused() {
    let symlinked_agents = single_root(vec![
        symlinked(IdentityKind::Agents, "elsewhere.md"),
        identity(IdentityKind::Claude, WRAPPER),
    ]);
    assert_eq!(
        refused(run(symlinked_agents, None, PlanOptions::default())).code(),
        "unsafe"
    );

    let irregular_claude = single_root(vec![
        identity(IdentityKind::Agents, AGENTS_TEXT),
        irregular(IdentityKind::Claude),
    ]);
    assert_eq!(
        refused(run(irregular_claude, None, PlanOptions::default())).code(),
        "unsafe"
    );

    let link_elsewhere = single_root(vec![
        identity(IdentityKind::Agents, AGENTS_TEXT),
        symlinked(IdentityKind::Claude, "../other/AGENTS.md"),
    ]);
    assert_eq!(
        refused(run(link_elsewhere, None, PlanOptions::default())).code(),
        "unsafe"
    );
}

#[test]
fn a_claude_link_to_agents_is_accepted_and_fingerprinted_by_its_target() {
    for target in ["AGENTS.md", "./AGENTS.md"] {
        let snapshot = single_root(vec![
            identity(IdentityKind::Agents, AGENTS_TEXT),
            symlinked(IdentityKind::Claude, target),
        ]);
        let plan = planned(run(snapshot, None, PlanOptions::default()));
        assert!(plan.operations.is_empty(), "{target}");
        assert_eq!(
            plan.inputs["CLAUDE.md"],
            InputFingerprint::Symlink(target.to_string())
        );
    }
}

#[test]
fn each_root_state_plans_its_identity_operations() {
    // Only CLAUDE.md holds content: AGENTS.md is written from it, then
    // CLAUDE.md becomes the wrapper.
    let plan = planned(run(claude_only(), None, PlanOptions::default()));
    assert_eq!(paths(&plan), vec!["AGENTS.md", "CLAUDE.md"]);
    assert_eq!(*plan.operations[0].before(), InputFingerprint::Absent);
    assert_eq!(*plan.operations[1].before(), digest_of(AGENTS_TEXT));

    // Only AGENTS.md: the wrapper is created.
    let agents_only = single_root(vec![identity(IdentityKind::Agents, AGENTS_TEXT)]);
    let plan = planned(run(agents_only, None, PlanOptions::default()));
    assert_eq!(paths(&plan), vec!["CLAUDE.md"]);
    assert_eq!(*plan.operations[0].before(), InputFingerprint::Absent);

    // Identical copies: the wrapper replaces the copy.
    let copies = single_root(vec![
        identity(IdentityKind::Agents, AGENTS_TEXT),
        identity(IdentityKind::Claude, AGENTS_TEXT),
    ]);
    let plan = planned(run(copies, None, PlanOptions::default()));
    assert_eq!(paths(&plan), vec!["CLAUDE.md"]);
    assert_eq!(*plan.operations[0].before(), digest_of(AGENTS_TEXT));
    match &plan.operations[0] {
        Operation::WriteWrapper { kind, after, .. } => {
            assert_eq!(*kind, IdentityKind::Claude);
            assert_eq!(InputFingerprint::Sha256(*after), digest_of(WRAPPER));
        }
        other @ Operation::WriteAgents { .. } => panic!("expected the wrapper, got {other:?}"),
    }

    // A near wrapper is replaced by the exact wrapper.
    let near = single_root(vec![
        identity(IdentityKind::Agents, AGENTS_TEXT),
        identity(IdentityKind::Claude, b"@AGENTS.md\n\n"),
    ]);
    let plan = planned(run(near, None, PlanOptions::default()));
    assert_eq!(paths(&plan), vec!["CLAUDE.md"]);

    // Canonical: no identity operations.
    let canonical = single_root(vec![
        identity(IdentityKind::Agents, AGENTS_TEXT),
        identity(IdentityKind::Claude, WRAPPER),
    ]);
    let plan = planned(run(canonical, None, PlanOptions::default()));
    assert!(plan.operations.is_empty());
    assert_eq!(plan.inputs.len(), 2);
}

#[test]
fn with_gemini_adds_the_gemini_wrapper_after_the_claude_wrapper() {
    let with_gemini = PlanOptions {
        with_gemini: true,
        ..PlanOptions::default()
    };
    let plan = planned(run(claude_only(), None, with_gemini));
    assert_eq!(paths(&plan), vec!["AGENTS.md", "CLAUDE.md", "GEMINI.md"]);
    assert_eq!(*plan.operations[2].before(), InputFingerprint::Absent);

    let canonical = single_root(vec![
        identity(IdentityKind::Agents, AGENTS_TEXT),
        identity(IdentityKind::Claude, WRAPPER),
    ]);
    let plan = planned(run(canonical, None, with_gemini));
    assert_eq!(paths(&plan), vec!["GEMINI.md"]);
}

#[test]
fn a_hand_edited_skills_block_refuses_the_plan() {
    let hasher = FakeHasher;
    let digest = |bytes: &[u8]| hasher.sha256(bytes);
    let body = "## Local skill index\n\n| Skill | Use when | Path |\n| --- | --- | --- |\n\
                | sample | x | [sample](skills/sample/SKILL.md) |"
        .to_string();
    let block = ManagedBlock::skills(body, &digest).expect("a block");
    let tampered = block.render(LineEnding::Lf).replace("sample", "edited");
    let agents = format!("# Sample\n\nYou are Sample.\n\n{tampered}\n").into_bytes();
    let snapshot = single_root(vec![
        identity(IdentityKind::Agents, &agents),
        identity(IdentityKind::Claude, WRAPPER),
    ]);
    let refusal = refused(run(snapshot, None, PlanOptions::default()));
    assert_eq!(refusal.code(), "edited_block");
}

#[test]
fn a_malformed_block_and_unsafe_skill_metadata_refuse_the_plan() {
    let hasher = FakeHasher;
    let digest = |bytes: &[u8]| hasher.sha256(bytes);
    let begin = ManagedBlock::skills("placeholder", &digest)
        .expect("a block")
        .begin_sentinel();
    let agents = format!("# Sample\n\nYou are Sample.\n\n{begin}\n").into_bytes();
    let malformed = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, &agents),
                identity(IdentityKind::Claude, WRAPPER),
            ],
            Vec::new(),
            Vec::new(),
        )
        .skill(
            "skills/sample/SKILL.md",
            b"---\nname: sample\n---\n".to_vec(),
        )
        .build();
    assert_eq!(
        refused(run(malformed, None, PlanOptions::default())).code(),
        "block_layout"
    );

    let unsafe_metadata = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, AGENTS_TEXT),
                identity(IdentityKind::Claude, WRAPPER),
            ],
            Vec::new(),
            Vec::new(),
        )
        .skill(
            "skills/mail/SKILL.md",
            b"---\nname: mail-helper\ndescription: Contact @support for assistance\n---\n".to_vec(),
        )
        .build();
    assert_eq!(
        refused(run(unsafe_metadata, None, PlanOptions::default())).code(),
        "unsafe_skill_metadata"
    );
}

#[test]
fn a_secondary_copy_that_the_skills_block_would_desynchronize_is_a_diverging_copy() {
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, AGENTS_TEXT),
                identity(IdentityKind::Claude, WRAPPER),
                identity(IdentityKind::Gemini, AGENTS_TEXT),
            ],
            Vec::new(),
            Vec::new(),
        )
        .skill(
            "skills/sample/SKILL.md",
            b"---\nname: sample\n---\n".to_vec(),
        )
        .build();
    let refusal = refused(run(snapshot, None, PlanOptions::default()));
    assert_eq!(refusal.code(), "diverging_copy");
    assert!(refusal.message().contains("GEMINI.md"));
}

#[test]
fn the_nested_guard_outranks_the_unsafe_refusal() {
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![identity(IdentityKind::Agents, AGENTS_TEXT)],
            Vec::new(),
            Vec::new(),
        )
        .directory(
            "child",
            vec![symlinked(IdentityKind::Agents, "elsewhere.md")],
            Vec::new(),
            Vec::new(),
        )
        .build();
    assert_eq!(
        refused(run(snapshot, Some("child"), PlanOptions::default())).code(),
        "nested_agent"
    );
}

#[test]
fn an_invalid_target_is_a_definition_error() {
    let error = run(claude_only(), Some("../up"), PlanOptions::default())
        .expect_err("a parent component is refused");
    assert_eq!(error.reason(), "nesting(parent-component)");
}
