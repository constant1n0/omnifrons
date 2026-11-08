//! Input forwarded to a live pseudo-terminal child after its prompt (#187):
//! `TokioProcessSupervisor::send_input` writes one line (its text, then
//! `\r`) or one key (its fixed bytes) to the terminal of a running PTY
//! child, in order with the typed prompt and with every other input, and
//! refuses every other case with a typed error and nothing written.
//!
//! Unix only, like every PTY launch test (`pty_launch.rs`). Assertions read
//! the `fake-agent --pty-ready-echo` child's own `typed: <line>` reports,
//! never the terminal's echo of what was typed (the line discipline echoes
//! input by default). The tests synchronize on the child's output lines
//! with generous deadlines, never on a sleep. Every line sent stays under
//! 1,024 bytes, the macOS canonical-buffer size.
#![cfg(unix)]

use std::future::Future;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant};

use omnifrons_app::{
    AgentPrompt, EnvPlan, ExecHandle, FramePayload, LaunchPlan, OutputFrame, ProcessId,
    ProcessOutput, ProcessSpec, ProcessSupervisor, StdinPlan, WorkspaceRoot,
};
use omnifrons_domain::adapter::TransportClass;
use omnifrons_domain::scope::ScopeMode;
use omnifrons_domain::terminal_input::{TerminalInput, TerminalKey, TypedLine};
use omnifrons_supervisor::{SendInputError, TokioProcessSupervisor};

/// Generous headroom for a fixture process on a loaded CI runner; the
/// behaviors under test never depend on how long it takes.
const OUTPUT_DEADLINE: Duration = Duration::from_secs(20);

/// The stop deadline: short, since every child here either exits on
/// `SIGTERM` or is meant to be escalated to `SIGKILL`.
const STOP_DEADLINE: Duration = Duration::from_millis(500);

fn temp_workspace(label: &str) -> (PathBuf, WorkspaceRoot) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "omnifrons-pty-input-test-{}-{label}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("failed to create the test workspace directory");
    let workspace = WorkspaceRoot::new(&dir).expect("the created dir must be a valid workspace");
    (dir, workspace)
}

fn fake_agent_path() -> PathBuf {
    std::fs::canonicalize(env!("CARGO_BIN_EXE_fake-agent"))
        .expect("the fake-agent binary must canonicalize")
}

fn fake_agent_handle() -> (ExecHandle, PathBuf) {
    let canonical = fake_agent_path();
    let file = std::fs::File::open(&canonical).expect("opening the fake-agent binary must succeed");
    (ExecHandle::File(file), canonical)
}

/// A `pty-cli`-shaped plan, exactly as `pty_launch.rs` builds it.
fn pty_plan(workspace: WorkspaceRoot, argv: &[&str], prompt: Option<&str>) -> LaunchPlan {
    LaunchPlan {
        argv: argv.iter().map(|arg| (*arg).to_string()).collect(),
        env: EnvPlan::new(&[]).expect("an empty declared-key list can never be secret-shaped"),
        cwd: workspace,
        stdin: StdinPlan::Null,
        prompt: prompt.map(|text| AgentPrompt::new(text).expect("valid prompt")),
        scope_mode: ScopeMode::Advisory,
        transport: TransportClass::Pty,
        output_dir: None,
    }
}

/// A running `--pty-ready-echo` child that already printed `ready`, its
/// frames, and its workspace directory.
fn ready_echo_child(
    supervisor: &mut TokioProcessSupervisor,
    label: &str,
) -> (ProcessId, Receiver<OutputFrame>, PathBuf) {
    let (dir, workspace) = temp_workspace(label);
    let (handle, canonical) = fake_agent_handle();
    let plan = pty_plan(workspace, &["--pty-ready-echo"], None);
    let id = supervisor
        .spawn_approved(handle, canonical, &plan)
        .expect("spawn_approved must succeed on the PTY path");
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    receive_until(&frames, |text| text == "ready");
    (id, frames, dir)
}

/// Await `future` on a throwaway runtime of the test's own: the delivery
/// future must not depend on the caller's executor.
fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a current-thread runtime must build")
        .block_on(future)
}

fn line(text: &str) -> TerminalInput {
    TerminalInput::Line(TypedLine::new(text).expect("a typeable line"))
}

/// Block on `frames` until a text frame satisfies `predicate`, or panic
/// once [`OUTPUT_DEADLINE`] passes. Returns every frame received up to and
/// including the matching one.
fn receive_until(frames: &Receiver<OutputFrame>, predicate: impl Fn(&str) -> bool) -> Vec<String> {
    let deadline = Instant::now() + OUTPUT_DEADLINE;
    let mut texts = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let frame = frames.recv_timeout(remaining).unwrap_or_else(|error| {
            panic!("no matching frame before the deadline ({error}); got {texts:#?}")
        });
        if let FramePayload::Text { text, .. } = frame.payload {
            let matched = predicate(&text);
            texts.push(text);
            if matched {
                return texts;
            }
        }
    }
}

/// The child's own reports among `texts`, prefix stripped, in order.
fn typed_lines(texts: &[String]) -> Vec<String> {
    texts
        .iter()
        .filter_map(|text| text.strip_prefix("typed: ").map(str::to_string))
        .collect()
}

fn stop(supervisor: &mut TokioProcessSupervisor, id: ProcessId, dir: &PathBuf) {
    let _ = supervisor.stop(id, STOP_DEADLINE);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_forwarded_line_reaches_the_child() {
    let _ptys = shared_ptys();
    let mut supervisor = TokioProcessSupervisor::new();
    let (id, frames, dir) = ready_echo_child(&mut supervisor, "line");

    assert_eq!(block_on(supervisor.send_input(id, &line("hello"))), Ok(()));
    let texts = receive_until(&frames, |text| text.starts_with("typed: "));
    assert_eq!(typed_lines(&texts), ["hello"]);

    stop(&mut supervisor, id, &dir);
}

/// Each key reaches the child as its fixed bytes: Escape, then each arrow,
/// each followed by Enter, which ends the line the child reports.
#[test]
fn each_key_reaches_the_child() {
    let _ptys = shared_ptys();
    let mut supervisor = TokioProcessSupervisor::new();
    let (id, frames, dir) = ready_echo_child(&mut supervisor, "keys");

    for (key, expected) in [
        (TerminalKey::Escape, r"\x1b"),
        (TerminalKey::Up, r"\x1b[A"),
        (TerminalKey::Down, r"\x1b[B"),
        (TerminalKey::Right, r"\x1b[C"),
        (TerminalKey::Left, r"\x1b[D"),
    ] {
        for input in [
            TerminalInput::Key(key),
            TerminalInput::Key(TerminalKey::Enter),
        ] {
            assert_eq!(
                block_on(supervisor.send_input(id, &input)),
                Ok(()),
                "{input:?}"
            );
        }
        let texts = receive_until(&frames, |text| text.starts_with("typed: "));
        assert_eq!(typed_lines(&texts), [expected], "{key:?} then Enter");
    }

    stop(&mut supervisor, id, &dir);
}

/// A line sent right after spawn, while a 3,000-byte typed prompt is still
/// being written, waits for the prompt's turn to end: the child reads every
/// prompt line intact, then the forwarded line. The prompt is split into
/// lines of under 1,024 bytes so no kernel truncates one.
#[test]
fn a_line_sent_during_a_long_typed_prompt_arrives_after_it_both_intact() {
    let _ptys = shared_ptys();
    let prompt_lines: Vec<String> = (0..30)
        .map(|n| format!("prompt line {n:02} {}", "p".repeat(85)))
        .collect();
    let prompt = prompt_lines.join("\n");
    assert!(prompt.len() >= 3000, "the prompt is {} bytes", prompt.len());

    let (dir, workspace) = temp_workspace("ordering");
    let (handle, canonical) = fake_agent_handle();
    let plan = pty_plan(workspace, &["--pty-ready-echo"], Some(&prompt));
    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(handle, canonical, &plan)
        .expect("spawn_approved must succeed on the PTY path");
    let delivery = supervisor.send_input(id, &line("after the prompt"));
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");

    assert_eq!(block_on(delivery), Ok(()));
    let texts = receive_until(&frames, |text| text == "typed: after the prompt");
    let mut expected = prompt_lines;
    expected.push("after the prompt".to_string());
    assert_eq!(typed_lines(&texts), expected);

    stop(&mut supervisor, id, &dir);
}

/// Two sends started at the same moment from two threads each arrive as
/// one intact line.
#[test]
fn two_concurrent_sends_each_arrive_intact() {
    let _ptys = shared_ptys();
    let mut supervisor = TokioProcessSupervisor::new();
    let (id, frames, dir) = ready_echo_child(&mut supervisor, "concurrent");
    let first = "a".repeat(600);
    let second = "b".repeat(600);

    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        for text in [&first, &second] {
            let (supervisor, barrier) = (&supervisor, &barrier);
            scope.spawn(move || {
                barrier.wait();
                assert_eq!(block_on(supervisor.send_input(id, &line(text))), Ok(()));
            });
        }
    });

    let texts = receive_until(&frames, {
        let count = std::cell::Cell::new(0);
        move |text| {
            if text.starts_with("typed: ") {
                count.set(count.get() + 1);
            }
            count.get() == 2
        }
    });
    let mut typed = typed_lines(&texts);
    typed.sort();
    assert_eq!(typed, [first, second]);

    stop(&mut supervisor, id, &dir);
}

#[test]
fn an_unknown_id_is_refused_as_unknown_process() {
    let supervisor = TokioProcessSupervisor::new();
    assert_eq!(
        block_on(supervisor.send_input(ProcessId(u32::MAX), &line("hello"))),
        Err(SendInputError::UnknownProcess)
    );
}

/// A child spawned with pipes has no terminal to type into.
#[test]
fn a_pipes_child_is_unavailable() {
    let mut supervisor = TokioProcessSupervisor::new();
    let spec = ProcessSpec::new(fake_agent_path().to_string_lossy().into_owned())
        .with_args(["--pty-ignore-sigterm"]);
    let id = supervisor
        .spawn(spec)
        .expect("spawning the fixture must succeed");
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    receive_until(&frames, |text| text == "ready");

    assert_eq!(
        block_on(supervisor.send_input(id, &line("hello"))),
        Err(SendInputError::Unavailable)
    );

    let _ = supervisor.stop(id, STOP_DEADLINE);
}

/// Once a PTY child is stopped its run is terminal: the input is refused
/// and the child never reports a line.
#[test]
fn a_stopped_pty_child_is_unavailable_and_nothing_is_written() {
    let _ptys = shared_ptys();
    let mut supervisor = TokioProcessSupervisor::new();
    let (id, frames, dir) = ready_echo_child(&mut supervisor, "stopped");
    supervisor
        .stop(id, STOP_DEADLINE)
        .expect("stopping a running child must succeed");

    assert_eq!(
        block_on(supervisor.send_input(id, &line("too late"))),
        Err(SendInputError::Unavailable)
    );
    let texts: Vec<String> = frames
        .iter()
        .filter_map(|frame| match frame.payload {
            FramePayload::Text { text, .. } => Some(text),
            FramePayload::State(_) => None,
        })
        .collect();
    assert!(typed_lines(&texts).is_empty(), "got {texts:#?}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Serializes the tests that count this process's open pty masters
/// (exclusive) against every other test here that opens one (shared), so a
/// count can only change through the run under test. Poisoning is ignored:
/// the lock guards no data.
static PTY_MASTERS: RwLock<()> = RwLock::new(());

fn shared_ptys() -> RwLockReadGuard<'static, ()> {
    PTY_MASTERS.read().unwrap_or_else(PoisonError::into_inner)
}

fn exclusive_ptys() -> RwLockWriteGuard<'static, ()> {
    PTY_MASTERS.write().unwrap_or_else(PoisonError::into_inner)
}

/// How many pty masters this process holds open, where `/proc` lists its
/// descriptors (Linux); `None` elsewhere (no `/proc` on macOS).
fn open_pty_masters() -> Option<usize> {
    let entries = std::fs::read_dir("/proc/self/fd").ok()?;
    let count = entries
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .filter(|target| target.to_string_lossy().contains("ptmx"))
        .count();
    Some(count)
}

/// The send's own bound (`pty::INPUT_WRITE_WAIT`, 2 s) plus slack for a
/// loaded runner: an input waiting for room must be resolved by then.
const IN_FLIGHT_BOUND: Duration = Duration::from_secs(4);

/// The outer bound on a whole hang-up scenario, so a supervisor whose
/// runtime is stuck fails the test instead of hanging the test binary.
const SCENARIO_DEADLINE: Duration = Duration::from_secs(40);

/// What a hang-up scenario observed, reported before its supervisor is
/// dropped (a stuck runtime would make that drop hang).
#[derive(Debug)]
struct HangUpOutcome {
    /// The pending input's result, `None` if unresolved within
    /// [`IN_FLIGHT_BOUND`] of its start; always `None` without one.
    in_flight: Option<Result<(), SendInputError>>,
    /// Whether the hung-up run's final `State` frame arrived.
    state_frame: bool,
    /// Whether the open pty master count fell back to its value before the
    /// run (`None` where it cannot be read).
    master_closed: Option<bool>,
    /// Whether a run spawned afterwards on the same supervisor produced
    /// its output.
    second_run_output: bool,
}

/// How the terminal of the first run hangs up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HangUp {
    /// The test stops the child.
    Stop,
    /// The child exits on its own.
    SelfExit,
}

/// How long, after its report, a scenario's own clean-up (stopping the
/// later run, dropping its supervisor) is waited for, so its masters are
/// closed before the next exclusive test counts them. Never fatal: a stuck
/// supervisor's drop never returns, and its report already failed the test.
const CLEAN_UP_WAIT: Duration = Duration::from_secs(10);

/// Run `scenario` on its own thread and return what it reports, failing
/// once [`SCENARIO_DEADLINE`] passes; then wait up to [`CLEAN_UP_WAIT`]
/// for the thread to end.
fn bounded(scenario: impl FnOnce(&mpsc::Sender<HangUpOutcome>) + Send + 'static) -> HangUpOutcome {
    let (report, outcome) = mpsc::channel();
    let thread = std::thread::spawn(move || scenario(&report));
    let outcome = outcome
        .recv_timeout(SCENARIO_DEADLINE)
        .expect("the scenario must report within its bound (a panic in it is printed above)");
    let deadline = Instant::now() + CLEAN_UP_WAIT;
    while !thread.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    outcome
}

/// Await `future` on a test-owned runtime with a timer, independent of the
/// supervisor's runtime, giving up after `bound`.
fn within<F: Future>(bound: Duration, future: F) -> Option<F::Output> {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a current-thread runtime must build")
        .block_on(async { tokio::time::timeout(bound, future).await.ok() })
}

/// Whether a `State` frame arrives on `frames` within `bound`.
fn state_frame_within(frames: &Receiver<OutputFrame>, bound: Duration) -> bool {
    let deadline = Instant::now() + bound;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match frames.recv_timeout(remaining) {
            Ok(frame) if matches!(frame.payload, FramePayload::State(_)) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
}

/// Whether the open pty master count returns to `baseline` within `bound`
/// (`None` where it cannot be read).
fn masters_back_to(baseline: Option<usize>, bound: Duration) -> Option<bool> {
    let baseline = baseline?;
    let deadline = Instant::now() + bound;
    loop {
        if open_pty_masters() == Some(baseline) {
            return Some(true);
        }
        if Instant::now() >= deadline {
            return Some(false);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Spawn a `--pty-ready-echo` run on `supervisor` and report whether its
/// `ready` line arrives within `bound`, with its id and workspace so the
/// caller can stop it once the outcome is reported.
fn second_run(
    supervisor: &mut TokioProcessSupervisor,
    bound: Duration,
) -> (bool, ProcessId, PathBuf) {
    let (dir, workspace) = temp_workspace("second");
    let (handle, canonical) = fake_agent_handle();
    let plan = pty_plan(workspace, &["--pty-ready-echo"], None);
    let id = supervisor
        .spawn_approved(handle, canonical, &plan)
        .expect("spawn_approved must succeed on the PTY path");
    let frames = supervisor.subscribe(id).expect("subscribe must succeed");
    let deadline = Instant::now() + bound;
    let mut ready = false;
    while let Ok(frame) = frames.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        if matches!(&frame.payload, FramePayload::Text { text, .. } if text == "ready") {
            ready = true;
            break;
        }
    }
    (ready, id, dir)
}

/// A child that never reads fills its terminal's input queue; one more
/// input is then left waiting for room while the terminal hangs up
/// (`hang_up`). Then: does that input resolve, does the run's final frame
/// arrive and its master close, and does a later run still get its output?
fn in_flight_hang_up(hang_up: HangUp) -> HangUpOutcome {
    bounded(move |report| {
        let baseline = open_pty_masters();
        let (dir, workspace) = temp_workspace("hang-up");
        let marker = dir.join("exit-now");
        let marker_arg = marker.to_string_lossy().into_owned();
        let (handle, canonical) = fake_agent_handle();
        let plan = pty_plan(workspace, &["--pty-hold-until", &marker_arg], None);
        let mut supervisor = TokioProcessSupervisor::new();
        let id = supervisor
            .spawn_approved(handle, canonical, &plan)
            .expect("spawn_approved must succeed on the PTY path");
        let frames = supervisor.subscribe(id).expect("subscribe must succeed");
        receive_until(&frames, |text| text == "ready");

        // Lines of 500 bytes stay under every kernel's canonical limit, so
        // the queue fills instead of a line being discarded; the first send
        // that finds no room times out.
        let queue_line = line(&"q".repeat(500));
        let queue_full =
            (0..400).any(|_| block_on(supervisor.send_input(id, &queue_line)).is_err());
        assert!(queue_full, "the terminal input queue never filled");

        let pending = supervisor.send_input(id, &line("in flight"));
        let started = Instant::now();
        std::thread::sleep(Duration::from_millis(200));
        match hang_up {
            HangUp::Stop => {
                let _ = supervisor.stop(id, STOP_DEADLINE);
            }
            HangUp::SelfExit => std::fs::write(&marker, b"").expect("the marker must be written"),
        }
        let in_flight = within(IN_FLIGHT_BOUND.saturating_sub(started.elapsed()), pending);
        let state_frame = state_frame_within(&frames, Duration::from_secs(5));
        let master_closed = masters_back_to(baseline, Duration::from_secs(3));
        let (second_run_output, second, second_dir) =
            second_run(&mut supervisor, Duration::from_secs(5));
        let _ = report.send(HangUpOutcome {
            in_flight,
            state_frame,
            master_closed,
            second_run_output,
        });
        let _ = supervisor.stop(second, STOP_DEADLINE);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&second_dir);
    })
}

/// A typed prompt larger than the terminal's input queue, to a child that
/// never reads, then `stop` while the prompt is still waiting for room.
fn prompt_hang_up() -> HangUpOutcome {
    bounded(|report| {
        let baseline = open_pty_masters();
        let prompt = (0..160)
            .map(|n| format!("{n:03}{}", "z".repeat(98)))
            .collect::<Vec<_>>()
            .join("\n");
        let (dir, workspace) = temp_workspace("prompt-hang-up");
        let never = dir.join("never-created").to_string_lossy().into_owned();
        let (handle, canonical) = fake_agent_handle();
        let plan = pty_plan(workspace, &["--pty-hold-until", &never], Some(&prompt));
        let mut supervisor = TokioProcessSupervisor::new();
        let id = supervisor
            .spawn_approved(handle, canonical, &plan)
            .expect("spawn_approved must succeed on the PTY path");
        let frames = supervisor.subscribe(id).expect("subscribe must succeed");
        // The prompt's echo, typed before the child turns echo off, may end
        // mid-line just ahead of `ready`.
        receive_until(&frames, |text| text.ends_with("ready"));
        std::thread::sleep(Duration::from_millis(300));
        let _ = supervisor.stop(id, STOP_DEADLINE);

        let state_frame = state_frame_within(&frames, Duration::from_secs(5));
        let master_closed = masters_back_to(baseline, Duration::from_secs(3));
        let (second_run_output, second, second_dir) =
            second_run(&mut supervisor, Duration::from_secs(5));
        let _ = report.send(HangUpOutcome {
            in_flight: None,
            state_frame,
            master_closed,
            second_run_output,
        });
        let _ = supervisor.stop(second, STOP_DEADLINE);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&second_dir);
    })
}

/// The three observations every hang-up must satisfy: the run ends with
/// its final frame, its master closes, and the supervisor keeps serving.
fn assert_hang_up_released_everything(outcome: &HangUpOutcome) {
    assert!(
        outcome.state_frame,
        "the hung-up run's state frame: {outcome:#?}"
    );
    assert_ne!(
        outcome.master_closed,
        Some(false),
        "the hung-up run's master must close: {outcome:#?}"
    );
    assert!(
        outcome.second_run_output,
        "a later run must still get its output: {outcome:#?}"
    );
}

#[test]
fn an_input_waiting_for_room_fails_within_its_bound_when_the_child_is_stopped() {
    let _ptys = exclusive_ptys();
    let outcome = in_flight_hang_up(HangUp::Stop);
    assert_eq!(
        outcome.in_flight,
        Some(Err(SendInputError::WriteFailed)),
        "{outcome:#?}"
    );
    assert_hang_up_released_everything(&outcome);
}

#[test]
fn an_input_waiting_for_room_fails_within_its_bound_when_the_child_exits_by_itself() {
    let _ptys = exclusive_ptys();
    let outcome = in_flight_hang_up(HangUp::SelfExit);
    assert_eq!(
        outcome.in_flight,
        Some(Err(SendInputError::WriteFailed)),
        "{outcome:#?}"
    );
    assert_hang_up_released_everything(&outcome);
}

#[test]
fn a_typed_prompt_waiting_for_room_never_stalls_the_supervisor_after_stop() {
    let _ptys = exclusive_ptys();
    assert_hang_up_released_everything(&prompt_hang_up());
}
