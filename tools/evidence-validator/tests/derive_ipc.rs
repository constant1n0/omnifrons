//! `derive_ipc(IpcObservations) -> (Outcome, IpcObservedState)`: the sole
//! place a VP-S3 (typed-IPC bridge confinement) result exists. A separate
//! file from `tests/derive_csp.rs` (VP-S1) and `tests/derive.rs` (VP-S6),
//! mirroring this crate's one-file-per-concern convention
//! (`tests/v1_mandatory_fields.rs`, `tests/v2_result_vocabulary.rs`, ...).

use evidence_validator::derive::{IpcObservations, IpcObservedState, Outcome, derive_ipc};

fn all_proofs() -> IpcObservations {
    IpcObservations {
        identity_gated: true,
        location_scheme_is_app_protocol: true,
        connect_src_policy_captured: true,
        connect_src_is_documented_source_for_os: true,
        registered_call_completed: true,
        registered_call_used_custom_protocol: true,
        postmessage_fallback_observed: false,
        bridge_connect_src_violation: false,
        unregistered_call_rejected: true,
        unregistered_call_resolved: false,
        artifact_load_attempted: true,
        artifact_load_succeeded: false,
        static_inventory_pinned: true,
    }
}

#[test]
fn every_positive_proof_yields_pass_and_confined() {
    assert_eq!(
        derive_ipc(all_proofs()),
        (Outcome::Pass, IpcObservedState::Confined)
    );
}

#[test]
fn removing_any_independent_positive_proof_yields_uncertain() {
    // `connect_src_is_documented_source_for_os` is deliberately excluded
    // here: turning it off while `connect_src_policy_captured` stays true
    // is the demonstrated `UndocumentedBridge` failure (rule 1), not a mere
    // absence of proof -- covered by
    // `an_undocumented_connect_src_source_always_yields_fail` below.
    let mutators: [fn(&mut IpcObservations); 8] = [
        |o| o.identity_gated = false,
        |o| o.location_scheme_is_app_protocol = false,
        |o| o.connect_src_policy_captured = false,
        |o| o.registered_call_completed = false,
        |o| o.registered_call_used_custom_protocol = false,
        |o| o.unregistered_call_rejected = false,
        |o| o.artifact_load_attempted = false,
        |o| o.static_inventory_pinned = false,
    ];
    for mutate in mutators {
        let mut observations = all_proofs();
        mutate(&mut observations);
        assert_eq!(
            derive_ipc(observations),
            (Outcome::Uncertain, IpcObservedState::Unverified)
        );
    }
}

#[test]
fn an_undocumented_connect_src_source_always_yields_fail() {
    let observations = IpcObservations {
        connect_src_is_documented_source_for_os: false,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc(observations),
        (Outcome::Fail, IpcObservedState::UndocumentedBridge)
    );
}

#[test]
fn a_postmessage_fallback_always_yields_fail() {
    let observations = IpcObservations {
        postmessage_fallback_observed: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc(observations),
        (Outcome::Fail, IpcObservedState::FellBack)
    );
}

#[test]
fn a_bridge_connect_src_violation_always_yields_fail() {
    let observations = IpcObservations {
        bridge_connect_src_violation: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc(observations),
        (Outcome::Fail, IpcObservedState::FellBack)
    );
}

#[test]
fn an_artifact_load_that_succeeds_always_yields_fail() {
    let observations = IpcObservations {
        artifact_load_succeeded: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc(observations),
        (Outcome::Fail, IpcObservedState::ArtifactServed)
    );
}

#[test]
fn an_unregistered_call_that_resolves_always_yields_fail() {
    let observations = IpcObservations {
        unregistered_call_resolved: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc(observations),
        (Outcome::Fail, IpcObservedState::UnregisteredCommandServed)
    );
}

#[test]
fn a_failure_condition_overrides_a_missing_identity_gate() {
    // Demonstrated failures override every other observation, including one
    // as basic as the identity gate never having held.
    let observations = IpcObservations {
        identity_gated: false,
        postmessage_fallback_observed: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc(observations),
        (Outcome::Fail, IpcObservedState::FellBack)
    );
}

#[test]
fn an_undocumented_bridge_outranks_a_postmessage_fallback() {
    let observations = IpcObservations {
        connect_src_is_documented_source_for_os: false,
        postmessage_fallback_observed: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc(observations),
        (Outcome::Fail, IpcObservedState::UndocumentedBridge)
    );
}

#[test]
fn a_postmessage_fallback_outranks_an_artifact_load_that_succeeds() {
    let observations = IpcObservations {
        postmessage_fallback_observed: true,
        artifact_load_succeeded: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc(observations),
        (Outcome::Fail, IpcObservedState::FellBack)
    );
}

#[test]
fn an_artifact_load_that_succeeds_outranks_an_unregistered_call_that_resolves() {
    let observations = IpcObservations {
        artifact_load_succeeded: true,
        unregistered_call_resolved: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_ipc(observations),
        (Outcome::Fail, IpcObservedState::ArtifactServed)
    );
}

#[test]
fn default_observations_with_no_proof_yield_uncertain() {
    assert_eq!(
        derive_ipc(IpcObservations::default()),
        (Outcome::Uncertain, IpcObservedState::Unverified)
    );
}
