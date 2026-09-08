//! `JsonOutboxPolicyStore` reads the policy's `assetRootId` (spike slice
//! 5b): a valid token loads as the policy's asset root identity, an absent
//! field leaves it undeclared, and a token that is not one path component
//! -- a raw path, a traversal -- is corrupt (HAP-001-R40: a synchronized
//! policy never names a raw path).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_adapters::JsonOutboxPolicyStore;
use omnifrons_app::WorkspaceRoot;
use omnifrons_app::outbox_policy::{OutboxPolicyStore, POLICY_FILE_PATH, PolicyError};
use omnifrons_domain::publication::AssetRootId;

struct TempProject(PathBuf);

impl TempProject {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-policy-asset-root-test-{}-{label}-{n}",
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

    fn write_policy(&self, json: &str) {
        let path = self.path().join(POLICY_FILE_PATH);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, json).expect("write policy");
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_declared_asset_root_id_loads() {
    let project = TempProject::new("declared");
    project.write_policy(r#"{"schema": 1, "assetRootId": "main"}"#);
    let policy = JsonOutboxPolicyStore::new()
        .load(&project.workspace())
        .expect("loads");
    assert_eq!(
        policy.asset_root_id(),
        Some(&AssetRootId::new("main").expect("valid"))
    );
}

#[test]
fn an_absent_asset_root_id_stays_undeclared() {
    let project = TempProject::new("absent");
    project.write_policy(r#"{"schema": 1}"#);
    let policy = JsonOutboxPolicyStore::new()
        .load(&project.workspace())
        .expect("loads");
    assert_eq!(policy.asset_root_id(), None);
}

#[test]
fn an_asset_root_id_that_is_not_one_token_is_corrupt() {
    for token in ["a/b", "..", "", "/var/assets"] {
        let project = TempProject::new("invalid");
        project.write_policy(&format!(r#"{{"schema": 1, "assetRootId": "{token}"}}"#));
        let result = JsonOutboxPolicyStore::new().load(&project.workspace());
        assert!(
            matches!(result, Err(PolicyError::Corrupt)),
            "{token:?} must be corrupt, got {result:?}"
        );
    }
}
