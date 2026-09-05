//! A reusable `ProcessSupervisor` contract, runnable against any
//! implementation: an in-memory fake (`omnifrons-app`'s own tests) or a
//! real adapter (`omnifrons-supervisor`'s tests, via this crate as a
//! dev-dependency with the `contract-tests` feature enabled).

use std::time::Duration;

use crate::process_supervisor::{
    ProcessSpec, ProcessStatus, ProcessSupervisor, ProcessTerminalState,
};

/// A spec any conformant `ProcessSupervisor` can spawn: an in-memory fake
/// is free to ignore its content, but a real, OS-backed implementation
/// must be able to actually execute it, observe it running, and stop it.
/// The command differs per platform because a real supervisor really
/// execs it; a fake, which does not, needs none of this.
fn probe_spec() -> ProcessSpec {
    #[cfg(unix)]
    {
        ProcessSpec::new("sleep").with_args(["5"])
    }
    #[cfg(windows)]
    {
        ProcessSpec::new("cmd").with_args(["/C", "timeout", "/T", "5"])
    }
    #[cfg(not(any(unix, windows)))]
    {
        ProcessSpec::new("contract-probe")
    }
}

/// Exercise the baseline `ProcessSupervisor` contract: spawning succeeds,
/// the freshly spawned process observes as running, and stopping it within
/// `deadline` yields a definite terminal state.
///
/// `make` builds a fresh supervisor instance so the contract can be run
/// against implementations that hold internal, per-instance state.
///
/// # Panics
///
/// Panics (via `expect`/`assert*`) if the supervisor under test violates
/// the contract: spawn fails, the process does not observe as running, or
/// stop does not report a definite terminal state within the deadline.
pub fn process_supervisor_contract<S: ProcessSupervisor>(make: impl Fn() -> S) {
    let mut supervisor = make();
    let id = supervisor
        .spawn(probe_spec())
        .expect("spawn must succeed for a contract-conformant supervisor");

    let status = supervisor
        .observe(id)
        .expect("a freshly spawned process must be observable");
    assert_eq!(
        status,
        ProcessStatus::Running,
        "a freshly spawned process must observe as running before it is stopped"
    );

    let terminal = supervisor
        .stop(id, Duration::from_secs(2))
        .expect("stop must succeed within the deadline for a contract-conformant supervisor");
    assert!(
        matches!(
            terminal,
            ProcessTerminalState::Exited { .. } | ProcessTerminalState::Killed
        ),
        "stop must report a definite terminal state (Exited or Killed), got {terminal:?}"
    );
}

/// Require the implementation to report
/// [`ProcessTerminalState::OrphanRiskUncertain`] when it cannot prove every
/// descendant process has stopped (docs/target-architecture.md § Required
/// failure states: "Process descendants unproven stopped ->
/// orphan-risk/uncertain"). `make` must build a supervisor configured to
/// exercise that unproven-descendants path.
///
/// # Panics
///
/// Panics (via `expect`/`assert_eq`) if spawn fails, or if stop does not
/// report `OrphanRiskUncertain`.
pub fn unproven_descendants_yield_orphan_risk_uncertain<S: ProcessSupervisor>(
    make: impl Fn() -> S,
) {
    let mut supervisor = make();
    let id = supervisor
        .spawn(probe_spec())
        .expect("spawn must succeed for a contract-conformant supervisor");

    let terminal = supervisor
        .stop(id, Duration::from_secs(2))
        .expect("stop must return a result even when descendants are unproven");
    assert_eq!(
        terminal,
        ProcessTerminalState::OrphanRiskUncertain,
        "an implementation that cannot prove descendants stopped must report \
         OrphanRiskUncertain rather than a clean exit"
    );
}

/// Require that stopping an already-terminal process is idempotent: a
/// second `stop` call for the same id must report the same terminal state
/// as the first, and `observe` must agree with it.
///
/// This guards against a supervisor re-signalling a process id/group after
/// it has already been confirmed terminal: once reaped, the OS is free to
/// recycle that id for an unrelated process, so a naive implementation that
/// blindly re-sends a signal on every `stop` call risks signalling a
/// stranger. A conformant implementation must instead remember the
/// terminal state and return it directly on any later `stop`, without
/// touching the OS again.
///
/// `make` builds a fresh supervisor instance so the contract can be run
/// against implementations that hold internal, per-instance state.
///
/// # Panics
///
/// Panics (via `expect`/`assert_eq`) if spawn fails, if either `stop` call
/// fails, if the two terminal states differ, or if `observe` disagrees
/// with the terminal state `stop` reported.
pub fn stop_is_idempotent_after_terminal_state<S: ProcessSupervisor>(make: impl Fn() -> S) {
    let mut supervisor = make();
    let id = supervisor
        .spawn(probe_spec())
        .expect("spawn must succeed for a contract-conformant supervisor");

    let first = supervisor
        .stop(id, Duration::from_secs(2))
        .expect("the first stop must succeed for a contract-conformant supervisor");
    let second = supervisor
        .stop(id, Duration::from_secs(2))
        .expect("stopping an already-terminal process again must still succeed");

    assert_eq!(
        first, second,
        "stopping an already-terminal process twice must report the same terminal state \
         each time, not re-signal a possibly-recycled process id/group"
    );

    let observed = supervisor
        .observe(id)
        .expect("a stopped process must remain observable");
    assert_eq!(
        observed,
        ProcessStatus::Terminal(second),
        "observe must agree with the terminal state stop reported"
    );
}
