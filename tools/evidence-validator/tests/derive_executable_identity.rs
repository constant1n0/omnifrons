//! `derive_executable_identity(ExecutableIdentityObservations) -> (Outcome,
//! ExecutableIdentityObservedState)`: the sole place a VP-S14 (executable
//! identity binding; renewal required after a material change) result
//! exists. A separate file from `tests/derive_ipc_boundary.rs` (VP-S13) and
//! the others, mirroring this crate's one-file-per-concern convention.

use evidence_validator::derive::{
    ExecutableIdentityObservations, ExecutableIdentityObservedState, Outcome,
    derive_executable_identity,
};

fn all_proofs() -> ExecutableIdentityObservations {
    ExecutableIdentityObservations {
        identity_gated: true,
        location_scheme_is_app_protocol: true,
        static_gate_pinned: true,
        approval_recorded: true,
        unchanged_launch_allowed: true,
        rewrite_attempted: true,
        rewrite_denied_changed: true,
        shadow_attempted: true,
        shadow_denied_shadowed: true,
        original_record_unchanged: true,
        reapproval_launched: true,
        changed_launch_allowed: false,
        shadow_launch_allowed: false,
        record_silently_rebound: false,
    }
}

#[test]
fn every_positive_proof_yields_pass_and_renewal_required() {
    assert_eq!(
        derive_executable_identity(all_proofs()),
        (
            Outcome::Pass,
            ExecutableIdentityObservedState::RenewalRequired
        )
    );
}

#[test]
fn default_observations_with_no_proof_yield_uncertain() {
    assert_eq!(
        derive_executable_identity(ExecutableIdentityObservations::default()),
        (
            Outcome::Uncertain,
            ExecutableIdentityObservedState::Unverified
        )
    );
}

#[test]
fn removing_any_independent_positive_proof_yields_uncertain() {
    let mutators: [fn(&mut ExecutableIdentityObservations); 11] = [
        |o| o.identity_gated = false,
        |o| o.location_scheme_is_app_protocol = false,
        |o| o.static_gate_pinned = false,
        |o| o.approval_recorded = false,
        |o| o.unchanged_launch_allowed = false,
        |o| o.rewrite_attempted = false,
        // A typed-but-different or untyped denial reaches this derive as a
        // false *_denied_* flag (the probe decides the code match), so #6
        // and #8 are also those cases: refused, but the mechanism unproven.
        |o| o.rewrite_denied_changed = false,
        |o| o.shadow_attempted = false,
        |o| o.shadow_denied_shadowed = false,
        |o| o.original_record_unchanged = false,
        |o| o.reapproval_launched = false,
    ];
    for (index, mutate) in mutators.into_iter().enumerate() {
        let mut observations = all_proofs();
        mutate(&mut observations);
        assert_eq!(
            derive_executable_identity(observations),
            (
                Outcome::Uncertain,
                ExecutableIdentityObservedState::Unverified
            ),
            "proof #{index} removed"
        );
    }
}

#[test]
fn changed_launch_allowed_overrides_all_proofs_including_a_contradicting_denial() {
    // all_proofs() already holds rewrite_denied_changed: a launch reported
    // alongside that denial is a contradiction, and still fails closed.
    let observations = ExecutableIdentityObservations {
        changed_launch_allowed: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_executable_identity(observations),
        (
            Outcome::Fail,
            ExecutableIdentityObservedState::LaunchedChanged
        )
    );
}

#[test]
fn changed_launch_allowed_alone_on_default_yields_fail_and_launched_changed() {
    // Proves failures override missing proofs, not just a full positive set.
    let observations = ExecutableIdentityObservations {
        changed_launch_allowed: true,
        ..ExecutableIdentityObservations::default()
    };
    assert_eq!(
        derive_executable_identity(observations),
        (
            Outcome::Fail,
            ExecutableIdentityObservedState::LaunchedChanged
        )
    );
}

#[test]
fn shadow_launch_allowed_overrides_all_proofs_including_a_contradicting_denial() {
    // all_proofs() already holds shadow_denied_shadowed: a launch reported
    // alongside that denial is a contradiction, and still fails closed.
    let observations = ExecutableIdentityObservations {
        shadow_launch_allowed: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_executable_identity(observations),
        (
            Outcome::Fail,
            ExecutableIdentityObservedState::LaunchedShadowed
        )
    );
}

#[test]
fn shadow_launch_allowed_alone_on_default_yields_fail_and_launched_shadowed() {
    let observations = ExecutableIdentityObservations {
        shadow_launch_allowed: true,
        ..ExecutableIdentityObservations::default()
    };
    assert_eq!(
        derive_executable_identity(observations),
        (
            Outcome::Fail,
            ExecutableIdentityObservedState::LaunchedShadowed
        )
    );
}

#[test]
fn record_silently_rebound_overrides_all_proofs_including_original_record_unchanged() {
    // all_proofs() already holds original_record_unchanged: a rebind reported
    // alongside it is a contradiction, and still fails closed.
    let observations = ExecutableIdentityObservations {
        record_silently_rebound: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_executable_identity(observations),
        (
            Outcome::Fail,
            ExecutableIdentityObservedState::SilentlyRebound
        )
    );
}

#[test]
fn record_silently_rebound_alone_on_default_yields_fail_and_silently_rebound() {
    let observations = ExecutableIdentityObservations {
        record_silently_rebound: true,
        ..ExecutableIdentityObservations::default()
    };
    assert_eq!(
        derive_executable_identity(observations),
        (
            Outcome::Fail,
            ExecutableIdentityObservedState::SilentlyRebound
        )
    );
}

#[test]
fn changed_launch_allowed_outranks_shadow_launch_allowed() {
    let observations = ExecutableIdentityObservations {
        changed_launch_allowed: true,
        shadow_launch_allowed: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_executable_identity(observations),
        (
            Outcome::Fail,
            ExecutableIdentityObservedState::LaunchedChanged
        )
    );
}

#[test]
fn changed_launch_allowed_outranks_record_silently_rebound() {
    let observations = ExecutableIdentityObservations {
        changed_launch_allowed: true,
        record_silently_rebound: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_executable_identity(observations),
        (
            Outcome::Fail,
            ExecutableIdentityObservedState::LaunchedChanged
        )
    );
}

#[test]
fn shadow_launch_allowed_outranks_record_silently_rebound() {
    let observations = ExecutableIdentityObservations {
        shadow_launch_allowed: true,
        record_silently_rebound: true,
        ..all_proofs()
    };
    assert_eq!(
        derive_executable_identity(observations),
        (
            Outcome::Fail,
            ExecutableIdentityObservedState::LaunchedShadowed
        )
    );
}
