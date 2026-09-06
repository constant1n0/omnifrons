//! `LaunchGate`: the application-layer decision point that stands between
//! an approval id and actually launching the executable it names.
//!
//! `decide` never trusts a cached identity: it re-probes the recorded
//! canonical path fresh on every call, so a file rewritten, shadowed, or
//! whose approval was revoked *after* approval is caught immediately
//! before launch, not only at approval time
//! (`docs/spike-log.md` § Slice 2).

use omnifrons_domain::executable::{
    ApprovalId, ApprovalRecord, ApprovalStatus, DenialReason, DeviceLocalUser, ExecutableIdentity,
};

use crate::approval_store::{ApprovalStore, ApprovalStoreError};
use crate::clock::Clock;
use crate::executable_prober::{ExecutableProber, ProbeOutcome, ProbedExecutable};

/// The outcome of a [`LaunchGate::decide`] call.
///
/// Unlike `omnifrons_domain::executable::LaunchDecision`, `Allowed` here
/// carries the exact [`ProbedExecutable`] -- identity plus open handle --
/// that `decide`'s own re-probe just produced, so a caller (the shell)
/// hands that *same* handle to the supervisor rather than re-opening the
/// path a second time by name, which would reopen a TOCTOU window this
/// re-probe had just closed. `Denied` carries no handle -- there is
/// nothing to launch -- so it stays the plain domain [`DenialReason`].
#[derive(Debug)]
pub enum GateDecision {
    /// The launch is allowed, under this approval id, with this exact
    /// probed executable.
    Allowed {
        /// The approval id this decision was made under.
        approval_id: ApprovalId,
        /// The identity and handle `decide`'s own re-probe just produced.
        executable: ProbedExecutable,
    },
    /// The launch is denied, for this reason.
    Denied(DenialReason),
}

/// Stands between an [`ApprovalId`] and permission to launch: re-probes,
/// compares, and only then allows.
///
/// Generic over its three collaborators (`P`: [`ExecutableProber`], `S`:
/// [`ApprovalStore`], `C`: [`Clock`]) so the same logic runs, unmodified,
/// against fakes in this crate's own tests and against the real adapters
/// (`omnifrons-adapters`, `omnifrons-supervisor`) in a real build.
pub struct LaunchGate<P, S, C> {
    prober: P,
    store: S,
    clock: C,
}

impl<P, S, C> LaunchGate<P, S, C>
where
    P: ExecutableProber,
    S: ApprovalStore,
    C: Clock,
{
    /// Build a gate over the given prober, store, and clock.
    pub const fn new(prober: P, store: S, clock: C) -> Self {
        Self {
            prober,
            store,
            clock,
        }
    }

    /// Decide whether `approval_id` may launch right now.
    ///
    /// Always re-probes the approval's recorded canonical path first --
    /// never a cached identity from approval time. In order: no matching
    /// record at all is [`DenialReason::Unapproved`]; a failed re-probe is
    /// [`DenialReason::ProbeFailed`]; a re-probe that resolves to a
    /// *different* canonical path is [`DenialReason::ShadowedPath`]; a
    /// matching path whose digest or size no longer matches is
    /// [`DenialReason::ChangedSinceApproval`]; a fully matching identity
    /// whose approval was revoked is [`DenialReason::Revoked`]; otherwise
    /// the launch is [`GateDecision::Allowed`], carrying the exact
    /// [`ProbedExecutable`] this re-probe just produced.
    ///
    /// If the approval store itself cannot be read, this denies fail-closed
    /// as [`DenialReason::Unapproved`] rather than returning an ambiguous
    /// "allowed": an approval this gate cannot prove is on record must
    /// never be treated as if it were.
    ///
    /// # Panics
    ///
    /// Never in practice: internally, a non-`Identity`
    /// [`ProbeOutcome`] is mapped via
    /// [`ProbeOutcome::as_domain_failure`], which only returns `None`
    /// for the `Identity` variant -- already excluded by the match arm
    /// that calls it -- so the `.expect()` guarding that mapping cannot
    /// actually fail for any value a conformant [`ExecutableProber`] can
    /// produce.
    pub fn decide(&mut self, approval_id: ApprovalId) -> GateDecision {
        let Some(record) = self.find_record(approval_id) else {
            return GateDecision::Denied(DenialReason::Unapproved);
        };

        let observed = match self.prober.probe(&record.identity.canonical_path) {
            ProbeOutcome::Identity(executable) => executable,
            other => {
                let failure = other
                    .as_domain_failure()
                    .expect("a non-Identity ProbeOutcome always maps to a domain failure");
                return GateDecision::Denied(DenialReason::ProbeFailed(failure));
            }
        };

        if observed.identity.canonical_path != record.identity.canonical_path {
            return GateDecision::Denied(DenialReason::ShadowedPath {
                approved: record.identity.canonical_path,
                resolved: observed.identity.canonical_path,
            });
        }

        if observed.identity.sha256 != record.identity.sha256
            || observed.identity.size != record.identity.size
        {
            return GateDecision::Denied(DenialReason::ChangedSinceApproval {
                recorded: record.identity.sha256,
                observed: observed.identity.sha256,
            });
        }

        if matches!(record.status, ApprovalStatus::Revoked { .. }) {
            return GateDecision::Denied(DenialReason::Revoked);
        }

        GateDecision::Allowed {
            approval_id,
            executable: observed,
        }
    }

    /// Approve `identity`, recording it with the current time and an
    /// opaque device-local approver.
    ///
    /// # Errors
    ///
    /// Returns [`ApprovalStoreError`] if the store could not be written.
    pub fn approve_candidate(
        &mut self,
        identity: ExecutableIdentity,
    ) -> Result<ApprovalRecord, ApprovalStoreError> {
        let now = self.clock.now();
        self.store.record(identity, DeviceLocalUser, now)
    }

    /// Revoke `id`, effective now.
    ///
    /// Never probes: revocation is a pure store write, over the recorded
    /// identity alone, and must not depend on the filesystem to succeed.
    ///
    /// # Errors
    ///
    /// Returns [`ApprovalStoreError`] if the store could not be written.
    pub fn revoke(&mut self, id: ApprovalId) -> Result<(), ApprovalStoreError> {
        let now = self.clock.now();
        self.store.revoke(id, now)
    }

    /// Find the record for `approval_id` by scanning `list()`; fails
    /// closed (`None`) if the store itself could not be read, per this
    /// method's own doc comment on [`Self::decide`].
    fn find_record(&self, approval_id: ApprovalId) -> Option<ApprovalRecord> {
        self.store
            .list()
            .ok()?
            .into_iter()
            .find(|record| record.approval_id == approval_id)
    }
}
