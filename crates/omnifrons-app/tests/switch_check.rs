//! The switch check over the in-memory definition tree: which file a
//! host reads from the active project root, and the definition's
//! reference (path and digest) for a startup brief.

#![cfg(feature = "contract-tests")]

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::content_hasher::ContentHasher;
use omnifrons_app::contract::definition::{InMemoryDefinitionTree, TreeSnapshotBuilder};
use omnifrons_app::contract::publication::FakeHasher;
use omnifrons_app::definition::DefinitionPorts;
use omnifrons_app::definition_tree::TreeSnapshot;
use omnifrons_app::switch_check::{SwitchCheck, switch_check};
use omnifrons_domain::adapter::AdapterId;
use omnifrons_domain::definition::{FileShape, IdentityFile, IdentityKind};
use omnifrons_domain::switch_check::{DefinitionReference, SwitchVerdict, UncertainReason};

const AGENTS_BYTES: &[u8] = b"# Sample\n\nYou are Sample.\n";

fn workspace() -> WorkspaceRoot {
    WorkspaceRoot::new(std::env::temp_dir()).expect("valid workspace")
}

fn file(kind: IdentityKind, bytes: &[u8]) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: bytes.to_vec(),
        shape: FileShape::Regular,
    }
}

fn linked(kind: IdentityKind) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: Vec::new(),
        shape: FileShape::Symlink {
            target: "elsewhere.md".to_string(),
        },
    }
}

fn root(files: Vec<IdentityFile>) -> TreeSnapshotBuilder {
    TreeSnapshotBuilder::new().directory(".", files, Vec::new(), Vec::new())
}

fn check(snapshot: TreeSnapshot, adapter: &AdapterId, host: Option<&str>) -> SwitchCheck {
    let workspace = workspace();
    let tree = InMemoryDefinitionTree::new().with_snapshot(&workspace, snapshot);
    let hasher = FakeHasher;
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };
    switch_check(&ports, &workspace, adapter, host).expect("check succeeds")
}

#[test]
fn claude_code_reaches_a_root_with_claude_md() {
    let snapshot = root(vec![
        file(IdentityKind::Agents, AGENTS_BYTES),
        file(IdentityKind::Claude, b"@AGENTS.md\n"),
    ])
    .build();
    let result = check(snapshot, &AdapterId::claude_code(), None);
    assert_eq!(result.host.as_deref(), Some("Claude Code"));
    assert!(
        matches!(&result.verdict, SwitchVerdict::Reaches { file, .. } if file == "CLAUDE.md"),
        "{:?}",
        result.verdict
    );
}

#[test]
fn claude_code_reaches_a_root_with_only_agents_md_through_the_fallback() {
    let snapshot = root(vec![file(IdentityKind::Agents, AGENTS_BYTES)]).build();
    let result = check(snapshot, &AdapterId::claude_code(), None);
    assert!(
        matches!(&result.verdict, SwitchVerdict::Reaches { file, .. } if file == "AGENTS.md"),
        "{:?}",
        result.verdict
    );
}

#[test]
fn claude_code_is_unreached_when_an_ancestor_claude_md_shadows_the_fallback() {
    let snapshot = root(vec![file(IdentityKind::Agents, AGENTS_BYTES)])
        .ancestor("/outer/inner", vec!["AGENTS.md".to_string()], Vec::new())
        .ancestor("/outer", vec!["CLAUDE.md".to_string()], Vec::new())
        .build();
    let result = check(snapshot, &AdapterId::claude_code(), None);
    match &result.verdict {
        SwitchVerdict::Unreached {
            shadowed_by: Some(shadow),
            ..
        } => {
            assert_eq!(shadow.path, "/outer");
            assert_eq!(shadow.file, "CLAUDE.md");
        }
        other => panic!("expected a shadowed verdict, got {other:?}"),
    }
}

#[test]
fn a_nested_identity_file_of_the_root_is_read_from_the_claude_dir() {
    let snapshot = root(vec![file(IdentityKind::ClaudeDir, b"notes\n")]).build();
    let result = check(snapshot, &AdapterId::claude_code(), None);
    assert!(
        matches!(&result.verdict, SwitchVerdict::Reaches { file, .. } if file == ".claude/CLAUDE.md"),
        "{:?}",
        result.verdict
    );
}

#[test]
fn a_generic_adapter_with_no_chosen_host_is_uncertain() {
    let snapshot = root(vec![file(IdentityKind::Agents, AGENTS_BYTES)]).build();
    let adapter = AdapterId::pty_cli();
    let result = check(snapshot, &adapter, None);
    assert_eq!(result.host, None);
    assert_eq!(
        result.verdict,
        SwitchVerdict::Uncertain {
            reason: UncertainReason::NoHostChosen { adapter }
        }
    );
}

#[test]
fn a_generic_adapter_with_gemini_cli_reaches_a_root_with_gemini_md() {
    let snapshot = root(vec![
        file(IdentityKind::Agents, AGENTS_BYTES),
        file(IdentityKind::Gemini, b"gemini\n"),
    ])
    .build();
    let result = check(snapshot, &AdapterId::pty_cli(), Some("Gemini CLI"));
    assert_eq!(result.host.as_deref(), Some("Gemini CLI"));
    assert!(
        matches!(&result.verdict, SwitchVerdict::Reaches { file, .. } if file == "GEMINI.md"),
        "{:?}",
        result.verdict
    );
}

#[test]
fn a_generic_adapter_with_gemini_cli_is_unreached_without_gemini_md() {
    let snapshot = root(vec![file(IdentityKind::Agents, AGENTS_BYTES)]).build();
    let result = check(snapshot, &AdapterId::stream_json_cli(), Some("Gemini CLI"));
    assert!(
        matches!(
            &result.verdict,
            SwitchVerdict::Unreached {
                shadowed_by: None,
                ..
            }
        ),
        "{:?}",
        result.verdict
    );
}

#[test]
fn the_reference_carries_agents_md_and_the_sha256_of_its_bytes() {
    let snapshot = root(vec![file(IdentityKind::Agents, AGENTS_BYTES)]).build();
    let result = check(snapshot, &AdapterId::claude_code(), None);
    assert_eq!(
        result.reference,
        Some(
            DefinitionReference::new("AGENTS.md", FakeHasher.sha256(AGENTS_BYTES).to_hex())
                .expect("a valid reference")
        )
    );
}

#[test]
fn the_reference_is_none_when_the_root_has_no_agents_md() {
    let snapshot = root(vec![file(IdentityKind::Claude, b"own text\n")]).build();
    assert_eq!(
        check(snapshot, &AdapterId::claude_code(), None).reference,
        None
    );
    let empty = TreeSnapshotBuilder::new().build();
    assert_eq!(
        check(empty, &AdapterId::claude_code(), None).reference,
        None
    );
}

#[test]
fn a_symlinked_agents_md_counts_as_absent_for_the_verdict_and_the_reference() {
    let snapshot = root(vec![linked(IdentityKind::Agents)]).build();
    let result = check(snapshot, &AdapterId::claude_code(), None);
    assert_eq!(result.reference, None);
    assert!(
        matches!(&result.verdict, SwitchVerdict::Unreached { .. }),
        "{:?}",
        result.verdict
    );
}

fn irregular(kind: IdentityKind) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: Vec::new(),
        shape: FileShape::Irregular,
    }
}

#[test]
fn an_irregular_agents_md_has_no_reference_and_counts_as_absent() {
    let snapshot = root(vec![irregular(IdentityKind::Agents)]).build();
    let result = check(snapshot, &AdapterId::claude_code(), None);
    assert_eq!(result.reference, None);
    assert!(
        matches!(&result.verdict, SwitchVerdict::Unreached { .. }),
        "{:?}",
        result.verdict
    );
}

#[test]
fn a_bound_adapter_ignores_a_conflicting_chosen_host() {
    let snapshot = root(vec![
        file(IdentityKind::Agents, AGENTS_BYTES),
        file(IdentityKind::Claude, b"@AGENTS.md\n"),
    ])
    .build();
    let result = check(snapshot, &AdapterId::claude_code(), Some("Gemini CLI"));
    assert_eq!(result.host.as_deref(), Some("Claude Code"));
    assert!(
        matches!(&result.verdict, SwitchVerdict::Reaches { file, claim } if file == "CLAUDE.md" && claim.host == "Claude Code"),
        "{:?}",
        result.verdict
    );
}

#[test]
fn an_ancestor_entry_spelled_as_a_kind_token_still_shadows() {
    let snapshot = root(vec![file(IdentityKind::Agents, AGENTS_BYTES)])
        .ancestor("/outer", vec!["claude".to_string()], Vec::new())
        .build();
    let result = check(snapshot, &AdapterId::claude_code(), None);
    match &result.verdict {
        SwitchVerdict::Unreached {
            shadowed_by: Some(shadow),
            fallback,
            ..
        } => {
            assert_eq!(shadow.file, "CLAUDE.md");
            assert_eq!(*fallback, Some("AGENTS.md"));
        }
        other => panic!("expected a shadowed verdict, got {other:?}"),
    }
}

#[test]
fn an_unrecognized_ancestor_entry_is_dropped_and_shadows_nothing() {
    let snapshot = root(vec![file(IdentityKind::Agents, AGENTS_BYTES)])
        .ancestor("/outer", vec!["NOTES.md".to_string()], Vec::new())
        .build();
    let result = check(snapshot, &AdapterId::claude_code(), None);
    assert!(
        matches!(&result.verdict, SwitchVerdict::Reaches { file, .. } if file == "AGENTS.md"),
        "{:?}",
        result.verdict
    );
}
