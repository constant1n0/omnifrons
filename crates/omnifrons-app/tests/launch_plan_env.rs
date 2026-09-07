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
