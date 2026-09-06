//! Verifies `tauri.conf.json` transcribes
//! docs/renderer-content-security.md § CSP baseline (the one documented
//! exception being `connect-src`, which the baseline sets to `'none'` but
//! Tauri's IPC bridge needs -- see docs/repository-layout.md § Crate map),
//! and that `capabilities/default.json` grants nothing beyond
//! `core:default`.

use std::collections::BTreeSet;
use std::fs;

use serde_json::Value;

fn read_json(relative_path: &str) -> Value {
    let path = format!("{}/{relative_path}", env!("CARGO_MANIFEST_DIR"));
    let raw = fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("failed to parse {path} as JSON: {e}"))
}

/// The CSP baseline directives transcribed unchanged from
/// renderer-content-security.md § CSP baseline -- every directive except
/// `connect-src`, which carries the documented platform-bridge exception
/// (see [`connect_src_is_exactly_the_documented_ipc_bridge_exception`]).
const BASELINE_DIRECTIVES: &[&str] = &[
    "default-src",
    "object-src",
    "frame-src",
    "base-uri",
    "form-action",
    "script-src",
    "style-src",
    "img-src",
    "media-src",
    "font-src",
    "manifest-src",
    "worker-src",
];

#[test]
fn csp_baseline_directives_are_transcribed() {
    let config = read_json("tauri.conf.json");
    let csp = &config["app"]["security"]["csp"];

    assert_eq!(csp["default-src"], "'none'");
    assert_eq!(csp["object-src"], "'none'");
    assert_eq!(csp["frame-src"], "'none'");
    assert_eq!(csp["base-uri"], "'none'");
    assert_eq!(csp["form-action"], "'none'");
    assert_eq!(csp["script-src"], "'self'");
    assert_eq!(csp["style-src"], "'self'");
    assert_eq!(csp["img-src"], "'self' artifact:");
    assert_eq!(csp["media-src"], "'self' artifact:");
    assert_eq!(csp["font-src"], "'self'");
    assert_eq!(csp["manifest-src"], "'self'");
    assert_eq!(csp["worker-src"], "'self'");

    // Exhaustiveness: the CSP object must carry exactly the baseline
    // directives asserted above plus the documented connect-src exception --
    // no directive silently added or dropped.
    let mut expected: BTreeSet<&str> = BASELINE_DIRECTIVES.iter().copied().collect();
    expected.insert("connect-src");
    let actual: BTreeSet<&str> = csp
        .as_object()
        .expect("csp must be an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        actual, expected,
        "csp directive set must match exactly the baseline plus the documented connect-src exception"
    );
}

#[test]
fn connect_src_is_exactly_the_documented_ipc_bridge_exception() {
    let config = read_json("tauri.conf.json");

    assert_eq!(
        config["app"]["security"]["csp"]["connect-src"], "ipc: http://ipc.localhost",
        "connect-src must be exactly the documented platform-bridge exception, no wider"
    );
}

#[test]
fn default_capability_grants_only_core_default() {
    let capability = read_json("capabilities/default.json");
    let permissions = capability["permissions"]
        .as_array()
        .expect("permissions must be an array");

    assert_eq!(
        permissions,
        &vec![Value::String("core:default".to_string())],
        "the default capability must grant exactly core:default and nothing else, found: {permissions:?}"
    );

    assert!(
        capability.get("remote").is_none_or(Value::is_null),
        "the default capability must not grant remote URL access, found: {:?}",
        capability.get("remote")
    );

    assert_eq!(
        capability["windows"],
        Value::Array(vec![Value::String("main".to_string())]),
        "the default capability must be scoped to exactly the main window, found: {:?}",
        capability["windows"]
    );

    assert_ne!(
        capability.get("local"),
        Some(&Value::Bool(false)),
        "the default capability must not disable local app URL access"
    );
}

/// `docs/spike-log.md`'s IPC contract keeps this shell's capabilities at
/// `core:default` only, even after adding the three demo-harness commands:
/// `build.rs` must never opt into `AppManifest::commands` (a
/// `tauri-build` mechanism for auto-generating a broader ACL from command
/// signatures), which would widen the capability surface silently.
#[test]
fn build_script_does_not_reference_app_manifest() {
    let path = format!("{}/build.rs", env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));

    assert!(
        !source.contains("AppManifest"),
        "build.rs must not reference AppManifest::commands; capabilities stay at core:default only"
    );
}
