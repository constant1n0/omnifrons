//! `derive(Observations) -> (Outcome, ObservedState)`: the sole place a
//! VP-S6 result exists (design.md "Honesty machinery"). A pure function --
//! no I/O, no retry -- that never collapses `Outcome` and `ObservedState`
//! into one compound value such as `orphan-risk/uncertain`; a scenario
//! record always carries them as two separate fields (`result`,
//! `observed_state`).

/// The closed `result` outcome (Closed Outcome Vocabulary requirement).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail,
    Uncertain,
}

/// The `observed_state` a row may additionally carry alongside `Outcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedState {
    /// The public state token for a completed, fully proven run.
    Verify,
    /// A recorded pid/starttime is still alive, or termination could not be
    /// proven -- the demonstrated `uncertain` this change exists to
    /// produce.
    OrphanRisk,
}

/// Every fact the procedure can observe, before any outcome is decided.
/// Field names mirror design.md's honesty-machinery observation table. Six
/// independent booleans, not a state machine: each names one fact the
/// procedure either did or did not observe, and [`derive`] is the single
/// place they combine into an outcome.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Observations {
    /// AV1 (digest) and AV2 (`--demo-harness` scan) both completed and gated
    /// before the `WebDriver` session was created.
    pub identity_gated: bool,
    /// The descendant process was observed alive before Stop was triggered.
    pub descendant_alive_before_stop: bool,
    /// The product's own `State:` badge confirmed the stop.
    pub stop_confirmed: bool,
    /// Every recorded `(pid, starttime)` was proven gone after the bounded
    /// wait.
    pub all_pids_proven_gone: bool,
    /// A recorded pid is still alive with the same starttime after the
    /// bounded wait.
    pub any_pid_alive_after_wait: bool,
    /// `/proc` enumeration for a recorded pid could not be read.
    pub enumeration_unreadable: bool,
}

/// Maps observations to the `(result, observed_state)` pair a scenario row
/// must carry as two always-separate values.
///
/// `pass` requires every positive proof (design.md "Honesty machinery" #2);
/// any absent or unreadable observation yields `uncertain`, never a pass. A
/// pid proven alive after the wait always yields `fail`, since it overrides
/// every other observation.
#[must_use]
pub fn derive(observations: Observations) -> (Outcome, ObservedState) {
    if observations.any_pid_alive_after_wait {
        return (Outcome::Fail, ObservedState::OrphanRisk);
    }
    if observations.identity_gated
        && observations.descendant_alive_before_stop
        && observations.stop_confirmed
        && observations.all_pids_proven_gone
        && !observations.enumeration_unreadable
    {
        return (Outcome::Pass, ObservedState::Verify);
    }
    (Outcome::Uncertain, ObservedState::OrphanRisk)
}

/// The public `observed_state` a VP-S1 (CSP) row may additionally carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CspObservedState {
    /// The public state token for a fully proven, enforced baseline.
    Enforced,
    /// The baseline was demonstrably not enforced: the inline script ran,
    /// or the external fetch was dispatched.
    Bypassed,
    /// Any fact absent, unreadable, or short of full proof; nothing was
    /// demonstrated either way.
    Unverified,
}

/// Every fact `docs/evidence/VP-001/procedures/vp-s1-probe.mjs` can
/// observe, before any outcome is decided. Mirrors [`Observations`]'s own
/// shape: independent booleans, not a state machine.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CspObservations {
    /// AV1/AV2 both gated before the session was created (`vp-s1-linux.sh`).
    pub identity_gated: bool,
    /// The packaged custom-protocol scheme was observed, never `http:`.
    pub location_scheme_is_app_protocol: bool,
    /// A `securitypolicyviolation` matched `script-src*` (inline script).
    pub inline_script_violation: bool,
    /// The inline script actually ran.
    pub inline_script_ran: bool,
    /// A `securitypolicyviolation` matched `connect-src*` (external fetch).
    pub external_fetch_violation: bool,
    /// The external fetch resolved: the request was dispatched.
    pub external_fetch_resolved: bool,
    /// A `securitypolicyviolation` matched `frame-src*`/`child-src*`.
    pub framed_context_violation: bool,
    /// A policy dump (a violation's `originalPolicy`, or `<meta>`) was captured.
    pub policy_dump_captured: bool,
    /// A third-party app surface was actually exercised by this run.
    pub third_party_surface_exercised: bool,
}

/// Maps VP-S1 observations to `(result, observed_state)`.
///
/// `inline_script_ran` or `external_fetch_resolved` always yields `Fail`,
/// overriding every other observation. `Pass` requires every positive proof, including
/// `third_party_surface_exercised`: no such surface exists in this renderer
/// today (ADR-0004), so `Pass` is unreachable until one does, by the
/// maintainer's decision -- not a bug here. Anything short of that is
/// `Uncertain`.
///
/// `CspObservedState`'s variant names are chosen here, not taken from
/// RCS-001's signal mapping (docs/renderer-content-security.md § Signal
/// mapping), which has no CSP entry -- the same gap VP-S6-03's
/// `proven-gone` token addressed for containment (records.md).
#[must_use]
pub fn derive_csp(observations: CspObservations) -> (Outcome, CspObservedState) {
    if observations.inline_script_ran || observations.external_fetch_resolved {
        return (Outcome::Fail, CspObservedState::Bypassed);
    }
    if observations.identity_gated
        && observations.location_scheme_is_app_protocol
        && observations.inline_script_violation
        && observations.external_fetch_violation
        && observations.framed_context_violation
        && observations.policy_dump_captured
        && observations.third_party_surface_exercised
    {
        return (Outcome::Pass, CspObservedState::Enforced);
    }
    (Outcome::Uncertain, CspObservedState::Unverified)
}
