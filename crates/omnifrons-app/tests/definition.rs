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
use omnifrons_app::definition_tree::{DefinitionTree, DefinitionTreeError, TreeSnapshot};
use omnifrons_domain::definition::{FileShape, IdentityFile, IdentityKind, RootState, WRAPPER};
use omnifrons_domain::nesting::{NestingError, Parent};
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
