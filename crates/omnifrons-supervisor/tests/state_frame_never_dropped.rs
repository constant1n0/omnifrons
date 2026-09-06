//! The terminal `State` frame must reach the subscriber even when the
//! delivery channel is completely full and undrained at the moment the
//! process is confirmed reaped: unlike a text frame, it is not best-effort.
//! A consumer that starts draining late -- after the process has already
//! exited -- must still see it as the very last frame once it drains
//! everything, never silently lose it to the same `try_send`-drops-the-
//! newest-frame policy that governs stdout/stderr text.
//!
//! Driven through a direct `ProcessSpec` with the test-only `--burst` flag
//! (bypassing `HarnessRequest`/`HarnessKind`, like `output_backpressure.rs`
//! and `output_framing.rs`), not the rate-limited real demo harness: this
//! test used to drive `HarnessKind::DemoLines` at 1000 Hz for 1500 lines,
//! relying on that producing enough output to overflow the channel within a
//! fixed wall-clock deadline. On macOS, coarse timer granularity stretches
//! the resulting 1 ms inter-line sleeps far past what was requested, so the
//! demo harness sometimes had not even finished emitting its lines -- let
//! alone exited -- by the time the deadline below elapsed, and the test
//! failed waiting for a terminal state that had not arrived yet, not on the
//! property actually under test. `--burst` writes every line with no
//! inter-line sleeping at all, which removes that platform dependency
//! entirely while still deterministically overflowing the 1024-capacity
//! channel.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use omnifrons_app::{FramePayload, ProcessOutput, ProcessSpec, ProcessStatus, ProcessSupervisor};
use omnifrons_supervisor::TokioProcessSupervisor;

/// Comfortably above the 1024-frame channel capacity, so the burst
/// deterministically overflows it regardless of platform timer granularity.
const BURST_LINES: u32 = 1500;

const REAP_DEADLINE: Duration = Duration::from_secs(10);
const DRAIN_DEADLINE: Duration = Duration::from_secs(5);

fn demo_harness_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_demo-harness"))
}

#[test]
fn the_final_state_frame_survives_a_full_undrained_channel() {
    let mut supervisor = TokioProcessSupervisor::with_demo_launcher(demo_harness_path());

    let spec = ProcessSpec::new(demo_harness_path().to_string_lossy().into_owned())
        .with_args(["--burst".to_string(), BURST_LINES.to_string()]);

    let id = supervisor
        .spawn(spec)
        .expect("spawning the demo harness must succeed");
    let rx = supervisor
        .subscribe(id)
        .expect("subscribing right after spawn must succeed");

    // Deliberately never drained while the process runs: by the time it
    // exits, the 1024-capacity channel must be full and dropping frames.
    let deadline = Instant::now() + REAP_DEADLINE;
    loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(_)) => break,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => panic!("demo harness did not reach a terminal state in time: {other:?}"),
        }
    }

    // Drain everything now, with a bounded per-item wait: if the final
    // state frame were dropped instead of guaranteed-delivered, this would
    // simply run out of frames without ever seeing one, not hang -- so a
    // plain `rx.iter()` (which blocks until the sender side is fully
    // dropped) is exactly the right drain here, bounded by wrapping the
    // whole drain in a timeout thread.
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let frames: Vec<_> = rx.iter().collect();
        let _ = done_tx.send(frames);
    });
    let frames = done_rx
        .recv_timeout(DRAIN_DEADLINE)
        .expect("draining the channel to its close must finish well within the drain deadline");

    assert!(
        !frames.is_empty(),
        "the drain must have delivered at least the final state frame"
    );

    let dropped_before_state: u64 = match &frames.last().expect("checked non-empty above").payload {
        FramePayload::State(state) => {
            assert_eq!(
                *state,
                omnifrons_app::ProcessTerminalState::Exited { code: Some(0) },
                "the demo harness must exit 0"
            );
            frames.last().unwrap().dropped_before
        }
        text @ FramePayload::Text { .. } => panic!(
            "the last frame delivered after a full drain must be the terminal State frame, \
             got {text:?} -- the state frame was dropped instead of guaranteed-delivered"
        ),
    };

    assert!(
        dropped_before_state > 0,
        "this run is only meaningful if frames were actually dropped before the state frame \
         arrived (proving the channel really was full); got dropped_before = 0"
    );
}
