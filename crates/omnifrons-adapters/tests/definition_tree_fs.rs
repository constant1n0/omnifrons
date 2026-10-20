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
