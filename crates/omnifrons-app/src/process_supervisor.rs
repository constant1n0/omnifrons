//! The `ProcessSupervisor` port: spawn, observe, and stop a single external
//! process, with a deadline-bounded graceful-then-forceful stop.

use std::path::PathBuf;
use std::time::Duration;

pub use omnifrons_domain::scope::ProcessTerminalState;

use crate::harness_adapter::{EnvPlan, StdinPlan};

/// A process identifier, opaque to callers beyond equality and hashing.
///
/// The wrapped `u32` is the platform process id for adapters that spawn
/// real OS processes (e.g. `omnifrons-supervisor`); a test double is free
/// to use it as an opaque counter instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessId(pub u32);

/// What to spawn: a program, its arguments, its environment, its working
/// directory, and how its stdin is configured.
///
/// `env`/`cwd`/`stdin` were added in the spike slice-3 spike
/// (`docs/spike-log.md` § Slice 3), alongside the first built-in harness
/// adapter; [`Self::new`]'s defaults (`EnvPlan::Inherit`, `cwd: None`,
/// `StdinPlan::Null`) preserve the demo-harness launch path's own
/// pre-existing behavior unchanged -- full environment inheritance (this
/// application's own binary, not a caller-supplied executable, so nothing
/// is at stake the way it would be for an adapter's process) and no
/// working-directory override. `StdinPlan::Null` is a deliberate
/// tightening over the literal previous default (an unconfigured stdin,
/// which `tokio::process::Command` inherits from the parent): the demo
/// harness never reads its own stdin, so this is behaviorally invisible to
/// every existing test while closing an unintended inherited-stdin gap for
/// every spawn this port ever performs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    /// The program to execute.
    pub program: String,
    /// Arguments passed to the program.
    pub args: Vec<String>,
    /// The spawned process's environment.
    pub env: EnvPlan,
    /// The spawned process's working directory, or `None` to inherit the
    /// supervisor's own.
    pub cwd: Option<PathBuf>,
    /// How the spawned process's stdin is configured.
    pub stdin: StdinPlan,
}

impl ProcessSpec {
    /// Build a spec for `program` with no arguments, inheriting the
    /// parent environment, no working-directory override, and
    /// [`StdinPlan::Null`].
    #[must_use]
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: EnvPlan::Inherit,
            cwd: None,
            stdin: StdinPlan::Null,
        }
    }

    /// Add arguments to the spec, builder-style.
    #[must_use]
    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }
}

/// The observed status of a process known to a supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    /// The process is still running.
    Running,
    /// The process has reached a terminal state.
    Terminal(ProcessTerminalState),
}

/// An error a `ProcessSupervisor` operation can report.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SupervisorError {
    /// The process failed to spawn.
    #[error("failed to spawn process: {0}")]
    Spawn(String),
    /// The given `ProcessId` is not known to this supervisor.
    #[error("unknown process id")]
    UnknownProcess,
    /// A second `subscribe` call was made for a `ProcessId` that already
    /// has a live subscription.
    ///
    /// Chosen over reusing [`Self::UnknownProcess`] for this case: the id
    /// *is* known, so collapsing the two would tell a caller that
    /// legitimately still holds the first receiver that its process
    /// vanished, when the real problem is a duplicate subscribe attempt
    /// (`docs/spike-log.md` § IPC contract records this choice).
    #[error("process output already subscribed")]
    AlreadySubscribed,
    /// `spawn` was refused because this supervisor already tracks its
    /// maximum number of concurrently `Running` processes.
    ///
    /// A closed cap, not an unbounded map, is what keeps a long-lived
    /// supervisor's own bookkeeping bounded regardless of how many spawn
    /// requests a caller issues (`docs/spike-log.md` § IPC contract).
    #[error("too many processes are already running")]
    TooManyProcesses,
}

/// A port for supervising a single external process end-to-end: spawn,
/// liveness observation, and a graceful-then-forceful stop bounded by a
/// deadline.
///
/// ## Why sync-with-deadline, not async
///
/// This port is deliberately synchronous rather than `async fn`.
/// `omnifrons-domain` and `omnifrons-app` must "run and test without Tauri
/// or a `WebView`" (docs/adr/0002-desktop-technology-stack.md § Proposed
/// decision), and by extension without requiring an async executor merely
/// to express the port's shape. Making `spawn`/`stop`/`observe` `async`
/// would force a runtime dependency (Tokio) into `omnifrons-app`, when the
/// crate map places Tokio only on the adapter that needs it,
/// `omnifrons-supervisor` (docs/repository-layout.md § Crate map). The
/// `deadline: Duration` parameter on `stop` gives callers the same
/// backpressure guarantee an async timeout would provide, without the port
/// itself depending on an executor. An adapter that is naturally
/// asynchronous underneath (`omnifrons-supervisor`, built on Tokio) bridges
/// internally -- it may run its own runtime and block on it from behind
/// this synchronous interface -- rather than leaking that choice into the
/// application layer.
pub trait ProcessSupervisor {
    /// Spawn a process for `spec`, returning its id.
    ///
    /// # Errors
    ///
    /// Returns [`SupervisorError::Spawn`] if the process could not be
    /// started.
    fn spawn(&mut self, spec: ProcessSpec) -> Result<ProcessId, SupervisorError>;

    /// Stop the process identified by `id`, first gracefully then
    /// forcefully, allowing up to `deadline` to reach a terminal state.
    ///
    /// A `deadline` elapsing without a confirmed reap is not an error: it
    /// is reported as `Ok(`[`ProcessTerminalState::OrphanRiskUncertain`]`)`,
    /// the required failure state for descendants unproven stopped
    /// (docs/target-architecture.md § Required failure states).
    ///
    /// # Errors
    ///
    /// Returns [`SupervisorError::UnknownProcess`] if `id` is not known to
    /// this supervisor.
    ///
    /// # Blocking
    ///
    /// An implementation may block the calling thread for up to `deadline`
    /// while polling for a terminal state (e.g. `omnifrons-supervisor`'s
    /// poll loop uses `std::thread::sleep`). A caller running on an async
    /// executor must invoke `stop` from a blocking context -- e.g. Tokio's
    /// `spawn_blocking` -- rather than from an async task, to avoid
    /// stalling the executor.
    fn stop(
        &mut self,
        id: ProcessId,
        deadline: Duration,
    ) -> Result<ProcessTerminalState, SupervisorError>;

    /// Observe the current status of `id`, or `None` if it is not known to
    /// this supervisor.
    fn observe(&self, id: ProcessId) -> Option<ProcessStatus>;
}
