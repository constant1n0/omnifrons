//! The agent-definition commands (ADR-0005 sub-slice 1h): `definition_status`
//! and `definition_verify`, registered next to the guidance and wrong-root
//! commands. Each is a thin wrapper over a pure body tested here without a
//! Tauri runtime, composing the application services
//! (`omnifrons_app::definition`) with the real filesystem adapters: a
//! no-follow directory walk rooted at the active workspace, and SHA-256
//! over identity file bytes.
//!
//! Both commands read; neither writes, locks the publication surface, or
//! is frozen while a run is active -- the prototype's own `scan` and
//! `verify_agent` are themselves read-only, and the ports they compose
//! over (`omnifrons_app::definition::{scan, verify}`) never touch a
//! filesystem except through the adapters they are given.
//!
//! The prototype's own `verify_agent` additionally accepts an explicit
//! agent path below the scan root and reports its write path's pending
//! snapshots (`backups`); this command verifies only the active
//! workspace root itself, read-only -- an explicit target is a later
//! extension.

use omnifrons_adapters::{FsDefinitionTree, Sha2Hasher};
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::definition::{DefinitionError, DefinitionPorts, scan, verify};
use tauri::AppHandle;

use crate::ipc::commands::active_workspace;
use crate::ipc::dto::{
    DefinitionStatusDto, DefinitionVerifyDto, ShellError, ShellErrorCode, ShellErrorDetail,
};

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
    fn from(error: DefinitionError) -> Self {
        let reason = error.reason().to_string();
        let code = match &error {
            DefinitionError::Tree(_) => ShellErrorCode::ScanFailed,
            DefinitionError::Nesting(_)
            | DefinitionError::DuplicateIdentityKind(_)
            | DefinitionError::DuplicateSnapshotPath { .. } => ShellErrorCode::DefinitionInvalid,
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

/// A verification of `workspace`'s own canonical definition: its
/// classification state, whether it is canonical, and every problem
/// found.
///
/// # Errors
///
/// As [`definition_status_for`].
pub fn definition_verify_for(workspace: &WorkspaceRoot) -> Result<DefinitionVerifyDto, ShellError> {
    let tree = FsDefinitionTree::from_environment();
    let hasher = Sha2Hasher::new();
    let ports = DefinitionPorts {
        tree: &tree,
        hasher: &hasher,
    };
    let report = verify(&ports, workspace)?;
    Ok(DefinitionVerifyDto::from_report(workspace, &report))
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

/// The active workspace's own canonical-definition verification.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`definition_verify_for`]'s errors.
#[tauri::command]
pub async fn definition_verify(app: AppHandle) -> Result<DefinitionVerifyDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || definition_verify_for(&workspace))
        .await
        .expect("the blocking definition-verify task panicked")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use omnifrons_app::WorkspaceRoot;
    use omnifrons_app::definition::DefinitionError;
    use omnifrons_app::definition_tree::DefinitionTreeError;
    use omnifrons_domain::definition::{DuplicateIdentityKindError, IdentityKind};
    use omnifrons_domain::nesting::NestingError;

    use super::{definition_status_for, definition_verify_for};
    use crate::ipc::dto::{ShellError, ShellErrorCode, ShellErrorDetail};

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
        let cases: [(DefinitionError, ShellErrorCode, &str); 4] = [
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

        let verify = definition_verify_for(&workspace).expect("verify succeeds");

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

        let verify = definition_verify_for(&workspace).expect("verify succeeds");

        assert_eq!(verify.state, crate::ipc::dto::RootStateTag::None);
        assert!(!verify.canonical, "{:?}", verify.problems);
        assert_eq!(verify.problems, vec!["state is none".to_string()]);
    }
}
