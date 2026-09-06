//! A fast, undrained demo harness must never make `spawn` or the capture
//! reader tasks block: `spawn` returns immediately, `stop` still succeeds
//! within its deadline while the channel is full, and once a consumer
//! finally starts draining, the very next delivered frame surfaces the
//! drops that accumulated while nothing was consuming as a nonzero
//! `dropped_before` -- with the total delivered staying far below the full
//! 20_000-line run, not close to it.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use omnifrons_app::{HarnessKind, HarnessRequest, ProcessOutput, ProcessSupervisor};
use omnifrons_supervisor::TokioProcessSupervisor;

/// Well below the full 20_000-line run: proves backpressure actually
/// bounded delivery rather than eventually delivering everything.
const DELIVERED_FAR_BELOW_FULL_RUN: usize = 5_000;

/// `stop`'s own deadline: the demo harness (`DemoLines`, not
/// `DemoIgnoresSigterm`) has no custom `SIGTERM` handling, so the default
/// disposition terminates it immediately -- this only bounds how long the
/// graceful-stop poll loop is willing to wait before concluding it is not
/// going to see a reap.
const STOP_DEADLINE: Duration = Duration::from_secs(3);

#[test]
fn undrained_high_rate_output_never_blocks_spawn_or_stop() {
    let mut supervisor = TokioProcessSupervisor::with_demo_launcher(PathBuf::from(env!(
        "CARGO_BIN_EXE_demo-harness"
    )));
    let request = HarnessRequest::new(HarnessKind::DemoLines, 1000, 20_000)
        .expect("1000 Hz, 20_000 lines must be a valid request");

    let spawn_started = Instant::now();
    let id = supervisor
        .spawn_harness(request)
        .expect("spawning the demo harness must succeed");
    let spawn_elapsed = spawn_started.elapsed();
    assert!(
        spawn_elapsed < Duration::from_millis(500),
        "spawn must return immediately regardless of how long the child will run, took {spawn_elapsed:?}"
    );

    let rx = supervisor
        .subscribe(id)
        .expect("subscribing right after spawn must succeed");

    // Let the harness run, undrained, long enough for its ~1000-1200
    // frames/second combined stdout+stderr rate to fill the 1024-capacity
    // channel and start dropping.
    std::thread::sleep(Duration::from_millis(1500));

    // Drain a small batch *while the harness is still running*: the
    // reader task's next successful send (there is room again now) is
    // where the drops that piled up while nothing was draining must
    // surface, as that frame's `dropped_before`.
    let mut delivered: Vec<_> = rx.try_iter().take(200).collect();
    assert_eq!(
        delivered.len(),
        200,
        "the channel must have been full (at least 200 queued) before this drain"
    );

    // Give the reader task a moment to notice the freed room and succeed
    // at least one more send before the harness is stopped.
    std::thread::sleep(Duration::from_millis(200));

    let stop_started = Instant::now();
    let terminal = supervisor
        .stop(id, STOP_DEADLINE)
        .expect("stop must succeed even while the output channel is full and undrained");
    let stop_elapsed = stop_started.elapsed();
    assert!(
        stop_elapsed <= STOP_DEADLINE + Duration::from_secs(1),
        "stop must complete within its deadline (plus scheduling slack), took {stop_elapsed:?}"
    );
    assert!(
        matches!(
            terminal,
            omnifrons_app::ProcessTerminalState::Exited { .. }
                | omnifrons_app::ProcessTerminalState::Killed
        ),
        "stop must report a definite terminal state, got {terminal:?}"
    );

    delivered.extend(rx.try_iter());

    assert!(
        delivered.len() < DELIVERED_FAR_BELOW_FULL_RUN,
        "backpressure must keep total delivered far below the full 20_000-line run, got {}",
        delivered.len()
    );
    assert!(
        delivered.iter().any(|frame| frame.dropped_before > 0),
        "at least one delivered frame must surface a nonzero dropped_before once draining \
         resumed after the channel was left full"
    );
}
