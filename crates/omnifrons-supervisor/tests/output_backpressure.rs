//! A fast, undrained demo harness must never make `spawn` or the capture
//! reader tasks block: `spawn` returns immediately, the child overflows the
//! bounded output channel and exits on its own, and once a consumer finally
//! drains it, the delivered frames stay bounded by the channel's own
//! capacity, at least one of them surfaces the drops that piled up while
//! nothing was consuming as a nonzero `dropped_before`, and the guaranteed
//! terminal `State` frame still arrives last. `stop` on the already-exited
//! child then still reports `Exited`, not an error or a hang.
//!
//! Deliberately deterministic, with no wall-clock assumption about how fast
//! the child can produce output: `--burst` (a test-only `demo-harness` flag,
//! reachable only through a direct `ProcessSpec` -- bypassing
//! `HarnessRequest`, like the framing flags in `output_framing.rs` -- never
//! via `HarnessKind`) writes every line with no inter-line sleeping, instead
//! of the rate-limited real demo harness (`HarnessKind::DemoLines`) this
//! test used to drive at 1000 Hz. That rate-based version relied on the
//! demo producing more than the channel's 1024-capacity within a fixed
//! wall-clock window; on macOS, coarse timer granularity made the 1 ms
//! sleeps between lines run far slower than requested, so the channel never
//! filled before the test stopped it. Writing the whole burst with no
//! sleeping at all removes that platform dependency entirely.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use omnifrons_app::{
    FramePayload, ProcessId, ProcessOutput, ProcessSpec, ProcessStatus, ProcessSupervisor,
    ProcessTerminalState,
};
use omnifrons_supervisor::TokioProcessSupervisor;

/// Comfortably more than the channel's 1024 capacity, so the burst is
/// guaranteed to overflow it regardless of scheduling.
const BURST_LINES: u32 = 20_000;

/// The per-child channel's own bound (`CHANNEL_CAPACITY` in
/// `src/output_capture.rs`) plus a small margin for the one guaranteed
/// terminal `State` frame, which is delivered outside the bounded
/// best-effort path and so can land as the one frame past capacity.
const MAX_DELIVERED: usize = 1024 + 8;

/// `stop`'s own deadline: irrelevant to how long this actually takes, since
/// the child is already `Terminal` by the time `stop` is called here (see
/// `unix::stop`/`windows::stop`'s own already-`Terminal` fast path), but a
/// bound is still required by the `ProcessSupervisor` contract.
const STOP_DEADLINE: Duration = Duration::from_secs(3);

/// Bounded poll deadline for the burst child to reach a terminal state on
/// its own. Generous: bursting `20_000` lines with no sleeping is normally
/// well under a second, but a loaded CI runner is given ample room.
const REAP_DEADLINE: Duration = Duration::from_secs(10);

fn demo_harness_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_demo-harness"))
}

/// Poll `observe` until `id` reports a terminal state, or panic once
/// `REAP_DEADLINE` elapses.
fn wait_for_terminal(supervisor: &TokioProcessSupervisor, id: ProcessId) -> ProcessTerminalState {
    let deadline = Instant::now() + REAP_DEADLINE;
    loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(state)) => return state,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            other => panic!("demo-harness did not reach a terminal state in time: {other:?}"),
        }
    }
}

#[test]
fn undrained_high_rate_output_never_blocks_spawn_or_stop() {
    let mut supervisor = TokioProcessSupervisor::with_demo_launcher(demo_harness_path());

    let spec = ProcessSpec::new(demo_harness_path().to_string_lossy().into_owned())
        .with_args(["--burst".to_string(), BURST_LINES.to_string()]);

    let spawn_started = Instant::now();
    let id = supervisor
        .spawn(spec)
        .expect("spawning the demo harness must succeed");
    let spawn_elapsed = spawn_started.elapsed();
    assert!(
        spawn_elapsed < Duration::from_millis(500),
        "spawn must return immediately regardless of how long the child will run, took {spawn_elapsed:?}"
    );

    // Undrained: subscribed right away, but nothing reads from `rx` until
    // the child has already run its whole burst to completion.
    let rx = supervisor
        .subscribe(id)
        .expect("subscribing right after spawn must succeed");

    // `--burst` writes with no sleeping, so the child exits on its own once
    // it has emitted every line -- no explicit `stop` needed to make it
    // stop producing.
    let terminal = wait_for_terminal(&supervisor, id);
    assert!(
        matches!(terminal, ProcessTerminalState::Exited { .. }),
        "the burst child must exit on its own once done bursting, got {terminal:?}"
    );

    let delivered: Vec<_> = rx.iter().collect();

    assert!(
        delivered.len() <= MAX_DELIVERED,
        "backpressure must keep total delivered at most the channel's own capacity plus a \
         small margin for the guaranteed state frame, got {} (burst was {BURST_LINES} lines)",
        delivered.len()
    );
    assert!(
        delivered.iter().any(|frame| frame.dropped_before > 0),
        "at least one delivered frame must surface a nonzero dropped_before, proving the \
         channel was left full and overflowing while nothing was draining it"
    );
    assert!(
        matches!(
            delivered.last().map(|frame| &frame.payload),
            Some(FramePayload::State(_))
        ),
        "the last delivered frame must be the guaranteed terminal State frame, got {:?}",
        delivered.last()
    );

    // Separate from the above: `stop` on a child already confirmed
    // `Terminal` must still succeed and report the same `Exited` state,
    // never error or hang.
    let stopped = supervisor
        .stop(id, STOP_DEADLINE)
        .expect("stop on an already-exited child must still succeed");
    assert!(
        matches!(stopped, ProcessTerminalState::Exited { .. }),
        "stop on an already-exited child must report Exited, got {stopped:?}"
    );
}
