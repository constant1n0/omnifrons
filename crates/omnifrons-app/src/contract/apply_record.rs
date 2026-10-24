//! A fake and a reusable contract for the apply-record port (ADR-0005
//! sub-slice 2c): an `InMemoryApplyRecordStore` that applies the shared
//! naming and ordering rules, and the contract the real store runs
//! against itself.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use omnifrons_domain::definition::IdentityKind;
use omnifrons_domain::definition_plan::{InputFingerprint, Operation, PlanId};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::publication::ProjectIdentity;

use crate::apply_record::{
    AppliedFile, ApplyRecord, ApplyRecordError, ApplyRecordId, ApplyRecordStore, StoredApplyRecord,
    apply_record_name, sort_records_newest_first,
};
use crate::snapshot_store::SnapshotId;

/// An in-memory [`ApplyRecordStore`] test double.
#[derive(Debug, Default)]
pub struct InMemoryApplyRecordStore {
    records: Mutex<HashMap<ProjectIdentity, Vec<StoredApplyRecord>>>,
}

impl InMemoryApplyRecordStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ApplyRecordStore for InMemoryApplyRecordStore {
    fn record(
        &self,
        project: &ProjectIdentity,
        record: &ApplyRecord,
    ) -> Result<ApplyRecordId, ApplyRecordError> {
        let mut records = self.records.lock().expect("records mutex poisoned");
        let entries = records.entry(*project).or_default();
        let id = (0_u32..10_000)
            .map(|sequence| apply_record_name(&record.plan_id, record.taken_at, sequence))
            .find(|candidate| entries.iter().all(|stored| &stored.id != candidate))
            .ok_or(ApplyRecordError::WriteFailed)?;
        entries.push(StoredApplyRecord {
            id: id.clone(),
            record: record.clone(),
        });
        Ok(id)
    }

    fn list(&self, project: &ProjectIdentity) -> Result<Vec<StoredApplyRecord>, ApplyRecordError> {
        let mut listed = self
            .records
            .lock()
            .expect("records mutex poisoned")
            .get(project)
            .cloned()
            .unwrap_or_default();
        sort_records_newest_first(&mut listed);
        Ok(listed)
    }
}

/// A record for plan `[plan_byte; 32]` taken `secs` past the epoch, with
/// one written file and one operation.
#[must_use]
pub fn sample_apply_record(plan_byte: u8, secs: u64) -> ApplyRecord {
    let after = Sha256Digest([0x22; 32]);
    let before = InputFingerprint::Sha256(Sha256Digest([0x11; 32]));
    ApplyRecord {
        plan_id: PlanId([plan_byte; 32]),
        workspace: "workspace".to_string(),
        agent: ".".to_string(),
        taken_at: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
        files: vec![AppliedFile {
            path: "AGENTS.md".to_string(),
            before: before.clone(),
            after: Some(after),
            snapshot: Some(SnapshotId(1)),
        }],
        operations: vec![Operation::WriteAgents {
            source: IdentityKind::Claude,
            after,
            before,
        }],
    }
}

/// The [`ApplyRecordStore`] contract: an empty listing; a record reads
/// back equal; two records of one plan id and one instant both survive
/// under distinct names; newest-first order; another project kept apart.
///
/// # Panics
///
/// Panics on any contract violation.
pub fn apply_record_store_contract<S: ApplyRecordStore>(store: &S) {
    let project = ProjectIdentity(Sha256Digest([7; 32]));
    let other = ProjectIdentity(Sha256Digest([8; 32]));
    assert!(store.list(&project).expect("list").is_empty());

    let first = store
        .record(&project, &sample_apply_record(1, 100))
        .expect("record");
    let listed = store.list(&project).expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, first);
    assert_eq!(listed[0].record, sample_apply_record(1, 100));

    let rerun = store
        .record(&project, &sample_apply_record(1, 100))
        .expect("record");
    assert_ne!(rerun, first, "a rerun never overwrites");
    let newer = store
        .record(&project, &sample_apply_record(2, 200))
        .expect("record");
    let ids: Vec<ApplyRecordId> = store
        .list(&project)
        .expect("list")
        .into_iter()
        .map(|stored| stored.id)
        .collect();
    assert_eq!(ids, vec![newer, rerun, first]);
    assert!(store.list(&other).expect("list").is_empty());
}
