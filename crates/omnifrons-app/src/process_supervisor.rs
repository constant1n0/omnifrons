//! The `ProcessSupervisor` port: spawn, observe, and stop a single external
//! process, with a deadline-bounded graceful-then-forceful stop.

use std::time::Duration;

pub use omnifrons_domain::scope::ProcessTerminalState;

/// A process identifier, opaque to callers beyond equality and hashing.
///
/// The wrapped `u32` is the platform process id for adapters that spawn
/// real OS processes (e.g. `omnifrons-supervisor`); a test double is free
/// to use it as an opaque counter instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessId(pub u32);

/// What to spawn: a program and its arguments.
///
/// Deliberately minimal for the skeleton -- no `cwd`, environment, or I/O
/// configuration yet; those join once a real adapter needs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    /// The program to execute.
    pub program: String,
    /// Arguments passed to the program.
    pub args: Vec<String>,
}

impl ProcessSpec {
    /// Build a spec for `program` with no arguments.
    #[must_use]
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
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
#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    /// The process failed to spawn.
    #[error("failed to spawn process: {0}")]
    Spawn(String),
    /// The given `ProcessId` is not known to this supervisor.
    #[error("unknown process id")]
    UnknownProcess,
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
