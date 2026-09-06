//! Proves the supervisor's private Tokio runtime is actually driven, not
//! merely entered.
//!
//! `TokioProcessSupervisor::new` used to build a current-thread `Runtime`
//! and only `enter()` it around each synchronous call -- enough for the
//! runtime's handle to exist so `tokio::process::Command` can be
//! constructed, but nothing ever polled a task spawned onto that runtime,
//! since `enter()` alone does not drive it (only `block_on`/`run` does).
//! The output-capture reader tasks (`docs/spike-log.md` § IPC contract)
//! need a runtime that is genuinely driven in the background, so this test
//! spawns a trivial task onto the supervisor's own runtime and asserts it
//! actually completes within a bounded wait -- red before the driver-thread
//! fix, since a task spawned but never polled would otherwise hang forever.

use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use omnifrons_supervisor::TokioProcessSupervisor;

#[test]
fn a_task_spawned_on_the_supervisors_runtime_is_actually_polled() {
    let supervisor = TokioProcessSupervisor::new();
    let (tx, rx) = std::sync::mpsc::channel();

    supervisor.spawn_on_runtime(async move {
        tx.send(()).expect("test receiver must still be alive");
    });

    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(()) => {}
        Err(RecvTimeoutError::Timeout) => panic!(
            "the spawned task never ran within 2s -- the supervisor's runtime is not being \
             driven in the background"
        ),
        Err(RecvTimeoutError::Disconnected) => {
            panic!("the sender was dropped without sending -- the spawned task never completed")
        }
    }
}

/// `output_capture.rs`'s `ReaderHandles` doc comment explains why its
/// `seq`/`pending_drops` counters need no lock around their own
/// read-then-write sequence in `try_send`: it is sound only because a
/// process's stdout and stderr reader tasks both run on this one
/// current-thread runtime, never truly in parallel. This test stands guard
/// on that assumption directly, from inside a task actually spawned onto
/// the supervisor's own runtime (not merely asserting on how the runtime
/// was configured from the outside).
#[test]
fn the_supervisors_runtime_is_current_thread_flavor() {
    let supervisor = TokioProcessSupervisor::new();
    let (tx, rx) = std::sync::mpsc::channel();

    supervisor.spawn_on_runtime(async move {
        let flavor = tokio::runtime::Handle::current().runtime_flavor();
        tx.send(flavor).expect("test receiver must still be alive");
    });

    let flavor = match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(flavor) => flavor,
        Err(RecvTimeoutError::Timeout) => {
            panic!("the spawned task never reported its runtime flavor within 2s")
        }
        Err(RecvTimeoutError::Disconnected) => {
            panic!("the sender was dropped without sending -- the spawned task never completed")
        }
    };

    assert_eq!(
        flavor,
        tokio::runtime::RuntimeFlavor::CurrentThread,
        "the supervisor's runtime must stay current-thread flavor: output_capture's \
         seq/dropped-before bookkeeping (ReaderHandles' own doc comment) is sound only \
         because a process's two reader tasks can never truly run in parallel"
    );
}
