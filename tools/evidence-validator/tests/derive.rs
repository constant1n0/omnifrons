//! `derive(Observations) -> (Outcome, ObservedState)`: the sole place a
//! VP-S6 result exists (design.md "Honesty machinery"). Every row below
//! exercises one derivation path and asserts `Outcome` and `ObservedState`
//! stay two always-separate values -- `derive` has no way to return a
//! compound value like `orphan-risk/uncertain`.

use evidence_validator::derive::{Observations, ObservedState, Outcome, derive};

fn all_positive() -> Observations {
    Observations {
        identity_gated: true,
        descendant_alive_before_stop: true,
        stop_confirmed: true,
        all_pids_proven_gone: true,
        any_pid_alive_after_wait: false,
        enumeration_unreadable: false,
    }
}

#[test]
fn three_positive_proofs_yield_pass_and_verify() {
    assert_eq!(
        derive(all_positive()),
        (Outcome::Pass, ObservedState::Verify)
    );
}

#[test]
fn a_pid_alive_after_the_bounded_wait_yields_fail_and_orphan_risk() {
    let observations = Observations {
        any_pid_alive_after_wait: true,
        ..all_positive()
    };
    assert_eq!(
        derive(observations),
        (Outcome::Fail, ObservedState::OrphanRisk)
    );
}

#[test]
fn ungated_identity_yields_uncertain_and_orphan_risk() {
    let observations = Observations {
        identity_gated: false,
        ..all_positive()
    };
    assert_eq!(
        derive(observations),
        (Outcome::Uncertain, ObservedState::OrphanRisk)
    );
}

#[test]
fn unreadable_enumeration_yields_uncertain_and_orphan_risk() {
    let observations = Observations {
        enumeration_unreadable: true,
        ..all_positive()
    };
    assert_eq!(
        derive(observations),
        (Outcome::Uncertain, ObservedState::OrphanRisk)
    );
}

#[test]
fn unconfirmed_stop_yields_uncertain_and_orphan_risk() {
    let observations = Observations {
        stop_confirmed: false,
        ..all_positive()
    };
    assert_eq!(
        derive(observations),
        (Outcome::Uncertain, ObservedState::OrphanRisk)
    );
}

#[test]
fn default_observations_with_no_proof_yield_uncertain() {
    assert_eq!(
        derive(Observations::default()),
        (Outcome::Uncertain, ObservedState::OrphanRisk)
    );
}

#[test]
fn outcome_and_observed_state_are_always_two_separate_fields() {
    // `derive`'s return type is a tuple of two independent enums -- there is
    // no compound variant that could ever render as `orphan-risk/uncertain`
    // stored as a single `result` value.
    let (outcome, observed_state) = derive(all_positive());
    assert_ne!(format!("{outcome:?}"), format!("{observed_state:?}"));
}
