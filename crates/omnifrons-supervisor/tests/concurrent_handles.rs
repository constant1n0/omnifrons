//! `TokioProcessSupervisor` must be a cheap `Clone` handle over shared
//! state, not a value that needs an external lock held for an entire call's
//! duration to be shared across threads (that external lock is exactly what
//! the Tauri shell used to wrap it in, serializing every command behind
//! whichever one happened to be running `stop`).
//!
//! This proves the property directly at the crate level: while `stop` is
//! blocked escalating SIGTERM -> SIGKILL against a process that ignores
//! SIGTERM (a multi-second call, bounded by its own deadline), `observe` of
//! a *different* process on another cloned handle, from another thread,
//! must return promptly -- it must never wait for the concurrent `stop` to
//! finish.
//!
//! The concurrency assertion itself holds on every platform, but what
//! `stop` completing actually proves is platform-specific: on unix it is a
//! confirmed `Killed` after a real SIGTERM/SIGKILL escalation, while on
//! Windows `stop` is still the Job Object placeholder (VP-001 VP-S5) and
//! always reports `OrphanRiskUncertain` -- see the platform-gated
//! assertions at the end of the test for why that also makes the
//! concurrency assertion trivially true there.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use omnifrons_app::{HarnessKind, HarnessRequest, ProcessSupervisor};
use omnifrons_supervisor::TokioProcessSupervisor;

/// Generous relative to realistic lock-hold times (a `try_wait` call and a
/// hash-map lookup), but far below `stop`'s multi-second deadline -- so this
/// only fails if `observe` is actually serialized behind the concurrent
/// `stop`, not on ordinary scheduling jitter.
const OBSERVE_MUST_RETURN_WITHIN: Duration = Duration::from_millis(200);

#[test]
fn observe_on_another_handle_is_not_blocked_by_a_concurrent_stop() {
    let mut supervisor = TokioProcessSupervisor::with_demo_launcher(PathBuf::from(env!(
        "CARGO_BIN_EXE_demo-harness"
    )));

    let stopping_request = HarnessRequest::new(HarnessKind::DemoIgnoresSigterm, 20, 10_000)
        .expect("20 Hz, 10_000 lines must be a valid request");
    let stopping_id = supervisor
        .spawn_harness(stopping_request)
        .expect("spawning the SIGTERM-ignoring demo harness must succeed");

    let observed_request = HarnessRequest::new(HarnessKind::DemoLines, 5, 5)
        .expect("5 Hz, 5 lines must be a valid request");
    let observed_id = supervisor
        .spawn_harness(observed_request)
        .expect("spawning the observed demo harness must succeed");

    // Give both children a moment to actually start (and the SIGTERM-ignore
    // handler to be installed) before `stop` sends its first signal.
    std::thread::sleep(Duration::from_millis(150));

    let mut stopper = supervisor.clone();
    let stop_thread = std::thread::spawn(move || stopper.stop(stopping_id, Duration::from_secs(3)));

    // Let the concurrent `stop` actually get underway -- past its SIGTERM
    // send and into its graceful-wait poll loop -- before measuring
    // `observe`'s own latency against it.
    std::thread::sleep(Duration::from_millis(200));

    let observer = supervisor.clone();
    let observe_started = Instant::now();
    let _ = observer.observe(observed_id);
    let observe_elapsed = observe_started.elapsed();

    assert!(
        observe_elapsed < OBSERVE_MUST_RETURN_WITHIN,
        "observe of an unrelated process must not be blocked by a concurrent stop \
         escalating against a different one, took {observe_elapsed:?}"
    );

    let terminal = stop_thread
        .join()
        .expect("the stop thread must not panic")
        .expect("stop must still succeed once it completes");

    // Unix: `stop` (crates/omnifrons-supervisor/src/lib.rs `unix::stop`)
    // genuinely waited out SIGTERM's grace period, escalated to SIGKILL,
    // and confirmed the reap -- `Killed` is a real, proven outcome, and the
    // concurrency assertion above means something: `observe` returned
    // promptly *while* that multi-second escalation was actually in
    // flight.
    #[cfg(unix)]
    assert_eq!(
        terminal,
        omnifrons_app::ProcessTerminalState::Killed,
        "the SIGTERM-ignoring process must have been escalated to SIGKILL"
    );

    // Windows: `windows::stop` (crates/omnifrons-supervisor/src/lib.rs) is
    // a stub pending the Job Object implementation (VP-001 VP-S5) -- it
    // never signals the process group or waits out a deadline at all, it
    // just calls `start_kill` best-effort and reports `OrphanRiskUncertain`
    // immediately. That also means the concurrency assertion above is
    // trivially true on this platform: `stop` returns almost instantly, so
    // there is no multi-second window for `observe` to actually race
    // against -- this test still runs unconditionally on Windows so that
    // fact stays documented here rather than the test being silently
    // skipped.
    #[cfg(windows)]
    assert_eq!(
        terminal,
        omnifrons_app::ProcessTerminalState::OrphanRiskUncertain,
        "the Windows stop stub (VP-001 VP-S5 Job Object placeholder) always reports \
         OrphanRiskUncertain, never a proven Killed"
    );
}
