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
//! `definition_verify` takes an optional agent directory below the
//! workspace (the prototype's own `verify --agent <path>`), defaulting
//! to the workspace root; it reports no pending snapshots (`backups`),
//! which belong to the write path.

use omnifrons_adapters::{FsDefinitionTree, Sha2Hasher};
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::definition::{DefinitionError, DefinitionPorts, scan, verify};
use omnifrons_domain::nesting;
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
/// plain names: non-empty, not `.` or `..`, and free of `\`, `:` and
/// NUL. That is checked on every platform before the filesystem is
/// touched, so a native path syntax (a drive prefix, a rooted or
/// backslash-separated component) can never reach the join below and
/// escape or replace the workspace prefix.
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
/// `..`, and carrying no backslash, colon or NUL, which a native path
/// would read as a separator, a drive prefix or a terminator.
fn is_plain_name(component: &str) -> bool {
    !component.is_empty()
        && component != "."
        && component != ".."
        && !component.contains(['\\', ':', '\0'])
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
    /// `agent` is the workspace path joined with the target.
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
        assert_eq!(
            verify.agent,
            workspace.path().join("child").to_string_lossy()
        );
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
    /// or a drive colon) are refused on every platform, before any
    /// filesystem call.
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
    /// directory literally named like that exists; it is still refused,
    /// because the target is a POSIX path of plain names only.
    #[cfg(unix)]
    #[test]
    fn definition_verify_for_refuses_an_existing_directory_named_with_native_syntax() {
        let project = TempDir::new("verify-native-existing");
        write(project.path(), "AGENTS.md", AGENT_BODY);
        for name in ["a\\..\\b", "..\\x", "C:", "C:\\x", "\\abs", "a:b"] {
            write(project.path(), &format!("{name}/AGENTS.md"), AGENT_BODY);
        }
        let workspace = project.workspace();

        for target in ["a\\..\\b", "..\\x", "C:", "C:\\x", "\\abs", "a:b"] {
            assert_invalid_request(definition_verify_for(&workspace, Some(target)));
        }
    }
}
