//! `ProcessSupervisor::spawn` -- the plain, `ProcessSpec`-driven path --
//! honors `ProcessSpec::new`'s default `StdinPlan::Null` on every
//! platform: a child that reads its own stdin to EOF observes zero bytes
//! (R3-014). Uses the `fake-agent` test binary's `--stdin-byte-count`
//! mode, exactly as `tests/approved_launch.rs` does for `spawn_approved`'s
//! `ExecHandle::File` branch, so both spawn paths are proven the same way.

use std::time::{Duration, Instant};

use omnifrons_app::{
    FramePayload, ProcessOutput, ProcessSpec, ProcessStatus, ProcessSupervisor, StdinPlan,
};
use omnifrons_supervisor::TokioProcessSupervisor;

/// Generous headroom for a short-lived fixture process to run to
/// completion and be confirmed reaped on a loaded CI runner.
const REAP_DEADLINE: Duration = Duration::from_secs(10);

#[test]
fn plain_spawn_defaults_to_a_null_stdin_on_every_platform() {
    let spec = ProcessSpec::new(env!("CARGO_BIN_EXE_fake-agent")).with_args(["--stdin-byte-count"]);
    assert_eq!(
        spec.stdin,
        StdinPlan::Null,
        "ProcessSpec::new must default to StdinPlan::Null"
    );

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn(spec)
        .expect("spawning the fake-agent binary directly must succeed");
    let rx = supervisor
        .subscribe(id)
        .expect("subscribing right after spawn must succeed");

    let deadline = Instant::now() + REAP_DEADLINE;
    loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(_)) => break,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => panic!("fake-agent did not reach a terminal state in time: {other:?}"),
        }
    }
    let frames: Vec<_> = rx.iter().collect();

    let saw_zero_bytes = frames.iter().any(|frame| {
        matches!(
            &frame.payload,
            FramePayload::Text { text, .. } if text == "stdin-byte-count 0"
        )
    });
    assert!(
        saw_zero_bytes,
        "the child must read exactly zero bytes from the default null stdin, got {frames:#?}"
    );
}
