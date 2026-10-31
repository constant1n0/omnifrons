//! A child that exits on its own ends its run without any operator action.
//!
//! Nothing here calls `observe` or `stop` before the terminal `State` frame
//! is asserted: once a child's output streams have closed, the supervisor
//! itself confirms the reap and delivers the final frame (issue #181).

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use omnifrons_app::{
    FramePayload, OutputFrame, ProcessId, ProcessOutput, ProcessSpec, ProcessStatus,
    ProcessSupervisor, ProcessTerminalState,
};
use omnifrons_supervisor::TokioProcessSupervisor;

/// Generous headroom for a short-lived child on a loaded runner.
const TERMINAL_DEADLINE: Duration = Duration::from_secs(10);
const STOP_DEADLINE: Duration = Duration::from_secs(5);
/// How long the channel may stay open after its `State` frame.
const CLOSE_DEADLINE: Duration = Duration::from_secs(5);
/// How long the "closes its output but keeps running" child lives after
/// closing its output (`sleep`, whole seconds).
#[cfg(unix)]
const LINGER_SECS: u64 = 2;
/// How long the test watches for a premature terminal frame. It must stay
/// well below [`LINGER_SECS`], or a premature frame would be indistinguishable
/// from the child's real exit.
#[cfg(unix)]
const PREMATURE_FRAME_WATCH: Duration = Duration::from_millis(900);

/// The first `State` frame on `frames` within `deadline`, if one arrives.
fn wait_for_state(
    frames: &Receiver<OutputFrame>,
    deadline: Duration,
) -> Option<ProcessTerminalState> {
    let end = Instant::now() + deadline;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        match frames.recv_timeout(left) {
            Ok(frame) => {
                if let FramePayload::State(state) = frame.payload {
                    return Some(state);
                }
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// Drain `frames` until the channel closes and count the `State` frames
/// seen. Bounded: a channel that stays open past [`CLOSE_DEADLINE`] fails
/// instead of hanging the test.
fn count_states_until_closed(frames: &Receiver<OutputFrame>) -> usize {
    let end = Instant::now() + CLOSE_DEADLINE;
    let mut states = 0;
    loop {
        match frames.recv_timeout(end.saturating_duration_since(Instant::now())) {
            Ok(frame) => states += usize::from(matches!(frame.payload, FramePayload::State(_))),
            Err(RecvTimeoutError::Disconnected) => return states,
            Err(RecvTimeoutError::Timeout) => {
                panic!("the output channel stayed open past {CLOSE_DEADLINE:?}")
            }
        }
    }
}

fn spawn_pipe_printer(supervisor: &mut TokioProcessSupervisor) -> ProcessId {
    let spec = ProcessSpec::new(env!("CARGO_BIN_EXE_fake-agent")).with_args(["--stdin-byte-count"]);
    supervisor
        .spawn(spec)
        .expect("spawning the fixture must succeed")
}

#[test]
fn a_pipe_child_that_exits_by_itself_delivers_its_exit_state() {
    let mut supervisor = TokioProcessSupervisor::new();
    let id = spawn_pipe_printer(&mut supervisor);
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");

    let state = wait_for_state(&frames, TERMINAL_DEADLINE);
    assert_eq!(
        state,
        Some(ProcessTerminalState::Exited { code: Some(0) }),
        "a self-exited pipe child must deliver its State frame with no observe or stop"
    );
}

#[cfg(unix)]
#[test]
fn a_pty_child_that_exits_by_itself_delivers_its_exit_state() {
    use std::path::PathBuf;

    use omnifrons_app::{EnvPlan, ExecHandle, LaunchPlan, StdinPlan, WorkspaceRoot};
    use omnifrons_domain::adapter::TransportClass;
    use omnifrons_domain::scope::ScopeMode;

    let dir = std::env::temp_dir().join(format!("omnifrons-self-exit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("failed to create the test workspace directory");
    let program = std::fs::canonicalize(env!("CARGO_BIN_EXE_fake-agent"))
        .expect("the fixture must canonicalize");
    let file = std::fs::File::open(&program).expect("opening the fixture must succeed");
    let plan = LaunchPlan {
        argv: vec!["--pty-report".to_string()],
        env: EnvPlan::new(&[]).expect("an empty declared-key list can never be secret-shaped"),
        cwd: WorkspaceRoot::new(&dir).expect("the created dir must be a valid workspace"),
        stdin: StdinPlan::Null,
        prompt: None,
        scope_mode: ScopeMode::Advisory,
        transport: TransportClass::Pty,
        output_dir: None,
    };

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(ExecHandle::File(file), PathBuf::from(&program), &plan)
        .expect("spawning the pty fixture must succeed");
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");

    let state = wait_for_state(&frames, TERMINAL_DEADLINE);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        state,
        Some(ProcessTerminalState::Exited { code: Some(0) }),
        "a self-exited pty child must deliver its State frame with no observe or stop"
    );
}

#[cfg(unix)]
#[test]
fn a_child_that_closes_its_output_but_keeps_running_is_not_finished_early() {
    let mut supervisor = TokioProcessSupervisor::new();
    let script = format!("exec 1>&- 2>&-; exec sleep {LINGER_SECS}");
    let spec = ProcessSpec::new("/bin/sh").with_args(["-c", script.as_str()]);
    let id = supervisor.spawn(spec).expect("spawning sh must succeed");
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");

    // Both streams are closed almost at once; the child is still alive.
    assert_eq!(
        wait_for_state(&frames, PREMATURE_FRAME_WATCH),
        None,
        "no terminal frame may be delivered while the child is still running"
    );
    assert_eq!(supervisor.observe(id), Some(ProcessStatus::Running));

    assert_eq!(
        wait_for_state(&frames, TERMINAL_DEADLINE),
        Some(ProcessTerminalState::Exited { code: Some(0) }),
        "the terminal frame must follow the child's own exit"
    );
}

#[test]
fn observe_and_stop_after_a_self_exit_report_the_same_terminal_state() {
    let mut supervisor = TokioProcessSupervisor::new();
    let id = spawn_pipe_printer(&mut supervisor);
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");

    let exited = ProcessTerminalState::Exited { code: Some(0) };
    assert_eq!(wait_for_state(&frames, TERMINAL_DEADLINE), Some(exited));

    assert_eq!(
        supervisor.observe(id),
        Some(ProcessStatus::Terminal(exited))
    );
    assert_eq!(
        supervisor.observe(id),
        Some(ProcessStatus::Terminal(exited))
    );
    assert_eq!(
        supervisor
            .stop(id, STOP_DEADLINE)
            .expect("stop after a self-exit must succeed"),
        exited
    );
    // Exactly one State frame was ever delivered: the channel is now closed.
    assert_eq!(
        count_states_until_closed(&frames),
        0,
        "a second State frame must never be delivered"
    );
}

/// A stop that begins while the reaper is polling a child that closed its
/// output decides the terminal state: the reaper must not pre-empt it with a
/// bare `Exited`, and exactly one `State` frame is delivered.
#[cfg(unix)]
#[test]
fn a_stop_while_the_reaper_polls_decides_the_terminal_state() {
    let mut supervisor = TokioProcessSupervisor::new();
    // SIGTERM is ignored (and the disposition survives the exec), so the
    // stop has to escalate to SIGKILL: `Killed` can only come from `stop`.
    let spec = ProcessSpec::new("/bin/sh")
        .with_args(["-c", "trap '' TERM; exec 1>&- 2>&-; exec sleep 30"]);
    let id = supervisor.spawn(spec).expect("spawning sh must succeed");
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");

    // Output is closed at once, so the reaper is polling by now.
    assert_eq!(wait_for_state(&frames, Duration::from_millis(400)), None);
    assert_eq!(supervisor.observe(id), Some(ProcessStatus::Running));

    let stopped = supervisor
        .stop(id, Duration::from_millis(300))
        .expect("stop must succeed");
    assert_eq!(stopped, ProcessTerminalState::Killed);
    assert_eq!(
        supervisor.observe(id),
        Some(ProcessStatus::Terminal(ProcessTerminalState::Killed))
    );

    let mut states = Vec::new();
    let end = Instant::now() + CLOSE_DEADLINE;
    loop {
        match frames.recv_timeout(end.saturating_duration_since(Instant::now())) {
            Ok(frame) => {
                if let FramePayload::State(state) = frame.payload {
                    states.push(state);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => panic!("the output channel stayed open"),
        }
    }
    assert_eq!(states, vec![ProcessTerminalState::Killed]);
}

/// Descendants left behind by a child that exits on its own make the self-exit
/// path report what `stop` would: the helper below keeps running in the
/// child's process group after its output was redirected away, so the
/// readers end at the child's exit with a descendant still alive.
#[cfg(target_os = "linux")]
#[test]
fn a_self_exit_that_leaves_a_descendant_reports_orphan_risk() {
    let mut supervisor = TokioProcessSupervisor::new();
    // Self-bounding helper: it exits on its own after 3 s.
    let spec = ProcessSpec::new("/bin/sh")
        .with_args(["-c", "sleep 3 >/dev/null 2>&1 </dev/null & exit 0"]);
    let id = supervisor.spawn(spec).expect("spawning sh must succeed");
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");

    assert_eq!(
        wait_for_state(&frames, TERMINAL_DEADLINE),
        Some(ProcessTerminalState::OrphanRiskUncertain),
        "a leftover descendant must not be reported as a clean exit"
    );
}

/// A child that closes its output, leaves a SIGTERM-ignoring helper the
/// reaper's census will record (so the settle sweep takes about 300 ms), then
/// starts a second helper the census cannot have seen and exits. The settled
/// verdict is `OrphanRiskUncertain` (the group is not empty); the window
/// between the reap and that verdict is wide enough to hit.
///
/// Returns the supervisor, the id, the subscription and the marker file the
/// child creates just before it exits.
#[cfg(target_os = "linux")]
fn spawn_late_helper_child() -> (
    TokioProcessSupervisor,
    ProcessId,
    Receiver<OutputFrame>,
    std::path::PathBuf,
) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let marker = std::env::temp_dir().join(format!(
        "omnifrons-settling-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&marker);
    let script = format!(
        "(trap '' TERM; exec sleep 5) >/dev/null 2>&1 </dev/null & \
         exec 1>&- 2>&-; sleep 0.4; \
         (exec sleep 3) >/dev/null 2>&1 </dev/null & \
         : > {marker}; exit 0",
        marker = marker.display()
    );
    let mut supervisor = TokioProcessSupervisor::new();
    let spec = ProcessSpec::new("/bin/sh").with_args(["-c", script.as_str()]);
    let id = supervisor.spawn(spec).expect("spawning sh must succeed");
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    let end = Instant::now() + TERMINAL_DEADLINE;
    while !marker.exists() {
        assert!(Instant::now() < end, "the child never reached its marker");
        std::thread::sleep(Duration::from_millis(5));
    }
    (supervisor, id, frames, marker)
}

#[cfg(target_os = "linux")]
#[test]
fn a_concurrent_observe_never_reports_the_unsettled_exit() {
    let (supervisor, id, frames, marker) = spawn_late_helper_child();
    // The child exits within milliseconds of its marker; from then on the
    // reaper is sweeping. Observe the whole time.
    let end = Instant::now() + TERMINAL_DEADLINE;
    let last = loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(state)) => break state,
            Some(ProcessStatus::Running) => {}
            other => panic!("unexpected status {other:?}"),
        }
        assert!(Instant::now() < end, "never reached a terminal state");
        std::thread::sleep(Duration::from_millis(2));
    };
    let _ = std::fs::remove_file(&marker);
    assert_eq!(
        last,
        ProcessTerminalState::OrphanRiskUncertain,
        "observe must not report the reap-derived state while the sweep runs"
    );
    assert_eq!(
        wait_for_state(&frames, TERMINAL_DEADLINE),
        Some(ProcessTerminalState::OrphanRiskUncertain)
    );
    assert_eq!(
        count_states_until_closed(&frames),
        0,
        "exactly one State frame"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_concurrent_stop_waits_for_the_settled_state() {
    let (mut supervisor, id, frames, marker) = spawn_late_helper_child();
    // Past the child's exit, inside the sweep.
    std::thread::sleep(Duration::from_millis(100));
    let stopped = supervisor
        .stop(id, STOP_DEADLINE)
        .expect("stop must succeed");
    let _ = std::fs::remove_file(&marker);
    assert_eq!(stopped, ProcessTerminalState::OrphanRiskUncertain);
    assert_eq!(
        wait_for_state(&frames, TERMINAL_DEADLINE),
        Some(ProcessTerminalState::OrphanRiskUncertain)
    );
    assert_eq!(
        count_states_until_closed(&frames),
        0,
        "exactly one State frame"
    );
}
