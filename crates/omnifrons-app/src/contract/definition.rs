//! Fakes and a reusable contract for the `DefinitionTree` port (ADR-0005
//! sub-slice 1e-A): an `InMemoryDefinitionTree` that returns a prepared
//! `TreeSnapshot` for a given workspace root, a small builder so a test
//! never has to assemble every field's `Vec` by hand, and
//! `definition_tree_contract`, which any implementation -- this fake
//! today, sub-slice 1f's filesystem adapter later -- must pass.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use omnifrons_domain::definition::IdentityFile;
use omnifrons_domain::nesting::Ancestor;

use crate::definition_tree::{
    DefinitionTree, DefinitionTreeError, DirectoryEntry, SkillFile, TreeSnapshot,
};
use crate::harness_adapter::WorkspaceRoot;

/// An in-memory [`DefinitionTree`] test double: one prepared
/// [`TreeSnapshot`] per workspace root, keyed by its canonical path.
#[derive(Debug, Default)]
pub struct InMemoryDefinitionTree {
    snapshots: HashMap<PathBuf, TreeSnapshot>,
}

impl InMemoryDefinitionTree {
    /// A double with no prepared snapshot for any root.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Prepares `snapshot` as the result of scanning `root`.
    #[must_use]
    pub fn with_snapshot(mut self, root: &WorkspaceRoot, snapshot: TreeSnapshot) -> Self {
        self.snapshots.insert(root.path().to_path_buf(), snapshot);
        self
    }
}

impl DefinitionTree for InMemoryDefinitionTree {
    fn snapshot(&self, root: &WorkspaceRoot) -> Result<TreeSnapshot, DefinitionTreeError> {
        self.snapshots
            .get(root.path())
            .cloned()
            .ok_or(DefinitionTreeError::RootUnreadable)
    }
}

/// A builder for a [`TreeSnapshot`] fixture (ADR-0005 sub-slice 1e-A): a
/// test adds a directory, a skill, or an ancestor one call at a time
/// instead of assembling every field's `Vec` by hand.
#[derive(Debug, Default)]
pub struct TreeSnapshotBuilder {
    directories: Vec<DirectoryEntry>,
    skills: Vec<SkillFile>,
    ancestors: Vec<Ancestor>,
}

impl TreeSnapshotBuilder {
    /// An empty snapshot under construction.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a directory holding `identity_files`, found at `path` (POSIX,
    /// relative to the scan root; `"."` for the scan root itself).
    ///
    /// # Panics
    ///
    /// Panics if `path` was already added by an earlier call: this
    /// builder never merges or reorders entries, so a fixture needing
    /// two directories sharing one path -- `scan`'s own contract makes
    /// that a malformed snapshot, never a legitimate fixture -- must
    /// build its [`TreeSnapshot`] directly instead.
    #[must_use]
    pub fn directory(
        mut self,
        path: impl Into<String>,
        identity_files: Vec<IdentityFile>,
        host_extras: Vec<String>,
        generated_state_dirs: Vec<&'static str>,
    ) -> Self {
        let path = path.into();
        assert!(
            !self
                .directories
                .iter()
                .any(|existing| existing.path == path),
            "TreeSnapshotBuilder::directory: {path} was already added; build a TreeSnapshot \
             directly for a fixture needing a duplicate directory path"
        );
        self.directories.push(DirectoryEntry {
            path,
            identity_files,
            host_extras,
            generated_state_dirs,
            portable_agent_folder: false,
        });
        self
    }

    /// Adds a discovered `SKILL.md` at `path` (POSIX, relative to the
    /// scan root; its basename must be `SKILL.md`).
    #[must_use]
    pub fn skill(mut self, path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        self.skills.push(SkillFile {
            path: path.into(),
            bytes: bytes.into(),
        });
        self
    }

    /// Adds an ancestor above the scan root, nearest first: callers must
    /// add ancestors in that order themselves, since this builder never
    /// reorders them. `host_extras` is this ancestor's host-specific
    /// extra file names (for example `.claude/AGENTS.md`), relevant to
    /// a host's upward or fallback walk but never an identity file
    /// itself -- see [`Ancestor::host_extras`]'s own doc comment.
    ///
    /// # Panics
    ///
    /// Panics if `path` was already added by an earlier call, for the
    /// same reason as [`Self::directory`]: this builder never merges
    /// or reorders entries, so a fixture needing a duplicate ancestor
    /// path must build its [`TreeSnapshot`] directly instead.
    #[must_use]
    pub fn ancestor(
        mut self,
        path: impl Into<String>,
        identity_files: Vec<String>,
        host_extras: Vec<String>,
    ) -> Self {
        let path = path.into();
        assert!(
            !self.ancestors.iter().any(|existing| existing.path == path),
            "TreeSnapshotBuilder::ancestor: {path} was already added; build a TreeSnapshot \
             directly for a fixture needing a duplicate ancestor path"
        );
        self.ancestors.push(Ancestor {
            path,
            identity_files,
            host_extras,
        });
        self
    }

    /// The assembled snapshot.
    #[must_use]
    pub fn build(self) -> TreeSnapshot {
        TreeSnapshot {
            directories: self.directories,
            skills: self.skills,
            ancestors: self.ancestors,
        }
    }
}

// -- contract --

/// A path's components, with `"."` (the scan root) represented as none
/// -- mirrors `omnifrons_domain::nesting`'s own, private convention for
/// the same shape.
fn components(path: &str) -> Vec<&str> {
    if path == "." {
        Vec::new()
    } else {
        path.split('/').collect()
    }
}

/// `path` is non-empty, never absolute, and carries no `..` or bare `.`
/// component -- the shape every [`DirectoryEntry`]/[`SkillFile`] path
/// must have.
fn assert_clean_relative(path: &str, label: &str) {
    assert!(!path.is_empty(), "{label} path must not be empty");
    assert!(
        !path.starts_with('/'),
        "{label} path {path} must be relative, not absolute"
    );
    assert!(
        path == "."
            || path
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."),
        "{label} path {path} must be a clean relative path"
    );
}

/// A skill path's own parent directory (mirrors `crate::definition`'s
/// own, private `parent_directory` convention for the same shape): the
/// path with its final `/`-separated segment dropped, or `"."` for a
/// skill at the scan root -- the directory the prototype's walk was
/// visiting the instant it recorded this `SKILL.md`.
fn skill_parent(path: &str) -> &str {
    path.rfind('/').map_or(".", |index| &path[..index])
}

/// `entries` pairs each item's own display path with the *ordering key*
/// that fixes its position in the prototype's sorted walk order: a
/// pre-order, depth-first walk visiting each directory's children in
/// name-sorted order. For a [`DirectoryEntry`] that key is its own
/// path; for a [`SkillFile`] it is the file's parent directory
/// ([`skill_parent`]), never the skill's own path -- the walk records a
/// directory's own file the instant it visits that directory, before
/// descending into any of its subdirectories, so the file's place in
/// the order is exactly its parent's, regardless of the file name
/// itself. Comparing full skill paths (basename included) would instead
/// compare the literal string `"SKILL.md"` against real subdirectory
/// names as if it were one more sibling to sort among them, wrongly
/// moving a directory's own skill after a subdirectory whose name sorts
/// below `"SKILL.md"` (for example `"A"`, `"1dir"`, or `".hidden"`).
///
/// Comparing each key's own component sequence lexicographically (a
/// shorter sequence that is a prefix of a longer one always sorts
/// first) reproduces exactly the walk order for these keys: removing
/// entries from a pre-order walk never reorders the ones that remain,
/// and two fixed directories' relative order under this comparison
/// never depends on what other entries exist.
fn assert_sorted_by_walk_order<'a>(entries: impl Iterator<Item = (&'a str, &'a str)>, label: &str) {
    let keyed: Vec<(&str, Vec<&str>)> = entries
        .map(|(display, key)| (display, components(key)))
        .collect();
    for pair in keyed.windows(2) {
        assert!(
            pair[0].1 <= pair[1].1,
            "{label} must be in the prototype's sorted walk order ({} then {})",
            pair[0].0,
            pair[1].0
        );
    }
}

/// Every path `entries` yields is distinct from every other: a
/// [`TreeSnapshot`]'s own `directories` or `ancestors` listing the same
/// path twice is malformed, since a caller that assumes each one names
/// a single, unambiguous entry -- `crate::definition::scan`'s own
/// `agent_directories`/`roots` pairing, for directories; `nesting::relate`'s
/// own `roots` precondition, downstream of the same snapshot -- would
/// otherwise have to guess which entry is the real one.
fn assert_unique_paths<'a>(paths: impl Iterator<Item = &'a str>, label: &str) {
    let mut seen: HashSet<&str> = HashSet::new();
    for path in paths {
        assert!(
            seen.insert(path),
            "{label} paths must be unique: {path} is listed more than once"
        );
    }
}

/// Asserts [`DefinitionTree`]'s own contract (`crate::definition_tree`'s
/// own documentation) on whatever `tree` returns for `root`: every
/// directory and skill path is clean and relative, a listed directory
/// always holds at least one identity file or host extra (never
/// neither), a skill's basename is `SKILL.md`, directories and skills
/// are in the prototype's sorted walk order, directory paths are
/// unique, ancestors are nearest first, ancestor paths are unique, and
/// a listed ancestor likewise always holds at least one identity file
/// or host extra (never neither).
///
/// Checks only the shape a [`TreeSnapshot`] value must have; it cannot
/// check the walk, pruning, link, or home-boundary rules that only a
/// filesystem adapter can violate -- see `crate::definition_tree`'s own
/// documentation.
///
/// # Panics
///
/// Panics on any contract violation, or if `tree` has no snapshot
/// prepared for `root`.
pub fn definition_tree_contract<T: DefinitionTree>(tree: &T, root: &WorkspaceRoot) {
    let snapshot = tree
        .snapshot(root)
        .expect("a prepared fixture must snapshot its root");

    for directory in &snapshot.directories {
        assert_clean_relative(&directory.path, "a directory");
        assert!(
            !directory.identity_files.is_empty() || !directory.host_extras.is_empty(),
            "a listed directory must hold at least one identity file or host extra: {}",
            directory.path
        );
    }
    assert_sorted_by_walk_order(
        snapshot
            .directories
            .iter()
            .map(|directory| (directory.path.as_str(), directory.path.as_str())),
        "directories",
    );
    assert_unique_paths(
        snapshot
            .directories
            .iter()
            .map(|directory| directory.path.as_str()),
        "directories",
    );

    for skill in &snapshot.skills {
        assert_clean_relative(&skill.path, "a skill");
        assert_eq!(
            skill.path.rsplit('/').next(),
            Some("SKILL.md"),
            "a skill path must end in SKILL.md, got {}",
            skill.path
        );
    }
    assert_sorted_by_walk_order(
        snapshot
            .skills
            .iter()
            .map(|skill| (skill.path.as_str(), skill_parent(skill.path.as_str()))),
        "skills",
    );

    assert_unique_paths(
        snapshot
            .ancestors
            .iter()
            .map(|ancestor| ancestor.path.as_str()),
        "ancestors",
    );
    let mut previous_depth: Option<usize> = None;
    for ancestor in &snapshot.ancestors {
        assert!(
            !ancestor.identity_files.is_empty() || !ancestor.host_extras.is_empty(),
            "a listed ancestor must hold at least one identity file or host extra: {}",
            ancestor.path
        );
        let depth = ancestor.path.split('/').count();
        if let Some(previous) = previous_depth {
            assert!(
                depth <= previous,
                "ancestors must be nearest first: {} is not nearer than the previous one",
                ancestor.path
            );
        }
        previous_depth = Some(depth);
    }
}
