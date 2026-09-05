//! Runs `omnifrons-app`'s reusable `ProcessSupervisor` contract against the
//! real, OS-backed supervisor -- not only the in-memory fake
//! `omnifrons-app`'s own tests use.
//!
//! Only the baseline contract and the idempotent-stop contract run on
//! unix, not `unproven_descendants_yield_orphan_risk_uncertain`: that
//! scenario needs a supervisor deliberately configured to be unable to
//! prove descendants stopped, which is the fake's dedicated knob
//! (`docs/repository-layout.md` § Crate map notes this is fake-only).
//! Forcing a real OS process into that state deterministically and
//! portably is exactly the kind of platform-specific proof VP-001 owns,
//! not this skeleton contract test.
//!
//! Gated `#[cfg(unix)]`/`#[cfg(windows)]` per test, like
//! `child_termination.rs`: on Windows, `TokioProcessSupervisor::stop`
//! always reports `OrphanRiskUncertain` (Job Object containment is not yet
//! proven, VP-001 VP-S5), which would violate `process_supervisor_contract`
//! -- so Windows instead runs
//! `unproven_descendants_yield_orphan_risk_uncertain`, asserting that
//! honest-uncertainty behaviour directly.
//!
//! No local `contract-tests` feature gate is needed here: the
//! `omnifrons-app` dev-dependency below always enables its own
//! `contract-tests` feature (see this crate's `Cargo.toml`), so
//! `omnifrons_app::contract` is available to every test in this crate.

use omnifrons_supervisor::TokioProcessSupervisor;

#[cfg(windows)]
use omnifrons_app::contract::process_supervisor::unproven_descendants_yield_orphan_risk_uncertain;
#[cfg(unix)]
use omnifrons_app::contract::process_supervisor::{
    process_supervisor_contract, stop_is_idempotent_after_terminal_state,
};

#[cfg(unix)]
#[test]
fn real_supervisor_satisfies_process_supervisor_contract() {
    process_supervisor_contract(TokioProcessSupervisor::new);
}

#[cfg(unix)]
#[test]
fn real_supervisor_stop_is_idempotent_after_terminal_state() {
    stop_is_idempotent_after_terminal_state(TokioProcessSupervisor::new);
}

#[cfg(windows)]
#[test]
fn windows_stub_reports_orphan_risk_uncertain_on_stop() {
    unproven_descendants_yield_orphan_risk_uncertain(TokioProcessSupervisor::new);
}
