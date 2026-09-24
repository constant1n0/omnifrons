//! `derive_csp(CspObservations) -> (Outcome, CspObservedState)`: the sole
//! place a VP-S1 (CSP baseline) result exists. A separate file from
//! `tests/derive.rs` (VP-S6), mirroring this crate's one-file-per-concern
//! convention (`tests/v1_mandatory_fields.rs`, `tests/v2_result_vocabulary.rs`, ...).

use evidence_validator::derive::{CspObservations, CspObservedState, Outcome, derive_csp};

fn all_positive() -> CspObservations {
    CspObservations {
        identity_gated: true,
        location_scheme_is_app_protocol: true,
        inline_script_violation: true,
        inline_script_ran: false,
        external_fetch_violation: true,
        external_fetch_resolved: false,
        framed_context_violation: true,
        policy_dump_captured: true,
        third_party_surface_exercised: true,
    }
}

#[test]
fn every_positive_proof_yields_pass_and_enforced() {
    assert_eq!(
        derive_csp(all_positive()),
        (Outcome::Pass, CspObservedState::Enforced)
    );
}

#[test]
fn an_enforced_baseline_with_no_third_party_surface_yields_uncertain() {
    // The maintainer's own decision (ADR-0004: no such surface exists yet):
    // `Pass` stays unreachable even when every other proof holds.
    let observations = CspObservations {
        third_party_surface_exercised: false,
        ..all_positive()
    };
    assert_eq!(
        derive_csp(observations),
        (Outcome::Uncertain, CspObservedState::Unverified)
    );
}

#[test]
fn the_inline_script_running_always_yields_fail() {
    let observations = CspObservations {
        inline_script_ran: true,
        ..all_positive()
    };
    assert_eq!(
        derive_csp(observations),
        (Outcome::Fail, CspObservedState::Bypassed)
    );
}

#[test]
fn a_dispatched_external_fetch_always_yields_fail() {
    let observations = CspObservations {
        external_fetch_resolved: true,
        ..all_positive()
    };
    assert_eq!(
        derive_csp(observations),
        (Outcome::Fail, CspObservedState::Bypassed)
    );
}

#[test]
fn a_missing_policy_dump_yields_uncertain() {
    let observations = CspObservations {
        policy_dump_captured: false,
        ..all_positive()
    };
    assert_eq!(
        derive_csp(observations),
        (Outcome::Uncertain, CspObservedState::Unverified)
    );
}

#[test]
fn an_http_scheme_yields_uncertain() {
    let observations = CspObservations {
        location_scheme_is_app_protocol: false,
        ..all_positive()
    };
    assert_eq!(
        derive_csp(observations),
        (Outcome::Uncertain, CspObservedState::Unverified)
    );
}

#[test]
fn default_observations_with_no_proof_yield_uncertain() {
    assert_eq!(
        derive_csp(CspObservations::default()),
        (Outcome::Uncertain, CspObservedState::Unverified)
    );
}
