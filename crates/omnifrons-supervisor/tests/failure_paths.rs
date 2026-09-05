//! Failure-path tests: an executable that cannot be spawned reports
//! `SupervisorError::Spawn`, and operations against a `ProcessId` this
//! supervisor never handed out via `spawn` report `SupervisorError::UnknownProcess`
//! (`stop`) or `None` (`observe`), rather than panicking or silently
//! succeeding.

use std::time::Duration;

use omnifrons_app::{ProcessId, ProcessSpec, ProcessSupervisor, SupervisorError};
use omnifrons_supervisor::TokioProcessSupervisor;

#[test]
fn spawning_a_nonexistent_executable_reports_spawn_error() {
    let mut supervisor = TokioProcessSupervisor::new();
    let spec = ProcessSpec::new("omnifrons-definitely-does-not-exist-binary");

    let error = supervisor
        .spawn(spec)
        .expect_err("spawning a nonexistent executable must fail");

    assert!(
        matches!(error, SupervisorError::Spawn(_)),
        "expected SupervisorError::Spawn, got {error:?}"
    );
}

#[test]
fn stop_on_an_unknown_process_id_reports_unknown_process() {
    let mut supervisor = TokioProcessSupervisor::new();
    let unknown = ProcessId(u32::MAX);

    let error = supervisor
        .stop(unknown, Duration::from_secs(1))
        .expect_err("stop on an id this supervisor never spawned must fail");

    assert!(
        matches!(error, SupervisorError::UnknownProcess),
        "expected SupervisorError::UnknownProcess, got {error:?}"
    );
}

#[test]
fn observe_on_an_unknown_process_id_reports_none() {
    let supervisor = TokioProcessSupervisor::new();
    let unknown = ProcessId(u32::MAX);

    assert_eq!(
        supervisor.observe(unknown),
        None,
        "observe on an id this supervisor never spawned must report None"
    );
}
