//! Integration tests against a real OS child process.
//!
//! `spawned_child_terminates_and_reports_exited` is the load-bearing test:
//! it spawns a genuinely long-lived process, observes it running, stops it
//! within a bounded deadline, and asserts both a definite terminal state
//! and that the PID is actually gone (unix: `kill(pid, 0)` fails with
//! `ESRCH`) -- not just that our own bookkeeping says so.

// Everything below (imports included) is `#[cfg(unix)]`-only: the only
// active test on Windows is the stub further down, which uses none of it,
// and an unconditional import/const here would otherwise be an unused-item
// warning on that platform -- denied as an error by `-D warnings`.

#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use omnifrons_app::{ProcessSpec, ProcessStatus, ProcessSupervisor, ProcessTerminalState};
#[cfg(unix)]
use omnifrons_supervisor::TokioProcessSupervisor;

// 5s, not 2s: this test spawns and reaps a real OS process, and a loaded CI
// runner needs more scheduling margin than a developer's own machine to
// avoid a flaky failure that has nothing to do with the behaviour under
// test. `KILL_GRACE` inside the supervisor is unaffected -- it bounds only
// the SIGKILL escalation step, not this test's overall deadline.
#[cfg(unix)]
const STOP_DEADLINE: Duration = Duration::from_secs(5);

#[cfg(unix)]
#[test]
fn spawned_child_terminates_and_reports_exited() {
    let mut supervisor = TokioProcessSupervisor::new();
    let spec = ProcessSpec::new("sleep").with_args(["30"]);
    let id = supervisor.spawn(spec).expect("spawn must succeed");

    let status = supervisor
        .observe(id)
        .expect("a freshly spawned process must be observable");
    assert_eq!(
        status,
        ProcessStatus::Running,
        "the child must observe as running before stop"
    );

    let terminal = supervisor
        .stop(id, STOP_DEADLINE)
        .expect("stop must succeed within the deadline");
    assert!(
        matches!(
            terminal,
            ProcessTerminalState::Exited { .. } | ProcessTerminalState::Killed
        ),
        "expected a definite terminal state, got {terminal:?}"
    );

    // The PID must actually be gone, not merely marked stopped in our own
    // bookkeeping: kill(pid, 0) sends no signal but still fails with ESRCH
    // once the process has been reaped.
    let pid = nix::unistd::Pid::from_raw(i32::try_from(id.0).expect("pid fits in i32"));
    let err = nix::sys::signal::kill(pid, None).expect_err("pid must no longer exist after stop");
    assert_eq!(err, nix::errno::Errno::ESRCH);
}

/// A supervisor dropped without an explicit `stop` for every child it
/// spawned (e.g. a panicking test, or an early return) must not orphan a
/// still-running child: `Drop` sends SIGKILL to the process group of every
/// entry still `Running`, best effort.
///
/// Both this test's own drop-triggered signal and Tokio's `kill_on_drop`
/// cleanup race to reap the child once dropped, so this polls for the pid
/// to be gone (`ESRCH`, as in `spawned_child_terminates_and_reports_exited`)
/// rather than asserting on who specifically reaped it -- a single
/// `waitpid` here can otherwise lose that race and observe `ECHILD`.
#[cfg(unix)]
#[test]
fn dropping_the_supervisor_kills_running_children() {
    let mut supervisor = TokioProcessSupervisor::new();
    let spec = ProcessSpec::new("sleep").with_args(["30"]);
    let id = supervisor.spawn(spec).expect("spawn must succeed");

    let status = supervisor
        .observe(id)
        .expect("a freshly spawned process must be observable");
    assert_eq!(
        status,
        ProcessStatus::Running,
        "the child must observe as running before the supervisor is dropped"
    );

    let pid = nix::unistd::Pid::from_raw(i32::try_from(id.0).expect("pid fits in i32"));

    // Dropped without an explicit `stop`.
    drop(supervisor);

    let deadline = std::time::Instant::now() + STOP_DEADLINE;
    loop {
        match nix::sys::signal::kill(pid, None) {
            Err(nix::errno::Errno::ESRCH) => break,
            _ if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            other => panic!(
                "expected the pid to be gone shortly after dropping the supervisor \
                 (not left running for its full lifetime), last check: {other:?}"
            ),
        }
    }
}

#[cfg(windows)]
#[test]
#[ignore = "VP-001 VP-S5: Job Object containment not yet implemented"]
fn windows_job_object_containment_terminates_descendants() {
    unimplemented!("Job Object containment pending VP-001 VP-S5 evidence")
}
