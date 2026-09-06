//! Captures a real demo harness run end to end: 10 lines at a modest rate
//! yields exactly 11 stdout frames (a leading `"ready"` line -- printed only
//! once the process has finished any setup for this run, `demo::run`'s own
//! doc comment -- plus the 10 `"line <n> out"` lines) and 2 stderr frames
//! (every 5th line, per `omnifrons_supervisor::demo::run`), and a final
//! `State(Exited{code: Some(0)})` frame, with a contiguous `seq` and the
//! correct stream on each text frame.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use omnifrons_app::{
    FramePayload, HarnessKind, HarnessRequest, OutputStream, ProcessOutput, ProcessStatus,
    ProcessSupervisor,
};
use omnifrons_supervisor::TokioProcessSupervisor;

/// How long to wait for the demo harness (a handful of lines at a modest
/// rate) to run to completion and be confirmed reaped.
///
/// 10s, not a shorter bound: this run's total wall-clock time still depends
/// on `rate_hz` sleeps between lines (unlike the burst-driven tests), and a
/// loaded/virtualized CI runner's coarser timer granularity can stretch
/// each of those sleeps well past what a developer's own machine sees, so
/// the deadline needs generous headroom rather than merely covering the
/// nominal (200 Hz, 10 lines) runtime.
const REAP_DEADLINE: Duration = Duration::from_secs(10);

#[test]
fn ten_lines_yield_ten_stdout_two_stderr_and_a_final_exited_state() {
    let mut supervisor = TokioProcessSupervisor::with_demo_launcher(PathBuf::from(env!(
        "CARGO_BIN_EXE_demo-harness"
    )));
    let request = HarnessRequest::new(HarnessKind::DemoLines, 200, 10)
        .expect("200 Hz, 10 lines must be a valid request");

    let id = supervisor
        .spawn_harness(request)
        .expect("spawning the demo harness must succeed");
    let rx = supervisor
        .subscribe(id)
        .expect("subscribing right after spawn must succeed");

    // Drive the supervisor's own reap confirmation -- output capture's
    // final `State` frame is sent only once that is confirmed (see
    // `docs/spike-log.md` § IPC contract).
    let deadline = Instant::now() + REAP_DEADLINE;
    loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(_)) => break,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => panic!("demo harness did not reach a terminal state in time: {other:?}"),
        }
    }

    let frames: Vec<_> = rx.iter().collect();

    let stdout_frames: Vec<_> = frames
        .iter()
        .filter(|frame| {
            matches!(
                frame.payload,
                FramePayload::Text {
                    stream: OutputStream::Stdout,
                    ..
                }
            )
        })
        .collect();
    let stderr_frames: Vec<_> = frames
        .iter()
        .filter(|frame| {
            matches!(
                frame.payload,
                FramePayload::Text {
                    stream: OutputStream::Stderr,
                    ..
                }
            )
        })
        .collect();
    let state_frames: Vec<_> = frames
        .iter()
        .filter(|frame| matches!(frame.payload, FramePayload::State(_)))
        .collect();

    assert_eq!(
        stdout_frames.len(),
        11,
        "expected 11 stdout frames (a leading \"ready\" plus 10 \"line <n> out\"), got {frames:#?}"
    );
    match &stdout_frames[0].payload {
        FramePayload::Text { text, .. } => {
            assert_eq!(
                text, "ready",
                "the first stdout frame must be the ready line"
            );
        }
        state @ FramePayload::State(_) => panic!("expected a Text payload, got {state:?}"),
    }
    assert_eq!(
        stderr_frames.len(),
        2,
        "expected 2 stderr frames, got {frames:#?}"
    );
    assert_eq!(
        state_frames.len(),
        1,
        "expected exactly one final state frame, got {frames:#?}"
    );

    match &state_frames[0].payload {
        FramePayload::State(state) => {
            assert_eq!(
                *state,
                omnifrons_app::ProcessTerminalState::Exited { code: Some(0) },
                "the demo harness must exit 0"
            );
        }
        text @ FramePayload::Text { .. } => panic!("expected a State payload, got {text:?}"),
    }

    // The state frame must be the very last one delivered.
    assert!(matches!(
        frames.last().unwrap().payload,
        FramePayload::State(_)
    ));

    // seq is contiguous across the whole per-process sequence space.
    let seqs: Vec<u64> = frames.iter().map(|frame| frame.seq).collect();
    let expected: Vec<u64> = (0..frames.len() as u64).collect();
    assert_eq!(seqs, expected, "seq must be contiguous from 0");

    // Every 5th stdout line also produced a stderr frame, so stderr lines
    // read "line 5 out" / "line 10 out" analogues on stderr.
    for frame in &stderr_frames {
        if let FramePayload::Text { text, .. } = &frame.payload {
            assert!(
                text == "line 5 err" || text == "line 10 err",
                "unexpected stderr text: {text}"
            );
        }
    }
}
