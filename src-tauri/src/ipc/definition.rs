//! The agent-definition commands (ADR-0005 sub-slices 1h and 2f):
//! `definition_status`, `definition_verify`, `definition_preview` and
//! `definition_apply`, registered next to the guidance and wrong-root
//! commands. Each is a thin wrapper over a pure body tested here without a
//! Tauri runtime, composing the application services
//! (`omnifrons_app::definition`) with the real filesystem adapters: a
//! no-follow directory walk rooted at the active workspace, and SHA-256
//! over identity file bytes.
//!
//! Status, verify and preview read; none writes, locks the publication
//! surface, or is frozen while a run is active -- the prototype's own
//! `scan` and `verify_agent` are themselves read-only, and the ports they
//! compose over (`omnifrons_app::definition::{scan, verify}`,
//! `omnifrons_app::definition_plan::plan`) never touch a filesystem
//! except through the adapters they are given.
//!
//! `definition_apply` writes: it takes the surface lock and refuses with
//! `run-active` while a run is live, like the guidance installer. It
//! takes the plan id a preview returned and nothing else the caller
//! could forge: the application plans again from the disk and refuses
//! with `plan-stale` when the id no longer matches.
//!
//! `definition_verify` takes an optional agent directory below the
//! workspace (the prototype's own `verify --agent <path>`), defaulting
//! to the workspace root; it reports no pending snapshots (`backups`),
//! which belong to the write path.

use omnifrons_adapters::{
    FsApplyRecordStore, FsDefinitionTree, FsDefinitionWriter, FsSnapshotStore, Sha2Hasher,
};
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::definition::{DefinitionError, DefinitionPorts, scan, verify};
use omnifrons_app::definition_apply::{ApplyPorts, apply, apply_create};
use omnifrons_app::definition_plan::{plan, plan_create_preview};
use omnifrons_app::switch_check::switch_check;
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::adapter::AdapterId;
use omnifrons_domain::definition::IdentityKind;
use omnifrons_domain::definition_create::{Answers, RawValue};
use omnifrons_domain::definition_plan::{PlanId, PlanOptions};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::nesting;
use omnifrons_supervisor::TokioProcessSupervisor;
use tauri::{AppHandle, Manager as _};

use crate::executable_state::SystemClock;
use crate::ipc::commands::active_workspace;
use crate::ipc::dto::{
    DefinitionApplyDto, DefinitionCreateApplyDto, DefinitionCreatePreviewDto, DefinitionFieldDto,
    DefinitionPreviewDto, DefinitionStatusDto, DefinitionSwitchCheckDto, DefinitionVerifyDto,
    ShellError, ShellErrorCode, ShellErrorDetail,
};
use crate::ipc::publication::{RunActivity, run_active};
use crate::publication_state::PublicationState;

impl From<DefinitionError> for ShellError {
    /// Maps every scan/verify failure to its closed code with a fixed,
    /// path-free message; `detail` always carries the wrapped error's own
    /// fixed reason token (never a path).
    ///
    /// | `DefinitionError` variant | code | example `reason` |
    /// |---|---|---|
    /// | `Tree` | `scan-failed` | `tree(root-unreadable)` |
    /// | `Nesting` | `definition-invalid` | `nesting(duplicate-root)` |
    /// | `DuplicateIdentityKind` | `definition-invalid` | `duplicate-identity-kind` |
    /// | `DuplicateSnapshotPath` | `definition-invalid` | `duplicate-snapshot-path` |
    /// | `InvalidCanonicalChoice` | `invalid-request` | `invalid-canonical-choice` |
    /// | `StalePlan` | `plan-stale` | `stale-plan` |
    /// | `Write` | `definition-invalid` | the write error's own token |
    /// | `Snapshot`, `Record` | `snapshot-unavailable` | the store error's own token |
    fn from(error: DefinitionError) -> Self {
        let reason = error.reason().to_string();
        let code = match &error {
            DefinitionError::Tree(_) => ShellErrorCode::ScanFailed,
            DefinitionError::InvalidCanonicalChoice { .. } => ShellErrorCode::InvalidRequest,
            DefinitionError::StalePlan { .. } => ShellErrorCode::PlanStale,
            DefinitionError::Nesting(_)
            | DefinitionError::DuplicateIdentityKind(_)
            | DefinitionError::DuplicateSnapshotPath { .. }
            | DefinitionError::Write(_) => ShellErrorCode::DefinitionInvalid,
            DefinitionError::Snapshot(_) | DefinitionError::Record(_) => {
                ShellErrorCode::SnapshotUnavailable
            }
        };
        let message = match &error {
            DefinitionError::Tree(_) => "the workspace could not be scanned",
            DefinitionError::Nesting(_) => {
                "the discovered agent roots could not be related to each other"
            }
            DefinitionError::DuplicateIdentityKind(_) => {
                "a directory reported the same identity file more than once"
            }
            DefinitionError::DuplicateSnapshotPath { .. } => {
                "the scan reported the same path more than once"
            }
            DefinitionError::InvalidCanonicalChoice { .. } => {
                "the canonical file choice does not apply to this definition"
            }
            DefinitionError::StalePlan { .. } => "the plan no longer matches the definition",
            DefinitionError::Write(_) => "a definition file could not be written",
            DefinitionError::Snapshot(_) | DefinitionError::Record(_) => {
                "the apply could not be recorded in the work area"
            }
        };
        Self {
            code,
            message: message.to_string(),
            detail: Some(ShellErrorDetail::DefinitionReason { reason }),
        }
    }
}

/// A scan of `workspace`'s agent definitions: every discovered agent
/// root, the agent roots above it, and each root's identity files,
/// skills-index status, and warnings.
///
/// # Errors
///
/// `scan-failed` if the workspace could not be walked at all;
/// `definition-invalid` if the walk's own result is internally
/// inconsistent (unreachable with the real filesystem adapter; see
/// [`ShellErrorCode::DefinitionInvalid`]).
pub fn definition_status_for(workspace: &WorkspaceRoot) -> Result<DefinitionStatusDto, ShellError> {
    let tree = FsDefinitionTree::from_environment();
    let hasher = Sha2Hasher::new();
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };
    let report = scan(&ports, workspace)?;
    Ok(DefinitionStatusDto::from_report(workspace, &report))
}

/// Whether the host a model switch to `adapter_id` would run reaches the
/// canonical definition from `workspace`. `host` is the loader-table host
/// the user chose for an adapter bound to none; it is ignored for an
/// adapter that has its own.
///
/// # Errors
///
/// `invalid-request` if `adapter_id` names no adapter; otherwise as
/// [`definition_status_for`].
pub fn definition_switch_check_for(
    workspace: &WorkspaceRoot,
    adapter_id: &str,
    host: Option<&str>,
) -> Result<DefinitionSwitchCheckDto, ShellError> {
    let adapter = AdapterId::parse(adapter_id).ok_or_else(|| {
        ShellError::new(ShellErrorCode::InvalidRequest, "the adapter is not known")
    })?;
    let tree = FsDefinitionTree::from_environment();
    let hasher = Sha2Hasher::new();
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };
    let check = switch_check(&ports, workspace, &adapter, host)?;
    Ok(DefinitionSwitchCheckDto::from_check(&adapter, &check))
}

/// The fixed refusal for a verify target that is not a plain directory
/// of the workspace; never the target itself.
fn not_a_workspace_directory() -> ShellError {
    ShellError::new(
        ShellErrorCode::InvalidRequest,
        "the agent is not a workspace directory",
    )
}

/// Refuses a verify `target` that is not a clean relative path naming an
/// existing directory under `workspace` with no symlink in any
/// component (the prototype's own `require_directory`, which refuses a
/// symlink or a non-directory as a usage error). Each prefix of the
/// target is checked in turn, so a symlinked intermediate component is
/// refused just like a symlinked final one. The path syntax is checked
/// first, with the same domain rules the application service applies,
/// so the service's own target error is unreachable from the command.
///
/// The target is a workspace-relative POSIX path whose components are
/// plain names: non-empty, not `.` or `..`, free of NUL, and on Windows
/// free of `\` and `:` as well. That is checked before the filesystem
/// is touched, so a native path syntax (a drive prefix, a rooted or
/// backslash-separated component) can never reach the join below and
/// escape or replace the workspace prefix there. On Unix those two are
/// ordinary name characters, so a directory the scan lists under such
/// a name verifies like any other.
fn require_workspace_directory(workspace: &WorkspaceRoot, target: &str) -> Result<(), ShellError> {
    nesting::relate(&[target], &[]).map_err(|_| not_a_workspace_directory())?;
    if target == "." {
        return Ok(());
    }
    if !target.split('/').all(is_plain_name) {
        return Err(not_a_workspace_directory());
    }
    let mut prefix = workspace.path().to_path_buf();
    for component in target.split('/') {
        prefix.push(component);
        let metadata =
            std::fs::symlink_metadata(&prefix).map_err(|_| not_a_workspace_directory())?;
        if !metadata.is_dir() {
            return Err(not_a_workspace_directory());
        }
    }
    Ok(())
}

/// Whether `component` is a plain directory name: non-empty, not `.` or
/// `..`, carrying no NUL, and on Windows no backslash or colon either,
/// which a native path there reads as a separator or a drive prefix.
fn is_plain_name(component: &str) -> bool {
    !component.is_empty()
        && component != "."
        && component != ".."
        && !component.contains('\0')
        && (!cfg!(windows) || !component.contains(['\\', ':']))
}

/// A verification of one agent directory's canonical definition: its
/// classification state, whether it is canonical, and every problem
/// found. `agent` is a clean relative path below `workspace` (`None`
/// is the workspace root itself).
///
/// # Errors
///
/// `invalid-request` if `agent` is not an existing, non-symlink
/// directory under `workspace`; otherwise as [`definition_status_for`].
pub fn definition_verify_for(
    workspace: &WorkspaceRoot,
    agent: Option<&str>,
) -> Result<DefinitionVerifyDto, ShellError> {
    let target = agent.unwrap_or(".");
    require_workspace_directory(workspace, target)?;
    let tree = FsDefinitionTree::from_environment();
    let hasher = Sha2Hasher::new();
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };
    let report = verify(&ports, workspace, target)?;
    Ok(DefinitionVerifyDto::from_report(workspace, target, &report))
}

/// What `definition_preview` and `definition_apply` plan: the agent
/// directory below the workspace (`None` is the workspace root) and the
/// flags that change what a plan may do.
#[derive(Debug, Clone, Default)]
pub struct DefinitionPlanRequest {
    /// A clean relative directory below the workspace.
    pub agent: Option<String>,
    /// The agent's own consent to be adapted when it sits below another.
    pub nested_consent: bool,
    /// `"CLAUDE.md"` or `"AGENTS.md"`: the winning side of a conflict.
    pub canonical_from: Option<String>,
    /// Also write the `GEMINI.md` wrapper.
    pub with_gemini: bool,
}

impl DefinitionPlanRequest {
    /// The checked target and plan options.
    ///
    /// # Errors
    ///
    /// `invalid-request` for an agent that is not a plain directory of
    /// the workspace, or a canonical choice that is neither `CLAUDE.md`
    /// nor `AGENTS.md`.
    fn resolve(&self, workspace: &WorkspaceRoot) -> Result<(String, PlanOptions), ShellError> {
        let target = self.agent.as_deref().unwrap_or(".").to_string();
        require_workspace_directory(workspace, &target)?;
        let canonical_from = match self.canonical_from.as_deref() {
            None => None,
            Some("CLAUDE.md") => Some(IdentityKind::Claude),
            Some("AGENTS.md") => Some(IdentityKind::Agents),
            Some(_) => {
                return Err(ShellError::new(
                    ShellErrorCode::InvalidRequest,
                    "the canonical file must be CLAUDE.md or AGENTS.md",
                ));
            }
        };
        Ok((
            target,
            PlanOptions {
                nested_consent: self.nested_consent,
                canonical_from,
                with_gemini: self.with_gemini,
            },
        ))
    }
}

/// Parses a plan id: exactly 64 lowercase hex characters.
fn parse_plan_id(text: &str) -> Result<PlanId, ShellError> {
    let lowercase_hex = text
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    Sha256Digest::from_hex(text)
        .filter(|_| lowercase_hex)
        .map(PlanId::from_digest)
        .ok_or_else(|| {
            ShellError::new(
                ShellErrorCode::InvalidRequest,
                "the plan id is not 64 lowercase hex characters",
            )
        })
}

/// The proposal (`definition_preview`): the operations an apply would
/// run, the block diff, and the id the apply must name. Writes nothing.
///
/// # Errors
///
/// `invalid-request` for an agent that is not an existing, non-symlink
/// directory under `workspace` or a canonical choice other than
/// `CLAUDE.md` or `AGENTS.md`; otherwise as [`definition_verify_for`].
/// A root the plan cannot adapt is an outcome (`refused`), not an error.
pub fn definition_preview_for(
    workspace: &WorkspaceRoot,
    request: &DefinitionPlanRequest,
) -> Result<DefinitionPreviewDto, ShellError> {
    let (target, options) = request.resolve(workspace)?;
    let tree = FsDefinitionTree::from_environment();
    let hasher = Sha2Hasher::new();
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };
    let outcome = plan(&ports, workspace, Some(&target), &options)?;
    Ok(DefinitionPreviewDto::from_outcome(
        workspace, &target, &outcome,
    ))
}

/// The approved write (`definition_apply`), bound to `plan_id`: the
/// application plans again from the disk and writes only when the id
/// still matches.
///
/// # Errors
///
/// `run-active` while a run is live; `invalid-request` for a malformed
/// plan id, and [`definition_preview_for`]'s request errors;
/// `plan-stale` when the id no longer matches the definition;
/// `work-area-invalid` and `snapshot-unavailable` for the work area;
/// `definition-invalid` for a write that fails. A root the plan cannot
/// adapt is an outcome (`refused`), not an error.
pub fn definition_apply_for(
    workspace: &WorkspaceRoot,
    publication: &PublicationState,
    run_activity: &dyn RunActivity,
    request: &DefinitionPlanRequest,
    plan_id: &str,
) -> Result<DefinitionApplyDto, ShellError> {
    let _surface = publication.lock_surface();
    if run_activity.any_running() {
        return Err(run_active());
    }
    let (target, options) = request.resolve(workspace)?;
    let expected = parse_plan_id(plan_id)?;
    let work_area = WorkAreaRoot::open(&publication.work_area, &[workspace])?;
    let tree = FsDefinitionTree::from_environment();
    let hasher = Sha2Hasher::new();
    let writer = FsDefinitionWriter::new();
    let mut snapshots = FsSnapshotStore::open(&work_area);
    let records = FsApplyRecordStore::open(&work_area);
    let clock = SystemClock;
    let mut ports = ApplyPorts {
        tree: &tree,
        hasher: &hasher,
        writer: &writer,
        snapshots: &mut snapshots,
        records: &records,
        clock: &clock,
    };
    let outcome = apply(&mut ports, workspace, Some(&target), &options, &expected)?;
    Ok(DefinitionApplyDto::from_outcome(
        workspace, &target, &outcome,
    ))
}

/// What `definition_create_preview` and `definition_create_apply` plan:
/// the new agent's directory below the workspace (`None` is the workspace
/// root) and the flags that change what a create may do.
#[derive(Debug, Clone, Default)]
pub struct DefinitionCreateRequest {
    /// A clean relative directory below the workspace.
    pub agent: Option<String>,
    /// The agent's own consent to be created when it sits below another.
    pub nested_consent: bool,
    /// Also write the `GEMINI.md` wrapper.
    pub with_gemini: bool,
}

impl DefinitionCreateRequest {
    /// The checked target and plan options.
    ///
    /// # Errors
    ///
    /// `invalid-request` for an agent that is not a plain directory of
    /// the workspace.
    fn resolve(&self, workspace: &WorkspaceRoot) -> Result<(String, PlanOptions), ShellError> {
        let target = self.agent.as_deref().unwrap_or(".").to_string();
        require_workspace_directory(workspace, &target)?;
        Ok((
            target,
            PlanOptions {
                nested_consent: self.nested_consent,
                canonical_from: None,
                with_gemini: self.with_gemini,
            },
        ))
    }
}

/// The loosely typed document `Answers::validate` reads, from the JSON
/// the form sent: an object's entries are converted in the order the JSON
/// value iterates them (key order with this workspace's `serde_json`,
/// which does not preserve the document's order), and a number or a
/// boolean is `Other`. Whether the answers are valid does not depend on
/// that order.
fn raw_answers(value: &serde_json::Value) -> RawValue {
    match value {
        serde_json::Value::Null => RawValue::Null,
        serde_json::Value::String(text) => RawValue::Text(text.clone()),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => RawValue::Other,
        serde_json::Value::Array(items) => RawValue::List(items.iter().map(raw_answers).collect()),
        serde_json::Value::Object(pairs) => RawValue::Object(
            pairs
                .iter()
                .map(|(key, value)| (key.clone(), raw_answers(value)))
                .collect(),
        ),
    }
}

/// The create interview's fields, in interview order, so the form does
/// not hardcode them.
#[tauri::command]
#[must_use]
pub fn definition_create_fields() -> Vec<DefinitionFieldDto> {
    DefinitionFieldDto::catalogue()
}

/// The proposal for a new definition (`definition_create_preview`): the
/// operations a create apply would run, the user's own words beside each
/// generated line, and the id the apply must name. Writes nothing and
/// keeps nothing of the answers.
///
/// # Errors
///
/// `invalid-request` for an agent that is not an existing, non-symlink
/// directory under `workspace`; otherwise as [`definition_verify_for`].
/// Answers that do not validate are an outcome (`invalid`), and so is a
/// target the plan cannot create in (`refused`).
pub fn definition_create_preview_for(
    workspace: &WorkspaceRoot,
    request: &DefinitionCreateRequest,
    answers: &serde_json::Value,
) -> Result<DefinitionCreatePreviewDto, ShellError> {
    let (target, options) = request.resolve(workspace)?;
    let answers = match Answers::validate(&raw_answers(answers)) {
        Ok(answers) => answers,
        Err(problems) => {
            return Ok(DefinitionCreatePreviewDto::invalid(
                workspace, &target, &problems,
            ));
        }
    };
    let tree = FsDefinitionTree::from_environment();
    let hasher = Sha2Hasher::new();
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };
    let (outcome, provenance) =
        plan_create_preview(&ports, workspace, Some(&target), &options, &answers)?;
    Ok(DefinitionCreatePreviewDto::from_outcome(
        workspace,
        &target,
        &outcome,
        &provenance,
    ))
}

/// The approved create (`definition_create_apply`), bound to `plan_id`:
/// the application plans again from the disk and the answers, and writes
/// only when the id still matches.
///
/// # Errors
///
/// `run-active` while a run is live; `invalid-request` for a malformed
/// plan id, and [`definition_create_preview_for`]'s request errors;
/// `plan-stale` when the id no longer matches; `work-area-invalid` and
/// `snapshot-unavailable` for the work area; `definition-invalid` for a
/// write that fails. Invalid answers (`invalid`) and a target the plan
/// cannot create in (`refused`) are outcomes.
pub fn definition_create_apply_for(
    workspace: &WorkspaceRoot,
    publication: &PublicationState,
    run_activity: &dyn RunActivity,
    request: &DefinitionCreateRequest,
    answers: &serde_json::Value,
    plan_id: &str,
) -> Result<DefinitionCreateApplyDto, ShellError> {
    let _surface = publication.lock_surface();
    if run_activity.any_running() {
        return Err(run_active());
    }
    let (target, options) = request.resolve(workspace)?;
    let expected = parse_plan_id(plan_id)?;
    let answers = match Answers::validate(&raw_answers(answers)) {
        Ok(answers) => answers,
        Err(problems) => {
            return Ok(DefinitionCreateApplyDto::invalid(
                workspace, &target, &problems,
            ));
        }
    };
    let work_area = WorkAreaRoot::open(&publication.work_area, &[workspace])?;
    let tree = FsDefinitionTree::from_environment();
    let hasher = Sha2Hasher::new();
    let writer = FsDefinitionWriter::new();
    let mut snapshots = FsSnapshotStore::open(&work_area);
    let records = FsApplyRecordStore::open(&work_area);
    let clock = SystemClock;
    let mut ports = ApplyPorts {
        tree: &tree,
        hasher: &hasher,
        writer: &writer,
        snapshots: &mut snapshots,
        records: &records,
        clock: &clock,
    };
    let outcome = apply_create(
        &mut ports,
        workspace,
        Some(&target),
        &options,
        &answers,
        &expected,
    )?;
    Ok(DefinitionCreateApplyDto::from_outcome(
        workspace, &target, &outcome,
    ))
}

/// The active workspace's agent-definition scan.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`definition_status_for`]'s errors.
#[tauri::command]
pub async fn definition_status(app: AppHandle) -> Result<DefinitionStatusDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || definition_status_for(&workspace))
        .await
        .expect("the blocking definition-status task panicked")
}

/// Whether the named adapter's host reaches the active workspace's
/// canonical definition, before a model switch restarts it.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`definition_switch_check_for`]'s errors.
#[tauri::command]
pub async fn definition_switch_check(
    app: AppHandle,
    adapter_id: String,
    host: Option<String>,
) -> Result<DefinitionSwitchCheckDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        definition_switch_check_for(&workspace, &adapter_id, host.as_deref())
    })
    .await
    .expect("the blocking definition-switch-check task panicked")
}

/// The canonical-definition verification of the active workspace's
/// root, or of `agent`, a directory below it.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`definition_verify_for`]'s errors.
#[tauri::command]
pub async fn definition_verify(
    app: AppHandle,
    agent: Option<String>,
) -> Result<DefinitionVerifyDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        definition_verify_for(&workspace, agent.as_deref())
    })
    .await
    .expect("the blocking definition-verify task panicked")
}

/// The proposal for the active workspace's root, or for `agent`, a
/// directory below it: what `definition_apply` would write.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`definition_preview_for`]'s errors.
#[tauri::command]
pub async fn definition_preview(
    app: AppHandle,
    agent: Option<String>,
    nested_consent: bool,
    canonical_from: Option<String>,
    with_gemini: bool,
) -> Result<DefinitionPreviewDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        definition_preview_for(
            &workspace,
            &DefinitionPlanRequest {
                agent,
                nested_consent,
                canonical_from,
                with_gemini,
            },
        )
    })
    .await
    .expect("the blocking definition-preview task panicked")
}

/// The approved write, bound to `planId`.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`definition_apply_for`]'s errors.
#[tauri::command]
pub async fn definition_apply(
    app: AppHandle,
    agent: Option<String>,
    nested_consent: bool,
    canonical_from: Option<String>,
    with_gemini: bool,
    plan_id: String,
) -> Result<DefinitionApplyDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let publication = app.state::<PublicationState>();
        let supervisor = app.state::<TokioProcessSupervisor>().inner().clone();
        definition_apply_for(
            &workspace,
            &publication,
            &supervisor,
            &DefinitionPlanRequest {
                agent,
                nested_consent,
                canonical_from,
                with_gemini,
            },
            &plan_id,
        )
    })
    .await
    .expect("the blocking definition-apply task panicked")
}

/// The proposal for a new agent in the active workspace's root, or in
/// `agent`, a directory below it. The answers are validated, used for the
/// plan and dropped: the shell stores and logs none of them.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`definition_create_preview_for`]'s errors.
#[tauri::command]
pub async fn definition_create_preview(
    app: AppHandle,
    agent: Option<String>,
    nested_consent: bool,
    with_gemini: bool,
    answers: serde_json::Value,
) -> Result<DefinitionCreatePreviewDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        definition_create_preview_for(
            &workspace,
            &DefinitionCreateRequest {
                agent,
                nested_consent,
                with_gemini,
            },
            &answers,
        )
    })
    .await
    .expect("the blocking definition-create-preview task panicked")
}

/// The approved create, bound to `planId`.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`definition_create_apply_for`]'s errors.
#[tauri::command]
pub async fn definition_create_apply(
    app: AppHandle,
    agent: Option<String>,
    nested_consent: bool,
    with_gemini: bool,
    answers: serde_json::Value,
    plan_id: String,
) -> Result<DefinitionCreateApplyDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let publication = app.state::<PublicationState>();
        let supervisor = app.state::<TokioProcessSupervisor>().inner().clone();
        definition_create_apply_for(
            &workspace,
            &publication,
            &supervisor,
            &DefinitionCreateRequest {
                agent,
                nested_consent,
                with_gemini,
            },
            &answers,
            &plan_id,
        )
    })
    .await
    .expect("the blocking definition-create-apply task panicked")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use omnifrons_app::WorkspaceRoot;
    use omnifrons_app::apply_record::ApplyRecordError;
    use omnifrons_app::definition::DefinitionError;
    use omnifrons_app::definition_tree::DefinitionTreeError;
    use omnifrons_app::definition_writer::DefinitionWriteError;
    use omnifrons_app::snapshot_store::SnapshotStoreError;
    use omnifrons_domain::definition::{DuplicateIdentityKindError, IdentityKind};
    use omnifrons_domain::definition_create::{Answers, CreateContext, render};
    use omnifrons_domain::definition_plan::PlanId;
    use omnifrons_domain::nesting::NestingError;

    use super::{
        DefinitionCreateRequest, DefinitionPlanRequest, definition_apply_for,
        definition_create_apply_for, definition_create_preview_for, definition_preview_for,
        definition_status_for, definition_switch_check_for, definition_verify_for,
    };
    use crate::ipc::dto::{ShellError, ShellErrorCode, ShellErrorDetail};
    use crate::ipc::publication::RunActivity;
    use crate::publication_state::PublicationState;

    /// A fixture agent body: content-bearing, never import-only.
    const AGENT_BODY: &[u8] = b"# Fixture\n\nYou are Fixture, a test agent.\n";

    /// A drop-guard temp directory (mirrors every other `*_fs.rs`/IPC
    /// fixture in this workspace).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "omnifrons-shell-definition-test-{}-{label}-{n}",
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

    /// Writes `bytes` at `base`/`relative`, creating every parent
    /// directory first.
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
    /// the filesystem adapter itself reports for an ancestor's own path
    /// (`crates/omnifrons-adapters/tests/definition_tree_fs.rs`'s own
    /// `canonical_posix`, reused here so a host's own real ancestry above
    /// the OS temp directory can never leak into an assertion).
    fn canonical_posix(path: &Path) -> String {
        std::fs::canonicalize(path)
            .expect("canonicalize")
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// Every `DefinitionError` variant maps to its documented code, and
    /// `detail` always carries the wrapped error's own fixed reason
    /// token. `DuplicateIdentityKind` and `DuplicateSnapshotPath` are
    /// unreachable through the real filesystem adapter (a directory
    /// cannot hold two files of the same name), so this is the only
    /// practical way to exercise their mapping.
    #[test]
    fn definition_error_maps_to_shell_error_by_variant() {
        let cases: [(DefinitionError, ShellErrorCode, &str); 9] = [
            (
                DefinitionError::Tree(DefinitionTreeError::RootUnreadable),
                ShellErrorCode::ScanFailed,
                "tree(root-unreadable)",
            ),
            (
                DefinitionError::Nesting(NestingError::EmptyPath),
                ShellErrorCode::DefinitionInvalid,
                "nesting(empty-path)",
            ),
            (
                DefinitionError::DuplicateIdentityKind(DuplicateIdentityKindError {
                    kind: IdentityKind::Agents,
                }),
                ShellErrorCode::DefinitionInvalid,
                "duplicate-identity-kind",
            ),
            (
                DefinitionError::DuplicateSnapshotPath {
                    list: "directory",
                    path: "nested".to_string(),
                },
                ShellErrorCode::DefinitionInvalid,
                "duplicate-snapshot-path",
            ),
            (
                DefinitionError::InvalidCanonicalChoice {
                    detail: "--canonical-from only resolves a conflict".to_string(),
                },
                ShellErrorCode::InvalidRequest,
                "invalid-canonical-choice",
            ),
            (
                DefinitionError::StalePlan {
                    expected: PlanId([1; 32]),
                    actual: Some(PlanId([2; 32])),
                },
                ShellErrorCode::PlanStale,
                "stale-plan",
            ),
            (
                DefinitionError::Write(DefinitionWriteError::ParentMissing {
                    path: "child/AGENTS.md".to_string(),
                }),
                ShellErrorCode::DefinitionInvalid,
                "parent-missing",
            ),
            (
                DefinitionError::Snapshot(SnapshotStoreError::WriteFailed),
                ShellErrorCode::SnapshotUnavailable,
                "snapshot-write-failed",
            ),
            (
                DefinitionError::Record(ApplyRecordError::WriteFailed),
                ShellErrorCode::SnapshotUnavailable,
                "apply-record-write-failed",
            ),
        ];
        for (error, code, reason) in cases {
            let shell_error: ShellError = error.into();
            assert_eq!(shell_error.code, code, "{reason}");
            assert_eq!(
                shell_error.detail,
                Some(ShellErrorDetail::DefinitionReason {
                    reason: reason.to_string()
                }),
                "{reason}"
            );
            assert!(
                !shell_error.message.is_empty(),
                "every mapped error has a fixed message"
            );
        }
    }

    /// A root `AGENTS.md`, a nested agent, a `skills/x/SKILL.md`
    /// discovered skill, and an outside ancestor, all in the shape the
    /// app's own
    /// `nested_boundaries_report_parent_nested_consent_and_skill_ownership`
    /// and `ancestors_outside_the_scan_root_are_recorded_nearest_first`
    /// fixtures (`crates/omnifrons-adapters/tests/definition_tree_fs.rs`)
    /// establish separately: parent/nested/consent, skill ownership, and
    /// the nearest outside ancestor, through this command's own DTO.
    #[test]
    fn definition_status_for_reports_agents_ancestors_and_skills() {
        let outer = TempDir::new("status-outer");
        write(outer.path(), "AGENTS.md", AGENT_BODY);
        let project = outer.path().join("workspace");
        write(&project, "AGENTS.md", AGENT_BODY);
        write(&project, "skills/x/SKILL.md", b"---\nname: x\n---\n");
        write(&project, "nested-agent/AGENTS.md", AGENT_BODY);
        let workspace = WorkspaceRoot::new(&project).expect("valid workspace");
        let outer_posix = canonical_posix(outer.path());

        let status = definition_status_for(&workspace).expect("scan succeeds");

        assert_eq!(status.schema, "omnifrons.definition-scan/1");
        assert_eq!(status.root, workspace.path().to_string_lossy());
        let paths: Vec<&str> = status
            .agents
            .iter()
            .map(|agent| agent.path.as_str())
            .collect();
        assert_eq!(
            paths,
            vec![".", "nested-agent"],
            "walk order: parent before nested child"
        );
        let root_agent = &status.agents[0];
        assert_eq!(root_agent.nested, vec!["nested-agent".to_string()]);
        // The outer fixture directory itself carries an `AGENTS.md`, so
        // it is the scan root's own nearest identity-bearing outside
        // ancestor (`Parent::Outside`), exactly as
        // `ancestors_outside_the_scan_root_are_recorded_nearest_first`
        // establishes at the adapter level: needing consent is a
        // property of having any parent at all, inside the scan or out.
        assert_eq!(
            root_agent.parent,
            Some(crate::ipc::dto::ParentDto::Outside {
                path: outer_posix.clone()
            })
        );
        assert!(root_agent.consent_required);
        assert_eq!(
            root_agent.skills.discovered,
            vec!["skills/x/SKILL.md".to_string()]
        );
        let nested = &status.agents[1];
        assert_eq!(
            nested.parent,
            Some(crate::ipc::dto::ParentDto::Root {
                path: ".".to_string()
            })
        );
        assert!(nested.consent_required);
        let ancestors: Vec<_> = status
            .ancestors
            .iter()
            .filter(|ancestor| ancestor.path.starts_with(&outer_posix))
            .collect();
        assert_eq!(
            ancestors.len(),
            1,
            "exactly the outer fixture directory: {:?}",
            status.ancestors
        );
        assert_eq!(ancestors[0].path, outer_posix);
        assert_eq!(ancestors[0].identity_files, vec!["AGENTS.md".to_string()]);
    }

    /// A `CLAUDE.md` wrapper importing a content-bearing `AGENTS.md` is
    /// the steady state after migration: canonical, no problems.
    #[test]
    fn definition_verify_for_reports_canonical_for_a_wrapped_root() {
        let project = TempDir::new("verify-canonical");
        write(project.path(), "AGENTS.md", AGENT_BODY);
        write(project.path(), "CLAUDE.md", b"@AGENTS.md\n");
        let workspace = project.workspace();

        let verify = definition_verify_for(&workspace, None).expect("verify succeeds");

        assert_eq!(verify.schema, "omnifrons.definition-verify/1");
        assert_eq!(verify.agent, workspace.path().to_string_lossy());
        assert_eq!(verify.state, crate::ipc::dto::RootStateTag::Canonical);
        assert!(verify.canonical, "{:?}", verify.problems);
        assert!(verify.problems.is_empty());
    }

    /// No identity file at all: the prototype's own `"state is none"`
    /// wording (app's own
    /// `verify_reports_none_state_for_an_extras_only_target`-style
    /// assertion, translated through this command's DTO).
    #[test]
    fn definition_verify_for_reports_none_state_with_its_problem() {
        let project = TempDir::new("verify-none");
        let workspace = project.workspace();

        let verify = definition_verify_for(&workspace, None).expect("verify succeeds");

        assert_eq!(verify.state, crate::ipc::dto::RootStateTag::None);
        assert!(!verify.canonical, "{:?}", verify.problems);
        assert_eq!(verify.problems, vec!["state is none".to_string()]);
    }

    /// A nested canonical agent verifies as itself, and the payload's
    /// `agent` is the workspace path with the target pushed on component by component.
    #[test]
    fn definition_verify_for_verifies_a_nested_agent() {
        let project = TempDir::new("verify-nested");
        write(project.path(), "AGENTS.md", AGENT_BODY);
        write(project.path(), "child/AGENTS.md", AGENT_BODY);
        write(project.path(), "child/CLAUDE.md", b"@AGENTS.md\n");
        let workspace = project.workspace();

        let verify = definition_verify_for(&workspace, Some("child")).expect("verify succeeds");

        assert_eq!(verify.state, crate::ipc::dto::RootStateTag::Canonical);
        assert!(verify.canonical, "{:?}", verify.problems);
        let mut expected = workspace.path().to_path_buf();
        expected.push("child");
        assert_eq!(verify.agent, expected.to_string_lossy());
        let root = definition_verify_for(&workspace, Some(".")).expect("verify succeeds");
        assert_eq!(root.agent, workspace.path().to_string_lossy());
    }

    fn assert_invalid_request(result: Result<crate::ipc::dto::DefinitionVerifyDto, ShellError>) {
        let error = result.expect_err("the target is refused");
        assert_eq!(error.code, ShellErrorCode::InvalidRequest);
        assert_eq!(error.message, "the agent is not a workspace directory");
    }

    /// A target that does not exist is refused before any scan.
    #[test]
    fn definition_verify_for_refuses_a_missing_target() {
        let project = TempDir::new("verify-missing");
        write(project.path(), "AGENTS.md", AGENT_BODY);
        let workspace = project.workspace();

        assert_invalid_request(definition_verify_for(&workspace, Some("nowhere")));
    }

    /// A regular file is not an agent directory.
    #[test]
    fn definition_verify_for_refuses_a_file_target() {
        let project = TempDir::new("verify-file");
        write(project.path(), "AGENTS.md", AGENT_BODY);
        let workspace = project.workspace();

        assert_invalid_request(definition_verify_for(&workspace, Some("AGENTS.md")));
    }

    /// A symlink to a directory is refused, not followed.
    #[cfg(unix)]
    #[test]
    fn definition_verify_for_refuses_a_symlinked_directory_target() {
        let project = TempDir::new("verify-symlink");
        write(project.path(), "real/AGENTS.md", AGENT_BODY);
        std::os::unix::fs::symlink(project.path().join("real"), project.path().join("link"))
            .expect("create the symlink");
        let workspace = project.workspace();

        assert_invalid_request(definition_verify_for(&workspace, Some("link")));
    }

    /// A symlinked intermediate component is refused too, even when the
    /// final component is a real directory outside the workspace.
    #[cfg(unix)]
    #[test]
    fn definition_verify_for_refuses_a_symlinked_intermediate_component() {
        let project = TempDir::new("verify-symlink-mid");
        let outside = TempDir::new("verify-symlink-mid-outside");
        write(outside.path(), "sub/AGENTS.md", AGENT_BODY);
        write(project.path(), "AGENTS.md", AGENT_BODY);
        std::os::unix::fs::symlink(outside.path(), project.path().join("link"))
            .expect("create the symlink");
        let workspace = project.workspace();

        assert_invalid_request(definition_verify_for(&workspace, Some("link/sub")));
    }

    /// A plain nested directory several levels deep still verifies.
    #[test]
    fn definition_verify_for_verifies_a_deeply_nested_agent() {
        let project = TempDir::new("verify-deep");
        write(project.path(), "AGENTS.md", AGENT_BODY);
        write(project.path(), "a/b/AGENTS.md", AGENT_BODY);
        write(project.path(), "a/b/CLAUDE.md", b"@AGENTS.md\n");
        let workspace = project.workspace();

        let verify = definition_verify_for(&workspace, Some("a/b")).expect("verify succeeds");

        assert!(verify.canonical, "{:?}", verify.problems);
        let mut expected = workspace.path().to_path_buf();
        expected.push("a");
        expected.push("b");
        assert_eq!(verify.agent, expected.to_string_lossy());
    }

    /// An unclean or escaping target is refused before it touches the
    /// filesystem.
    #[test]
    fn definition_verify_for_refuses_an_unclean_target() {
        let project = TempDir::new("verify-unclean");
        write(project.path(), "child/AGENTS.md", AGENT_BODY);
        let workspace = project.workspace();

        for target in ["../x", "/abs", "a//b", "./child", "child/", "", "child/./x"] {
            assert_invalid_request(definition_verify_for(&workspace, Some(target)));
        }
    }

    /// Targets whose components carry a native path syntax (a backslash
    /// or a drive colon) are refused on every platform: on Windows before
    /// any filesystem call, on Unix because no directory of that name
    /// exists here.
    #[test]
    fn definition_verify_for_refuses_a_native_path_syntax_target() {
        let project = TempDir::new("verify-native-syntax");
        write(project.path(), "AGENTS.md", AGENT_BODY);
        let workspace = project.workspace();

        for target in [
            "a\\..\\b", "..\\x", "C:", "C:\\x", "\\abs", "a:b", "ok/C:", "ok/a\\b",
        ] {
            assert_invalid_request(definition_verify_for(&workspace, Some(target)));
        }
    }

    /// On Unix a backslash or colon is an ordinary name character, so a
    /// directory literally named like that exists, the scan lists it,
    /// and its row verifies like any other: the name is one component
    /// that never reaches a native path parser.
    #[cfg(unix)]
    #[test]
    fn definition_verify_for_verifies_an_existing_directory_named_with_native_syntax() {
        let project = TempDir::new("verify-native-existing");
        write(project.path(), "AGENTS.md", AGENT_BODY);
        for name in ["a\\..\\b", "..\\x", "C:", "C:\\x", "\\abs", "a:b"] {
            write(project.path(), &format!("{name}/AGENTS.md"), AGENT_BODY);
        }
        let workspace = project.workspace();

        for target in ["a\\..\\b", "..\\x", "C:", "C:\\x", "\\abs", "a:b"] {
            let verify = definition_verify_for(&workspace, Some(target)).expect("verify succeeds");
            assert_eq!(
                verify.state,
                crate::ipc::dto::RootStateTag::AgentsOnly,
                "{target}"
            );
            let mut expected = workspace.path().to_path_buf();
            expected.push(target);
            assert_eq!(verify.agent, expected.to_string_lossy());
        }
    }

    /// No supervised process is running.
    struct Idle;

    impl RunActivity for Idle {
        fn any_running(&self) -> bool {
            false
        }
    }

    /// A supervised process is running.
    struct Busy;

    impl RunActivity for Busy {
        fn any_running(&self) -> bool {
            true
        }
    }

    /// A project with a device directory holding the work area.
    struct Fixture {
        project: TempDir,
        // Holds the work area; kept alive for the test body.
        device: TempDir,
        publication: PublicationState,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let project = TempDir::new(label);
            let device = TempDir::new(&format!("{label}-device"));
            let publication = PublicationState::under(device.path());
            Self {
                project,
                device,
                publication,
            }
        }

        fn workspace(&self) -> WorkspaceRoot {
            self.project.workspace()
        }

        /// A root holding only a content-bearing `CLAUDE.md`.
        fn claude_only(label: &str) -> Self {
            let fixture = Self::new(label);
            write(fixture.project.path(), "CLAUDE.md", AGENT_BODY);
            fixture
        }
    }

    fn request(agent: Option<&str>) -> DefinitionPlanRequest {
        DefinitionPlanRequest {
            agent: agent.map(str::to_string),
            ..DefinitionPlanRequest::default()
        }
    }

    fn to_json(value: &impl serde::Serialize) -> serde_json::Value {
        serde_json::to_value(value).expect("serializes")
    }

    /// The plan id of a `planned` preview, asserting it is 64 lowercase
    /// hex characters.
    fn planned_id(preview: &serde_json::Value) -> String {
        assert_eq!(preview["outcome"]["kind"], "planned");
        let id = preview["outcome"]["planId"]
            .as_str()
            .expect("a plan id")
            .to_string();
        assert_eq!(id.len(), 64);
        assert!(
            id.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        id
    }

    #[test]
    fn preview_of_a_claude_only_root_plans_the_two_identity_operations() {
        let fixture = Fixture::claude_only("preview-claude-only");
        let preview =
            definition_preview_for(&fixture.workspace(), &request(None)).expect("preview succeeds");
        let value = to_json(&preview);

        assert_eq!(value["schema"], "omnifrons.definition-preview/1");
        assert_eq!(
            value["agent"],
            fixture.workspace().path().to_string_lossy().as_ref()
        );
        planned_id(&value);
        let operations = value["outcome"]["operations"]
            .as_array()
            .expect("operations");
        let ops: Vec<(&str, &str)> = operations
            .iter()
            .map(|op| (op["op"].as_str().unwrap(), op["path"].as_str().unwrap()))
            .collect();
        assert_eq!(
            ops,
            vec![
                ("write-agents", "AGENTS.md"),
                ("write-wrapper", "CLAUDE.md")
            ]
        );
        assert_eq!(operations[0]["source"], "CLAUDE.md");
        assert_eq!(
            operations[0]["before"],
            serde_json::json!({"kind": "absent"})
        );
        assert_eq!(value["outcome"]["blockDiff"], serde_json::Value::Null);
        // A preview writes nothing.
        assert!(!fixture.project.path().join("AGENTS.md").exists());
    }

    #[test]
    fn apply_writes_the_planned_files_then_a_second_preview_is_empty_and_its_apply_a_noop() {
        let fixture = Fixture::claude_only("apply-claude-only");
        let workspace = fixture.workspace();
        let id = planned_id(&to_json(
            &definition_preview_for(&workspace, &request(None)).expect("preview"),
        ));

        let applied = to_json(
            &definition_apply_for(&workspace, &fixture.publication, &Idle, &request(None), &id)
                .expect("apply succeeds"),
        );

        assert_eq!(applied["schema"], "omnifrons.definition-apply/1");
        assert_eq!(applied["outcome"]["kind"], "applied");
        assert_eq!(applied["outcome"]["planId"], id.as_str());
        let files = applied["outcome"]["files"].as_array().expect("files");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0]["path"], "AGENTS.md");
        assert_eq!(files[1]["path"], "CLAUDE.md");
        assert_eq!(
            std::fs::read(fixture.project.path().join("AGENTS.md")).expect("AGENTS.md"),
            AGENT_BODY
        );
        assert_eq!(
            std::fs::read(fixture.project.path().join("CLAUDE.md")).expect("CLAUDE.md"),
            omnifrons_domain::definition::WRAPPER
        );

        let second = to_json(&definition_preview_for(&workspace, &request(None)).expect("again"));
        let second_id = planned_id(&second);
        assert_eq!(second["outcome"]["operations"], serde_json::json!([]));
        let noop = to_json(
            &definition_apply_for(
                &workspace,
                &fixture.publication,
                &Idle,
                &request(None),
                &second_id,
            )
            .expect("noop"),
        );
        assert_eq!(
            noop["outcome"],
            serde_json::json!({"kind": "noop", "planId": second_id})
        );
    }

    #[test]
    fn apply_with_a_wrong_plan_id_is_plan_stale_and_writes_nothing() {
        let fixture = Fixture::claude_only("apply-stale");
        let wrong = "0".repeat(64);
        let error = definition_apply_for(
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &request(None),
            &wrong,
        )
        .expect_err("the id matches no plan");
        assert_eq!(error.code, ShellErrorCode::PlanStale);
        assert_eq!(
            error.detail,
            Some(ShellErrorDetail::DefinitionReason {
                reason: "stale-plan".to_string()
            })
        );
        assert!(!fixture.project.path().join("AGENTS.md").exists());
    }

    #[test]
    fn a_conflicting_root_previews_as_refused_with_the_conflict_code() {
        let fixture = Fixture::new("preview-conflict");
        write(
            fixture.project.path(),
            "CLAUDE.md",
            b"# One\n\nFirst voice.\n",
        );
        write(
            fixture.project.path(),
            "AGENTS.md",
            b"# Two\n\nSecond voice.\n",
        );
        let value = to_json(
            &definition_preview_for(&fixture.workspace(), &request(None)).expect("preview"),
        );
        assert_eq!(value["outcome"]["kind"], "refused");
        assert_eq!(value["outcome"]["code"], "conflict");
        assert!(
            !value["outcome"]["message"]
                .as_str()
                .expect("message")
                .is_empty()
        );
    }

    #[test]
    fn a_nested_target_is_refused_without_its_own_consent_and_planned_with_it() {
        let fixture = Fixture::new("preview-nested");
        write(fixture.project.path(), "AGENTS.md", AGENT_BODY);
        write(fixture.project.path(), "child/CLAUDE.md", AGENT_BODY);
        let workspace = fixture.workspace();

        let refused =
            to_json(&definition_preview_for(&workspace, &request(Some("child"))).expect("preview"));
        assert_eq!(refused["outcome"]["kind"], "refused");
        assert_eq!(refused["outcome"]["code"], "nested_agent");
        assert_eq!(
            refused["agent"],
            workspace.path().join("child").to_string_lossy().as_ref()
        );

        let consenting = DefinitionPlanRequest {
            nested_consent: true,
            ..request(Some("child"))
        };
        let planned = to_json(&definition_preview_for(&workspace, &consenting).expect("preview"));
        planned_id(&planned);
    }

    #[test]
    fn a_bad_canonical_choice_or_plan_id_or_agent_is_invalid_request() {
        let fixture = Fixture::claude_only("invalid-requests");
        let workspace = fixture.workspace();
        let bad_choice = DefinitionPlanRequest {
            canonical_from: Some("GEMINI.md".to_string()),
            ..request(None)
        };
        let error = definition_preview_for(&workspace, &bad_choice).expect_err("only two choices");
        assert_eq!(error.code, ShellErrorCode::InvalidRequest);

        let malformed = [
            String::new(),
            "abc".to_string(),
            "g".repeat(64),
            "A".repeat(64),
            "0".repeat(63),
            "+".repeat(64),
        ];
        for id in &malformed {
            let error =
                definition_apply_for(&workspace, &fixture.publication, &Idle, &request(None), id)
                    .expect_err("malformed id");
            assert_eq!(error.code, ShellErrorCode::InvalidRequest, "{id}");
        }

        let error = definition_preview_for(&workspace, &request(Some("missing")))
            .expect_err("no such directory");
        assert_eq!(error.code, ShellErrorCode::InvalidRequest);
    }

    #[test]
    fn a_canonical_choice_names_the_winning_side_of_a_conflict() {
        let fixture = Fixture::new("preview-canonical");
        write(
            fixture.project.path(),
            "CLAUDE.md",
            b"# One\n\nFirst voice.\n",
        );
        write(
            fixture.project.path(),
            "AGENTS.md",
            b"# Two\n\nSecond voice.\n",
        );
        let choose = DefinitionPlanRequest {
            canonical_from: Some("AGENTS.md".to_string()),
            ..request(None)
        };
        let value =
            to_json(&definition_preview_for(&fixture.workspace(), &choose).expect("preview"));
        planned_id(&value);
    }

    #[test]
    fn apply_refuses_while_a_run_is_active_and_writes_nothing() {
        let fixture = Fixture::claude_only("apply-run-active");
        let id = "0".repeat(64);
        let error = definition_apply_for(
            &fixture.workspace(),
            &fixture.publication,
            &Busy,
            &request(None),
            &id,
        )
        .expect_err("a run is live");
        assert_eq!(error.code, ShellErrorCode::RunActive);
        assert!(!fixture.project.path().join("AGENTS.md").exists());
    }

    /// Every file below `dir`, recursively.
    fn files_below(dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return found;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(files_below(&path));
            } else {
                found.push(path);
            }
        }
        found
    }

    fn with_gemini() -> DefinitionPlanRequest {
        DefinitionPlanRequest {
            with_gemini: true,
            ..request(None)
        }
    }

    #[test]
    fn preview_with_gemini_adds_a_third_wrapper_operation_and_without_it_lists_two() {
        let fixture = Fixture::claude_only("preview-gemini");
        let workspace = fixture.workspace();

        let plain = to_json(&definition_preview_for(&workspace, &request(None)).expect("plain"));
        assert_eq!(plain["outcome"]["operations"].as_array().unwrap().len(), 2);

        let gemini = to_json(&definition_preview_for(&workspace, &with_gemini()).expect("gemini"));
        let operations = gemini["outcome"]["operations"].as_array().unwrap();
        assert_eq!(operations.len(), 3);
        assert_eq!(operations[2]["op"], "write-wrapper");
        assert_eq!(operations[2]["path"], "GEMINI.md");
        assert_eq!(
            operations[2]["before"],
            serde_json::json!({"kind": "absent"})
        );
        assert_eq!(operations[2].as_object().unwrap().len(), 4);
    }

    #[test]
    fn apply_with_gemini_writes_three_files_including_the_gemini_wrapper() {
        let fixture = Fixture::claude_only("apply-gemini");
        let workspace = fixture.workspace();
        let id = planned_id(&to_json(
            &definition_preview_for(&workspace, &with_gemini()).expect("preview"),
        ));

        let applied = to_json(
            &definition_apply_for(&workspace, &fixture.publication, &Idle, &with_gemini(), &id)
                .expect("apply succeeds"),
        );

        assert_eq!(applied["outcome"]["kind"], "applied");
        let files = applied["outcome"]["files"].as_array().unwrap();
        assert_eq!(files.len(), 3);
        assert!(files.iter().any(|file| file["path"] == "GEMINI.md"));
        assert_eq!(
            std::fs::read(fixture.project.path().join("GEMINI.md")).expect("GEMINI.md"),
            omnifrons_domain::definition::WRAPPER
        );
    }

    #[test]
    fn apply_after_an_identity_file_changed_since_the_preview_is_plan_stale_and_writes_nothing() {
        let fixture = Fixture::claude_only("apply-changed-since-preview");
        let workspace = fixture.workspace();
        let id = planned_id(&to_json(
            &definition_preview_for(&workspace, &request(None)).expect("preview"),
        ));
        let edited: &[u8] = b"# Fixture\n\nYou are Fixture, edited after the preview.\n";
        write(fixture.project.path(), "CLAUDE.md", edited);

        let error =
            definition_apply_for(&workspace, &fixture.publication, &Idle, &request(None), &id)
                .expect_err("the definition moved on");

        assert_eq!(error.code, ShellErrorCode::PlanStale);
        assert_eq!(
            std::fs::read(fixture.project.path().join("CLAUDE.md")).expect("CLAUDE.md"),
            edited
        );
        assert!(!fixture.project.path().join("AGENTS.md").exists());
        assert!(
            files_below(fixture.device.path()).is_empty(),
            "no record or snapshot was stored"
        );
    }

    #[test]
    fn a_refused_apply_reports_the_refusal_not_an_error() {
        let fixture = Fixture::new("apply-refused");
        write(
            fixture.project.path(),
            "CLAUDE.md",
            b"# One\n\nFirst voice.\n",
        );
        write(
            fixture.project.path(),
            "AGENTS.md",
            b"# Two\n\nSecond voice.\n",
        );
        let value = to_json(
            &definition_apply_for(
                &fixture.workspace(),
                &fixture.publication,
                &Idle,
                &request(None),
                &"0".repeat(64),
            )
            .expect("a refusal is an outcome"),
        );
        assert_eq!(value["outcome"]["kind"], "refused");
        assert_eq!(value["outcome"]["code"], "conflict");
    }

    // -- create (ADR-0005 slice 3, sub-slice 3d) --

    fn create_request(agent: Option<&str>) -> DefinitionCreateRequest {
        DefinitionCreateRequest {
            agent: agent.map(str::to_string),
            ..DefinitionCreateRequest::default()
        }
    }

    fn item(text: &str, said: &str) -> serde_json::Value {
        serde_json::json!({"text": text, "said": said, "status": "confirmed"})
    }

    /// The smallest valid answers: the five required fields.
    fn minimal_answers() -> serde_json::Value {
        serde_json::json!({
            "name": item("Harbor", "call it Harbor"),
            "role": item("a scheduling assistant", "it schedules things"),
            "deliverables": [item("Propose slots.", "slots, please")],
            "authority": [item("Draft and advise only.", "only advise")],
            "work_language": item("the language of each user", "whatever they speak"),
        })
    }

    /// The `AGENTS.md` the domain renders for `answers` in a folder with no
    /// child agents and no enclosing one.
    fn rendered(answers: &serde_json::Value) -> Vec<u8> {
        let raw = super::raw_answers(answers);
        let validated = Answers::validate(&raw).expect("valid answers");
        let context = CreateContext::new(Vec::new(), false).expect("a plain context");
        render(&validated, &context).into_bytes()
    }

    #[test]
    fn create_preview_of_an_empty_folder_plans_the_generated_write_with_provenance() {
        let fixture = Fixture::new("create-preview");
        let preview = definition_create_preview_for(
            &fixture.workspace(),
            &create_request(None),
            &minimal_answers(),
        )
        .expect("preview succeeds");
        let value = to_json(&preview);

        assert_eq!(value["schema"], "omnifrons.definition-create-preview/1");
        planned_id(&value);
        let operations = value["outcome"]["operations"]
            .as_array()
            .expect("operations");
        let ops: Vec<(&str, &str)> = operations
            .iter()
            .map(|op| (op["op"].as_str().unwrap(), op["path"].as_str().unwrap()))
            .collect();
        assert_eq!(
            ops,
            vec![
                ("write-generated", "AGENTS.md"),
                ("write-wrapper", "CLAUDE.md")
            ]
        );
        let provenance = value["outcome"]["provenance"]
            .as_array()
            .expect("provenance");
        let said: Vec<&str> = provenance
            .iter()
            .flat_map(|entry| entry["said"].as_array().unwrap())
            .map(|said| said.as_str().unwrap())
            .collect();
        for words in [
            "call it Harbor",
            "it schedules things",
            "slots, please",
            "only advise",
            "whatever they speak",
        ] {
            assert!(said.contains(&words), "{words} is shown beside a line");
        }
        let lines: Vec<&str> = provenance
            .iter()
            .map(|entry| entry["line"].as_str().unwrap())
            .collect();
        assert!(
            lines.iter().all(|line| !line.contains("call it Harbor")),
            "the user's words are not the generated lines"
        );
        let file = String::from_utf8(rendered(&minimal_answers())).unwrap();
        for entry in provenance {
            assert!(file.contains(entry["line"].as_str().unwrap()));
        }
        // A preview writes nothing.
        assert!(files_below(fixture.project.path()).is_empty());
        assert!(files_below(fixture.device.path()).is_empty());
    }

    #[test]
    fn create_preview_of_invalid_answers_lists_each_problem_and_touches_nothing() {
        let fixture = Fixture::new("create-invalid");
        let answers = serde_json::json!({
            "name": item("Harbor.", "call it Harbor"),
            "role": {"text": "a scheduling assistant", "said": "x", "status": "assumption"},
            "deliverables": [item("Propose slots.", "slots, please")],
            "authority": [item("Draft and advise only.", "only advise")],
            "work_language": item("the language of each user", "whatever they speak"),
            "mystery": "x",
        });
        let value = to_json(
            &definition_create_preview_for(&fixture.workspace(), &create_request(None), &answers)
                .expect("an invalid answer is an outcome"),
        );
        assert_eq!(value["outcome"]["kind"], "invalid");
        let problems: Vec<(&str, &str)> = value["outcome"]["problems"]
            .as_array()
            .expect("problems")
            .iter()
            .map(|p| (p["field"].as_str().unwrap(), p["reason"].as_str().unwrap()))
            .collect();
        assert!(problems.contains(&("mystery", "unknown_field")));
        assert!(problems.contains(&("name", "name_sentence_end")));
        assert!(problems.contains(&("role", "required_not_confirmed")));
        assert!(
            value["outcome"]["problems"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| !p["message"].as_str().unwrap().is_empty())
        );
        assert!(files_below(fixture.project.path()).is_empty());
    }

    #[test]
    fn create_apply_writes_the_rendered_file_and_the_wrapper_and_nothing_else() {
        let fixture = Fixture::new("create-apply");
        let workspace = fixture.workspace();
        let answers = minimal_answers();
        let id = planned_id(&to_json(
            &definition_create_preview_for(&workspace, &create_request(None), &answers)
                .expect("preview"),
        ));

        let applied = to_json(
            &definition_create_apply_for(
                &workspace,
                &fixture.publication,
                &Idle,
                &create_request(None),
                &answers,
                &id,
            )
            .expect("apply succeeds"),
        );

        assert_eq!(applied["schema"], "omnifrons.definition-create-apply/1");
        assert_eq!(applied["outcome"]["kind"], "applied");
        assert_eq!(applied["outcome"]["planId"], id.as_str());
        let files = applied["outcome"]["files"].as_array().expect("files");
        let paths: Vec<&str> = files.iter().map(|f| f["path"].as_str().unwrap()).collect();
        assert_eq!(paths, vec!["AGENTS.md", "CLAUDE.md"]);
        assert_eq!(
            std::fs::read(fixture.project.path().join("AGENTS.md")).expect("AGENTS.md"),
            rendered(&answers)
        );
        assert_eq!(
            std::fs::read(fixture.project.path().join("CLAUDE.md")).expect("CLAUDE.md"),
            omnifrons_domain::definition::WRAPPER
        );
        assert_eq!(
            files_below(fixture.project.path()).len(),
            2,
            "no skills or other file was written"
        );
    }

    #[test]
    fn a_second_create_preview_is_empty_its_apply_a_noop_and_other_answers_are_refused() {
        let fixture = Fixture::new("create-converged");
        let workspace = fixture.workspace();
        let answers = minimal_answers();
        let id = planned_id(&to_json(
            &definition_create_preview_for(&workspace, &create_request(None), &answers)
                .expect("preview"),
        ));
        definition_create_apply_for(
            &workspace,
            &fixture.publication,
            &Idle,
            &create_request(None),
            &answers,
            &id,
        )
        .expect("apply");

        let again = to_json(
            &definition_create_preview_for(&workspace, &create_request(None), &answers)
                .expect("second preview"),
        );
        let again_id = planned_id(&again);
        assert_eq!(again["outcome"]["operations"], serde_json::json!([]));
        let noop = to_json(
            &definition_create_apply_for(
                &workspace,
                &fixture.publication,
                &Idle,
                &create_request(None),
                &answers,
                &again_id,
            )
            .expect("second apply"),
        );
        assert_eq!(
            noop["outcome"],
            serde_json::json!({"kind": "noop", "planId": again_id})
        );

        let mut other = minimal_answers();
        other["name"] = item("Lighthouse", "call it Lighthouse");
        let refused = to_json(
            &definition_create_preview_for(&workspace, &create_request(None), &other)
                .expect("a refusal is an outcome"),
        );
        assert_eq!(refused["outcome"]["kind"], "refused");
        assert_eq!(refused["outcome"]["code"], "identity_exists");
    }

    #[test]
    fn a_nested_create_target_needs_consent() {
        let fixture = Fixture::new("create-nested");
        write(fixture.project.path(), "AGENTS.md", AGENT_BODY);
        std::fs::create_dir(fixture.project.path().join("child")).expect("child");
        let workspace = fixture.workspace();

        let refused = to_json(
            &definition_create_preview_for(
                &workspace,
                &create_request(Some("child")),
                &minimal_answers(),
            )
            .expect("a refusal is an outcome"),
        );
        assert_eq!(refused["outcome"]["kind"], "refused");
        assert_eq!(refused["outcome"]["code"], "nested_agent");

        let consenting = DefinitionCreateRequest {
            nested_consent: true,
            ..create_request(Some("child"))
        };
        let planned = to_json(
            &definition_create_preview_for(&workspace, &consenting, &minimal_answers())
                .expect("preview"),
        );
        planned_id(&planned);
    }

    #[test]
    fn create_apply_refuses_a_malformed_plan_id_and_a_live_run() {
        let fixture = Fixture::new("create-apply-refusals");
        let workspace = fixture.workspace();
        let answers = minimal_answers();

        let malformed = definition_create_apply_for(
            &workspace,
            &fixture.publication,
            &Idle,
            &create_request(None),
            &answers,
            "ABC",
        )
        .expect_err("not a plan id");
        assert_eq!(malformed.code, ShellErrorCode::InvalidRequest);

        let busy = definition_create_apply_for(
            &workspace,
            &fixture.publication,
            &Busy,
            &create_request(None),
            &answers,
            &"0".repeat(64),
        )
        .expect_err("a run is live");
        assert_eq!(busy.code, ShellErrorCode::RunActive);
        assert!(files_below(fixture.project.path()).is_empty());
    }

    #[test]
    fn create_apply_of_invalid_answers_is_an_outcome_and_writes_nothing() {
        let fixture = Fixture::new("create-apply-invalid");
        let value = to_json(
            &definition_create_apply_for(
                &fixture.workspace(),
                &fixture.publication,
                &Idle,
                &create_request(None),
                &serde_json::json!([]),
                &"0".repeat(64),
            )
            .expect("an invalid answer is an outcome"),
        );
        assert_eq!(value["outcome"]["kind"], "invalid");
        assert_eq!(value["outcome"]["problems"][0]["field"], "answers");
        assert_eq!(
            value["outcome"]["problems"][0]["reason"],
            "not_an_object_document"
        );
        assert!(files_below(fixture.project.path()).is_empty());
    }

    #[test]
    fn create_requests_check_the_target_like_the_adapt_requests() {
        let fixture = Fixture::new("create-target");
        let error = definition_create_preview_for(
            &fixture.workspace(),
            &create_request(Some("../outside")),
            &minimal_answers(),
        )
        .expect_err("not a workspace directory");
        assert_eq!(error.code, ShellErrorCode::InvalidRequest);
    }

    #[test]
    fn raw_answers_keeps_objects_lists_and_text_and_reads_other_scalars_as_other() {
        use omnifrons_domain::definition_create::RawValue;

        let raw = super::raw_answers(&serde_json::json!({
            "a": ["x", null, 1, true, {"b": "y"}],
        }));
        assert_eq!(
            raw,
            RawValue::Object(vec![(
                "a".to_string(),
                RawValue::List(vec![
                    RawValue::Text("x".to_string()),
                    RawValue::Null,
                    RawValue::Other,
                    RawValue::Other,
                    RawValue::Object(vec![("b".to_string(), RawValue::Text("y".to_string()))]),
                ])
            )])
        );
    }

    #[test]
    fn the_field_catalogue_pins_the_eleven_keys_in_interview_order() {
        let catalogue = to_json(&super::definition_create_fields());
        let rows: Vec<(&str, bool, bool)> = catalogue
            .as_array()
            .expect("an array")
            .iter()
            .map(|row| {
                (
                    row["key"].as_str().unwrap(),
                    row["required"].as_bool().unwrap(),
                    row["list"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                ("role", true, false),
                ("serves", false, false),
                ("deliverables", true, true),
                ("objectives", false, true),
                ("authority", true, true),
                ("sources", false, true),
                ("never_invent", false, true),
                ("tone", false, false),
                ("work_language", true, false),
                ("name", true, false),
                ("open_decisions", false, true),
            ]
        );
        assert!(
            catalogue
                .as_array()
                .unwrap()
                .iter()
                .all(|row| !row["prompt"].as_str().unwrap().is_empty())
        );
    }

    #[test]
    fn create_apply_with_a_different_well_formed_id_is_plan_stale_and_writes_nothing() {
        let fixture = Fixture::new("create-stale");
        let workspace = fixture.workspace();
        let answers = minimal_answers();
        let id = planned_id(&to_json(
            &definition_create_preview_for(&workspace, &create_request(None), &answers)
                .expect("preview"),
        ));
        let other = if id.starts_with('0') {
            "1".repeat(64)
        } else {
            "0".repeat(64)
        };

        let error = definition_create_apply_for(
            &workspace,
            &fixture.publication,
            &Idle,
            &create_request(None),
            &answers,
            &other,
        )
        .expect_err("the id names another plan");

        assert_eq!(error.code, ShellErrorCode::PlanStale);
        assert!(files_below(fixture.project.path()).is_empty());
        assert!(files_below(fixture.device.path()).is_empty());
    }

    #[test]
    fn create_with_gemini_plans_and_writes_the_gemini_wrapper_too() {
        let fixture = Fixture::new("create-gemini");
        let workspace = fixture.workspace();
        let answers = minimal_answers();
        let request = DefinitionCreateRequest {
            with_gemini: true,
            ..create_request(None)
        };
        let preview = to_json(
            &definition_create_preview_for(&workspace, &request, &answers).expect("preview"),
        );
        let id = planned_id(&preview);
        let paths: Vec<&str> = preview["outcome"]["operations"]
            .as_array()
            .expect("operations")
            .iter()
            .map(|op| op["path"].as_str().unwrap())
            .collect();
        assert!(paths.contains(&"GEMINI.md"), "{paths:?}");

        let applied = to_json(
            &definition_create_apply_for(
                &workspace,
                &fixture.publication,
                &Idle,
                &request,
                &answers,
                &id,
            )
            .expect("apply"),
        );
        assert_eq!(applied["outcome"]["kind"], "applied");
        assert_eq!(
            std::fs::read(fixture.project.path().join("GEMINI.md")).expect("GEMINI.md"),
            omnifrons_domain::definition::WRAPPER
        );
        assert_eq!(files_below(fixture.project.path()).len(), 3);
    }

    #[test]
    fn switch_check_reports_a_reaching_root_and_the_agents_reference() {
        let project = TempDir::new("switch-reaches");
        write(project.path(), "AGENTS.md", AGENT_BODY);
        let dto = definition_switch_check_for(&project.workspace(), "claude-code", None)
            .expect("check succeeds");
        let value = serde_json::to_value(&dto).expect("serializes");
        assert_eq!(value["schema"], "omnifrons.definition-switch-check/1");
        assert_eq!(value["adapter"], "claude-code");
        assert_eq!(value["host"], "Claude Code");
        assert_eq!(value["verdict"]["kind"], "reaches");
        assert_eq!(value["verdict"]["file"], "AGENTS.md");
        assert_eq!(value["reference"]["path"], "AGENTS.md");
        let digest = value["reference"]["digest"].as_str().expect("a digest");
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn switch_check_for_a_generic_adapter_uses_the_chosen_host() {
        let project = TempDir::new("switch-generic");
        write(project.path(), "AGENTS.md", AGENT_BODY);
        let none = definition_switch_check_for(&project.workspace(), "pty-cli", None)
            .expect("check succeeds");
        assert_eq!(
            serde_json::to_value(&none).expect("json")["verdict"]["kind"],
            "uncertain"
        );
        let gemini =
            definition_switch_check_for(&project.workspace(), "pty-cli", Some("Gemini CLI"))
                .expect("check succeeds");
        let value = serde_json::to_value(&gemini).expect("json");
        assert_eq!(value["host"], "Gemini CLI");
        assert_eq!(value["verdict"]["kind"], "unreached");
    }

    #[test]
    fn switch_check_refuses_an_unknown_adapter_token() {
        let project = TempDir::new("switch-unknown");
        let error = definition_switch_check_for(&project.workspace(), "no-such-adapter", None)
            .expect_err("an unknown adapter is refused");
        assert_eq!(error.code, ShellErrorCode::InvalidRequest);
    }

    #[test]
    fn switch_check_without_agents_md_has_no_reference() {
        let project = TempDir::new("switch-no-reference");
        write(project.path(), "CLAUDE.md", AGENT_BODY);
        let dto = definition_switch_check_for(&project.workspace(), "claude-code", None)
            .expect("check succeeds");
        let value = serde_json::to_value(&dto).expect("json");
        assert_eq!(value["reference"], serde_json::Value::Null);
        assert_eq!(value["verdict"]["kind"], "reaches");
    }

    #[test]
    fn switch_check_for_a_bound_adapter_ignores_a_conflicting_chosen_host() {
        let project = TempDir::new("switch-bound");
        write(project.path(), "AGENTS.md", AGENT_BODY);
        let dto =
            definition_switch_check_for(&project.workspace(), "claude-code", Some("Gemini CLI"))
                .expect("check succeeds");
        let value = serde_json::to_value(&dto).expect("json");
        assert_eq!(value["host"], "Claude Code");
        assert_eq!(value["verdict"]["kind"], "reaches");
        assert_eq!(value["verdict"]["claim"]["host"], "Claude Code");
    }
}
