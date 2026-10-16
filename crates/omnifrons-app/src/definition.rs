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
    let roots: Vec<&str> = snapshot
        .directories
        .iter()
        .map(|directory| directory.path.as_str())
        .collect();

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
        // Computed once, before the block outcome: a legacy marker
        // means `plan_block`'s own `find_block` would already have
        // found *a* block (just not this crate's own sentinel format),
        // and the prototype's own `plan_block` always plans exactly one
        // operation once a block is found (`replace_block`, never
        // `append_block` too). Mirroring that, the migration replaces
        // whatever ordinary pending `skills_block_outcome` would
        // otherwise report, rather than sitting beside it as a second,
        // independent skills-block index.
        let legacy_pending = legacy_block_pending(text);
        match skills_block_outcome(ports.hasher, text, &status, &rows, &root_absolute) {
            BlockOutcome::Refusal(refusal) => return vec![refusal],
            BlockOutcome::Pending(pending) => {
                let mut problems =
                    wrapper_problems(classification.state, ports.hasher, identity_files);
                problems.push(legacy_pending.unwrap_or(pending));
                return problems;
            }
            BlockOutcome::None => {
                let mut problems =
                    wrapper_problems(classification.state, ports.hasher, identity_files);
                if let Some(migrate) = legacy_pending {
                    problems.push(migrate);
                }
                return problems;
            }
        }
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

/// The prototype's own skills-block begin marker (`BLOCK_BEGIN` in the
/// prototype's `adapter/portable-agent/scripts/portable_agent.py`),
/// never emitted by this crate's own [`ManagedBlock`] (which begins
/// `<!-- omnifrons:begin skills ` instead): present only in a file the
/// prototype itself last wrote, or one hand-migrated partway.
const LEGACY_SKILLS_BLOCK_MARKER: &str = "<!-- portable-agent:skills:begin";

/// A pending migration of a legacy [`LEGACY_SKILLS_BLOCK_MARKER`] block
/// to this crate's own sentinel format, when the canonical text still
/// carries one -- outside the prototype's own vocabulary, since the
/// prototype never needs to migrate itself (ADR-0005 sub-slice 1e-D).
/// [`verify_problems_after_state`] uses this result to *replace*
/// whatever ordinary skills-block pending [`skills_block_outcome`]
/// would otherwise report, never to report both: the prototype's own
/// `plan_block` only ever plans one skills-block operation once
/// `find_block` has located a block of any format, and a legacy marker
/// here means exactly that -- a block already exists, just not in this
/// crate's own sentinel format.
fn legacy_block_pending(canonical_text: &[u8]) -> Option<String> {
    let decoded = String::from_utf8_lossy(canonical_text);
    decoded.contains(LEGACY_SKILLS_BLOCK_MARKER).then(|| {
        "pending: migrate the portable-agent skill index block in AGENTS.md to the current \
         omnifrons sentinel format"
            .to_string()
    })
}
