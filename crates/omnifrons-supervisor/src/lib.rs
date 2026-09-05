//! Rust process supervisor: implements `omnifrons_app::ProcessSupervisor`
//! over `tokio::process`, with Unix process-group termination and a
//! Windows Job Object stub.
//!
//! Containment is platform-specific and "must be proven in the planned
//! desktop verification plan" (docs/adr/0002-desktop-technology-stack.md §
//! Process supervision). On Unix this crate spawns each child in its own
//! process group and terminates the whole group (SIGTERM, then SIGKILL
//! after the deadline). On Windows, group termination is not yet proven --
//! that requires a Job Object policy (VP-001 VP-S5) -- so `stop` reports
//! [`ProcessTerminalState::OrphanRiskUncertain`] there rather than a false
//! "cleanly stopped" (docs/target-architecture.md § Required failure
//! states).

use std::collections::HashMap;
use std::sync::Mutex;

use omnifrons_app::{
    ProcessId, ProcessSpec, ProcessStatus, ProcessSupervisor, ProcessTerminalState, SupervisorError,
};
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::runtime::Runtime;

/// The bookkeeping this supervisor holds for one spawned process.
///
/// A child is evicted to `Terminal` the moment it is confirmed reaped (in
/// `stop`, or when `observe` notices exit via `try_wait`), and its `Child`
/// handle is dropped at that point. This is what makes a second `stop` (or
/// `observe`) call on the same id safe: once `Terminal`, neither ever
/// touches the OS with `id`'s pid/pgid again, so there is no way to
/// re-signal a process id the OS has since recycled for something else.
/// Every other outcome (a signal error, or a deadline elapsing without a
/// confirmed reap) reports `OrphanRiskUncertain` to the caller but leaves
/// the entry `Running`: the `Child` handle is still live and unreaped, so
/// the OS cannot yet have recycled its pid.
enum Tracked {
    /// A live child, not yet confirmed reaped. Boxed: `Child` is
    /// significantly larger on Windows than the `Terminal` variant, and
    /// boxing keeps every `Tracked` entry (including the far more common
    /// eventual `Terminal` one) at the smaller size instead of every entry
    /// paying for the largest variant.
    Running(Box<Child>),
    /// Confirmed terminal: the `Child` handle has been dropped, and this
    /// state is returned directly to any later `stop`/`observe` call.
    Terminal(ProcessTerminalState),
}

/// A `ProcessSupervisor` backed by `tokio::process`, driven from behind a
/// synchronous interface.
///
/// The port is sync-with-deadline (see `omnifrons_app::ProcessSupervisor`
/// docs), while the actual process I/O this crate needs -- Tokio's SIGCHLD-
/// driven child-exit notification -- is async. This struct owns a private
/// current-thread [`Runtime`] and enters it around every Tokio call, so the
/// asynchrony stays an implementation detail behind the synchronous port.
pub struct TokioProcessSupervisor {
    runtime: Runtime,
    children: Mutex<HashMap<ProcessId, Tracked>>,
}

impl Default for TokioProcessSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl TokioProcessSupervisor {
    /// Build a new supervisor with its own private Tokio runtime.
    ///
    /// # Panics
    ///
    /// Panics if a current-thread Tokio runtime could not be built (e.g.
    /// the OS refuses to create the runtime's I/O/timer driver).
    #[must_use]
    pub fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build the process supervisor's Tokio runtime");
        Self {
            runtime,
            children: Mutex::new(HashMap::new()),
        }
    }
}

impl ProcessSupervisor for TokioProcessSupervisor {
    fn spawn(&mut self, spec: ProcessSpec) -> Result<ProcessId, SupervisorError> {
        let _guard = self.runtime.enter();

        let mut command = Command::new(&spec.program);
        command.args(&spec.args);

        #[cfg(unix)]
        {
            // Group the child under its own process group so a later stop
            // can signal the whole group, not just the direct child
            // (docs/adr/0002 § Process supervision). Stable since Rust
            // 1.64 (`std::os::unix::process::CommandExt::process_group`,
            // re-exposed by `tokio::process::Command`).
            command.process_group(0);
        }

        // Best-effort safety net: if this supervisor is dropped without an
        // explicit `stop` for this child (a panicking test, an early
        // return), `Drop` below still tries to signal it, but `kill_on_drop`
        // also asks Tokio itself to try killing the direct child should the
        // `Child` handle be dropped some other way. Neither is containment
        // -- see `Drop`'s own doc comment.
        command.kill_on_drop(true);

        let child = command.spawn().map_err(|error| {
            tracing::warn!(program = %spec.program, %error, "failed to spawn process");
            SupervisorError::Spawn(error.to_string())
        })?;
        let pid = child
            .id()
            .ok_or_else(|| SupervisorError::Spawn("spawned child reported no pid".to_string()))?;
        let id = ProcessId(pid);

        self.children
            .lock()
            .expect("children mutex poisoned by a prior panic")
            .insert(id, Tracked::Running(Box::new(child)));

        tracing::debug!(program = %spec.program, pid, "spawned process");
        Ok(id)
    }

    fn stop(
        &mut self,
        id: ProcessId,
        deadline: Duration,
    ) -> Result<ProcessTerminalState, SupervisorError> {
        let _guard = self.runtime.enter();

        #[cfg(unix)]
        {
            unix::stop(&self.children, id, deadline)
        }
        #[cfg(windows)]
        {
            windows::stop(&self.children, id, deadline)
        }
    }

    fn observe(&self, id: ProcessId) -> Option<ProcessStatus> {
        let _guard = self.runtime.enter();
        let mut children = self
            .children
            .lock()
            .expect("children mutex poisoned by a prior panic");
        let tracked = children.get_mut(&id)?;
        match tracked {
            Tracked::Terminal(state) => Some(ProcessStatus::Terminal(*state)),
            Tracked::Running(child) => match child.try_wait() {
                Ok(Some(status)) => {
                    let state = ProcessTerminalState::Exited {
                        code: status.code(),
                    };
                    // Confirmed reaped: evict now, so nothing ever touches
                    // this pid/pgid again.
                    *tracked = Tracked::Terminal(state);
                    Some(ProcessStatus::Terminal(state))
                }
                Ok(None) => Some(ProcessStatus::Running),
                // The OS could not answer whether the child is still
                // running -- not a confirmed reap, so the entry stays
                // `Running` (its pid cannot yet have been recycled).
                // Descendants (and the child itself) are then unproven
                // stopped rather than provably running or exited.
                Err(_) => Some(ProcessStatus::Terminal(
                    ProcessTerminalState::OrphanRiskUncertain,
                )),
            },
        }
    }
}

impl Drop for TokioProcessSupervisor {
    /// Best-effort cleanup, not containment: if this supervisor is dropped
    /// without a prior `stop` for every child it spawned (e.g. a panicking
    /// test, or an early return), send SIGKILL to the process group of
    /// every entry still `Running` on unix, so a bug elsewhere in this
    /// process does not silently orphan a live descendant. Errors are
    /// swallowed except for a `warn`: there is no caller left to report
    /// them to, and this runs during unwinding as readily as during a
    /// normal drop, where panicking would abort the process.
    ///
    /// Also makes a short, bounded best-effort attempt to reap each child it
    /// kills: `stop`'s normal reap relies on this supervisor's own Tokio
    /// runtime, but that runtime is torn down around the same time as this
    /// cleanup runs (it is a sibling field, dropped in declaration order),
    /// so it cannot be relied on here. Without this, a killed child would
    /// become a zombie nobody ever collects. If the bounded wait gives up,
    /// this leaves a zombie rather than blocking drop indefinitely --
    /// acceptable, since this is best effort, not containment.
    ///
    /// This does not attempt Windows containment (the Job Object policy
    /// VP-001 VP-S5 needs is not yet implemented there, matching `stop`'s
    /// own honesty about that gap).
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let children = match self.children.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            for (id, tracked) in children.iter() {
                if matches!(tracked, Tracked::Running(_)) {
                    let Ok(raw_pid) = i32::try_from(id.0) else {
                        continue;
                    };
                    let pid = nix::unistd::Pid::from_raw(raw_pid);
                    if let Err(error) =
                        nix::sys::signal::killpg(pid, nix::sys::signal::Signal::SIGKILL)
                    {
                        tracing::warn!(
                            pid = id.0,
                            %error,
                            "best-effort SIGKILL on supervisor drop failed"
                        );
                    }
                    // Bounded best-effort reap: a killed child normally
                    // dies within milliseconds, so give it a short window
                    // rather than blocking drop indefinitely.
                    let deadline =
                        std::time::Instant::now() + std::time::Duration::from_millis(200);
                    loop {
                        match nix::sys::wait::waitpid(
                            pid,
                            Some(nix::sys::wait::WaitPidFlag::WNOHANG),
                        ) {
                            Ok(nix::sys::wait::WaitStatus::StillAlive)
                                if std::time::Instant::now() < deadline =>
                            {
                                std::thread::sleep(std::time::Duration::from_millis(5));
                            }
                            _ => break,
                        }
                    }
                }
            }
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use nix::errno::Errno;
    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;
    use omnifrons_app::{ProcessId, ProcessTerminalState, SupervisorError};

    use crate::Tracked;

    const POLL_INTERVAL: Duration = Duration::from_millis(20);
    const KILL_GRACE: Duration = Duration::from_millis(500);

    /// The outcome of a `killpg` call, classified for what the caller can
    /// safely conclude from it.
    #[derive(Debug, PartialEq, Eq)]
    enum SignalOutcome {
        /// The signal was delivered, or the group was already gone
        /// (`ESRCH`) -- not itself a failure, since the child may have
        /// exited in a race with this call; the reap poll confirms the
        /// actual terminal state.
        Proceed,
        /// `killpg` failed with something other than "group already
        /// gone" (e.g. `EPERM`): the caller cannot tell whether the
        /// signal landed, so the outcome is unproven rather than assumed
        /// delivered.
        Uncertain(Errno),
    }

    /// Classify a `killpg` result into what the caller may safely assume.
    fn classify_killpg_result(result: nix::Result<()>) -> SignalOutcome {
        match result {
            Ok(()) | Err(Errno::ESRCH) => SignalOutcome::Proceed,
            Err(other) => SignalOutcome::Uncertain(other),
        }
    }

    pub(crate) fn stop(
        children: &Mutex<HashMap<ProcessId, Tracked>>,
        id: ProcessId,
        deadline: Duration,
    ) -> Result<ProcessTerminalState, SupervisorError> {
        {
            let guard = children
                .lock()
                .expect("children mutex poisoned by a prior panic");
            match guard.get(&id) {
                None => return Err(SupervisorError::UnknownProcess),
                // Already confirmed terminal: report the same recorded
                // state again, without ever touching this pid/pgid --
                // which the OS may since have recycled for something
                // else.
                Some(Tracked::Terminal(state)) => return Ok(*state),
                Some(Tracked::Running(_)) => {}
            }
        }

        // `process_group(0)` at spawn time made the child its own group
        // leader, so its pid doubles as the pgid: killpg targets the
        // whole group, not only the direct child.
        let Ok(raw_pid) = i32::try_from(id.0) else {
            return Err(SupervisorError::UnknownProcess);
        };
        let pgid = Pid::from_raw(raw_pid);

        // Best-effort: a group that has already exited yields ESRCH here,
        // which is not itself a failure -- the loop below confirms the
        // actual terminal state via `try_wait`. Any other error (e.g.
        // EPERM) means we cannot tell whether the signal landed, so that
        // outcome must not be treated as if it had.
        tracing::debug!(pid = id.0, "sending SIGTERM to process group");
        if let SignalOutcome::Uncertain(errno) =
            classify_killpg_result(killpg(pgid, Signal::SIGTERM))
        {
            tracing::warn!(
                pid = id.0,
                %errno,
                "killpg(SIGTERM) failed unexpectedly; orphan-risk/uncertain"
            );
            return Ok(ProcessTerminalState::OrphanRiskUncertain);
        }

        if let Some(state) = poll_until_reaped(
            children,
            id,
            deadline,
            ProcessTerminalState::Exited { code: None },
        ) {
            tracing::debug!(pid = id.0, ?state, "process group terminated gracefully");
            return Ok(state);
        }

        tracing::warn!(
            pid = id.0,
            ?deadline,
            "deadline elapsed; escalating to SIGKILL"
        );
        if let SignalOutcome::Uncertain(errno) =
            classify_killpg_result(killpg(pgid, Signal::SIGKILL))
        {
            tracing::warn!(
                pid = id.0,
                %errno,
                "killpg(SIGKILL) failed unexpectedly; orphan-risk/uncertain"
            );
            return Ok(ProcessTerminalState::OrphanRiskUncertain);
        }

        if let Some(state) =
            poll_until_reaped(children, id, KILL_GRACE, ProcessTerminalState::Killed)
        {
            return Ok(state);
        }

        // Even SIGKILL could not be confirmed reaped within the grace
        // period: descendants are not provably stopped
        // (docs/target-architecture.md § Required failure states).
        tracing::error!(
            pid = id.0,
            "SIGKILL sent but reap unconfirmed; orphan-risk/uncertain"
        );
        Ok(ProcessTerminalState::OrphanRiskUncertain)
    }

    /// Poll `try_wait` until the child is reaped or `budget` elapses.
    ///
    /// On reap, evicts the entry to `Tracked::Terminal` (dropping the
    /// `Child` handle) and returns the exit-derived terminal state
    /// (`Exited` carries the real exit code; the caller-supplied `on_reap`
    /// template is used only to pick `Exited`/`Killed` framing when no
    /// exit code is meaningful, e.g. after a signal). Until reap is
    /// confirmed, the entry is left `Running`: its pid cannot yet have
    /// been recycled by the OS.
    fn poll_until_reaped(
        children: &Mutex<HashMap<ProcessId, Tracked>>,
        id: ProcessId,
        budget: Duration,
        on_reap: ProcessTerminalState,
    ) -> Option<ProcessTerminalState> {
        let start = Instant::now();
        loop {
            {
                let mut guard = children
                    .lock()
                    .expect("children mutex poisoned by a prior panic");
                let tracked = guard.get_mut(&id).expect("checked present by the caller");
                let Tracked::Running(child) = tracked else {
                    unreachable!("this entry is only ever reached while still Running")
                };
                if let Ok(Some(status)) = child.try_wait() {
                    let state = match on_reap {
                        ProcessTerminalState::Exited { .. } => ProcessTerminalState::Exited {
                            code: status.code(),
                        },
                        other => other,
                    };
                    *tracked = Tracked::Terminal(state);
                    return Some(state);
                }
            }
            let elapsed = start.elapsed();
            if elapsed >= budget {
                return None;
            }
            std::thread::sleep(POLL_INTERVAL.min(budget.saturating_sub(elapsed)));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::SignalOutcome;
        use super::classify_killpg_result;
        use nix::errno::Errno;

        #[test]
        fn ok_proceeds() {
            assert!(matches!(
                classify_killpg_result(Ok(())),
                SignalOutcome::Proceed
            ));
        }

        #[test]
        fn esrch_proceeds_as_group_already_gone() {
            assert!(matches!(
                classify_killpg_result(Err(Errno::ESRCH)),
                SignalOutcome::Proceed
            ));
        }

        #[test]
        fn eperm_is_uncertain() {
            assert!(matches!(
                classify_killpg_result(Err(Errno::EPERM)),
                SignalOutcome::Uncertain(Errno::EPERM)
            ));
        }

        #[test]
        fn other_errno_is_uncertain() {
            assert!(matches!(
                classify_killpg_result(Err(Errno::EINVAL)),
                SignalOutcome::Uncertain(Errno::EINVAL)
            ));
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Duration;

    use omnifrons_app::{ProcessId, ProcessTerminalState, SupervisorError};

    use crate::Tracked;

    /// Stub pending the Job Object implementation (VP-001 VP-S5).
    ///
    /// Windows process-group termination is not yet proven: without a Job
    /// Object policy, a breakaway-attempting descendant may survive
    /// unnoticed. Until that is implemented and verified, this always
    /// reports `OrphanRiskUncertain` rather than a containment claim this
    /// crate cannot back (docs/target-architecture.md § Required failure
    /// states; docs/adr/0002-desktop-technology-stack.md § Process
    /// supervision). Because containment is unproven, a confirmed reap
    /// never happens here either, so the entry is deliberately never
    /// evicted to `Terminal`: every call, including a repeat one, takes the
    /// same honest, unproven path.
    pub(crate) fn stop(
        children: &Mutex<HashMap<ProcessId, Tracked>>,
        id: ProcessId,
        _deadline: Duration,
    ) -> Result<ProcessTerminalState, SupervisorError> {
        let mut guard = children
            .lock()
            .expect("children mutex poisoned by a prior panic");
        match guard.get_mut(&id).ok_or(SupervisorError::UnknownProcess)? {
            Tracked::Terminal(state) => Ok(*state),
            Tracked::Running(child) => {
                // Best-effort direct-child termination; containment of any
                // descendant it spawned is unproven without a Job Object.
                let _ = child.start_kill();
                Ok(ProcessTerminalState::OrphanRiskUncertain)
            }
        }
    }
}
