//! The `ProcessOutput` port: a one-shot subscription to a process's
//! captured stdout/stderr/state frames.
//!
//! Deliberately independent of [`crate::ProcessSupervisor`]: a supervisor
//! spawns, observes, and stops a process, while this port only ever hands
//! back the frames an adapter has already captured for it. Keeping the two
//! ports separate means the existing `ProcessSupervisor` contract tests
//! (`crates/omnifrons-app/tests/contract_process_supervisor.rs`) need no
//! change to accommodate output capture.

use std::sync::mpsc::Receiver;

pub use omnifrons_domain::output::{FramePayload, OutputFrame, OutputStream};

use crate::process_supervisor::{ProcessId, SupervisorError};

/// A port for subscribing to a single process's captured output frames.
///
/// `subscribe` is one-shot per id: a conformant implementation hands back a
/// receiver at most once for any given [`ProcessId`], and reports
/// [`SupervisorError::AlreadySubscribed`] on a second call for the same id
/// rather than silently returning a fresh (and now-diverging) receiver.
pub trait ProcessOutput {
    /// Subscribe to `id`'s output frames, returning a receiver that yields
    /// them in `seq` order.
    ///
    /// # Errors
    ///
    /// Returns [`SupervisorError::UnknownProcess`] if `id` is not known.
    /// Returns [`SupervisorError::AlreadySubscribed`] if `id` already has a
    /// live subscription.
    fn subscribe(&mut self, id: ProcessId) -> Result<Receiver<OutputFrame>, SupervisorError>;
}
