//! A run whose child has exited ends after a bounded grace even when a
//! descendant keeps the child's output open, and the settled verdict names a
//! descendant that left the process group (issue #191).
//!
//! Case 1: `sh -c 'sleep N & exit 0'` -- the helper inherits stdout, so the
//! readers never reach EOF. The supervisor polls for the child's exit from
//! spawn, gives the readers a grace of 2 s from that exit, then ends the run
//! as `orphan-risk/uncertain`, preceded by a diagnostic `stderr` frame. The
//! helper stays alive, and whatever it writes later is discarded: the `State`
//! frame stays the last frame on the channel. Stop can still signal the
//! process group while a recorded member of it is alive.
//!
//! Case 2 (Linux): a descendant that calls `setsid` leaves the group, so the
//! group check cannot see it; the census taken while the child lives can, and
//! the verdict names it.
//!
//! Every helper here is self-bounding ([`HELPER_SECS`]) and killed by the
//! test's own [`Helper`] guard, even when an assertion fails first.

#![cfg(unix)]

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use omnifrons_app::{
    FramePayload, OutputFrame, OutputStream, ProcessId, ProcessOutput, ProcessSpec, ProcessStatus,
    ProcessSupervisor, ProcessTerminalState,
};
use omnifrons_supervisor::TokioProcessSupervisor;

/// The grace the supervisor gives a run's readers after its child exits.
const EXIT_GRACE: Duration = Duration::from_secs(2);

/// Scheduling headroom on top of [`EXIT_GRACE`]: noticing the exit (one
/// 250 ms poll while the output is still open) and settling the verdict take
/// well under a second; the rest is room for a loaded runner.
const MARGIN: Duration = Duration::from_secs(4);

/// How long a test waits for a line the child prints at once.
const LINE_DEADLINE: Duration = Duration::from_secs(10);

/// How long every helper sleeps if nothing kills it first.
const HELPER_SECS: u32 = 30;

/// The diagnostic that precedes the `State` frame when a descendant still
/// holds the output open once the grace is over.
const LINGERING_NOTE: &str = "the child exited, but a descendant still holds its output open: \
                              the run ends here and later output is discarded";

/// The diagnostic naming a recorded descendant that left the process group.
#[cfg(target_os = "linux")]
fn escaped_note(pid: i32, comm: &str) -> String {
    format!("descendant {pid} ({comm}) left the run's process group and is still running")
}

/// The supervisor's `tracing` output in this test binary, as text.
static LOG: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

/// Where the global `tracing` subscriber writes: into [`LOG`].
struct Capture;

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        LOG.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Route the supervisor's warnings into [`LOG`] (once per binary).
fn capture_warnings() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .with_writer(|| Capture)
        .try_init();
}

/// The captured log lines for process `pid` that contain `needle`.
fn warnings_for(pid: u32, needle: &str) -> Vec<String> {
    let log = String::from_utf8_lossy(
        &LOG.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
    .into_owned();
    let field = format!("pid={pid} ");
    log.lines()
        .filter(|line| line.contains(&field) && line.contains(needle))
        .map(str::to_string)
        .collect()
}

/// Kills the helper it names when dropped, so a failing assertion never
/// leaves a process behind.
struct Helper(nix::unistd::Pid);

impl Helper {
    #[cfg(target_os = "linux")]
    fn pid(&self) -> i32 {
        self.0.as_raw()
    }

    /// Whether the helper is still running (a zombie counts as gone).
    fn is_running(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            match std::fs::read_to_string(format!("/proc/{}/stat", self.pid())) {
                Ok(stat) => {
                    let after_comm = &stat[stat.rfind(')').map_or(0, |at| at + 1)..];
                    after_comm.split_whitespace().next() != Some("Z")
                }
                Err(_) => false,
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            nix::sys::signal::kill(self.0, None).is_ok()
        }
    }

    /// Whether the helper is gone within `budget`.
    #[cfg(target_os = "linux")]
    fn ends_within(&self, budget: Duration) -> bool {
        let end = Instant::now() + budget;
        loop {
            if !self.is_running() {
                return true;
            }
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = nix::sys::signal::kill(self.0, nix::sys::signal::Signal::SIGKILL);
    }
}

/// Wait for the first text frame and read it as the helper's pid (the
/// scripts below print `$!` before anything else).
fn helper_from_first_line(frames: &Receiver<OutputFrame>, seen: &mut Vec<OutputFrame>) -> Helper {
    let end = Instant::now() + LINE_DEADLINE;
    loop {
        let frame = frames
            .recv_timeout(end.saturating_duration_since(Instant::now()))
            .expect("the child must print the helper's pid");
        let pid = match &frame.payload {
            FramePayload::Text { text, .. } => text.trim().parse::<i32>().ok(),
            FramePayload::State(_) => None,
        };
        seen.push(frame);
        if let Some(pid) = pid {
            return Helper(nix::unistd::Pid::from_raw(pid));
        }
    }
}

/// Wait until every one of `wanted` has arrived as a text line, counting
/// those already in `seen`; every frame received is kept in `seen`.
fn wait_for_lines(frames: &Receiver<OutputFrame>, seen: &mut Vec<OutputFrame>, wanted: &[&str]) {
    let has = |seen: &[OutputFrame], line: &str| {
        seen.iter().any(|frame| {
            matches!(&frame.payload, FramePayload::Text { text, .. } if text.trim() == line)
        })
    };
    let end = Instant::now() + LINE_DEADLINE;
    while !wanted.iter().all(|line| has(seen, line)) {
        let frame = frames
            .recv_timeout(end.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|_| panic!("never saw all of {wanted:?}: {:#?}", tail(seen)));
        seen.push(frame);
    }
}

/// Collect every frame until the channel closes, or fail once `budget` has
/// passed since `since`. Returns the frames and when the channel closed.
fn collect_until_closed(
    frames: &Receiver<OutputFrame>,
    seen: &mut Vec<OutputFrame>,
    since: Instant,
    budget: Duration,
) -> Duration {
    let end = since + budget;
    loop {
        match frames.recv_timeout(end.saturating_duration_since(Instant::now())) {
            Ok(frame) => seen.push(frame),
            Err(RecvTimeoutError::Disconnected) => return since.elapsed(),
            Err(RecvTimeoutError::Timeout) => panic!(
                "the run did not end within {budget:?}; frames so far: {:#?}",
                tail(seen)
            ),
        }
    }
}

/// The last few frames, for a failure message.
fn tail(frames: &[OutputFrame]) -> &[OutputFrame] {
    &frames[frames.len().saturating_sub(4)..]
}

/// The run's one `State` frame, asserting it is the last frame received.
fn last_state(frames: &[OutputFrame]) -> ProcessTerminalState {
    let states: Vec<_> = frames
        .iter()
        .filter(|frame| matches!(frame.payload, FramePayload::State(_)))
        .collect();
    assert_eq!(
        states.len(),
        1,
        "exactly one State frame per run: {frames:#?}"
    );
    match frames.last().map(|frame| &frame.payload) {
        Some(FramePayload::State(state)) => *state,
        _ => panic!(
            "the State frame must be the last frame: {:#?}",
            tail(frames)
        ),
    }
}

/// The text of the frame right before the `State` frame, asserting it is a
/// `stderr` diagnostic.
fn diagnostic_before_state(frames: &[OutputFrame]) -> String {
    let before = frames
        .len()
        .checked_sub(2)
        .and_then(|at| frames.get(at))
        .map(|frame| &frame.payload);
    match before {
        Some(FramePayload::Text {
            stream: OutputStream::Stderr,
            text,
            ..
        }) => text.clone(),
        _ => panic!(
            "a stderr diagnostic must precede the State frame: {:#?}",
            tail(frames)
        ),
    }
}

fn spawn_sh(supervisor: &mut TokioProcessSupervisor, script: &str) -> ProcessId {
    let spec = ProcessSpec::new("/bin/sh").with_args(["-c", script]);
    supervisor.spawn(spec).expect("spawning sh must succeed")
}

/// Issue #191, case 1, through the pipe transport: the helper inherits
/// stdout, the child exits at once, and the run still ends after the grace.
#[test]
fn a_pipe_run_ends_after_the_grace_while_a_descendant_holds_its_output() {
    let mut supervisor = TokioProcessSupervisor::new();
    let started = Instant::now();
    let id = spawn_sh(
        &mut supervisor,
        &format!("sleep {HELPER_SECS} & echo \"$!\"; exit 0"),
    );
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    let mut seen = Vec::new();
    let helper = helper_from_first_line(&frames, &mut seen);

    let closed_after = collect_until_closed(&frames, &mut seen, started, EXIT_GRACE + MARGIN);

    assert_eq!(
        last_state(&seen),
        ProcessTerminalState::OrphanRiskUncertain,
        "a descendant holding the output open is not a clean exit"
    );
    assert_eq!(diagnostic_before_state(&seen), LINGERING_NOTE);
    assert!(
        closed_after >= EXIT_GRACE,
        "the readers get the whole grace after the exit; the run ended after {closed_after:?}"
    );
    assert!(
        helper.is_running(),
        "the descendant stays alive: ending the run signals nothing"
    );
    assert_eq!(
        supervisor.observe(id),
        Some(ProcessStatus::Terminal(
            ProcessTerminalState::OrphanRiskUncertain
        ))
    );
}

/// Issue #191, case 1, through the PTY transport. The child is the session
/// leader, and what its exit does to the terminal differs by kernel:
///
/// - Linux (`drivers/tty/tty_jobctrl.c`, `disassociate_ctty`): on exit, a
///   session leader's *pseudo*-terminal is not hung up; its foreground group
///   only gets SIGHUP. The helper ignores SIGHUP, keeps holding the slave
///   open, and the run ends after the grace, `orphan-risk/uncertain`.
/// - macOS (XNU `bsd/kern/kern_exit.c`, `proc_exit`): for a session leader
///   with a controlling terminal the kernel signals the foreground group,
///   drains the terminal, and revokes access to it (`VNOP_REVOKE` with
///   `REVOKEALL`), so the helper's slave descriptors stop holding it: the
///   master reaches EOF and the run ends then, without the grace. With no
///   `/proc` there is no census or group check either, so the helper that
///   is still running goes unseen and the state is the child's own
///   `Exited(0)` -- the documented gap of the platforms without `/proc`.
///   (Read from the kernel source; not measured here, only by CI.)
#[test]
fn a_pty_run_ends_after_the_grace_while_a_descendant_holds_the_terminal() {
    use std::path::PathBuf;

    use omnifrons_app::{EnvPlan, ExecHandle, LaunchPlan, StdinPlan, WorkspaceRoot};
    use omnifrons_domain::adapter::TransportClass;
    use omnifrons_domain::scope::ScopeMode;

    let workspace = Scratch::new("pty-lingering");
    let shell = PathBuf::from("/bin/sh");
    let file = std::fs::File::open(&shell).expect("opening /bin/sh must succeed");
    let plan = LaunchPlan {
        argv: vec![
            "-c".to_string(),
            format!("trap '' HUP; sleep {HELPER_SECS} & echo \"$!\"; exit 0"),
        ],
        env: EnvPlan::new(&[]).expect("an empty declared-key list can never be secret-shaped"),
        cwd: WorkspaceRoot::new(workspace.path()).expect("the scratch dir is a valid workspace"),
        stdin: StdinPlan::Null,
        prompt: None,
        scope_mode: ScopeMode::Advisory,
        transport: TransportClass::Pty,
        output_dir: None,
    };

    let mut supervisor = TokioProcessSupervisor::new();
    let started = Instant::now();
    let id = supervisor
        .spawn_approved(ExecHandle::File(file), shell, &plan)
        .expect("spawning sh on the PTY path must succeed");
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    let mut seen = Vec::new();
    let helper = helper_from_first_line(&frames, &mut seen);

    let closed_after = collect_until_closed(&frames, &mut seen, started, EXIT_GRACE + MARGIN);

    #[cfg(target_os = "linux")]
    {
        assert_eq!(last_state(&seen), ProcessTerminalState::OrphanRiskUncertain);
        assert_eq!(diagnostic_before_state(&seen), LINGERING_NOTE);
        assert!(closed_after >= EXIT_GRACE, "ended after {closed_after:?}");
    }
    #[cfg(not(target_os = "linux"))]
    {
        // No hang either: the revoked terminal reaches EOF and the run ends
        // there, so no lingering note precedes the state.
        let _ = closed_after;
        assert_eq!(
            last_state(&seen),
            ProcessTerminalState::Exited { code: Some(0) }
        );
        assert!(
            !seen.iter().any(|frame| matches!(
                &frame.payload,
                FramePayload::Text { text, .. } if text == LINGERING_NOTE
            )),
            "a revoked terminal ends the run at EOF: {seen:#?}"
        );
    }
    assert!(helper.is_running(), "the descendant stays alive");
}

/// Issue #191, case 2: the helper calls `setsid`, so the process-group check
/// cannot see it. The child lives on for a second -- longer than the census
/// interval -- so a census taken while it lives records the helper, and the
/// verdict names it. (With the issue's literal `& exit 0`, the helper
/// escapes within the census's stated blind spot: the child is gone before
/// any walk can link the two.)
#[cfg(target_os = "linux")]
#[test]
fn an_escaped_descendant_recorded_while_the_child_lived_is_named_in_the_verdict() {
    let mut supervisor = TokioProcessSupervisor::new();
    let id = spawn_sh(
        &mut supervisor,
        &format!(
            "setsid sleep {HELPER_SECS} >/dev/null 2>&1 </dev/null & echo \"$!\"; sleep 1; exit 0"
        ),
    );
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    let mut seen = Vec::new();
    let helper = helper_from_first_line(&frames, &mut seen);

    let _ = collect_until_closed(&frames, &mut seen, Instant::now(), LINE_DEADLINE);

    let state = last_state(&seen);
    assert_ne!(
        state,
        ProcessTerminalState::Exited { code: Some(0) },
        "an escaped descendant that outlived the child is not a clean exit"
    );
    assert_eq!(state, ProcessTerminalState::OrphanRiskUncertain);
    assert_eq!(
        diagnostic_before_state(&seen),
        escaped_note(helper.pid(), "sleep")
    );
    assert!(helper.is_running(), "the verdict signals nothing");
}

/// The `State` frame stays the last frame even while a lingering descendant
/// keeps writing after the grace: its later output is discarded.
#[test]
fn output_written_after_the_grace_never_follows_the_state_frame() {
    let mut supervisor = TokioProcessSupervisor::new();
    let started = Instant::now();
    // Writes a line every 100 ms for ten seconds, far past the grace.
    let id = spawn_sh(
        &mut supervisor,
        "(i=0; while [ \"$i\" -lt 100 ]; do echo late; sleep 0.1; i=$((i+1)); done) & \
         echo \"$!\"; exit 0",
    );
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    let mut seen = Vec::new();
    let helper = helper_from_first_line(&frames, &mut seen);

    let closed_after = collect_until_closed(&frames, &mut seen, started, EXIT_GRACE + MARGIN);

    assert_eq!(last_state(&seen), ProcessTerminalState::OrphanRiskUncertain);
    assert!(
        seen.iter().any(|frame| matches!(
            &frame.payload,
            FramePayload::Text { text, .. } if text == "late"
        )),
        "the helper's output before the end is delivered"
    );
    assert!(
        helper.is_running(),
        "the helper was still writing when the run ended after {closed_after:?}"
    );
}

/// The verifier's probe c2: what a lingering descendant writes after the
/// run ended is discarded, and the reader that discarded it logs how many
/// bytes, once, when it ends. The other reader (stderr, which the helper
/// also holds but never writes to) discarded nothing and logs nothing.
#[test]
fn only_a_reader_that_discarded_output_logs_it_once() {
    const NEEDLE: &str = "outlived its run's final state frame";
    capture_warnings();
    let mut supervisor = TokioProcessSupervisor::new();
    let started = Instant::now();
    let id = spawn_sh(
        &mut supervisor,
        "(i=0; while [ \"$i\" -lt 100 ]; do echo late; sleep 0.1; i=$((i+1)); done) & \
         echo \"$!\"; exit 0",
    );
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    let mut seen = Vec::new();
    let helper = helper_from_first_line(&frames, &mut seen);
    let _ = collect_until_closed(&frames, &mut seen, started, EXIT_GRACE + MARGIN);

    // Let the helper write past the end of the run, then end it: both
    // readers reach EOF within its last `sleep 0.1`.
    std::thread::sleep(Duration::from_millis(500));
    drop(helper);
    let end = Instant::now() + LINE_DEADLINE;
    while warnings_for(id.0, NEEDLE).is_empty() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(500));

    let warnings = warnings_for(id.0, NEEDLE);
    assert_eq!(warnings.len(), 1, "one warning, from stdout: {warnings:#?}");
    assert!(warnings[0].contains("stream=Stdout"), "{warnings:#?}");
    let discarded: u64 = warnings[0]
        .split("discarded_bytes=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|count| count.parse().ok())
        .expect("the warning names its discarded byte count");
    assert!(discarded > 0, "{warnings:#?}");
}

/// After a self-exit whose descendant holds the output, Stop still reaches
/// the process group: a member the census sighted while the child lived is
/// alive with its original start time, so the group is provably the
/// original one. The child lives a second after forking the helper, longer
/// than the census interval, so a walk sights the helper; one forked and
/// orphaned within a millisecond (`sleep N & exit 0`) is never sighted, and
/// Stop then signals nothing (`descendants::Leftovers::members`).
#[cfg(target_os = "linux")]
#[test]
fn stop_after_the_recorded_exit_still_signals_the_lingering_group() {
    let mut supervisor = TokioProcessSupervisor::new();
    let id = spawn_sh(
        &mut supervisor,
        &format!("sleep {HELPER_SECS} & echo \"$!\"; sleep 1; exit 0"),
    );
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    let mut seen = Vec::new();
    let helper = helper_from_first_line(&frames, &mut seen);

    let end = Instant::now() + Duration::from_secs(1) + EXIT_GRACE + MARGIN;
    let recorded = loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(state)) => break state,
            _ if Instant::now() < end => std::thread::sleep(Duration::from_millis(20)),
            other => panic!("the exit was never recorded: {other:?}"),
        }
    };

    let stopped = supervisor
        .stop(id, Duration::from_secs(2))
        .expect("stop after a recorded exit must succeed");

    assert!(
        helper.ends_within(Duration::from_secs(3)),
        "stop must reach the lingering descendant through its process group"
    );
    assert_eq!(recorded, ProcessTerminalState::OrphanRiskUncertain);
    assert_eq!(
        stopped, recorded,
        "stop reports the recorded state unchanged"
    );
    let _ = collect_until_closed(&frames, &mut seen, Instant::now(), LINE_DEADLINE);
    assert_eq!(last_state(&seen), recorded);
}

/// The same grace applies after Stop: a descendant that left the process
/// group and lost its parent link to the child before Stop -- so neither
/// Stop's group signal nor its census can reach it -- keeps the output open
/// after the stopped child is reaped, and the run still ends, with the state
/// Stop reported.
///
/// Stop is called only once both are established. The escapee
/// (`fake-agent --setsid-sleep`, since macOS has no `setsid(1)`) prints
/// `descendant-ready` after its own `setsid(2)`, and the child prints
/// `escaped` only after the subshell that forked it has exited. The pid line
/// alone proves neither: a Stop that arrives earlier still reaches the
/// escapee, through the group signal or by sweeping it as a recorded
/// descendant -- correctly -- and the run then ends at EOF without the grace
/// (the CI failure on #196).
#[test]
fn a_stopped_run_ends_after_the_grace_while_an_escaped_descendant_holds_its_output() {
    let mut supervisor = TokioProcessSupervisor::new();
    let script =
        format!("(\"$1\" --setsid-sleep & echo \"$!\"); echo escaped; exec sleep {HELPER_SECS}");
    let spec = ProcessSpec::new("/bin/sh").with_args([
        "-c".to_string(),
        script,
        "sh".to_string(),
        env!("CARGO_BIN_EXE_fake-agent").to_string(),
    ]);
    let id = supervisor.spawn(spec).expect("spawning sh must succeed");
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    let mut seen = Vec::new();
    let helper = helper_from_first_line(&frames, &mut seen);
    wait_for_lines(&frames, &mut seen, &["descendant-ready", "escaped"]);

    let stopped = supervisor
        .stop(id, Duration::from_secs(2))
        .expect("stop must succeed");
    let stopped_at = Instant::now();

    let closed_after = collect_until_closed(&frames, &mut seen, stopped_at, EXIT_GRACE + MARGIN);

    assert_eq!(last_state(&seen), stopped, "the frame carries stop's state");
    assert_eq!(diagnostic_before_state(&seen), LINGERING_NOTE);
    assert!(closed_after <= EXIT_GRACE + MARGIN);
    assert!(
        helper.is_running(),
        "the escaped helper is beyond stop's reach"
    );
}

/// The verifier's probe c4: a Stop that arrives during the exit grace waits
/// for the settled verdict, reports it, and still reaches the lingering
/// group. As above, the child lives long enough for a census walk to sight
/// the helper.
#[cfg(target_os = "linux")]
#[test]
fn a_stop_during_the_grace_reports_the_verdict_and_reaches_the_group() {
    let mut supervisor = TokioProcessSupervisor::new();
    let id = spawn_sh(
        &mut supervisor,
        &format!("sleep {HELPER_SECS} & echo \"$!\"; sleep 1; exit 0"),
    );
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    let mut seen = Vec::new();
    let helper = helper_from_first_line(&frames, &mut seen);

    // Once the reaper has reaped the child its grace is running, and the
    // helper keeps the readers inside it.
    let child = std::path::PathBuf::from(format!("/proc/{}", id.0));
    let end = Instant::now() + LINE_DEADLINE;
    while child.exists() {
        assert!(Instant::now() < end, "the child was never reaped");
        std::thread::sleep(Duration::from_millis(10));
    }
    let stopped = supervisor
        .stop(id, Duration::from_secs(1))
        .expect("stop during the grace must succeed");

    assert!(
        helper.ends_within(Duration::from_secs(3)),
        "stop must reach the lingering descendant through its process group"
    );
    assert_eq!(stopped, ProcessTerminalState::OrphanRiskUncertain);
    let _ = collect_until_closed(&frames, &mut seen, Instant::now(), LINE_DEADLINE);
    assert_eq!(last_state(&seen), stopped);
    assert_eq!(diagnostic_before_state(&seen), LINGERING_NOTE);
}

/// Collect every frame until the channel closes, at most `budget` after
/// `since`, polling `observe` every 5 ms meanwhile when `observing`. Returns
/// whether the channel closed in time.
fn collect_while_observing(
    supervisor: &TokioProcessSupervisor,
    id: ProcessId,
    observing: bool,
    frames: &Receiver<OutputFrame>,
    seen: &mut Vec<OutputFrame>,
    since: Instant,
    budget: Duration,
) -> bool {
    let end = since + budget;
    loop {
        let wait = if observing {
            let _ = supervisor.observe(id);
            Duration::from_millis(5)
        } else {
            end.saturating_duration_since(Instant::now())
        };
        match frames.recv_timeout(wait) {
            Ok(frame) => seen.push(frame),
            Err(RecvTimeoutError::Disconnected) => return true,
            Err(RecvTimeoutError::Timeout) if Instant::now() >= end => return false,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// One `State` frame, last, and every `seq` from 0 without a gap.
fn run_invariants_hold(frames: &[OutputFrame]) -> bool {
    let states = frames
        .iter()
        .filter(|frame| matches!(frame.payload, FramePayload::State(_)))
        .count();
    let contiguous = frames
        .iter()
        .zip(0_u64..)
        .all(|(frame, expected)| frame.seq == expected);
    states == 1
        && matches!(
            frames.last().map(|frame| &frame.payload),
            Some(FramePayload::State(_))
        )
        && contiguous
}

/// The verifier's probe c5: clean exits, lingering descendants, Stop of a
/// running child, Stop during the grace, and a child that closes its output
/// before it exits, on twelve threads sharing one supervisor, half of them
/// observing while they collect. Every run ends, with exactly one `State`
/// frame, last, and frame numbers without a gap.
#[test]
fn a_concurrent_mix_of_runs_keeps_every_run_invariant() {
    let supervisor = TokioProcessSupervisor::new();
    let workers: Vec<_> = (0..12_usize)
        .map(|thread| {
            let mut supervisor = supervisor.clone();
            std::thread::spawn(move || {
                let mut broken = Vec::new();
                for run in 0..4 {
                    let scenario = (thread + run) % 5;
                    let script = match scenario {
                        0 => "echo a; echo b >&2; exit 0".to_string(),
                        1 | 4 => format!("sleep {HELPER_SECS} & echo \"$!\"; exit 0"),
                        2 => format!("echo up; exec sleep {HELPER_SECS}"),
                        _ => "echo up; exec >&- 2>&-; sleep 0.3; exit 2".to_string(),
                    };
                    let started = Instant::now();
                    let id = spawn_sh(&mut supervisor, &script);
                    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
                    let mut seen = Vec::new();
                    // Killed when this run ends, whatever Stop did.
                    let _helper = matches!(scenario, 1 | 4)
                        .then(|| helper_from_first_line(&frames, &mut seen));
                    if scenario == 2 {
                        if let Ok(frame) = frames.recv_timeout(LINE_DEADLINE) {
                            seen.push(frame);
                        }
                        let _ = supervisor.stop(id, Duration::from_secs(1));
                    } else if scenario == 4 {
                        std::thread::sleep(Duration::from_millis(700));
                        let _ = supervisor.stop(id, Duration::from_secs(1));
                    }
                    let closed = collect_while_observing(
                        &supervisor,
                        id,
                        thread % 2 == 0,
                        &frames,
                        &mut seen,
                        started,
                        Duration::from_secs(12),
                    );
                    if !closed || !run_invariants_hold(&seen) {
                        broken.push(format!(
                            "thread {thread} run {run} scenario {scenario} closed={closed}: \
                             {:#?}",
                            tail(&seen)
                        ));
                    }
                }
                broken
            })
        })
        .collect();
    let broken: Vec<String> = workers
        .into_iter()
        .flat_map(|worker| worker.join().expect("a worker thread panicked"))
        .collect();
    assert!(broken.is_empty(), "{broken:#?}");
}

/// A scratch directory under the system temp dir, removed on drop.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "omnifrons-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("the scratch dir must be creatable");
        Self(dir)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
