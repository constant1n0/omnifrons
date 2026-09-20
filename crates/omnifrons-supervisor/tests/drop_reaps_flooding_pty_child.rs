//! Regression test for `impl Drop for Inner` (`src/lib.rs`): dropping a
//! supervisor with a still-flooding PTY child must leave that child
//! reaped, not merely killed.
//!
//! Its own test binary, deliberately, with exactly one `#[test]`. Tokio
//! keeps a process-wide orphan queue, and any *other* live Tokio runtime
//! in the same process could reap this test's zombie by accident and mask
//! the defect this pins. With one test and one supervisor in the process,
//! the only thing that can reap the child is `Drop` itself.
//!
//! The child is `fake-agent --flood 4000000000` (`src/bin/fake-agent.rs`),
//! spawned on the PTY path (`spawn_approved`, `TransportClass::Pty`, no
//! prompt): it writes lines as fast as it can with no inter-line sleeping,
//! so with a count this large it is still writing when the supervisor is
//! dropped. The output is never subscribed to on purpose: an undrained
//! subscriber channel is fine, since text frames are simply dropped once
//! it is full, by design (`output_capture`), and the supervisor's reader
//! task keeps draining the master all the same. That reader is the point.
//! The old `Drop` order stopped it first, while the child went on writing,
//! so by the time of the kill the pty held output nobody would ever read.
//!
//! This test CANNOT go red on Linux: Linux reaps a killed child in
//! milliseconds regardless of whether its output was ever drained, so it
//! passes under either drop order. It guards macOS, where a session
//! leader with unread pty output is not reapable, not even after SIGKILL,
//! until its master is drained or closed (measured in
//! `tests/pty_exit_drain_probe.rs`): the old `Drop` order stopped the
//! reader tasks (and therefore all draining) before killing children, so
//! this exact scenario left a permanent zombie there. On Linux this test
//! is a regression guard only.

#![cfg(unix)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use omnifrons_app::{
    AgentPrompt, EnvPlan, ExecHandle, LaunchPlan, ProcessStatus, ProcessSupervisor, StdinPlan,
    WorkspaceRoot,
};
use omnifrons_domain::adapter::TransportClass;
use omnifrons_domain::scope::ScopeMode;
use omnifrons_supervisor::TokioProcessSupervisor;

/// How long to let the child actually be mid-flood -- with unread output
/// already sitting in the pty -- before dropping the supervisor, rather
/// than racing the drop against the child's very first write.
const FLOOD_WARMUP: Duration = Duration::from_millis(200);

/// Generous headroom for the post-drop reap check on a loaded CI runner;
/// the behavior under test never depends on how long it takes.
const REAP_DEADLINE: Duration = Duration::from_secs(5);

/// Copied from `tests/pty_launch.rs` (duplicating small helpers per test
/// file is this crate's existing convention, not an oversight).
fn temp_workspace(label: &str) -> (PathBuf, WorkspaceRoot) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "omnifrons-drop-reaps-flooding-pty-child-test-{}-{label}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("failed to create the test workspace directory");
    let workspace = WorkspaceRoot::new(&dir).expect("the created dir must be a valid workspace");
    (dir, workspace)
}

/// Copied from `tests/pty_launch.rs`.
fn fake_agent_handle() -> (ExecHandle, PathBuf) {
    let canonical = std::fs::canonicalize(env!("CARGO_BIN_EXE_fake-agent"))
        .expect("the fake-agent binary must canonicalize");
    let file = std::fs::File::open(&canonical).expect("opening the fake-agent binary must succeed");
    (ExecHandle::File(file), canonical)
}

/// Copied from `tests/pty_launch.rs`: a `pty-cli`-shaped plan.
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

#[test]
fn dropping_the_supervisor_reaps_a_flooding_pty_child() {
    let (dir, workspace) = temp_workspace("flood");
    let (handle, canonical) = fake_agent_handle();
    let plan = pty_plan(workspace, &["--flood", "4000000000"], None);

    let mut supervisor = TokioProcessSupervisor::new();
    let id = supervisor
        .spawn_approved(handle, canonical, &plan)
        .expect("spawn_approved must succeed on the PTY path");

    assert_eq!(
        supervisor.observe(id),
        Some(ProcessStatus::Running),
        "the flooding child must observe as running right after spawn"
    );

    // Give it a moment to actually be mid-flood -- with unread output
    // already sitting in the pty -- before the supervisor is dropped.
    std::thread::sleep(FLOOD_WARMUP);
    assert_eq!(
        supervisor.observe(id),
        Some(ProcessStatus::Running),
        "the child must still be running once it has actually been flooding for a while"
    );

    let pid = nix::unistd::Pid::from_raw(i32::try_from(id.0).expect("pid fits in i32"));

    // Dropped without an explicit `stop`, and never subscribed to.
    drop(supervisor);

    // A zombie still answers `kill(pid, 0)` with `Ok(())` -- only a
    // confirmed reap makes it `ESRCH` (`tests/child_termination.rs`'s
    // `dropping_the_supervisor_kills_running_children` relies on the same
    // fact). Poll for that, bounded, rather than asserting on the first
    // check.
    let deadline = Instant::now() + REAP_DEADLINE;
    loop {
        match nix::sys::signal::kill(pid, None) {
            Err(nix::errno::Errno::ESRCH) => break,
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            other => panic!(
                "the flooding pty child was left un-reaped after the supervisor was dropped \
                 (last check: {other:?})"
            ),
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}
