//! `EnvPlan::new` refuses a secret-shaped declared key outright, even
//! though the adapter itself asked for it, and otherwise builds exactly
//! the base allowlist plus the declared keys, deduplicated.

use omnifrons_app::EnvPlan;
use omnifrons_app::LaunchPlanError;

#[test]
fn secret_shaped_declared_keys_are_refused() {
    for key in [
        "MY_TOKEN",
        "SOME_SECRET",
        "API_KEY",
        "DB_PASSWORD",
        "ANTHROPIC_API_KEY",
        "AWS_ACCESS_KEY_ID",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "my_token",
    ] {
        let error = EnvPlan::new(&[key.to_string()]).unwrap_err();
        assert_eq!(
            error,
            LaunchPlanError::SecretShapedEnvKey(key.to_string()),
            "{key} must be refused as secret-shaped"
        );
    }
}

#[test]
fn ordinary_declared_keys_are_accepted() {
    let plan = EnvPlan::new(&["MY_APP_CONFIG_DIR".to_string()])
        .expect("an ordinary declared key must be accepted");
    assert!(
        plan.allowed_keys()
            .contains(&"MY_APP_CONFIG_DIR".to_string())
    );
}

#[test]
fn allowlist_is_exactly_the_base_set_plus_declared_keys() {
    let plan = EnvPlan::new(&["MY_APP_CONFIG_DIR".to_string()])
        .expect("an ordinary declared key must be accepted");
    let mut keys = plan.allowed_keys().to_vec();
    keys.sort();
    keys.dedup();

    let mut expected: Vec<String> = vec!["PATH", "LANG", "LC_ALL", "TMPDIR"]
        .into_iter()
        .map(str::to_string)
        .collect();
    if cfg!(windows) {
        expected.extend(
            ["USERPROFILE", "SystemRoot", "SystemDrive", "TEMP", "TMP"]
                .into_iter()
                .map(str::to_string),
        );
    } else {
        expected.push("HOME".to_string());
    }
    expected.push("MY_APP_CONFIG_DIR".to_string());
    expected.sort();
    expected.dedup();

    assert_eq!(
        keys, expected,
        "the allowlist must be exactly the base set plus declared keys"
    );
}

#[test]
fn declaring_the_same_key_twice_does_not_duplicate_it() {
    let plan = EnvPlan::new(&["PATH".to_string(), "PATH".to_string()])
        .expect("re-declaring a base key must be accepted, not an error");
    let occurrences = plan
        .allowed_keys()
        .iter()
        .filter(|key| key.as_str() == "PATH")
        .count();
    assert_eq!(
        occurrences, 1,
        "PATH must appear exactly once in the allowlist"
    );
}

#[test]
fn inherit_carries_no_closed_key_list() {
    assert_eq!(EnvPlan::Inherit.allowed_keys(), &[] as &[String]);
}

// -- Slice 5: product-set assignments alongside the allowlist --

/// A product-set key/value pair (spike slice 5, HAP-001 D4) joins the
/// allowlist's keys and is exposed as an assignment; the same key assigned
/// twice keeps the latest value and appears once.
#[test]
fn assign_adds_the_key_once_and_exposes_the_latest_value() {
    let mut plan = EnvPlan::new(&[]).expect("empty declared keys must be accepted");
    plan.assign("OMNIFRONS_OUTPUT_DIR", "/first")
        .expect("an ordinary key must be assignable");
    plan.assign("OMNIFRONS_OUTPUT_DIR", "/second")
        .expect("re-assigning the same key must be accepted");

    let occurrences = plan
        .allowed_keys()
        .iter()
        .filter(|key| key.as_str() == "OMNIFRONS_OUTPUT_DIR")
        .count();
    assert_eq!(
        occurrences, 1,
        "an assigned key appears once in the allowlist"
    );
    assert_eq!(plan.assignments().len(), 1);
    assert_eq!(plan.assignments()[0].key, "OMNIFRONS_OUTPUT_DIR");
    assert_eq!(
        plan.assignments()[0].value,
        std::ffi::OsString::from("/second")
    );
    assert_eq!(
        plan.assignment_for("OMNIFRONS_OUTPUT_DIR")
            .map(|assignment| assignment.value.clone()),
        Some(std::ffi::OsString::from("/second"))
    );
    assert!(plan.assignment_for("PATH").is_none());
}

/// The secret-shape refusal applies to an assigned key's name exactly as
/// to a declared one (HAP-001-R37: the only key this contract adds is
/// non-secret).
#[test]
fn assign_refuses_a_secret_shaped_key() {
    let mut plan = EnvPlan::new(&[]).expect("empty declared keys must be accepted");
    let error = plan.assign("OMNIFRONS_SECRET", "x").unwrap_err();
    assert_eq!(
        error,
        LaunchPlanError::SecretShapedEnvKey("OMNIFRONS_SECRET".to_string())
    );
    assert!(
        plan.assignments().is_empty(),
        "a refused assignment leaves nothing behind"
    );
}

/// An inherited environment has no closed key list to assign into.
#[test]
fn assign_on_an_inherited_plan_is_refused() {
    let mut plan = EnvPlan::Inherit;
    assert_eq!(
        plan.assign("OMNIFRONS_OUTPUT_DIR", "/x").unwrap_err(),
        LaunchPlanError::AssignmentOnInheritedEnv
    );
}

#[test]
fn a_fresh_allowlist_has_no_assignments() {
    let plan = EnvPlan::new(&[]).expect("empty declared keys must be accepted");
    assert!(plan.assignments().is_empty());
}
