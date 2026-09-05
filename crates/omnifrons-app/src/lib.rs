//! Application services over the `omnifrons-domain` core.
//!
//! Depends on `omnifrons-domain` and `thiserror` only
//! (docs/repository-layout.md § Crate map): this crate and the domain core
//! it sits on must run and test without Tauri, Tokio, or any adapter.

mod process_supervisor;

pub use process_supervisor::{
    ProcessId, ProcessSpec, ProcessStatus, ProcessSupervisor, ProcessTerminalState, SupervisorError,
};

/// Reusable contract tests for any `ProcessSupervisor` implementation.
///
/// Gated behind the `contract-tests` feature so production builds never
/// compile or pay for them; enable it (`--features contract-tests` or
/// `--all-features`) to run them from a dependent crate's own tests.
#[cfg(feature = "contract-tests")]
pub mod contract;
