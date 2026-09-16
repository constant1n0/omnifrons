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
