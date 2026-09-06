//! Output-capture bookkeeping must not accumulate forever: once a process's
//! output channel is both finalized (its final `State` frame produced) and
//! subscribed-out (the one-shot receiver already taken), the entry is
//! removed entirely rather than retained for the supervisor's whole
//! lifetime. That removal is externally observable through
//! `ProcessOutput::subscribe`'s own error contract: a *live* subscription
//! reports `AlreadySubscribed` on a second attempt, but a *removed* entry
//! is indistinguishable from an id this supervisor never captured output
//! for at all, so a second `subscribe` after full drain reports
//! `UnknownProcess` instead.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use omnifrons_app::{
    FramePayload, HarnessKind, HarnessRequest, ProcessOutput, ProcessStatus, ProcessSupervisor,
    SupervisorError,
};
use omnifrons_supervisor::TokioProcessSupervisor;

// 10s, not a shorter bound: this waits for a demo harness driven by
// `rate_hz` sleeps to reach a terminal state, and a loaded CI runner's
// coarser timer granularity can stretch those sleeps well past what this
// machine sees, so the deadline needs generous headroom.
const REAP_DEADLINE: Duration = Duration::from_secs(10);

#[test]
fn a_finalized_and_fully_drained_entry_is_removed() {
    let mut supervisor = TokioProcessSupervisor::with_demo_launcher(PathBuf::from(env!(
        "CARGO_BIN_EXE_demo-harness"
    )));
    let request =
        HarnessRequest::new(HarnessKind::DemoLines, 200, 5).expect("valid harness request");

    let id = supervisor
        .spawn_harness(request)
        .expect("spawning the demo harness must succeed");
    let rx = supervisor
        .subscribe(id)
        .expect("the first subscribe must succeed");

    let deadline = Instant::now() + REAP_DEADLINE;
    loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(_)) => break,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => panic!("demo harness did not reach a terminal state in time: {other:?}"),
        }
    }

    // Drain to the channel's actual close: the sender side (held by the
    // finalize logic's delivery thread) must be dropped once the state
    // frame is sent, which only happens once finalize has fully run --
    // including any removal of this entry from the supervisor's own table.
    let frames: Vec<_> = rx.iter().collect();
    assert!(
        matches!(
            frames.last().map(|f| &f.payload),
            Some(FramePayload::State(_))
        ),
        "the drain must end with the terminal State frame, got {frames:?}"
    );

    let second_subscribe = supervisor
        .subscribe(id)
        .expect_err("a second subscribe after full drain must be rejected");
    assert_eq!(
        second_subscribe,
        SupervisorError::UnknownProcess,
        "a finalized, fully-drained entry must be removed entirely -- reported as \
         UnknownProcess, not AlreadySubscribed (which would mean it is still retained)"
    );
}
