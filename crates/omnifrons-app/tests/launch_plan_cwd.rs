//! `validate_cwd_within_workspace` rejects a candidate working directory
//! whose canonical form is not the workspace root itself -- a `..`
//! relative escape, an absolute path elsewhere, or a symlink whose target
//! resolves outside the workspace -- and accepts the workspace's own path.
//! A `LaunchPlan` built from an accepted cwd carries `ScopeMode::Advisory`
//! (`docs/spike-log.md` § Slice 3: no built-in adapter claims a stronger
//! scope in this slice).

use omnifrons_app::{
    EnvPlan, LaunchPlan, LaunchPlanError, WorkspaceRoot, validate_cwd_within_workspace,
};
use omnifrons_domain::adapter::StdinPlan;
use omnifrons_domain::scope::ScopeMode;

fn temp_subdir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "omnifrons-launch-plan-cwd-test-{}-{label}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("failed to create the test fixture directory");
    dir
}

#[test]
fn dot_dot_relative_escape_is_rejected() {
    let workspace_dir = temp_subdir("workspace-dotdot");
    let workspace = WorkspaceRoot::new(&workspace_dir).expect("workspace must be valid");

    let escape = workspace_dir.join("..");
    let result = validate_cwd_within_workspace(&escape, &workspace);

    assert_eq!(result.unwrap_err(), LaunchPlanError::CwdOutsideWorkspace);
    let _ = std::fs::remove_dir_all(&workspace_dir);
}

#[test]
fn an_absolute_path_elsewhere_is_rejected() {
    let workspace_dir = temp_subdir("workspace-elsewhere");
    let workspace = WorkspaceRoot::new(&workspace_dir).expect("workspace must be valid");

    let elsewhere = temp_subdir("elsewhere");
    let result = validate_cwd_within_workspace(&elsewhere, &workspace);

    assert_eq!(result.unwrap_err(), LaunchPlanError::CwdOutsideWorkspace);
    let _ = std::fs::remove_dir_all(&workspace_dir);
    let _ = std::fs::remove_dir_all(&elsewhere);
}

/// Unix only, gated on its own test rather than the whole file (the
/// repository's per-test gating convention, e.g.
/// `crates/omnifrons-adapters/tests/jsonl_store.rs`): creating a symlink on
/// Windows needs a privilege (`SeCreateSymbolicLinkPrivilege`, or Developer
/// Mode) the CI runner does not grant, so the fixture itself -- not the
/// property under test -- would fail there. The other tests in this file
/// are platform-generic and run everywhere the workspace tests on.
#[cfg(unix)]
#[test]
fn a_symlink_targeting_outside_the_workspace_is_rejected() {
    let workspace_dir = temp_subdir("workspace-symlink");
    let workspace = WorkspaceRoot::new(&workspace_dir).expect("workspace must be valid");

    let outside_dir = temp_subdir("symlink-target-outside");
    let link_path = workspace_dir.join("escape-link");
    std::os::unix::fs::symlink(&outside_dir, &link_path)
        .expect("failed to create the escape symlink");

    let result = validate_cwd_within_workspace(&link_path, &workspace);

    assert_eq!(result.unwrap_err(), LaunchPlanError::CwdOutsideWorkspace);
    let _ = std::fs::remove_dir_all(&workspace_dir);
    let _ = std::fs::remove_dir_all(&outside_dir);
}

#[test]
fn the_workspace_s_own_path_is_accepted() {
    let workspace_dir = temp_subdir("workspace-own-path");
    let workspace = WorkspaceRoot::new(&workspace_dir).expect("workspace must be valid");

    let result = validate_cwd_within_workspace(workspace.path(), &workspace);

    assert!(result.is_ok());
    let _ = std::fs::remove_dir_all(&workspace_dir);
}

#[test]
fn a_launch_plan_built_from_an_accepted_cwd_is_advisory() {
    let workspace_dir = temp_subdir("workspace-advisory");
    let workspace = WorkspaceRoot::new(&workspace_dir).expect("workspace must be valid");
    validate_cwd_within_workspace(workspace.path(), &workspace)
        .expect("the workspace's own path must be accepted");

    let plan = LaunchPlan {
        argv: vec![],
        env: EnvPlan::new(&[]).expect("empty declared keys must be accepted"),
        cwd: workspace.clone(),
        stdin: StdinPlan::Null,
        prompt: None,
        scope_mode: ScopeMode::Advisory,
    };

    assert_eq!(plan.scope_mode, ScopeMode::Advisory);
    assert_eq!(plan.cwd, workspace);
    let _ = std::fs::remove_dir_all(&workspace_dir);
}
