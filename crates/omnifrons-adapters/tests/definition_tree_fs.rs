//! Filesystem-backed tests for `FsDefinitionTree` (ADR-0005 sub-slice
//! 1f): the shared `DefinitionTree` port contract on a representative
//! temp tree, the four `base_omnifrons` prototype real-filesystem
//! scenarios translated end to end through `omnifrons_app::definition::scan`,
//! and adapter-level cases the prototype has no direct counterpart for
//! (the size cap, the symlink shape, and the two filesystem-level
//! refusals).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_adapters::{FsDefinitionTree, Sha2Hasher};
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::contract::definition::definition_tree_contract;
use omnifrons_app::definition::{AgentReport, DefinitionPorts, ScanReport, scan};
use omnifrons_app::definition_tree::{
    DefinitionTree, DefinitionTreeError, MAX_DEFINITION_ENTRY_BYTES,
};
use omnifrons_domain::definition::RootState;
#[cfg(unix)]
use omnifrons_domain::definition::{FileShape, IdentityKind};
use omnifrons_domain::nesting::{Ancestor, Parent};

const SIMPLE_BODY: &[u8] = b"# Simple\n\nYou are Simple, a fixture agent.\n";
const OTHER_BODY: &[u8] = b"# Simple\n\nYou are Simple, a different fixture agent.\n";
const WRAPPER: &[u8] = b"@AGENTS.md\n";

/// A drop-guard temp directory (mirrors every other `*_fs.rs` adapter
/// test in this crate; there is no `tempfile` dependency here).
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-definition-tree-fs-test-{}-{label}-{n}",
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

/// Writes `bytes` at `base`/`relative`, creating every parent directory
/// first.
fn write(base: &Path, relative: &str, bytes: &[u8]) {
    let target = base.join(relative);
    std::fs::create_dir_all(
        target
            .parent()
            .expect("a relative fixture path has a parent"),
    )
    .expect("mkdir for fixture");
    std::fs::write(&target, bytes).expect("write fixture file");
}

/// `path`'s canonical, `/`-separated absolute form -- the same shape
/// `FsDefinitionTree` itself reports for an
/// `omnifrons_domain::nesting::Ancestor::path`.
fn canonical_posix(path: &Path) -> String {
    std::fs::canonicalize(path)
        .expect("canonicalize")
        .to_string_lossy()
        .replace('\\', "/")
}

/// The entries of `ancestors` at or under `root_posix` (`canonical_posix`'s
/// own `/`-separated absolute form): every ancestor assertion in this
/// file filters through this helper first, so it only ever covers the
/// test's own temp tree, never whatever identity file or host extra
/// already happens to sit above it on the host actually running the
/// test -- the adapter's ancestor walk always climbs all the way to
/// the real filesystem root, so an assertion on the raw, unfiltered
/// list would be hostage to that host's own directory tree.
fn ancestors_under<'a>(ancestors: &'a [Ancestor], root_posix: &str) -> Vec<&'a Ancestor> {
    ancestors
        .iter()
        .filter(|ancestor| ancestor.path.starts_with(root_posix))
        .collect()
}

/// The agent at `path` in `report`, or a panic naming every path that
/// was actually there.
fn agent_at<'a>(report: &'a ScanReport, path: &str) -> &'a AgentReport {
    report
        .agents
        .iter()
        .find(|agent| agent.path == path)
        .unwrap_or_else(|| {
            let found: Vec<&str> = report
                .agents
                .iter()
                .map(|agent| agent.path.as_str())
                .collect();
            panic!("no agent at {path}; found {found:?}")
        })
}

// -- the shared contract --

/// `FsDefinitionTree` satisfies `definition_tree_contract` on a
/// representative tree: a root agent (via a host-config
/// `.claude/CLAUDE.md` attributed to it), an extras-only directory, a
/// nested agent, a discovered skill, and one `SKIP_DIRS` member holding
/// an `AGENTS.md` that must never surface.
#[test]
fn fs_definition_tree_satisfies_the_definition_tree_contract() {
    let project = TempDir::new("tree-contract");
    write(project.path(), ".claude/CLAUDE.md", SIMPLE_BODY);
    write(
        project.path(),
        "extras-only/CLAUDE.local.md",
        b"local notes\n",
    );
    write(project.path(), "nested-agent/AGENTS.md", SIMPLE_BODY);
    write(
        project.path(),
        "skills/demo/SKILL.md",
        b"---\nname: demo\n---\n",
    );
    write(project.path(), "node_modules/pkg/AGENTS.md", OTHER_BODY);
    let workspace = project.workspace();
    let tree = FsDefinitionTree::new(None);

    definition_tree_contract(&tree, &workspace);

    let snapshot = tree.snapshot(&workspace).expect("snapshot");
    let directory_paths: Vec<&str> = snapshot
        .directories
        .iter()
        .map(|directory| directory.path.as_str())
        .collect();
    assert_eq!(directory_paths, vec![".", "extras-only", "nested-agent"]);
    let skill_paths: Vec<&str> = snapshot
        .skills
        .iter()
        .map(|skill| skill.path.as_str())
        .collect();
    assert_eq!(skill_paths, vec!["skills/demo/SKILL.md"]);
    assert!(
        !directory_paths
            .iter()
            .any(|path| path.starts_with("node_modules")),
        "a SKIP_DIRS member must never surface: {directory_paths:?}"
    );
}

// -- the four prototype scenarios, translated through `scan` --

/// The prototype's `test_scan_nested_boundaries`: a nested agent reports
/// its enclosing root as `parent` and needs consent, the enclosing root
/// reports the nested agent in `nested` and needs no consent of its
/// own, and each agent's own discovered skills never include the
/// other's.
#[test]
fn nested_boundaries_report_parent_nested_consent_and_skill_ownership() {
    let base = TempDir::new("nested-boundaries");
    write(base.path(), "AGENTS.md", SIMPLE_BODY);
    write(
        base.path(),
        "skills/atlas-skill/SKILL.md",
        b"---\nname: atlas-skill\n---\n",
    );
    write(base.path(), "community/AGENTS.md", SIMPLE_BODY);
    write(
        base.path(),
        "community/skills/draft-reply/SKILL.md",
        b"---\nname: draft-reply\n---\n",
    );
    let workspace = base.workspace();
    let fs_tree = FsDefinitionTree::new(None);
    let hasher = Sha2Hasher::new();
    let ports = DefinitionPorts {
        tree: &fs_tree,
        hasher: &hasher,
    };

    let report = scan(&ports, &workspace).expect("scan succeeds");

    let paths: Vec<&str> = report
        .agents
        .iter()
        .map(|agent| agent.path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![".", "community"],
        "walk order: parent before nested child"
    );
    let parent = agent_at(&report, ".");
    let child = agent_at(&report, "community");
    assert_eq!(parent.nested, vec!["community".to_string()]);
    assert_eq!(parent.parent, None);
    assert!(!parent.consent_required);
    assert_eq!(child.parent, Some(Parent::Root(".".to_string())));
    assert!(child.consent_required);
    assert!(child.nested.is_empty());
    assert_eq!(
        parent.skills.discovered,
        vec!["skills/atlas-skill/SKILL.md".to_string()]
    );
    assert_eq!(
        child.skills.discovered,
        vec!["skills/draft-reply/SKILL.md".to_string()]
    );
}

/// The prototype's `test_scan_records_ancestors_outside_root`: scanning
/// a nested agent directly records its enclosing root as the one
/// ancestor, nearest first, holding both identity kinds it carries; a
/// deeper, agent-less scan root below that same enclosing root still
/// sees it as its only ancestor, with zero agents of its own.
#[test]
fn ancestors_outside_the_scan_root_are_recorded_nearest_first() {
    let base = TempDir::new("ancestors-outside");
    write(base.path(), "AGENTS.md", SIMPLE_BODY);
    write(base.path(), "CLAUDE.md", WRAPPER);
    write(base.path(), "community/AGENTS.md", SIMPLE_BODY);
    std::fs::create_dir_all(base.path().join("skills")).expect("mkdir skills");
    let outer_posix = canonical_posix(base.path());

    let fs_tree = FsDefinitionTree::new(None);
    let hasher = Sha2Hasher::new();
    let ports = DefinitionPorts {
        tree: &fs_tree,
        hasher: &hasher,
    };

    let inner_workspace = WorkspaceRoot::new(base.path().join("community")).expect("workspace");
    let report = scan(&ports, &inner_workspace).expect("scan succeeds");
    let paths: Vec<&str> = report
        .agents
        .iter()
        .map(|agent| agent.path.as_str())
        .collect();
    assert_eq!(paths, vec!["."]);
    let ancestors = ancestors_under(&report.ancestors, &outer_posix);
    let ancestor_paths: Vec<&str> = ancestors
        .iter()
        .map(|ancestor| ancestor.path.as_str())
        .collect();
    assert_eq!(ancestor_paths, vec![outer_posix.as_str()]);
    let mut kinds = ancestors[0].identity_files.clone();
    kinds.sort();
    assert_eq!(
        kinds,
        vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()]
    );
    let child = agent_at(&report, ".");
    assert_eq!(child.parent, Some(Parent::Outside(outer_posix.clone())));
    assert!(child.consent_required);

    let deeper_workspace = WorkspaceRoot::new(base.path().join("skills")).expect("workspace");
    let deeper_report = scan(&ports, &deeper_workspace).expect("scan succeeds");
    assert!(deeper_report.agents.is_empty());
    let deeper_ancestors = ancestors_under(&deeper_report.ancestors, &outer_posix);
    let deeper_ancestor_paths: Vec<&str> = deeper_ancestors
        .iter()
        .map(|ancestor| ancestor.path.as_str())
        .collect();
    assert_eq!(deeper_ancestor_paths, vec![outer_posix.as_str()]);
}

/// The prototype's `test_scan_prunes_ignored_and_host_config_dirs`,
/// minus its two symlinks (`symlinked_directories_are_never_descended_into`
/// below covers those): every `SKIP_DIRS` member is pruned whole --
/// its own identity files and skills never surface -- a host-config
/// directory's identity file is attributed to its parent, and the
/// result is `RootState::HostExtras` once a host extra sits alongside
/// an otherwise-canonical root.
#[test]
fn pruned_and_host_config_directories_never_become_agents_or_skills() {
    let base = TempDir::new("pruning");
    write(base.path(), "AGENTS.md", SIMPLE_BODY);
    write(base.path(), "CLAUDE.md", WRAPPER);
    write(base.path(), ".claude/CLAUDE.md", SIMPLE_BODY);
    write(base.path(), ".kimi/AGENTS.md", b"Kimi notes.\n");
    write(base.path(), ".git/CLAUDE.md", OTHER_BODY);
    write(base.path(), "node_modules/pkg/CLAUDE.md", OTHER_BODY);
    write(
        base.path(),
        "node_modules/pkg/SKILL.md",
        b"---\nname: pkg\n---\n",
    );
    write(base.path(), "skills/own/SKILL.md", b"---\nname: own\n---\n");
    let workspace = base.workspace();
    let fs_tree = FsDefinitionTree::new(None);
    let hasher = Sha2Hasher::new();
    let ports = DefinitionPorts {
        tree: &fs_tree,
        hasher: &hasher,
    };

    let report = scan(&ports, &workspace).expect("scan succeeds");

    let paths: Vec<&str> = report
        .agents
        .iter()
        .map(|agent| agent.path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["."],
        "node_modules/pkg must never become its own agent"
    );
    let root = agent_at(&report, ".");
    assert!(root.nested.is_empty());
    assert_eq!(root.state, RootState::HostExtras);
    assert!(
        root.skills
            .discovered
            .contains(&"skills/own/SKILL.md".to_string()),
        "{:?}",
        root.skills.discovered
    );
    assert!(
        !root
            .skills
            .discovered
            .iter()
            .any(|path| path.starts_with("node_modules/")),
        "{:?}",
        root.skills.discovered
    );
}

/// A host-config directory (`.claude`) is walked but never pruned the
/// way a `SKIP_DIRS` member is: `.claude/skills/x/SKILL.md` inside the
/// scan root is still discovered, and attributed to the *root* agent --
/// `.claude` itself is never a root to attribute it to -- exactly as
/// the prototype's own `walk_workspace` would discover it too, since
/// its own pruning touches only `SKIP_DIRS`
/// (`crates/omnifrons-domain/src/nesting.rs`'s own module documentation
/// states this rule).
#[test]
fn a_skill_inside_a_host_config_directory_is_attributed_to_the_enclosing_root() {
    let project = TempDir::new("skill-inside-host-config-dir");
    write(project.path(), "AGENTS.md", SIMPLE_BODY);
    write(
        project.path(),
        ".claude/skills/x/SKILL.md",
        b"---\nname: x\n---\n",
    );
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);
    let hasher = Sha2Hasher::new();
    let ports = DefinitionPorts {
        tree: &fs_tree,
        hasher: &hasher,
    };

    let report = scan(&ports, &workspace).expect("scan succeeds");

    let paths: Vec<&str> = report
        .agents
        .iter()
        .map(|agent| agent.path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["."],
        "a host-config directory never becomes its own agent root"
    );
    let root = agent_at(&report, ".");
    assert!(
        root.skills
            .discovered
            .contains(&".claude/skills/x/SKILL.md".to_string()),
        "{:?}",
        root.skills.discovered
    );
}

/// A regular directory nested *inside* a host-config directory
/// (`.claude/sub`) is still a bona fide agent root when it holds its
/// own identity file -- `.claude/sub` is not itself named `.claude`,
/// `.kimi`, or `.agents`, so `identity_kinds_for` does not exempt it --
/// nested under the enclosing root, while `.claude` itself is walked
/// through and never becomes an agent of its own. Pins the prototype's
/// own rule documented in `crates/omnifrons-domain/src/nesting.rs`:
/// pruning touches only `SKIP_DIRS`, never a host-config directory.
#[test]
fn a_directory_nested_inside_a_host_config_directory_is_a_nested_agent_while_the_host_config_directory_itself_never_is()
 {
    let base = TempDir::new("nested-inside-host-config-dir");
    write(base.path(), "AGENTS.md", SIMPLE_BODY);
    write(base.path(), ".claude/sub/AGENTS.md", SIMPLE_BODY);
    let workspace = base.workspace();
    let fs_tree = FsDefinitionTree::new(None);
    let hasher = Sha2Hasher::new();
    let ports = DefinitionPorts {
        tree: &fs_tree,
        hasher: &hasher,
    };

    let report = scan(&ports, &workspace).expect("scan succeeds");

    let paths: Vec<&str> = report
        .agents
        .iter()
        .map(|agent| agent.path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![".", ".claude/sub"],
        ".claude/sub is a nested agent; .claude itself is never one"
    );
    let parent = agent_at(&report, ".");
    let child = agent_at(&report, ".claude/sub");
    assert_eq!(parent.nested, vec![".claude/sub".to_string()]);
    assert_eq!(child.parent, Some(Parent::Root(".".to_string())));
    assert!(child.consent_required);
}

/// The prototype's `test_scan_prunes_ignored_and_host_config_dirs`
/// symlink half: a symlinked directory is never descended into,
/// whether it points at another real agent or at an in-scope skill
/// directory -- the real path still gets found, the symlinked alias
/// never does.
#[cfg(unix)]
#[test]
fn symlinked_directories_are_never_descended_into() {
    let root = TempDir::new("symlink-root");
    let other = TempDir::new("symlink-other");
    write(root.path(), "AGENTS.md", SIMPLE_BODY);
    write(root.path(), "skills/own/SKILL.md", b"---\nname: own\n---\n");
    write(other.path(), "claude-only/CLAUDE.md", SIMPLE_BODY);
    std::os::unix::fs::symlink(
        other.path().join("claude-only"),
        root.path().join("linked-agent"),
    )
    .expect("symlink agent");
    std::os::unix::fs::symlink(
        root.path().join("skills").join("own"),
        root.path().join("linked-skill"),
    )
    .expect("symlink skill");
    let workspace = root.workspace();
    let fs_tree = FsDefinitionTree::new(None);
    let hasher = Sha2Hasher::new();
    let ports = DefinitionPorts {
        tree: &fs_tree,
        hasher: &hasher,
    };

    let report = scan(&ports, &workspace).expect("scan succeeds");

    let paths: Vec<&str> = report
        .agents
        .iter()
        .map(|agent| agent.path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["."],
        "the symlinked agent must never be discovered"
    );
    let agent = agent_at(&report, ".");
    assert!(
        agent
            .skills
            .discovered
            .contains(&"skills/own/SKILL.md".to_string()),
        "{:?}",
        agent.skills.discovered
    );
    assert!(
        !agent
            .skills
            .discovered
            .iter()
            .any(|path| path.starts_with("linked-skill/")),
        "{:?}",
        agent.skills.discovered
    );
}

/// The prototype's `test_home_user_memory_is_not_an_ancestor`: home's
/// own `.claude/CLAUDE.md` is suppressed at exactly that directory, so
/// it alone never makes home an ancestor; a plain `AGENTS.md` at home
/// does.
#[test]
fn home_level_claude_memory_is_not_an_ancestor_but_a_home_level_agents_md_is() {
    let base = TempDir::new("home-boundary");
    write(base.path(), "home/.claude/CLAUDE.md", b"User memory.\n");
    write(base.path(), "home/work/CLAUDE.md", SIMPLE_BODY);
    let home_path = base.path().join("home");
    let base_posix = canonical_posix(base.path());
    let work_workspace = WorkspaceRoot::new(home_path.join("work")).expect("workspace");
    let fs_tree = FsDefinitionTree::new(Some(home_path.clone()));

    let snapshot = fs_tree.snapshot(&work_workspace).expect("snapshot");
    assert!(
        ancestors_under(&snapshot.ancestors, &base_posix).is_empty(),
        "{:?}",
        snapshot.ancestors
    );

    write(base.path(), "home/AGENTS.md", b"Home-level agent.\n");
    let snapshot = fs_tree.snapshot(&work_workspace).expect("snapshot");
    let home_posix = canonical_posix(&home_path);
    let ancestors = ancestors_under(&snapshot.ancestors, &base_posix);
    assert_eq!(ancestors.len(), 1, "{:?}", snapshot.ancestors);
    assert_eq!(ancestors[0].path, home_posix);
    assert_eq!(ancestors[0].identity_files, vec!["AGENTS.md".to_string()]);
}

// -- adapter-level cases --

/// An extras-only directory inside the scan and an extras-only ancestor
/// outside it both appear -- the port's own widened rule
/// (`omnifrons_app::definition_tree`'s documentation), which the
/// prototype's own agent-roots-only `walk_workspace`/`find_ancestors`
/// would drop.
#[test]
fn extras_only_directories_and_ancestors_both_appear_in_the_snapshot() {
    let outer = TempDir::new("extras-outer");
    write(outer.path(), "CLAUDE.local.md", b"outer local notes\n");
    write(
        outer.path(),
        "root/extras-inside/CLAUDE.local.md",
        b"inner local notes\n",
    );
    let outer_posix = canonical_posix(outer.path());
    let workspace = WorkspaceRoot::new(outer.path().join("root")).expect("workspace");
    let fs_tree = FsDefinitionTree::new(None);

    let snapshot = fs_tree.snapshot(&workspace).expect("snapshot");

    let inside = snapshot
        .directories
        .iter()
        .find(|directory| directory.path == "extras-inside")
        .expect("the extras-only directory must appear");
    assert!(inside.identity_files.is_empty());
    assert_eq!(inside.host_extras, vec!["CLAUDE.local.md".to_string()]);

    let ancestors = ancestors_under(&snapshot.ancestors, &outer_posix);
    assert_eq!(ancestors.len(), 1, "{:?}", snapshot.ancestors);
    assert!(ancestors[0].identity_files.is_empty());
    assert_eq!(
        ancestors[0].host_extras,
        vec!["CLAUDE.local.md".to_string()]
    );
}

/// A symlinked identity file reports `FileShape::Symlink` with its own
/// target text, never the file's content -- the prototype's own
/// `read_identity` behaviour for a symlinked final component.
#[cfg(unix)]
#[test]
fn a_symlinked_identity_file_reports_the_symlink_shape() {
    let project = TempDir::new("symlinked-identity");
    std::os::unix::fs::symlink("nowhere", project.path().join("AGENTS.md")).expect("symlink");
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    let snapshot = fs_tree.snapshot(&workspace).expect("snapshot");

    let root = snapshot
        .directories
        .iter()
        .find(|directory| directory.path == ".")
        .expect("the root must be listed");
    let file = root
        .identity_files
        .iter()
        .find(|file| file.kind == IdentityKind::Agents)
        .expect("AGENTS.md must be reported");
    assert_eq!(
        file.shape,
        FileShape::Symlink {
            target: "nowhere".to_string()
        }
    );
    assert!(file.bytes.is_empty());
}

/// An identity file over `MAX_DEFINITION_ENTRY_BYTES` is refused
/// closed, never truncated or silently admitted.
#[test]
fn an_identity_file_over_the_cap_is_refused() {
    let project = TempDir::new("identity-too-large");
    let oversized = vec![b'a'; usize::try_from(MAX_DEFINITION_ENTRY_BYTES).expect("fits") + 1];
    write(project.path(), "AGENTS.md", &oversized);
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    assert_eq!(
        fs_tree.snapshot(&workspace),
        Err(DefinitionTreeError::EntryTooLarge)
    );
}

/// An oversized `AGENTS.md` *above* the scan root is only a presence
/// check, never a content read: unlike
/// `an_identity_file_over_the_cap_is_refused` above, whose oversized
/// file sits *at* the scan root and is read in full, an ancestor
/// identity file's bytes are never needed -- see `FsDefinitionTree`'s
/// own `find_ancestors` documentation -- so the snapshot succeeds and
/// still lists that ancestor.
#[test]
fn an_oversized_ancestor_identity_file_does_not_fail_the_scan() {
    let base = TempDir::new("ancestor-too-large");
    let oversized = vec![b'a'; usize::try_from(MAX_DEFINITION_ENTRY_BYTES).expect("fits") + 1];
    write(base.path(), "AGENTS.md", &oversized);
    std::fs::create_dir_all(base.path().join("nested")).expect("mkdir nested");
    let base_posix = canonical_posix(base.path());
    let workspace = WorkspaceRoot::new(base.path().join("nested")).expect("workspace");
    let fs_tree = FsDefinitionTree::new(None);

    let snapshot = fs_tree
        .snapshot(&workspace)
        .expect("snapshot succeeds despite the oversized ancestor");

    let ancestors = ancestors_under(&snapshot.ancestors, &base_posix);
    assert_eq!(ancestors.len(), 1, "{:?}", snapshot.ancestors);
    assert_eq!(ancestors[0].path, base_posix);
    assert_eq!(ancestors[0].identity_files, vec!["AGENTS.md".to_string()]);
}

/// A `SKILL.md` over the cap is refused the same way an oversized
/// identity file is.
#[test]
fn a_skill_file_over_the_cap_is_refused() {
    let project = TempDir::new("skill-too-large");
    let oversized = vec![b'a'; usize::try_from(MAX_DEFINITION_ENTRY_BYTES).expect("fits") + 1];
    write(project.path(), "SKILL.md", &oversized);
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    assert_eq!(
        fs_tree.snapshot(&workspace),
        Err(DefinitionTreeError::EntryTooLarge)
    );
}

/// Documents a boundary this adapter relies on rather than re-checks: a
/// nonexistent path can never even become a `WorkspaceRoot`, so
/// `FsDefinitionTree::snapshot` is never called with one.
#[test]
fn a_nonexistent_root_cannot_become_a_workspace_root() {
    let base = TempDir::new("nonexistent-root-parent");
    let missing = base.path().join("does-not-exist");
    assert!(WorkspaceRoot::new(&missing).is_err());
}

/// A scan root this process cannot search at all (unix mode `0o000`)
/// reports `RootUnreadable`: `WorkspaceRoot::new` only needs to resolve
/// the root's own *parent*, so it still succeeds, and every one of
/// `FsDefinitionTree`'s own root-level checks then hits the same
/// permission error, uniformly reported this one way -- see this
/// adapter's own module documentation for why.
///
/// Skipped when running as `root`, which bypasses unix permission bits
/// entirely (mirrors the same skip in `omnifrons_adapters`'s own
/// `fs_prober` tests).
#[cfg(unix)]
#[test]
fn an_unreadable_root_reports_root_unreadable() {
    use std::os::unix::fs::PermissionsExt as _;

    if nix::unistd::Uid::effective().is_root() {
        eprintln!(
            "skipping an_unreadable_root_reports_root_unreadable: running as root, \
             which bypasses unix permission bits entirely"
        );
        return;
    }
    let project = TempDir::new("unreadable-root");
    let workspace = project.workspace();
    std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o000))
        .expect("chmod 0o000");
    let fs_tree = FsDefinitionTree::new(None);

    let result = fs_tree.snapshot(&workspace);

    std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o700))
        .expect("restore permissions before cleanup");
    assert_eq!(result, Err(DefinitionTreeError::RootUnreadable));
}

/// A subdirectory this process cannot even list (unix mode `0o000`)
/// below the scan root is skipped entirely, along with everything it
/// holds -- mirroring the prototype's own
/// `os.walk(root, followlinks=False)` under its default
/// `onerror=None` -- while a readable sibling agent is still found:
/// the snapshot succeeds, the blocked directory never appears in
/// `directories` or `skills`, and the sibling does.
///
/// A directory removed between being listed by its parent and being
/// visited itself would hit this same listing-failure path, but is
/// not exercised here: nothing in `std::fs` lets a test
/// deterministically remove a directory in the narrow window between
/// `FsDefinitionTree` listing its parent and recursing into it.
///
/// Skipped when running as `root`, which bypasses unix permission bits
/// entirely (mirrors the same skip in
/// `an_unreadable_root_reports_root_unreadable` above).
#[cfg(unix)]
#[test]
fn an_unreadable_subdirectory_below_the_root_is_skipped() {
    use std::os::unix::fs::PermissionsExt as _;

    if nix::unistd::Uid::effective().is_root() {
        eprintln!(
            "skipping an_unreadable_subdirectory_below_the_root_is_skipped: running as \
             root, which bypasses unix permission bits entirely"
        );
        return;
    }
    let project = TempDir::new("unreadable-subdirectory");
    write(project.path(), "blocked/AGENTS.md", SIMPLE_BODY);
    write(
        project.path(),
        "blocked/SKILL.md",
        b"---\nname: blocked\n---\n",
    );
    write(project.path(), "sibling/AGENTS.md", SIMPLE_BODY);
    let blocked = project.path().join("blocked");
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000))
        .expect("chmod 0o000");
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    let result = fs_tree.snapshot(&workspace);

    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o700))
        .expect("restore permissions before cleanup");
    let snapshot = result.expect("snapshot succeeds despite the blocked subdirectory");
    let directory_paths: Vec<&str> = snapshot
        .directories
        .iter()
        .map(|directory| directory.path.as_str())
        .collect();
    assert!(
        !directory_paths
            .iter()
            .any(|path| path.starts_with("blocked")),
        "{directory_paths:?}"
    );
    assert!(directory_paths.contains(&"sibling"), "{directory_paths:?}");
    assert!(
        !snapshot
            .skills
            .iter()
            .any(|skill| skill.path.starts_with("blocked")),
        "{:?}",
        snapshot.skills
    );
}

/// An identity file this process cannot read (unix mode `0o000`)
/// inside an otherwise fully readable subdirectory still aborts the
/// whole scan with `WalkFailed`: unlike a directory *listing* failure
/// (`an_unreadable_subdirectory_below_the_root_is_skipped` above), a
/// content-read failure on an already-discovered identity file is
/// never skipped, matching the prototype's own `read_identity`, which
/// raises on exactly this failure rather than silently omitting the
/// file.
///
/// Skipped when running as `root`, which bypasses unix permission bits
/// entirely.
#[cfg(unix)]
#[test]
fn an_unreadable_identity_file_still_aborts_the_walk() {
    use std::os::unix::fs::PermissionsExt as _;

    if nix::unistd::Uid::effective().is_root() {
        eprintln!(
            "skipping an_unreadable_identity_file_still_aborts_the_walk: running as root, \
             which bypasses unix permission bits entirely"
        );
        return;
    }
    let project = TempDir::new("unreadable-identity-file");
    write(project.path(), "nested/AGENTS.md", SIMPLE_BODY);
    let identity_file = project.path().join("nested").join("AGENTS.md");
    std::fs::set_permissions(&identity_file, std::fs::Permissions::from_mode(0o000))
        .expect("chmod 0o000");
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    let result = fs_tree.snapshot(&workspace);

    std::fs::set_permissions(&identity_file, std::fs::Permissions::from_mode(0o600))
        .expect("restore permissions before cleanup");
    assert_eq!(result, Err(DefinitionTreeError::WalkFailed));
}

/// An ancestor `AGENTS.md` this process cannot read (unix mode
/// `0o000`) is only a presence check, never a content read: unlike
/// `an_unreadable_identity_file_still_aborts_the_walk` above, whose
/// unreadable file sits inside the scan root and is read in full, the
/// snapshot succeeds and still lists that ancestor -- `symlink_metadata`
/// never needs read permission on the file itself, only search
/// permission on its parent directory, which this test never revokes.
///
/// Skipped when running as `root`, which bypasses unix permission bits
/// entirely.
#[cfg(unix)]
#[test]
fn an_unreadable_ancestor_identity_file_does_not_fail_the_scan() {
    use std::os::unix::fs::PermissionsExt as _;

    if nix::unistd::Uid::effective().is_root() {
        eprintln!(
            "skipping an_unreadable_ancestor_identity_file_does_not_fail_the_scan: running \
             as root, which bypasses unix permission bits entirely"
        );
        return;
    }
    let base = TempDir::new("ancestor-unreadable");
    write(base.path(), "AGENTS.md", SIMPLE_BODY);
    std::fs::create_dir_all(base.path().join("nested")).expect("mkdir nested");
    let ancestor_file = base.path().join("AGENTS.md");
    std::fs::set_permissions(&ancestor_file, std::fs::Permissions::from_mode(0o000))
        .expect("chmod 0o000");
    let base_posix = canonical_posix(base.path());
    let workspace = WorkspaceRoot::new(base.path().join("nested")).expect("workspace");
    let fs_tree = FsDefinitionTree::new(None);

    let result = fs_tree.snapshot(&workspace);

    std::fs::set_permissions(&ancestor_file, std::fs::Permissions::from_mode(0o600))
        .expect("restore permissions before cleanup");
    let snapshot = result.expect("snapshot succeeds despite the unreadable ancestor");
    let ancestors = ancestors_under(&snapshot.ancestors, &base_posix);
    assert_eq!(ancestors.len(), 1, "{:?}", snapshot.ancestors);
    assert_eq!(ancestors[0].identity_files, vec!["AGENTS.md".to_string()]);
}

/// A `SKILL.md` this process cannot read (unix mode `0o000`) still
/// aborts the whole scan with `WalkFailed`, exactly like an unreadable
/// identity file above: a content-read failure, never a directory
/// listing failure, so it is never skipped.
///
/// Skipped when running as `root`, which bypasses unix permission bits
/// entirely.
#[cfg(unix)]
#[test]
fn an_unreadable_skill_file_still_aborts_the_walk() {
    use std::os::unix::fs::PermissionsExt as _;

    if nix::unistd::Uid::effective().is_root() {
        eprintln!(
            "skipping an_unreadable_skill_file_still_aborts_the_walk: running as root, \
             which bypasses unix permission bits entirely"
        );
        return;
    }
    let project = TempDir::new("unreadable-skill-file");
    write(
        project.path(),
        "nested/SKILL.md",
        b"---\nname: nested\n---\n",
    );
    let skill_file = project.path().join("nested").join("SKILL.md");
    std::fs::set_permissions(&skill_file, std::fs::Permissions::from_mode(0o000))
        .expect("chmod 0o000");
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    let result = fs_tree.snapshot(&workspace);

    std::fs::set_permissions(&skill_file, std::fs::Permissions::from_mode(0o600))
        .expect("restore permissions before cleanup");
    assert_eq!(result, Err(DefinitionTreeError::WalkFailed));
}

// -- lenient, `lexists`-shaped presence checks under a permission error --

/// An ancestor's own `.claude` directory this process cannot search at
/// all (unix mode `0o000`) makes both the `.claude/CLAUDE.md` identity
/// kind and the `.claude/AGENTS.md` host extra absent at that ancestor,
/// never a hard failure: the prototype's own `has_identity_files` and
/// `find_host_extras` ask a blanket `os.path.lexists`, which answers
/// "not there" for a permission error exactly like a missing file. The
/// ancestor's own plain `AGENTS.md` is unaffected by `.claude`'s own
/// permissions and is still reported.
///
/// Skipped when running as `root`, which bypasses unix permission bits
/// entirely.
#[cfg(unix)]
#[test]
fn an_unsearchable_dot_claude_at_an_ancestor_makes_its_identity_and_extra_absent_not_fatal() {
    use std::os::unix::fs::PermissionsExt as _;

    if nix::unistd::Uid::effective().is_root() {
        eprintln!(
            "skipping an_unsearchable_dot_claude_at_an_ancestor_makes_its_identity_and_extra_absent_not_fatal: \
             running as root, which bypasses unix permission bits entirely"
        );
        return;
    }
    let base = TempDir::new("ancestor-dot-claude-unsearchable");
    write(base.path(), "AGENTS.md", SIMPLE_BODY);
    std::fs::create_dir_all(base.path().join(".claude")).expect("mkdir .claude");
    std::fs::create_dir_all(base.path().join("nested")).expect("mkdir nested");
    let dot_claude = base.path().join(".claude");
    std::fs::set_permissions(&dot_claude, std::fs::Permissions::from_mode(0o000))
        .expect("chmod 0o000");
    let base_posix = canonical_posix(base.path());
    let workspace = WorkspaceRoot::new(base.path().join("nested")).expect("workspace");
    let fs_tree = FsDefinitionTree::new(None);

    let result = fs_tree.snapshot(&workspace);

    std::fs::set_permissions(&dot_claude, std::fs::Permissions::from_mode(0o700))
        .expect("restore permissions before cleanup");
    let snapshot = result.expect("snapshot succeeds despite the unsearchable .claude");
    let ancestors = ancestors_under(&snapshot.ancestors, &base_posix);
    assert_eq!(ancestors.len(), 1, "{:?}", snapshot.ancestors);
    assert_eq!(
        ancestors[0].identity_files,
        vec!["AGENTS.md".to_string()],
        "the unsearchable .claude/CLAUDE.md must be absent, not a hard failure: {:?}",
        ancestors[0]
    );
    assert!(
        ancestors[0].host_extras.is_empty(),
        "the unsearchable .claude/AGENTS.md host extra must be absent too: {:?}",
        ancestors[0]
    );
}

/// A host extra whose own path sits behind an unsearchable `.claude`
/// *inside* the scan root (as opposed to the previous test's ancestor)
/// is absent, not a reason to fail the whole scan: the root's own
/// `AGENTS.md` still makes it a reported agent, and `find_host_extras`
/// answers "not there" for `.claude/AGENTS.md` through the same
/// `lexists`-shaped leniency.
///
/// Skipped when running as `root`, which bypasses unix permission bits
/// entirely.
#[cfg(unix)]
#[test]
fn a_host_extra_behind_an_unsearchable_dot_claude_inside_the_scan_is_absent_not_fatal() {
    use std::os::unix::fs::PermissionsExt as _;

    if nix::unistd::Uid::effective().is_root() {
        eprintln!(
            "skipping a_host_extra_behind_an_unsearchable_dot_claude_inside_the_scan_is_absent_not_fatal: \
             running as root, which bypasses unix permission bits entirely"
        );
        return;
    }
    let project = TempDir::new("root-dot-claude-unsearchable");
    write(project.path(), "AGENTS.md", SIMPLE_BODY);
    std::fs::create_dir_all(project.path().join(".claude")).expect("mkdir .claude");
    let dot_claude = project.path().join(".claude");
    std::fs::set_permissions(&dot_claude, std::fs::Permissions::from_mode(0o000))
        .expect("chmod 0o000");
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    let result = fs_tree.snapshot(&workspace);

    std::fs::set_permissions(&dot_claude, std::fs::Permissions::from_mode(0o700))
        .expect("restore permissions before cleanup");
    let snapshot = result.expect(
        "find_host_extras asks a blanket lexists-shaped question, so an unsearchable \
         .claude makes the .claude/AGENTS.md extra absent, never a hard failure",
    );
    let root = snapshot
        .directories
        .iter()
        .find(|directory| directory.path == ".")
        .expect("the root must still be listed, via its own AGENTS.md");
    assert!(
        !root.host_extras.contains(&".claude/AGENTS.md".to_string()),
        "{:?}",
        root.host_extras
    );
}

/// A below-root directory this process can list but not search (unix
/// mode `0o400`, `r--`) still aborts the whole scan with `WalkFailed`,
/// pinning the *other* half of this adapter's split: `record_directory`
/// checks every one of [`IdentityKind::ALL`] unconditionally, and
/// `.claude/CLAUDE.md`'s own intermediate segment (`.claude`) goes
/// through the strict, raising-equivalent
/// `symlink_metadata_or_absent`, which still aborts on the same
/// permission error that makes a single-segment kind like `AGENTS.md`
/// merely absent -- see `fs_definition_tree`'s own inline `#[cfg(test)]`
/// unit tests for `AGENTS.md`'s lenient resolution in isolation, and
/// this module's own documentation for why a directory
/// visited through the *full* walk cannot show that lenient resolution
/// on its own: a listing failure is skipped per-directory, but an
/// identity-kind check is not.
///
/// Skipped when running as `root`, which bypasses unix permission bits
/// entirely.
#[cfg(unix)]
#[test]
fn a_listable_but_unsearchable_subdirectory_still_aborts_via_a_multi_segment_kind() {
    use std::os::unix::fs::PermissionsExt as _;

    if nix::unistd::Uid::effective().is_root() {
        eprintln!(
            "skipping a_listable_but_unsearchable_subdirectory_still_aborts_via_a_multi_segment_kind: \
             running as root, which bypasses unix permission bits entirely"
        );
        return;
    }
    let project = TempDir::new("subdirectory-listable-not-searchable");
    write(project.path(), "blocked/AGENTS.md", SIMPLE_BODY);
    let blocked = project.path().join("blocked");
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o400))
        .expect("chmod 0o400 (r--: listable, not searchable)");
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    let result = fs_tree.snapshot(&workspace);

    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o700))
        .expect("restore permissions before cleanup");
    assert_eq!(
        result,
        Err(DefinitionTreeError::WalkFailed),
        "the .claude/CLAUDE.md intermediate segment check stays strict even though \
         AGENTS.md's own check, for the same directory, is now lenient"
    );
}

/// A `.claude` directory this process can list but not search (unix
/// mode `0o400`, `r--`) holding a real `SKILL.md` fails the whole scan
/// with `WalkFailed`, never silently omitting a skill the listing
/// already discovered: the prototype's own walk finds `SKILL.md`
/// through `SKILL_FILE in filenames` -- a check against the listing
/// `.claude` itself still yields even though it cannot be searched --
/// and only then calls its own `lstat`, which raises on the very
/// permission error this directory's restricted mode produces.
/// `.claude` being one of `HOST_CONFIG_DIRS` still means
/// `identity_kinds_for` hands it no identity kinds at all, so
/// `record_directory`'s own per-kind loop, and the *other* strict
/// check the sibling test
/// `a_listable_but_unsearchable_subdirectory_still_aborts_via_a_multi_segment_kind`
/// pins, never run for it; this directory fails only because
/// `record_skill` itself must fail once its own listing already names
/// `SKILL.md`, not for the sibling test's own reason.
///
/// Skipped when running as `root`, which bypasses unix permission
/// bits entirely.
#[cfg(unix)]
#[test]
fn a_listable_but_unsearchable_dot_claude_directory_with_a_listed_skill_md_fails_the_walk() {
    use std::os::unix::fs::PermissionsExt as _;

    if nix::unistd::Uid::effective().is_root() {
        eprintln!(
            "skipping a_listable_but_unsearchable_dot_claude_directory_with_a_listed_skill_md_fails_the_walk: \
             running as root, which bypasses unix permission bits entirely"
        );
        return;
    }
    let project = TempDir::new("dot-claude-skill-listable-not-searchable");
    write(
        project.path(),
        ".claude/SKILL.md",
        b"---\nname: leaked\n---\n",
    );
    write(project.path(), "agent/AGENTS.md", SIMPLE_BODY);
    let dot_claude = project.path().join(".claude");
    std::fs::set_permissions(&dot_claude, std::fs::Permissions::from_mode(0o400))
        .expect("chmod 0o400 (r--: listable, not searchable)");
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    let result = fs_tree.snapshot(&workspace);

    std::fs::set_permissions(&dot_claude, std::fs::Permissions::from_mode(0o700))
        .expect("restore permissions before cleanup");
    assert_eq!(
        result,
        Err(DefinitionTreeError::WalkFailed),
        "a SKILL.md the listing already discovered must fail the scan when it cannot \
         then be stat'd, never be silently treated as absent"
    );
}

/// A `.claude` directory this process can list but not search (unix
/// mode `0o400`, `r--`) that holds no `SKILL.md` at all still yields a
/// successful snapshot: `record_skill`'s own listing gate -- the same
/// `read_dir` pass `list_subdirectories` already made -- answers
/// absent without ever attempting the strict `symlink_metadata` that
/// would otherwise hit this directory's own permission error. This is
/// the shape the listing gate exists to keep working; the sibling test
/// above pins the opposite case, where `SKILL.md` *is* listed and that
/// same permission error becomes fatal instead.
///
/// Skipped when running as `root`, which bypasses unix permission
/// bits entirely.
#[cfg(unix)]
#[test]
fn a_listable_but_unsearchable_dot_claude_directory_without_a_skill_md_still_succeeds() {
    use std::os::unix::fs::PermissionsExt as _;

    if nix::unistd::Uid::effective().is_root() {
        eprintln!(
            "skipping a_listable_but_unsearchable_dot_claude_directory_without_a_skill_md_still_succeeds: \
             running as root, which bypasses unix permission bits entirely"
        );
        return;
    }
    let project = TempDir::new("dot-claude-no-skill-listable-not-searchable");
    std::fs::create_dir_all(project.path().join(".claude")).expect("mkdir .claude");
    write(project.path(), "agent/AGENTS.md", SIMPLE_BODY);
    let dot_claude = project.path().join(".claude");
    std::fs::set_permissions(&dot_claude, std::fs::Permissions::from_mode(0o400))
        .expect("chmod 0o400 (r--: listable, not searchable)");
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    let result = fs_tree.snapshot(&workspace);

    std::fs::set_permissions(&dot_claude, std::fs::Permissions::from_mode(0o700))
        .expect("restore permissions before cleanup");
    let snapshot = result.expect(
        "SKILL.md absent from .claude's own listing must short-circuit record_skill \
         before any stat is attempted, so an unsearchable .claude with nothing listed \
         there never fails the scan",
    );
    assert!(
        !snapshot
            .skills
            .iter()
            .any(|skill| skill.path.starts_with(".claude/")),
        "{:?}",
        snapshot.skills
    );
    assert!(
        snapshot
            .directories
            .iter()
            .any(|directory| directory.path == "agent"),
        "the sibling agent must still be reported: {:?}",
        snapshot.directories
    );
}

/// A scan root this process can list but not search (unix mode `0o400`,
/// `r--`) holding a real, listed `SKILL.md` reports `RootUnreadable`,
/// never silently omitting a skill its own listing already discovered:
/// `WorkspaceRoot::new` only needs to resolve the root's own *parent*
/// (exactly like `an_unreadable_root_reports_root_unreadable`'s `0o000`
/// case above), so it still succeeds here too, and `list_subdirectories`
/// itself only needs *read* permission to enumerate `AGENTS.md` and
/// `SKILL.md` by name -- `0o400` grants that -- so the listing succeeds
/// and marks `SKILL.md` listed. Only the *next* step, `record_skill`'s
/// own strict `symlink_metadata_or_absent` stat on `root/SKILL.md`,
/// needs *search* permission on `root` to resolve that name at all;
/// `0o400` denies it, and this adapter reports that failure as
/// `RootUnreadable` (`is_root` is `true` for the scan root itself, unlike
/// every `WalkFailed` sibling test in this file).
///
/// `home` is set to this same root so `identity_kinds_for` hands
/// `record_directory` every kind except `ClaudeDir`'s two-segment
/// `.claude/CLAUDE.md`: every remaining, single-segment kind's check is
/// the *lenient* `read_identity_at`, which this same permission error
/// turns into "absent" rather than a failure, exactly like `AGENTS.md`
/// itself. Without `home` set this way, `ClaudeDir`'s own strict
/// intermediate-segment check would hit the identical permission error
/// first and report `RootUnreadable` from `record_directory` instead,
/// before `record_skill` is ever reached -- still the right *result*,
/// but not the specific check this test means to pin, the same reason
/// the `.claude`-based sibling tests below lean on `HOST_CONFIG_DIRS` to
/// skip `record_directory`'s own per-kind loop entirely.
///
/// Skipped when running as `root`, which bypasses unix permission bits
/// entirely.
#[cfg(unix)]
#[test]
fn a_listable_but_unsearchable_scan_root_with_a_listed_skill_md_reports_root_unreadable() {
    use std::os::unix::fs::PermissionsExt as _;

    if nix::unistd::Uid::effective().is_root() {
        eprintln!(
            "skipping a_listable_but_unsearchable_scan_root_with_a_listed_skill_md_reports_root_unreadable: \
             running as root, which bypasses unix permission bits entirely"
        );
        return;
    }
    let project = TempDir::new("root-skill-listable-not-searchable");
    write(project.path(), "AGENTS.md", SIMPLE_BODY);
    write(project.path(), "SKILL.md", b"---\nname: own\n---\n");
    std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o400))
        .expect("chmod 0o400 (r--: listable, not searchable)");

    let workspace = WorkspaceRoot::new(project.path()).expect(
        "WorkspaceRoot::new only resolves the root's own parent, so a 0o400 root still \
         canonicalizes",
    );
    let fs_tree = FsDefinitionTree::new(Some(project.path().to_path_buf()));

    let result = fs_tree.snapshot(&workspace);

    std::fs::set_permissions(project.path(), std::fs::Permissions::from_mode(0o700))
        .expect("restore permissions before cleanup");
    assert_eq!(
        result,
        Err(DefinitionTreeError::RootUnreadable),
        "a SKILL.md the root's own listing already discovered must fail the scan when it \
         cannot then be stat'd, never be silently treated as absent"
    );
}

/// The listing gate that discovers `SKILL.md` -- `list_subdirectories`'s
/// own `name == SKILL_FILE_NAME` check, read back by `record_skill`'s
/// `skill_listed` gate -- compares an entry's name byte-for-byte with the
/// literal `"SKILL.md"`, the same `SKILL_FILE in filenames` membership
/// test the prototype's own walk makes against its listing, translated
/// verbatim rather than loosened into a case-insensitive comparison. A
/// directory holding a differently-cased `skill.md` is therefore never
/// discovered as a skill, and this holds on every platform this crate
/// ships for, not just a case-sensitive one: `list_subdirectories` only
/// ever compares the *enumerated* entry name `std::fs::read_dir` itself
/// returns, never re-resolves that name through a fresh, potentially
/// case-folding path lookup. On a case-sensitive filesystem (ext4, say)
/// `skill.md` and `SKILL.md` simply name two different files, so the
/// comparison is exact by construction. On a case-insensitive *but
/// case-preserving* filesystem (APFS's default mode on macOS, NTFS on
/// Windows) a file created as `skill.md` is still *listed* with that
/// exact lower-case spelling -- case-insensitivity there governs path
/// *lookup* (opening `"SKILL.md"` would still find it), never what
/// `read_dir` enumerates back -- so the byte-for-byte gate still sees
/// `skill.md` and still rejects it. Both kinds of filesystem therefore
/// agree: `skills` stays empty. No `#[cfg]` here, deliberately: this must
/// hold on every platform, not just unix.
#[test]
fn a_differently_cased_skill_md_is_never_discovered() {
    let project = TempDir::new("case-exact-skill-name");
    write(project.path(), "skill.md", b"---\nname: lowercase\n---\n");
    write(project.path(), "AGENTS.md", SIMPLE_BODY);
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    let snapshot = fs_tree.snapshot(&workspace).expect("snapshot succeeds");

    assert!(
        snapshot.skills.is_empty(),
        "a lower-case skill.md must never satisfy the byte-exact SKILL.md gate: {:?}",
        snapshot.skills
    );
    assert_eq!(
        snapshot
            .directories
            .iter()
            .map(|directory| directory.path.as_str())
            .collect::<Vec<_>>(),
        vec!["."],
        "the root must still be reported for its own AGENTS.md"
    );
}

// -- refusing, not folding, a *reported* non-Unicode path --

/// Treats `result` -- the outcome of a fixture's own `create_dir` or
/// `write` call that names a non-Unicode path component directly -- as
/// a graceful skip rather than a fixture panic when the filesystem
/// itself refuses such a name outright (a UTF-8-enforcing filesystem,
/// APFS on macOS in particular, refuses the `create_dir` before this
/// suite ever reaches `FsDefinitionTree` at all). Prints a one-line
/// note naming `label` and the observed error, then returns `false` so
/// the caller can return early -- the same shape the "skipped when
/// running as root" tests above use, keyed on filesystem behaviour
/// instead of process identity. Returns `true` on `Ok`, so every
/// non-Unicode test below still runs to completion and passes on a
/// filesystem that allows such names, Linux included.
#[cfg(unix)]
fn non_unicode_fixture_supported(label: &str, result: std::io::Result<()>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "skipping {label}: this filesystem refuses a non-Unicode path component \
                 ({error})"
            );
            false
        }
    }
}

/// Two sibling directories whose names are each a single invalid UTF-8
/// byte (`0xff` and `0xfe`) both lossily collapse to the same `U+FFFD`
/// replacement text; once both hold an `AGENTS.md`, reporting either
/// one's path at all would either silently misattribute one
/// directory's files to the other's displayed path, or collide as a
/// duplicate `DirectoryEntry::path`. This adapter refuses the whole
/// snapshot with `PathNotUnicode` instead of ever reaching that
/// choice.
///
/// Skipped when the filesystem itself refuses a single-invalid-byte
/// directory name, via `non_unicode_fixture_supported` (APFS on macOS
/// enforces valid UTF-8 names).
#[cfg(unix)]
#[test]
fn two_non_unicode_sibling_names_refuse_the_snapshot_instead_of_colliding() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let project = TempDir::new("non-unicode-siblings");
    let first = OsStr::from_bytes(&[0xff]);
    let second = OsStr::from_bytes(&[0xfe]);
    if !non_unicode_fixture_supported(
        "two_non_unicode_sibling_names_refuse_the_snapshot_instead_of_colliding",
        std::fs::create_dir(project.path().join(first)),
    ) {
        return;
    }
    if !non_unicode_fixture_supported(
        "two_non_unicode_sibling_names_refuse_the_snapshot_instead_of_colliding",
        std::fs::create_dir(project.path().join(second)),
    ) {
        return;
    }
    std::fs::write(project.path().join(first).join("AGENTS.md"), SIMPLE_BODY)
        .expect("write AGENTS.md under 0xff");
    std::fs::write(project.path().join(second).join("AGENTS.md"), SIMPLE_BODY)
        .expect("write AGENTS.md under 0xfe");
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    let result = fs_tree.snapshot(&workspace);

    assert_eq!(result, Err(DefinitionTreeError::PathNotUnicode));
}

/// A non-Unicode directory name that holds nothing reportable -- no
/// identity file, no host extra, no `SKILL.md` -- never needs a
/// `String` built from it at all, so it is walked through silently,
/// exactly like any other directory with nothing of interest; the
/// root's own, perfectly ordinary `AGENTS.md` is unaffected.
///
/// Skipped when the filesystem itself refuses a single-invalid-byte
/// directory name, via `non_unicode_fixture_supported` (APFS on macOS
/// enforces valid UTF-8 names).
#[cfg(unix)]
#[test]
fn a_non_unicode_directory_holding_nothing_reportable_is_walked_through_silently() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let project = TempDir::new("non-unicode-empty");
    write(project.path(), "AGENTS.md", SIMPLE_BODY);
    let name = OsStr::from_bytes(&[0xff]);
    if !non_unicode_fixture_supported(
        "a_non_unicode_directory_holding_nothing_reportable_is_walked_through_silently",
        std::fs::create_dir(project.path().join(name)),
    ) {
        return;
    }
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    let snapshot = fs_tree
        .snapshot(&workspace)
        .expect("a non-Unicode directory holding nothing reportable never needs a String");

    let directory_paths: Vec<&str> = snapshot
        .directories
        .iter()
        .map(|directory| directory.path.as_str())
        .collect();
    assert_eq!(directory_paths, vec!["."]);
}

/// Proves the `SKILL.md`-specific refusal path: a `SKILL.md` sitting
/// directly inside a non-Unicode-named directory refuses with
/// `PathNotUnicode` from `record_skill`'s own check, even though that
/// same directory holds no identity file or host extra of its own --
/// `record_directory`'s emptiness check lets that directory through
/// first, so this test pins that `record_skill` repeats the identical
/// refusal independently rather than relying on `record_directory`
/// having already caught it.
///
/// Skipped when the filesystem itself refuses a single-invalid-byte
/// directory name, via `non_unicode_fixture_supported` (APFS on macOS
/// enforces valid UTF-8 names).
#[cfg(unix)]
#[test]
fn a_skill_file_under_a_non_unicode_directory_refuses_via_record_skill() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let project = TempDir::new("non-unicode-skill");
    let lossy = OsStr::from_bytes(&[0xff]);
    let lossy_dir = project.path().join(lossy);
    if !non_unicode_fixture_supported(
        "a_skill_file_under_a_non_unicode_directory_refuses_via_record_skill",
        std::fs::create_dir(&lossy_dir),
    ) {
        return;
    }
    std::fs::write(lossy_dir.join("SKILL.md"), b"---\nname: lossy\n---\n")
        .expect("write SKILL.md under the non-Unicode directory");
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    let result = fs_tree.snapshot(&workspace);

    assert_eq!(result, Err(DefinitionTreeError::PathNotUnicode));
}

/// Proves the inherited-flag propagation through `visit`: a
/// non-Unicode directory holding nothing reportable of its own is
/// walked through without error, exactly like
/// `a_non_unicode_directory_holding_nothing_reportable_is_walked_through_silently`
/// above -- but its own lossy flag still carries down to its `child`
/// subdirectory, whose own name is perfectly ordinary Unicode. Once
/// `child` turns out to hold a reportable `AGENTS.md`, the snapshot
/// still refuses with `PathNotUnicode`, proving the flag is inherited
/// from an ancestor segment rather than recomputed from each
/// directory's own name alone.
///
/// Skipped when the filesystem itself refuses a single-invalid-byte
/// directory name, via `non_unicode_fixture_supported` (APFS on macOS
/// enforces valid UTF-8 names).
#[cfg(unix)]
#[test]
fn a_reportable_directory_beneath_a_non_unicode_ancestor_segment_refuses_via_the_inherited_flag() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let project = TempDir::new("non-unicode-ancestor-segment");
    let lossy = OsStr::from_bytes(&[0xff]);
    let lossy_dir = project.path().join(lossy);
    if !non_unicode_fixture_supported(
        "a_reportable_directory_beneath_a_non_unicode_ancestor_segment_refuses_via_the_inherited_flag",
        std::fs::create_dir(&lossy_dir),
    ) {
        return;
    }
    write(&lossy_dir, "child/AGENTS.md", SIMPLE_BODY);
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    let result = fs_tree.snapshot(&workspace);

    assert_eq!(result, Err(DefinitionTreeError::PathNotUnicode));
}

/// Proves the ancestor-side refusal: a scan root built *inside* a
/// non-Unicode-named directory that itself holds an identity file
/// refuses with `PathNotUnicode` from `posix_absolute`, the only
/// fallible step in `find_ancestors` -- mirroring how
/// `an_unreadable_ancestor_identity_file_does_not_fail_the_scan` and
/// its neighbours above build an outer directory and a `WorkspaceRoot`
/// below it, rather than at it.
///
/// Skipped when the filesystem itself refuses a single-invalid-byte
/// directory name, via `non_unicode_fixture_supported` (APFS on macOS
/// enforces valid UTF-8 names).
#[cfg(unix)]
#[test]
fn a_non_unicode_ancestor_path_refuses_from_posix_absolute() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let base = TempDir::new("non-unicode-ancestor");
    let lossy = OsStr::from_bytes(&[0xff]);
    let lossy_dir = base.path().join(lossy);
    if !non_unicode_fixture_supported(
        "a_non_unicode_ancestor_path_refuses_from_posix_absolute",
        std::fs::create_dir(&lossy_dir),
    ) {
        return;
    }
    std::fs::write(lossy_dir.join("AGENTS.md"), SIMPLE_BODY)
        .expect("write AGENTS.md under the non-Unicode ancestor");
    std::fs::create_dir_all(lossy_dir.join("nested")).expect("mkdir nested");
    let workspace = WorkspaceRoot::new(lossy_dir.join("nested")).expect("workspace");
    let fs_tree = FsDefinitionTree::new(None);

    let result = fs_tree.snapshot(&workspace);

    assert_eq!(result, Err(DefinitionTreeError::PathNotUnicode));
}

/// Pins the success side alongside the three refusals above: a
/// non-Unicode sibling holding nothing reportable, sitting beside a
/// reportable Unicode-named sibling -- rather than beside a reportable
/// scan root itself, which
/// `a_non_unicode_directory_holding_nothing_reportable_is_walked_through_silently`
/// already covers -- still yields a successful snapshot that lists
/// only the reportable sibling.
///
/// Skipped when the filesystem itself refuses a single-invalid-byte
/// directory name, via `non_unicode_fixture_supported` (APFS on macOS
/// enforces valid UTF-8 names).
#[cfg(unix)]
#[test]
fn a_non_unicode_sibling_holding_nothing_reportable_leaves_a_reportable_sibling_intact() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let project = TempDir::new("non-unicode-sibling-pin");
    write(project.path(), "reportable/AGENTS.md", SIMPLE_BODY);
    let lossy = OsStr::from_bytes(&[0xff]);
    if !non_unicode_fixture_supported(
        "a_non_unicode_sibling_holding_nothing_reportable_leaves_a_reportable_sibling_intact",
        std::fs::create_dir(project.path().join(lossy)),
    ) {
        return;
    }
    let workspace = project.workspace();
    let fs_tree = FsDefinitionTree::new(None);

    let snapshot = fs_tree
        .snapshot(&workspace)
        .expect("the non-Unicode sibling holds nothing reportable, so it never blocks the scan");

    let directory_paths: Vec<&str> = snapshot
        .directories
        .iter()
        .map(|directory| directory.path.as_str())
        .collect();
    assert_eq!(directory_paths, vec!["reportable"]);
}
