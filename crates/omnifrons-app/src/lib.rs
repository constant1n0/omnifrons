//! Application services over the `omnifrons-domain` core.
//!
//! Depends on `omnifrons-domain` and `thiserror` only
//! (docs/repository-layout.md § Crate map): this crate and the domain core
//! it sits on must run and test without Tauri, Tokio, or any adapter.

pub mod approval_store;
pub mod clock;
pub mod executable_prober;
pub mod harness_adapter;
mod harness_catalog;
pub mod launch_gate;
pub mod outbox_policy;
mod process_output;
mod process_supervisor;
pub mod run_outbox;
pub mod terminal_normalizer;

pub use approval_store::{ApprovalStore, ApprovalStoreError};
pub use clock::Clock;
pub use executable_prober::{
    ExecHandle, ExecutableIdentity, ExecutableProber, ProbeOutcome, ProbedExecutable,
};
pub use harness_adapter::{
    AdapterCatalog, AdapterDescriptor, AgentPrompt, Allowlist, Assembled, AssembledLine,
    EnvAssignment, EnvPlan, HarnessAdapter, LaunchPlan, LaunchPlanError, LaunchRequest,
    LineAssembler, OUTPUT_DIR_ENV_KEY, StdinPlan, WorkspaceRoot, is_secret_shaped,
    validate_cwd_within_workspace,
};
pub use harness_catalog::{HarnessKind, HarnessRequest, InvalidRequest};
pub use launch_gate::{GateDecision, LaunchGate};
pub use outbox_policy::{ArtifactClassifier, OutboxPolicy, OutboxPolicyStore, PolicyError};
pub use process_output::{FramePayload, OutputFrame, OutputStream, ProcessOutput};
pub use process_supervisor::{
    ProcessId, ProcessSpec, ProcessStatus, ProcessSupervisor, ProcessTerminalState, SupervisorError,
};
pub use run_outbox::{
    CandidateProber, DirectoryHandle, OutboxInventory, PreparedRunSubdirectory, RunOutboxPreparer,
};
pub use terminal_normalizer::{TerminalChunk, TerminalNormalizer};

/// Reusable contract tests for any `ProcessSupervisor` implementation.
///
/// Gated behind the `contract-tests` feature so production builds never
/// compile or pay for them; enable it (`--features contract-tests` or
/// `--all-features`) to run them from a dependent crate's own tests.
#[cfg(feature = "contract-tests")]
pub mod contract;
