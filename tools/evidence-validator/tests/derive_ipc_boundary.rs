//! `derive_ipc_boundary(IpcBoundaryObservations) -> (Outcome,
//! IpcBoundaryObservedState)`: the sole place a VP-S13 (typed IPC boundary;
//! path attacks rejected) result exists. A separate file from
//! `tests/derive_ipc.rs` (VP-S3) and the others, mirroring this crate's
//! one-file-per-concern convention.

use evidence_validator::derive::{
    IpcBoundaryObservations, IpcBoundaryObservedState, Outcome, derive_ipc_boundary,
};

fn all_proofs() -> IpcBoundaryObservations {
    IpcBoundaryObservations {
        identity_gated: true,
        location_scheme_is_app_protocol: true,
        workspace_selected: true,
        generic_capability_absent_pinned: true,
        validators_pinned: true,
        malformed_payloads_sent: true,
        path_attacks_sent: true,
        every_attack_rejected: true,
        every_rejection_typed: true,
        outside_snapshot_taken: true,
        outside_target_modified: false,
        canary_leaked: false,
        attack_accepted: false,
        generic_capability_observed: false,
    }
}

#[test]
fn every_positive_proof_yields_pass_and_rejected() {
    assert_eq!(
        derive_ipc_boundary(all_proofs()),
        (Outcome::Pass, IpcBoundaryObservedState::Rejected)
    );
}

#[test]
fn removing_any_independent_positive_proof_yields_uncertain() {
    let mutators: [fn(&mut IpcBoundaryObservations); 10] = [
        |o| o.identity_gated = false,
        |o| o.location_scheme_is_app_protocol = false,
        |o| o.workspace_selected = false,
        |o| o.generic_capability_absent_pinned = false,
        |o| o.validators_pinned = false,
        |o| o.malformed_payloads_sent = false,
        |o| o.path_attacks_sent = false,
        |o| o.every_attack_rejected = false,
        |o| o.every_rejection_typed = false,
        |o| o.outside_snapshot_taken = false,
    ];
    for (index, mutate) in mutators.into_iter().enumerate() {
        let mut observations = all_proofs();
        mutate(&mut observations);
        assert_eq!(
            derive_ipc_boundary(observations),
            (Outcome::Uncertain, IpcBoundaryObservedState::Unverified),
            "proof #{index} removed"
        );
    }
}

#[test]
fn a_modification_reported_without_a_snapshot_gate_still_fails_closed_as_escaped() {
    let mut observations = all_proofs();
    observations.outside_snapshot_taken = false;
    observations.outside_target_modified = true;
    assert_eq!(
        derive_ipc_boundary(observations),
        (Outcome::Fail, IpcBoundaryObservedState::Escaped)
    );
}

#[test]
fn an_untyped_rejection_is_uncertain_never_a_failure() {
    // A crash or timeout instead of a typed error: still a rejection, so no
    // best-effort interpretation, but it does not show why the payload was
    // refused, and a timeout can come from the harness itself.
    let mut observations = all_proofs();
    observations.every_rejection_typed = false;
    assert_eq!(
        derive_ipc_boundary(observations),
        (Outcome::Uncertain, IpcBoundaryObservedState::Unverified)
    );
}

#[test]
fn outside_target_modified_always_yields_fail_and_escaped() {
    let observations = IpcBoundaryObservations {
        outside_target_modified: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc_boundary(observations),
        (Outcome::Fail, IpcBoundaryObservedState::Escaped)
    );
}

#[test]
fn canary_leaked_always_yields_fail_and_leaked() {
    let observations = IpcBoundaryObservations {
        canary_leaked: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc_boundary(observations),
        (Outcome::Fail, IpcBoundaryObservedState::Leaked)
    );
}

#[test]
fn attack_accepted_always_yields_fail_and_best_effort() {
    let observations = IpcBoundaryObservations {
        attack_accepted: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc_boundary(observations),
        (Outcome::Fail, IpcBoundaryObservedState::BestEffort)
    );
}

#[test]
fn generic_capability_observed_always_yields_fail_and_generic_capability() {
    let observations = IpcBoundaryObservations {
        generic_capability_observed: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc_boundary(observations),
        (Outcome::Fail, IpcBoundaryObservedState::GenericCapability)
    );
}

#[test]
fn a_failure_condition_overrides_a_missing_identity_gate() {
    // Demonstrated failures override every other observation, including one
    // as basic as the identity gate never having held.
    let observations = IpcBoundaryObservations {
        identity_gated: false,
        attack_accepted: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc_boundary(observations),
        (Outcome::Fail, IpcBoundaryObservedState::BestEffort)
    );
}

#[test]
fn an_escape_outranks_a_canary_leak() {
    let observations = IpcBoundaryObservations {
        outside_target_modified: true,
        canary_leaked: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc_boundary(observations),
        (Outcome::Fail, IpcBoundaryObservedState::Escaped)
    );
}

#[test]
fn a_canary_leak_outranks_an_accepted_attack() {
    let observations = IpcBoundaryObservations {
        canary_leaked: true,
        attack_accepted: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc_boundary(observations),
        (Outcome::Fail, IpcBoundaryObservedState::Leaked)
    );
}

#[test]
fn an_accepted_attack_outranks_an_observed_generic_capability() {
    let observations = IpcBoundaryObservations {
        attack_accepted: true,
        generic_capability_observed: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc_boundary(observations),
        (Outcome::Fail, IpcBoundaryObservedState::BestEffort)
    );
}

#[test]
fn default_observations_with_no_proof_yield_uncertain() {
    assert_eq!(
        derive_ipc_boundary(IpcBoundaryObservations::default()),
        (Outcome::Uncertain, IpcBoundaryObservedState::Unverified)
    );
}
