//! A reusable `ApprovalStore` contract, runnable against any
//! implementation, plus the fakes (`FakeProber`, `InMemoryApprovalStore`,
//! `FixedClock`) `omnifrons-app`'s own `LaunchGate` tests and a real
//! adapter's contract test both drive.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use omnifrons_domain::executable::{
    ApprovalId, ApprovalRecord, ApprovalStatus, DeviceLocalUser, ExecutableIdentity,
    PlatformEvidence, ProbeOutcome as ScriptedProbeOutcome, Sha256Digest,
};

use crate::approval_store::{ApprovalStore, ApprovalStoreError};
use crate::clock::Clock;
use crate::executable_prober::{ExecHandle, ExecutableProber, ProbeOutcome, ProbedExecutable};

/// Build a real, throwaway, already-open file handle for a fake
/// [`ProbedExecutable`] to carry -- never a fabricated or dangling one,
/// keeping [`FakeProber`] honest about what a `ProbedExecutable` promises
/// even though the file's actual content is irrelevant to any test using
/// it (only the accompanying [`ExecutableIdentity`], scripted separately,
/// is ever asserted on).
///
/// Unlinked immediately after opening on unix, where that leaves the
/// returned handle perfectly valid (an open file description outlives
/// the directory entry that created it) while never leaving a stray file
/// behind; left in place on every other platform, where deleting an
/// open file is not generally possible.
fn fake_handle_file() -> std::fs::File {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "omnifrons-fake-prober-handle-{}-{n}",
        std::process::id()
    ));
    let file =
        std::fs::File::create(&path).expect("FakeProber's fake handle temp file must be creatable");
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(&path);
    }
    file
}

/// Wrap `identity` into the [`ProbeOutcome::Identity`] a [`FakeProber`]
/// can be scripted with, backed by a real (if content-irrelevant) open
/// handle from [`fake_handle_file`].
#[must_use]
pub fn fake_probed_identity(identity: ExecutableIdentity) -> ProbeOutcome {
    ProbeOutcome::Identity(ProbedExecutable {
        identity,
        handle: ExecHandle::File(fake_handle_file()),
    })
}

/// A scripted [`ExecutableProber`] test double: returns a fixed or
/// scripted sequence of outcomes regardless of the candidate path, and
/// counts how many times it was called.
///
/// Scripted with the plain, `Clone`-able domain
/// `omnifrons_domain::executable::ProbeOutcome` (aliased here as
/// `ScriptedProbeOutcome`) -- a *recipe*, not the app-level
/// [`ProbeOutcome`] `probe` actually returns -- because the app-level
/// type wraps a live [`ExecHandle`], which is neither `Clone` nor a
/// sensible thing to script by value. Each `probe` call materializes a
/// fresh app-level outcome from that recipe, backed by a genuine (if
/// content-irrelevant) open handle for the `Identity` case
/// ([`fake_probed_identity`]), so this fake never lies about
/// `ProbedExecutable`'s own "the handle is real" contract even under
/// repeated calls.
///
/// Cheaply `Clone` (an `Arc` handle over shared state), so a test can keep
/// a spy clone after moving the original into a
/// [`crate::launch_gate::LaunchGate`].
#[derive(Clone)]
pub struct FakeProber {
    inner: Arc<FakeProberState>,
}

struct FakeProberState {
    outcomes: Mutex<VecDeque<ScriptedProbeOutcome>>,
    calls: AtomicUsize,
}

impl FakeProber {
    /// Every call to `probe` returns `outcome`, ignoring the candidate
    /// path.
    #[must_use]
    pub fn always(outcome: ScriptedProbeOutcome) -> Self {
        Self::scripted([outcome])
    }

    /// Returns each scripted outcome once, in order; once exhausted,
    /// repeats the last scripted outcome for any further call.
    ///
    /// # Panics
    ///
    /// Panics if `outcomes` is empty: a `FakeProber` with nothing to
    /// return is a test-setup bug, not a valid scripted double.
    #[must_use]
    pub fn scripted(outcomes: impl IntoIterator<Item = ScriptedProbeOutcome>) -> Self {
        let queue: VecDeque<ScriptedProbeOutcome> = outcomes.into_iter().collect();
        assert!(
            !queue.is_empty(),
            "FakeProber must be scripted with at least one outcome"
        );
        Self {
            inner: Arc::new(FakeProberState {
                outcomes: Mutex::new(queue),
                calls: AtomicUsize::new(0),
            }),
        }
    }

    /// How many times `probe` has been called so far.
    #[must_use]
    pub fn call_count(&self) -> usize {
        self.inner.calls.load(Ordering::Acquire)
    }
}

impl ExecutableProber for FakeProber {
    fn probe(&self, _candidate: &Path) -> ProbeOutcome {
        self.inner.calls.fetch_add(1, Ordering::AcqRel);
        let scripted = {
            let mut outcomes = self
                .inner
                .outcomes
                .lock()
                .expect("FakeProber outcomes mutex poisoned by a prior panic");
            if outcomes.len() > 1 {
                outcomes
                    .pop_front()
                    .expect("checked non-empty by the length check above")
            } else {
                outcomes
                    .front()
                    .cloned()
                    .expect("constructors require at least one scripted outcome")
            }
        };
        match scripted {
            ScriptedProbeOutcome::Identity(identity) => fake_probed_identity(identity),
            ScriptedProbeOutcome::NotRegularFile => ProbeOutcome::NotRegularFile,
            ScriptedProbeOutcome::NotExecutable => ProbeOutcome::NotExecutable,
            ScriptedProbeOutcome::Unreadable => ProbeOutcome::Unreadable,
            ScriptedProbeOutcome::TooLarge => ProbeOutcome::TooLarge,
        }
    }
}

/// A [`Clock`] test double that always reports the same, caller-chosen
/// instant.
pub struct FixedClock(SystemTime);

impl FixedClock {
    /// Build a clock fixed at `time`.
    #[must_use]
    pub const fn new(time: SystemTime) -> Self {
        Self(time)
    }
}

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

/// An in-memory [`ApprovalStore`] test double: never persists anything
/// beyond the process's own memory.
#[derive(Default)]
pub struct InMemoryApprovalStore {
    records: Vec<ApprovalRecord>,
    next_id: u64,
}

impl InMemoryApprovalStore {
    /// Build an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ApprovalStore for InMemoryApprovalStore {
    fn list(&self) -> Result<Vec<ApprovalRecord>, ApprovalStoreError> {
        Ok(self.records.clone())
    }

    fn record(
        &mut self,
        identity: ExecutableIdentity,
        approver: DeviceLocalUser,
        approved_at: SystemTime,
    ) -> Result<ApprovalRecord, ApprovalStoreError> {
        self.next_id += 1;
        let record = ApprovalRecord {
            approval_id: ApprovalId(self.next_id),
            identity,
            approved_at,
            approver,
            status: ApprovalStatus::Active,
        };
        self.records.push(record.clone());
        Ok(record)
    }

    fn revoke(&mut self, id: ApprovalId, at: SystemTime) -> Result<(), ApprovalStoreError> {
        if let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.approval_id == id)
        {
            record.status = ApprovalStatus::Revoked { revoked_at: at };
        }
        Ok(())
    }

    fn find_active(
        &self,
        identity: &ExecutableIdentity,
    ) -> Result<Option<ApprovalRecord>, ApprovalStoreError> {
        Ok(self
            .records
            .iter()
            .rev()
            .find(|record| &record.identity == identity && record.status == ApprovalStatus::Active)
            .cloned())
    }
}

/// A sample identity for the contract test: its exact content is
/// arbitrary, only its stability across calls (so `find_active` can match
/// it back) matters.
fn sample_identity() -> ExecutableIdentity {
    ExecutableIdentity {
        canonical_path: PathBuf::from("/opt/tool/app"),
        size: 4096,
        sha256: Sha256Digest([7; 32]),
        modified_at: None,
        platform: PlatformEvidence::Unix { mode: 0o755 },
    }
}

/// Exercise the baseline `ApprovalStore` contract against any conformant
/// implementation: record, then list and find it active; revoke, then
/// confirm it is no longer found active; re-approve the same identity,
/// confirming that is allowed and mints a fresh id.
///
/// `make` builds a fresh store instance so the contract can be run against
/// implementations that hold internal, per-instance state (e.g. an open
/// file handle).
///
/// # Panics
///
/// Panics (via `expect`/`assert*`) if the store under test violates the
/// contract.
pub fn approval_store_contract<S: ApprovalStore>(make: impl Fn() -> S) {
    let mut store = make();
    let identity = sample_identity();
    let approver = DeviceLocalUser;
    let t0 = SystemTime::UNIX_EPOCH;

    let record = store
        .record(identity.clone(), approver, t0)
        .expect("recording a fresh approval must succeed");
    assert_eq!(record.identity, identity);
    assert_eq!(record.status, ApprovalStatus::Active);

    let listed = store.list().expect("list must succeed");
    assert!(
        listed.iter().any(|r| r.approval_id == record.approval_id),
        "a recorded approval must appear in list()"
    );

    let active = store
        .find_active(&identity)
        .expect("find_active must succeed");
    assert_eq!(
        active,
        Some(record.clone()),
        "find_active must find the just-recorded approval"
    );

    let t1 = t0 + Duration::from_secs(1);
    store
        .revoke(record.approval_id, t1)
        .expect("revoking a known approval must succeed");

    let after_revoke = store
        .find_active(&identity)
        .expect("find_active must succeed after a revoke");
    assert_eq!(
        after_revoke, None,
        "a revoked approval must not be found as active"
    );

    let t2 = t1 + Duration::from_secs(1);
    let re_approved = store
        .record(identity.clone(), approver, t2)
        .expect("re-approval after revoke must be allowed");
    assert_ne!(
        re_approved.approval_id, record.approval_id,
        "a re-approval must mint a fresh id, never resurrect the revoked one"
    );

    let active_again = store
        .find_active(&identity)
        .expect("find_active must succeed after re-approval");
    assert_eq!(active_again, Some(re_approved));
}
