//! The pseudo-terminal launch path (spike slice 4, `docs/spike-log.md` §
//! Slice 4): `TokioProcessSupervisor::spawn_approved` given a
//! `LaunchPlan` whose `transport` is `TransportClass::Pty` runs the
//! approved executable as the session leader of a controlling terminal it
//! opened, types the prompt into that terminal, and captures the master
//! side as raw `stdout` frames.
//!
//! Every launch test here is `#[cfg(unix)]`: the PTY path exists only on
//! unix in this slice (`openpty`, `setsid`, `TIOCSCTTY` -- D2 of the
//! brief; Windows `ConPTY` is recorded as debt next to the Job Object debt),
//! so on Windows the one test that runs is the typed-error one at the
//! bottom, asserting the path refuses before it touches anything. The
//! tests synchronize on the child's own output lines (`ready`,
//! `corpus-done`, the report lines) with generous deadlines -- never on a
//! sleep -- and the output-volume test bursts, it does not rate-limit.
//!
//! The planted-secret env test re-execs this very test binary with the
//! key set through `Command::env` on that child, exactly like slice 3's
//! `adapter_launch.rs`: `std::env::set_var` is `unsafe` and races every
//! concurrent environment reader in this process.

use std::path::PathBuf;
// Used only by the unix launch tests; the Windows typed-error test needs
// no clock, no output port, and no `ProcessSupervisor` trait method, and an
// unconditional import would be an unused-import error under `-D warnings`
// on that platform (the same per-item gating `child_termination.rs` uses).
#[cfg(unix)]
use std::time::{Duration, Instant};

use omnifrons_app::{AgentPrompt, EnvPlan, ExecHandle, LaunchPlan, StdinPlan, WorkspaceRoot};
#[cfg(unix)]
use omnifrons_app::{
    FramePayload, ProcessOutput, ProcessStatus, ProcessSupervisor, ProcessTerminalState,
};
use omnifrons_domain::adapter::TransportClass;
use omnifrons_domain::scope::ScopeMode;
use omnifrons_supervisor::TokioProcessSupervisor;

/// Generous headroom for a short-lived fixture process on a loaded CI
/// runner; the behaviors under test never depend on how long it takes.
#[cfg(unix)]
const REAP_DEADLINE: Duration = Duration::from_secs(20);

/// The stop deadline for the SIGTERM-ignoring child: short, so the test
/// proves escalation to SIGKILL rather than waiting on a graceful exit
/// that never comes.
#[cfg(unix)]
const STOP_DEADLINE: Duration = Duration::from_millis(500);

fn temp_workspace(label: &str) -> (PathBuf, WorkspaceRoot) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "omnifrons-pty-launch-test-{}-{label}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("failed to create the test workspace directory");
    let workspace = WorkspaceRoot::new(&dir).expect("the created dir must be a valid workspace");
    (dir, workspace)
}

fn fake_agent_handle() -> (ExecHandle, PathBuf) {
    let canonical = std::fs::canonicalize(env!("CARGO_BIN_EXE_fake-agent"))
        .expect("the fake-agent binary must canonicalize");
    let file = std::fs::File::open(&canonical).expect("opening the fake-agent binary must succeed");
    (ExecHandle::File(file), canonical)
}

/// A `pty-cli`-shaped plan: `TransportClass::Pty`, `StdinPlan::Null`
/// (unused on this path), the base allowlist environment, the workspace
/// as cwd, and `prompt` typed into the terminal when `Some`.
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

#[cfg(unix)]
fn wait_for_terminal(
    supervisor: &TokioProcessSupervisor,
    id: omnifrons_app::ProcessId,
    deadline: Instant,
) -> ProcessTerminalState {
    loop {
        match supervisor.observe(id) {
            Some(ProcessStatus::Terminal(state)) => return state,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            other => panic!("process did not reach a terminal state in time: {other:?}"),
        }
    }
}

/// Every text frame's `(text, continued)`, asserting along the way that
/// no frame is labelled `stderr`: a PTY has no stderr, so the one master
/// reader is always `stdout`.
#[cfg(unix)]
fn stdout_texts(frames: &[omnifrons_app::OutputFrame]) -> Vec<(String, bool)> {
    frames
        .iter()
        .filter_map(|frame| match &frame.payload {
            FramePayload::Text {
                stream: omnifrons_app::OutputStream::Stdout,
                text,
                continued,
            } => Some((text.clone(), *continued)),
            FramePayload::Text {
                stream: omnifrons_app::OutputStream::Stderr,
                text,
                ..
            } => panic!("a PTY launch must never produce a stderr frame, got {text:?}"),
            FramePayload::State(_) => None,
        })
        .collect()
}

/// Block on `receiver` until a frame's text satisfies `predicate`, or
/// panic once `deadline` passes. Returns every frame received up to and
/// including the matching one.
#[cfg(unix)]
fn receive_until(
    receiver: &std::sync::mpsc::Receiver<omnifrons_app::OutputFrame>,
    deadline: Instant,
    predicate: impl Fn(&str) -> bool,
) -> Vec<omnifrons_app::OutputFrame> {
    let mut frames = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let frame = receiver.recv_timeout(remaining).unwrap_or_else(|error| {
            panic!("no matching frame before the deadline ({error}); got {frames:#?}")
        });
        let matched = matches!(&frame.payload, FramePayload::Text { text, .. } if predicate(text));
        frames.push(frame);
        if matched {
            return frames;
        }
    }
}

/// The child sees a terminal on stdin, stdout, and stderr, `TERM=dumb`,
/// `COLUMNS=80`, and `LINES=24` -- set by the supervisor, never read from
/// its own environment -- and exits 0 through a confirmed reap.
#[cfg(unix)]
#[test]
fn pty_child_sees_a_tty_on_all_three_fds_and_term_dumb() {
    let (dir, workspace) = temp_workspace("report");
    let (handle, canonical) = fake_agent_handle();
    let plan = pty_plan(workspace, &["--pty-report"], None);

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(handle, canonical, &plan)
        .expect("spawn_approved must succeed on the PTY path");
    let rx = supervisor.subscribe(id).expect("subscribe must succeed");

    let terminal = wait_for_terminal(&supervisor, id, Instant::now() + REAP_DEADLINE);
    assert_eq!(terminal, ProcessTerminalState::Exited { code: Some(0) });
    let frames: Vec<_> = rx.iter().collect();
    let texts: Vec<String> = stdout_texts(&frames)
        .into_iter()
        .map(|(text, _)| text)
        .collect();

    for expected in [
        "isatty stdin=true stdout=true stderr=true",
        "TERM=dumb",
        "COLUMNS=80",
        "LINES=24",
    ] {
        assert!(
            texts.iter().any(|line| line == expected),
            "expected the line {expected:?}, got {texts:#?}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// The prompt is typed into the terminal followed by a carriage return:
/// the child reads exactly that line and echoes it back with its own
/// prefix.
#[cfg(unix)]
#[test]
fn typed_prompt_is_read_by_the_child_as_one_line() {
    let (dir, workspace) = temp_workspace("echo");
    let (handle, canonical) = fake_agent_handle();
    let plan = pty_plan(workspace, &["--pty-echo"], Some("hello pty"));

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(handle, canonical, &plan)
        .expect("spawn_approved must succeed on the PTY path");
    let rx = supervisor.subscribe(id).expect("subscribe must succeed");

    let terminal = wait_for_terminal(&supervisor, id, Instant::now() + REAP_DEADLINE);
    assert_eq!(terminal, ProcessTerminalState::Exited { code: Some(0) });
    let frames: Vec<_> = rx.iter().collect();
    let texts: Vec<String> = stdout_texts(&frames)
        .into_iter()
        .map(|(text, _)| text)
        .collect();
    assert!(
        texts.iter().any(|line| line == "typed: hello pty"),
        "the child must read the typed prompt as one line, got {texts:#?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Set on the re-exec'd inner run of
/// [`observed_env_is_exactly_the_base_allowlist_plus_term_columns_lines`]
/// so it performs the real assertions instead of re-exec'ing again.
#[cfg(unix)]
const INNER_RUN_MARKER: &str = "OMNIFRONS_TEST_INNER";

/// The secret-shaped key planted in the inner run's environment. Its
/// value is a dummy literal, never a credential.
#[cfg(unix)]
const PLANTED_SECRET_SHAPED_KEY: &str = "FAKE_SECRET_TOKEN";

#[cfg(unix)]
const ENV_TEST_NAME: &str = "observed_env_is_exactly_the_base_allowlist_plus_term_columns_lines";

#[cfg(unix)]
fn rerun_env_test_with_a_planted_key() {
    let test_binary = std::env::current_exe().expect("the test binary's own path must resolve");
    let output = std::process::Command::new(test_binary)
        .args(["--exact", ENV_TEST_NAME, "--nocapture"])
        .env(INNER_RUN_MARKER, "1")
        .env(PLANTED_SECRET_SHAPED_KEY, "planted")
        .output()
        .expect("re-exec'ing the test binary must start");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the inner run failed with status {:?}\n--- inner stdout ---\n{stdout}\n--- inner stderr \
         ---\n{stderr}",
        output.status
    );
    assert!(
        stdout.contains("1 passed"),
        "the inner run must have executed exactly one test, got:\n{stdout}"
    );
}

/// The PTY child's environment is the base allowlist (resolved from the
/// supervisor's own environment, so only keys actually set there appear)
/// plus exactly `TERM`, `COLUMNS`, and `LINES`; a planted secret-shaped
/// key never reaches it. Only key names are ever printed, never values.
#[cfg(unix)]
#[test]
fn observed_env_is_exactly_the_base_allowlist_plus_term_columns_lines() {
    if std::env::var_os(INNER_RUN_MARKER).is_none() {
        rerun_env_test_with_a_planted_key();
        return;
    }
    assert!(
        std::env::var_os(PLANTED_SECRET_SHAPED_KEY).is_some(),
        "the inner run must see the planted key in its own environment, or the test proves \
         nothing"
    );

    let (dir, workspace) = temp_workspace("env");
    let (handle, canonical) = fake_agent_handle();
    let plan = pty_plan(workspace, &["--pty-report"], None);

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(handle, canonical, &plan)
        .expect("spawn_approved must succeed on the PTY path");
    let rx = supervisor.subscribe(id).expect("subscribe must succeed");
    wait_for_terminal(&supervisor, id, Instant::now() + REAP_DEADLINE);
    let frames: Vec<_> = rx.iter().collect();
    let texts: Vec<String> = stdout_texts(&frames)
        .into_iter()
        .map(|(text, _)| text)
        .collect();

    let keys_line = texts
        .iter()
        .find(|line| line.starts_with("env-keys"))
        .expect("the report must carry an env-keys line");
    let observed: std::collections::BTreeSet<&str> = keys_line.split_whitespace().skip(1).collect();

    assert!(
        !observed.contains(PLANTED_SECRET_SHAPED_KEY),
        "a planted secret-shaped env var must never reach the child, observed keys: {observed:?}"
    );
    let allowed: std::collections::BTreeSet<&str> = [
        "PATH", "LANG", "LC_ALL", "TMPDIR", "HOME", "TERM", "COLUMNS", "LINES",
    ]
    .into_iter()
    .collect();
    for key in &observed {
        assert!(
            allowed.contains(key),
            "observed env var {key} is outside the base allowlist plus TERM/COLUMNS/LINES"
        );
    }
    for key in ["TERM", "COLUMNS", "LINES"] {
        assert!(
            observed.contains(key),
            "{key} must always be set on a PTY child, observed keys: {observed:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// The shared corpus (`omnifrons_app::terminal_normalizer::corpus`),
/// written verbatim by `fake-agent --pty-corpus`, comes back through the
/// master as raw line frames that re-join to exactly the corpus bytes
/// (lossily decoded): the terminal's own output post-processing (`\n` to
/// `\r\n`) is undone by the supervisor's CRLF framing, nothing else is
/// altered, and no line is split (every corpus line is under the 8 KiB
/// frame cap).
#[cfg(unix)]
#[test]
fn corpus_round_trips_through_the_supervisor_as_raw_frames() {
    let (dir, workspace) = temp_workspace("corpus");
    let (handle, canonical) = fake_agent_handle();
    let plan = pty_plan(workspace, &["--pty-corpus"], None);

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(handle, canonical, &plan)
        .expect("spawn_approved must succeed on the PTY path");
    let rx = supervisor.subscribe(id).expect("subscribe must succeed");

    let frames = receive_until(&rx, Instant::now() + REAP_DEADLINE, |text| {
        text == "corpus-done"
    });
    let texts = stdout_texts(&frames);
    assert!(
        texts.iter().all(|(_, continued)| !continued),
        "no corpus line reaches the per-frame cap, so no frame is continued"
    );
    let before_done: Vec<&str> = texts
        .iter()
        .map(|(text, _)| text.as_str())
        .take_while(|text| *text != "corpus-done")
        .collect();
    let rejoined = before_done.join("\n");

    let expected =
        String::from_utf8_lossy(&omnifrons_app::terminal_normalizer::corpus::bytes()).into_owned();
    assert_eq!(
        rejoined, expected,
        "the raw frames must re-join to the corpus"
    );

    wait_for_terminal(&supervisor, id, Instant::now() + REAP_DEADLINE);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The PTY child is its own session and process-group leader (`setsid`
/// in `pre_exec`, no `process_group(0)`), so the unchanged group-signal
/// stop path still reaches it: a child that ignores SIGTERM is escalated
/// to SIGKILL and reported `Killed`, and its pid is gone afterward.
#[cfg(unix)]
#[test]
fn pty_child_is_a_session_leader_and_a_sigterm_ignoring_child_ends_killed() {
    let (dir, workspace) = temp_workspace("sigterm");
    let (handle, canonical) = fake_agent_handle();
    let plan = pty_plan(workspace, &["--pty-ignore-sigterm"], None);

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(handle, canonical, &plan)
        .expect("spawn_approved must succeed on the PTY path");
    let rx = supervisor.subscribe(id).expect("subscribe must succeed");

    // Observable synchronization: the child prints `ready` only once its
    // SIGTERM-ignore disposition is installed.
    let _ = receive_until(&rx, Instant::now() + REAP_DEADLINE, |text| text == "ready");

    let pid = nix::unistd::Pid::from_raw(i32::try_from(id.0).expect("pid fits in i32"));
    assert_eq!(
        nix::unistd::getsid(Some(pid)).expect("getsid must succeed"),
        pid,
        "the PTY child must be its own session leader"
    );
    assert_eq!(
        nix::unistd::getpgid(Some(pid)).expect("getpgid must succeed"),
        pid,
        "the PTY child must be its own process-group leader"
    );

    let terminal = supervisor
        .stop(id, STOP_DEADLINE)
        .expect("stop must succeed");
    assert_eq!(terminal, ProcessTerminalState::Killed);
    let error = nix::sys::signal::kill(pid, None).expect_err("pid must be gone after stop");
    assert_eq!(error, nix::errno::Errno::ESRCH);

    // The final state frame still arrives, and the master is closed once
    // the reader observed EOF: the receiver's iterator ends.
    let frames: Vec<_> = rx.iter().collect();
    assert!(
        frames.iter().any(|frame| matches!(
            frame.payload,
            FramePayload::State(ProcessTerminalState::Killed)
        )),
        "the terminal state frame must be delivered, got {frames:#?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Linux only: `ExecHandle::SealedMemory` -- the `/proc/self/fd/<n>` exec
/// of a sealed memfd -- keeps working on the PTY path, whose `pre_exec`
/// forces `std::process::Command` onto its fork-then-exec path: the fd
/// survives into the child and the sealed content runs inside the
/// terminal.
#[cfg(target_os = "linux")]
#[test]
fn sealed_memfd_exec_keeps_working_on_the_pty_path() {
    use std::io::Write as _;

    use nix::fcntl::{FcntlArg, SealFlag, fcntl};
    use nix::sys::memfd::{MFdFlags, memfd_create};

    let content = std::fs::read(env!("CARGO_BIN_EXE_fake-agent"))
        .expect("the fake-agent binary must be readable");
    let memfd = memfd_create(
        "omnifrons-pty-launch-test-fixture",
        MFdFlags::MFD_CLOEXEC | MFdFlags::MFD_ALLOW_SEALING,
    )
    .expect("memfd_create must succeed for the test fixture");
    let mut sealed = std::fs::File::from(memfd);
    sealed
        .write_all(&content)
        .expect("writing the fixture content into the memfd must succeed");
    fcntl(
        &sealed,
        FcntlArg::F_ADD_SEALS(
            SealFlag::F_SEAL_SHRINK
                | SealFlag::F_SEAL_GROW
                | SealFlag::F_SEAL_WRITE
                | SealFlag::F_SEAL_SEAL,
        ),
    )
    .expect("sealing the fixture memfd must succeed");

    let (dir, workspace) = temp_workspace("memfd");
    let plan = pty_plan(workspace, &["--pty-report"], None);
    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(
            ExecHandle::SealedMemory(sealed),
            PathBuf::from("/approved/pty-memfd-fixture"),
            &plan,
        )
        .expect("spawn_approved must succeed against a sealed memfd on the PTY path");
    let rx = supervisor.subscribe(id).expect("subscribe must succeed");

    let terminal = wait_for_terminal(&supervisor, id, Instant::now() + REAP_DEADLINE);
    assert_eq!(terminal, ProcessTerminalState::Exited { code: Some(0) });
    let frames: Vec<_> = rx.iter().collect();
    let texts: Vec<String> = stdout_texts(&frames)
        .into_iter()
        .map(|(text, _)| text)
        .collect();
    assert!(
        texts
            .iter()
            .any(|line| line == "isatty stdin=true stdout=true stderr=true"),
        "the sealed content must run inside the terminal, got {texts:#?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Windows: the PTY path is not implemented in this slice (`ConPTY` is
/// recorded as debt next to the Job Object debt). `spawn_approved` with a
/// `Pty` plan returns the typed error before touching the handle, the
/// running cap, or any bookkeeping -- nothing is spawned.
#[cfg(windows)]
#[test]
fn pty_transport_returns_the_typed_unsupported_error_before_spawning() {
    let (dir, workspace) = temp_workspace("windows");
    let (handle, canonical) = fake_agent_handle();
    let plan = pty_plan(workspace, &["--pty-report"], None);

    let mut supervisor = TokioProcessSupervisor::new();
    let result = supervisor.spawn_approved(handle, canonical, &plan);
    assert_eq!(
        result.expect_err("a Pty plan must be refused on Windows"),
        omnifrons_app::SupervisorError::PtyUnsupported
    );

    let _ = std::fs::remove_dir_all(&dir);
}
