//! The read-only `scan` service (ADR-0005 "Agent identity and portable
//! definition" sub-slice 1e-A): composes the domain's pure
//! classification, nesting, skills-index, and warnings modules over one
//! [`crate::definition_tree::DefinitionTree`] snapshot into the
//! prototype's own `scan()` report, without ever touching a filesystem
//! itself.
//!
//! Translated from the prototype's `scan`, `inspect_agent`,
//! `canonical_bytes`, and `ancestor_chain`: this module's own [`scan`]
//! walks nothing -- `ports.tree` already decided what exists -- and
//! instead classifies each discovered directory, attributes skills to
//! their nearest owning root, and asks the domain's `warnings` module for
//! each root's warnings, in that order. `verify` (sub-slice 1e-B) is a
//! separate, later service over the same ports; nothing in this module
//! anticipates it.
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
use omnifrons_domain::hosts;
use omnifrons_domain::nesting::{self, Ancestor, ChainLink, NestingError, Parent, RootRelation};
use omnifrons_domain::skills_index::read_skill_index;
use omnifrons_domain::warnings::{AgentInput, ChainAncestor, Warning, agent_warnings};

use crate::content_hasher::ContentHasher;
use crate::definition_tree::{DefinitionTree, DefinitionTreeError, DirectoryEntry, SkillFile};
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
/// reads them straight from that root's own [`DirectoryEntry`]; an
/// outside ancestor's are recognized from [`Ancestor::identity_files`]
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
            ChainLink::Root(path) => directories
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
/// keys. A link inside the scan reads them from that root's own
/// [`DirectoryEntry::host_extras`]; an outside ancestor's come from
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
                ChainLink::Root(path) => directories
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

    let chain_links = nesting::warning_chain(roots, &directory.path, ancestors);
    let chain_files = chain_identity_files(directories, &chain_links);
    let chain_extras = chain_host_extras(directories, &chain_links);
    let chain: Vec<ChainAncestor<'_>> = chain_links
        .iter()
        .zip(chain_files.iter())
        .zip(chain_extras.iter())
        .map(|((link, files), extras)| ChainAncestor {
            display: match link {
                ChainLink::Root(path) => path.as_str(),
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
    /// Every discovered directory's path, for [`nesting`] calls.
    roots: &'a [&'a str],
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
    let roots: Vec<&str> = snapshot
        .directories
        .iter()
        .map(|directory| directory.path.as_str())
        .collect();
    let relations =
        nesting::relate(&roots, &snapshot.ancestors).map_err(DefinitionError::Nesting)?;

    let mut classifications: Vec<Classification> = Vec::with_capacity(snapshot.directories.len());
    for directory in &snapshot.directories {
        let identity = DirectoryIdentity::new(
            directory.identity_files.clone(),
            directory.host_extras.clone(),
        )
        .map_err(DefinitionError::DuplicateIdentityKind)?;
        classifications.push(classify(&identity));
    }

    let discovered_by_root = attribute_skills(&roots, &snapshot.skills);
    let agents: Vec<AgentReport> = {
        let ctx = ScanContext {
            hasher: ports.hasher,
            workspace,
            directories: &snapshot.directories,
            roots: &roots,
            ancestors: &snapshot.ancestors,
            discovered_by_root: &discovered_by_root,
        };
        snapshot
            .directories
            .iter()
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
