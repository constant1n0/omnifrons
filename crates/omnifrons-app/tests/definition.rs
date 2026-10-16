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
use omnifrons_app::definition::{DefinitionError, DefinitionPorts, scan, verify};
use omnifrons_app::definition_tree::{DefinitionTree, DefinitionTreeError, TreeSnapshot};
use omnifrons_domain::definition::{FileShape, IdentityFile, IdentityKind, RootState, WRAPPER};
use omnifrons_domain::guidance::{LineEnding, ManagedBlock};
use omnifrons_domain::nesting::{NestingError, Parent};
use omnifrons_domain::skills_index::{PRECEDENCE_RULE, SkillRow, render_body};
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

/// A symlinked identity file of `kind`, pointing at `target`: never read
/// for content, so its bytes are empty, matching
/// [`IdentityFile::bytes`]'s own documented shape for this case.
fn symlinked_identity(kind: IdentityKind, target: &str) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: Vec::new(),
        shape: FileShape::Symlink {
            target: target.to_string(),
        },
    }
}

/// An irregular identity file of `kind` -- neither a regular file nor a
/// symlink, and never read for content either, so its bytes are empty.
fn irregular_identity(kind: IdentityKind) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: Vec::new(),
        shape: FileShape::Irregular,
    }
}

/// A [`DefinitionTree`] stub that always fails with a fixed error,
/// exercising `scan`'s `DefinitionTreeError` -> `DefinitionError::Tree`
/// mapping for a reason [`InMemoryDefinitionTree`] never itself
/// produces (it only ever reports
/// [`DefinitionTreeError::RootUnreadable`]), so the mapping is proven
/// generic rather than tied to one coincidental variant.
struct FailingTree(DefinitionTreeError);

impl DefinitionTree for FailingTree {
    fn snapshot(&self, _root: &WorkspaceRoot) -> Result<TreeSnapshot, DefinitionTreeError> {
        Err(self.0)
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
        .ancestor("/outside", vec!["claude".to_string()], Vec::new())
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

/// `definition_tree_contract` passes the prototype's real top-down
/// skill order: the scan root's own `SKILL.md` always precedes every
/// skill found inside a subdirectory, and subdirectories are visited in
/// sorted name order -- `.hidden`, `1dir`, `A`, `a` -- regardless of how
/// each subdirectory's name compares to the literal string `SKILL.md`.
#[test]
fn definition_tree_contract_passes_the_real_top_down_skill_order() {
    let project = TempDir::new("skill-order-ok");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new()
        .skill("SKILL.md", b"---\n".to_vec())
        .skill(".hidden/SKILL.md", b"---\n".to_vec())
        .skill("1dir/SKILL.md", b"---\n".to_vec())
        .skill("A/SKILL.md", b"---\n".to_vec())
        .skill("a/SKILL.md", b"---\n".to_vec())
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);

    definition_tree_contract(&tree, &workspace);
}

/// The order a naive per-component comparison of the *full* skill path
/// (basename included) would accept is not the real top-down order: it
/// sorts the scan root's own `SKILL.md` wherever the literal string
/// `SKILL.md` happens to fall among sibling directory names, instead of
/// always first.
#[test]
#[should_panic(expected = "sorted walk order")]
fn definition_tree_contract_fails_the_old_component_sorted_skill_order() {
    let project = TempDir::new("skill-order-bad");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new()
        .skill(".hidden/SKILL.md", b"---\n".to_vec())
        .skill("1dir/SKILL.md", b"---\n".to_vec())
        .skill("A/SKILL.md", b"---\n".to_vec())
        .skill("SKILL.md", b"---\n".to_vec())
        .skill("a/SKILL.md", b"---\n".to_vec())
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);

    definition_tree_contract(&tree, &workspace);
}

/// A [`DefinitionTree`] port failure is surfaced as
/// `DefinitionError::Tree`, carrying that error's own delegated
/// `reason()` token -- proven with a reason
/// [`InMemoryDefinitionTree`] never itself produces, so the mapping is
/// generic rather than coincidental.
#[test]
fn tree_port_failure_maps_to_definition_error_tree_with_its_reason() {
    let project = TempDir::new("tree-failure");
    let workspace = project.workspace();
    let tree = FailingTree(DefinitionTreeError::EntryTooLarge);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let error = scan(&ports, &workspace).expect_err("a failing tree port is surfaced");

    assert!(matches!(
        error,
        DefinitionError::Tree(DefinitionTreeError::EntryTooLarge)
    ));
    assert_eq!(error.reason(), "tree(entry-too-large)");
}

/// A root path repeated in the port's own snapshot is refused by
/// `nesting::relate` and surfaced as `DefinitionError::Nesting`,
/// carrying that error's own delegated `reason()` token.
#[test]
fn duplicate_root_from_the_port_maps_to_definition_error_nesting_with_its_reason() {
    let project = TempDir::new("duplicate-root");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            "dup",
            vec![identity(IdentityKind::Agents, b"# One\n")],
            Vec::new(),
            Vec::new(),
        )
        .directory(
            "dup",
            vec![identity(IdentityKind::Claude, b"# Two\n")],
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

    let error = scan(&ports, &workspace).expect_err("a duplicate root path is refused");

    assert!(matches!(
        error,
        DefinitionError::Nesting(NestingError::DuplicateRoot(_))
    ));
    assert_eq!(error.reason(), "nesting(duplicate-root)");
}

/// A symlinked identity file and an irregular one both report no
/// content: `bytes: 0` and `sha256: None`, since neither is ever read.
#[test]
fn symlinked_and_irregular_identity_files_report_no_bytes_or_hash() {
    let project = TempDir::new("unsafe-shapes");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                symlinked_identity(IdentityKind::Agents, "elsewhere"),
                irregular_identity(IdentityKind::Claude),
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

    assert_eq!(report.agents.len(), 1);
    let agent = &report.agents[0];
    let agents_report = agent
        .identity_files
        .iter()
        .find(|file| file.kind == IdentityKind::Agents)
        .expect("AGENTS.md reported");
    assert_eq!(agents_report.bytes, 0);
    assert_eq!(agents_report.sha256, None);
    assert_eq!(
        agents_report.shape,
        FileShape::Symlink {
            target: "elsewhere".to_string()
        }
    );

    let claude_report = agent
        .identity_files
        .iter()
        .find(|file| file.kind == IdentityKind::Claude)
        .expect("CLAUDE.md reported");
    assert_eq!(claude_report.bytes, 0);
    assert_eq!(claude_report.sha256, None);
    assert_eq!(claude_report.shape, FileShape::Irregular);
}

/// A skill outside every discovered root is dropped from every agent's
/// `discovered` list, not just the nearest one.
#[test]
fn skill_outside_every_root_is_dropped_from_every_agents_discovered_list() {
    let project = TempDir::new("outside-skill");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            "agent",
            vec![identity(IdentityKind::Agents, b"# Agent\n")],
            Vec::new(),
            Vec::new(),
        )
        .directory(
            "other",
            vec![identity(IdentityKind::Agents, b"# Other\n")],
            Vec::new(),
            Vec::new(),
        )
        .skill("outside/SKILL.md", b"---\n".to_vec())
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = scan(&ports, &workspace).expect("scan succeeds");

    assert_eq!(report.agents.len(), 2);
    for agent in &report.agents {
        assert!(
            agent.skills.discovered.is_empty(),
            "a skill outside every root must not be attributed to {}, got {:?}",
            agent.path,
            agent.skills.discovered
        );
    }
}

/// An outside ancestor's `identity_files` entry may be written in
/// either the identity kind's file-name form (`"AGENTS.md"`, the
/// prototype's own shape) or its token form (`"agents"`):
/// `chain_identity_files` accepts both -- trying
/// `IdentityKind::from_file_name` first, then falling back to
/// `IdentityKind::parse` -- so either one feeds the warning chain and
/// produces an `ancestor_shadowing` warning; an entry neither form
/// recognizes is still dropped silently, producing no `ancestor_*`
/// warning for that ancestor at all.
#[test]
fn outside_ancestor_identity_form_controls_the_warning_chain() {
    let hasher = FakeHasher;
    let cases = [("AGENTS.md", true), ("agents", true), ("nonsense", false)];

    for (entry, expect_shadowing) in cases {
        let project = TempDir::new(&format!("ancestor-form-{entry}"));
        let workspace = project.workspace();
        let snapshot = TreeSnapshotBuilder::new()
            .directory(
                ".",
                vec![identity(IdentityKind::Claude, b"# Root\n\nReal content.\n")],
                Vec::new(),
                Vec::new(),
            )
            .ancestor("/outside", vec![entry.to_string()], Vec::new())
            .build();
        let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
        let ports = DefinitionPorts {
            tree: &tree,
            hasher: &hasher,
        };
        let report = scan(&ports, &workspace).expect("scan succeeds");
        let root = &report.agents[0];
        let has_shadowing = root
            .warnings
            .iter()
            .any(|warning| warning.code == WarningCode::AncestorShadowing);
        let has_any_ancestor_warning = root.warnings.iter().any(|warning| {
            matches!(
                warning.code,
                WarningCode::AncestorShadowing | WarningCode::AncestorExposure
            )
        });
        if expect_shadowing {
            assert!(
                has_shadowing,
                "{entry}: expected an ancestor_shadowing warning, got {:?}",
                root.warnings
            );
        } else {
            assert!(
                !has_any_ancestor_warning,
                "{entry}: an unrecognized ancestor entry must be dropped silently, got {:?}",
                root.warnings
            );
        }
    }
}

/// A skill owned by a nested root, not the scan root, is reported
/// relative to that nested root (`relative_within`'s non-root branch,
/// stripping the owner prefix and its separator): the owning agent's
/// `discovered` holds the skill's path relative to itself, the scan
/// root's own `discovered` stays empty, and a link in the owning
/// agent's own canonical text to that relative path resolves it as
/// `indexed`, not `missing`.
#[test]
fn skill_owned_by_a_nested_root_is_relative_to_that_root() {
    let project = TempDir::new("nested-root-skill");
    let workspace = project.workspace();
    let root_text = b"# Root Agent\n\nYou are Root.\n";
    let child_text = b"# Child Agent\n\nYou are Child.\n\n[x](skills/x/SKILL.md)\n";
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![identity(IdentityKind::Agents, root_text)],
            Vec::new(),
            Vec::new(),
        )
        .directory(
            "child",
            vec![identity(IdentityKind::Agents, child_text)],
            Vec::new(),
            Vec::new(),
        )
        .skill("child/skills/x/SKILL.md", b"---\nname: x\n---\n".to_vec())
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = scan(&ports, &workspace).expect("scan succeeds");

    let root = report
        .agents
        .iter()
        .find(|agent| agent.path == ".")
        .expect("root agent");
    assert!(
        root.skills.discovered.is_empty(),
        "the scan root must not own the nested agent's skill, got {:?}",
        root.skills.discovered
    );

    let child = report
        .agents
        .iter()
        .find(|agent| agent.path == "child")
        .expect("child agent");
    assert_eq!(
        child.skills.discovered,
        vec!["skills/x/SKILL.md".to_string()]
    );
    assert_eq!(child.skills.indexed, vec!["skills/x/SKILL.md".to_string()]);
    assert!(
        child.skills.missing.is_empty(),
        "expected no missing skills, got {:?}",
        child.skills.missing
    );
}

/// `host_extras` produces an `extra_host_file` warning only for a name
/// `omnifrons_domain::hosts::EXTRA_READERS` recognizes, naming that
/// extra's own readers; an unrecognized name is silently dropped
/// (`agent_warnings`'s own defensive skip), producing no warning at
/// all.
#[test]
fn host_extras_warns_only_for_a_known_extra_and_drops_the_rest() {
    let project = TempDir::new("host-extras");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![identity(IdentityKind::Agents, b"# Root\n")],
            vec![
                "CLAUDE.local.md".to_string(),
                "not-a-real-extra.txt".to_string(),
            ],
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

    assert_eq!(report.agents.len(), 1);
    let agent = &report.agents[0];
    let extra_warnings: Vec<_> = agent
        .warnings
        .iter()
        .filter(|warning| warning.code == WarningCode::ExtraHostFile)
        .collect();
    assert_eq!(
        extra_warnings.len(),
        1,
        "expected exactly one extra_host_file warning, got {:?}",
        agent.warnings
    );
    assert_eq!(extra_warnings[0].hosts, vec!["Claude Code"]);
    assert_eq!(
        extra_warnings[0].detail,
        "CLAUDE.local.md is read only by Claude Code; it is reported and never changed"
    );
}

/// A `omnifrons_domain::hosts::GENERATED_STATE_DIRS` entry present at a
/// root passes straight through into a `generated_state_dir` warning
/// naming no host.
#[test]
fn generated_state_dir_present_reports_a_hostless_warning() {
    let project = TempDir::new("generated-state-dir");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![identity(IdentityKind::Agents, b"# Root\n")],
            Vec::new(),
            vec![".atl"],
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = scan(&ports, &workspace).expect("scan succeeds");

    assert_eq!(report.agents.len(), 1);
    let agent = &report.agents[0];
    let state_dir_warnings: Vec<_> = agent
        .warnings
        .iter()
        .filter(|warning| warning.code == WarningCode::GeneratedStateDir)
        .collect();
    assert_eq!(
        state_dir_warnings.len(),
        1,
        "expected exactly one generated_state_dir warning, got {:?}",
        agent.warnings
    );
    assert!(state_dir_warnings[0].hosts.is_empty());
    assert_eq!(
        state_dir_warnings[0].detail,
        ".atl/ holds generated tool state, not agent instructions; it is not part of the \
         portable definition and the adapter never changes it"
    );
}

// ---------------------------------------------------------------------
// Review findings on the merged 1e-B: a chain link's own `host_extras`
// now reach the warning chain the same way its identity files do
// (`chain_host_extras`), for both an inside root and an outside
// ancestor.
// ---------------------------------------------------------------------

/// An inside root's own `host_extras` disable Claude Code's `AGENTS.md`
/// fallback for its nested child exactly like a real `CLAUDE.md` there
/// would: the parent root holds `CLAUDE.local.md` only as a host
/// extra, never as an identity file, and the `AGENTS.md`-only child
/// still gets no Claude Code `ancestor_*` warning.
#[test]
fn an_inside_ancestors_host_extra_disables_the_claude_code_fallback_for_its_nested_child() {
    let project = TempDir::new("inside-ancestor-extra");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![identity(IdentityKind::Agents, b"# Root\n\nYou are Root.\n")],
            vec!["CLAUDE.local.md".to_string()],
            Vec::new(),
        )
        .directory(
            "child",
            vec![identity(
                IdentityKind::Agents,
                b"# Child\n\nYou are Child.\n",
            )],
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

    let child = report
        .agents
        .iter()
        .find(|agent| agent.path == "child")
        .expect("child agent");
    assert!(
        !child
            .warnings
            .iter()
            .any(|warning| warning.hosts.contains(&"Claude Code")),
        "expected no Claude Code warning for the child, got {:?}",
        child.warnings
    );
}

/// An outside ancestor's own `host_extras` reach the warning chain too:
/// an ancestor holding `.claude/AGENTS.md` only as a host extra -- never
/// an identity file, since it is not one of `IdentityKind`'s six kinds
/// -- still fires `ancestor_exposure` naming Claude Code for an
/// `AGENTS.md`-only root once the fallback applies.
#[test]
fn an_outside_ancestors_claude_agents_host_extra_yields_the_claude_code_warning() {
    let project = TempDir::new("outside-ancestor-extra");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![identity(IdentityKind::Agents, b"# Root\n\nYou are Root.\n")],
            Vec::new(),
            Vec::new(),
        )
        .ancestor(
            "/outside",
            Vec::new(),
            vec![".claude/AGENTS.md".to_string()],
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = scan(&ports, &workspace).expect("scan succeeds");

    let root = &report.agents[0];
    let exposure: Vec<_> = root
        .warnings
        .iter()
        .filter(|warning| warning.code == WarningCode::AncestorExposure)
        .collect();
    assert!(
        exposure.iter().any(|w| w.hosts.contains(&"Claude Code")),
        "expected an ancestor_exposure warning naming Claude Code, got {:?}",
        root.warnings
    );
}

// ---------------------------------------------------------------------
// `chain_host_extras` documents that a `host_extras` entry
// `hosts::EXTRA_READERS` does not recognize is silently dropped, the
// same way `chain_identity_files` drops an unrecognized identity
// entry. These two tests pin that for both an inside ancestor root and
// an outside one.
// ---------------------------------------------------------------------

/// An unrecognized extra alongside the inside ancestor's real
/// `CLAUDE.local.md` changes nothing for the nested child: its
/// warnings are exactly those of the same tree without that extra --
/// no panic, no extra warning for the unknown name, and the same
/// disabled Claude Code fallback
/// `an_inside_ancestors_host_extra_disables_the_claude_code_fallback_for_its_nested_child`
/// already pins.
#[test]
fn an_unrecognized_host_extra_on_an_inside_ancestor_changes_nothing() {
    let run = |extras: Vec<String>, label: &str| {
        let project = TempDir::new(label);
        let workspace = project.workspace();
        let snapshot = TreeSnapshotBuilder::new()
            .directory(
                ".",
                vec![identity(IdentityKind::Agents, b"# Root\n\nYou are Root.\n")],
                extras,
                Vec::new(),
            )
            .directory(
                "child",
                vec![identity(
                    IdentityKind::Agents,
                    b"# Child\n\nYou are Child.\n",
                )],
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
        scan(&ports, &workspace)
            .expect("scan succeeds")
            .agents
            .iter()
            .find(|agent| agent.path == "child")
            .expect("child agent")
            .warnings
            .clone()
    };

    let baseline = run(vec!["CLAUDE.local.md".to_string()], "inside-extra-baseline");
    let with_unknown = run(
        vec![
            "CLAUDE.local.md".to_string(),
            "not-a-real-extra.txt".to_string(),
        ],
        "inside-extra-unknown",
    );

    assert_eq!(with_unknown, baseline, "{with_unknown:?}");
    assert!(
        !with_unknown
            .iter()
            .any(|warning| warning.hosts.contains(&"Claude Code")),
        "expected no Claude Code warning for the child, got {with_unknown:?}"
    );
}

/// An unrecognized extra alongside the outside ancestor's real
/// `.claude/AGENTS.md` changes nothing for the root it reaches through
/// the warning chain: its warnings are exactly those of the same tree
/// without that extra -- no panic, no extra warning for the unknown
/// name, and the same `ancestor_exposure`
/// `an_outside_ancestors_claude_agents_host_extra_yields_the_claude_code_warning`
/// already pins.
#[test]
fn an_unrecognized_host_extra_on_an_outside_ancestor_changes_nothing() {
    let run = |extras: Vec<String>, label: &str| {
        let project = TempDir::new(label);
        let workspace = project.workspace();
        let snapshot = TreeSnapshotBuilder::new()
            .directory(
                ".",
                vec![identity(IdentityKind::Agents, b"# Root\n\nYou are Root.\n")],
                Vec::new(),
                Vec::new(),
            )
            .ancestor("/outside", Vec::new(), extras)
            .build();
        let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
        let hasher = FakeHasher;
        let ports = DefinitionPorts {
            tree: &tree,
            hasher: &hasher,
        };
        scan(&ports, &workspace)
            .expect("scan succeeds")
            .agents
            .first()
            .expect("root agent")
            .warnings
            .clone()
    };

    let baseline = run(
        vec![".claude/AGENTS.md".to_string()],
        "outside-extra-baseline",
    );
    let with_unknown = run(
        vec![
            ".claude/AGENTS.md".to_string(),
            "also-not-a-real-extra.txt".to_string(),
        ],
        "outside-extra-unknown",
    );

    assert_eq!(with_unknown, baseline, "{with_unknown:?}");
    assert!(
        with_unknown.iter().any(|warning| {
            warning.code == WarningCode::AncestorExposure && warning.hosts.contains(&"Claude Code")
        }),
        "expected an ancestor_exposure warning naming Claude Code, got {with_unknown:?}"
    );
}

// -- verify (ADR-0005 sub-slice 1e-D) --

/// (a) A canonical agent -- `AGENTS.md` holds the text and `CLAUDE.md` is
/// the exact wrapper, with no skills discovered -- has no problems at
/// all (the prototype's own `verify_agent`, `canonical` state, empty
/// `problems`).
#[test]
fn verify_reports_canonical_with_no_problems() {
    let project = TempDir::new("verify-canonical");
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

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(report.canonical, "{:?}", report.problems);
    assert_eq!(report.problems, Vec::<String>::new());
}

/// (b) A canonical agent with a host-specific extra file alongside it is
/// still canonical overall ([`RootState::HostExtras`] is one of the two
/// states `verify_agent` never reports a `"state is ..."` problem for).
#[test]
fn verify_reports_host_extras_state_with_no_problems() {
    let project = TempDir::new("verify-host-extras");
    let workspace = project.workspace();
    let agents_bytes = b"# Sample\n\nYou are Sample, a fixture agent.\n";
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, agents_bytes),
                identity(IdentityKind::Claude, WRAPPER),
            ],
            vec!["CLAUDE.local.md".to_string()],
            Vec::new(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::HostExtras);
    assert!(report.canonical, "{:?}", report.problems);
    assert_eq!(report.problems, Vec::<String>::new());
}

/// (c) `AGENTS.md` with no `CLAUDE.md` at all: `"state is agents_only"`,
/// plus the pending `write_wrapper` operation `build_plan` would still
/// carry (`describe_operation`'s own `"creates the file"` wording, since
/// there is no existing `CLAUDE.md` to replace).
#[test]
fn verify_reports_agents_only_state_and_pending_wrapper_write() {
    let project = TempDir::new("verify-agents-only");
    let workspace = project.workspace();
    let agents_bytes = b"# Sample\n\nYou are Sample, a fixture agent.\n";
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![identity(IdentityKind::Agents, agents_bytes)],
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

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::AgentsOnly);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "state is agents_only".to_string(),
            "pending: write the wrapper CLAUDE.md (creates the file)".to_string(),
        ]
    );
}

/// (d) A discovered skill the canonical text never links, and no
/// skills block yet: `verify` reports the pending `append_block`
/// operation `build_plan`'s own `plan_block` would plan
/// (`describe_operation`'s own wording: the row count, and the
/// precedence rule since the text states neither).
#[test]
fn verify_reports_pending_skills_block_for_an_unindexed_discovered_skill() {
    let project = TempDir::new("verify-skills-pending");
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
        .skill(
            "skills/sample/SKILL.md",
            b"---\nname: sample\n---\n".to_vec(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "pending: append the skill index block in AGENTS.md (1 skill rows and the \
             precedence rule)"
                .to_string()
        ]
    );
}

/// (e) A discovered skill whose `description:` holds a possible `@`
/// import: `verify` refuses with the prototype's own
/// `unsafe_skill_metadata` code, rather than planning a skill-index row
/// Claude Code could load as a file import ([`unsafe_row_fields`]'s own
/// finding, read back through `unsafe_rows_detail`).
///
/// [`unsafe_row_fields`]: omnifrons_domain::skills_safety::unsafe_row_fields
#[test]
fn verify_refuses_a_skill_whose_description_holds_an_unsafe_at_token() {
    let project = TempDir::new("verify-unsafe-at-token");
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
        .skill(
            "skills/mail/SKILL.md",
            b"---\nname: mail-helper\ndescription: Contact @support for assistance\n---\n".to_vec(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "refused (unsafe_skill_metadata): these local skills cannot be written into the \
             skill index of AGENTS.md: 'skills/mail/SKILL.md' (its description holds \
             \"@support\"). The index writes each skill's name and description as Markdown \
             text outside code and its path as a link, and Claude Code may load an @ that \
             follows a space, a symbol (a code span, emphasis, a link or a backslash escape \
             included) or nothing as an import of another file. The adapter never edits or \
             renames a skill: edit the `name:` or `description:` named here in that SKILL.md \
             (write \"at\" or rephrase; an @ right after a letter or digit, as in \
             ops@example.com, is fine; a skill without `name:` is listed by its folder name), \
             or rename the folder of a path named here, then run the dry-run again"
                .to_string()
        ]
    );
}

/// The lone-surrogate refusal's path is formatted with this crate's own
/// `python_repr`, which single-quotes a path unless it holds a single
/// quote and no double quote -- then it switches to double quotes,
/// exactly like Python's own `repr()` (`python3 -c
/// "print(repr(\"skills/it's/SKILL.md\"))"` prints
/// `"skills/it's/SKILL.md"`).
#[test]
fn verify_refuses_a_skill_path_holding_an_apostrophe_in_double_quotes() {
    let project = TempDir::new("verify-apostrophe-path");
    let workspace = project.workspace();
    let agents_bytes = b"# Sample\n\nYou are Sample, a fixture agent.\n";
    let skill_bytes = br#"---
name: "mail-helper"
description: "x\ud800y"
---
"#
    .to_vec();
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
        .skill("skills/it's/SKILL.md", skill_bytes)
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "refused (unsafe_skill_metadata): these local skills cannot be written into the \
             skill index of AGENTS.md: \"skills/it's/SKILL.md\" (its description holds \
             invalid Unicode: the lone surrogate U+D800, an unpaired \\u escape, cannot be \
             written as UTF-8). The index quotes each skill's name and description in \
             AGENTS.md, which is UTF-8 text, and a lone surrogate has no UTF-8 form. No \
             skill is ever edited for you: in each SKILL.md named here, edit the `name:` or \
             `description:` named so that no \\u escape stands alone (write the character \
             itself, or a full pair such as \\ud83d\\ude00), then run the dry-run again"
                .to_string()
        ]
    );
}

/// (f) A hand-edited skills block: its recorded sha256 no longer
/// matches its body, so `verify` refuses rather than planning to
/// regenerate over a human's own edit (the prototype's own
/// `EditedBlock("edited_block", ...)`, raised by `plan_block` before any
/// row is ever generated).
#[test]
fn verify_refuses_a_hand_edited_skills_block() {
    let project = TempDir::new("verify-edited-block");
    let workspace = project.workspace();
    let hasher = FakeHasher;
    let digest = |bytes: &[u8]| hasher.sha256(bytes);
    let body = "## Local skill index\n\n| Skill | Use when | Path |\n| --- | --- | --- |\n\
                | sample | x | [sample](skills/sample/SKILL.md) |"
        .to_string();
    let block = ManagedBlock::skills(body, &digest).expect("a non-empty body builds a block");
    let rendered = block.render(LineEnding::Lf);
    let tampered = rendered.replace("sample", "sample-edited");
    let agents_bytes = format!("# Sample\n\nYou are Sample.\n\n{tampered}\n").into_bytes();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, &agents_bytes),
                identity(IdentityKind::Claude, WRAPPER),
            ],
            Vec::new(),
            Vec::new(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "refused (edited_block): the skill index block was edited by hand (its recorded \
             sha256 no longer matches) and cannot be regenerated; restore it or index the \
             precedence rule by hand"
                .to_string()
        ]
    );
}

/// (g) `CLAUDE.md` and `AGENTS.md` hold different, content-bearing
/// text: `"state is conflict"`, plus the `CONFLICT` refusal
/// `build_plan` raises before `source` can ever be resolved (the
/// prototype's own `resolve_source` returning `None`, read back as
/// `plan.refusal`).
#[test]
fn verify_reports_conflict_state_and_refusal() {
    let project = TempDir::new("verify-conflict");
    let workspace = project.workspace();
    let agents_bytes = b"# Sample\n\nAgents version of the instructions.\n";
    let claude_bytes = b"# Sample\n\nClaude version, totally different content.\n";
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, agents_bytes),
                identity(IdentityKind::Claude, claude_bytes),
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

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Conflict);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "state is conflict".to_string(),
            "refused (conflict): CLAUDE.md and AGENTS.md hold different content; choose the \
             canonical file with --canonical-from CLAUDE.md or --canonical-from AGENTS.md"
                .to_string(),
        ]
    );
}

// -- verify: skills-block stability --

/// (h) A skills block this crate already generated for its one
/// discovered skill, including the precedence rule, reports no problem
/// at all: `verify` must not regenerate a block that already says
/// exactly what a fresh render would say. Before the fix, the
/// regenerate decision read `rule_needed` from
/// [`omnifrons_domain::skills_index::SkillIndexStatus::precedence`]
/// over the *whole* canonical text; since the block's own rule section
/// always satisfies that whole-text check, `rule_needed` was always
/// `false` once a block existed, so the fresh render always omitted
/// the rule section this very block carries, never matched
/// `located.body`, and `verify` reported `pending: regenerate ...`
/// forever for an otherwise canonical agent. The fix reads it from the
/// text *before* the block instead, exactly as the prototype's own
/// `plan_block` does (`rule = not has_precedence_phrases(prefix)`), so
/// a block already stating the rule renders the same body again.
#[test]
fn verify_reports_canonical_for_a_block_this_crate_already_generated() {
    let project = TempDir::new("verify-stable-block");
    let workspace = project.workspace();
    let hasher = FakeHasher;
    let digest = |bytes: &[u8]| hasher.sha256(bytes);
    let row = SkillRow {
        name: "sample".to_string(),
        description: "A sample skill".to_string(),
        path: "skills/sample/SKILL.md".to_string(),
        had_undecodable_bytes: false,
    };
    let body = render_body(&[row], true);
    let block = ManagedBlock::skills(body, &digest).expect("a non-empty body builds a block");
    let rendered = block.render(LineEnding::Lf);
    let agents_bytes =
        format!("# Sample\n\nYou are Sample, a fixture agent.\n\n{rendered}\n").into_bytes();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, &agents_bytes),
                identity(IdentityKind::Claude, WRAPPER),
            ],
            Vec::new(),
            Vec::new(),
        )
        .skill(
            "skills/sample/SKILL.md",
            b"---\nname: sample\ndescription: A sample skill\n---\n".to_vec(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(report.canonical, "{:?}", report.problems);
    assert_eq!(report.problems, Vec::<String>::new());
}

/// (i) The same block, but a second skill was discovered since it was
/// generated: `verify` reports exactly one `pending: regenerate ...`,
/// naming both rows and the precedence rule -- never the `append`
/// shape a `None`-located block would report, and never silently
/// dropping the rule the way the pre-fix whole-text `rule_needed` did
/// (the previous test). The rule is still needed because the text
/// before the block never states it on its own; the prototype's own
/// `plan_block` computes `rule` the same way, over `prefix`, never the
/// whole file.
#[test]
fn verify_reports_one_regenerate_pending_when_a_skill_was_added_since() {
    let project = TempDir::new("verify-stale-block");
    let workspace = project.workspace();
    let hasher = FakeHasher;
    let digest = |bytes: &[u8]| hasher.sha256(bytes);
    let sample_row = SkillRow {
        name: "sample".to_string(),
        description: "A sample skill".to_string(),
        path: "skills/sample/SKILL.md".to_string(),
        had_undecodable_bytes: false,
    };
    let body = render_body(&[sample_row], true);
    let block = ManagedBlock::skills(body, &digest).expect("a non-empty body builds a block");
    let rendered = block.render(LineEnding::Lf);
    let agents_bytes =
        format!("# Sample\n\nYou are Sample, a fixture agent.\n\n{rendered}\n").into_bytes();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, &agents_bytes),
                identity(IdentityKind::Claude, WRAPPER),
            ],
            Vec::new(),
            Vec::new(),
        )
        .skill(
            "skills/sample/SKILL.md",
            b"---\nname: sample\ndescription: A sample skill\n---\n".to_vec(),
        )
        .skill(
            "skills/newer/SKILL.md",
            b"---\nname: newer\ndescription: A newer skill\n---\n".to_vec(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "pending: regenerate the skill index block in AGENTS.md (2 skill rows and the \
             precedence rule)"
                .to_string()
        ]
    );
}

/// A block that already exists, already covers every discovered skill,
/// and already states the precedence rule -- all from the text
/// *before* the block -- is left alone exactly as the prototype's own
/// `plan_block` leaves it: its upfront gate (`if not status.missing and
/// status.precedence: return None`, read over the *whole* canonical
/// text, which already counts the block's own rows and rule section as
/// coverage) returns with nothing pending before the block is ever
/// compared to a fresh render of it. A stale row the block itself
/// still carries -- here, for the one skill the prefix text now also
/// links on its own -- never turns into a `pending: regenerate ... ()`
/// with an empty parenthetical.
#[test]
fn verify_reports_nothing_pending_for_a_stale_block_the_prefix_already_covers() {
    let project = TempDir::new("verify-stale-block-covered");
    let workspace = project.workspace();
    let hasher = FakeHasher;
    let digest = |bytes: &[u8]| hasher.sha256(bytes);
    let sample_row = SkillRow {
        name: "sample".to_string(),
        description: "A sample skill".to_string(),
        path: "skills/sample/SKILL.md".to_string(),
        had_undecodable_bytes: false,
    };
    // Generated when the skill was not yet linked anywhere else; the
    // block still carries that row and the rule, even though the text
    // below now links the skill on its own, before the block.
    let body = render_body(&[sample_row], true);
    let block = ManagedBlock::skills(body, &digest).expect("a non-empty body builds a block");
    let rendered = block.render(LineEnding::Lf);
    let agents_bytes = format!(
        "# Sample\n\nYou are Sample, a fixture agent. See [sample](skills/sample/SKILL.md) \
         for the sample skill.\n\n{PRECEDENCE_RULE}\n\n{rendered}\n"
    )
    .into_bytes();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, &agents_bytes),
                identity(IdentityKind::Claude, WRAPPER),
            ],
            Vec::new(),
            Vec::new(),
        )
        .skill(
            "skills/sample/SKILL.md",
            b"---\nname: sample\ndescription: A sample skill\n---\n".to_vec(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(report.canonical, "{:?}", report.problems);
    assert_eq!(report.problems, Vec::<String>::new());
}

// -- verify: legacy skills-block migration --

/// (j) A legacy `portable-agent:skills:begin` block and a discovered
/// skill it does not yet link: before the fix, `verify` reported both
/// the ordinary `pending: append ...` [`skills_block_outcome`] would
/// plan for a `None`-located block, and a separate
/// `pending: migrate ...`, naming two indexes beside each other. The
/// prototype's own `plan_block` always finds at most one block
/// (`find_block`) and always plans exactly one operation once a block
/// is found (`replace_block`, never `append_block` too); mirroring
/// that, the migration is the *only* skills-block operation once a
/// legacy marker is found, replacing the ordinary pending rather than
/// sitting beside it.
#[test]
fn verify_reports_only_migrate_for_a_legacy_block_with_an_unlinked_skill() {
    let project = TempDir::new("verify-legacy-append");
    let workspace = project.workspace();
    let hasher = FakeHasher;
    // The body between the markers is empty (they are adjacent lines),
    // so its digest is the recorded one `verify` now checks: `verify`
    // must find this block intact, never a `block_layout`/`edited_block`
    // refusal, for the "migrate replaces append" behavior below to be
    // the thing actually under test.
    let digest_hex = hasher.sha256(b"").to_hex();
    let legacy_block = format!(
        "<!-- portable-agent:skills:begin sha256={digest_hex} -->\n<!-- \
         portable-agent:skills:end -->"
    );
    let agents_bytes =
        format!("# Sample\n\nYou are Sample, a fixture agent.\n\n{legacy_block}\n").into_bytes();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, &agents_bytes),
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
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "pending: migrate the portable-agent skill index block in AGENTS.md to the \
             current omnifrons sentinel format"
                .to_string()
        ]
    );
}

// -- verify: untested branches pinned exactly --

/// (k) A malformed skills-block sentinel pair -- a begin line with no
/// matching end -- refuses with `block_layout`, naming the skill
/// `verify` cannot index without one, rather than silently treating
/// the block as absent: the prototype's own `find_block` never reports
/// no block once it has seen a begin marker (`begins=1, ends=0` still
/// returns an `IndexBlock`, just not an intact one), and `plan_block`
/// raises `EditedBlock("block_layout", ...)` for it.
#[test]
fn verify_refuses_a_malformed_skills_block_sentinel_pair() {
    let project = TempDir::new("verify-block-layout");
    let workspace = project.workspace();
    let hasher = FakeHasher;
    let digest = |bytes: &[u8]| hasher.sha256(bytes);
    let placeholder =
        ManagedBlock::skills("placeholder", &digest).expect("a non-empty body builds a block");
    let begin_line = placeholder.begin_sentinel();
    let agents_bytes =
        format!("# Sample\n\nYou are Sample, a fixture agent.\n\n{begin_line}\n").into_bytes();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, &agents_bytes),
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
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "refused (block_layout): the skill index block cannot be regenerated: its \
             sentinels are not one intact pair. Fix the markers by hand or index \
             skills/sample/SKILL.md by hand"
                .to_string()
        ]
    );
}

/// (l) A lone, unpaired `\u` escape in a skill's front matter refuses
/// before any row is generated, wording the singular and plural forms
/// correctly -- one surrogate in one skill's `description:`, two
/// (non-adjacent high surrogates, so neither pairs up) in another's --
/// and quoting each path with the prototype's own `repr()` convention
/// (the prototype's `invalid_unicode_fields`).
#[test]
fn verify_refuses_skills_whose_metadata_holds_lone_surrogates() {
    let project = TempDir::new("verify-invalid-unicode");
    let workspace = project.workspace();
    let agents_bytes = b"# Sample\n\nYou are Sample, a fixture agent.\n";
    let one_bytes = br#"---
name: "one"
description: "x\ud800y"
---
"#
    .to_vec();
    let two_bytes = br#"---
name: "two"
description: "x\ud800y\ud801z"
---
"#
    .to_vec();
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
        .skill("skills/one/SKILL.md", one_bytes)
        .skill("skills/two/SKILL.md", two_bytes)
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "refused (unsafe_skill_metadata): these local skills cannot be written into the \
             skill index of AGENTS.md: 'skills/one/SKILL.md' (its description holds invalid \
             Unicode: the lone surrogate U+D800, an unpaired \\u escape, cannot be written as \
             UTF-8); 'skills/two/SKILL.md' (its description holds invalid Unicode: the lone \
             surrogates U+D800, U+D801, unpaired \\u escapes, cannot be written as UTF-8). \
             The index quotes each skill's name and description in AGENTS.md, which is UTF-8 \
             text, and a lone surrogate has no UTF-8 form. No skill is ever edited for you: \
             in each SKILL.md named here, edit the `name:` or `description:` named so that no \
             \\u escape stands alone (write the character itself, or a full pair such as \
             \\ud83d\\ude00), then run the dry-run again"
                .to_string()
        ]
    );
}

/// (m) The same legacy marker, but its rows and precedence rule are
/// already current: `block_needed` alone would say nothing is pending,
/// yet `verify` still reports the migration, because the block's
/// *family* (the prototype's own sentinel, not this crate's) is itself
/// the problem. Unaffected by the fix above, since this path never
/// reaches `skills_block_outcome`'s `Pending` branch -- it already
/// passes before the fix; no RED needed for this shape.
#[test]
fn verify_reports_migrate_for_a_legacy_block_whose_rows_are_current() {
    let project = TempDir::new("verify-legacy-current");
    let workspace = project.workspace();
    let hasher = FakeHasher;
    // The recorded digest must equal the hasher's own digest of this
    // exact body (the prototype's own `sha256_hex(body)`), so it is
    // computed here instead of a placeholder: `verify` now checks it.
    let body = format!(
        "## Local skill index\n\n\
         | Skill | Use when | Path |\n\
         | --- | --- | --- |\n\
         | sample | x | [sample](skills/sample/SKILL.md) |\n\n\
         ## Local skill precedence\n\n\
         {PRECEDENCE_RULE}\n"
    );
    let digest_hex = hasher.sha256(body.as_bytes()).to_hex();
    let legacy_block = format!(
        "<!-- portable-agent:skills:begin sha256={digest_hex} -->\n{body}<!-- \
         portable-agent:skills:end -->"
    );
    let agents_bytes =
        format!("# Sample\n\nYou are Sample, a fixture agent.\n\n{legacy_block}\n").into_bytes();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, &agents_bytes),
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
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "pending: migrate the portable-agent skill index block in AGENTS.md to the \
             current omnifrons sentinel format"
                .to_string()
        ]
    );
}

/// An intact-shaped legacy block -- one begin marker, one end marker
/// right after it, nothing following -- whose body was altered by a
/// single byte after its digest was recorded: `verify` refuses exactly
/// as the prototype's own `plan_block` does for `block.edited` alone
/// (no layout problem), never reporting the migration over a body that
/// no longer matches what was recorded.
#[test]
fn verify_refuses_a_legacy_block_whose_body_no_longer_matches_its_digest() {
    let project = TempDir::new("verify-legacy-edited");
    let workspace = project.workspace();
    let hasher = FakeHasher;
    // The digest is recorded for "sample\n"; the text on disk holds
    // "sbmple\n" instead -- one byte changed, so the two no longer
    // agree.
    let digest_hex = hasher.sha256(b"sample\n").to_hex();
    let legacy_block = format!(
        "<!-- portable-agent:skills:begin sha256={digest_hex} -->\nsbmple\n<!-- \
         portable-agent:skills:end -->"
    );
    let agents_bytes =
        format!("# Sample\n\nYou are Sample, a fixture agent.\n\n{legacy_block}\n").into_bytes();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, &agents_bytes),
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
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "refused (edited_block): the skill index block was edited by hand (its recorded \
             sha256 no longer matches) and cannot be regenerated; restore it or index \
             skills/sample/SKILL.md by hand"
                .to_string()
        ]
    );
}

/// A legacy begin marker with no end marker anywhere after it: `verify`
/// refuses with the prototype's own dynamic `block_layout` wording
/// (`find_block`'s own "has no end marker after it" cause), never the
/// simplified text this port's own omnifrons-block refusal uses --
/// `ManagedFileError` never applies to the legacy family, since there
/// is no `ManagedBlock` of this crate's own sentinel for it to locate.
#[test]
fn verify_refuses_an_unpaired_legacy_begin_marker() {
    let project = TempDir::new("verify-legacy-unpaired-begin");
    let workspace = project.workspace();
    let hasher = FakeHasher;
    let agents_bytes = format!(
        "# Sample\n\nYou are Sample, a fixture agent.\n\n<!-- portable-agent:skills:begin \
         sha256={} -->\n",
        "0".repeat(64)
    )
    .into_bytes();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, &agents_bytes),
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
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "refused (block_layout): the skill index block cannot be regenerated: the begin \
             marker on line 5 has no end marker after it. Fix the markers by hand or index \
             skills/sample/SKILL.md by hand"
                .to_string()
        ]
    );
}

/// (n) `CLAUDE.md` imports `AGENTS.md` but is not byte-exact (missing
/// its trailing newline): `verify` reports the pending wrapper write
/// with `describe_operation`'s own `"replaces sha256 ..."` wording,
/// the first 12 hex characters of the *current* `CLAUDE.md` bytes'
/// own digest -- never `AGENTS.md`'s, and never the `"creates the
/// file"` wording [`RootState::AgentsOnly`] gets when there is no
/// `CLAUDE.md` to replace at all.
#[test]
fn verify_reports_near_wrapper_state_and_replaces_sha256_of_claude() {
    let project = TempDir::new("verify-near-wrapper");
    let workspace = project.workspace();
    let claude_bytes = b"@AGENTS.md";
    let agents_bytes = b"# Sample\n\nYou are Sample, a fixture agent.\n";
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, agents_bytes),
                identity(IdentityKind::Claude, claude_bytes),
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

    let report = verify(&ports, &workspace).expect("verify succeeds");

    let wrapper_pending = format!(
        "pending: write the wrapper CLAUDE.md (replaces sha256 {})",
        &hasher.sha256(claude_bytes).to_hex()[..12]
    );
    assert_eq!(report.state, RootState::NearWrapper);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec!["state is near_wrapper".to_string(), wrapper_pending]
    );
}

/// (o) `CLAUDE.md` is a symlink to something other than `AGENTS.md`,
/// and no `AGENTS.md` exists to accept it as the one link `classify`
/// trusts: `"state is unsafe"`, plus the refusal `build_plan` raises
/// for it (`report.classification.reason`, read back as-is).
#[test]
fn verify_reports_unsafe_state_and_refusal_for_a_stray_claude_symlink() {
    let project = TempDir::new("verify-unsafe");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![symlinked_identity(IdentityKind::Claude, "nowhere.md")],
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

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Unsafe);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "state is unsafe".to_string(),
            "refused (unsafe): CLAUDE.md is a symlink to nowhere.md".to_string(),
        ]
    );
}

/// (p) No identity file at all: [`RootState::None`] yields exactly
/// `"state is none"`, the prototype's own `if report.state == NONE:
/// return plan` read back through `verify_agent` -- an unrefused,
/// empty plan, so no refusal and no pending operation ever joins it.
#[test]
fn verify_reports_none_state_with_only_the_state_problem() {
    let project = TempDir::new("verify-none-state");
    let workspace = project.workspace();
    let snapshot = TreeSnapshotBuilder::new().build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::None);
    assert!(!report.canonical);
    assert_eq!(report.problems, vec!["state is none".to_string()]);
}

/// (q) `CLAUDE.md` holds the instructions and there is no `AGENTS.md`
/// at all: `classify` names `CLAUDE.md` as the source
/// ([`RootState::ClaudeOnly`]), so the skills check reads *its* bytes,
/// not a nonexistent `AGENTS.md` -- a wrong source would make
/// `canonical_bytes` return `None`, and this pending text would never
/// appear at all.
#[test]
fn verify_reads_claude_md_as_the_source_for_claude_only_state() {
    let project = TempDir::new("verify-claude-only-source");
    let workspace = project.workspace();
    let claude_bytes = b"# Sample\n\nYou are Sample, a fixture agent.\n";
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![identity(IdentityKind::Claude, claude_bytes)],
            Vec::new(),
            Vec::new(),
        )
        .skill(
            "skills/sample/SKILL.md",
            b"---\nname: sample\n---\n".to_vec(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::ClaudeOnly);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "state is claude_only".to_string(),
            "pending: append the skill index block in AGENTS.md (1 skill rows and the \
             precedence rule)"
                .to_string(),
        ]
    );
}

/// (r) `CLAUDE.md` and `AGENTS.md` are byte-identical but neither the
/// wrapper nor an import: [`RootState::IdenticalCopies`] still names
/// `AGENTS.md` as the source (`classify_pair`'s own
/// `(RootState::IdenticalCopies, IdentityKind::Agents, ...)`), so the
/// skills check and its pending text are unaffected by the duplicate
/// `CLAUDE.md`.
#[test]
fn verify_reads_agents_md_as_the_source_for_identical_copies_state() {
    let project = TempDir::new("verify-identical-copies-source");
    let workspace = project.workspace();
    let shared_bytes = b"# Sample\n\nYou are Sample, a fixture agent.\n";
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, shared_bytes),
                identity(IdentityKind::Claude, shared_bytes),
            ],
            Vec::new(),
            Vec::new(),
        )
        .skill(
            "skills/sample/SKILL.md",
            b"---\nname: sample\n---\n".to_vec(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::IdenticalCopies);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "state is identical_copies".to_string(),
            "pending: append the skill index block in AGENTS.md (1 skill rows and the \
             precedence rule)"
                .to_string(),
        ]
    );
}

/// When both the `CLAUDE.md` wrapper write and a skills-block operation
/// are pending at once, `verify` reports the wrapper first: the
/// prototype's own `plan_operations` places the wrapper write among
/// its returned steps, and `with_block_step` always appends the
/// skill-index block operation last, after every step
/// `plan_operations` already returned.
#[test]
fn verify_reports_the_wrapper_write_before_the_skills_block_pending() {
    let project = TempDir::new("verify-wrapper-then-block");
    let workspace = project.workspace();
    let agents_bytes = b"# Sample\n\nYou are Sample, a fixture agent.\n";
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![identity(IdentityKind::Agents, agents_bytes)],
            Vec::new(),
            Vec::new(),
        )
        .skill(
            "skills/sample/SKILL.md",
            b"---\nname: sample\n---\n".to_vec(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::AgentsOnly);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "state is agents_only".to_string(),
            "pending: write the wrapper CLAUDE.md (creates the file)".to_string(),
            "pending: append the skill index block in AGENTS.md (1 skill rows and the \
             precedence rule)"
                .to_string(),
        ]
    );
}

// -- verify: a real legacy block, not merely a mentioned marker --

/// A legacy begin marker line inside a fenced code block still counts
/// as a real marker: the prototype's own `find_block` matches a marker
/// line anywhere in the file, fenced or not -- fencing there only ever
/// *explains* a problem `find_block` already found some other way, it
/// never changes whether a marker counts, the same role its own
/// `quoted_markers` plays. Here the fenced begin has no end marker
/// anywhere in the file, so `verify` refuses with both the
/// fenced-marker explanation and the unpaired-begin cause, in that
/// order -- never treating a hand-migration example in the agent's own
/// prose as harmless content the way the pre-fix fence-skipping scan
/// did.
#[test]
fn verify_refuses_a_fenced_legacy_begin_marker_with_no_end() {
    let project = TempDir::new("verify-legacy-fenced-begin");
    let workspace = project.workspace();
    let hasher = FakeHasher;
    let agents_bytes = format!(
        "# Sample\n\nYou are Sample, a fixture agent.\n\nTo migrate a legacy block by \
         hand, look for a line like this:\n\n```\n<!-- portable-agent:skills:begin \
         sha256={} -->\n```\n",
        "0".repeat(64)
    )
    .into_bytes();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, &agents_bytes),
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
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "refused (block_layout): the skill index block cannot be regenerated: the marker \
             on line 8 is inside a fenced code block; the adapter matches its markers \
             anywhere, so a quoted marker reads as a broken block; the begin marker on line \
             8 has no end marker after it. Fix the markers by hand or index \
             skills/sample/SKILL.md by hand"
                .to_string()
        ],
        "a fenced begin marker must still be found, and the fencing must only explain the \
         problem, never hide it"
    );
}

/// The same marker, mentioned inline in an ordinary sentence rather
/// than fenced: still never a full, standalone line, so it is content
/// by the same rule `find_block`'s own full-line regex already applies
/// -- the whole-text `contains` check this fix replaces could not tell
/// this apart from a real marker.
#[test]
fn verify_treats_a_prose_legacy_marker_mention_as_content() {
    let project = TempDir::new("verify-legacy-prose-mention");
    let workspace = project.workspace();
    let agents_bytes = b"# Sample\n\nYou are Sample, a fixture agent.\n\nOlder agents \
        carried a block starting with `<!-- portable-agent:skills:begin sha256=... -->` \
        and ending with `<!-- portable-agent:skills:end -->`, a format this project no \
        longer writes.\n"
        .to_vec();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, &agents_bytes),
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
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "pending: append the skill index block in AGENTS.md (1 skill rows and the \
             precedence rule)"
                .to_string()
        ],
        "a marker mentioned in prose must not read as a real legacy block"
    );
}

/// A real legacy block beside an already-intact omnifrons block: the
/// prototype's own `find_block` has no notion of this crate's own
/// sentinel at all, so it checks the legacy pair exactly as it would
/// alone -- including its own "the block must be the last thing in
/// the file" rule, which this fixture respects by placing the legacy
/// pair *after* the omnifrons block, not before: `find_block` reads
/// everything past the legacy end marker as the legacy block's own
/// tail, omnifrons sentinel or not, so a trailing omnifrons block
/// would otherwise read as disqualifying text following the end
/// marker. With the legacy pair last, the legacy scan finds it intact
/// regardless of the omnifrons block sitting before it, and the
/// existing "replace, never both" rule in `verify_problems_after_state`
/// (unchanged by this fix) then reports the migration alone, even
/// though the omnifrons block on its own would need nothing.
#[test]
fn verify_reports_migrate_for_a_legacy_block_beside_an_intact_omnifrons_block() {
    let project = TempDir::new("verify-legacy-beside-intact");
    let workspace = project.workspace();
    let hasher = FakeHasher;
    let digest = |bytes: &[u8]| hasher.sha256(bytes);
    let row = SkillRow {
        name: "sample".to_string(),
        description: "A sample skill".to_string(),
        path: "skills/sample/SKILL.md".to_string(),
        had_undecodable_bytes: false,
    };
    let body = render_body(&[row], true);
    let block = ManagedBlock::skills(body, &digest).expect("a non-empty body builds a block");
    let rendered = block.render(LineEnding::Lf);
    // Empty body between adjacent markers again (as in the "unlinked
    // skill" case above): its digest is the recorded one, so this
    // legacy block is intact and the migration is reported, unchanged.
    let legacy_digest_hex = hasher.sha256(b"").to_hex();
    let legacy_block = format!(
        "<!-- portable-agent:skills:begin sha256={legacy_digest_hex} -->\n<!-- \
         portable-agent:skills:end -->"
    );
    let agents_bytes =
        format!("# Sample\n\nYou are Sample, a fixture agent.\n\n{rendered}\n\n{legacy_block}\n")
            .into_bytes();
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, &agents_bytes),
                identity(IdentityKind::Claude, WRAPPER),
            ],
            Vec::new(),
            Vec::new(),
        )
        .skill(
            "skills/sample/SKILL.md",
            b"---\nname: sample\ndescription: A sample skill\n---\n".to_vec(),
        )
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(!report.canonical);
    assert_eq!(
        report.problems,
        vec![
            "pending: migrate the portable-agent skill index block in AGENTS.md to the \
             current omnifrons sentinel format"
                .to_string()
        ]
    );
}

// -- verify: a nested agent's own skill never joins the root's rows --

/// A nested agent directory with its own identity and its own skill
/// contributes nothing to the scan root's own `verify`: the root here
/// has none of its own skills, so `verify` must report nothing at all,
/// not a missing or pending entry for the nested agent's
/// `skills/x/SKILL.md` -- that skill's nearest root is `child`, never
/// the scan root, so it never reaches the root's own `discovered` list
/// or its skill rows.
#[test]
fn verify_excludes_a_nested_agents_own_skill_from_the_root_report() {
    let project = TempDir::new("verify-nested-skill-excluded");
    let workspace = project.workspace();
    let root_bytes = b"# Root\n\nYou are Root, a fixture agent.\n";
    let child_bytes = b"# Child\n\nYou are Child.\n\n[x](skills/x/SKILL.md)\n";
    let snapshot = TreeSnapshotBuilder::new()
        .directory(
            ".",
            vec![
                identity(IdentityKind::Agents, root_bytes),
                identity(IdentityKind::Claude, WRAPPER),
            ],
            Vec::new(),
            Vec::new(),
        )
        .directory(
            "child",
            vec![identity(IdentityKind::Agents, child_bytes)],
            Vec::new(),
            Vec::new(),
        )
        .skill("child/skills/x/SKILL.md", b"---\nname: x\n---\n".to_vec())
        .build();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };

    let report = verify(&ports, &workspace).expect("verify succeeds");

    assert_eq!(report.state, RootState::Canonical);
    assert!(
        report.problems.is_empty(),
        "the root must report nothing for a skill the nested agent owns, got {:?}",
        report.problems
    );
    assert!(
        !report
            .problems
            .iter()
            .any(|problem| problem.contains("skills/x/SKILL.md")),
        "no problem must name the nested agent's own skill, got {:?}",
        report.problems
    );
}
