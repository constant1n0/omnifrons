//! `LaunchGate` re-probes the recorded canonical path on every `decide`
//! call, never cached: an unapproved id is denied, a rewritten file is
//! denied with both digests, a shadowed path is denied with both paths, a
//! revoked approval is denied, and a fresh approval after revocation is
//! allowed again, carrying the exact `ProbedExecutable` that decision's
//! own re-probe produced. The spy-prober tests prove `decide` probes
//! exactly once per call, never zero and never more than once, and that
//! `revoke` never probes at all.
//!
//! Only compiled with `--features contract-tests` (see `omnifrons-app`'s
//! `contract-tests` feature): the fakes this file drives
//! (`FakeProber`, `InMemoryApprovalStore`, `FixedClock`) live behind that
//! feature.

#![cfg(feature = "contract-tests")]

use std::path::PathBuf;
use std::time::SystemTime;

use omnifrons_app::contract::approval_store::{FakeProber, FixedClock, InMemoryApprovalStore};
use omnifrons_app::launch_gate::{GateDecision, LaunchGate};
use omnifrons_domain::executable::{
    DenialReason, ExecutableIdentity, PlatformEvidence, ProbeOutcome, Sha256Digest,
};

fn identity(path: &str, size: u64, digest_byte: u8) -> ExecutableIdentity {
    ExecutableIdentity {
        canonical_path: PathBuf::from(path),
        size,
        sha256: Sha256Digest([digest_byte; 32]),
        modified_at: None,
        platform: PlatformEvidence::Unix { mode: 0o755 },
    }
}

#[test]
fn unapproved_id_is_denied() {
    let prober = FakeProber::always(ProbeOutcome::NotRegularFile);
    let mut gate = LaunchGate::new(
        prober,
        InMemoryApprovalStore::new(),
        FixedClock::new(SystemTime::UNIX_EPOCH),
    );

    let decision = gate.decide(omnifrons_domain::executable::ApprovalId(999));

    assert!(
        matches!(decision, GateDecision::Denied(DenialReason::Unapproved)),
        "an approval id with no record on file must be denied as Unapproved, got {decision:?}"
    );
}

#[test]
fn rewritten_file_is_denied_with_both_digests() {
    let original = identity("/opt/tool/app", 100, 1);
    let rewritten = identity("/opt/tool/app", 100, 2);

    let prober = FakeProber::always(ProbeOutcome::Identity(rewritten.clone()));
    let mut gate = LaunchGate::new(
        prober,
        InMemoryApprovalStore::new(),
        FixedClock::new(SystemTime::UNIX_EPOCH),
    );

    let record = gate
        .approve_candidate(original.clone())
        .expect("approving a fresh candidate must succeed");

    let decision = gate.decide(record.approval_id);

    match decision {
        GateDecision::Denied(DenialReason::ChangedSinceApproval { recorded, observed }) => {
            assert_eq!(recorded, original.sha256);
            assert_eq!(observed, rewritten.sha256);
        }
        other => panic!("expected ChangedSinceApproval, got {other:?}"),
    }
}

#[test]
fn shadowed_path_is_denied_with_both_paths() {
    let approved_identity = identity("/opt/tool/app", 100, 1);
    let shadowing_identity = identity("/opt/tool/app-shadow", 100, 1);

    let prober = FakeProber::always(ProbeOutcome::Identity(shadowing_identity.clone()));
    let mut gate = LaunchGate::new(
        prober,
        InMemoryApprovalStore::new(),
        FixedClock::new(SystemTime::UNIX_EPOCH),
    );

    let record = gate
        .approve_candidate(approved_identity.clone())
        .expect("approving a fresh candidate must succeed");

    let decision = gate.decide(record.approval_id);

    match decision {
        GateDecision::Denied(DenialReason::ShadowedPath { approved, resolved }) => {
            assert_eq!(approved, approved_identity.canonical_path);
            assert_eq!(resolved, shadowing_identity.canonical_path);
        }
        other => panic!("expected ShadowedPath, got {other:?}"),
    }
}

#[test]
fn revoked_approval_is_denied() {
    let target = identity("/opt/tool/app", 100, 1);
    let prober = FakeProber::always(ProbeOutcome::Identity(target.clone()));
    let mut gate = LaunchGate::new(
        prober,
        InMemoryApprovalStore::new(),
        FixedClock::new(SystemTime::UNIX_EPOCH),
    );

    let record = gate
        .approve_candidate(target)
        .expect("approving a fresh candidate must succeed");
    gate.revoke(record.approval_id)
        .expect("revoking a known approval must succeed");

    let decision = gate.decide(record.approval_id);

    assert!(
        matches!(decision, GateDecision::Denied(DenialReason::Revoked)),
        "expected Revoked, got {decision:?}"
    );
}

#[test]
fn re_approval_after_revoke_is_allowed() {
    let target = identity("/opt/tool/app", 100, 1);
    let prober = FakeProber::always(ProbeOutcome::Identity(target.clone()));
    let mut gate = LaunchGate::new(
        prober,
        InMemoryApprovalStore::new(),
        FixedClock::new(SystemTime::UNIX_EPOCH),
    );

    let first = gate
        .approve_candidate(target.clone())
        .expect("first approval must succeed");
    gate.revoke(first.approval_id)
        .expect("revoking a known approval must succeed");

    let second = gate
        .approve_candidate(target.clone())
        .expect("re-approval after revoke must be allowed");
    assert_ne!(
        second.approval_id, first.approval_id,
        "a re-approval must mint a fresh approval id, not reuse the revoked one"
    );

    let decision = gate.decide(second.approval_id);

    match decision {
        GateDecision::Allowed {
            approval_id,
            executable,
        } => {
            assert_eq!(approval_id, second.approval_id);
            assert_eq!(executable.identity, target);
        }
        other @ GateDecision::Denied(_) => panic!("expected Allowed, got {other:?}"),
    }
}

#[test]
fn decide_probes_exactly_once_per_call() {
    let target = identity("/opt/tool/app", 100, 1);
    let prober = FakeProber::always(ProbeOutcome::Identity(target.clone()));
    let spy = prober.clone();
    let mut gate = LaunchGate::new(
        prober,
        InMemoryApprovalStore::new(),
        FixedClock::new(SystemTime::UNIX_EPOCH),
    );

    let record = gate
        .approve_candidate(target)
        .expect("approving a fresh candidate must succeed");

    assert_eq!(
        spy.call_count(),
        0,
        "approve_candidate must not itself probe"
    );

    for expected_calls in 1..=3 {
        let decision = gate.decide(record.approval_id);
        assert!(
            matches!(decision, GateDecision::Allowed { approval_id, .. } if approval_id == record.approval_id),
            "expected Allowed({:?}), got {decision:?}",
            record.approval_id
        );
        assert_eq!(
            spy.call_count(),
            expected_calls,
            "decide must re-probe exactly once per call, never cached"
        );
    }
}

/// R3-001: `revoke` is a pure store write over the recorded identity
/// alone -- it must never consult the prober, unlike `decide`, which
/// always re-probes.
#[test]
fn revoke_never_probes() {
    let target = identity("/opt/tool/app", 100, 1);
    let prober = FakeProber::always(ProbeOutcome::Identity(target.clone()));
    let spy = prober.clone();
    let mut gate = LaunchGate::new(
        prober,
        InMemoryApprovalStore::new(),
        FixedClock::new(SystemTime::UNIX_EPOCH),
    );

    let record = gate
        .approve_candidate(target)
        .expect("approving a fresh candidate must succeed");
    gate.revoke(record.approval_id)
        .expect("revoking a known approval must succeed");

    assert_eq!(
        spy.call_count(),
        0,
        "revoke must never probe the filesystem"
    );
}

/// A minimal `ProcessSupervisor` fake that never actually spawns an OS
/// process: `spawn` always reports `Running`, and nothing (including a
/// `LaunchGate` revocation elsewhere) ever changes that -- only this
/// fake's own `observe` decides its status.
struct AlwaysRunningFakeSupervisor;

impl omnifrons_app::ProcessSupervisor for AlwaysRunningFakeSupervisor {
    fn spawn(
        &mut self,
        _spec: omnifrons_app::ProcessSpec,
    ) -> Result<omnifrons_app::ProcessId, omnifrons_app::SupervisorError> {
        Ok(omnifrons_app::ProcessId(1))
    }

    fn stop(
        &mut self,
        _id: omnifrons_app::ProcessId,
        _deadline: std::time::Duration,
    ) -> Result<omnifrons_domain::scope::ProcessTerminalState, omnifrons_app::SupervisorError> {
        unimplemented!("not exercised by this test")
    }

    fn observe(&self, _id: omnifrons_app::ProcessId) -> Option<omnifrons_app::ProcessStatus> {
        Some(omnifrons_app::ProcessStatus::Running)
    }
}

/// Slice 3: revoking an approval mid-run must never reach into (or affect)
/// an already-running process -- `LaunchGate::revoke` is a pure store
/// write (see `revoke_never_probes` above), and a real supervisor has no
/// hook a revocation could even call. This test makes that boundary
/// explicit: a fake supervisor's already-`Running` process stays `Running`
/// through a revoke, and only the *next* `decide` call (e.g. before a
/// hypothetical restart) reflects the revocation.
#[test]
fn revoke_mid_run_leaves_the_running_process_unaffected_but_denies_the_next_decide() {
    let target = identity("/opt/tool/app", 100, 1);
    let prober = FakeProber::always(ProbeOutcome::Identity(target.clone()));
    let mut gate = LaunchGate::new(
        prober,
        InMemoryApprovalStore::new(),
        FixedClock::new(SystemTime::UNIX_EPOCH),
    );

    let record = gate
        .approve_candidate(target)
        .expect("approving a fresh candidate must succeed");

    let first_decision = gate.decide(record.approval_id);
    assert!(
        matches!(first_decision, GateDecision::Allowed { .. }),
        "expected Allowed before revocation, got {first_decision:?}"
    );

    let mut supervisor = AlwaysRunningFakeSupervisor;
    let id = omnifrons_app::ProcessSupervisor::spawn(
        &mut supervisor,
        omnifrons_app::ProcessSpec::new("fake"),
    )
    .expect("the fake supervisor's spawn never fails");

    gate.revoke(record.approval_id)
        .expect("revoking a known approval must succeed");

    assert_eq!(
        omnifrons_app::ProcessSupervisor::observe(&supervisor, id),
        Some(omnifrons_app::ProcessStatus::Running),
        "a revoke must never affect an already-running process's observed status"
    );

    let next_decision = gate.decide(record.approval_id);
    assert!(
        matches!(next_decision, GateDecision::Denied(DenialReason::Revoked)),
        "the next decide after a mid-run revoke must deny as Revoked, got {next_decision:?}"
    );
}
