//! The running cap is enforced before any pseudo-terminal resource is
//! allocated (spike slice 4, `docs/spike-log.md` § Slice 4): with
//! `MAX_RUNNING_CHILDREN` children already `Running`, a
//! `TransportClass::Pty` launch is refused with exactly the
//! `SupervisorError::TooManyProcesses` the pipe path reports, and no
//! `openpty` call was ever made on its behalf.
//!
//! Its own test binary, deliberately. The proof lowers this process's
//! `RLIMIT_NOFILE` soft limit to zero for the duration of the refused
//! call, so that any descriptor allocation -- `openpty` included -- would
//! fail with `EMFILE` and surface as `SupervisorError::Spawn` instead of
//! the expected refusal. A before/after descriptor count could not tell
//! the two orders apart: a pair opened and then dropped on the cap error
//! leaves the count unchanged. Lowering the limit is process-wide, which
//! is why no other test shares this binary. Unix only, like the PTY path
//! itself (Windows refuses a `Pty` plan before the cap is consulted).
#![cfg(unix)]

use std::path::PathBuf;
use std::time::Duration;

use nix::sys::resource::{Resource, getrlimit, setrlimit};
use omnifrons_app::{
    EnvPlan, ExecHandle, HarnessKind, HarnessRequest, LaunchPlan, ProcessSupervisor, StdinPlan,
    SupervisorError, WorkspaceRoot,
};
use omnifrons_domain::adapter::TransportClass;
use omnifrons_domain::scope::ScopeMode;
use omnifrons_supervisor::TokioProcessSupervisor;

/// The supervisor's running cap (`MAX_RUNNING_CHILDREN` in
/// `crates/omnifrons-supervisor/src/lib.rs`), restated here because it is
/// private; `tests/bookkeeping_caps.rs` pins the same value.
const RUNNING_CAP: usize = 16;

/// Restores the descriptor soft limit it lowered when dropped, so a failed
/// assertion never leaves the process unable to open anything.
struct NoDescriptorsAllowed {
    soft: nix::libc::rlim_t,
    hard: nix::libc::rlim_t,
}

impl NoDescriptorsAllowed {
    fn enter() -> Self {
        let (soft, hard) =
            getrlimit(Resource::RLIMIT_NOFILE).expect("reading RLIMIT_NOFILE must succeed");
        setrlimit(Resource::RLIMIT_NOFILE, 0, hard)
            .expect("lowering the RLIMIT_NOFILE soft limit to zero must succeed");
        Self { soft, hard }
    }
}

impl Drop for NoDescriptorsAllowed {
    fn drop(&mut self) {
        let _ = setrlimit(Resource::RLIMIT_NOFILE, self.soft, self.hard);
    }
}

fn temp_workspace() -> (PathBuf, WorkspaceRoot) {
    let dir = std::env::temp_dir().join(format!(
        "omnifrons-pty-running-cap-test-{}",
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

/// A `pty-cli`-shaped plan, exactly as `tests/pty_launch.rs` builds one.
fn pty_plan(workspace: WorkspaceRoot) -> LaunchPlan {
    LaunchPlan {
        argv: vec!["--pty-report".to_string()],
        env: EnvPlan::new(&[]).expect("an empty declared-key list can never be secret-shaped"),
        cwd: workspace,
        stdin: StdinPlan::Null,
        prompt: None,
        scope_mode: ScopeMode::Advisory,
        transport: TransportClass::Pty,
    }
}

/// At the cap, a PTY launch is refused with the pipe path's own error and
/// without opening a pseudo-terminal: inside the no-descriptor window an
/// `openpty` would have failed with `EMFILE` and turned the result into
/// `SupervisorError::Spawn`, so `TooManyProcesses` proves the cap was
/// checked first.
#[test]
fn a_pty_launch_at_the_running_cap_is_refused_before_any_pty_is_opened() {
    let demo_harness = PathBuf::from(env!("CARGO_BIN_EXE_demo-harness"));
    let mut supervisor = TokioProcessSupervisor::with_demo_launcher(demo_harness);

    // Long-lived enough (5 Hz, 200 lines ~= 40s) to stay Running for the
    // whole test; stopped explicitly at the end so no real child leaks.
    let long_lived =
        || HarnessRequest::new(HarnessKind::DemoLines, 5, 200).expect("a valid demo request");
    let running: Vec<_> = (0..RUNNING_CAP)
        .map(|_| {
            supervisor
                .spawn_harness(&long_lived())
                .expect("spawning up to the running cap must succeed")
        })
        .collect();

    let pipe_refusal = supervisor
        .spawn_harness(&long_lived())
        .expect_err("the pipe path must refuse a launch beyond the running cap");
    assert_eq!(pipe_refusal, SupervisorError::TooManyProcesses);

    let (dir, workspace) = temp_workspace();
    let (handle, canonical) = fake_agent_handle();
    let plan = pty_plan(workspace);

    let pty_refusal = {
        let _no_descriptors = NoDescriptorsAllowed::enter();
        supervisor.spawn_approved(handle, canonical, &plan)
    }
    .expect_err("the PTY path must refuse a launch beyond the running cap");
    assert_eq!(
        pty_refusal, pipe_refusal,
        "a PTY launch at the cap must be refused with the pipe path's own error, before any \
         pseudo-terminal is opened (an openpty inside the no-descriptor window would have \
         surfaced as SupervisorError::Spawn)"
    );

    for id in running {
        let _ = supervisor.stop(id, Duration::from_secs(3));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
