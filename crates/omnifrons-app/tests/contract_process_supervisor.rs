//! Runs the reusable `ProcessSupervisor` contract over an in-memory fake.
//!
//! Only compiled with `--features contract-tests` (see
//! `omnifrons-app`'s `contract-tests` feature and
//! `docs/repository-layout.md` § Build and test commands).

#![cfg(feature = "contract-tests")]

use std::collections::HashMap;
use std::time::Duration;

use omnifrons_app::contract::process_supervisor::{
    process_supervisor_contract, stop_is_idempotent_after_terminal_state,
    unproven_descendants_yield_orphan_risk_uncertain,
};
use omnifrons_app::{ProcessId, ProcessSpec, ProcessStatus, ProcessSupervisor, SupervisorError};
use omnifrons_domain::scope::ProcessTerminalState;

/// An in-memory `ProcessSupervisor` test double.
///
/// It never spawns a real OS process. `stop` resolves immediately to a
/// canned terminal state and *remembers* it, so a later `stop` or `observe`
/// call for the same id reports that same recorded state rather than
/// recomputing it -- mirroring the real supervisor's requirement that an
/// already-terminal process must not be re-signalled or misreported. The
/// `unproven_descendants` knob makes the first `stop` call report
/// `OrphanRiskUncertain` instead of a clean exit, simulating a supervisor
/// that cannot prove every descendant has stopped
/// (docs/target-architecture.md § Required failure states).
struct FakeSupervisor {
    next_id: u32,
    // `None` means still running; `Some(state)` records the terminal state
    // reported by `stop`, which `observe` and any later `stop` call must
    // agree with.
    processes: HashMap<ProcessId, Option<ProcessTerminalState>>,
    unproven_descendants: bool,
}

impl FakeSupervisor {
    fn new() -> Self {
        Self {
            next_id: 0,
            processes: HashMap::new(),
            unproven_descendants: false,
        }
    }

    fn with_unproven_descendants() -> Self {
        Self {
            unproven_descendants: true,
            ..Self::new()
        }
    }
}

impl ProcessSupervisor for FakeSupervisor {
    fn spawn(&mut self, _spec: ProcessSpec) -> Result<ProcessId, SupervisorError> {
        self.next_id += 1;
        let id = ProcessId(self.next_id);
        self.processes.insert(id, None);
        Ok(id)
    }

    fn stop(
        &mut self,
        id: ProcessId,
        _deadline: Duration,
    ) -> Result<ProcessTerminalState, SupervisorError> {
        match self.processes.get(&id) {
            None => Err(SupervisorError::UnknownProcess),
            // Already terminal: report the same recorded state again,
            // rather than recomputing (and never re-signalling).
            Some(Some(state)) => Ok(*state),
            Some(None) => {
                let state = if self.unproven_descendants {
                    ProcessTerminalState::OrphanRiskUncertain
                } else {
                    ProcessTerminalState::Exited { code: Some(0) }
                };
                self.processes.insert(id, Some(state));
                Ok(state)
            }
        }
    }

    fn observe(&self, id: ProcessId) -> Option<ProcessStatus> {
        self.processes
            .get(&id)
            .map(|maybe_state| match maybe_state {
                Some(state) => ProcessStatus::Terminal(*state),
                None => ProcessStatus::Running,
            })
    }
}

#[test]
fn fake_supervisor_satisfies_process_supervisor_contract() {
    process_supervisor_contract(FakeSupervisor::new);
}

#[test]
fn fake_supervisor_reports_orphan_risk_for_unproven_descendants() {
    unproven_descendants_yield_orphan_risk_uncertain(FakeSupervisor::with_unproven_descendants);
}

#[test]
fn fake_supervisor_stop_is_idempotent_after_terminal_state() {
    stop_is_idempotent_after_terminal_state(FakeSupervisor::new);
}

#[test]
fn fake_supervisor_stop_is_idempotent_after_terminal_state_with_unproven_descendants() {
    stop_is_idempotent_after_terminal_state(FakeSupervisor::with_unproven_descendants);
}
