//! The read-only `plan` service (ADR-0005 decisions 2 and 3, sub-slices
//! 2a and 2b): what adapting one agent root to the portable layout would do,
//! without doing it.
//!
//! [`plan`] is the dry run of the eventual write path. It reads one
//! [`crate::definition_tree::TreeSnapshot`] through the same port
//! [`crate::definition::verify`] uses, shares that service's target
//! validation and its skill-metadata and managed-block analysis, and
//! returns either a [`Plan`] (the operations, the fingerprint
//! of every input, and a plan id) or the one [`PlanRefusal`] that stops
//! it. It never writes.
//!
//! # Refusal order
//!
//! A directory without identity files plans nothing. Otherwise the first
//! applicable refusal wins, in this order: an unusable
//! `canonical_from` choice, a nested agent without its own consent, an
//! unsafe identity file, a conflict with no chosen canonical file, a
//! secondary file that would still differ from the source, a skill or
//! managed-block problem (`unsafe_skill_metadata`, `edited_block`,
//! `block_layout`), and a secondary byte copy that a pending skills
//! block would desynchronize (`diverging_copy`). A legacy migration that
//! would leave a second, current block beside the new one is refused as
//! `block_layout` once the block is planned.
//!
//! # Scope of sub-slices 2a and 2b
//!
//! Operations cover the identity files and the one `skills` block write
//! that follows them: an append, a regeneration of the current block, or
//! the one-time migration of an intact legacy `portable-agent:` block to
//! the Omnifrons sentinels. A hand-edited block of either family is
//! refused, never replaced or migrated. The dry run returns the
//! operations, the block's lines before and after ([`BlockDiff`]) and
//! the plan id; the bytes of every write stay here, in
//! [`PlannedWrite`], for the apply service. Backups of overwritten files
//! belong to the write path, which can derive them from each operation's
//! `before` fingerprint.
//!
//! # Notices
//!
//! A `.portable-agent/` backup folder directly inside the agent's own
//! directory is reported, never deleted
//! ([`omnifrons_domain::definition_plan::Notice`]). The directory walk
//! prunes that folder, so the snapshot carries only a presence flag on
//! the directory ([`crate::definition_tree::DirectoryEntry::portable_agent_folder`]);
//! a plan reports the flag of its target's directory, never an
//! ancestor's or a child's. A notice is informational: it is not part of
//! the plan id.

use std::collections::BTreeMap;

use omnifrons_domain::definition::{
    FileShape, IdentityFile, IdentityKind, RootState, WRAPPER, is_import_only,
};
use omnifrons_domain::definition_create::{
    Answers, CREATE_TEMPLATE_VERSION, CreateContext, CreationState, Field, ProvenanceEntry,
    answers_canonical, creation_state, provenance, render,
};
use omnifrons_domain::definition_plan::{
    BlockDiff, InputFingerprint, Notice, Operation, Plan, PlanId, PlanOptions, PlanRefusal,
    SourceProblem, canonical_bytes, home_target_message, host_config_index,
    host_config_target_message, identity_exists_message, inside_host_config_message,
    linked_component_message, nested_create_message, pruned_directory_index, pruned_target_message,
    symlink_target_message,
};
use omnifrons_domain::guidance::{LineEnding, ManagedBlock, ManagedFileKind};
use omnifrons_domain::nesting::{self, Parent};

use crate::content_hasher::ContentHasher;
use crate::definition::{
    BlockAnalysis, BlockRefusal, BlockRefusalCode, DefinitionError, DefinitionPorts, LoadedTarget,
    PendingBlock, absolute_posix, agent_root_paths, block_analysis, load_target, parent_directory,
    relative_within,
};
use crate::definition_tree::{DefinitionTree, PathProbe, TreeSnapshot};
use crate::harness_adapter::WorkspaceRoot;

/// The bytes one planned operation would write.
///
/// [`plan_with_writes`] returns one per operation, in operation order.
/// They live here, not on the public [`Plan`]: the plan states what will
/// change by digest, and the apply service writes these bytes after
/// checking that each one still hashes to its operation's `after`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedWrite {
    /// The file the bytes go to, relative to the agent root.
    pub path: &'static str,
    /// The whole file after the write.
    pub bytes: Vec<u8>,
}

/// What [`plan`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanOutcome {
    /// A plan, possibly with no operations.
    Planned(Plan),
    /// The one refusal that stopped planning. A refused outcome has no
    /// operations and no plan id.
    Refused(PlanRefusal),
}

/// Plans the portable layout for `target` (a clean, workspace-relative
/// POSIX path; `None` means the workspace root, `"."`), without writing.
///
/// # Errors
///
/// The errors [`crate::definition::verify`] documents: an invalid
/// `target` path, a failing [`crate::definition_tree::DefinitionTree`]
/// port, a snapshot that repeats a path, or identity files that repeat a
/// kind; plus [`DefinitionError::InvalidCanonicalChoice`] when
/// `canonical_from` does not apply to the target.
pub fn plan(
    ports: &DefinitionPorts<'_>,
    workspace: &WorkspaceRoot,
    target: Option<&str>,
    options: &PlanOptions,
) -> Result<PlanOutcome, DefinitionError> {
    plan_with_writes(ports, workspace, target, options).map(|(outcome, _)| outcome)
}

/// [`plan`], together with the bytes each planned operation would write.
///
/// The writes are index-aligned with the plan's operations: the `n`th
/// [`PlannedWrite`] holds the bytes whose SHA-256 is the `n`th
/// operation's `after`. A refused outcome has none. The apply service
/// consumes this pair; the public [`Plan`] never carries the bytes.
///
/// # Errors
///
/// The errors [`plan`] documents.
pub fn plan_with_writes(
    ports: &DefinitionPorts<'_>,
    workspace: &WorkspaceRoot,
    target: Option<&str>,
    options: &PlanOptions,
) -> Result<(PlanOutcome, Vec<PlannedWrite>), DefinitionError> {
    plan_for(ports, workspace, target, *options, None)
}

/// [`plan`] for a create: what writing a new definition from the
/// validated `answers` into `target` would do, without doing it.
///
/// The answers are never kept: the plan covers their digest (the
/// operation's `answers_digest`) and the digest of the bytes rendered
/// from them. `canonical_from` plays no part in a create.
///
/// # Errors
///
/// The errors [`plan`] documents, except
/// [`DefinitionError::InvalidCanonicalChoice`]; plus
/// [`DefinitionError::Tree`] when the tree cannot probe paths or the
/// target directory cannot be located.
pub fn plan_create(
    ports: &DefinitionPorts<'_>,
    workspace: &WorkspaceRoot,
    target: Option<&str>,
    options: &PlanOptions,
    answers: &Answers,
) -> Result<PlanOutcome, DefinitionError> {
    plan_create_with_writes(ports, workspace, target, options, answers).map(|(outcome, _)| outcome)
}

/// [`plan_create`], together with the bytes each planned operation would
/// write, index-aligned with the operations as [`plan_with_writes`]
/// documents. The apply service consumes this pair.
///
/// # Errors
///
/// The errors [`plan_create`] documents.
pub fn plan_create_with_writes(
    ports: &DefinitionPorts<'_>,
    workspace: &WorkspaceRoot,
    target: Option<&str>,
    options: &PlanOptions,
    answers: &Answers,
) -> Result<(PlanOutcome, Vec<PlannedWrite>), DefinitionError> {
    plan_for(ports, workspace, target, *options, Some(answers))
}

/// The lines of the file a create from `answers` would write that are
/// built from them, each with the user's own words behind it, in file
/// order: what the preview shows beside the generated text.
///
/// It reads the same tree the planner reads and builds the same
/// [`CreateContext`] (the child agents below `target` and whether another
/// agent encloses it). When that context cannot be built the planner
/// refuses the target, and there are no lines to show: the result is empty.
///
/// # Errors
///
/// The errors [`plan_create`] documents for reading the tree.
pub fn create_provenance(
    ports: &DefinitionPorts<'_>,
    workspace: &WorkspaceRoot,
    target: Option<&str>,
    answers: &Answers,
) -> Result<Vec<ProvenanceEntry>, DefinitionError> {
    let target = target.unwrap_or(".");
    let loaded = load_target(ports, workspace, target)?;
    let site = create_site(&loaded.snapshot, target)?;
    Ok(
        CreateContext::new(site.children.clone(), site.parent.is_some())
            .map_or_else(|_| Vec::new(), |context| provenance(answers, &context)),
    )
}

/// The planner behind [`plan_with_writes`] and [`plan_create_with_writes`]:
/// with `create` answers, the target is planned as a new definition.
pub(crate) fn plan_for(
    ports: &DefinitionPorts<'_>,
    workspace: &WorkspaceRoot,
    target: Option<&str>,
    options: PlanOptions,
    create: Option<&Answers>,
) -> Result<(PlanOutcome, Vec<PlannedWrite>), DefinitionError> {
    let target = target.unwrap_or(".");
    let loaded = load_target(ports, workspace, target)?;
    if let Some(answers) = create {
        return plan_new_definition(ports, workspace, target, options, answers, &loaded);
    }
    plan_existing(ports, workspace, target, options, loaded)
}

/// The adapt planner over an already loaded `target`.
fn plan_existing(
    ports: &DefinitionPorts<'_>,
    workspace: &WorkspaceRoot,
    target: &str,
    options: PlanOptions,
    loaded: LoadedTarget,
) -> Result<(PlanOutcome, Vec<PlannedWrite>), DefinitionError> {
    let LoadedTarget {
        snapshot,
        identity_files,
        classification,
    } = loaded;
    let hasher = ports.hasher;
    let inputs = input_fingerprints(hasher, &identity_files);

    let source = resolve_source(
        target,
        classification.state,
        classification.source,
        &identity_files,
        options.canonical_from,
    )?;
    if classification.state == RootState::None {
        let empty = finish(
            hasher,
            workspace,
            target,
            inputs,
            Vec::new(),
            None,
            &snapshot,
        );
        return Ok((PlanOutcome::Planned(empty), Vec::new()));
    }

    let roots = agent_root_paths(&snapshot.directories);
    if !options.nested_consent
        && let Some(refusal) = nested_without_consent(&snapshot, &roots, target)?
    {
        return Ok(refused(refusal));
    }
    if classification.state == RootState::Unsafe {
        return Ok(refused(PlanRefusal::Unsafe(classification.reason)));
    }
    let Some(source) = source else {
        return Ok(refused(PlanRefusal::Conflict(format!(
            "{}; choose the canonical file with --canonical-from CLAUDE.md or \
             --canonical-from AGENTS.md",
            classification.reason
        ))));
    };
    let Some(source_file) = find(&identity_files, source) else {
        return Ok(refused(PlanRefusal::Unsafe(classification.reason)));
    };

    let (mut operations, mut writes) =
        identity_operations(hasher, &identity_files, source_file, options.with_gemini);
    let unmanaged = unmanaged_secondaries(&identity_files, options.with_gemini);
    let unresolved = unresolved_files(&identity_files, &unmanaged, source_file);
    if !unresolved.is_empty() {
        return Ok(refused(PlanRefusal::UnresolvedFile(format!(
            "{} would still differ from {source}{}",
            file_names(&unresolved),
            gemini_hint(&unresolved)
        ))));
    }

    let pending = match block_analysis(
        ports,
        workspace,
        &snapshot,
        &roots,
        target,
        &identity_files,
        Some(source),
    ) {
        BlockAnalysis::Refused(refusal) => {
            return Ok(refused(block_refusal(refusal)));
        }
        BlockAnalysis::Settled { pending } => pending,
    };
    if pending.is_some() && !unmanaged.is_empty() {
        return Ok(refused(PlanRefusal::DivergingCopy(format!(
            "{}: byte copy of {source} that would differ from AGENTS.md once the skill index \
             block is written, and this plan does not write it. Handle the copy yourself and \
             run the dry-run again{}",
            file_names(&unmanaged),
            gemini_hint(&unmanaged)
        ))));
    }

    let mut block_diff = None;
    if let Some(pending) = pending {
        match plan_block(hasher, &source_file.bytes, &pending) {
            Ok(block) => {
                operations.push(block.operation);
                writes.push(block.write);
                block_diff = Some(block.diff);
            }
            Err(refusal) => return Ok(refused(refusal)),
        }
    }
    Ok((
        PlanOutcome::Planned(finish(
            hasher, workspace, target, inputs, operations, block_diff, &snapshot,
        )),
        writes,
    ))
}

/// The refusal for a `target` that sits below another agent root while
/// its parent's consent is missing, or `None` when it has no parent.
fn nested_without_consent(
    snapshot: &TreeSnapshot,
    roots: &[&str],
    target: &str,
) -> Result<Option<PlanRefusal>, DefinitionError> {
    let relations =
        nesting::relate(roots, &snapshot.ancestors).map_err(DefinitionError::Nesting)?;
    Ok(relations
        .iter()
        .find(|relation| relation.root == target)
        .and_then(|relation| relation.parent.as_ref())
        .map(|parent| nested_refusal(target, parent)))
}

/// The notices for `target`: a `.portable-agent/` folder directly inside
/// its own directory, reported at its workspace-relative path.
fn state_folder_notices(snapshot: &TreeSnapshot, target: &str) -> Vec<Notice> {
    let flagged = snapshot
        .directories
        .iter()
        .any(|entry| entry.path == target && entry.portable_agent_folder);
    if !flagged {
        return Vec::new();
    }
    let path = if target == "." {
        STATE_FOLDER.to_string()
    } else {
        format!("{target}/{STATE_FOLDER}")
    };
    vec![Notice::PortableAgentFolder { path }]
}

/// The prototype's adapter-state folder, beside which a plan reports it.
const STATE_FOLDER: &str = ".portable-agent";

/// The unmanaged secondary files whose bytes differ from the source's.
fn unresolved_files(
    identity_files: &[IdentityFile],
    unmanaged: &[IdentityKind],
    source: &IdentityFile,
) -> Vec<IdentityKind> {
    unmanaged
        .iter()
        .copied()
        .filter(|kind| find(identity_files, *kind).is_some_and(|f| f.bytes != source.bytes))
        .collect()
}

/// The refused outcome: no plan and no writes.
fn refused(refusal: PlanRefusal) -> (PlanOutcome, Vec<PlannedWrite>) {
    (PlanOutcome::Refused(refusal), Vec::new())
}

/// The plan for `agent` with its id derived from the canonical text.
///
/// The id covers the workspace root's raw platform bytes; the
/// `/`-separated lossy text kept in [`Plan::workspace`] is display only
/// and two roots differing in a non-UTF-8 byte may share it.
fn finish(
    hasher: &dyn ContentHasher,
    workspace: &WorkspaceRoot,
    agent: &str,
    inputs: BTreeMap<String, InputFingerprint>,
    operations: Vec<Operation>,
    block_diff: Option<BlockDiff>,
    snapshot: &TreeSnapshot,
) -> Plan {
    let id = PlanId::from_digest(hasher.sha256(&canonical_bytes(
        workspace.path().as_os_str().as_encoded_bytes(),
        agent,
        &inputs,
        &operations,
    )));
    Plan {
        workspace: absolute_posix(workspace, "."),
        agent: agent.to_string(),
        inputs,
        operations,
        block_diff,
        notices: state_folder_notices(snapshot, agent),
        id,
    }
}

/// The fingerprint of every identity file found, keyed by file name.
pub(crate) fn input_fingerprints(
    hasher: &dyn ContentHasher,
    identity_files: &[IdentityFile],
) -> BTreeMap<String, InputFingerprint> {
    identity_files
        .iter()
        .map(|file| (file.kind.file_name().to_string(), fingerprint(hasher, file)))
        .collect()
}

fn fingerprint(hasher: &dyn ContentHasher, file: &IdentityFile) -> InputFingerprint {
    match &file.shape {
        FileShape::Regular => InputFingerprint::Sha256(hasher.sha256(&file.bytes)),
        FileShape::Symlink { target } => InputFingerprint::Symlink(target.clone()),
        FileShape::Irregular => InputFingerprint::Irregular,
    }
}

fn find(identity_files: &[IdentityFile], kind: IdentityKind) -> Option<&IdentityFile> {
    identity_files.iter().find(|file| file.kind == kind)
}

fn is_regular(file: &IdentityFile) -> bool {
    matches!(file.shape, FileShape::Regular)
}

fn is_content_bearing(file: &IdentityFile) -> bool {
    is_regular(file) && !is_import_only(&file.bytes)
}

/// The identity kind whose bytes become `AGENTS.md`: the
/// classification's own source, or the kind `canonical_from` chose to
/// resolve a conflict.
fn resolve_source(
    target: &str,
    state: RootState,
    classified: Option<IdentityKind>,
    identity_files: &[IdentityFile],
    chosen: Option<IdentityKind>,
) -> Result<Option<IdentityKind>, DefinitionError> {
    let Some(kind) = chosen else {
        return Ok(classified);
    };
    let invalid = |detail: String| Err(DefinitionError::InvalidCanonicalChoice { detail });
    if state != RootState::Conflict {
        return invalid(format!(
            "--canonical-from only resolves a conflict; {target} is {}",
            state.as_str()
        ));
    }
    if !matches!(kind, IdentityKind::Claude | IdentityKind::Agents) {
        return invalid(format!(
            "--canonical-from must name CLAUDE.md or AGENTS.md, not {kind}"
        ));
    }
    if !find(identity_files, kind).is_some_and(is_content_bearing) {
        return invalid(format!(
            "--canonical-from {kind}: {kind} holds no content in {target}"
        ));
    }
    Ok(Some(kind))
}

fn nested_refusal(target: &str, parent: &Parent) -> PlanRefusal {
    let within = match parent {
        Parent::Root(path) => format!("the agent at {path}"),
        Parent::Outside(_) => "an agent above the workspace".to_string(),
    };
    PlanRefusal::NestedAgent(format!(
        "{target} is nested inside {within}; adapt it separately with its own consent"
    ))
}

/// The operations the identity files need, in apply order -- the
/// canonical `AGENTS.md`, then the `CLAUDE.md` wrapper, then (opt-in)
/// the `GEMINI.md` wrapper -- each paired, by index, with the bytes it
/// writes.
fn identity_operations(
    hasher: &dyn ContentHasher,
    identity_files: &[IdentityFile],
    source: &IdentityFile,
    with_gemini: bool,
) -> (Vec<Operation>, Vec<PlannedWrite>) {
    let mut operations = Vec::new();
    let mut writes = Vec::new();
    let agents = find(identity_files, IdentityKind::Agents);
    if agents.is_none_or(|file| file.bytes != source.bytes) {
        operations.push(Operation::WriteAgents {
            source: source.kind,
            after: hasher.sha256(&source.bytes),
            before: before_of(hasher, agents),
        });
        writes.push(PlannedWrite {
            path: IdentityKind::Agents.file_name(),
            bytes: source.bytes.clone(),
        });
    }
    let wrappers: &[IdentityKind] = if with_gemini {
        &[IdentityKind::Claude, IdentityKind::Gemini]
    } else {
        &[IdentityKind::Claude]
    };
    for kind in wrappers.iter().copied() {
        let current = find(identity_files, kind);
        if current.is_none_or(|file| is_regular(file) && file.bytes != WRAPPER) {
            operations.push(Operation::WriteWrapper {
                kind,
                after: hasher.sha256(WRAPPER),
                before: before_of(hasher, current),
            });
            writes.push(PlannedWrite {
                path: kind.file_name(),
                bytes: WRAPPER.to_vec(),
            });
        }
    }
    (operations, writes)
}

/// One planned `skills` block write.
struct BlockPlan {
    operation: Operation,
    write: PlannedWrite,
    diff: BlockDiff,
}

/// The block write for `pending` over the canonical `text`: the
/// operation, the whole file after it, and the diff.
///
/// The analysis never hands over an empty body (a regeneration falls
/// back to the rule section alone), so an empty one here is a defect in
/// the caller: it is reported as a `block_layout` refusal naming the
/// cause, never dropped.
///
/// Everything outside the block is copied byte for byte. An append adds
/// one blank line and the block at the end of the text; a replacement or
/// a migration swaps exactly the block's span (and a migration beside a
/// current block also drops the legacy span), so text after the block
/// and its terminator stay as they were. The block uses the file's
/// dominant line ending.
fn plan_block(
    hasher: &dyn ContentHasher,
    text: &[u8],
    pending: &PendingBlock,
) -> Result<BlockPlan, PlanRefusal> {
    let digest = |bytes: &[u8]| hasher.sha256(bytes);
    let decoded = String::from_utf8_lossy(text);
    let block = ManagedBlock::skills(pending.body.clone(), &digest).map_err(|_| {
        PlanRefusal::BlockLayout(
            "the skill index block cannot be written: its body would be empty".to_string(),
        )
    })?;
    let ending = LineEnding::dominant(&decoded);
    let rendered = block.render(ending);
    let terminator = ending.as_str();

    let mut bytes = Vec::with_capacity(text.len() + rendered.len() + 2 * terminator.len());
    let mut replaced = Vec::new();
    if let Some(span) = &pending.span {
        // The spans do not overlap; apply them in file order.
        let mut edits = vec![(span.clone(), rendered.as_bytes())];
        edits.extend(
            pending
                .also_remove
                .iter()
                .map(|range| (range.clone(), &b""[..])),
        );
        edits.sort_by_key(|(range, _)| range.start);
        let mut at = 0;
        for (range, replacement) in edits {
            bytes.extend_from_slice(&text[at..range.start]);
            bytes.extend_from_slice(replacement);
            replaced.push(String::from_utf8_lossy(&text[range.clone()]).into_owned());
            at = range.end;
        }
        bytes.extend_from_slice(&text[at..]);
    } else {
        bytes.extend_from_slice(text);
        // One blank line before the block, judged from the actual tail:
        // none when the text already ends in one, one terminator when it
        // ends in a line terminator (LF or CRLF), two otherwise.
        let blank_line = text.ends_with(b"\n\n") || text.ends_with(b"\n\r\n");
        let padding = if text.is_empty() || blank_line {
            0
        } else if text.ends_with(b"\n") {
            1
        } else {
            2
        };
        for _ in 0..padding {
            bytes.extend_from_slice(terminator.as_bytes());
        }
        bytes.extend_from_slice(rendered.as_bytes());
        bytes.extend_from_slice(terminator.as_bytes());
    }
    let operation = Operation::WriteBlock {
        kind: ManagedFileKind::Skills,
        mode: pending.mode,
        body: block.digest(),
        after: hasher.sha256(&bytes),
        // The block runs on the canonical text: the current `AGENTS.md`,
        // or the bytes `WriteAgents` has just written.
        before: InputFingerprint::Sha256(hasher.sha256(text)),
    };
    let diff = BlockDiff {
        path: IdentityKind::Agents.file_name(),
        before: replaced
            .iter()
            .flat_map(|removed| removed.lines().map(str::to_string))
            .collect(),
        after: rendered.lines().map(str::to_string).collect(),
    };
    Ok(BlockPlan {
        operation,
        write: PlannedWrite {
            path: IdentityKind::Agents.file_name(),
            bytes,
        },
        diff,
    })
}

fn before_of(hasher: &dyn ContentHasher, file: Option<&IdentityFile>) -> InputFingerprint {
    file.map_or(InputFingerprint::Absent, |file| fingerprint(hasher, file))
}

/// The content-bearing secondary identity files this plan never writes.
fn unmanaged_secondaries(identity_files: &[IdentityFile], with_gemini: bool) -> Vec<IdentityKind> {
    IdentityKind::ALL
        .into_iter()
        .filter(IdentityKind::is_secondary)
        .filter(|kind| !(with_gemini && *kind == IdentityKind::Gemini))
        .filter(|kind| find(identity_files, *kind).is_some_and(is_content_bearing))
        .collect()
}

fn file_names(kinds: &[IdentityKind]) -> String {
    kinds
        .iter()
        .map(IdentityKind::file_name)
        .collect::<Vec<_>>()
        .join(", ")
}

fn gemini_hint(kinds: &[IdentityKind]) -> &'static str {
    if kinds.contains(&IdentityKind::Gemini) {
        " (--with-gemini replaces GEMINI.md with the wrapper)"
    } else {
        ""
    }
}

/// The plan refusal for a block analysis refusal.
fn block_refusal(refusal: BlockRefusal) -> PlanRefusal {
    let BlockRefusal { code, detail } = refusal;
    match code {
        BlockRefusalCode::EditedBlock => PlanRefusal::EditedBlock(detail),
        BlockRefusalCode::BlockLayout => PlanRefusal::BlockLayout(detail),
        BlockRefusalCode::UnsafeSkillMetadata => PlanRefusal::UnsafeSkillMetadata(detail),
    }
}

// ------------------------------------------------------------- create

/// What a create reads from the snapshot besides the identity files:
/// every agent root (the target among them), the target's direct child
/// agents, and the agent that encloses it.
struct CreateSite<'a> {
    roots: Vec<&'a str>,
    children: Vec<String>,
    parent: Option<Parent>,
}

/// Plans `target` as a new definition rendered from `answers`.
///
/// The gates run in the prototype's order, each refusing before the next
/// reads anything: an unsafe target (a symlink, a host configuration
/// folder or a folder inside one, the home directory); an existing agent
/// (any identity file that is not this writer's own result); a child path
/// the template cannot hold; the skill refusals of the adapt path; an
/// `AGENTS.md` that is not what the answers render; a nested target
/// without its own consent; a `sources` entry that is missing, a symlink,
/// or inside or around a child agent.
///
/// The enclosing agent's files are not fingerprinted in the plan inputs: a
/// parent appearing or vanishing changes the rendered body (the enclosing
/// line) and so the plan id.
fn plan_new_definition(
    ports: &DefinitionPorts<'_>,
    workspace: &WorkspaceRoot,
    target: &str,
    options: PlanOptions,
    answers: &Answers,
    loaded: &LoadedTarget,
) -> Result<(PlanOutcome, Vec<PlannedWrite>), DefinitionError> {
    let hasher = ports.hasher;
    let files = &loaded.identity_files;
    if let Some(refusal) = unsafe_target(ports.tree, workspace, target)? {
        return Ok(refused(refusal));
    }
    let own = own_files(files, options.with_gemini);
    let existing = find(&own, IdentityKind::Agents).map_or(&[][..], |file| &file.bytes[..]);
    if creation_state(&own, existing) == CreationState::IdentityExists {
        return Ok(refused(existing_agent(target, files, "")));
    }
    let site = create_site(&loaded.snapshot, target)?;
    let context = match CreateContext::new(site.children.clone(), site.parent.is_some()) {
        Ok(context) => context,
        Err(error) => return Ok(refused(PlanRefusal::UnsafeTarget(error.to_string()))),
    };
    let body = render(answers, &context).into_bytes();
    let full = match with_skills_block(
        ports,
        workspace,
        &loaded.snapshot,
        &site.roots,
        target,
        body,
    ) {
        Ok(full) => full,
        Err(refusal) => return Ok(refused(refusal)),
    };
    if creation_state(&own, &full) == CreationState::IdentityExists {
        let why = " whose AGENTS.md is not what these answers render";
        return Ok(refused(existing_agent(target, files, why)));
    }
    if let Some(refusal) = nested_create_refusal(options, &site, target) {
        return Ok(refused(refusal));
    }
    if let Some(refusal) = source_refusal(ports.tree, workspace, target, answers, &site.children)? {
        return Ok(refused(refusal));
    }
    let (operations, writes) = create_operations(hasher, answers, files, full, options.with_gemini);
    let inputs = input_fingerprints(hasher, files);
    let plan = finish(
        hasher,
        workspace,
        target,
        inputs,
        operations,
        None,
        &loaded.snapshot,
    );
    Ok((PlanOutcome::Planned(plan), writes))
}

/// The identity files a create judges: all of them, except a regular
/// `GEMINI.md` that already is the wrapper when the caller asked for the
/// Gemini wrapper (a previous run of this same create wrote it).
///
/// So a rerun of a create that asked for the Gemini wrapper converges: a
/// finished one plans nothing, and one stopped before `GEMINI.md` plans
/// only that wrapper.
fn own_files(files: &[IdentityFile], with_gemini: bool) -> Vec<IdentityFile> {
    files
        .iter()
        .filter(|file| {
            !(with_gemini
                && file.kind == IdentityKind::Gemini
                && is_regular(file)
                && file.bytes == WRAPPER)
        })
        .cloned()
        .collect()
}

fn existing_agent(target: &str, files: &[IdentityFile], why: &str) -> PlanRefusal {
    let names: Vec<&str> = files.iter().map(|file| file.kind.file_name()).collect();
    PlanRefusal::IdentityExists(identity_exists_message(target, &names, why))
}

/// The refusal for a create target that is a symlink, lies in or is a host
/// configuration folder, or is the home directory, judged on the physical
/// path as hosts see it.
fn unsafe_target(
    tree: &dyn DefinitionTree,
    workspace: &WorkspaceRoot,
    target: &str,
) -> Result<Option<PlanRefusal>, DefinitionError> {
    let given: Vec<&str> = target
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if let Some(index) = pruned_directory_index(&given) {
        let message = pruned_target_message(target, given[index]);
        return Ok(Some(PlanRefusal::UnsafeTarget(message)));
    }
    let mut prefix = String::new();
    for (index, part) in given.iter().enumerate() {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        let probe = tree
            .probe(workspace, &prefix)
            .map_err(DefinitionError::Tree)?;
        if let PathProbe::Symlink { link } = probe {
            let message = if index + 1 == given.len() {
                symlink_target_message(target, &link)
            } else {
                linked_component_message(target, &prefix, &link)
            };
            return Ok(Some(PlanRefusal::UnsafeTarget(message)));
        }
    }
    if given.is_empty() {
        let probe = tree
            .probe(workspace, target)
            .map_err(DefinitionError::Tree)?;
        if let PathProbe::Symlink { link } = probe {
            let message = symlink_target_message(target, &link);
            return Ok(Some(PlanRefusal::UnsafeTarget(message)));
        }
    }
    let location = tree
        .locate(workspace, target)
        .map_err(DefinitionError::Tree)?;
    let names: Vec<&str> = location.components.iter().map(String::as_str).collect();
    // The walk starts at the workspace root, so only the components below
    // it are ever pruned: strip the root's own physical components, which
    // the root's location reports in the same form.
    let root_names = tree
        .locate(workspace, ".")
        .map_err(DefinitionError::Tree)?
        .components;
    let offset = if names.len() >= root_names.len()
        && names.iter().zip(&root_names).all(|(a, b)| *a == b.as_str())
    {
        root_names.len()
    } else {
        0
    };
    let below_root = pruned_directory_index(&names[offset..]).map(|index| (offset, index));
    let physical = |end: usize| format!("/{}", names[..end].join("/"));
    let message = match host_config_index(&names) {
        Some(index) if index + 1 == names.len() => {
            host_config_target_message(&physical(names.len()), names[index])
        }
        Some(index) => inside_host_config_message(
            &physical(names.len()),
            &physical(index + 1),
            names[index],
            &physical(index),
        ),
        None if below_root.is_some() => {
            let (offset, index) = below_root.unwrap_or_default();
            pruned_target_message(&physical(names.len()), names[offset + index])
        }
        None if location.home => home_target_message(&physical(names.len())),
        None => return Ok(None),
    };
    Ok(Some(PlanRefusal::UnsafeTarget(message)))
}

/// The roots, child agents and enclosing agent of a create `target`.
fn create_site<'a>(
    snapshot: &'a TreeSnapshot,
    target: &'a str,
) -> Result<CreateSite<'a>, DefinitionError> {
    let mut roots = agent_root_paths(&snapshot.directories);
    if !roots.contains(&target) {
        roots.push(target);
    }
    let relations =
        nesting::relate(&roots, &snapshot.ancestors).map_err(DefinitionError::Nesting)?;
    let parent = relations
        .into_iter()
        .find(|relation| relation.root == target)
        .and_then(|relation| relation.parent);
    let children = child_agents(&roots, target);
    Ok(CreateSite {
        roots,
        children,
        parent,
    })
}

/// The agents directly below `target`, relative to it and sorted by path
/// component: every deeper agent belongs to one of them.
fn child_agents(roots: &[&str], target: &str) -> Vec<String> {
    let prefix = format!("{target}/");
    let below: Vec<&str> = roots
        .iter()
        .copied()
        .filter(|root| *root != target && (target == "." || root.starts_with(&prefix)))
        .collect();
    let mut children: Vec<String> = below
        .iter()
        .filter(|root| nesting::nearest_root(&below, parent_directory(root)).is_none())
        .map(|root| relative_within(target, root))
        .collect();
    children.sort_by(|a, b| a.split('/').cmp(b.split('/')));
    children
}

/// The whole `AGENTS.md` a create writes: `body`, followed by one blank
/// line and the skills block when the folder holds local skills. The
/// block is the adapt path's own, appended to the rendered body.
fn with_skills_block(
    ports: &DefinitionPorts<'_>,
    workspace: &WorkspaceRoot,
    snapshot: &TreeSnapshot,
    roots: &[&str],
    target: &str,
    body: Vec<u8>,
) -> Result<Vec<u8>, PlanRefusal> {
    let agents = [IdentityFile {
        kind: IdentityKind::Agents,
        bytes: body,
        shape: FileShape::Regular,
    }];
    let analysis = block_analysis(
        ports,
        workspace,
        snapshot,
        roots,
        target,
        &agents,
        Some(IdentityKind::Agents),
    );
    let [agents] = agents;
    match analysis {
        BlockAnalysis::Refused(refusal) => Err(block_refusal(refusal)),
        BlockAnalysis::Settled { pending: None } => Ok(agents.bytes),
        BlockAnalysis::Settled {
            pending: Some(pending),
        } => plan_block(ports.hasher, &agents.bytes, &pending).map(|block| block.write.bytes),
    }
}

/// The refusal for a create target below another agent without its own
/// consent.
fn nested_create_refusal(
    options: PlanOptions,
    site: &CreateSite<'_>,
    target: &str,
) -> Option<PlanRefusal> {
    if options.nested_consent {
        return None;
    }
    let parent = match site.parent.as_ref()? {
        Parent::Root(path) | Parent::Outside(path) => path,
    };
    Some(PlanRefusal::NestedAgent(nested_create_message(
        target, parent,
    )))
}

/// The refusal for the first `sources` entry that is not a real entry of
/// the new agent's workspace.
fn source_refusal(
    tree: &dyn DefinitionTree,
    workspace: &WorkspaceRoot,
    target: &str,
    answers: &Answers,
    children: &[String],
) -> Result<Option<PlanRefusal>, DefinitionError> {
    for (index, item) in answers.items(Field::Sources).iter().enumerate() {
        if let Some(problem) = source_problem(tree, workspace, target, &item.text, children)? {
            return Ok(Some(problem.refusal(index, &item.text, target)));
        }
    }
    Ok(None)
}

/// Why `entry` is not a plain path relative to the target, judged on the
/// text alone and before anything is probed: the plan target's own rules
/// ([`nesting::relate`]), `.` itself, a NUL byte everywhere, and a
/// backslash or colon on Windows.
fn malformed_source(entry: &str) -> Option<String> {
    if entry.contains('\0') {
        return Some("it holds a NUL byte".to_string());
    }
    if cfg!(windows) && entry.contains(['\\', ':']) {
        return Some("it holds a backslash or a colon".to_string());
    }
    if entry == "." {
        return Some("it names the target itself".to_string());
    }
    nesting::relate(&[entry], &[])
        .err()
        .map(|error| error.to_string())
}

/// Why `entry` (a path relative to `target`) is not a real source: each
/// part must exist and not be a symlink, and neither the entry nor
/// anything above it may be a child agent, nor may the entry hold one.
fn source_problem(
    tree: &dyn DefinitionTree,
    workspace: &WorkspaceRoot,
    target: &str,
    entry: &str,
    children: &[String],
) -> Result<Option<SourceProblem>, DefinitionError> {
    if let Some(reason) = malformed_source(entry) {
        return Ok(Some(SourceProblem::Malformed { reason }));
    }
    let mut current = String::new();
    for part in entry.split('/') {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(part);
        let path = if target == "." {
            current.clone()
        } else {
            format!("{target}/{current}")
        };
        match tree
            .probe(workspace, &path)
            .map_err(DefinitionError::Tree)?
        {
            PathProbe::Symlink { .. } => {
                return Ok(Some(SourceProblem::Symlink { shown: current }));
            }
            PathProbe::Absent => return Ok(Some(SourceProblem::Missing)),
            PathProbe::Present => {}
        }
        if let Some(child) = children.iter().find(|child| **child == current) {
            return Ok(Some(SourceProblem::InsideChild {
                child: child.clone(),
            }));
        }
    }
    let held = format!("{current}/");
    Ok(children
        .iter()
        .find(|child| child.starts_with(&held))
        .map(|child| SourceProblem::ContainsChild {
            child: child.clone(),
        }))
}

/// The operations of a create, in apply order, each paired with its
/// bytes: `AGENTS.md` when it does not exist yet, then every wrapper that
/// does not. Nothing is overwritten, so each `before` is absent.
fn create_operations(
    hasher: &dyn ContentHasher,
    answers: &Answers,
    files: &[IdentityFile],
    full: Vec<u8>,
    with_gemini: bool,
) -> (Vec<Operation>, Vec<PlannedWrite>) {
    let mut operations = Vec::new();
    let mut writes = Vec::new();
    if find(files, IdentityKind::Agents).is_none() {
        operations.push(Operation::WriteGenerated {
            template_version: CREATE_TEMPLATE_VERSION.to_string(),
            answers_digest: hasher.sha256(&answers_canonical(answers)),
            after: hasher.sha256(&full),
            before: InputFingerprint::Absent,
        });
        writes.push(PlannedWrite {
            path: IdentityKind::Agents.file_name(),
            bytes: full,
        });
    }
    let wrappers: &[IdentityKind] = if with_gemini {
        &[IdentityKind::Claude, IdentityKind::Gemini]
    } else {
        &[IdentityKind::Claude]
    };
    for kind in wrappers.iter().copied() {
        if find(files, kind).is_none() {
            operations.push(Operation::WriteWrapper {
                kind,
                after: hasher.sha256(WRAPPER),
                before: InputFingerprint::Absent,
            });
            writes.push(PlannedWrite {
                path: kind.file_name(),
                bytes: WRAPPER.to_vec(),
            });
        }
    }
    (operations, writes)
}

#[cfg(test)]
mod tests {
    use omnifrons_domain::definition_plan::BlockMode;

    use super::*;
    use crate::contract::publication::FakeHasher;

    /// An empty body is reported, never dropped: the plan would
    /// otherwise lose its block operation while `verify` kept saying it
    /// is pending.
    #[test]
    fn an_empty_block_body_is_a_refusal_not_a_dropped_operation() {
        let pending = PendingBlock {
            mode: BlockMode::Replace,
            span: Some(0..3),
            also_remove: None,
            body: String::new(),
            rows: 0,
            rule: false,
        };
        let Err(refusal) = plan_block(&FakeHasher, b"abc", &pending) else {
            panic!("an empty body must be refused");
        };
        assert_eq!(refusal.code(), "block_layout");
        assert!(refusal.message().contains("empty"), "{refusal}");
    }
}
