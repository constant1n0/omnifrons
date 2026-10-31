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
    let spec = ProcessSpec::new("/bin/sh").with_args(["-c", "exec 1>&- 2>&-; exec sleep 2"]);
    let id = supervisor.spawn(spec).expect("spawning sh must succeed");
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");

    // Both streams are closed almost at once; the child is still alive.
    assert_eq!(
        wait_for_state(&frames, Duration::from_millis(900)),
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
    assert!(
        frames
            .iter()
            .all(|frame| !matches!(frame.payload, FramePayload::State(_))),
        "a second State frame must never be delivered"
    );
}
