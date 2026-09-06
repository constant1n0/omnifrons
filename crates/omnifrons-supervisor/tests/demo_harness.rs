//! `DemoIgnoresSigterm` survives `SIGTERM` (its whole point), so `stop`
//! must escalate to `SIGKILL` and still report a definite terminal state
//! (`Killed`) within a bounded deadline. Unix only: the demo harness's
//! `SIGTERM`-ignore handler is installed via `nix::sys::signal::signal`,
//! which this crate only builds on unix (`Cargo.toml`'s
//! `cfg(unix)`-gated `nix` dependency).
//!
//! Waits for the harness's own `"ready"` frame (`demo::run`'s doc comment)
//! before calling `stop`, rather than a fixed sleep and a hope that it was
//! long enough: `ready` is printed only once the handler is already
//! installed (for `DemoIgnoresSigterm`), so observing it is a genuine
//! synchronization point, not a guess.

#![cfg(unix)]

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use omnifrons_app::{
    FramePayload, HarnessKind, HarnessRequest, OutputFrame, ProcessOutput, ProcessSupervisor,
    ProcessTerminalState,
};
use omnifrons_supervisor::TokioProcessSupervisor;

/// How long to wait for the harness's `"ready"` frame before giving up.
///
/// 10s, not a shorter bound: a loaded CI runner needs generous scheduling
/// margin to spawn demo-harness and have it print `"ready"`, independent of
/// `stop`'s own deadline handling measured separately below (`stop_started`
/// is not set until after this wait returns).
const READY_DEADLINE: Duration = Duration::from_secs(10);

/// Block until `rx` delivers the harness's `"ready"` text frame, or panic
/// once `deadline` passes -- the synchronization point that replaces a
/// fixed, hopeful sleep.
fn wait_for_ready(rx: &Receiver<OutputFrame>, deadline: Instant) {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "did not observe the demo harness's ready frame within the deadline"
        );
        match rx.recv_timeout(remaining) {
            Ok(OutputFrame {
                payload: FramePayload::Text { text, .. },
                ..
            }) if text == "ready" => return,
            // Some other frame arrived first, or nothing arrived within this
            // recv's own slice of the budget: either way, loop again -- the
            // outer deadline check above is what actually governs giving up.
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => panic!(
                "the output channel closed before the demo harness's ready frame was observed"
            ),
        }
    }
}

/// The graceful-`SIGTERM` budget passed to `stop`. Deliberately short: a
/// `SIGTERM`-ignoring process can never reap within it by definition, so
/// `stop` is guaranteed to wait out this entire budget before escalating
/// to `SIGKILL` -- keeping it short is what leaves headroom under the 5s
/// overall deadline this test asserts on, not a claim about how fast a
/// normal graceful stop should be.
const SIGTERM_GRACE: Duration = Duration::from_secs(1);

/// The overall deadline this test asserts `stop` completes within,
/// matching the numbered test's own "within a 5s deadline" requirement.
const OVERALL_DEADLINE: Duration = Duration::from_secs(5);

#[test]
fn demo_ignores_sigterm_survives_and_stop_reports_killed_within_deadline() {
    let mut supervisor = TokioProcessSupervisor::with_demo_launcher(PathBuf::from(env!(
        "CARGO_BIN_EXE_demo-harness"
    )));
    // A slow, long-lived run: `SIGTERM` must not be enough to stop it, so
    // it needs to still be going by the time `stop` is called.
    let request = HarnessRequest::new(HarnessKind::DemoIgnoresSigterm, 20, 10_000)
        .expect("20 Hz, 10_000 lines must be a valid request");

    let id = supervisor
        .spawn_harness(request)
        .expect("spawning the demo harness must succeed");
    let rx = supervisor
        .subscribe(id)
        .expect("subscribing right after spawn must succeed");

    // Wait for the harness's own "ready" frame -- printed only once its
    // SIGTERM-ignore handler is already installed -- rather than a fixed
    // sleep and a hope that it was long enough before stop sends the first
    // signal.
    wait_for_ready(&rx, Instant::now() + READY_DEADLINE);

    let stop_started = Instant::now();
    let terminal = supervisor
        .stop(id, SIGTERM_GRACE)
        .expect("stop must succeed within the deadline even against a SIGTERM-ignoring process");
    let elapsed = stop_started.elapsed();

    assert_eq!(
        terminal,
        ProcessTerminalState::Killed,
        "a SIGTERM-ignoring process must be escalated to SIGKILL, not reported as a graceful exit"
    );
    assert!(
        elapsed <= OVERALL_DEADLINE,
        "stop must report Killed within a 5s deadline, took {elapsed:?}"
    );
}
