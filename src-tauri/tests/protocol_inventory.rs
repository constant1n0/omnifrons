//! Pins the static half of VP-001 VP-S3's ("Handler inventory per OS",
//! docs/desktop-stack-verification-plan.md § Scenario catalog) two-part
//! evidence: the protocol handlers and policy schemes this build carries by
//! source and build configuration alone, independent of any packaged run.
//! The runtime half -- observing what the packaged `AppImage` actually serves
//! -- is a separate probe, not this file.
//!
//! The inventory, derived from the pinned Tauri version's own scheme
//! registration (Cargo.lock `tauri` 2.11.5;
//! `tauri-2.11.5/src/manager/webview.rs:229-365`): Tauri always registers its
//! own `tauri` scheme (serves the frontend) and `ipc` (the typed-IPC bridge),
//! plus every scheme the app itself registers via
//! `register_uri_scheme_protocol`/`register_asynchronous_uri_scheme_protocol`
//! (none today -- see [`app_registers_no_uri_scheme`]), every scheme a
//! plugin registers through its builder (the dialog and fs plugins in the
//! graph register none -- see [`no_other_crate_can_register_a_scheme`]) and,
//! only under the `protocol-asset` or `isolation` Cargo features (neither
//! enabled -- see [`tauri_is_resolved_without_scheme_adding_features`]), an
//! `asset` scheme or an isolation scheme. So today the only registered
//! handlers are `tauri` and `ipc`.
//!
//! `src-tauri/tests/config.rs` already pins the CSP's exact directive values
//! and the merged per-OS config; this file does not repeat those assertions,
//! only the scheme/URL sources the CSP names as policy (see
//! [`csp_scheme_sources_are_the_inventoried_ones`]), which is `ipc:` /
//! `http://ipc.localhost` in `connect-src` (the typed-IPC bridge, per OS) and
//! `artifact:` in `img-src`/`media-src`. `artifact:` has **no registered
//! handler**: RCS-001 § CSP baseline reserves it for the artifact tier, and
//! it gains a handler only together with that tier.
//!
//! A Tauri version bump must re-check `src/manager/webview.rs`'s scheme
//! registration logic: this inventory is derived from that source, not from
//! Tauri's public API surface, and a new version could change which schemes
//! get registered and under which feature.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn read_json(relative_path: &str) -> Value {
    let path = format!("{}/{relative_path}", env!("CARGO_MANIFEST_DIR"));
    let raw = fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("failed to parse {path} as JSON: {e}"))
}

fn rs_files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("failed to list {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("directory entry must be readable");
        let path = entry.path();
        if path.is_dir() {
            rs_files_under(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Tauri's app-registered handler inventory (Cargo.lock `tauri` 2.11.5;
/// `tauri-2.11.5/src/manager/webview.rs:229-365`) is closed to two calls,
/// including their builder form (`.register_uri_scheme_protocol(`), which
/// this substring match also catches. If this ever fires, the static
/// inventory in this file's module doc comment is stale and must be updated
/// alongside whichever new scheme the app now registers.
#[test]
fn app_registers_no_uri_scheme() {
    let src_dir = format!("{}/src", env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rs_files_under(Path::new(&src_dir), &mut files);
    assert!(
        !files.is_empty(),
        "expected to find .rs files under {src_dir}"
    );

    for path in files {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        assert!(
            !source.contains("register_uri_scheme_protocol")
                && !source.contains("register_asynchronous_uri_scheme_protocol"),
            "{} registers a URI scheme, widening Tauri's handler inventory beyond `tauri` and \
             `ipc` pinned in this file's module doc comment",
            path.display()
        );
    }
}

/// The resolved workspace graph (`cargo metadata --locked`), the same one
/// `cargo` builds from, so feature unification and transitive crates are
/// visible rather than inferred from `Cargo.toml`.
fn cargo_metadata() -> Value {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri must have a parent directory, the workspace root");
    let manifest_path = workspace_root.join("Cargo.toml");

    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--manifest-path",
        ])
        .arg(&manifest_path)
        .output()
        .unwrap_or_else(|e| panic!("failed to run cargo metadata: {e}"));

    assert!(
        output.status.success(),
        "cargo metadata failed (exit {:?}): {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("failed to parse cargo metadata stdout as JSON: {e}"))
}

/// Crates other than `omnifrons-shell` that depend on `tauri`, checked at
/// exactly these versions to register no URI scheme (neither
/// `register_uri_scheme_protocol` nor `register_asynchronous_uri_scheme_protocol`
/// appears in their `src/`): the dialog plugin the shell initialises
/// (`src/lib.rs`) and the fs plugin it pulls in. Any other crate that
/// depends on `tauri`, whatever its name, and any version change must repeat
/// that source check before it is listed here.
const SCHEME_FREE_TAURI_DEPENDENTS: &[(&str, &str)] = &[
    ("tauri-plugin-dialog", "2.7.3"),
    ("tauri-plugin-fs", "2.5.2"),
];

/// Everything in the resolved graph that could register a URI scheme the
/// source scan in [`app_registers_no_uri_scheme`] would not see: a second
/// `tauri` (whose features the feature check might not inspect), and any
/// crate that depends on `tauri` -- plugin, helper library or workspace
/// member, regardless of its name -- other than the `omnifrons-shell`
/// workspace member and the [`SCHEME_FREE_TAURI_DEPENDENTS`] at their checked
/// versions. Registering a scheme on Tauri's webviews goes through `tauri`'s
/// own API, so it takes a crate that depends on `tauri`.
fn dependency_graph_violations(metadata: &Value) -> Vec<String> {
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata output must carry a packages array");
    let members: BTreeSet<&str> = metadata["workspace_members"]
        .as_array()
        .expect("cargo metadata output must carry a workspace_members array")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut violations = Vec::new();

    let tauri_count = packages.iter().filter(|p| p["name"] == "tauri").count();
    if tauri_count != 1 {
        violations.push(format!(
            "expected exactly one `tauri` package, found {tauri_count}"
        ));
    }

    for package in packages {
        let name = package["name"].as_str().unwrap_or_default();
        let version = package["version"].as_str().unwrap_or_default();
        let depends_on_tauri = package["dependencies"]
            .as_array()
            .is_some_and(|deps| deps.iter().any(|d| d["name"] == "tauri"));
        let is_the_shell = name == "omnifrons-shell"
            && package["id"]
                .as_str()
                .is_some_and(|id| members.contains(id));
        if depends_on_tauri
            && !is_the_shell
            && !SCHEME_FREE_TAURI_DEPENDENTS.contains(&(name, version))
        {
            violations.push(format!(
                "`{name}` {version} depends on `tauri` and is not checked for URI scheme registration"
            ));
        }
    }

    violations
}

/// Makes the module doc comment's "only `tauri` and `ipc`" hold for the
/// whole build, not just for `src-tauri/src`.
#[test]
fn no_other_crate_can_register_a_scheme() {
    let violations = dependency_graph_violations(&cargo_metadata());
    assert!(
        violations.is_empty(),
        "the resolved graph can register URI schemes the static inventory does not account for: \
         {violations:?}"
    );
}

/// Guards [`dependency_graph_violations`] itself, one detection path per
/// violation: a second `tauri`, a workspace crate other than the shell that
/// depends on `tauri`, a checked plugin at another version, a plugin never
/// checked, a `tauri` dependent that does not follow the plugin naming
/// convention, and a non-member crate borrowing the shell's name. The clean
/// shape -- the checked dependents at their checked versions -- yields none.
#[test]
fn dependency_graph_violations_flags_each_way_a_scheme_could_slip_in() {
    let on_tauri = serde_json::json!([{"name": "tauri"}]);
    let clean = serde_json::json!({
        "workspace_members": ["shell-id", "domain-id"],
        "packages": [
            {"id": "shell-id", "name": "omnifrons-shell", "version": "0.1.0", "dependencies": on_tauri},
            {"id": "domain-id", "name": "omnifrons-domain", "version": "0.1.0", "dependencies": []},
            {"id": "tauri-id", "name": "tauri", "version": "2.11.5", "dependencies": []},
            {"id": "dialog-id", "name": "tauri-plugin-dialog", "version": "2.7.3", "dependencies": on_tauri},
            {"id": "fs-id", "name": "tauri-plugin-fs", "version": "2.5.2", "dependencies": on_tauri}
        ]
    });
    assert_eq!(dependency_graph_violations(&clean), Vec::<String>::new());

    let widened = serde_json::json!({
        "workspace_members": ["shell-id", "domain-id"],
        "packages": [
            {"id": "shell-id", "name": "omnifrons-shell", "version": "0.1.0", "dependencies": on_tauri},
            {"id": "domain-id", "name": "omnifrons-domain", "version": "0.1.0", "dependencies": on_tauri},
            {"id": "tauri-id", "name": "tauri", "version": "2.11.5", "dependencies": []},
            {"id": "tauri-old-id", "name": "tauri", "version": "2.0.0", "dependencies": []},
            {"id": "dialog-id", "name": "tauri-plugin-dialog", "version": "2.8.0", "dependencies": on_tauri},
            {"id": "shell-plugin-id", "name": "tauri-plugin-shell", "version": "2.0.0", "dependencies": on_tauri},
            {"id": "helper-id", "name": "scheme-helper", "version": "1.0.0", "dependencies": on_tauri},
            {"id": "impostor-id", "name": "omnifrons-shell", "version": "9.9.9", "dependencies": on_tauri}
        ]
    });
    let mut violations = dependency_graph_violations(&widened);
    violations.sort();
    let mut expected = vec![
        "expected exactly one `tauri` package, found 2".to_string(),
        "`omnifrons-domain` 0.1.0 depends on `tauri` and is not checked for URI scheme registration"
            .to_string(),
        "`tauri-plugin-dialog` 2.8.0 depends on `tauri` and is not checked for URI scheme registration"
            .to_string(),
        "`tauri-plugin-shell` 2.0.0 depends on `tauri` and is not checked for URI scheme registration"
            .to_string(),
        "`scheme-helper` 1.0.0 depends on `tauri` and is not checked for URI scheme registration"
            .to_string(),
        "`omnifrons-shell` 9.9.9 depends on `tauri` and is not checked for URI scheme registration"
            .to_string(),
    ];
    expected.sort();
    assert_eq!(violations, expected);
}

/// Tauri only registers an extra `asset` scheme under its `protocol-asset`
/// feature, or an isolation scheme under `isolation` (`webview.rs:229-365`).
/// This resolves the actual feature set `cargo` builds `tauri` with --
/// rather than trusting `Cargo.toml`'s declared dependency, which cannot by
/// itself show what feature unification across the workspace resolved to --
/// so a feature silently pulled in by another crate would still be caught.
#[test]
fn tauri_is_resolved_without_scheme_adding_features() {
    let metadata = cargo_metadata();

    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata output must carry a packages array");
    let tauri_id =
        packages
            .iter()
            .find(|package| package["name"] == "tauri")
            .unwrap_or_else(|| {
                panic!("cargo metadata packages must include a package named tauri")
            })["id"]
            .clone();

    let resolve_nodes = metadata["resolve"]["nodes"]
        .as_array()
        .expect("cargo metadata output must carry a resolve.nodes array");
    let tauri_node = resolve_nodes
        .iter()
        .find(|node| node["id"] == tauri_id)
        .unwrap_or_else(|| panic!("cargo metadata resolve.nodes must include tauri's node"));

    let features: Vec<&str> = tauri_node["features"]
        .as_array()
        .expect("tauri's resolve node must carry a features array")
        .iter()
        .map(|feature| {
            feature
                .as_str()
                .expect("each resolved feature must be a string")
        })
        .collect();

    assert!(
        !features.contains(&"protocol-asset"),
        "tauri is resolved with the protocol-asset feature, which registers an extra `asset` \
         scheme this inventory does not account for; resolved features: {features:?}"
    );
    assert!(
        !features.contains(&"isolation"),
        "tauri is resolved with the isolation feature, which registers an extra isolation scheme \
         this inventory does not account for; resolved features: {features:?}"
    );
}

/// A "scheme source" per the Fetch/CSP source-list grammar: either a bare
/// scheme (`ipc:`) or a URL carrying one (`http://ipc.localhost`) -- never a
/// quoted keyword like `'self'` or `'none'`, which never ends with `:` or
/// carries `://`.
fn scheme_sources(csp_value: &str) -> BTreeSet<String> {
    csp_value
        .split_whitespace()
        .filter(|token| token.ends_with(':') || token.contains("://"))
        .map(String::from)
        .collect()
}

/// Guards [`scheme_sources`] itself against under-collection: a helper that
/// silently ignored an extra scheme source would let
/// [`csp_scheme_sources_are_the_inventoried_ones`] pass even if the CSP grew
/// an undocumented one, so this proves the extraction sees an extra source.
#[test]
fn scheme_sources_collects_every_scheme_and_url_token() {
    let extracted = scheme_sources("'self' ipc: https://example.invalid 'none' artifact:");
    assert_eq!(
        extracted,
        BTreeSet::from([
            String::from("ipc:"),
            String::from("https://example.invalid"),
            String::from("artifact:"),
        ])
    );
}

/// The complete set of scheme/URL sources the CSP names as policy, across
/// every directive in the base config and the Windows overlay
/// (`src-tauri/tests/config.rs` already pins the exact directive values and
/// the merged per-OS config -- this only pins which sources are scheme
/// sources). `artifact:` in `img-src`/`media-src` is a policy scheme with
/// **no registered handler** (see this file's module doc comment and
/// [`app_registers_no_uri_scheme`]): RCS-001 reserves it for the artifact
/// tier, so it stays in the policy with no handler until that tier lands.
#[test]
fn csp_scheme_sources_are_the_inventoried_ones() {
    let base = read_json("tauri.conf.json");
    let overlay = read_json("tauri.windows.conf.json");

    let base_csp = base["app"]["security"]["csp"]
        .as_object()
        .expect("base csp must be an object");
    let overlay_csp = overlay["app"]["security"]["csp"]
        .as_object()
        .expect("overlay csp must be an object");

    let mut found: BTreeMap<(String, &'static str), BTreeSet<String>> = BTreeMap::new();
    for (directive, value) in base_csp {
        let value = value
            .as_str()
            .expect("csp directive value must be a string");
        let sources = scheme_sources(value);
        if !sources.is_empty() {
            found.insert((directive.clone(), "base"), sources);
        }
    }
    for (directive, value) in overlay_csp {
        let value = value
            .as_str()
            .expect("csp directive value must be a string");
        let sources = scheme_sources(value);
        if !sources.is_empty() {
            found.insert((directive.clone(), "windows"), sources);
        }
    }

    let mut expected: BTreeMap<(String, &'static str), BTreeSet<String>> = BTreeMap::new();
    expected.insert(
        ("connect-src".to_string(), "base"),
        BTreeSet::from([String::from("ipc:")]),
    );
    expected.insert(
        ("connect-src".to_string(), "windows"),
        BTreeSet::from([String::from("http://ipc.localhost")]),
    );
    expected.insert(
        ("img-src".to_string(), "base"),
        BTreeSet::from([String::from("artifact:")]),
    );
    expected.insert(
        ("media-src".to_string(), "base"),
        BTreeSet::from([String::from("artifact:")]),
    );

    assert_eq!(
        found, expected,
        "the CSP's scheme/URL sources across every directive and OS must be exactly the \
         inventoried ones, nothing else"
    );
}
