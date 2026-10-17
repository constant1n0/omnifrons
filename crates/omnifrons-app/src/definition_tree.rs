//! The `DefinitionTree` port (ADR-0005 "Agent identity and portable
//! definition" sub-slice 1e-A): one read-only pass over a workspace that
//! an adapter (sub-slice 1f) performs once, so [`crate::definition::scan`]
//! never walks a filesystem itself.
//!
//! # The port's contract
//!
//! A [`TreeSnapshot`] is the filesystem adapter's own, documented duty;
//! this module only names the shape every implementation -- the real
//! adapter and [`crate::contract::definition::InMemoryDefinitionTree`]
//! alike -- must agree on, mirroring the prototype's own `walk_workspace`
//! and `find_ancestors`:
//!
//! - [`TreeSnapshot::directories`] holds exactly the directories that hold
//!   at least one identity file or at least one host-specific extra
//!   relevant to some host's upward or fallback walk: the prototype's
//!   own `agent_roots`, built from `has_identity_files`, widened by this
//!   port so a directory holding only an extra is not silently dropped
//!   the way the prototype would drop it. Such a directory is never a
//!   root in [`crate::definition::scan`]'s own sense -- it never gets an
//!   [`crate::definition::AgentReport`] of its own, and it is never a
//!   nesting parent or anything needing consent -- but it still reaches
//!   an enclosing agent's own warning chain, as
//!   [`omnifrons_domain::nesting::ChainLink::Extras`]. A directory
//!   holding neither an identity file nor a host extra is never listed,
//!   even when it is walked.
//! - A host configuration directory's identity files (for example
//!   `.claude/CLAUDE.md`) are attributed to its *parent* directory's
//!   [`DirectoryEntry`], never listed as a root of their own -- the
//!   prototype's own `read_identity_files` reads a host config
//!   directory's files into the *enclosing* directory's result.
//! - Every directory in [`omnifrons_domain::hosts::SKIP_DIRS`] is pruned from the
//!   walk entirely (the prototype's `dirnames[:] = sorted(... not in
//!   SKIP_DIRS)`); nothing below one is ever visited.
//! - The walk never follows a symlink (the prototype's
//!   `os.walk(root, followlinks=False)`): a symlinked directory is never
//!   descended into, and a symlinked identity file is reported with
//!   [`omnifrons_domain::definition::FileShape::Symlink`] rather than
//!   read.
//! - [`TreeSnapshot::directories`] and [`TreeSnapshot::skills`] are both
//!   in the prototype's own sorted walk order: a pre-order, depth-first
//!   walk that visits each directory's children in name-sorted order
//!   (the prototype's `sorted(dirnames)` at every level of `os.walk`).
//!   In particular, a directory's own `SKILL.md`, if any, is recorded
//!   the instant that directory is visited, before any of its
//!   subdirectories are descended into -- it is never reordered
//!   relative to a descendant's `SKILL.md` by comparing the file name
//!   `SKILL.md` against a subdirectory's own name, as if the two were
//!   siblings at the same level.
//!   [`crate::contract::definition::definition_tree_contract`] asserts
//!   this order mechanically.
//! - [`TreeSnapshot::ancestors`] holds the agent roots above the scan
//!   root, nearest first (the prototype's `find_ancestors`, walking
//!   `root.parents`), stopping at the user's home directory boundary --
//!   the prototype's own `user_home()` guard in `has_identity_files`,
//!   applied by the adapter, never by this port's caller. Each
//!   `Ancestor::identity_files` entry the adapter supplies may be
//!   either that file's identity-kind file name (for example
//!   `"AGENTS.md"`, the prototype's own shape -- it keys its own
//!   `files` mapping this way -- and the form
//!   [`omnifrons_domain::definition::IdentityKind::file_name`]
//!   returns) or its token (for example `"agents"`, the form
//!   [`omnifrons_domain::definition::IdentityKind::as_str`] returns):
//!   [`crate::definition`]'s own warnings composition tries
//!   [`omnifrons_domain::definition::IdentityKind::from_file_name`]
//!   first, then falls back to
//!   [`omnifrons_domain::definition::IdentityKind::parse`], so either
//!   form survives into that ancestor's warning chain; an entry
//!   neither recognizes is silently dropped rather than reported as an
//!   error. Each `Ancestor::host_extras` entry is, by contrast, always
//!   a host-specific extra file name relevant to a host's upward or
//!   fallback walk (for example `CLAUDE.local.md`, `.claude/AGENTS.md`),
//!   never an identity file and never parsed through either
//!   `IdentityKind` lookup -- [`crate::definition`]'s own warnings
//!   composition matches a host's kinds against `identity_files` and
//!   `host_extras` together. This port's own contract does not
//!   constrain which of an ancestor's files land in one field or the
//!   other beyond that; sub-slice 1f's filesystem adapter fills both.
//!
//! This module only declares the shape; sub-slice 1f's filesystem adapter
//! is the one piece of code that must honor every rule above against a
//! real filesystem. [`crate::contract::definition::InMemoryDefinitionTree`]
//! never walks anything -- a test hands it an already-decided
//! [`TreeSnapshot`] -- so only [`crate::contract::definition::definition_tree_contract`]'s
//! *shape* assertions (clean relative paths, sorted order, a `SKILL.md`
//! basename) are checked against it; the walk, pruning, link, and home
//! rules above are sub-slice 1f's own responsibility to prove against a
//! real directory tree.

use omnifrons_domain::definition::IdentityFile;
use omnifrons_domain::nesting::Ancestor;

use crate::harness_adapter::WorkspaceRoot;

/// One read-only pass over a workspace's agent roots, discovered skills,
/// and outside ancestors -- the prototype's `walk_workspace` plus
/// `find_ancestors`, combined into the one value [`crate::definition::scan`]
/// composes over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeSnapshot {
    /// The directories holding at least one identity file or host
    /// extra, in the prototype's sorted walk order; see this module's
    /// own documentation for why a host-extra-only entry is allowed
    /// here even though it is never a root.
    pub directories: Vec<DirectoryEntry>,
    /// Every discovered `SKILL.md`, in the prototype's sorted walk order.
    pub skills: Vec<SkillFile>,
    /// The agent roots above the scan root, nearest first. Each
    /// entry's `identity_files` must use the token form, never the
    /// file-name form -- see this module's own documentation.
    pub ancestors: Vec<Ancestor>,
}

/// One directory the walk found holding at least one identity file or
/// host extra (the prototype's per-directory `read_identity_files` plus
/// `find_host_extras`, read back as one value, widened to allow an
/// entry with no identity file at all -- see this module's own
/// documentation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    /// This directory's path, POSIX-style and relative to the scan root;
    /// `"."` names the scan root itself.
    pub path: String,
    /// The identity files found here, including any attributed to this
    /// directory from a host configuration directory beneath it (for
    /// example `.claude/CLAUDE.md`); empty exactly when
    /// [`Self::host_extras`] is not, for a directory holding a host
    /// extra but no identity file of its own.
    pub identity_files: Vec<IdentityFile>,
    /// The host-specific extra file names found alongside the identity
    /// files (for example `CLAUDE.local.md`), never read as identity.
    pub host_extras: Vec<String>,
    /// Which of [`omnifrons_domain::hosts::GENERATED_STATE_DIRS`]'s entries exist at
    /// this directory.
    pub generated_state_dirs: Vec<&'static str>,
}

/// One discovered `SKILL.md` (the prototype's own walk, which records a
/// regular file at that name and nothing else).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFile {
    /// This file's path, POSIX-style and relative to the scan root; its
    /// basename is always `SKILL.md`.
    pub path: String,
    /// The file's bytes.
    pub bytes: Vec<u8>,
}

/// Why a [`DefinitionTree`] could not produce a [`TreeSnapshot`]. A
/// minimal, documented set: every variant has a fixed reason token
/// ([`Self::reason`]); no message carries a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DefinitionTreeError {
    /// The scan root itself cannot be opened, read, or is not a
    /// directory.
    #[error("the scan root cannot be read")]
    RootUnreadable,
    /// The walk failed below the scan root (for example a permission
    /// error on a subdirectory the walk could not skip).
    #[error("the directory walk failed")]
    WalkFailed,
    /// A discovered identity file or `SKILL.md` exceeds the adapter's
    /// size cap.
    #[error("a discovered entry exceeds the size limit")]
    EntryTooLarge,
}

impl DefinitionTreeError {
    /// This error's fixed reason token.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::RootUnreadable => "root-unreadable",
            Self::WalkFailed => "walk-failed",
            Self::EntryTooLarge => "entry-too-large",
        }
    }
}

/// A port over one read-only pass of a workspace's agent definition
/// (ADR-0005 sub-slice 1e-A/1f).
pub trait DefinitionTree {
    /// Walk `root` once and report every agent-identity directory,
    /// discovered skill, and outside ancestor -- see this module's own
    /// documentation for the shape every implementation must agree on.
    ///
    /// # Errors
    ///
    /// Returns [`DefinitionTreeError`] naming why the walk could not be
    /// completed.
    fn snapshot(&self, root: &WorkspaceRoot) -> Result<TreeSnapshot, DefinitionTreeError>;
}
