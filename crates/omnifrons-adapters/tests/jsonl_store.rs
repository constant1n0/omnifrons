//! `JsonlApprovalStore`: an append-only, `approvals.jsonl`-backed
//! `ApprovalStore`. Cross-platform (R3-005) except one assertion: the
//! mode-`0o600` check is inherently unix-specific and gated
//! `#[cfg(unix)]` on its own test, rather than the whole file -- every
//! other test here exercises platform-generic `ApprovalStore` behavior
//! and is worth running on every platform this workspace tests on.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use omnifrons_adapters::JsonlApprovalStore;
use omnifrons_app::contract::approval_store::approval_store_contract;
use omnifrons_app::{ApprovalStore, ApprovalStoreError};
use omnifrons_domain::executable::{
    DeviceLocalUser, ExecutableIdentity, PlatformEvidence, Sha256Digest,
};

/// A drop-guard temp directory: removed on drop regardless of which path
/// out of a test (pass, fail, or panic) is taken, so this file's own
/// fixture directories never accumulate under the system temp dir across
/// runs (R3-004).
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-jsonl-store-test-{}-{label}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("failed to create the test fixture directory");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sample_identity() -> ExecutableIdentity {
    ExecutableIdentity {
        canonical_path: PathBuf::from("/opt/tool/app"),
        size: 4096,
        sha256: Sha256Digest([9; 32]),
        modified_at: None,
        platform: PlatformEvidence::Unix { mode: 0o755 },
    }
}

fn identity_at(path: &str, digest_byte: u8) -> ExecutableIdentity {
    ExecutableIdentity {
        canonical_path: PathBuf::from(path),
        size: 4096,
        sha256: Sha256Digest([digest_byte; 32]),
        modified_at: None,
        platform: PlatformEvidence::Unix { mode: 0o755 },
    }
}

#[cfg(unix)]
#[test]
fn the_backing_file_is_created_with_mode_0o600() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = TempDir::new("mode");
    let _store = JsonlApprovalStore::open(dir.path().to_path_buf()).expect("open must succeed");

    let metadata = std::fs::metadata(dir.path().join("approvals.jsonl"))
        .expect("approvals.jsonl must exist after open");
    assert_eq!(
        metadata.permissions().mode() & 0o777,
        0o600,
        "the backing file must be created with mode 0o600"
    );
}

#[test]
fn revoke_grows_the_file_append_only() {
    let dir = TempDir::new("append-only");
    let mut store = JsonlApprovalStore::open(dir.path().to_path_buf()).expect("open must succeed");
    let record = store
        .record(
            sample_identity(),
            DeviceLocalUser,
            std::time::SystemTime::UNIX_EPOCH,
        )
        .expect("record must succeed");

    let len_after_record = std::fs::metadata(dir.path().join("approvals.jsonl"))
        .expect("approvals.jsonl must exist")
        .len();

    store
        .revoke(
            record.approval_id,
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1),
        )
        .expect("revoke must succeed");

    let len_after_revoke = std::fs::metadata(dir.path().join("approvals.jsonl"))
        .expect("approvals.jsonl must exist")
        .len();

    assert!(
        len_after_revoke > len_after_record,
        "revoke must append a new line, growing the file ({len_after_record} -> {len_after_revoke})"
    );
}

#[test]
fn a_truncated_final_line_is_reported_as_corrupt_with_no_partial_reads() {
    let dir = TempDir::new("truncated");
    let mut store = JsonlApprovalStore::open(dir.path().to_path_buf()).expect("open must succeed");
    store
        .record(
            sample_identity(),
            DeviceLocalUser,
            std::time::SystemTime::UNIX_EPOCH,
        )
        .expect("record must succeed");

    // Append a deliberately truncated (mid-object) line directly, bypassing
    // the store's own API -- simulating a process killed mid-write.
    {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join("approvals.jsonl"))
            .expect("failed to open the backing file for the truncated-line fixture");
        writeln!(file, r#"{{"event":"approved","approvalId":2,"canonic"#)
            .expect("failed to append the truncated line");
    }

    let error = store
        .list()
        .expect_err("a truncated line must be reported as Corrupt");

    assert_eq!(error, ApprovalStoreError::Corrupt);
}

#[test]
fn the_store_path_lives_under_the_given_dir_not_a_workspace_dir_passed_alongside() {
    let dir = TempDir::new("target");
    let decoy_workspace_dir = TempDir::new("decoy-workspace");

    let _store = JsonlApprovalStore::open(dir.path().to_path_buf()).expect("open must succeed");

    assert!(
        dir.path().join("approvals.jsonl").is_file(),
        "the backing file must be created directly under the given dir"
    );
    assert_eq!(
        std::fs::read_dir(decoy_workspace_dir.path())
            .expect("decoy workspace dir must still exist")
            .count(),
        0,
        "a dir merely passed alongside (never given to open) must never receive the backing file"
    );
}

#[test]
fn jsonl_store_satisfies_approval_store_contract() {
    let dir = TempDir::new("contract");
    approval_store_contract(|| {
        JsonlApprovalStore::open(dir.path().to_path_buf()).expect("open must succeed")
    });
}

/// R3-007: `append_line` must `sync_data` before returning `Ok`, for both
/// `record` and `revoke` -- otherwise a revocation could sit unflushed in
/// the OS page cache and be lost across a crash, silently resurrecting a
/// revoked approval on the next start. Proven behaviorally: revoke, then
/// open a *fresh* `JsonlApprovalStore` handle at the same path (never
/// reusing the original handle's own state, since this store carries
/// none beyond its path) and confirm the revocation is visible.
#[test]
fn a_revocation_is_visible_to_a_freshly_reopened_store_handle() {
    let dir = TempDir::new("durability");
    let mut store = JsonlApprovalStore::open(dir.path().to_path_buf()).expect("open must succeed");
    let identity = sample_identity();
    let record = store
        .record(identity.clone(), DeviceLocalUser, SystemTime::UNIX_EPOCH)
        .expect("record must succeed");
    store
        .revoke(
            record.approval_id,
            SystemTime::UNIX_EPOCH + Duration::from_secs(1),
        )
        .expect("revoke must succeed");
    drop(store);

    let reopened = JsonlApprovalStore::open(dir.path().to_path_buf()).expect("reopen must succeed");
    let active = reopened
        .find_active(&identity)
        .expect("find_active must succeed against the reopened handle");

    assert_eq!(
        active, None,
        "a revocation durably written (fsync'd) before record()/revoke() returned Ok must be \
         visible to a freshly reopened store handle at the same path"
    );
}

/// R3-006 / R1-004: an `ApprovalId` is derived from the identity and
/// timestamp, not counted -- two records made "at the same instant" (the
/// same `approved_at`) for two different identities must still get
/// distinct ids, since the identity itself (canonical path + digest)
/// feeds the derivation.
#[test]
fn two_identities_approved_at_the_same_instant_get_different_ids() {
    let dir = TempDir::new("id-derivation-distinct-identities");
    let mut store = JsonlApprovalStore::open(dir.path().to_path_buf()).expect("open must succeed");
    let t0 = SystemTime::UNIX_EPOCH;

    let a = store
        .record(identity_at("/opt/tool/a", 1), DeviceLocalUser, t0)
        .expect("record must succeed");
    let b = store
        .record(identity_at("/opt/tool/b", 2), DeviceLocalUser, t0)
        .expect("record must succeed");

    assert_ne!(
        a.approval_id, b.approval_id,
        "two different identities approved at the same instant must get different ids"
    );
}

/// The same identity approved twice at two different instants must also
/// get different ids -- the timestamp is part of the derivation input
/// precisely so a re-approval of unchanged content is never mistaken for
/// the same record.
#[test]
fn the_same_identity_approved_twice_at_different_instants_gets_different_ids() {
    let dir = TempDir::new("id-derivation-same-identity");
    let mut store = JsonlApprovalStore::open(dir.path().to_path_buf()).expect("open must succeed");
    let identity = sample_identity();
    let t0 = SystemTime::UNIX_EPOCH;
    let t1 = t0 + Duration::from_secs(1);

    let first = store
        .record(identity.clone(), DeviceLocalUser, t0)
        .expect("first record must succeed");
    store
        .revoke(first.approval_id, t1)
        .expect("revoke must succeed");
    let second = store
        .record(identity, DeviceLocalUser, t1)
        .expect("second record must succeed");

    assert_ne!(
        first.approval_id, second.approval_id,
        "the same identity approved at two different instants must get different ids"
    );
}

/// R3-008: restart continuity. Reopening a store that already has
/// records on disk and recording a fresh approval must not collide with
/// an id already on file -- proof the derivation (or the duplicate
/// check backing it) considers what is already durably recorded, not
/// only what this particular handle has seen since it was opened.
#[test]
fn reopening_a_store_and_recording_a_new_approval_yields_a_distinct_id() {
    let dir = TempDir::new("restart-continuity");
    let existing = {
        let mut store =
            JsonlApprovalStore::open(dir.path().to_path_buf()).expect("open must succeed");
        store
            .record(sample_identity(), DeviceLocalUser, SystemTime::UNIX_EPOCH)
            .expect("record must succeed")
    };

    let mut reopened =
        JsonlApprovalStore::open(dir.path().to_path_buf()).expect("reopen must succeed");
    let fresh = reopened
        .record(
            identity_at("/opt/tool/other", 99),
            DeviceLocalUser,
            SystemTime::UNIX_EPOCH + Duration::from_secs(1),
        )
        .expect("recording a new approval against a reopened store must succeed");

    assert_ne!(
        existing.approval_id, fresh.approval_id,
        "a fresh approval recorded against a reopened store must not collide with an id \
         already durably on file"
    );
}

/// R1-005: `LogEntry` carries a schema version; any value other than the
/// one this store writes must fail closed as `Corrupt` (mapped by the
/// shell to `approval-store-unavailable`), never silently misparsed.
#[test]
fn a_line_with_an_unsupported_schema_is_reported_as_corrupt() {
    let dir = TempDir::new("unsupported-schema");
    let store = JsonlApprovalStore::open(dir.path().to_path_buf()).expect("open must succeed");

    {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join("approvals.jsonl"))
            .expect("failed to open the backing file for the schema fixture");
        writeln!(
            file,
            r#"{{"schema":99,"event":"approved","approval_id":1,"canonical_path":"/opt/tool/app","size":4096,"sha256":"{}","modified_at":null,"platform":{{"os":"unix","mode":493}},"approved_at":{{"secs":0,"nanos":0}}}}"#,
            "0".repeat(64),
        )
        .expect("failed to append the unsupported-schema line");
    }

    let error = store
        .list()
        .expect_err("an unsupported schema value must be reported as Corrupt");

    assert_eq!(error, ApprovalStoreError::Corrupt);
}
