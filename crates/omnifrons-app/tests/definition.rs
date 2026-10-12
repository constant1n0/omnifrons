//! The read-only `scan` service (ADR-0005 sub-slice 1e-A) against the
//! in-memory `DefinitionTree` fake: a canonical single agent, a nested
//! pair with a skill and an outside ancestor exercising every
//! `AgentReport` field, the empty root, a refused duplicate identity
//! kind, and the `DefinitionTree` contract itself.
//!
//! Only compiled with `--features contract-tests`: the fake lives behind
//! that feature.

#![cfg(feature = "contract-tests")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::content_hasher::ContentHasher;
use omnifrons_app::contract::definition::{
    InMemoryDefinitionTree, TreeSnapshotBuilder, definition_tree_contract,
};
use omnifrons_app::contract::publication::FakeHasher;
use omnifrons_app::definition::{DefinitionError, DefinitionPorts, scan};
use omnifrons_domain::definition::{FileShape, IdentityFile, IdentityKind, RootState, WRAPPER};
use omnifrons_domain::nesting::Parent;
use omnifrons_domain::warnings::WarningCode;

/// A drop-guard temp directory standing in for a scanned workspace.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-definition-test-{}-{label}-{n}",
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

/// A regular identity file of `kind` holding `bytes`.
fn identity(kind: IdentityKind, bytes: &[u8]) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: bytes.to_vec(),
        shape: FileShape::Regular,
    }
}

/// Translation of the prototype's `test_scan_reports_canonical_atlas`,
/// with a generic stand-in agent: `AGENTS.md` holds the canonical text
/// and `CLAUDE.md` is the exact wrapper, so the root classifies as
/// canonical, is its own report's only agent, has no parent, and needs
/// no consent.
#[test]
fn canonical_single_agent_reports_title_and_no_parent() {
    let project = TempDir::new("canonical");
    let workspace = project.workspace();
    let agents_bytes = b"# Sample\n\nYou are Sample, a fixture agent.\n";
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, agents_bytes),
                identity(IdentityKind::Claude, WRAPPER),
            ],
            Vec::new(),
            Vec::new(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = scan(&ports, &workspace).expect("scan succeeds");

    assert!(report.ancestors.is_empty());
    assert_eq!(report.agents.len(), 1);
    let agent = &report.agents[0];
    assert_eq!(agent.path, ".");
    assert_eq!(agent.state, RootState::Canonical);
    assert_eq!(agent.title, Some("Sample".to_string()));
    assert_eq!(agent.parent, None);
    assert!(!agent.consent_required);
    assert!(agent.nested.is_empty());

    let agents_report = agent
        .identity_files
        .iter()
        .find(|file| file.kind == IdentityKind::Agents)
        .expect("AGENTS.md reported");
    assert_eq!(agents_report.bytes, agents_bytes.len() as u64);
    assert_eq!(
        agents_report.sha256,
        Some(hasher.sha256(agents_bytes).to_hex())
    );
    assert_eq!(agents_report.shape, FileShape::Regular);

    let claude_report = agent
        .identity_files
        .iter()
        .find(|file| file.kind == IdentityKind::Claude)
        .expect("CLAUDE.md reported");
    assert_eq!(claude_report.bytes, WRAPPER.len() as u64);
    assert_eq!(claude_report.sha256, Some(hasher.sha256(WRAPPER).to_hex()));
}

/// The composition counterpart of the prototype's
/// `test_scan_json_schema_key_set`: a root nested under itself by a
/// child agent, a skill the root's text never links, and an outside
/// ancestor whose `CLAUDE.md` shadows the root's own instructions --
/// every `AgentReport` field ends up populated, on both agents.
#[test]
fn nested_pair_with_skill_and_outside_ancestor_populates_every_field() {
    let project = TempDir::new("nested");
    let workspace = project.workspace();
    let root_text = b"# Sample Agent\n\nYou are Sample.\n\n[dangling](other/SKILL.md)\n";
    let child_text = b"# Child Agent\n\nYou are Child, a nested fixture agent.\n";
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![identity(IdentityKind::Agents, root_text)],
            Vec::new(),
            Vec::new(),
        )
        .directory(
            "child",
            vec![identity(IdentityKind::Claude, child_text)],
            Vec::new(),
            Vec::new(),
        )
        .skill(
            "skills/sample/SKILL.md",
            b"---\nname: sample\n---\n".to_vec(),
        )
        .ancestor("/outside", vec!["claude".to_string()])
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = scan(&ports, &workspace).expect("scan succeeds");

    assert_eq!(report.ancestors.len(), 1);
    assert_eq!(report.agents.len(), 2);

    let root = report
        .agents
        .iter()
        .find(|agent| agent.path == ".")
        .expect("root agent");
    assert_eq!(root.state, RootState::AgentsOnly);
    assert_eq!(root.title, Some("Sample Agent".to_string()));
    // The scan root itself has no inner parent; with an outside
    // ancestor present it falls back to that ancestor, exactly like any
    // other root with no inner parent (`nesting::relate`'s own
    // documentation).
    assert_eq!(root.parent, Some(Parent::Outside("/outside".to_string())));
    assert!(root.consent_required);
    assert_eq!(root.nested, vec!["child".to_string()]);
    assert_eq!(
        root.skills.discovered,
        vec!["skills/sample/SKILL.md".to_string()]
    );
    assert_eq!(root.skills.indexed, vec!["other/SKILL.md".to_string()]);
    assert_eq!(
        root.skills.missing,
        vec!["skills/sample/SKILL.md".to_string()]
    );
    assert_eq!(root.skills.dangling, vec!["other/SKILL.md".to_string()]);
    assert!(!root.skills.precedence);
    assert!(
        root.warnings
            .iter()
            .any(|warning| warning.code == WarningCode::AncestorShadowing),
        "expected an ancestor_shadowing warning, got {:?}",
        root.warnings
    );

    let child = report
        .agents
        .iter()
        .find(|agent| agent.path == "child")
        .expect("child agent");
    assert_eq!(child.state, RootState::ClaudeOnly);
    assert_eq!(child.title, Some("Child Agent".to_string()));
    assert_eq!(child.parent, Some(Parent::Root(".".to_string())));
    assert!(child.consent_required);
    assert!(child.nested.is_empty());
    assert!(child.skills.discovered.is_empty());
    assert!(
        child
            .warnings
            .iter()
            .any(|warning| warning.code == WarningCode::AncestorExposure),
        "expected an ancestor_exposure warning, got {:?}",
        child.warnings
    );
}

/// A workspace with no identity file anywhere yields no agents.
#[test]
fn empty_root_yields_no_agents() {
    let project = TempDir::new("empty");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new().build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = scan(&ports, &workspace).expect("scan succeeds");

    assert!(report.agents.is_empty());
    assert!(report.ancestors.is_empty());
}

/// Two identity files of the same kind at one directory are refused by
/// `DirectoryIdentity::new`, surfaced as `DefinitionError::DuplicateIdentityKind`
/// with the fixed `duplicate-identity-kind` reason token.
#[test]
fn duplicate_identity_kind_from_the_port_is_refused() {
    let project = TempDir::new("duplicate");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, b"# One\n"),
                identity(IdentityKind::Agents, b"# Two\n"),
            ],
            Vec::new(),
            Vec::new(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let error = scan(&ports, &workspace).expect_err("a duplicate identity kind is refused");

    assert!(matches!(error, DefinitionError::DuplicateIdentityKind(_)));
    assert_eq!(error.reason(), "duplicate-identity-kind");
}

/// `definition_tree_contract` passes a well-formed snapshot: clean
/// relative paths, a `SKILL.md` basename, and the prototype's own
/// sorted walk order.
#[test]
fn definition_tree_contract_passes_a_well_formed_snapshot() {
    let project = TempDir::new("contract-ok");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![identity(IdentityKind::Agents, b"# One\n")],
            Vec::new(),
            Vec::new(),
        )
        .directory(
            "child",
            vec![identity(IdentityKind::Claude, b"# Two\n")],
            Vec::new(),
            Vec::new(),
        )
        .skill("child/skills/sample/SKILL.md", b"---\n".to_vec())
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);

    definition_tree_contract(&tree, &workspace);
}

/// `definition_tree_contract` fails a malformed snapshot: two
/// directories listed out of the prototype's sorted walk order.
#[test]
#[should_panic(expected = "sorted walk order")]
fn definition_tree_contract_fails_a_malformed_snapshot() {
    let project = TempDir::new("contract-bad");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            "b",
            vec![identity(IdentityKind::Agents, b"# One\n")],
            Vec::new(),
            Vec::new(),
        )
        .directory(
            "a",
            vec![identity(IdentityKind::Claude, b"# Two\n")],
            Vec::new(),
            Vec::new(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);

    definition_tree_contract(&tree, &workspace);
}
