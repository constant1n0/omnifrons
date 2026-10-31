//! Reproduction attempt for issue #185: a `pty-cli` launch that never
//! returned in the packaged app (no fork, no pty, no log, no error).
//!
//! The pre-fork path is `plan_stdio` -> `pty::open_pair` under `SPAWN_LOCK`
//! -> `finish_spawn` -> `SPAWN_LOCK` again -> fork. This file drives that
//! path with the `fake-agent` fixture over the PTY transport, in two shapes:
//!
//! * 50 children spawned and run to their end one after another;
//! * 5 rounds of 8 children spawned at the same time from 8 threads.
//!
//! Every `spawn_approved` must return within [`SPAWN_BOUND`] and every run
//! must deliver its terminal `State` frame within [`STATE_BOUND`], with no
//! `observe` or `stop` call (the self-exit reaper makes that possible).
//!
//! Runs in the default test run: the whole file takes a few seconds. A
//! failure names the child, the bound and the elapsed time.

#![cfg(unix)]

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use omnifrons_app::{
    EnvPlan, ExecHandle, FramePayload, LaunchPlan, OutputFrame, ProcessOutput,
    ProcessTerminalState, StdinPlan, WorkspaceRoot,
};
use omnifrons_domain::adapter::TransportClass;
use omnifrons_domain::scope::ScopeMode;
use omnifrons_supervisor::TokioProcessSupervisor;

const SEQUENTIAL_CHILDREN: usize = 50;
const PARALLEL_THREADS: usize = 8;
const PARALLEL_ROUNDS: usize = 5;
const SPAWN_BOUND: Duration = Duration::from_secs(5);
const STATE_BOUND: Duration = Duration::from_secs(10);

fn plan(dir: &std::path::Path) -> LaunchPlan {
    LaunchPlan {
        argv: vec!["--pty-report".to_string()],
        env: EnvPlan::new(&[]).expect("an empty declared-key list can never be secret-shaped"),
        cwd: WorkspaceRoot::new(dir).expect("the created dir must be a valid workspace"),
        stdin: StdinPlan::Null,
        prompt: None,
        scope_mode: ScopeMode::Advisory,
        transport: TransportClass::Pty,
        output_dir: None,
    }
}

fn wait_for_state(frames: &Receiver<OutputFrame>, bound: Duration) -> Option<ProcessTerminalState> {
    let end = Instant::now() + bound;
    loop {
        match frames.recv_timeout(end.saturating_duration_since(Instant::now())) {
            Ok(frame) => {
                if let FramePayload::State(state) = frame.payload {
                    return Some(state);
                }
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// Spawn one child with a watchdog on the spawn call itself: a hung
/// `spawn_approved` is reported as a failure naming `label`, not a hang.
fn spawn_bounded(
    supervisor: &TokioProcessSupervisor,
    dir: &std::path::Path,
    label: &str,
) -> (omnifrons_app::ProcessId, Receiver<OutputFrame>, Duration) {
    let program = std::fs::canonicalize(env!("CARGO_BIN_EXE_fake-agent"))
        .expect("the fixture must canonicalize");
    let file = std::fs::File::open(&program).expect("opening the fixture must succeed");
    let launch = plan(dir);
    let mut handle = supervisor.clone();
    let (tx, rx) = mpsc::channel();
    let began = Instant::now();
    let path = PathBuf::from(&program);
    std::thread::spawn(move || {
        let spawned = handle
            .spawn_approved(ExecHandle::File(file), path, &launch)
            .and_then(|id| handle.subscribe(id).map(|frames| (id, frames)));
        let _ = tx.send(spawned);
    });
    match rx.recv_timeout(SPAWN_BOUND) {
        Ok(Ok((id, frames))) => (id, frames, began.elapsed()),
        Ok(Err(error)) => panic!("{label}: spawn failed: {error}"),
        Err(RecvTimeoutError::Timeout) => {
            panic!("{label}: spawn_approved did not return within {SPAWN_BOUND:?}")
        }
        Err(RecvTimeoutError::Disconnected) => panic!("{label}: the spawn thread died"),
    }
}

fn run_one(supervisor: &TokioProcessSupervisor, dir: &std::path::Path, label: &str) -> Duration {
    let (_id, frames, spawn) = spawn_bounded(supervisor, dir, label);
    let began = Instant::now();
    let state = wait_for_state(&frames, STATE_BOUND);
    assert_eq!(
        state,
        Some(ProcessTerminalState::Exited { code: Some(0) }),
        "{label}: no terminal State frame within {STATE_BOUND:?} (waited {:?})",
        began.elapsed()
    );
    spawn
}

fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "omnifrons-pty-stress-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("failed to create the test workspace directory");
    dir
}

#[test]
fn fifty_pty_children_in_sequence_each_spawn_and_end() {
    let dir = workspace("seq");
    let supervisor = TokioProcessSupervisor::new();
    let mut slowest = Duration::ZERO;
    for n in 0..SEQUENTIAL_CHILDREN {
        slowest = slowest.max(run_one(&supervisor, &dir, &format!("sequential child {n}")));
    }
    let _ = std::fs::remove_dir_all(&dir);
    eprintln!(
        "pty-spawn-stress sequential children={SEQUENTIAL_CHILDREN} slowest_spawn={slowest:?}"
    );
}

#[test]
fn eight_parallel_pty_children_for_five_rounds_each_spawn_and_end() {
    let dir = workspace("par");
    let supervisor = TokioProcessSupervisor::new();
    let mut slowest = Duration::ZERO;
    for round in 0..PARALLEL_ROUNDS {
        let workers: Vec<_> = (0..PARALLEL_THREADS)
            .map(|thread| {
                let supervisor = supervisor.clone();
                let dir = dir.clone();
                std::thread::spawn(move || {
                    run_one(&supervisor, &dir, &format!("round {round} thread {thread}"))
                })
            })
            .collect();
        for worker in workers {
            slowest = slowest.max(worker.join().expect("a stress worker panicked"));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    eprintln!(
        "pty-spawn-stress parallel rounds={PARALLEL_ROUNDS} threads={PARALLEL_THREADS} slowest_spawn={slowest:?}"
    );
}
