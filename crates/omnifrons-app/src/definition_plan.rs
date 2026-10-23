//! The read-only `plan` service (ADR-0005 decisions 2 and 3, sub-slice
//! 2a): what adapting one agent root to the portable layout would do,
//! without doing it.
//!
//! [`plan`] is the dry run of the eventual write path. It reads one
//! [`crate::definition_tree::TreeSnapshot`] through the same port
//! [`crate::definition::verify`] uses, shares that service's target
//! validation and its skill-metadata and managed-block analysis, and
//! returns either a [`Plan`] (identity-file operations, the fingerprint
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
//! block would desynchronize (`diverging_copy`).
//!
//! # Scope of sub-slice 2a
//!
//! Operations cover the identity files only. The skills block and the
//! legacy-block migration are not planned here, but their refusals
//! already apply, and a pending block write is read only to decide
//! `diverging_copy`. Backups of overwritten files belong to the write
//! path, which can derive them from each operation's `before`
//! fingerprint.

use std::collections::BTreeMap;

use omnifrons_domain::definition::{
    FileShape, IdentityFile, IdentityKind, RootState, WRAPPER, is_import_only,
};
use omnifrons_domain::definition_plan::{
    InputFingerprint, Operation, Plan, PlanId, PlanOptions, PlanRefusal, canonical_bytes,
};
use omnifrons_domain::nesting::{self, Parent};

use crate::content_hasher::ContentHasher;
use crate::definition::{
    BlockAnalysis, DefinitionError, DefinitionPorts, LoadedTarget, agent_root_paths,
    block_analysis, load_target, split_refusal,
};
use crate::harness_adapter::WorkspaceRoot;

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
    let target = target.unwrap_or(".");
    let LoadedTarget {
        snapshot,
        identity_files,
        classification,
    } = load_target(ports, workspace, target)?;
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
        return Ok(PlanOutcome::Planned(finish(
            hasher,
            target,
            inputs,
            Vec::new(),
        )));
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
            return Ok(PlanOutcome::Refused(nested_refusal(target, parent)));
        }
    }
    if classification.state == RootState::Unsafe {
        return Ok(PlanOutcome::Refused(PlanRefusal::Unsafe(
            classification.reason,
        )));
    }
    let Some(source) = source else {
        return Ok(PlanOutcome::Refused(PlanRefusal::Conflict(format!(
            "{}; choose the canonical file with --canonical-from CLAUDE.md or \
             --canonical-from AGENTS.md",
            classification.reason
        ))));
    };
    let Some(source_file) = find(&identity_files, source) else {
        return Ok(PlanOutcome::Refused(PlanRefusal::Unsafe(
            classification.reason,
        )));
    };

    let operations = identity_operations(hasher, &identity_files, source_file, options.with_gemini);
    let unmanaged = unmanaged_secondaries(&identity_files, options.with_gemini);
    let unresolved: Vec<IdentityKind> = unmanaged
        .iter()
        .copied()
        .filter(|kind| find(&identity_files, *kind).is_some_and(|f| f.bytes != source_file.bytes))
        .collect();
    if !unresolved.is_empty() {
        return Ok(PlanOutcome::Refused(PlanRefusal::UnresolvedFile(format!(
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
        BlockAnalysis::Refused(message) => {
            return Ok(PlanOutcome::Refused(block_refusal(message)));
        }
        BlockAnalysis::Settled { pending } => pending,
    };
    if pending.is_some() && !unmanaged.is_empty() {
        return Ok(PlanOutcome::Refused(PlanRefusal::DivergingCopy(format!(
            "{}: byte copy of {source} that would differ from AGENTS.md once the skill index \
             block is written, and this plan does not write it. Handle the copy yourself and \
             run the dry-run again{}",
            file_names(&unmanaged),
            gemini_hint(&unmanaged)
        ))));
    }

    Ok(PlanOutcome::Planned(finish(
        hasher, target, inputs, operations,
    )))
}

/// The plan for `agent` with its id derived from the canonical text.
fn finish(
    hasher: &dyn ContentHasher,
    agent: &str,
    inputs: BTreeMap<String, InputFingerprint>,
    operations: Vec<Operation>,
) -> Plan {
    let id = PlanId::from_digest(hasher.sha256(&canonical_bytes(agent, &inputs, &operations)));
    Plan {
        agent: agent.to_string(),
        inputs,
        operations,
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

/// The operations the identity files need, in apply order: the
/// canonical `AGENTS.md`, then the `CLAUDE.md` wrapper, then (opt-in)
/// the `GEMINI.md` wrapper.
fn identity_operations(
    hasher: &dyn ContentHasher,
    identity_files: &[IdentityFile],
    source: &IdentityFile,
    with_gemini: bool,
) -> Vec<Operation> {
    let mut operations = Vec::new();
    let agents = find(identity_files, IdentityKind::Agents);
    if agents.is_none_or(|file| file.bytes != source.bytes) {
        operations.push(Operation::WriteAgents {
            source: source.kind,
            after: hasher.sha256(&source.bytes),
            before: before_of(hasher, agents),
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
        }
    }
    operations
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

/// The typed refusal for a `refused (<code>): <detail>` string
/// [`block_analysis`] produced.
fn block_refusal(message: String) -> PlanRefusal {
    match split_refusal(&message) {
        Some((code, detail)) => PlanRefusal::from_block_code(code, detail.to_string())
            .unwrap_or_else(|| PlanRefusal::Unsafe(message.clone())),
        None => PlanRefusal::Unsafe(message),
    }
}
