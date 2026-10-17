//! The read-only `scan` and `verify` services (ADR-0005 "Agent identity
//! and portable definition" sub-slices 1e-A and 1e-D): composes the
//! domain's pure classification, nesting, skills-index, and warnings
//! modules over one [`crate::definition_tree::DefinitionTree`] snapshot
//! into the prototype's own `scan()` and `verify_agent()` reports,
//! without ever touching a filesystem itself.
//!
//! Translated from the prototype's `scan`, `inspect_agent`,
//! `canonical_bytes`, and `ancestor_chain`: this module's own [`scan`]
//! walks nothing -- `ports.tree` already decided what exists -- and
//! instead classifies each discovered directory, attributes skills to
//! their nearest owning root, and asks the domain's `warnings` module for
//! each root's warnings, in that order.
//!
//! [`verify`] is translated from the prototype's own `verify_agent`,
//! which plans a full adapt (`build_plan(agent, PlanOptions(nested=True),
//! home)`) purely to read its `state`, `refusal`, and `operations` back
//! off, and never once writes. This module's own [`verify`] folds that
//! same reading-without-writing boundary one step earlier: it is the
//! read-only *subset* of `build_plan` -- the state, the refusals, and the
//! pending operations this crate can already know from one
//! [`crate::definition_tree::TreeSnapshot`] -- and never plans an actual
//! write itself. The write path (an eventual `adapt`) is sub-slice 2's
//! own, later service; see [`verify`]'s own "Deferred to slice 2"
//! section for exactly which of `build_plan`'s refusals and operations
//! that boundary leaves out.
//!
//! # Seam with sub-slice 1f
//!
//! Sub-slice 1f's filesystem adapter is the only remaining piece: an
//! implementation of [`crate::definition_tree::DefinitionTree`] that
//! walks a real workspace into a [`crate::definition_tree::TreeSnapshot`]
//! honoring that port's own documented contract
//! (`crate::definition_tree`'s module documentation). Nothing in this
//! module changes once that adapter exists.

use std::collections::HashMap;

use omnifrons_domain::definition::{
    Classification, DirectoryIdentity, FileShape, IdentityFile, IdentityKind, RootState, classify,
    title as extract_title,
};
use omnifrons_domain::guidance::{ManagedBlock, ManagedFileError, ManagedFileKind};
use omnifrons_domain::hosts;
use omnifrons_domain::nesting::{self, Ancestor, ChainLink, NestingError, Parent, RootRelation};
use omnifrons_domain::skills_index::{
    SkillIndexStatus, SkillRow, block_needed, has_precedence_phrases, read_skill_index,
    render_body, rows_to_generate,
};
use omnifrons_domain::skills_safety::{
    InvalidUnicodeField, SkillMetadataError, python_repr, read_skill_metadata, unsafe_row_fields,
};
use omnifrons_domain::warnings::{AgentInput, ChainAncestor, Warning, agent_warnings};

use crate::content_hasher::ContentHasher;
use crate::definition_tree::{
    DefinitionTree, DefinitionTreeError, DirectoryEntry, SkillFile, TreeSnapshot,
};
use crate::harness_adapter::WorkspaceRoot;

/// The ports [`scan`] runs over.
pub struct DefinitionPorts<'a> {
    /// The read-only pass over the workspace.
    pub tree: &'a dyn DefinitionTree,
    /// SHA-256 over identity file bytes.
    pub hasher: &'a dyn ContentHasher,
}

/// Why [`scan`] could not produce a [`ScanReport`]. Every variant has a
/// fixed reason token ([`Self::reason`]); no message carries a path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DefinitionError {
    /// The [`DefinitionTree`] port could not produce a snapshot.
    #[error("the definition tree could not be read: {0}")]
    Tree(DefinitionTreeError),
    /// The discovered roots could not be related ([`nesting::relate`]):
    /// the port returned a malformed or duplicated root path.
    #[error("the discovered roots could not be related: {0}")]
    Nesting(NestingError),
    /// A directory's identity files repeated a kind
    /// ([`DirectoryIdentity::new`]).
    #[error("{0}")]
    DuplicateIdentityKind(omnifrons_domain::definition::DuplicateIdentityKindError),
}

impl DefinitionError {
    /// This error's fixed reason token.
    ///
    /// `tree(...)`/`nesting(...)` name the wrapped port or domain
    /// error's own variant without embedding its message, which for a
    /// [`NestingError`] carries a path. Mirroring
    /// [`DefinitionTreeError::reason`]'s tokens through a static match,
    /// rather than calling it and composing the result at runtime,
    /// keeps this a `const fn` returning `&'static str`, like every
    /// other reason token in this crate.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::Tree(error) => match error {
                DefinitionTreeError::RootUnreadable => "tree(root-unreadable)",
                DefinitionTreeError::WalkFailed => "tree(walk-failed)",
                DefinitionTreeError::EntryTooLarge => "tree(entry-too-large)",
            },
            Self::Nesting(error) => match error {
                NestingError::AbsolutePath(_) => "nesting(absolute-path)",
                NestingError::ParentComponent(_) => "nesting(parent-component)",
                NestingError::EmptyPath => "nesting(empty-path)",
                NestingError::DotSlashPrefix(_) => "nesting(dot-slash-prefix)",
                NestingError::TrailingSlash(_) => "nesting(trailing-slash)",
                NestingError::DoubledSlash(_) => "nesting(doubled-slash)",
                NestingError::DotComponent(_) => "nesting(dot-component)",
                NestingError::DuplicateRoot(_) => "nesting(duplicate-root)",
            },
            Self::DuplicateIdentityKind(_) => "duplicate-identity-kind",
        }
    }
}

/// One identity file's report (the prototype's `IdentityFile.to_json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityFileReport {
    /// Which identity kind this is.
    pub kind: IdentityKind,
    /// The file's size in bytes: `0` for a symlink or an irregular file,
    /// which [`IdentityFile::bytes`] never holds content for -- a
    /// deliberate divergence from the prototype's own `null` there,
    /// chosen so this field stays a plain `u64` rather than an
    /// `Option<u64>`.
    pub bytes: u64,
    /// The SHA-256 of the file's bytes, hex-encoded; `None` for a
    /// symlink or an irregular file, whose bytes were never read.
    pub sha256: Option<String>,
    /// The file's shape on disk.
    pub shape: FileShape,
}

/// The skills-index status for one agent (the prototype's own
/// `SkillIndex`, read back through `AgentReport.to_json`'s `skills` key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillsReport {
    /// The `SKILL.md` files discovered under this agent's own root,
    /// relative to it, normalized and de-duplicated.
    pub discovered: Vec<String>,
    /// The linked `SKILL.md` targets this agent's canonical text indexes.
    pub indexed: Vec<String>,
    /// Discovered, but not indexed.
    pub missing: Vec<String>,
    /// Indexed, but not discovered.
    pub dangling: Vec<String>,
    /// Whether the canonical text already states the skill-precedence
    /// rule.
    pub precedence: bool,
}

/// One agent root's full report (the prototype's `AgentReport.to_json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentReport {
    /// This root's path, POSIX-style and relative to the scan root.
    pub path: String,
    /// This root's classification state.
    pub state: RootState,
    /// This root's identity files, in [`IdentityKind::ALL`] order.
    pub identity_files: Vec<IdentityFileReport>,
    /// The first unfenced `# ` heading in the canonical text, if any.
    pub title: Option<String>,
    /// This root's parent, if any.
    pub parent: Option<Parent>,
    /// The roots nested directly under this one, relative to the scan
    /// root like [`Self::path`] itself.
    pub nested: Vec<String>,
    /// Whether adapting this root needs its parent's consent.
    pub consent_required: bool,
    /// This root's skills-index status.
    pub skills: SkillsReport,
    /// This root's warnings, in `WARNING_ORDER`.
    pub warnings: Vec<Warning>,
}

/// A full scan (the prototype's `ScanResult.to_json`, minus the `root`
/// and `schema` keys this crate's own caller already knows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    /// The agent roots above the scan root, nearest first.
    pub ancestors: Vec<Ancestor>,
    /// Every discovered agent, in the prototype's own agent order (the
    /// walk's own order).
    pub agents: Vec<AgentReport>,
}

/// A full verification of the scan root's own canonical definition (the
/// prototype's own `verify_agent` return value, minus the `schema` and
/// `agent` keys this crate's own caller already knows, and minus
/// `backups`, which belongs to the write path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    /// The scan root's own classification state (the prototype's own
    /// `plan.state`).
    pub state: RootState,
    /// Whether [`Self::problems`] is empty -- the prototype's own
    /// `not problems`.
    pub canonical: bool,
    /// Every problem found, each in the prototype's own exact wording:
    /// `"state is <token>"`, `"refused (<code>): <detail>"`, or
    /// `"pending: <description>"`. See [`verify`]'s own documentation for
    /// exactly which of the prototype's refusals and pending operations
    /// this read-only pass can and cannot know.
    pub problems: Vec<String>,
}

/// `identity_files`' reports, in [`IdentityKind::ALL`] order (the
/// prototype's own `IDENTITY_KINDS`-ordered dict, read back through
/// `IdentityFile.to_json`).
fn identity_file_reports(
    identity_files: &[IdentityFile],
    hasher: &dyn ContentHasher,
) -> Vec<IdentityFileReport> {
    IdentityKind::ALL
        .into_iter()
        .filter_map(|kind| identity_files.iter().find(|file| file.kind == kind))
        .map(|file| IdentityFileReport {
            kind: file.kind,
            bytes: file.bytes.len() as u64,
            sha256: matches!(file.shape, FileShape::Regular)
                .then(|| hasher.sha256(&file.bytes).to_hex()),
            shape: file.shape.clone(),
        })
        .collect()
}

/// The bytes [`classify`] named canonical for this directory, if any
/// (the prototype's own `canonical_bytes`, read back from a
/// [`Classification::source`] this module already computed instead of
/// re-deriving it).
fn canonical_bytes(identity_files: &[IdentityFile], source: Option<IdentityKind>) -> Option<&[u8]> {
    let kind = source?;
    identity_files
        .iter()
        .find(|file| file.kind == kind)
        .map(|file| file.bytes.as_slice())
}

/// `relative`'s absolute POSIX path under `workspace`'s canonical root
/// (`"."` for the root itself) -- [`read_skill_index`]'s own `root`
/// parameter, which only uses it to relativize an absolute link target.
fn absolute_posix(workspace: &WorkspaceRoot, relative: &str) -> String {
    let joined = if relative == "." {
        workspace.path().to_path_buf()
    } else {
        workspace.path().join(relative)
    };
    joined.to_string_lossy().replace('\\', "/")
}

/// `path`'s own suffix past `root`, POSIX-joined (the prototype's
/// `child.relative_to(path).as_posix()`/
/// `skill.relative_to(owner).as_posix()`, applied to this crate's
/// scan-root-relative convention instead of real filesystem paths):
/// every caller already established that `path` equals `root` or sits
/// below it, through [`nesting::nearest_root`] or a [`RootRelation`].
fn relative_within(root: &str, path: &str) -> String {
    if root == "." {
        return path.to_string();
    }
    path.strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path)
        .to_string()
}

/// The parent directory of a clean, scan-root-relative POSIX `path` (the
/// prototype's `skill.parent`): `"."` for a top-level entry.
fn parent_directory(path: &str) -> &str {
    path.rfind('/').map_or(".", |index| &path[..index])
}

/// Groups `skills` by their nearest owning root (the prototype's own
/// `owner = nearest_root(skill.parent, roots)` loop in `scan`), each
/// skill's path made relative to that root ([`relative_within`]); a
/// skill outside every discovered root is dropped, exactly as the
/// prototype's own `if owner is not None` guard drops it.
fn attribute_skills<'a>(roots: &[&'a str], skills: &[SkillFile]) -> HashMap<&'a str, Vec<String>> {
    let mut by_root: HashMap<&'a str, Vec<String>> = HashMap::new();
    for skill in skills {
        let parent = parent_directory(&skill.path);
        let Some(owner) = nesting::nearest_root(roots, parent) else {
            continue;
        };
        by_root
            .entry(owner)
            .or_default()
            .push(relative_within(owner, &skill.path));
    }
    by_root
}

/// Every directory in `directories` holding at least one identity file
/// -- the prototype's own `agent_roots`, built by `walk_workspace`'s
/// `has_identity_files` gate -- in `directories`' own order. Shared by
/// [`scan`] and [`verify`] so both derive `roots` the identical way: an
/// extras-only entry (`crate::definition_tree`'s own documentation) is
/// never one of these, so it can never become an agent, a nesting
/// parent, or anything needing consent through [`nesting::relate`].
fn agent_root_paths(directories: &[DirectoryEntry]) -> Vec<&str> {
    directories
        .iter()
        .filter(|directory| !directory.identity_files.is_empty())
        .map(|directory| directory.path.as_str())
        .collect()
}

/// Every directory in `directories` holding no identity file of its own
/// but at least one host extra, in `directories`' own order --
/// [`nesting::warning_chain`]'s own `extras_only` argument: never a
/// root ([`agent_root_paths`] excludes it), but still a chain member
/// for an enclosing agent.
fn extras_only_paths(directories: &[DirectoryEntry]) -> Vec<&str> {
    directories
        .iter()
        .filter(|directory| {
            directory.identity_files.is_empty() && !directory.host_extras.is_empty()
        })
        .map(|directory| directory.path.as_str())
        .collect()
}

/// The skills report for one agent ([`read_skill_index`] against `text`,
/// empty when the agent has no canonical text to read -- so every
/// discovered skill reports `missing` rather than this struct going
/// absent, a deliberate divergence from the prototype's own
/// `Optional[SkillIndex]`).
fn skills_report(text: Option<&[u8]>, discovered: &[String], root_absolute: &str) -> SkillsReport {
    let status = read_skill_index(text.unwrap_or(&[]), discovered, root_absolute);
    SkillsReport {
        discovered: status.discovered,
        indexed: status.indexed,
        missing: status.missing,
        dangling: status.dangling,
        precedence: status.precedence,
    }
}

/// Each link of `chain`'s own identity-file names, `&'static str` (the
/// prototype's own `ancestor_files` dict keys): a link inside the scan
/// -- a root or an extras-only directory alike -- reads them straight
/// from its own [`DirectoryEntry`] (always empty for an extras-only
/// one, by construction); an outside ancestor's are recognized from
/// [`Ancestor::identity_files`]
/// by trying [`IdentityKind::from_file_name`] first -- the prototype's
/// own shape (for example `"AGENTS.md"`) -- then falling back to
/// [`IdentityKind::parse`] (the token form, for example `"agents"`), so
/// either spelling survives into the warning chain; an entry neither
/// function recognizes is dropped, exactly like any other string
/// neither recognizes. Whoever populates [`Ancestor::identity_files`]
/// -- today, this crate's own tests; later, sub-slice 1f's adapter --
/// may use either form; the file-name form is preferred, since it is
/// the one the prototype itself stores.
fn chain_identity_files(
    directories: &[DirectoryEntry],
    chain: &[ChainLink],
) -> Vec<Vec<&'static str>> {
    chain
        .iter()
        .map(|link| match link {
            ChainLink::Root(path) | ChainLink::Extras(path) => directories
                .iter()
                .find(|directory| directory.path == *path)
                .map(|owner| {
                    owner
                        .identity_files
                        .iter()
                        .map(|file| file.kind.file_name())
                        .collect()
                })
                .unwrap_or_default(),
            ChainLink::Outside(ancestor) => ancestor
                .identity_files
                .iter()
                .filter_map(|entry| {
                    IdentityKind::from_file_name(entry).or_else(|| IdentityKind::parse(entry))
                })
                .map(|kind| kind.file_name())
                .collect(),
        })
        .collect()
}

/// Each link of `chain`'s own host-specific extra file names,
/// `&'static str`, recognized the same way [`warnings_for`] recognizes
/// this agent's own root -- mapped through [`hosts::EXTRA_READERS`]'s
/// keys. A link inside the scan -- a root or an extras-only directory
/// alike -- reads them from its own [`DirectoryEntry::host_extras`]
/// (the whole reason an extras-only directory is a chain member at
/// all); an outside ancestor's come from
/// [`Ancestor::host_extras`]. An entry neither recognizes is dropped,
/// exactly like [`chain_identity_files`] drops an unrecognized identity
/// entry.
fn chain_host_extras(
    directories: &[DirectoryEntry],
    chain: &[ChainLink],
) -> Vec<Vec<&'static str>> {
    chain
        .iter()
        .map(|link| {
            let names: &[String] = match link {
                ChainLink::Root(path) | ChainLink::Extras(path) => directories
                    .iter()
                    .find(|directory| directory.path == *path)
                    .map_or(&[], |owner| owner.host_extras.as_slice()),
                ChainLink::Outside(ancestor) => ancestor.host_extras.as_slice(),
            };
            names
                .iter()
                .filter_map(|name| {
                    hosts::EXTRA_READERS
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|&(key, _)| key)
                })
                .collect()
        })
        .collect()
}

/// `directory`'s warnings (the prototype's own `agent_warnings` call in
/// `scan`): builds the [`AgentInput`] this root needs from
/// already-computed pieces and asks the domain's pure [`agent_warnings`]
/// for the answer.
fn warnings_for(
    directories: &[DirectoryEntry],
    roots: &[&str],
    extras_only: &[&str],
    ancestors: &[Ancestor],
    directory: &DirectoryEntry,
    relation: &RootRelation,
    canonical_text: Option<&[u8]>,
) -> Vec<Warning> {
    let identity_kinds: Vec<IdentityKind> = directory
        .identity_files
        .iter()
        .map(|file| file.kind)
        .collect();
    let decoded_text = canonical_text.map(String::from_utf8_lossy);
    let text: Option<&str> = decoded_text.as_deref();
    let override_bytes: Option<&[u8]> = directory
        .identity_files
        .iter()
        .find(|file| file.kind == IdentityKind::AgentsOverride)
        .map(|file| file.bytes.as_slice());

    let chain_links = nesting::warning_chain(roots, &directory.path, extras_only, ancestors);
    let chain_files = chain_identity_files(directories, &chain_links);
    let chain_extras = chain_host_extras(directories, &chain_links);
    let chain: Vec<ChainAncestor<'_>> = chain_links
        .iter()
        .zip(chain_files.iter())
        .zip(chain_extras.iter())
        .map(|((link, files), extras)| ChainAncestor {
            display: match link {
                ChainLink::Root(path) | ChainLink::Extras(path) => path.as_str(),
                ChainLink::Outside(ancestor) => ancestor.path.as_str(),
            },
            identity_files: files.as_slice(),
            host_extras: extras.as_slice(),
        })
        .collect();

    let nested_relative: Vec<String> = relation
        .nested
        .iter()
        .map(|child| relative_within(&directory.path, child))
        .collect();
    let nested_refs: Vec<&str> = nested_relative.iter().map(String::as_str).collect();

    let host_extras: Vec<&'static str> = directory
        .host_extras
        .iter()
        .filter_map(|name| {
            hosts::EXTRA_READERS
                .iter()
                .find(|(key, _)| key == name)
                .map(|&(key, _)| key)
        })
        .collect();

    agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        text,
        override_bytes,
        chain: &chain,
        nested: &nested_refs,
        host_extras: &host_extras,
        generated_state_dirs_present: &directory.generated_state_dirs,
    })
}

/// The pieces every per-root composition step shares, gathered once by
/// [`scan`] before it loops over the discovered roots.
struct ScanContext<'a> {
    /// SHA-256 over identity file bytes.
    hasher: &'a dyn ContentHasher,
    /// The workspace being scanned.
    workspace: &'a WorkspaceRoot,
    /// Every discovered directory, for chain lookups.
    directories: &'a [DirectoryEntry],
    /// Every agent root's own path (a directory holding at least one
    /// identity file), for `nesting` calls needing roots-only
    /// semantics: [`nesting::relate`], [`attribute_skills`], and
    /// [`nesting::warning_chain`]'s own `roots` argument.
    roots: &'a [&'a str],
    /// Every directory with no identity file of its own but at least
    /// one host extra -- never a root -- for
    /// [`nesting::warning_chain`]'s own `extras_only` argument.
    extras_only: &'a [&'a str],
    /// The agent roots above the scan root, nearest first.
    ancestors: &'a [Ancestor],
    /// Every root's discovered skills, relative to it.
    discovered_by_root: &'a HashMap<&'a str, Vec<String>>,
}

/// One agent root's full report: [`scan`] already computed this root's
/// classification and relation to the others; this composes the title,
/// identity-file reports, skills report, and warnings around them.
fn build_agent_report(
    ctx: &ScanContext<'_>,
    directory: &DirectoryEntry,
    classification: &Classification,
    relation: &RootRelation,
) -> AgentReport {
    let canonical_text = canonical_bytes(&directory.identity_files, classification.source);
    let title = canonical_text.and_then(extract_title);
    let discovered = ctx
        .discovered_by_root
        .get(directory.path.as_str())
        .cloned()
        .unwrap_or_default();
    let root_absolute = absolute_posix(ctx.workspace, &directory.path);
    let skills = skills_report(canonical_text, &discovered, &root_absolute);
    let warnings = warnings_for(
        ctx.directories,
        ctx.roots,
        ctx.extras_only,
        ctx.ancestors,
        directory,
        relation,
        canonical_text,
    );
    AgentReport {
        path: directory.path.clone(),
        state: classification.state,
        identity_files: identity_file_reports(&directory.identity_files, ctx.hasher),
        title,
        parent: relation.parent.clone(),
        nested: relation.nested.clone(),
        consent_required: relation.consent_required(),
        skills,
        warnings,
    }
}

/// Scans `workspace` through `ports.tree` and composes the domain's
/// classification, nesting, skills-index, and warnings modules into a
/// full report (the prototype's own `scan`).
///
/// # Errors
///
/// Returns [`DefinitionError::Tree`] when the [`DefinitionTree`] port
/// could not produce a snapshot, [`DefinitionError::Nesting`] when the
/// discovered roots cannot be related (a malformed or duplicated root
/// path), or [`DefinitionError::DuplicateIdentityKind`] when a
/// directory's identity files repeat a kind.
pub fn scan(
    ports: &DefinitionPorts<'_>,
    workspace: &WorkspaceRoot,
) -> Result<ScanReport, DefinitionError> {
    let snapshot = ports
        .tree
        .snapshot(workspace)
        .map_err(DefinitionError::Tree)?;

    // `roots` is only every directory holding at least one identity
    // file -- never an extras-only entry, which `nesting::relate` must
    // never treat as a root, a nesting parent, or anything needing
    // consent. `extras_only` is the complement that still matters: no
    // identity file of its own, but a host extra relevant to an
    // enclosing agent's own warning chain.
    let roots = agent_root_paths(&snapshot.directories);
    let extras_only = extras_only_paths(&snapshot.directories);

    // Classification never runs on a directory that cannot be an
    // agent: filtering by membership in `roots` -- rather than
    // re-testing `directory.identity_files.is_empty()` here too --
    // keeps `agent_root_paths` the one place that decides which
    // directories are roots, so `agent_directories` is already in
    // `roots`' own order.
    let agent_directories: Vec<&DirectoryEntry> = snapshot
        .directories
        .iter()
        .filter(|directory| roots.contains(&directory.path.as_str()))
        .collect();
    let mut classifications: Vec<Classification> = Vec::with_capacity(agent_directories.len());
    for directory in agent_directories.iter().copied() {
        let identity = DirectoryIdentity::new(
            directory.identity_files.clone(),
            directory.host_extras.clone(),
        )
        .map_err(DefinitionError::DuplicateIdentityKind)?;
        classifications.push(classify(&identity));
    }

    let relations =
        nesting::relate(&roots, &snapshot.ancestors).map_err(DefinitionError::Nesting)?;

    let discovered_by_root = attribute_skills(&roots, &snapshot.skills);
    let agents: Vec<AgentReport> = {
        let ctx = ScanContext {
            hasher: ports.hasher,
            workspace,
            directories: &snapshot.directories,
            roots: &roots,
            extras_only: &extras_only,
            ancestors: &snapshot.ancestors,
            discovered_by_root: &discovered_by_root,
        };
        // `agent_directories`, `classifications`, and `relations` are
        // already exactly one entry per agent root, in the same
        // order: no second filter is needed to keep them aligned.
        agent_directories
            .iter()
            .copied()
            .zip(classifications.iter())
            .zip(relations.iter())
            .map(|((directory, classification), relation)| {
                build_agent_report(&ctx, directory, classification, relation)
            })
            .collect()
    };

    Ok(ScanReport {
        ancestors: snapshot.ancestors,
        agents,
    })
}

/// Verifies the scan root's own canonical definition through `ports`
/// (ADR-0005 sub-slice 1e-D; the prototype's own `verify_agent`, called
/// with `PlanOptions(nested=True)` -- so the prototype's own
/// `nested_agent` refusal never fires inside it, and this port never
/// computes it either). Composed from the same snapshot [`scan`] uses,
/// sharing its classification helpers ([`classify`] and this module's
/// own private `canonical_bytes`) rather than re-deriving them: the
/// verified agent is always the scan root `"."`, and a nested agent
/// elsewhere in the snapshot is never a problem of its own, exactly as
/// `PlanOptions(nested=True)` says.
///
/// Mirrors `verify_agent`'s own three problem categories, in the
/// prototype's own order: the root's [`RootState`]
/// (`"state is <token>"`, when it is not [`RootState::Canonical`] or
/// [`RootState::HostExtras`]), then the one refusal `build_plan` would
/// raise before ever planning a write (`"refused (<code>): <detail>"`),
/// then every pending operation `build_plan` would still carry once no
/// refusal applies (`"pending: <description>"`). A refusal and a
/// pending operation never both appear: the prototype's own `build_plan`
/// returns its refusal before it ever assigns `plan.steps`, so a refused
/// plan's `operations` list is always empty; this module's own private
/// `verify_problems_after_state` mirrors that short circuit directly
/// through its own early returns.
///
/// # Deferred to slice 2 (the write path)
///
/// This pass can only know what a snapshot already answers; it never
/// plans a write, so it cannot know:
///
/// - the `nested_agent` refusal (excluded structurally, as above);
/// - the `unresolved_file` refusal (an unmanaged secondary identity file
///   -- `GEMINI.md`, `QWEN.md`, `.claude/CLAUDE.md`, `AGENTS.override.md`
///   -- that would still differ from the chosen source once copied) and
///   the `diverging_copy` refusal (an unmanaged secondary that would
///   stop matching once the skill index block is written): both need
///   `plan_operations`'s own secondary-file comparison, not only the
///   root's classification;
/// - the `AGENTS.md` copy operation (`ClaudeOnly`/`IdenticalCopies`
///   states resolve `CLAUDE.md` as the source and would copy it onto
///   `AGENTS.md`) and the opt-in `GEMINI.md` wrapper write
///   (`--with-gemini`): this pass only ever reports the `CLAUDE.md`
///   wrapper, for [`RootState::AgentsOnly`]/[`RootState::NearWrapper`];
/// - every `backup` operation `build_plan` would plan alongside a write
///   it is about to make: a mechanical side effect of the write path,
///   never a problem describing what is wrong with the canonical text
///   itself.
///
/// Within the refusals and pending operations this pass *can* know, one
/// further simplification is documented where it is computed: the
/// `block_layout`/`edited_block` refusal detail never repeats
/// `build_plan`'s own dynamic list of malformed-marker causes (Rust's
/// [`ManagedFileError`] does not distinguish them). The regenerated
/// block's own precedence-rule need is read from the text *before* a
/// located block, exactly as `build_plan`'s own `plan_block` does
/// (never from the whole canonical text, which a block already stating
/// the rule would always satisfy on its own) -- see
/// [`skills_block_outcome`]'s own documentation.
///
/// # Errors
///
/// Returns [`DefinitionError::Tree`] when the [`DefinitionTree`] port
/// could not produce a snapshot, or
/// [`DefinitionError::DuplicateIdentityKind`] when the scan root's
/// identity files repeat a kind.
pub fn verify(
    ports: &DefinitionPorts<'_>,
    workspace: &WorkspaceRoot,
) -> Result<VerifyReport, DefinitionError> {
    let snapshot = ports
        .tree
        .snapshot(workspace)
        .map_err(DefinitionError::Tree)?;
    let roots = agent_root_paths(&snapshot.directories);

    let root_entry = snapshot
        .directories
        .iter()
        .find(|directory| directory.path == ".");
    let identity_files: Vec<IdentityFile> =
        root_entry.map_or_else(Vec::new, |directory| directory.identity_files.clone());
    let host_extras: Vec<String> =
        root_entry.map_or_else(Vec::new, |directory| directory.host_extras.clone());
    let identity = DirectoryIdentity::new(identity_files.clone(), host_extras)
        .map_err(DefinitionError::DuplicateIdentityKind)?;
    let classification = classify(&identity);

    let mut problems = Vec::new();
    if !matches!(
        classification.state,
        RootState::Canonical | RootState::HostExtras
    ) {
        problems.push(format!("state is {}", classification.state.as_str()));
    }
    problems.extend(verify_problems_after_state(
        ports,
        workspace,
        &snapshot,
        &roots,
        &identity_files,
        &classification,
    ));

    Ok(VerifyReport {
        state: classification.state,
        canonical: problems.is_empty(),
        problems,
    })
}

/// Every problem after the state check: the prototype's own refusal, or
/// else its pending operations, directly mirroring `build_plan`'s own
/// short circuit (see [`verify`]'s own documentation) through this
/// function's early returns. Every state [`classify`] can produce is
/// covered: [`RootState::None`] returns nothing (the prototype's own
/// `if report.state == NONE: return plan`, an unrefused, empty plan),
/// [`RootState::Unsafe`]/[`RootState::Conflict`] each return their one
/// refusal, and every other state falls through to the skill-metadata
/// and skills-block checks below.
fn verify_problems_after_state(
    ports: &DefinitionPorts<'_>,
    workspace: &WorkspaceRoot,
    snapshot: &TreeSnapshot,
    roots: &[&str],
    identity_files: &[IdentityFile],
    classification: &Classification,
) -> Vec<String> {
    match classification.state {
        RootState::Unsafe => {
            return vec![format!(
                "refused ({}): {}",
                RootState::Unsafe.as_str(),
                classification.reason
            )];
        }
        RootState::Conflict => {
            return vec![format!(
                "refused ({}): {}; choose the canonical file with --canonical-from \
                 CLAUDE.md or --canonical-from AGENTS.md",
                RootState::Conflict.as_str(),
                classification.reason
            )];
        }
        RootState::None => return Vec::new(),
        _ => {}
    }

    let root_skills: Vec<&SkillFile> = snapshot
        .skills
        .iter()
        .filter(|skill| nesting::nearest_root(roots, parent_directory(&skill.path)) == Some("."))
        .collect();
    let (rows, skill_refusal) = skill_rows(&root_skills);
    if let Some(refusal) = skill_refusal {
        return vec![refusal];
    }

    let discovered: Vec<String> = root_skills.iter().map(|skill| skill.path.clone()).collect();
    let root_absolute = absolute_posix(workspace, ".");
    let canonical_text = canonical_bytes(identity_files, classification.source);
    let status = read_skill_index(canonical_text.unwrap_or(&[]), &discovered, &root_absolute);

    if let Some(text) = canonical_text {
        // Both families' own outcomes are always computed, never
        // short-circuited by the other: a hand-edited or malformed
        // block in *either* family must still refuse on its own,
        // exactly as the prototype's own `plan_block` raises its one
        // `EditedBlock` before ever reaching a write. A refusal from
        // either family outranks any pending operation -- the
        // prototype's own plan never plans past the first refusal it
        // raises -- and `legacy_marker_precedes_omnifrons_marker`
        // below resolves the rare case where both refuse at once, by
        // preferring whichever family's own begin marker reads first
        // in the file; that is the block problem a human reading top
        // to bottom would meet first. Only once *neither* family
        // refuses does the prototype's own "one block, one operation"
        // rule apply: a pending legacy migration, when a legacy
        // marker is present and intact, replaces this crate's own
        // append/regenerate outright, rather than sitting beside it.
        let needed = block_needed_names(&status);
        let (legacy_refusal, legacy_pending) =
            match legacy_block_outcome(ports.hasher, text, &needed) {
                LegacyBlockOutcome::Refusal {
                    message,
                    marker_offset,
                } => (Some((message, marker_offset)), None),
                LegacyBlockOutcome::Pending(migrate) => (None, Some(migrate)),
                LegacyBlockOutcome::None => (None, None),
            };
        let (omnifrons_refusal, omnifrons_pending) =
            match skills_block_outcome(ports.hasher, text, &status, &rows, &root_absolute) {
                BlockOutcome::Refusal(refusal) => (Some(refusal), None),
                BlockOutcome::Pending(pending) => (None, Some(pending)),
                BlockOutcome::None => (None, None),
            };

        if let (Some((legacy_message, legacy_offset)), Some(omnifrons_refusal)) =
            (&legacy_refusal, &omnifrons_refusal)
        {
            let winner = if legacy_marker_precedes_omnifrons_marker(text, *legacy_offset) {
                legacy_message
            } else {
                omnifrons_refusal
            };
            return vec![winner.clone()];
        }
        if let Some((legacy_message, _)) = legacy_refusal {
            return vec![legacy_message];
        }
        if let Some(refusal) = omnifrons_refusal {
            return vec![refusal];
        }
        if let Some(migrate) = legacy_pending {
            let mut problems = wrapper_problems(classification.state, ports.hasher, identity_files);
            problems.push(migrate);
            return problems;
        }
        if let Some(pending) = omnifrons_pending {
            let mut problems = wrapper_problems(classification.state, ports.hasher, identity_files);
            problems.push(pending);
            return problems;
        }
        return wrapper_problems(classification.state, ports.hasher, identity_files);
    }

    wrapper_problems(classification.state, ports.hasher, identity_files)
}

/// The pending `CLAUDE.md` wrapper write, when `state` is
/// [`RootState::AgentsOnly`] or [`RootState::NearWrapper`] (the
/// prototype's own `write_wrapper` operation, read back through
/// `describe_operation`): empty for every other state, since this pass
/// never plans the `AGENTS.md` copy or the opt-in `GEMINI.md` wrapper --
/// see [`verify`]'s own "Deferred to slice 2" section.
fn wrapper_problems(
    state: RootState,
    hasher: &dyn ContentHasher,
    identity_files: &[IdentityFile],
) -> Vec<String> {
    if !matches!(state, RootState::AgentsOnly | RootState::NearWrapper) {
        return Vec::new();
    }
    let claude = identity_files
        .iter()
        .find(|file| file.kind == IdentityKind::Claude && matches!(file.shape, FileShape::Regular));
    let effect = claude.map_or_else(
        || "creates the file".to_string(),
        |file| {
            format!(
                "replaces sha256 {}",
                &hasher.sha256(&file.bytes).to_hex()[..12]
            )
        },
    );
    vec![format!("pending: write the wrapper CLAUDE.md ({effect})")]
}

/// The stable refusal code the prototype's own `UnsafeSkillMetadata`
/// exception always carries (`UNSAFE_SKILL_METADATA` in the prototype's
/// `adapter/portable-agent/scripts/portable_agent.py`), for both of its
/// causes -- a lone surrogate, or a possible `@` import. Deliberately
/// not [`omnifrons_domain::skills_safety::SkillSafetyRefusal::as_str`]'s
/// kebab-case `"unsafe-skill-metadata"`: that is this crate's own house
/// convention for an IPC-facing reason token, while every string
/// [`verify`] produces mirrors the prototype's own wire format verbatim,
/// `snake_case` code included.
const UNSAFE_SKILL_METADATA_CODE: &str = "unsafe_skill_metadata";

/// Every root skill's row, and the one refusal this pass can raise from
/// them (the prototype's own `plan_block`): a lone surrogate from
/// [`read_skill_metadata`] refuses every flagged skill at once, before a
/// possible `@` import is ever checked for any of them; only when none
/// is found does [`unsafe_row_fields`] run over the rest. Either way all
/// of `root_skills` shares the one refusal, never one row at a time.
fn skill_rows(root_skills: &[&SkillFile]) -> (Vec<SkillRow>, Option<String>) {
    let mut invalid_problems: Vec<String> = Vec::new();
    let mut rows: Vec<SkillRow> = Vec::with_capacity(root_skills.len());
    for skill in root_skills {
        match read_skill_metadata(&skill.path, &skill.bytes) {
            Ok(metadata) => rows.push(metadata.into()),
            Err(SkillMetadataError::InvalidUnicode(fields)) => {
                invalid_problems.extend(invalid_unicode_problems(&skill.path, &fields));
            }
        }
    }
    if !invalid_problems.is_empty() {
        return (
            rows,
            Some(format!(
                "refused ({UNSAFE_SKILL_METADATA_CODE}): {}",
                invalid_unicode_detail(&invalid_problems)
            )),
        );
    }
    let unsafe_fields = unsafe_row_fields(&rows);
    let refusal = (!unsafe_fields.is_empty()).then(|| {
        format!(
            "refused ({UNSAFE_SKILL_METADATA_CODE}): {}",
            unsafe_rows_detail(&unsafe_fields)
        )
    });
    (rows, refusal)
}

/// One flagged skill's own invalid-Unicode problems, one per indexed
/// field holding a lone surrogate (the prototype's own
/// `invalid_unicode_fields`).
fn invalid_unicode_problems(path: &str, fields: &[InvalidUnicodeField]) -> Vec<String> {
    fields
        .iter()
        .map(|field| {
            let points = field
                .surrogates
                .iter()
                .map(|point| format!("U+{point:04X}"))
                .collect::<Vec<_>>()
                .join(", ");
            let shown = if field.surrogates.len() == 1 {
                format!("the lone surrogate {points}, an unpaired \\u escape,")
            } else {
                format!("the lone surrogates {points}, unpaired \\u escapes,")
            };
            format!(
                "{} (its {} holds invalid Unicode: {shown} cannot be written as UTF-8)",
                python_repr(path),
                field.field
            )
        })
        .collect()
}

/// The `unsafe_skill_metadata` detail for a lone surrogate (the
/// prototype's own `invalid_unicode_detail`).
fn invalid_unicode_detail(problems: &[String]) -> String {
    format!(
        "these local skills cannot be written into the skill index of AGENTS.md: {}. The \
         index quotes each skill's name and description in AGENTS.md, which is UTF-8 text, \
         and a lone surrogate has no UTF-8 form. No skill is ever edited for you: in each \
         SKILL.md named here, edit the `name:` or `description:` named so that no \\u escape \
         stands alone (write the character itself, or a full pair such as \\ud83d\\ude00), \
         then run the dry-run again",
        problems.join("; ")
    )
}

/// The `unsafe_skill_metadata` detail for a possible `@` import or a
/// link-breaking path character (the prototype's own
/// `unsafe_rows_detail`).
fn unsafe_rows_detail(problems: &[String]) -> String {
    format!(
        "these local skills cannot be written into the skill index of AGENTS.md: {}. The \
         index writes each skill's name and description as Markdown text outside code and \
         its path as a link, and Claude Code may load an @ that follows a space, a symbol (a \
         code span, emphasis, a link or a backslash escape included) or nothing as an import \
         of another file. The adapter never edits or renames a skill: edit the `name:` or \
         `description:` named here in that SKILL.md (write \"at\" or rephrase; an @ right \
         after a letter or digit, as in ops@example.com, is fine; a skill without `name:` is \
         listed by its folder name), or rename the folder of a path named here, then run the \
         dry-run again",
        problems.join("; ")
    )
}

/// What [`skills_block_outcome`] found for the canonical text's skills
/// block.
enum BlockOutcome {
    /// Nothing pending and nothing to refuse.
    None,
    /// The block is malformed or was edited by hand (the prototype's own
    /// `EditedBlock`).
    Refusal(String),
    /// A write or update is pending (the prototype's own
    /// `append_block`/`replace_block`).
    Pending(String),
}

/// The skills-block refusal or pending operation this pass can know
/// without planning a write (the prototype's own `plan_block`, called
/// through `build_plan`): a malformed or hand-edited block always
/// refuses; otherwise, no block yet and [`block_needed`] saying one is
/// due appends one, a located block [`block_needed`] says is not due at
/// all is left alone, and an already-located, still-due block whose
/// body no longer matches a fresh render of it regenerates one. For
/// that regenerate comparison, both the covered-skills lookup and the
/// precedence-rule need are read only from the text *before* the
/// located block (`prefix_text`), exactly as the prototype's own
/// `plan_block` reads `covered` and `rule` from its own `prefix` --
/// never from `status`, which was computed over the *whole* canonical
/// text and so would already count the block's own rows and rule
/// section as already stated, making a regenerate decision that can
/// never go stable.
fn skills_block_outcome(
    hasher: &dyn ContentHasher,
    canonical_text: &[u8],
    status: &SkillIndexStatus,
    rows: &[SkillRow],
    root_absolute: &str,
) -> BlockOutcome {
    let decoded = String::from_utf8_lossy(canonical_text);
    let digest = |bytes: &[u8]| hasher.sha256(bytes);
    let located = match ManagedBlock::locate(ManagedFileKind::Skills, &decoded, &digest) {
        Err(ManagedFileError::Malformed) => {
            return BlockOutcome::Refusal(format!(
                "refused (block_layout): the skill index block cannot be regenerated: its \
                 sentinels are not one intact pair. Fix the markers by hand or index {} by \
                 hand",
                block_needed_names(status)
            ));
        }
        Err(ManagedFileError::Modified) => {
            return BlockOutcome::Refusal(format!(
                "refused (edited_block): the skill index block was edited by hand (its \
                 recorded sha256 no longer matches) and cannot be regenerated; restore it or \
                 index {} by hand",
                block_needed_names(status)
            ));
        }
        Ok(located) => located,
    };

    let Some(located) = located else {
        return if block_needed(status) {
            let fresh_rows = rows_to_generate(canonical_text, rows, root_absolute);
            BlockOutcome::Pending(skills_block_pending_text(
                "append",
                &fresh_rows,
                !status.precedence,
            ))
        } else {
            BlockOutcome::None
        };
    };

    // The prototype's own `plan_block` checks this same gate --
    // `if not status.missing and status.precedence: return None` --
    // before it ever looks for a block at all, over `status`, which is
    // read from the *whole* canonical text and so already counts the
    // block's own rows and rule section as coverage. Mirrored here for
    // a block that *is* located: when the whole text already covers
    // every discovered skill and already states the rule somewhere,
    // the prototype leaves the block alone outright, even one whose
    // body a fresh, prefix-only render would no longer match -- it
    // never reaches a `regenerate` operation, and never describes one
    // with an empty parenthetical.
    if !block_needed(status) {
        return BlockOutcome::None;
    }

    // The prototype's own `plan_block` reads `rule` from the text
    // *before* the block (`rule = not has_precedence_phrases(prefix)`),
    // never from `status.precedence`, which spans the *whole* canonical
    // text and so is already satisfied by the very rule section this
    // block carries. Reusing `status.precedence` here would make a
    // block this crate generated with the rule count as already
    // stating it outside the block too: the fresh render would then
    // always omit the rule the existing body carries, never match it,
    // and `verify` would report `pending: regenerate ...` forever for
    // an otherwise canonical agent.
    let prefix_text = &decoded[..located.span.start];
    let rule_needed = !has_precedence_phrases(prefix_text);
    let fresh_rows = rows_to_generate(prefix_text.as_bytes(), rows, root_absolute);
    let fresh_body = render_body(&fresh_rows, rule_needed);
    if located.body == fresh_body {
        BlockOutcome::None
    } else {
        BlockOutcome::Pending(skills_block_pending_text(
            "regenerate",
            &fresh_rows,
            rule_needed,
        ))
    }
}

/// The skills `build_plan` would name as still needing an index entry,
/// joined as the prototype's own `", ".join(status.missing) or "the
/// precedence rule"` -- used only inside a block refusal's detail text.
fn block_needed_names(status: &SkillIndexStatus) -> String {
    if status.missing.is_empty() {
        "the precedence rule".to_string()
    } else {
        status.missing.join(", ")
    }
}

/// One skills-block pending operation's description (the prototype's own
/// `describe_operation`, `append_block`/`replace_block` branch): always
/// against `AGENTS.md`, the one file [`classify`] ever names as the
/// canonical source for a state this pass reaches this check from.
fn skills_block_pending_text(verb: &str, rows: &[SkillRow], rule_needed: bool) -> String {
    let mut parts = Vec::new();
    if !rows.is_empty() {
        parts.push(format!("{} skill rows", rows.len()));
    }
    if rule_needed {
        parts.push("the precedence rule".to_string());
    }
    format!(
        "pending: {verb} the skill index block in AGENTS.md ({})",
        parts.join(" and ")
    )
}

/// The prototype's own legacy skills-block begin marker's fixed prefix
/// (`BLOCK_BEGIN_PATTERN`'s own literal text up to the hex sha256
/// digest), never emitted by this crate's own [`ManagedBlock`] (which
/// begins `<!-- omnifrons:begin skills ` instead): present only in a
/// file the prototype itself last wrote, or one hand-migrated partway.
const LEGACY_BEGIN_PREFIX: &str = "<!-- portable-agent:skills:begin sha256=";

/// The fixed suffix closing the same marker, after the digest.
const LEGACY_BEGIN_SUFFIX: &str = " -->";

/// The prototype's own legacy skills-block end marker (`BLOCK_END`),
/// matched as a full line, never a substring.
const LEGACY_END_LINE: &str = "<!-- portable-agent:skills:end -->";

/// `line`'s own recorded digest when it is the prototype's own legacy
/// begin marker, matched in full exactly as its own
/// `BLOCK_BEGIN_PATTERN.fullmatch` would: [`LEGACY_BEGIN_PREFIX`], then
/// exactly 64 lowercase hex characters (returned), then
/// [`LEGACY_BEGIN_SUFFIX`], and nothing else on the line. Matched
/// bytewise against `line`'s own original bytes, never a decoded
/// stand-in for them: the prefix, the suffix, and every valid digest
/// character are themselves ASCII, so a byte-for-byte match is exact,
/// and a line holding non-UTF-8 bytes simply fails to match -- it can
/// never be mistaken for a marker.
fn legacy_begin_digest(line: &[u8]) -> Option<&str> {
    let hex = line
        .strip_prefix(LEGACY_BEGIN_PREFIX.as_bytes())?
        .strip_suffix(LEGACY_BEGIN_SUFFIX.as_bytes())?;
    let is_hex_digest = hex.len() == 64
        && hex
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'));
    if is_hex_digest {
        // Every byte just checked is ASCII, so this can never fail.
        std::str::from_utf8(hex).ok()
    } else {
        None
    }
}

/// A minimal, line-at-a-time fence open/close toggle for
/// [`legacy_lines`]'s own scan: a trimmed line starting with a
/// run of three or more backticks or tildes opens a fence; once open,
/// only a compatible closer -- the same character, a run at least as
/// long as the opener's, with nothing but whitespace after it -- closes
/// it again. The same `CommonMark`-precise rule
/// `omnifrons_domain::fences::FenceTracker` already applies elsewhere
/// in this workspace, reimplemented here in miniature because that
/// tracker is `pub(crate)` to the domain crate, not this one.
#[derive(Default)]
struct LegacyMarkerFence {
    opener: Option<(char, usize)>,
    /// The index of the line that opened [`Self::opener`]'s own fence,
    /// cleared the moment that fence closes again: the prototype's own
    /// `quoted_markers` scans only lines before `fences[-1]` once a
    /// trailing fence is still unclosed at end of file, and this is
    /// that same cutoff line -- read back once the whole file has been
    /// fed through [`Self::advance`], by [`legacy_lines`].
    opened_at: Option<usize>,
}

impl LegacyMarkerFence {
    /// Feeds one already-trimmed line at `index` and reports whether it
    /// sits inside a fence: a fence-shaped line is always reported as
    /// fenced, whether it opens, closes, or -- unable to close what is
    /// already open -- is itself fenced content; a plain line is
    /// fenced exactly when a fence is still open around it.
    fn advance(&mut self, index: usize, trimmed_line: &str) -> bool {
        let Some(marker) = trimmed_line
            .chars()
            .next()
            .filter(|c| *c == '`' || *c == '~')
        else {
            return self.opener.is_some();
        };
        let run = trimmed_line.chars().take_while(|c| *c == marker).count();
        if run < 3 {
            return self.opener.is_some();
        }
        let has_info_string = trimmed_line[run..].chars().any(|c| !c.is_whitespace());
        match self.opener {
            None => {
                self.opener = Some((marker, run));
                self.opened_at = Some(index);
            }
            Some((open_marker, open_run))
                if marker == open_marker && run >= open_run && !has_info_string =>
            {
                self.opener = None;
                self.opened_at = None;
            }
            Some(_) => {}
        }
        true
    }
}

/// What locating the prototype's own legacy skills-block sentinel
/// found (mirrors [`BlockOutcome`] for the `portable-agent:skills`
/// family), outside the prototype's own vocabulary, since the
/// prototype never needs to migrate itself (ADR-0005 sub-slice 1e-D).
/// [`verify_problems_after_state`] gives a [`Self::Refusal`] here the
/// same standing as a [`BlockOutcome::Refusal`]: either one, on its
/// own, outranks any pending operation, and the two are resolved
/// against each other by file order on the rare occasion both refuse
/// at once. Only once *neither* family refuses does this crate's own
/// "one block, one operation" rule apply: a [`Self::Pending`]
/// migration *replaces* whatever `skills_block_outcome` would
/// otherwise report for this crate's own sentinel, never sitting
/// beside it, since the prototype's own `plan_block` only ever plans
/// one skills-block operation once `find_block` has located a block
/// of any format, and a real legacy block here means exactly that --
/// a block already exists, just not in this crate's own sentinel
/// format.
enum LegacyBlockOutcome {
    /// No legacy begin marker line anywhere in the text.
    None,
    /// An intact block (the prototype's own `IndexBlock.intact`) whose
    /// recorded digest matches its body: pending migration to this
    /// crate's own sentinel format.
    Pending(String),
    /// A malformed or hand-edited block (the prototype's own
    /// `EditedBlock`), already carrying its fixed code token and the
    /// prototype's own detail wording, alongside `marker_offset`: the
    /// byte offset of this refusing block's own first begin marker
    /// line, exactly as this scan located it (`lines[first].offset`,
    /// never a fresh, laxer prefix search over the raw text).
    /// [`verify_problems_after_state`]'s own file-order tie-break reads
    /// this straight from here instead of recomputing it, so a line
    /// that only looks like a begin marker -- a fenced documentation
    /// example with a placeholder instead of a real digest, say --
    /// never stands in for the block that is actually refusing.
    Refusal {
        message: String,
        marker_offset: usize,
    },
}

/// One line of canonical text, classified for the legacy scan
/// ([`legacy_lines`]) over the ORIGINAL bytes -- never a decoded
/// stand-in for them, so a body a caller slices out by [`Self::offset`]
/// is always the exact bytes the prototype's own `sha256_hex(body)`
/// would hash, non-UTF-8 or not.
struct LegacyLine<'a> {
    /// This line's own start offset within the canonical bytes
    /// (including its own terminator) -- the prototype's own
    /// `offsets[index]`, built the same way alongside
    /// `data.splitlines(keepends=True)`.
    offset: usize,
    /// This line's bytes with its own trailing `"\r\n"`, `"\n"`, or
    /// lone `"\r"` terminator removed, matched unindented and
    /// untrimmed against the exact marker bytes, exactly as the
    /// prototype's own
    /// `BLOCK_BEGIN_PATTERN.fullmatch`/`== BLOCK_END` would -- never
    /// trimmed the way a fence line is below: a marker with incidental
    /// leading or trailing whitespace is not this exact sentinel.
    text: &'a [u8],
    /// Whether [`Self::text`] is a begin marker line.
    is_begin: bool,
    /// Whether [`Self::text`] is the end marker line.
    is_end: bool,
    /// [`Self::is_begin`]'s own recorded digest.
    begin_digest: Option<&'a str>,
    /// Whether a fenced code block was open around this line, fence
    /// lines themselves included (the prototype's own `fence_flags`,
    /// read through [`LegacyMarkerFence`]). Decided from a lossily
    /// decoded view of this one line alone -- safe because a fence
    /// delimiter is itself ASCII, so a non-UTF-8 byte elsewhere on the
    /// line can never turn a fence line into a plain one or back; this
    /// flag never feeds the digest.
    fenced: bool,
}

/// `text` split into lines that each keep their own terminator and know
/// their own start offset -- the prototype's own
/// `data.splitlines(keepends=True)` paired with its own `offsets` list.
/// For `bytes` (never `str`), that prototype method splits only on
/// `"\n"`, a lone `"\r"`, or `"\r\n"` (one terminator, not two):
/// confirmed with a real `python3`, where
/// `b"a\rb\r\nc\x0bd\n".splitlines(keepends=True)` answers
/// `[b"a\r", b"b\r\n", b"c\x0bd\n"]` -- `"\x0b"` and the rest of
/// `str.splitlines`'s wider Unicode line-break set stay plain bytes
/// here, never boundaries. No trailing empty line is produced for text
/// ending in a terminator.
fn lines_with_terminators(text: &[u8]) -> Vec<(usize, &[u8])> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < text.len() {
        match text[index] {
            b'\n' => {
                lines.push((start, &text[start..=index]));
                index += 1;
                start = index;
            }
            b'\r' => {
                let end = if text.get(index + 1) == Some(&b'\n') {
                    index + 1
                } else {
                    index
                };
                lines.push((start, &text[start..=end]));
                index = end + 1;
                start = index;
            }
            _ => index += 1,
        }
    }
    if start < text.len() {
        lines.push((start, &text[start..]));
    }
    lines
}

/// `raw`'s own bytes with one trailing line terminator removed:
/// trailing `"\n"` first, then trailing `"\r"` -- the same two-step
/// rule `omnifrons_domain::guidance`'s own `strip_terminator` applies,
/// so a line [`omnifrons_lines`] hands to
/// [`first_omnifrons_marker_offset`] is trimmed exactly as
/// `ManagedBlock::locate` itself trims the same line before comparing
/// it, even though the two sides split their lines differently.
fn strip_line_ending(raw: &[u8]) -> &[u8] {
    let trimmed = raw.strip_suffix(b"\n").unwrap_or(raw);
    trimmed.strip_suffix(b"\r").unwrap_or(trimmed)
}

/// This crate's own skills-block begin sentinel's fixed prefix, up to
/// and including the space before the version -- the same literal
/// `omnifrons_domain::guidance::ManagedFileKind::Skills`'s own private
/// `begin_prefix` returns, duplicated here because that method is not
/// visible outside its own crate. Paired with
/// [`OMNIFRONS_SKILLS_BEGIN_SUFFIX`] by
/// [`is_omnifrons_begin_sentinel_line`] to match a begin sentinel line
/// in full, and again, on its own terms, by
/// [`first_omnifrons_marker_offset`]'s own fallback shape: a line that
/// opens and closes the envelope -- starts with this prefix, ends with
/// [`OMNIFRONS_SKILLS_BEGIN_SUFFIX`] -- without the strict match above
/// accepting it, such as a digest hand-edited past 64 hex characters.
/// A line that only starts like this prefix and then continues as
/// ordinary prose past its own closing `-->` never closes that
/// envelope, so it is never mistaken for this family's own line
/// either way. Never emitted by the legacy family -- see
/// [`LEGACY_BEGIN_PREFIX`].
const OMNIFRONS_SKILLS_BEGIN_PREFIX: &str = "<!-- omnifrons:begin skills ";

/// This crate's own skills-block begin sentinel's fixed suffix, after
/// the hex digest -- the same literal
/// `omnifrons_domain::guidance::ManagedFileKind::Skills`'s own private
/// `begin_suffix` returns. Paired with
/// [`OMNIFRONS_SKILLS_BEGIN_PREFIX`] by
/// [`is_omnifrons_begin_sentinel_line`] below; never used alone.
const OMNIFRONS_SKILLS_BEGIN_SUFFIX: &str = " -->";

/// This crate's own skills-block end sentinel, matched as a full line --
/// the same literal `omnifrons_domain::guidance::ManagedFileKind::Skills`'s
/// own private `end_sentinel` returns, duplicated here for the same
/// reason as [`OMNIFRONS_SKILLS_BEGIN_PREFIX`]: that method is not
/// visible outside its own crate. Used by
/// [`first_omnifrons_marker_offset`] alongside the begin shapes above,
/// never behind them: a lone end sentinel -- a malformed pair with no
/// begin line at all, the shape [`ManagedBlock::locate`] itself raises
/// [`ManagedFileError::Malformed`] over -- still gives this side a real
/// position to be scanned at, rather than reading as absent, and a
/// stray extra end line above an otherwise valid pair must still win
/// the earliest-line comparison over that pair's own, later begin
/// line.
const OMNIFRONS_SKILLS_END_LINE: &str = "<!-- omnifrons:end skills -->";

/// Whether `line` is a complete omnifrons skills-block begin sentinel:
/// [`OMNIFRONS_SKILLS_BEGIN_PREFIX`], then a non-empty version holding
/// no whitespace, then the literal `" sha256:"`, then exactly 64
/// lowercase hex characters, then [`OMNIFRONS_SKILLS_BEGIN_SUFFIX`] --
/// and nothing else on the line, the same shape
/// `ManagedBlock::begin_sentinel` itself renders. `line` is expected
/// already stripped of its own line terminator, exactly as
/// [`strip_line_ending`] leaves it.
///
/// Matched bytewise against the original bytes, mirroring
/// [`legacy_begin_digest`]'s own full-line rule: every fixed byte here
/// is ASCII, so a line holding non-UTF-8 bytes simply fails to match,
/// never a false positive.
///
/// Returning `false` here does not mean the line plays no part in
/// this family's own anchor. A line that still opens and closes the
/// envelope -- ends with [`OMNIFRONS_SKILLS_BEGIN_SUFFIX`] -- but
/// carries a placeholder version, a non-hex digest, or one of the
/// wrong length fails this strict check, yet
/// [`first_omnifrons_marker_offset`]'s own separate envelope shape
/// still admits it, deliberately:
/// `omnifrons_domain::guidance::ManagedBlock::locate` itself counts
/// ANY line starting with the kind's begin prefix toward its own
/// begin-marker tally, never checking that the rest of the line
/// parses, so a line shaped like that is exactly what makes the real
/// block malformed over there. Anchoring on it, rather than skipping
/// past it to a later line, anchors on the family's own actual
/// offending line. Only a line that continues past its own closing
/// `-->` with ordinary prose -- never closing the envelope at all --
/// plays no part in any of the three shapes
/// [`first_omnifrons_marker_offset`] checks, even though a plain
/// prefix search would still match its opening bytes; see that
/// function's own doc comment, the only caller.
fn is_omnifrons_begin_sentinel_line(line: &[u8]) -> bool {
    let Some(after_prefix) = line.strip_prefix(OMNIFRONS_SKILLS_BEGIN_PREFIX.as_bytes()) else {
        return false;
    };
    let Some(rest) = after_prefix.strip_suffix(OMNIFRONS_SKILLS_BEGIN_SUFFIX.as_bytes()) else {
        return false;
    };
    let Some(hex_start) = rest.len().checked_sub(64) else {
        return false;
    };
    let (head, hex) = rest.split_at(hex_start);
    let Some(version) = head.strip_suffix(b" sha256:") else {
        return false;
    };
    let is_hex_digest = hex
        .iter()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'));
    !version.is_empty() && !version.iter().any(u8::is_ascii_whitespace) && is_hex_digest
}

/// `text` split into lines that each keep their own terminator and
/// know their own start offset, scanned over the ORIGINAL bytes --
/// never a decoded stand-in for them -- exactly as
/// `omnifrons_domain::guidance::ManagedBlock::locate` itself splits
/// its own input: `str::split_inclusive('\n')`, `"\n"` only. Never
/// [`lines_with_terminators`]'s own wider rule, which also splits on
/// a lone `"\r"` to match the legacy family's own prototype
/// `splitlines` behavior: the two rules agree on a `"\r\n"` pair
/// (still one boundary, at the trailing `'\n'` byte) and on plain
/// `"\n"`, and disagree only on a lone `"\r"` with no `'\n'` after
/// it, which starts a new line for [`lines_with_terminators`] but
/// never for `locate`, or here.
///
/// No trailing empty line is produced for text ending in `"\n"`,
/// matching `split_inclusive`'s own behavior.
///
/// [`lines_with_terminators`] and this function cannot be merged into
/// one shared split: [`legacy_lines`] must keep matching the legacy
/// prototype's own `splitlines(keepends=True)` rule exactly, and
/// [`first_omnifrons_marker_offset`] must keep matching `locate`'s own
/// `split_inclusive('\n')` rule exactly, and those two upstream rules
/// themselves disagree on a lone `"\r"`.
fn omnifrons_lines(text: &[u8]) -> Vec<(usize, &[u8])> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    for (index, &byte) in text.iter().enumerate() {
        if byte == b'\n' {
            lines.push((start, &text[start..=index]));
            start = index + 1;
        }
    }
    if start < text.len() {
        lines.push((start, &text[start..]));
    }
    lines
}

/// The byte offset of this crate's own omnifrons skills-block marker's
/// own earliest position in `canonical_text`, scanned line by line
/// exactly as `omnifrons_domain::guidance::ManagedBlock::locate`
/// itself would -- through [`omnifrons_lines`], never
/// [`legacy_lines`]'s own wider [`lines_with_terminators`] split --
/// and always over the ORIGINAL bytes, never a decoded stand-in for
/// them, for [`legacy_marker_precedes_omnifrons_marker`]'s own
/// file-order tie-break.
///
/// Following `locate`'s own line boundaries here, rather than
/// [`lines_with_terminators`]'s wider ones, is what keeps this
/// family's own anchor honest: a line `locate` would never break at
/// -- text after a lone `"\r"` that happens to equal the end
/// sentinel, say -- must never anchor this family's own tie-break
/// position either, since a line like that plays no part in the real
/// omnifrons refusal `locate` itself would raise. See
/// [`omnifrons_lines`]'s own doc comment for why the two splits
/// cannot be shared.
///
/// A line counts as this family's own position in any of three shapes:
/// a complete begin sentinel line, matched in full by
/// [`is_omnifrons_begin_sentinel_line`] -- the primary, documented
/// shape `ManagedBlock::begin_sentinel` itself renders; the complete
/// end sentinel line ([`OMNIFRONS_SKILLS_END_LINE`], matched the same
/// way, as a whole line); or a line that opens and closes the same
/// envelope -- starts with [`OMNIFRONS_SKILLS_BEGIN_PREFIX`] and ends
/// with [`OMNIFRONS_SKILLS_BEGIN_SUFFIX`] -- without the strict match
/// above accepting it, such as a digest hand-edited past 64 hex
/// characters or swapped for non-hex bytes, but still recognizably
/// this family's own. This third shape exists because
/// `omnifrons_domain::guidance::ManagedBlock::locate` is exactly this
/// permissive on its own side: it counts ANY line starting with the
/// kind's begin prefix toward its own begin-marker tally, never
/// checking that the rest of the line parses, so a malformed-interior
/// line like this is itself what that function finds and refuses
/// over; this shape anchors on that same line instead of skipping
/// past it to a later one. A prose or fenced documentation example
/// that only *starts* like a sentinel and then continues past its own
/// closing `-->` with ordinary prose never closes that envelope, so
/// none of the three shapes ever mistakes it for a real marker,
/// exactly as [`is_omnifrons_begin_sentinel_line`]'s own doc comment
/// already explains for the strict match alone; a fenced sentinel
/// that is otherwise complete still counts, exactly as
/// [`legacy_lines`] still counts a fenced legacy marker.
///
/// Of every line matching any of the three shapes, the EARLIEST one in
/// the file wins, never a fixed priority among the shapes themselves: a
/// single malformed or refusing pair can carry more than one such line
/// at once -- a begin line whose hand-edited digest fails the strict
/// match, sitting above its own intact end sentinel, or a stray extra
/// end sentinel sitting above an otherwise valid pair's own begin line
/// -- and this family's own position in the tie-break must always be
/// the first of them a reader scanning top to bottom would meet, never
/// whichever shape a fixed check order happened to try first.
///
/// `None` only when no line matches any of the three shapes anywhere in
/// the file: the refusing pair's own markers failed to parse some other
/// way, so there is nothing here to anchor on; see
/// [`legacy_marker_precedes_omnifrons_marker`]'s own doc comment.
fn first_omnifrons_marker_offset(canonical_text: &[u8]) -> Option<usize> {
    omnifrons_lines(canonical_text)
        .into_iter()
        .find(|&(_, raw)| {
            let line = strip_line_ending(raw);
            is_omnifrons_begin_sentinel_line(line)
                || line == OMNIFRONS_SKILLS_END_LINE.as_bytes()
                || (line.starts_with(OMNIFRONS_SKILLS_BEGIN_PREFIX.as_bytes())
                    && line.ends_with(OMNIFRONS_SKILLS_BEGIN_SUFFIX.as_bytes()))
        })
        .map(|(offset, _)| offset)
}

/// Whether the legacy family's own refusing block sits earlier in
/// `canonical_text` than this crate's own omnifrons skills-block begin
/// marker -- [`verify_problems_after_state`]'s own file-order
/// tie-break, applied only when both families refuse at once.
///
/// `legacy_marker_offset` is `legacy_block_outcome`'s own
/// [`LegacyBlockOutcome::Refusal::marker_offset`], never a fresh
/// search over `canonical_text` by [`LEGACY_BEGIN_PREFIX`] alone: a
/// fresh prefix-only search would also match a line that merely starts
/// with that prefix without being a complete marker -- a fenced
/// documentation example with a placeholder instead of a real digest,
/// say -- which [`legacy_begin_digest`]'s own strict full-line match
/// never counts as a begin marker at all. Using the scan's own located
/// offset instead means a line like that can never stand in for the
/// block that is actually refusing.
///
/// The omnifrons side carries no equivalent located offset to thread --
/// [`ManagedBlock::locate`] reports no span on the
/// [`ManagedFileError`] that produces an omnifrons refusal -- so
/// [`first_omnifrons_marker_offset`] is searched instead: it scans
/// every line for any of the three shapes a real omnifrons marker can
/// take and keeps the earliest match, so a fake example -- a fenced or
/// prose placeholder that only starts like a sentinel -- never stands
/// in for a real marker, the same guarantee the legacy side has, even
/// without a located span to thread. Taking the earliest match across
/// all three shapes, rather than a fixed priority among them, is what
/// lets a malformed begin line above an intact end, or a stray extra
/// end above an otherwise valid pair, still anchor on whichever of the
/// two reads first. When no line matches any of the three shapes at
/// all, the omnifrons marker is treated as reading after the legacy
/// one, since there is no position to prefer it by.
fn legacy_marker_precedes_omnifrons_marker(
    canonical_text: &[u8],
    legacy_marker_offset: usize,
) -> bool {
    match first_omnifrons_marker_offset(canonical_text) {
        Some(omnifrons_offset) => legacy_marker_offset <= omnifrons_offset,
        None => true,
    }
}

/// Classifies every line of `canonical_text` for the legacy scan
/// ([`LegacyLine`]), tracking fence state across them with one
/// [`LegacyMarkerFence`], and returns alongside it that fence's own
/// cutoff for a "quoted" marker explanation: the prototype's own
/// `quoted_markers` stops looking once a trailing fence is still open
/// at end of file (`fences[-1]`, for an odd fence-line count), since
/// that fence then runs to the end of the file and the adapter's own
/// block may follow it there. Mirrored here as
/// [`LegacyMarkerFence::opened_at`], read back once every line has been
/// fed through, defaulting to the line count (no cutoff) once every
/// fence has closed again. Operates on the ORIGINAL bytes throughout --
/// nothing here ever runs a whole-buffer lossy decode, so marker
/// matching and the body a caller later slices out by offset both stay
/// exact for a body holding non-UTF-8 bytes; only the fence tracker
/// (comment on [`LegacyLine::fenced`]) sees a decoded view, and only of
/// one line at a time.
fn legacy_lines(canonical_text: &[u8]) -> (Vec<LegacyLine<'_>>, usize) {
    let mut fence = LegacyMarkerFence::default();
    let lines: Vec<LegacyLine<'_>> = lines_with_terminators(canonical_text)
        .into_iter()
        .enumerate()
        .map(|(index, (offset, raw))| {
            let text = strip_line_ending(raw);
            let decoded_text = String::from_utf8_lossy(text);
            let fenced = fence.advance(index, decoded_text.trim());
            let begin_digest = legacy_begin_digest(text);
            LegacyLine {
                offset,
                text,
                is_begin: begin_digest.is_some(),
                is_end: text == LEGACY_END_LINE.as_bytes(),
                begin_digest,
                fenced,
            }
        })
        .collect();
    let quoted_limit = fence.opened_at.unwrap_or(lines.len());
    (lines, quoted_limit)
}

/// Locates the prototype's own legacy skills-block sentinel
/// (`LEGACY_BEGIN_PREFIX`/[`LEGACY_END_LINE`]) by the same full-line
/// rule its own `find_block` applies: every begin or end marker line
/// counts wherever it sits in the file, fenced or not -- fencing only
/// ever *explains* a problem this scan already found some other way
/// (an unpaired begin, a second begin, or text after the end), the
/// same role the prototype's own `quoted_markers` plays for
/// `find_block`; it never hides a marker, unlike the fence-skipping
/// scan this replaces.
///
/// A block is intact exactly as the prototype's own
/// `IndexBlock.intact` defines it: exactly one begin marker, at least
/// one end marker after it, nothing but whitespace after the first
/// such end marker, and -- checked only here, never by the
/// prototype's own `scan`, only by its `plan_block` -- the recorded
/// sha256 equal to `hasher.sha256` of the exact body bytes between the
/// two markers. The digest is never checked when a second begin or a
/// missing end already makes the body ambiguous, exactly as the
/// prototype's own `body, edited = data[offsets[first + 1]:], False`
/// branch skips it.
///
/// An intact block with a matching digest reports the migration
/// pending; anything else raises the same refusal the prototype's own
/// `plan_block` raises for a hand-edited block -- `block_layout` when
/// a layout problem exists (the dynamic cause list, with a
/// fenced-marker explanation prepended when one applies), the digest
/// mismatch alone else -- naming `needed` exactly as
/// [`block_needed_names`] already does for this crate's own sentinel.
///
/// A trailing, never-closed fence runs to end of file exactly as the
/// prototype's own `quoted_markers` treats it: a marker sitting after
/// that fence's own opening line never explains a problem as quoted --
/// read back from [`legacy_lines`]'s own cutoff, itself
/// [`LegacyMarkerFence::opened_at`] -- even though, like every marker
/// here, it is still found as a real one.
fn legacy_block_outcome(
    hasher: &dyn ContentHasher,
    canonical_text: &[u8],
    needed: &str,
) -> LegacyBlockOutcome {
    let (lines, quoted_limit) = legacy_lines(canonical_text);

    let begins: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.is_begin)
        .map(|(index, _)| index)
        .collect();
    let Some(&first) = begins.first() else {
        return LegacyBlockOutcome::None;
    };
    // Threaded into every `Refusal` below instead of a fresh prefix
    // search: see `LegacyBlockOutcome::Refusal`'s own doc comment for
    // why a fresh search over the raw text is not equivalent.
    let marker_offset = lines[first].offset;
    let ends: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(index, line)| *index > first && line.is_end)
        .map(|(index, _)| index)
        .collect();

    let mut layout: Vec<String> = Vec::new();
    if begins.len() > 1 {
        layout.push(format!(
            "line {} holds a second begin marker",
            begins[1] + 1
        ));
    }
    if ends.is_empty() {
        layout.push(format!(
            "the begin marker on line {} has no end marker after it",
            first + 1
        ));
    } else if lines[(ends[0] + 1)..]
        .iter()
        .any(|line| !line.text.iter().all(u8::is_ascii_whitespace))
    {
        layout.push(format!(
            "text follows the end marker on line {}; the block must be the last thing in \
             the file",
            ends[0] + 1
        ));
    }

    let edited = if begins.len() > 1 || ends.is_empty() {
        false
    } else {
        // The exact original bytes between the two markers -- the
        // prototype's own `data[offsets[first + 1] : offsets[ends[0]]]`
        // -- never a decoded-and-reencoded restatement of them, so a
        // body holding non-UTF-8 bytes still hashes to the digest the
        // prototype itself would record.
        let body = &canonical_text[lines[first + 1].offset..lines[ends[0]].offset];
        let recorded = lines[first].begin_digest.unwrap_or_default();
        hasher.sha256(body).to_hex() != recorded
    };

    if let Some(quoted) = lines
        .iter()
        .enumerate()
        .find(|(index, line)| {
            *index < quoted_limit && line.fenced && (line.is_begin || line.is_end)
        })
        .map(|(index, _)| index)
        && (!layout.is_empty() || edited)
    {
        layout.insert(
            0,
            format!(
                "the marker on line {} is inside a fenced code block; the adapter matches \
                 its markers anywhere, so a quoted marker reads as a broken block",
                quoted + 1
            ),
        );
    }

    if layout.is_empty() {
        if edited {
            return LegacyBlockOutcome::Refusal {
                message: format!(
                    "refused (edited_block): the skill index block was edited by hand (its \
                     recorded sha256 no longer matches) and cannot be regenerated; restore \
                     it or index {needed} by hand"
                ),
                marker_offset,
            };
        }
        return LegacyBlockOutcome::Pending(
            "pending: migrate the portable-agent skill index block in AGENTS.md to the \
             current omnifrons sentinel format"
                .to_string(),
        );
    }
    if edited {
        layout.push("its recorded sha256 no longer matches".to_string());
    }
    LegacyBlockOutcome::Refusal {
        message: format!(
            "refused (block_layout): the skill index block cannot be regenerated: {}. Fix \
             the markers by hand or index {needed} by hand",
            layout.join("; ")
        ),
        marker_offset,
    }
}

#[cfg(test)]
mod tests {
    use omnifrons_domain::executable::Sha256Digest;
    use omnifrons_domain::guidance::LineEnding;

    use super::{
        ManagedBlock, OMNIFRONS_SKILLS_BEGIN_PREFIX, OMNIFRONS_SKILLS_BEGIN_SUFFIX, render_body,
    };

    /// [`OMNIFRONS_SKILLS_BEGIN_PREFIX`]'s and
    /// [`OMNIFRONS_SKILLS_BEGIN_SUFFIX`]'s own doc comments explain why
    /// they duplicate `ManagedFileKind::Skills`'s private `begin_prefix`
    /// and `begin_suffix`: those methods are not visible outside the
    /// domain crate. This pins the duplication against the domain's own
    /// renderer, so a sentinel change on that side fails loudly here
    /// instead of silently disabling
    /// [`legacy_marker_precedes_omnifrons_marker`]'s own file-order
    /// tie-break, which reads both constants -- through
    /// [`is_omnifrons_begin_sentinel_line`] -- to find where the
    /// omnifrons family's own block begins.
    #[test]
    fn omnifrons_skills_begin_prefix_matches_the_domains_own_rendered_sentinel() {
        let digest = |_: &[u8]| Sha256Digest([0; 32]);
        let body = render_body(&[], true);
        let block = ManagedBlock::skills(body, &digest).expect("a non-empty body builds a block");
        let rendered = block.render(LineEnding::Lf);
        let first_line = rendered
            .lines()
            .next()
            .expect("a rendered block always has a begin sentinel line");

        assert!(
            first_line.starts_with(OMNIFRONS_SKILLS_BEGIN_PREFIX),
            "the domain's own rendered skills sentinel no longer starts with this crate's \
             own duplicated prefix; got {first_line:?}"
        );
        assert!(
            first_line.ends_with(OMNIFRONS_SKILLS_BEGIN_SUFFIX),
            "the domain's own rendered skills sentinel no longer ends with this crate's own \
             duplicated suffix; got {first_line:?}"
        );
    }
}
