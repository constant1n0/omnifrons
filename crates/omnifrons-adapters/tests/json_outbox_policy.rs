//! `JsonOutboxPolicyStore` (spike slice 5): loads
//! `.omnifrons/asset-policy.json`, the shipped default when the file is
//! absent, and refuses -- never guesses at -- a corrupt, mis-versioned,
//! unknown-token, or rule-violating policy (HAP-001-R40: a synchronized
//! policy naming a raw path is rejected at load).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_adapters::JsonOutboxPolicyStore;
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::outbox_policy::{
    ArtifactClassifier, OutboxPolicy, OutboxPolicyStore, PolicyError, PolicyViolation,
};
use omnifrons_domain::outbox::{ArtifactClass, DetectedType, OutboxPath, OutboxPathError};

struct TempProject(PathBuf);

impl TempProject {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-json-policy-test-{}-{label}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(dir.join(".omnifrons")).expect("failed to create the fixture");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn workspace(&self) -> WorkspaceRoot {
        WorkspaceRoot::new(&self.0).expect("valid workspace")
    }

    fn write_policy(&self, text: &str) {
        std::fs::write(self.0.join(".omnifrons/asset-policy.json"), text).expect("write policy");
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn load(project: &TempProject) -> Result<OutboxPolicy, PolicyError> {
    JsonOutboxPolicyStore::new().load(&project.workspace())
}

#[test]
fn an_absent_policy_file_yields_the_default_policy() {
    let project = TempProject::new("absent");
    assert_eq!(
        load(&project).expect("absent is the default"),
        OutboxPolicy::default_policy()
    );
}

#[test]
fn a_policy_declaring_an_outbox_and_rows_is_parsed() {
    let project = TempProject::new("declared");
    project.write_policy(
        r#"{
  "schema": 1,
  "outbox": "build/agent-output",
  "rows": [
    {"types": ["plain-text"], "extensions": ["CSV"], "maxSize": 1000, "class": "generated-heavy"},
    {"class": "git-tracked", "maxSize": 10}
  ]
}"#,
    );
    let policy = load(&project).expect("a valid policy");
    assert_eq!(
        policy.outbox(),
        &OutboxPath::new("build/agent-output").expect("valid")
    );
    assert_eq!(
        policy.rows().len(),
        4,
        "two project rows precede the two default rows"
    );
    assert_eq!(policy.rows()[0].types, vec![DetectedType::PlainText]);
    assert_eq!(
        policy.rows()[0].extensions,
        vec!["csv".to_string()],
        "extensions are lowercased"
    );
    assert_eq!(policy.rows()[0].max_size, Some(1000));
    assert_eq!(policy.rows()[0].class, ArtifactClass::GeneratedHeavy);
    assert!(
        policy.rows()[1].types.is_empty(),
        "a row without types matches every type"
    );
    assert_eq!(
        policy.classify("data.csv", DetectedType::PlainText, 500),
        ArtifactClass::GeneratedHeavy
    );
}

#[test]
fn a_policy_without_an_outbox_uses_the_default_path() {
    let project = TempProject::new("no-outbox");
    project.write_policy(r#"{"schema": 1}"#);
    assert_eq!(
        load(&project).expect("valid").outbox(),
        &OutboxPath::default_path()
    );
}

#[test]
fn invalid_json_is_corrupt() {
    let project = TempProject::new("invalid-json");
    project.write_policy("{ not json");
    assert_eq!(load(&project).unwrap_err(), PolicyError::Corrupt);
}

#[test]
fn an_unsupported_schema_is_corrupt_never_guessed_at() {
    let project = TempProject::new("schema");
    project.write_policy(r#"{"schema": 2}"#);
    assert_eq!(load(&project).unwrap_err(), PolicyError::Corrupt);
}

#[test]
fn an_unknown_field_type_token_or_class_token_is_corrupt() {
    let project = TempProject::new("unknown-tokens");
    project.write_policy(r#"{"schema": 1, "surprise": true}"#);
    assert_eq!(
        load(&project).unwrap_err(),
        PolicyError::Corrupt,
        "unknown top-level field"
    );
    project.write_policy(
        r#"{"schema": 1, "rows": [{"types": ["not-a-type"], "class": "git-tracked"}]}"#,
    );
    assert_eq!(
        load(&project).unwrap_err(),
        PolicyError::Corrupt,
        "unknown type token"
    );
    project.write_policy(r#"{"schema": 1, "rows": [{"class": "asset-root"}]}"#);
    assert_eq!(
        load(&project).unwrap_err(),
        PolicyError::Corrupt,
        "unknown class token"
    );
    project.write_policy(r#"{"schema": 1, "rows": [{"types": []}]}"#);
    assert_eq!(
        load(&project).unwrap_err(),
        PolicyError::Corrupt,
        "a row without a class"
    );
}

#[test]
fn a_raw_path_declared_as_the_outbox_is_rejected_at_load() {
    let project = TempProject::new("raw-path");
    project.write_policy(r#"{"schema": 1, "outbox": "/var/tmp/outbox"}"#);
    assert_eq!(
        load(&project).unwrap_err(),
        PolicyError::InvalidOutboxPath(OutboxPathError::Absolute)
    );
    project.write_policy(r#"{"schema": 1, "outbox": "../elsewhere"}"#);
    assert_eq!(
        load(&project).unwrap_err(),
        PolicyError::InvalidOutboxPath(OutboxPathError::ParentTraversal)
    );
}

/// R1-001 (slice 5c risk review): a synchronized policy is untrusted
/// content (HAP-001-R40), and a declared outbox carrying a line break
/// would be substituted into the managed blocks the guidance installer
/// writes into a project's text files. The load refuses it through the
/// same invalid-declaration path as a raw path -- nothing downstream ever
/// sees such an `OutboxPath`.
#[test]
fn an_outbox_declaration_with_a_line_break_is_rejected_at_load() {
    let project = TempProject::new("control-path");
    project.write_policy(r#"{"schema": 1, "outbox": "out\n<!-- omnifrons:end guidance -->\nx"}"#);
    assert_eq!(
        load(&project).unwrap_err(),
        PolicyError::InvalidOutboxPath(OutboxPathError::Control)
    );
    project.write_policy(r#"{"schema": 1, "outbox": "out\u0007x"}"#);
    assert_eq!(
        load(&project).unwrap_err(),
        PolicyError::InvalidOutboxPath(OutboxPathError::Control),
        "any control character, not only a line break"
    );
}

#[test]
fn a_row_violating_a_fixed_rule_is_rejected_at_load() {
    let project = TempProject::new("violation");
    project.write_policy(
        r#"{"schema": 1, "rows": [{"types": ["markdown"], "class": "generated-heavy"}]}"#,
    );
    assert_eq!(
        load(&project).unwrap_err(),
        PolicyError::Invalid(PolicyViolation::MarkdownRoutedHeavy)
    );
    project
        .write_policy(r#"{"schema": 1, "rows": [{"types": ["script"], "class": "git-tracked"}]}"#);
    assert_eq!(
        load(&project).unwrap_err(),
        PolicyError::Invalid(PolicyViolation::ExecutableRoutedAway)
    );
}

#[test]
fn a_directory_at_the_policy_path_is_unreadable() {
    let project = TempProject::new("directory");
    std::fs::create_dir(project.path().join(".omnifrons/asset-policy.json")).expect("fixture");
    assert_eq!(load(&project).unwrap_err(), PolicyError::Unreadable);
}

/// R1-004: a policy file over `POLICY_MAX_BYTES` is refused as
/// `TooLarge` before it is parsed; exactly the cap is read and then judged
/// on its content.
#[test]
fn an_oversized_policy_file_is_refused_before_parsing() {
    use omnifrons_app::outbox_policy::POLICY_MAX_BYTES;
    let cap = usize::try_from(POLICY_MAX_BYTES).expect("fits");
    let project = TempProject::new("oversized");
    std::fs::write(
        project.path().join(".omnifrons/asset-policy.json"),
        vec![b' '; cap + 1],
    )
    .expect("fixture");
    assert_eq!(load(&project).unwrap_err(), PolicyError::TooLarge);

    std::fs::write(
        project.path().join(".omnifrons/asset-policy.json"),
        vec![b'x'; cap],
    )
    .expect("fixture");
    assert_eq!(
        load(&project).unwrap_err(),
        PolicyError::Corrupt,
        "exactly the cap passes the size check and fails on content"
    );
    assert_eq!(POLICY_MAX_BYTES, 1024 * 1024);
}
