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
use std::sync::mpsc::Receiver;
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
