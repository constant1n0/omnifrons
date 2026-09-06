//! Managed state for the executable-identity-and-approval surface
//! (`docs/spike-log.md` § Slice 2): a `LaunchGate` over the real
//! prober/store/clock, the probed-candidate table `executable_pick_and_probe`
//! populates, and the approval store's own directory.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use omnifrons_adapters::{FsExecutableProber, JsonlApprovalStore};
use omnifrons_app::{ApprovalStoreError, Clock, LaunchGate};
use omnifrons_domain::executable::ExecutableIdentity;

/// The real wall clock, backing the managed [`LaunchGate`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// A probed-but-not-yet-approved candidate's opaque handle. Valid only for
/// this shell process's own lifetime -- never persisted, never stable
/// across a restart (`crate::ipc::dto::CandidateIdDto` is its wire form).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CandidateId(u64);

impl CandidateId {
    /// Build a `CandidateId` from its raw wire value
    /// (`crate::ipc::dto::CandidateIdDto`'s own field).
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// This id's raw wire value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// The maximum number of probed candidates remembered at once. Bounded so
/// repeatedly probing candidates that are never approved cannot grow this
/// table without limit; the oldest is evicted first.
const MAX_CANDIDATES: usize = 32;

/// Probed-but-not-yet-approved candidates: bounded, oldest evicted.
#[derive(Default)]
pub struct CandidateTable {
    next_id: u64,
    order: VecDeque<CandidateId>,
    entries: HashMap<CandidateId, ExecutableIdentity>,
}

impl CandidateTable {
    /// Remember `identity` under a fresh id, evicting the oldest entry if
    /// this table is now over [`MAX_CANDIDATES`].
    pub fn insert(&mut self, identity: ExecutableIdentity) -> CandidateId {
        self.next_id += 1;
        let id = CandidateId(self.next_id);
        self.order.push_back(id);
        self.entries.insert(id, identity);
        if self.order.len() > MAX_CANDIDATES
            && let Some(evicted) = self.order.pop_front()
        {
            self.entries.remove(&evicted);
        }
        id
    }

    /// Look up a previously probed candidate by id.
    #[must_use]
    pub fn get(&self, id: CandidateId) -> Option<&ExecutableIdentity> {
        self.entries.get(&id)
    }
}

/// The concrete `LaunchGate` type this shell manages.
pub type ShellLaunchGate = LaunchGate<FsExecutableProber, JsonlApprovalStore, SystemClock>;

/// This shell's managed state for the executable-identity-and-approval
/// surface.
///
/// `store_dir` is kept alongside the gate (not only inside it) so
/// `approvals_list` and the approved-launch path can open a second,
/// independent [`JsonlApprovalStore`] handle without needing `&mut`
/// access to the gate itself: the store is stateless beyond its own path,
/// so two handles pointed at the same file always agree
/// (`crates/omnifrons-adapters/src/jsonl_approval_store.rs`'s own doc
/// comment).
pub struct ExecutableState {
    pub gate: Mutex<ShellLaunchGate>,
    pub candidates: Mutex<CandidateTable>,
    store_dir: PathBuf,
}

impl ExecutableState {
    /// Build the managed state, opening (or creating) the approval store
    /// directly under `store_dir`.
    ///
    /// # Errors
    ///
    /// Returns [`ApprovalStoreError`] if the store could not be opened.
    pub fn open(store_dir: PathBuf) -> Result<Self, ApprovalStoreError> {
        let store = JsonlApprovalStore::open(store_dir.clone())?;
        let gate = LaunchGate::new(FsExecutableProber::new(), store, SystemClock);
        Ok(Self {
            gate: Mutex::new(gate),
            candidates: Mutex::new(CandidateTable::default()),
            store_dir,
        })
    }

    /// Open a fresh, independent handle to the same backing approval
    /// store this state's gate uses.
    ///
    /// # Errors
    ///
    /// Returns [`ApprovalStoreError`] if the store could not be opened.
    pub fn open_store(&self) -> Result<JsonlApprovalStore, ApprovalStoreError> {
        JsonlApprovalStore::open(self.store_dir.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{CandidateTable, MAX_CANDIDATES};
    use omnifrons_domain::executable::{ExecutableIdentity, PlatformEvidence, Sha256Digest};
    use std::path::PathBuf;

    fn identity(n: u8) -> ExecutableIdentity {
        ExecutableIdentity {
            canonical_path: PathBuf::from(format!("/opt/tool/app-{n}")),
            size: 4096,
            sha256: Sha256Digest([n; 32]),
            modified_at: None,
            platform: PlatformEvidence::Unix { mode: 0o755 },
        }
    }

    /// R3-009: the `(MAX_CANDIDATES + 1)`th insert must evict the oldest
    /// entry, never grow the table without bound. `MAX_CANDIDATES` is
    /// `32`, so the 33rd insert is this test's own trigger.
    #[test]
    fn the_33rd_insert_evicts_the_oldest_entry() {
        let mut table = CandidateTable::default();
        let mut ids = Vec::with_capacity(MAX_CANDIDATES + 1);

        for n in 0..=MAX_CANDIDATES {
            // `n` doesn't fit in a `u8` once it reaches 256, but
            // `MAX_CANDIDATES` (32) is comfortably within range.
            ids.push(table.insert(identity(u8::try_from(n).expect("n fits in u8"))));
        }

        assert_eq!(ids.len(), MAX_CANDIDATES + 1);
        let oldest = ids[0];
        let newest = *ids.last().expect("at least one id was inserted");

        assert!(
            table.get(oldest).is_none(),
            "the oldest entry must be evicted once the table exceeds MAX_CANDIDATES"
        );
        assert!(
            table.get(newest).is_some(),
            "the newest entry must still be present"
        );
    }
}
