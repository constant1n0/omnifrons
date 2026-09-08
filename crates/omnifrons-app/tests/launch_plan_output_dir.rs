//! `LaunchPlan::declare_output_dir` (spike slice 5, HAP-001-R9): the run
//! subdirectory is declared to the harness through exactly one non-secret
//! environment key, `OMNIFRONS_OUTPUT_DIR`, whose value is set by the
//! product -- carried on the plan as an assignment, never resolved from
//! the parent environment -- and mirrored on `LaunchPlan::output_dir`.

use std::path::PathBuf;

use omnifrons_app::{
    EnvPlan, LaunchPlan, OUTPUT_DIR_ENV_KEY, StdinPlan, WorkspaceRoot, is_secret_shaped,
};
use omnifrons_domain::adapter::TransportClass;
use omnifrons_domain::scope::ScopeMode;

fn plan() -> LaunchPlan {
    let workspace =
        WorkspaceRoot::new(std::env::temp_dir()).expect("the temp dir must be a valid workspace");
    LaunchPlan {
        argv: vec![],
        env: EnvPlan::new(&[]).expect("empty declared keys must be accepted"),
        cwd: workspace,
        stdin: StdinPlan::Null,
        prompt: None,
        scope_mode: ScopeMode::Advisory,
        transport: TransportClass::StructuredStreamingCli,
        output_dir: None,
    }
}

#[test]
fn the_output_dir_key_is_the_documented_non_secret_name() {
    assert_eq!(OUTPUT_DIR_ENV_KEY, "OMNIFRONS_OUTPUT_DIR");
    assert!(
        !is_secret_shaped(OUTPUT_DIR_ENV_KEY),
        "the one key HAP-001 adds must never match the secret-shape refusal"
    );
}

#[test]
fn declare_output_dir_sets_the_field_and_the_assignment_together() {
    let mut plan = plan();
    assert!(plan.output_dir.is_none());
    assert!(plan.env.assignment_for(OUTPUT_DIR_ENV_KEY).is_none());

    let dir = PathBuf::from("/project/.omnifrons/outbox/run-1");
    plan.declare_output_dir(dir.clone())
        .expect("declaring the output dir on an allowlist plan must succeed");

    assert_eq!(plan.output_dir, Some(dir.clone()));
    let assignment = plan
        .env
        .assignment_for(OUTPUT_DIR_ENV_KEY)
        .expect("the plan must carry the assignment");
    assert_eq!(assignment.value, dir.into_os_string());
    assert!(
        plan.env
            .allowed_keys()
            .contains(&OUTPUT_DIR_ENV_KEY.to_string()),
        "the key is declared through the allowlist"
    );
}

#[test]
fn declare_output_dir_on_an_inherited_env_is_refused_and_leaves_the_field_unset() {
    let mut plan = plan();
    plan.env = EnvPlan::Inherit;
    let result = plan.declare_output_dir(PathBuf::from("/x"));
    assert_eq!(
        result.unwrap_err(),
        omnifrons_app::LaunchPlanError::AssignmentOnInheritedEnv
    );
    assert!(plan.output_dir.is_none());
}
