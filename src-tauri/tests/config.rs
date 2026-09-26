//! Verifies `tauri.conf.json` transcribes
//! docs/renderer-content-security.md § CSP baseline (including its
//! `connect-src`: `ipc:`, the Linux/macOS typed-IPC bridge source, with
//! `src-tauri/tauri.windows.conf.json` replacing it with the Windows bridge
//! source, `http://ipc.localhost` -- see docs/repository-layout.md § Crate
//! map), and that `capabilities/default.json` grants nothing beyond
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
/// `connect-src`, which carries the documented per-OS typed-IPC bridge
/// source: `ipc:` at the base (Linux, macOS), replaced on Windows by
/// `tauri.windows.conf.json`'s `http://ipc.localhost` (see
/// [`base_connect_src_is_exactly_the_linux_macos_bridge_source`] and
/// [`windows_overlay_replaces_connect_src_with_exactly_the_windows_bridge_source`]).
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
    // directives asserted above plus the per-OS connect-src -- no directive
    // silently added or dropped.
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
        "csp directive set must match exactly the baseline plus the per-OS connect-src"
    );
}

#[test]
fn base_connect_src_is_exactly_the_linux_macos_bridge_source() {
    let config = read_json("tauri.conf.json");

    assert_eq!(
        config["app"]["security"]["csp"]["connect-src"], "ipc:",
        "connect-src must be exactly the Linux/macOS typed-IPC bridge source; \
         Windows replaces it via tauri.windows.conf.json"
    );
}

/// Tauri's platform-config merge (JSON Merge Patch) means
/// `tauri.windows.conf.json` overrides `connect-src` on Windows only; every
/// other directive is inherited unchanged from `tauri.conf.json`. This
/// overlay must carry exactly that one override -- nothing else, or it
/// could silently widen the Windows CSP beyond what RCS-001 documents.
#[test]
fn windows_overlay_replaces_connect_src_with_exactly_the_windows_bridge_source() {
    let overlay = read_json("tauri.windows.conf.json");

    assert_eq!(
        overlay,
        serde_json::json!({
            "app": {
                "security": {
                    "csp": {
                        "connect-src": "http://ipc.localhost"
                    }
                }
            }
        }),
        "tauri.windows.conf.json must contain exactly the Windows bridge source override, found: {overlay:?}"
    );
}

/// The config each OS actually builds with, read by Tauri's own reader
/// (`tauri::utils::config::parse::read_from`, the one `tauri-build` and
/// `generate_context!` use). The file-level tests above pin each file; these
/// pin the merged result, so a change in how Tauri merges a platform overlay
/// cannot silently drop or add a directive on any OS.
fn merged_config(target: tauri::utils::platform::Target) -> (Value, usize) {
    let (config, paths) = tauri::utils::config::parse::read_from(
        target,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
    )
    .unwrap_or_else(|e| panic!("tauri must read the config for {target:?}: {e}"));
    (config, paths.len())
}

#[test]
fn merged_windows_config_is_the_base_with_only_the_windows_bridge_source() {
    let (merged, files_read) = merged_config(tauri::utils::platform::Target::Windows);
    assert_eq!(
        files_read, 2,
        "Windows must read the base config plus exactly its one overlay"
    );

    let mut expected = read_json("tauri.conf.json");
    expected["app"]["security"]["csp"]["connect-src"] =
        Value::String("http://ipc.localhost".to_string());
    assert_eq!(
        merged, expected,
        "on Windows the merged config must differ from the base only in connect-src"
    );
}

#[test]
fn merged_linux_and_macos_configs_are_the_base_config() {
    for target in [
        tauri::utils::platform::Target::Linux,
        tauri::utils::platform::Target::MacOS,
    ] {
        let (merged, files_read) = merged_config(target);
        assert_eq!(files_read, 1, "{target:?} must read only the base config");
        assert_eq!(
            merged,
            read_json("tauri.conf.json"),
            "{target:?} must build with the base config unchanged, connect-src `ipc:` included"
        );
    }
}

/// Tauri reads its base config from `tauri.conf.json[5]` or `Tauri.toml` and
/// merges a `tauri.<platform>.conf.json[5]` or `Tauri.<platform>.toml`
/// overlay over it via JSON Merge Patch, for macOS, Windows, Linux, Android
/// and iOS alike (tauri-utils `config::parse`). Any config file beyond the
/// two reviewed ones -- another platform's overlay, or a second spelling of
/// the Windows one -- could silently patch `connect-src` somewhere, so this
/// is an allowlist of every file Tauri would read, not a list of known-bad
/// names.
#[test]
fn only_the_reviewed_tauri_config_files_exist() {
    let dir = env!("CARGO_MANIFEST_DIR");
    let config_files: BTreeSet<String> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("failed to list {dir}: {e}"))
        .map(|entry| {
            entry
                .expect("directory entry must be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| {
            // Case-insensitive: on a case-insensitive filesystem Tauri's
            // exact-name lookup would find a differently-cased file too.
            let name = name.to_ascii_lowercase();
            let extension = std::path::Path::new(&name)
                .extension()
                .and_then(|e| e.to_str());
            name.starts_with("tauri.")
                && match extension {
                    Some("json" | "json5") => name.contains(".conf."),
                    Some("toml") => true,
                    _ => false,
                }
        })
        .collect();

    let expected: BTreeSet<String> = ["tauri.conf.json", "tauri.windows.conf.json"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(
        config_files, expected,
        "only tauri.conf.json and tauri.windows.conf.json may exist; any other Tauri config file could patch connect-src"
    );
}

/// Tauri derives the Windows bridge origin from each window's
/// `useHttpsScheme` (`http://ipc.localhost` when false, the default;
/// `https://ipc.localhost` when true). `tauri.windows.conf.json` pins the
/// `http` form, so a window switching to the `https` scheme would push every
/// Windows bridge call outside `connect-src` -- and the framework would fall
/// back to `postMessage` silently rather than fail. Only the base config's
/// windows need checking because
/// [`windows_overlay_replaces_connect_src_with_exactly_the_windows_bridge_source`]
/// keeps the overlay to its single `connect-src` key, so it cannot add or
/// patch a window. A window built at runtime is outside this test; VP-S3's
/// per-call transport record covers it.
#[test]
fn no_window_switches_the_bridge_to_the_https_scheme() {
    let config = read_json("tauri.conf.json");
    let windows = config["app"]["windows"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);

    for window in windows {
        assert!(
            window
                .get("useHttpsScheme")
                .is_none_or(|v| v == &Value::Bool(false)),
            "a window enables useHttpsScheme, which moves the Windows bridge to https://ipc.localhost \
             outside the documented connect-src source: {window:?}"
        );
    }
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

/// Tauri v2's recognized `BundleType` values (`tauri-utils::config`), so a
/// typo or unsupported format is caught here instead of failing a build.
const KNOWN_TAURI_BUNDLE_TYPES: &[&str] = &["deb", "rpm", "appimage", "msi", "nsis", "app", "dmg"];

/// docs/evidence/VP-001 design.md § Architecture Decisions D4: CI pins the
/// packaging job to `ubuntu-24.04` and an explicit Tauri CLI version, so the
/// bundle format list must also be explicit rather than the bundler's
/// implicit `"all"` default -- otherwise a future Tauri release could widen
/// or narrow the retained artifact's format with no visible config change.
#[test]
fn bundle_targets_is_an_explicit_non_default_array() {
    let config = read_json("tauri.conf.json");
    let targets = &config["bundle"]["targets"];

    assert!(
        targets.is_array(),
        "bundle.targets must be an explicit array, not the default \"all\" string; found: {targets:?}"
    );

    let targets = targets.as_array().expect("bundle.targets must be an array");
    assert!(
        !targets.is_empty(),
        "bundle.targets must name at least one explicit bundle format"
    );

    for target in targets {
        let target = target
            .as_str()
            .expect("each bundle.targets entry must be a string");
        assert!(
            KNOWN_TAURI_BUNDLE_TYPES.contains(&target),
            "unexpected bundle target '{target}', expected one of {KNOWN_TAURI_BUNDLE_TYPES:?}"
        );
    }
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
