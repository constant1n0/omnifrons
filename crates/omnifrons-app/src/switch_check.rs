//! The pre-restart switch check as an application service: runs the
//! definition scan once and asks the domain whether the target host
//! reaches the canonical definition from the active project root
//! (ADR-0005, "The loader table in model switching").
//!
//! No restart flow and no persistence live here: the check only reads.

use omnifrons_domain::adapter::AdapterId;
use omnifrons_domain::definition::{FileShape, IdentityKind};
use omnifrons_domain::switch_check::{
    AncestorSite, DefinitionReference, SwitchSite, SwitchVerdict, host_for_adapter,
    switch_check_for_adapter,
};

use crate::definition::{AgentReport, DefinitionError, DefinitionPorts, scan};
use crate::harness_adapter::WorkspaceRoot;

/// The scan's name for the active project root.
const ROOT_PATH: &str = ".";

/// The result of [`switch_check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchCheck {
    /// The host the verdict is about: the adapter's bound host, else the
    /// host the user chose; `None` when there is neither.
    pub host: Option<String>,
    /// Whether that host reaches the canonical definition.
    pub verdict: SwitchVerdict,
    /// The canonical definition, for a startup brief: `AGENTS.md` at the
    /// root with the SHA-256 of its bytes; `None` when the root holds no
    /// regular `AGENTS.md`.
    pub reference: Option<DefinitionReference>,
}

/// Whether the host `adapter` runs (or `chosen_host`, for an adapter
/// bound to none) reaches the canonical definition from `workspace`.
///
/// Runs [`scan`] once and reads everything from that report. A
/// symlinked or irregular identity file counts as absent: the hosts
/// would read through a link, but the definition rules treat links as
/// unsafe, so a verdict never rests on one and the reference never
/// names one.
///
/// # Errors
///
/// Returns whatever [`scan`] returns when the tree cannot be read or
/// the snapshot is malformed.
pub fn switch_check(
    ports: &DefinitionPorts<'_>,
    workspace: &WorkspaceRoot,
    adapter: &AdapterId,
    chosen_host: Option<&str>,
) -> Result<SwitchCheck, DefinitionError> {
    let report = scan(ports, workspace)?;
    let root = report.agents.iter().find(|agent| agent.path == ROOT_PATH);
    let site = SwitchSite {
        root_files: root.map(root_file_names).unwrap_or_default(),
        ancestors: report
            .ancestors
            .iter()
            .map(|ancestor| AncestorSite {
                path: ancestor.path.clone(),
                files: ancestor
                    .identity_files
                    .iter()
                    .filter_map(|entry| ancestor_file_name(entry))
                    .collect(),
            })
            .collect(),
    };
    let verdict = switch_check_for_adapter(adapter, chosen_host, &site);
    Ok(SwitchCheck {
        host: host_for_adapter(adapter)
            .or(chosen_host)
            .map(str::to_string),
        verdict,
        reference: root.and_then(reference_of),
    })
}

/// The file names of `agent`'s regular identity files, plus the host
/// extras present at that root: a host that lists one of its own files
/// ahead of the definition (Hermes Agent's `.hermes.md`) must see it, or
/// the verdict would say it reaches a file it never loads.
fn root_file_names(agent: &AgentReport) -> Vec<String> {
    agent
        .identity_files
        .iter()
        .filter(|file| matches!(file.shape, FileShape::Regular))
        .map(|file| file.kind.file_name().to_string())
        .chain(agent.host_extras.iter().cloned())
        .collect()
}

/// An ancestor's identity entry as a file name: the port may supply
/// either the file name or the kind's token. An entry neither form
/// recognizes is dropped. It cannot shadow anything: the loader table
/// only ever lists the identity file names, so an unknown entry is
/// never one a host reads.
fn ancestor_file_name(entry: &str) -> Option<String> {
    IdentityKind::from_file_name(entry)
        .or_else(|| IdentityKind::parse(entry))
        .map(|kind| kind.file_name().to_string())
}

/// The reference to the root's regular `AGENTS.md`, if it has one.
///
/// The entry is selected by kind and by [`FileShape::Regular`] here, so
/// a symlinked or irregular file is never named whatever the report
/// holds for it. [`scan`] only hashes regular files, so a non-regular
/// entry with a digest cannot be built through it. A digest that does
/// not satisfy [`DefinitionReference::new`] (the scan always yields 64
/// lowercase hex characters, so this cannot happen) yields no
/// reference rather than a made-up error: no [`DefinitionError`]
/// variant describes it, and the reference is optional by contract.
fn reference_of(root: &AgentReport) -> Option<DefinitionReference> {
    let agents = root.identity_files.iter().find(|file| {
        file.kind == IdentityKind::Agents && matches!(file.shape, FileShape::Regular)
    })?;
    DefinitionReference::new(IdentityKind::Agents.file_name(), agents.sha256.clone()?).ok()
}
