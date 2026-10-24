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
//! # Not yet reported
//!
//! A `.portable-agent/` backup folder beside the definition is meant to
//! be reported, never deleted ([`omnifrons_domain::definition_plan::Notice`]).
//! The directory walk prunes that folder (`SKIP_DIRS`), so the tree
//! snapshot cannot show it and [`Plan::notices`] stays empty until the
//! adapter exposes it.

use std::collections::BTreeMap;

use omnifrons_domain::definition::{
    FileShape, IdentityFile, IdentityKind, RootState, WRAPPER, is_import_only,
};
use omnifrons_domain::definition_plan::{
    BlockDiff, InputFingerprint, Operation, Plan, PlanId, PlanOptions, PlanRefusal, canonical_bytes,
};
use omnifrons_domain::guidance::{LineEnding, ManagedBlock, ManagedFileKind};
use omnifrons_domain::nesting::{self, Parent};

use crate::content_hasher::ContentHasher;
use crate::definition::{
    BlockAnalysis, BlockRefusal, BlockRefusalCode, DefinitionError, DefinitionPorts, LoadedTarget,
    PendingBlock, absolute_posix, agent_root_paths, block_analysis, load_target,
};
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
    let target = target.unwrap_or(".");
    let LoadedTarget {
        snapshot,
        identity_files,
        classification,
    } = load_target(ports, workspace, target)?;
    let hasher = ports.hasher;
    // The location the plan is scoped to: its id covers it.
    let location = absolute_posix(workspace, ".");
    let inputs = input_fingerprints(hasher, &identity_files);

    let source = resolve_source(
        target,
        classification.state,
        classification.source,
        &identity_files,
        options.canonical_from,
    )?;
    if classification.state == RootState::None {
        return Ok((
            PlanOutcome::Planned(finish(hasher, &location, target, inputs, Vec::new(), None)),
            Vec::new(),
        ));
    }

    let roots = agent_root_paths(&snapshot.directories);
    if !options.nested_consent {
        let relations =
            nesting::relate(&roots, &snapshot.ancestors).map_err(DefinitionError::Nesting)?;
        let parent = relations
            .iter()
            .find(|relation| relation.root == target)
            .and_then(|relation| relation.parent.as_ref());
        if let Some(parent) = parent {
            return Ok(refused(nested_refusal(target, parent)));
        }
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
        // An empty body means there is nothing to write.
        if let Some(block) = plan_block(hasher, &source_file.bytes, &pending) {
            operations.push(block.operation);
            writes.push(block.write);
            block_diff = Some(block.diff);
        }
    }
    Ok((
        PlanOutcome::Planned(finish(
            hasher, &location, target, inputs, operations, block_diff,
        )),
        writes,
    ))
}

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
fn finish(
    hasher: &dyn ContentHasher,
    workspace: &str,
    agent: &str,
    inputs: BTreeMap<String, InputFingerprint>,
    operations: Vec<Operation>,
    block_diff: Option<BlockDiff>,
) -> Plan {
    let id = PlanId::from_digest(hasher.sha256(&canonical_bytes(
        workspace,
        agent,
        &inputs,
        &operations,
    )));
    Plan {
        workspace: workspace.to_string(),
        agent: agent.to_string(),
        inputs,
        operations,
        block_diff,
        notices: Vec::new(),
        id,
    }
}

/// The fingerprint of every identity file found, keyed by file name.
fn input_fingerprints(
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
/// operation, the whole file after it, and the diff. `None` when the
/// pending body is empty, which `render_body` documents as "no block to
/// write".
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
) -> Option<BlockPlan> {
    let digest = |bytes: &[u8]| hasher.sha256(bytes);
    let decoded = String::from_utf8_lossy(text);
    let block = ManagedBlock::skills(pending.body.clone(), &digest).ok()?;
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
        if !text.is_empty() {
            if !bytes.ends_with(terminator.as_bytes()) {
                bytes.extend_from_slice(terminator.as_bytes());
            }
            if !bytes.ends_with(format!("{terminator}{terminator}").as_bytes()) {
                bytes.extend_from_slice(terminator.as_bytes());
            }
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
    Some(BlockPlan {
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
