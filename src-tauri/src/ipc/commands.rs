//! The demo-harness IPC commands (`harness_spawn`, `harness_stop`,
//! `harness_observe`), the spike slice-2 executable-identity-and-approval
//! commands (`executable_pick_and_probe`, `executable_approve`,
//! `executable_revoke`, `approvals_list`), and the spike slice-3 built-in
//! adapter surface (`workspace_pick`, `workspace_current`,
//! `adapters_list`, and `harness_spawn`'s `kind: "adapter"` case).
//! Registered next to `shell_health` (`crate::shell_health`).
//!
//! No program path or argument vector for a demo-harness request ever
//! crosses IPC (`docs/spike-log.md` § IPC contract): the renderer names a
//! [`dto::HarnessKindDto`] plus small bounded numbers, validated into an
//! `omnifrons_app::HarnessRequest` before the supervisor ever sees it.
//! `harness_spawn`'s `kind: "approved"` and `kind: "adapter"` cases are the
//! two paths that do reach a real, caller-supplied executable, and only
//! ever by canonical path after a `LaunchGate` decision -- never a raw
//! argument vector; `kind: "adapter"`'s own argv comes only from the named
//! adapter's own fixed template (`docs/spike-log.md` § Slice 3). Every
//! error surfaces as a [`dto::ShellError`] with a fixed catalogue message
//! -- never the underlying `SupervisorError::Spawn`'s `io::Error` text,
//! which can carry a real filesystem path.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use omnifrons_adapters::{FsOutboxInventory, FsRunOutboxPreparer, JsonOutboxPolicyStore};
use omnifrons_app::outbox_policy::{
    OutboxPolicy, OutboxPolicyStore, POLICY_FILE_PATH, PolicyError,
};
use omnifrons_app::run_outbox::{
    CandidateProbe, EscapeReason, InventoriedEntry, OutboxDeclarationError, OutboxInventory,
    OutboxLocation, PrepareError, PreparedRunSubdirectory, RunOutboxPreparer,
    assemble_run_candidates, assemble_unattributed_candidates, summarize,
    validate_outbox_declaration,
};
use omnifrons_app::{
    ApprovalStore, ApprovalStoreError, EnvPlan, ExecutableProber, GateDecision, HarnessKind,
    HarnessRequest, InvalidRequest, LaunchPlan, LaunchPlanError, LaunchRequest, LineAssembler,
    OutputFrame, ProcessId, ProcessOutput, ProcessSupervisor, ProcessTerminalState, StdinPlan,
    SupervisorError, TerminalChunk, TerminalNormalizer, WorkspaceRoot,
};
use omnifrons_domain::adapter::{
    AdapterEvent, AdapterId, AgentPrompt, PromptError, TransportClass,
};
use omnifrons_domain::executable::{ApprovalId, DenialReason};
use omnifrons_domain::outbox::{OutboxFailure, PublishProposal, RunId};
use omnifrons_domain::scope::ScopeMode;
use omnifrons_supervisor::TokioProcessSupervisor;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::adapter_state::AdapterState;
use crate::executable_state::{CandidateId, CandidateTable, ExecutableState, ShellLaunchGate};
use crate::ipc::dto::{
    AdapterDescriptorDto, AdapterEventDto, ApprovalDto, CandidateDto, CandidateIdDto, EvidenceDto,
    HarnessFrame, HarnessKindDto, OutboxReasonTag, OutboxStateTag, OutboxStatusDto, ProbeResultDto,
    ProcessIdDto, ProcessStatusDto, ProcessTerminalStateDto, ShellError, ShellErrorCode,
    ShellErrorDetail, WorkAreaStateTag, WorkspaceDto,
};
use crate::outbox_state::{MAX_PROPOSED_ENTRIES, OutboxState, RunRecord, RunTable};
use crate::publication_state::PublicationState;
use omnifrons_app::work_area::WorkAreaRoot;

impl From<SupervisorError> for ShellError {
    /// Maps every `SupervisorError` to a fixed catalogue message. Never
    /// the underlying error's own text: `SupervisorError::Spawn` wraps an
    /// `io::Error` whose `Display` text can (and for a nonexistent-program
    /// error, does) contain the attempted program path.
    fn from(error: SupervisorError) -> Self {
        let (code, message) = match error {
            SupervisorError::Spawn(_) => (
                ShellErrorCode::SpawnFailed,
                "failed to start the requested process",
            ),
            SupervisorError::UnknownProcess => (
                ShellErrorCode::UnknownProcess,
                "no process with that id is known",
            ),
            SupervisorError::AlreadySubscribed => (
                ShellErrorCode::AlreadySubscribed,
                "output for that process id is already subscribed",
            ),
            SupervisorError::TooManyProcesses => (
                ShellErrorCode::TooManyProcesses,
                "too many processes are already running",
            ),
            SupervisorError::PtyUnsupported => (
                ShellErrorCode::PtyUnsupported,
                "pseudo-terminal launches are not available on this platform",
            ),
        };
        Self::new(code, message)
    }
}

impl From<InvalidRequest> for ShellError {
    fn from(_: InvalidRequest) -> Self {
        Self::new(
            ShellErrorCode::InvalidRequest,
            "the requested rate_hz or lines value is out of the accepted range",
        )
    }
}

impl From<ApprovalStoreError> for ShellError {
    /// Maps every `ApprovalStoreError` to the single
    /// `approval-store-unavailable` code: the four closed reasons a store
    /// operation can fail (unreadable, corrupt, a torn write, a colliding
    /// derived id) are all, from a caller's perspective, "the approval
    /// store is not usable right now" -- none of them is actionable
    /// differently by a caller, so splitting them into separate codes
    /// would add surface with no behavioral payoff.
    fn from(_: ApprovalStoreError) -> Self {
        Self::new(
            ShellErrorCode::ApprovalStoreUnavailable,
            "the approval store is unavailable",
        )
    }
}

impl From<DenialReason> for ShellError {
    /// Maps a `LaunchGate::decide` denial to its catalogue error.
    /// `ChangedSinceApproval` is the one case carrying structured detail
    /// (`ShellErrorDetail`): both digests as short hex prefixes, never in
    /// the message text itself (`docs/spike-log.md` § Slice 2).
    fn from(reason: DenialReason) -> Self {
        match reason {
            DenialReason::Unapproved => Self::new(
                ShellErrorCode::Unapproved,
                "this executable has not been approved",
            ),
            DenialReason::Revoked => Self::new(
                ShellErrorCode::Revoked,
                "the approval for this executable was revoked",
            ),
            DenialReason::ShadowedPath { .. } => Self::new(
                ShellErrorCode::ShadowedPath,
                "the approved path now resolves somewhere else",
            ),
            DenialReason::ChangedSinceApproval { recorded, observed } => Self::with_detail(
                ShellErrorCode::ChangedSinceApproval,
                "the executable's content has changed since it was approved",
                ShellErrorDetail::ChangedSinceApproval {
                    recorded_sha256_short: recorded.short_hex(),
                    observed_sha256_short: observed.short_hex(),
                },
            ),
            DenialReason::ProbeFailed(ref outcome) => Self::from_probe_failure(outcome),
        }
    }
}

impl From<LaunchPlanError> for ShellError {
    /// Maps a `HarnessAdapter::build_launch` failure to its catalogue
    /// error (`docs/spike-log.md` § Slice 3). `CwdOutsideWorkspace` and
    /// `PromptChannelUnsupported` are both structurally unreachable in
    /// this slice's own wired flow (every built-in adapter always uses
    /// the workspace's own path as cwd, and each declares exactly the
    /// prompt channel its own `build_launch` implements) -- mapped
    /// defensively rather than left a `todo!()`, so this stays total.
    /// `PromptNotTypeable` is reachable: a `pty-cli` launch whose prompt
    /// carries a control character the terminal's line discipline would
    /// interpret is refused by `PtyCli::build_launch` (spike slice 4).
    fn from(error: LaunchPlanError) -> Self {
        match error {
            LaunchPlanError::SecretShapedEnvKey(_) => Self::new(
                ShellErrorCode::SecretShapedEnv,
                "the adapter declared an environment variable that looks secret-shaped",
            ),
            LaunchPlanError::CwdOutsideWorkspace => Self::new(
                ShellErrorCode::WorkspaceUnavailable,
                "the working directory resolves outside the workspace",
            ),
            LaunchPlanError::PromptChannelUnsupported => Self::new(
                ShellErrorCode::InvalidRequest,
                "this adapter's prompt channel is not supported",
            ),
            LaunchPlanError::PromptNotTypeable => Self::new(
                ShellErrorCode::PromptNotTypeable,
                "prompt contains control characters a terminal would interpret",
            ),
            // Structurally unreachable in the wired flow (every adapter
            // plan is an allowlist); mapped defensively so this stays total.
            LaunchPlanError::AssignmentOnInheritedEnv => Self::new(
                ShellErrorCode::InvalidRequest,
                "this launch plan cannot declare an output directory",
            ),
        }
    }
}

impl From<PrepareError> for ShellError {
    /// Maps every run-subdirectory preparation failure to
    /// `outbox-unavailable` (HAP-001-R10, D14): a launch without a
    /// verified output location is refused, whichever step failed, and an
    /// invalid declaration blocks every launch under it (HAP-001-R8). The
    /// message names the step class, never a path or the underlying
    /// error's text.
    fn from(error: PrepareError) -> Self {
        let message = match error {
            PrepareError::Declaration(_) => {
                "the declared outbox is invalid or unavailable, so no run subdirectory can be \
                 declared"
            }
            PrepareError::OutboxMissing | PrepareError::OutboxUnopenable => {
                "the outbox could not be created or opened"
            }
            PrepareError::AlreadyExists => {
                "something already sits at the run subdirectory path; remove it and relaunch"
            }
            PrepareError::CreateFailed | PrepareError::OpenFailed => {
                "the run subdirectory could not be created"
            }
            PrepareError::Verification(_) => "the run subdirectory could not be verified by handle",
        };
        Self::new(ShellErrorCode::OutboxUnavailable, message)
    }
}

impl From<PolicyError> for ShellError {
    /// Maps a policy that cannot be loaded to `outbox-invalid`: the
    /// declaration itself is unknowable, so ingestion is blocked
    /// (HAP-001-R8, R40).
    fn from(_: PolicyError) -> Self {
        Self::new(
            ShellErrorCode::OutboxInvalid,
            "the classification policy could not be loaded",
        )
    }
}

impl From<OutboxDeclarationError> for ShellError {
    /// Maps a declaration failure to its HAP-001 token: `outbox-invalid`
    /// for a link or an escape, `outbox-unavailable` for a non-directory.
    fn from(error: OutboxDeclarationError) -> Self {
        match error.failure() {
            OutboxFailure::OutboxInvalid => Self::new(
                ShellErrorCode::OutboxInvalid,
                "the declared outbox resolves outside the project or is a link",
            ),
            OutboxFailure::OutboxUnavailable => Self::new(
                ShellErrorCode::OutboxUnavailable,
                "the declared outbox is not a readable directory",
            ),
        }
    }
}

impl From<PromptError> for ShellError {
    /// Maps an `AgentPrompt::new` validation failure to its catalogue
    /// error.
    fn from(error: PromptError) -> Self {
        match error {
            PromptError::TooLarge => Self::new(
                ShellErrorCode::PromptTooLarge,
                "the prompt exceeds the size limit",
            ),
            PromptError::ContainsNul => Self::new(
                ShellErrorCode::InvalidRequest,
                "the prompt contains an interior NUL byte",
            ),
        }
    }
}

/// The accepted range for `harness_stop`'s `deadline_ms`, inclusive on both
/// ends. A caller-supplied deadline is IPC-controlled input like any other
/// argument, and an unbounded value would let a single `harness_stop` call
/// block its blocking-thread slot indefinitely (`docs/spike-log.md` § IPC
/// contract).
const MIN_DEADLINE_MS: u64 = 1;
const MAX_DEADLINE_MS: u64 = 30_000;

/// Validate `deadline_ms` into a bounded [`Duration`], or a catalogue
/// [`ShellError`] if it falls outside `1..=30_000`.
fn validate_deadline_ms(deadline_ms: u64) -> Result<Duration, ShellError> {
    if (MIN_DEADLINE_MS..=MAX_DEADLINE_MS).contains(&deadline_ms) {
        Ok(Duration::from_millis(deadline_ms))
    } else {
        Err(ShellError::new(
            ShellErrorCode::InvalidRequest,
            "deadlineMs must be between 1 and 30000",
        ))
    }
}

/// Run `f` against a cloned handle to the managed supervisor, on a blocking
/// thread, so a blocking `ProcessSupervisor` call (`spawn`/`stop`/
/// `subscribe`) never stalls the async executor.
///
/// `TokioProcessSupervisor` is itself a cheap `Clone` handle over shared
/// state (`omnifrons_supervisor`'s own doc comment), managed with no outer
/// `Mutex`: cloning it here, once per command, is what lets an unrelated
/// command (say, `harness_observe` of a different process) run concurrently
/// with a long-running `harness_stop` instead of queuing behind it -- a
/// single shared `Mutex<TokioProcessSupervisor>` would otherwise hold the
/// lock for that call's entire, deadline-bounded duration.
///
/// Takes `app` rather than a `State<'_, _>` directly: `State` borrows from
/// the app and is not `'static`, so it cannot be moved into
/// `spawn_blocking`'s closure -- fetching it again, inside the closure,
/// via `AppHandle::state` is the documented way to move managed state onto
/// another thread (Tauri's state-management docs: "if you need to move
/// the state into a thread where using an `AppHandle` is easier").
///
/// # Panics
///
/// Panics if the blocking task itself panics.
async fn with_supervisor<T, F>(app: AppHandle, f: F) -> T
where
    F: FnOnce(&mut TokioProcessSupervisor) -> T + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let mut supervisor = app.state::<TokioProcessSupervisor>().inner().clone();
        f(&mut supervisor)
    })
    .await
    .expect("the blocking supervisor task panicked")
}

/// Start a detached forwarder thread that relays every captured frame from
/// `receiver` onto `on_frame` until the channel closes (the frontend
/// navigated away or dropped it) or the process's output channel itself
/// closes (the process is confirmed terminal and fully drained). Shared
/// by both the demo-harness and approved-launch spawn paths.
fn forward_output(id: ProcessId, receiver: Receiver<OutputFrame>, on_frame: Channel<HarnessFrame>) {
    let id_dto = ProcessIdDto::from(id);
    std::thread::spawn(move || {
        let mut send_failures = 0u32;
        for frame in receiver {
            if on_frame
                .send(HarnessFrame::from_domain(id_dto, frame))
                .is_err()
            {
                send_failures += 1;
                break;
            }
        }
        if send_failures > 0 {
            tracing::warn!(
                pid = id_dto.0,
                send_failures,
                "harness output forwarder stopped after a channel send failure"
            );
        }
    });
}

/// Start a detached forwarder thread for an adapter launch: every captured
/// raw frame is mapped to the zero, one, or several `event`/`state` wire
/// frames it produces, which are then relayed onto `on_frame` in order
/// (`docs/spike-log.md` § Slice 3: `stdout`/`stderr` raw frames are never
/// emitted for an adapter launch). The mapping is chosen by the adapter's
/// transport class: [`adapter_wire_frames`] (a `LineAssembler` plus the
/// adapter's own `parse_line`) for `StructuredStreamingCli`, or
/// [`pty_wire_frames`] (a `TerminalNormalizer` over the raw text) for `Pty`
/// (`docs/spike-log.md` § Slice 4).
///
/// `adapter_id` is looked up in the managed [`AdapterState`] catalog
/// inside this thread (via a cloned `app`), never carried in as a
/// borrowed reference: this thread outlives the command call that started
/// it.
///
/// # Panics
///
/// Panics if `adapter_id` is not present in the managed catalog -- callers
/// must validate it against the catalog before ever reaching this
/// function.
fn forward_adapter_output(
    id: ProcessId,
    adapter_id: AdapterId,
    receiver: Receiver<OutputFrame>,
    on_frame: Channel<HarnessFrame>,
    app: AppHandle,
) {
    let id_dto = ProcessIdDto::from(id);
    std::thread::spawn(move || {
        let adapter_state = app.state::<AdapterState>();
        let adapter = adapter_state.catalog.get(&adapter_id).expect(
            "adapter_id must already have been validated against the catalog before spawning",
        );

        // Every adapter launch prepared a run subdirectory before spawn
        // (`spawn_adapter_harness`), so the sequencer records the run's
        // proposals and inventories the subdirectory at run end
        // (`docs/spike-log.md` § Slice 5).
        let outbox_state = app.state::<OutboxState>();
        let tracker =
            RunEndTracker::new(Arc::clone(&outbox_state.runs), id, outbox_state.inventory);
        let mut sequencer = AdapterWireSequencer::with_outbox(id_dto, tracker);
        match forwarder_for(adapter.describe().transport_class) {
            ForwarderKind::Line => {
                let mut assembler = LineAssembler::new();
                relay_wire_frames(id_dto, receiver, &on_frame, |frame| {
                    adapter_wire_frames(adapter, &mut assembler, &mut sequencer, frame)
                });
            }
            ForwarderKind::Pty => {
                let mut forwarder = PtyForwarder::new();
                relay_wire_frames(id_dto, receiver, &on_frame, |frame| {
                    pty_wire_frames(&mut forwarder, &mut sequencer, frame)
                });
            }
        }
    });
}

/// The two ways [`forward_adapter_output`] can map a launch's raw frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForwarderKind {
    /// Slice 3's line assembler plus the adapter's own `parse_line`.
    Line,
    /// Slice 4's byte normalizer over a [`PtyForwarder`].
    Pty,
}

/// Which forwarder a launch gets, decided by the adapter's transport class
/// alone -- pure, so the dispatch [`forward_adapter_output`] performs is
/// testable without Tauri state or a live process.
const fn forwarder_for(transport: TransportClass) -> ForwarderKind {
    match transport {
        TransportClass::StructuredStreamingCli => ForwarderKind::Line,
        TransportClass::Pty => ForwarderKind::Pty,
    }
}

/// Relay every raw frame of `receiver` through `map` onto `on_frame`, in
/// order, until the receiver closes (the process is confirmed terminal and
/// fully drained) or a send fails (the frontend navigated away or dropped
/// the channel).
fn relay_wire_frames(
    id_dto: ProcessIdDto,
    receiver: Receiver<OutputFrame>,
    on_frame: &Channel<HarnessFrame>,
    mut map: impl FnMut(OutputFrame) -> Vec<HarnessFrame>,
) {
    let mut send_failures = 0u32;
    'frames: for frame in receiver {
        for wire_frame in map(frame) {
            if on_frame.send(wire_frame).is_err() {
                send_failures += 1;
                break 'frames;
            }
        }
    }
    if send_failures > 0 {
        tracing::warn!(
            pid = id_dto.0,
            send_failures,
            "adapter output forwarder stopped after a channel send failure"
        );
    }
}

/// The per-launch state of the PTY forwarder (`docs/spike-log.md` §
/// Slice 4): the normalizer whose state persists across raw frames (a
/// sequence or a multi-byte character may straddle two frames), and the
/// `dropped_before` accumulated from raw frames that produced no wire
/// frame yet, so a drop count is never lost to a silent frame.
struct PtyForwarder {
    normalizer: TerminalNormalizer,
    pending_dropped_before: u64,
}

impl PtyForwarder {
    fn new() -> Self {
        Self {
            normalizer: TerminalNormalizer::new(),
            pending_dropped_before: 0,
        }
    }

    /// The `dropped_before` for the next wire frame: everything pending,
    /// then zero for the frames after it.
    fn take_dropped_before(&mut self) -> u64 {
        std::mem::take(&mut self.pending_dropped_before)
    }
}

/// Map one captured raw frame of a PTY launch to the wire frames it
/// produces, in order -- pure over `forwarder`/`sequencer`, so the
/// forwarder's behavior is unit-testable without a live process:
///
/// - a `stdout` text frame's text (plus a trailing `\n` unless
///   `continued`, restoring the line end the supervisor's framing
///   consumed) is pushed through the normalizer; each `Text` chunk becomes
///   a `terminal-text` event (adjacent text chunks coalesced), each action
///   a `terminal-action` event, and, when the drop counts changed, exactly
///   one `terminal-drops` event closes the frame;
/// - a `stderr` text frame becomes one `Diagnostic` event exactly as on the
///   line path (a terminal has no stderr: the only such frame is the
///   supervisor's own synthetic `stdin write failed`);
/// - a `State` frame first finishes the normalizer (a sequence still open
///   is malformed and reported as drops), then becomes the `state` frame.
///
/// The raw frame's `dropped_before` (summed with whatever earlier frames
/// left pending) rides on the first wire frame produced, `0` on the rest.
fn pty_wire_frames(
    forwarder: &mut PtyForwarder,
    sequencer: &mut AdapterWireSequencer,
    frame: OutputFrame,
) -> Vec<HarnessFrame> {
    forwarder.pending_dropped_before = forwarder
        .pending_dropped_before
        .saturating_add(frame.dropped_before);

    let (chunks, terminal) = match frame.payload {
        omnifrons_app::FramePayload::Text {
            stream: omnifrons_app::OutputStream::Stdout,
            text,
            continued,
        } => {
            let mut bytes = text.into_bytes();
            if !continued {
                bytes.push(b'\n');
            }
            (forwarder.normalizer.push(&bytes), None)
        }
        omnifrons_app::FramePayload::Text {
            stream: omnifrons_app::OutputStream::Stderr,
            text,
            continued: _,
        } => {
            let dropped_before = forwarder.take_dropped_before();
            return vec![sequencer.event(dropped_before, AdapterEvent::Diagnostic { text })];
        }
        omnifrons_app::FramePayload::State(state) => (forwarder.normalizer.finish(), Some(state)),
    };

    let mut events: Vec<AdapterEvent> = Vec::new();
    for chunk in chunks {
        match chunk {
            TerminalChunk::Text(text) => match events.last_mut() {
                Some(AdapterEvent::TerminalText { text: previous }) => previous.push_str(&text),
                _ => events.push(AdapterEvent::TerminalText { text }),
            },
            TerminalChunk::Action(action) => events.push(AdapterEvent::TerminalAction(action)),
        }
    }
    let drops = forwarder.normalizer.take_drops();
    if !drops.is_zero() {
        events.push(AdapterEvent::TerminalDrops(drops));
    }

    let mut frames: Vec<HarnessFrame> = events
        .into_iter()
        .map(|event| {
            let dropped_before = forwarder.take_dropped_before();
            sequencer.event(dropped_before, event)
        })
        .collect();
    if let Some(state) = terminal {
        let dropped_before = forwarder.take_dropped_before();
        frames.extend(sequencer.state(dropped_before, state));
    }
    frames
}

/// The run-end side of one adapter launch's outbox (spike slice 5,
/// HAP-001-R11, R12): records every `artifact.publish` proposal the run
/// surfaces, and at the terminal state inventories the run subdirectory
/// through the held handle, attributes each entry by digest against those
/// proposals, classifies it, caps the held handles (D22), stores the
/// candidates on the run's record, and reports the summary.
struct RunEndTracker {
    runs: Arc<Mutex<RunTable>>,
    id: ProcessId,
    inventory: FsOutboxInventory,
}

impl RunEndTracker {
    fn new(runs: Arc<Mutex<RunTable>>, id: ProcessId, inventory: FsOutboxInventory) -> Self {
        Self {
            runs,
            id,
            inventory,
        }
    }

    /// Remember one proposal the run surfaced (a proposal only; the
    /// digests it names are compared, never trusted).
    fn record_proposal(&self, proposal: PublishProposal) {
        if let Some(record) = self
            .runs
            .lock()
            .expect("run table mutex poisoned by a prior panic")
            .get_mut(self.id)
        {
            record.push_proposal(proposal);
        }
    }

    /// The run ended: inventory its subdirectory and produce, in order,
    /// one `diagnostic` for proposed entries counted beyond the per-run
    /// cap, one `diagnostic` per recorded proposal naming a digest no entry
    /// carries, the `candidates` summary, and -- only if the record was
    /// gone by the time the inventory completed -- one `diagnostic` saying
    /// how many candidates were not retained. Empty for a launch with no
    /// record. Two phases under two lock scopes ([`Self::snapshot`], then
    /// the inventory on a duplicate of the held handle outside the lock,
    /// then [`Self::conclude`]), so a concurrent `candidates_list` never
    /// waits on file hashing.
    fn finish(&self) -> Vec<AdapterEventDto> {
        match self.snapshot() {
            RunEndPhase::NoRecord => Vec::new(),
            RunEndPhase::HandleUnavailable => vec![inventory_failed_diagnostic()],
            RunEndPhase::Ready(input) => self.conclude(*input),
        }
    }

    /// Phase one, under the table's lock: what the inventory needs from
    /// the run's record -- a duplicate of the held handle, the policy, the
    /// proposals, the run id, and the proposal-cap counts.
    fn snapshot(&self) -> RunEndPhase {
        let table = self
            .runs
            .lock()
            .expect("run table mutex poisoned by a prior panic");
        let Some(record) = table.get(self.id) else {
            return RunEndPhase::NoRecord;
        };
        match record.subdirectory_clone() {
            Ok(subdirectory) => RunEndPhase::Ready(Box::new(RunEndInput {
                subdirectory,
                policy: record.policy().clone(),
                proposals: record.proposals().to_vec(),
                run_id: record.run_id().clone(),
                recorded_entries: record.recorded_proposal_entries(),
                dropped_entries: record.dropped_proposal_entries(),
            })),
            Err(_) => RunEndPhase::HandleUnavailable,
        }
    }

    /// Phase two: inventory, attribute, classify, summarize, then store
    /// through the table (where the D22 cap is applied). If the record was
    /// evicted or forgotten in between, the table hands the candidates
    /// back: their handles are released here, explicitly, and the loss is
    /// reported as a diagnostic naming the run id and the count -- never
    /// silently.
    fn conclude(&self, input: RunEndInput) -> Vec<AdapterEventDto> {
        let RunEndInput {
            subdirectory,
            policy,
            proposals,
            run_id,
            recorded_entries,
            dropped_entries,
        } = input;
        let Ok(entries) = self
            .inventory
            .inventory(&subdirectory.handle, &subdirectory.path)
        else {
            return vec![inventory_failed_diagnostic()];
        };
        let assembled = assemble_run_candidates(&run_id, entries, &proposals, &policy);
        let summary = summarize(&assembled);

        let mut events: Vec<AdapterEventDto> = Vec::new();
        if dropped_entries > 0 {
            events.push(AdapterEventDto::Diagnostic {
                text: format!(
                    "artifact.publish named {dropped_entries} entries beyond the \
                     {recorded_entries} recorded for this run (the cap is \
                     {MAX_PROPOSED_ENTRIES} per run); they were not recorded and took no part \
                     in attribution"
                ),
            });
        }
        events.extend(assembled.unmatched_proposals.iter().map(|entry| {
            AdapterEventDto::Diagnostic {
                text: format!(
                    "artifact.publish names {} by a digest not found in the run subdirectory ({})",
                    entry.name,
                    entry.sha256.short_hex()
                ),
            }
        }));
        events.push(AdapterEventDto::Candidates {
            run_id: run_id.as_str().to_string(),
            total: summary.total,
            candidate: summary.candidate,
            outbox_escape: summary.outbox_escape,
            outbox_linked: summary.outbox_linked,
            attributed: summary.attributed,
            unattributed: summary.unattributed,
            unreadable: summary.unreadable,
            unmatched_proposals: summary.unmatched_proposals,
        });

        // The handle cap (HAP-001 D22) is applied by the table, under its
        // lock, against every other remembered run's held handles.
        let stored = self
            .runs
            .lock()
            .expect("run table mutex poisoned by a prior panic")
            .store_candidates(self.id, assembled.candidates);
        if let Err(not_retained) = stored {
            let count = not_retained.len();
            drop(not_retained);
            events.push(AdapterEventDto::Diagnostic {
                text: format!(
                    "the record of run {run_id} was no longer remembered when its inventory \
                     completed; {count} candidates were not retained and their handles were \
                     released"
                ),
            });
        }
        events
    }
}

/// What [`RunEndTracker::snapshot`] captured under the table's lock for
/// the inventory that follows outside it.
#[derive(Debug)]
struct RunEndInput {
    subdirectory: PreparedRunSubdirectory,
    policy: OutboxPolicy,
    proposals: Vec<PublishProposal>,
    run_id: RunId,
    recorded_entries: usize,
    dropped_entries: u64,
}

/// The outcome of [`RunEndTracker::snapshot`].
#[derive(Debug)]
enum RunEndPhase {
    /// The launch has no record (never an adapter launch that prepared an
    /// outbox, or already forgotten): nothing to inventory.
    NoRecord,
    /// The held handle could not be duplicated: reported, not inventoried.
    HandleUnavailable,
    /// Everything the inventory needs.
    Ready(Box<RunEndInput>),
}

/// The fixed diagnostic for a run subdirectory that could not be
/// inventoried at run end; never the underlying error's text.
fn inventory_failed_diagnostic() -> AdapterEventDto {
    AdapterEventDto::Diagnostic {
        text: "the run subdirectory could not be inventoried".to_string(),
    }
}

/// The shell's own `seq` for one adapter launch's wire frames: a
/// contiguous count of the `event`/`state` frames actually emitted for
/// that launch, starting at `0`.
///
/// Deliberately not the captured raw frame's own `seq`: a raw frame can
/// produce zero wire frames (a continuation still being assembled), one,
/// or several (R3-004: a line carrying several content blocks yields one
/// `event` per block), so raw `seq` would leave gaps and duplicates on the
/// event stream -- and the renderer keys transcript entries by
/// `${id}-${seq}`, so a duplicate would collide. Keeping the wire `seq`
/// contiguous per launch is what `docs/spike-log.md` § Slice 3's own wire
/// examples already show.
///
/// With a [`RunEndTracker`] attached (every adapter launch; spike slice
/// 5), every `ArtifactPublish` event passing through is recorded, and the
/// terminal `state` frame is preceded by the tracker's run-end events --
/// so `state` stays the last frame of the launch and `seq` stays
/// contiguous across them.
struct AdapterWireSequencer {
    id: ProcessIdDto,
    next_seq: u64,
    tracker: Option<RunEndTracker>,
}

impl AdapterWireSequencer {
    /// A sequencer with no run tracker. Test-only: every production
    /// adapter launch prepares a run subdirectory and attaches a tracker
    /// ([`Self::with_outbox`]), so no production path needs one.
    #[cfg(test)]
    fn new(id: ProcessIdDto) -> Self {
        Self {
            id,
            next_seq: 0,
            tracker: None,
        }
    }

    fn with_outbox(id: ProcessIdDto, tracker: RunEndTracker) -> Self {
        Self {
            id,
            next_seq: 0,
            tracker: Some(tracker),
        }
    }

    fn next_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    fn event(&mut self, dropped_before: u64, event: AdapterEvent) -> HarnessFrame {
        if let (Some(tracker), AdapterEvent::ArtifactPublish(proposal)) = (&self.tracker, &event) {
            tracker.record_proposal(proposal.clone());
        }
        let seq = self.next_seq();
        HarnessFrame::event(self.id, seq, dropped_before, event)
    }

    /// The frames the raw `State` frame produces: the tracker's run-end
    /// events, if any, then the terminal `state` frame, always last. The
    /// raw frame's `dropped_before` rides on the first of them.
    fn state(&mut self, dropped_before: u64, state: ProcessTerminalState) -> Vec<HarnessFrame> {
        let run_end = self
            .tracker
            .as_ref()
            .map(RunEndTracker::finish)
            .unwrap_or_default();
        let mut dropped_before = Some(dropped_before);
        let mut frames: Vec<HarnessFrame> = run_end
            .into_iter()
            .map(|event| {
                let seq = self.next_seq();
                HarnessFrame::event_dto(self.id, seq, dropped_before.take().unwrap_or(0), event)
            })
            .collect();
        let seq = self.next_seq();
        frames.push(HarnessFrame::State {
            id: self.id,
            seq,
            dropped_before: dropped_before.take().unwrap_or(0),
            terminal: state.into(),
        });
        frames
    }
}

/// Map one captured raw frame of an adapter launch to the wire frames it
/// produces, in order -- pure over `assembler`/`sequencer`, so the
/// forwarder's behavior is unit-testable without a live process:
///
/// - a stdout text frame is fed to `assembler`; nothing is emitted until
///   it completes a line, and then `adapter.parse_line`'s events (or one
///   `Unknown { truncated: true }` for a truncated line) each become an
///   `event` frame, the line's summed `dropped_before` (R3-002) attached
///   to the first of them and `0` to the rest;
/// - a stderr text frame becomes one `Diagnostic` `event` frame with the
///   raw frame's own `dropped_before`;
/// - a `State` frame first flushes any partial line still pending in
///   `assembler` (its terminator can no longer arrive) as
///   `Unknown { truncated: true }`, then becomes the `state` frame.
fn adapter_wire_frames(
    adapter: &dyn omnifrons_app::HarnessAdapter,
    assembler: &mut LineAssembler,
    sequencer: &mut AdapterWireSequencer,
    frame: OutputFrame,
) -> Vec<HarnessFrame> {
    match frame.payload {
        omnifrons_app::FramePayload::Text {
            stream: omnifrons_app::OutputStream::Stdout,
            text,
            continued,
        } => assembler
            .push(&text, continued, frame.dropped_before)
            .map_or_else(Vec::new, |assembled| {
                assembled_wire_frames(adapter, sequencer, assembled)
            }),
        omnifrons_app::FramePayload::Text {
            stream: omnifrons_app::OutputStream::Stderr,
            text,
            continued: _,
        } => vec![sequencer.event(frame.dropped_before, AdapterEvent::Diagnostic { text })],
        omnifrons_app::FramePayload::State(state) => {
            let mut frames = assembler.finish().map_or_else(Vec::new, |pending| {
                assembled_wire_frames(adapter, sequencer, pending)
            });
            frames.extend(sequencer.state(frame.dropped_before, state));
            frames
        }
    }
}

/// The `event` wire frames for one completed line: one per event, the
/// line's summed `dropped_before` on the first only. Defensive against an
/// adapter violating `parse_line`'s never-empty contract: an empty event
/// list is replaced by one `Unknown` carrying the line's bytes, so the
/// line (and its drop count) is never silently lost.
fn assembled_wire_frames(
    adapter: &dyn omnifrons_app::HarnessAdapter,
    sequencer: &mut AdapterWireSequencer,
    assembled: omnifrons_app::Assembled,
) -> Vec<HarnessFrame> {
    let omnifrons_app::Assembled {
        line,
        dropped_before,
    } = assembled;
    let events = match line {
        omnifrons_app::AssembledLine::Line(line) => {
            let mut events = adapter.parse_line(&line);
            if events.is_empty() {
                events.push(AdapterEvent::Unknown {
                    raw: line.into_bytes(),
                    truncated: false,
                });
            }
            events
        }
        omnifrons_app::AssembledLine::Truncated { raw } => vec![AdapterEvent::Unknown {
            raw,
            truncated: true,
        }],
    };
    events
        .into_iter()
        .enumerate()
        .map(|(index, event)| {
            let dropped_before = if index == 0 { dropped_before } else { 0 };
            sequencer.event(dropped_before, event)
        })
        .collect()
}

/// Spawn a harness or an approved/adapter executable, and stream its
/// captured output over `on_frame`. `kind` names one of the two synthetic
/// demo behaviors, a real, previously approved executable
/// (`docs/spike-log.md` § Slice 2), or a built-in adapter launch
/// (`docs/spike-log.md` § Slice 3) -- see [`HarnessKindDto`]'s own doc
/// comment for why the wire shape is tagged rather than flat.
///
/// # Errors
///
/// Returns [`ShellError`] with [`ShellErrorCode::InvalidRequest`] if a demo
/// kind's `rateHz`/`lines` are out of range, the mapped [`SupervisorError`]
/// if the process could not be started, the mapped [`DenialReason`] if the
/// `LaunchGate` denies an `"approved"`/`"adapter"` launch, or -- for
/// `kind: "adapter"` specifically -- [`ShellErrorCode::UnknownAdapter`],
/// [`ShellErrorCode::PromptTooLarge`]/`invalid-request`,
/// [`ShellErrorCode::WorkspaceUnavailable`], or the mapped
/// [`LaunchPlanError`].
#[tauri::command]
pub async fn harness_spawn(
    app: AppHandle,
    kind: HarnessKindDto,
    on_frame: Channel<HarnessFrame>,
) -> Result<ProcessIdDto, ShellError> {
    match kind {
        HarnessKindDto::DemoLines { rate_hz, lines } => {
            spawn_demo_harness(app, HarnessKind::DemoLines, rate_hz, lines, on_frame).await
        }
        HarnessKindDto::DemoIgnoresSigterm { rate_hz, lines } => {
            spawn_demo_harness(
                app,
                HarnessKind::DemoIgnoresSigterm,
                rate_hz,
                lines,
                on_frame,
            )
            .await
        }
        HarnessKindDto::Approved { approval_id } => {
            spawn_approved_harness(app, approval_id.into(), on_frame).await
        }
        HarnessKindDto::Adapter {
            adapter_id,
            approval_id,
            prompt,
        } => spawn_adapter_harness(app, adapter_id, approval_id.into(), prompt, on_frame).await,
    }
}

/// Validates `rate_hz`/`lines` into an `HarnessRequest` for `kind`, spawns
/// and subscribes on a blocking thread, then starts the forwarder thread.
async fn spawn_demo_harness(
    app: AppHandle,
    kind: HarnessKind,
    rate_hz: u16,
    lines: u32,
    on_frame: Channel<HarnessFrame>,
) -> Result<ProcessIdDto, ShellError> {
    let request = HarnessRequest::new(kind, rate_hz, lines)?;

    let (id, receiver) = with_supervisor(app, move |supervisor| {
        let id = supervisor.spawn_harness(&request)?;
        let receiver = supervisor.subscribe(id)?;
        Ok::<_, SupervisorError>((id, receiver))
    })
    .await?;

    forward_output(id, receiver, on_frame);
    Ok(ProcessIdDto::from(id))
}

/// Decide (via the managed `LaunchGate`) whether `approval_id` may launch
/// right now, and if so, spawn *the exact `ProbedExecutable` that decision
/// re-probed* with no arguments and stream its captured output over
/// `on_frame`.
///
/// Unlike an earlier version of this function, there is no separate
/// approval-store lookup for the canonical path once `decide` reports
/// `Allowed`: the decision itself already carries the identity and open
/// handle its own re-probe produced
/// (`omnifrons_app::launch_gate::GateDecision::Allowed`), so this hands
/// that straight to `spawn_approved` rather than re-opening the path a
/// second time by name, which would reopen a TOCTOU window `decide`'s own
/// re-probe had just closed.
///
/// # Errors
///
/// Returns the mapped [`DenialReason`] if the gate denies the launch, or
/// the mapped [`SupervisorError`] if the process could not be started.
async fn spawn_approved_harness(
    app: AppHandle,
    approval_id: ApprovalId,
    on_frame: Channel<HarnessFrame>,
) -> Result<ProcessIdDto, ShellError> {
    let decision = with_gate(app.clone(), move |gate| gate.decide(approval_id)).await;
    let executable = match decision {
        GateDecision::Allowed { executable, .. } => executable,
        GateDecision::Denied(reason) => return Err(ShellError::from(reason)),
    };

    let display_path = executable.identity.canonical_path.clone();
    let plan = default_approved_plan()?;
    let (id, receiver) = with_supervisor(app, move |supervisor| {
        let id = supervisor.spawn_approved(executable.handle, display_path, &plan)?;
        let receiver = supervisor.subscribe(id)?;
        Ok::<_, SupervisorError>((id, receiver))
    })
    .await?;

    forward_output(id, receiver, on_frame);
    Ok(ProcessIdDto::from(id))
}

/// The plan `kind: "approved"` launches with: empty argv (no arguments,
/// unchanged from the spike slice-2 spike), the same `env_clear()` plus
/// base-allowlist environment an adapter launch with no declared extras
/// gets (`EnvPlan::new(&[])` -- never `EnvPlan::Inherit`: an approved
/// executable is a caller-supplied program exactly like an adapter's, and
/// this shell's own environment can carry credentials no launch should
/// inherit by default; R1-001, fixed in review), no working-directory
/// override (this shell's own current directory, behaviorally identical
/// to never setting one), and `StdinPlan::Null` -- which was always this
/// method's own documented contract, even before `LaunchPlan` existed, and
/// this now actually enforces on every platform (`docs/spike-log.md` §
/// Slice 3). Both kinds then pass through `finish_spawn`'s single
/// secret-shaped re-check.
///
/// # Errors
///
/// Returns [`ShellError`] if this shell's own current directory could not
/// be resolved into a [`WorkspaceRoot`] -- expected never to happen in
/// practice -- or the mapped [`LaunchPlanError`] if the base allowlist
/// could not be built (structurally impossible: it declares no extra key).
fn default_approved_plan() -> Result<LaunchPlan, ShellError> {
    let cwd = std::env::current_dir().map_err(|_| {
        ShellError::new(
            ShellErrorCode::WorkspaceUnavailable,
            "this shell's own current directory could not be resolved",
        )
    })?;
    let workspace = WorkspaceRoot::new(cwd).map_err(|_| {
        ShellError::new(
            ShellErrorCode::WorkspaceUnavailable,
            "this shell's own current directory could not be resolved",
        )
    })?;
    let env = EnvPlan::new(&[])?;
    Ok(LaunchPlan {
        argv: vec![],
        env,
        cwd: workspace,
        stdin: StdinPlan::Null,
        prompt: None,
        scope_mode: ScopeMode::Advisory,
        transport: TransportClass::StructuredStreamingCli,
        // A plain approved launch declares no run subdirectory: only an
        // adapter launch prepares one (spike slice 5, HAP-001-R9 binds
        // every *adapter* launch plan).
        output_dir: None,
    })
}

/// Validate `adapter_id`/`prompt` and an active workspace, decide (via the
/// managed `LaunchGate`) whether `approval_id` may launch right now, build
/// that adapter's own [`LaunchPlan`] for the current workspace and prompt,
/// and spawn it -- streaming captured output over `on_frame` through
/// [`forward_adapter_output`] rather than raw (`docs/spike-log.md` §
/// Slice 3).
///
/// # Errors
///
/// Returns [`ShellError`] with [`ShellErrorCode::UnknownAdapter`] if
/// `adapter_id` is not one of the closed, built-in adapters; the mapped
/// [`PromptError`] if `prompt` is invalid;
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active; the
/// mapped [`DenialReason`] if the `LaunchGate` denies the launch; the
/// mapped [`LaunchPlanError`] if the adapter's own `build_launch` fails;
/// or the mapped [`SupervisorError`] if the process could not be started.
async fn spawn_adapter_harness(
    app: AppHandle,
    adapter_id: String,
    approval_id: ApprovalId,
    prompt: String,
    on_frame: Channel<HarnessFrame>,
) -> Result<ProcessIdDto, ShellError> {
    let adapter_id = AdapterId::parse(&adapter_id).ok_or_else(|| {
        ShellError::new(
            ShellErrorCode::UnknownAdapter,
            "the requested adapter is not recognized",
        )
    })?;
    let prompt = AgentPrompt::new(prompt)?;

    let workspace = {
        let state = app.state::<AdapterState>();
        state
            .active_workspace
            .lock()
            .expect("active workspace mutex poisoned by a prior panic")
            .clone()
            .ok_or_else(|| {
                ShellError::new(
                    ShellErrorCode::WorkspaceUnavailable,
                    "no workspace has been picked yet",
                )
            })?
    };

    let decision = with_gate(app.clone(), move |gate| gate.decide(approval_id)).await;
    let executable = match decision {
        GateDecision::Allowed { executable, .. } => executable,
        GateDecision::Denied(reason) => return Err(ShellError::from(reason)),
    };

    let plan = {
        let state = app.state::<AdapterState>();
        let adapter = state
            .catalog
            .get(&adapter_id)
            .expect("adapter_id was already validated via AdapterId::parse against the catalog");
        let request = LaunchRequest {
            prompt,
            workspace: workspace.clone(),
        };
        adapter.build_launch(&request)?
    };

    // The run subdirectory is prepared before spawn and declared on the
    // plan (HAP-001-R9, R10; spike slice 5): the id is minted here, the
    // filesystem work runs on the blocking thread with the spawn itself,
    // and a failure at any step refuses the launch with
    // `outbox-unavailable` before anything is spawned (D14).
    let (store, preparer, run_id) = {
        let outbox_state = app.state::<OutboxState>();
        (
            outbox_state.policy_store,
            outbox_state.preparer,
            crate::outbox_state::mint_next_run_id(),
        )
    };
    let display_path = executable.identity.canonical_path.clone();
    let (launched, receiver) = with_supervisor(app.clone(), move |supervisor| {
        let launched =
            launch_with_run_subdirectory(&store, &preparer, &workspace, &run_id, plan, |plan| {
                supervisor.spawn_approved(executable.handle, display_path, plan)
            })?;
        let receiver = supervisor.subscribe(launched.id)?;
        Ok::<_, ShellError>((launched, receiver))
    })
    .await?;

    let id = launched.id;
    // The record keeps the launch's provenance -- the adapter and the
    // executable approval that governed it -- for the Catalog record of an
    // attributed entry (HAP-001-R11, R36; spike slice 5b).
    app.state::<OutboxState>()
        .runs
        .lock()
        .expect("run table mutex poisoned by a prior panic")
        .insert(
            id,
            RunRecord::with_provenance(
                launched.subdirectory,
                launched.policy,
                Some(adapter_id.clone()),
                Some(approval_id),
            ),
        );

    forward_adapter_output(id, adapter_id, receiver, on_frame, app);
    Ok(ProcessIdDto::from(id))
}

/// What a successful [`launch_with_run_subdirectory`] hands back: the
/// process id, the prepared run subdirectory with its held handle, and the
/// policy in force at launch.
#[derive(Debug)]
struct LaunchedRun {
    id: ProcessId,
    subdirectory: PreparedRunSubdirectory,
    policy: OutboxPolicy,
}

/// The launch sequence after the gate decision, as one plain function the
/// tests drive with a fake `spawn`: prepare the run subdirectory
/// ([`prepare_run`]), declare it on `plan` (HAP-001-R9), then `spawn`. If
/// the declaration or the spawn fails, the subdirectory just created is
/// removed again -- only if still empty
/// ([`remove_empty_run_subdirectory`]) -- and the error is returned with
/// nothing spawned; a failed preparation itself created nothing.
fn launch_with_run_subdirectory<E>(
    store: &dyn OutboxPolicyStore,
    preparer: &dyn RunOutboxPreparer,
    workspace: &WorkspaceRoot,
    run_id: &RunId,
    mut plan: LaunchPlan,
    spawn: impl FnOnce(&LaunchPlan) -> Result<ProcessId, E>,
) -> Result<LaunchedRun, ShellError>
where
    ShellError: From<E>,
{
    let (subdirectory, policy) = prepare_run(store, preparer, workspace, run_id)?;
    let launched = plan
        .declare_output_dir(subdirectory.path.clone())
        .map_err(<ShellError as From<LaunchPlanError>>::from)
        .and_then(|()| spawn(&plan).map_err(Into::into));
    match launched {
        Ok(id) => Ok(LaunchedRun {
            id,
            subdirectory,
            policy,
        }),
        Err(error) => {
            remove_empty_run_subdirectory(&subdirectory.path);
            Err(error)
        }
    }
}

/// Remove the run subdirectory a failed launch leaves behind, only while
/// it is still empty: `remove_dir` refuses a non-empty directory, so
/// nothing a same-user process dropped in after creation is ever deleted
/// (TM-001 A6). A refusal is logged by kind only, at debug level.
fn remove_empty_run_subdirectory(path: &std::path::Path) {
    if let Err(error) = std::fs::remove_dir(path) {
        tracing::debug!(
            kind = ?error.kind(),
            "the run subdirectory of a failed launch was left in place"
        );
    }
}

/// Load the project's policy and prepare `run_id`'s subdirectory under
/// its outbox (HAP-001-R8, R10). A policy that cannot be loaded blocks the
/// launch as `outbox-unavailable` (HAP-001-R10: an invalid outbox renders
/// `outbox-unavailable` at launch), and so does every preparation failure.
fn prepare_run(
    store: &dyn OutboxPolicyStore,
    preparer: &dyn RunOutboxPreparer,
    workspace: &WorkspaceRoot,
    run_id: &RunId,
) -> Result<(PreparedRunSubdirectory, OutboxPolicy), ShellError> {
    let policy = store.load(workspace).map_err(|_| {
        ShellError::new(
            ShellErrorCode::OutboxUnavailable,
            "the classification policy could not be loaded, so no run subdirectory can be declared",
        )
    })?;
    let subdirectory = preparer.prepare(workspace, policy.outbox(), run_id)?;
    Ok((subdirectory, policy))
}

/// The outbox's status for `project` (`outbox_status`, spike slice 5):
/// the policy's declaration validated against the project root, with the
/// canonical path shown only for a valid, existing outbox.
fn outbox_status_for(project: &WorkspaceRoot, store: JsonOutboxPolicyStore) -> OutboxStatusDto {
    let policy_path = POLICY_FILE_PATH.to_string();
    let policy = match store.load(project) {
        Ok(policy) => policy,
        Err(error) => {
            return OutboxStatusDto {
                declared: None,
                outbox: None,
                exists: false,
                state: OutboxStateTag::OutboxInvalid,
                reason: Some(match error {
                    PolicyError::Unreadable => OutboxReasonTag::PolicyUnreadable,
                    PolicyError::Corrupt => OutboxReasonTag::PolicyCorrupt,
                    PolicyError::Invalid(_)
                    | PolicyError::InvalidOutboxPath(_)
                    | PolicyError::TooLarge => OutboxReasonTag::PolicyInvalid,
                }),
                policy_path,
                asset_root_id: None,
            };
        }
    };
    let declared = Some(policy.outbox().to_string());
    // The destination shown before the decision (HAP-001-R22): the asset
    // root identity token, never a path.
    let asset_root_id = policy.asset_root_id().map(|id| id.as_str().to_string());
    match validate_outbox_declaration(project, policy.outbox()) {
        Ok(OutboxLocation::Present(canonical)) => OutboxStatusDto {
            declared,
            outbox: Some(canonical.to_string_lossy().into_owned()),
            exists: true,
            state: OutboxStateTag::Valid,
            reason: None,
            policy_path,
            asset_root_id,
        },
        Ok(OutboxLocation::Missing(_)) => OutboxStatusDto {
            declared,
            outbox: None,
            exists: false,
            state: OutboxStateTag::Valid,
            reason: None,
            policy_path,
            asset_root_id,
        },
        Err(error) => OutboxStatusDto {
            declared,
            outbox: None,
            exists: true,
            state: match error.failure() {
                OutboxFailure::OutboxInvalid => OutboxStateTag::OutboxInvalid,
                OutboxFailure::OutboxUnavailable => OutboxStateTag::OutboxUnavailable,
            },
            reason: Some(match error {
                OutboxDeclarationError::OutsideProject => OutboxReasonTag::OutsideProject,
                OutboxDeclarationError::IsLink => OutboxReasonTag::Link,
                OutboxDeclarationError::NotADirectory => OutboxReasonTag::NotADirectory,
                OutboxDeclarationError::Unreadable => OutboxReasonTag::Unreadable,
            }),
            policy_path,
            asset_root_id,
        },
    }
}

/// The whole-outbox inventory (`candidates_list` without a run id; the
/// session-start proposal of HAP-001-R12): every entry at the outbox root
/// and one level down, under a run subdirectory (D20), listed as an
/// unattributed candidate named relative to the outbox -- a proposal
/// only. A directory at the root whose name is not a run id, and any
/// deeper directory, is a non-regular entry (`outbox-escape`,
/// HAP-001-R15). No handle is held for these entries: publication (slice
/// 5b) re-opens each under HAP-001-R15 when its turn comes. An outbox
/// that does not exist yet is an empty inventory.
///
/// # Errors
///
/// `outbox-invalid` if the policy cannot be loaded or the declaration
/// resolves outside the project or is a link; `outbox-unavailable` if the
/// outbox cannot be opened or listed.
fn inventory_whole_outbox(
    project: &WorkspaceRoot,
    store: JsonOutboxPolicyStore,
    preparer: FsRunOutboxPreparer,
    inventory: FsOutboxInventory,
) -> Result<Vec<CandidateDto>, ShellError> {
    let policy = store.load(project)?;
    let outbox = match preparer.open_outbox(project, policy.outbox(), false) {
        Ok(outbox) => outbox,
        Err(PrepareError::OutboxMissing) => return Ok(Vec::new()),
        Err(PrepareError::Declaration(error)) => return Err(ShellError::from(error)),
        Err(error) => return Err(ShellError::from(error)),
    };
    let unavailable = || {
        ShellError::new(
            ShellErrorCode::OutboxUnavailable,
            "the outbox could not be listed",
        )
    };
    let entries = inventory
        .inventory(&outbox.handle, &outbox.path)
        .map_err(|_| unavailable())?;

    let mut root_entries: Vec<InventoriedEntry> = Vec::new();
    let mut subdirectories = Vec::new();
    for entry in entries {
        let run_id = RunId::new(entry.name.to_string_lossy()).ok();
        if let (CandidateProbe::Escape(EscapeReason::NotRegular), Some(run_id)) =
            (&entry.probe, run_id)
        {
            match inventory.open_subdirectory(&outbox.handle, &outbox.path, &entry.name) {
                Ok(handle) => subdirectories.push((run_id, handle)),
                Err(_) => root_entries.push(entry),
            }
        } else {
            root_entries.push(entry);
        }
    }

    let mut listed: Vec<CandidateDto> = Vec::new();
    let root = assemble_unattributed_candidates(None, root_entries, &policy);
    listed.extend(
        root.candidates
            .iter()
            .map(|candidate| CandidateDto::from_candidate(&candidate.entry, candidate.state)),
    );
    for (run_id, handle) in subdirectories {
        let path = outbox.path.join(run_id.as_str());
        let entries = inventory
            .inventory(&handle, &path)
            .map_err(|_| unavailable())?;
        let assembled = assemble_unattributed_candidates(Some(&run_id), entries, &policy);
        listed.extend(
            assembled
                .candidates
                .iter()
                .map(|candidate| CandidateDto::from_candidate(&candidate.entry, candidate.state)),
        );
    }
    Ok(listed)
}

/// The candidates `candidates_list` returns for one run named by `token`:
/// the inventory taken at that run's end. `invalid-request` for a token
/// that is not a run id, a run the table does not remember, or a run that
/// has not ended yet (no inventory taken).
fn candidates_for_run(table: &RunTable, token: &str) -> Result<Vec<CandidateDto>, ShellError> {
    let run_id = RunId::new(token)
        .map_err(|_| ShellError::new(ShellErrorCode::InvalidRequest, "the run id is not valid"))?;
    let record = table.find_by_run_id(&run_id).ok_or_else(|| {
        ShellError::new(
            ShellErrorCode::InvalidRequest,
            "no run with that id is remembered",
        )
    })?;
    let candidates = record.candidates().ok_or_else(|| {
        ShellError::new(
            ShellErrorCode::InvalidRequest,
            "the run has not ended yet, so its subdirectory has not been inventoried",
        )
    })?;
    Ok(candidates
        .iter()
        .map(|candidate| CandidateDto::from_candidate(&candidate.entry, candidate.state))
        .collect())
}

/// The active workspace, or `workspace-unavailable`.
pub(crate) fn active_workspace(app: &AppHandle) -> Result<WorkspaceRoot, ShellError> {
    app.state::<AdapterState>()
        .active_workspace
        .lock()
        .expect("active workspace mutex poisoned by a prior panic")
        .clone()
        .ok_or_else(|| {
            ShellError::new(
                ShellErrorCode::WorkspaceUnavailable,
                "no workspace has been picked yet",
            )
        })
}

/// The active workspace's outbox status (spike slice 5): the declared
/// path, whether it is valid, whether it exists, and -- for a valid,
/// existing outbox only -- its canonical path as identity evidence.
///
/// # Errors
///
/// Returns [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is
/// active.
#[tauri::command]
pub async fn outbox_status(app: AppHandle) -> Result<OutboxStatusDto, ShellError> {
    let workspace = active_workspace(&app)?;
    let store = app.state::<OutboxState>().policy_store;
    tauri::async_runtime::spawn_blocking(move || outbox_status_for(&workspace, store))
        .await
        .map_err(|_| {
            ShellError::new(
                ShellErrorCode::OutboxUnavailable,
                "the outbox status could not be computed",
            )
        })
}

/// The candidate entries of one run (`run_id` given: the inventory taken
/// at that run's end, attributed by the run's own proposals) or of the
/// whole outbox (`run_id` absent: every entry, unattributed, as a
/// proposal only). Never a device path.
///
/// # Errors
///
/// Returns [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is
/// active; [`ShellErrorCode::InvalidRequest`] if `run_id` names no
/// remembered run, or a run whose inventory has not been taken yet;
/// [`ShellErrorCode::OutboxInvalid`]/[`ShellErrorCode::OutboxUnavailable`]
/// if the whole-outbox inventory cannot run.
#[tauri::command]
pub async fn candidates_list(
    app: AppHandle,
    run_id: Option<String>,
) -> Result<Vec<CandidateDto>, ShellError> {
    if let Some(token) = run_id {
        {
            let outbox_state = app.state::<OutboxState>();
            let table = outbox_state
                .runs
                .lock()
                .expect("run table mutex poisoned by a prior panic");
            candidates_for_run(&table, &token)
        }
    } else {
        {
            let workspace = active_workspace(&app)?;
            let (store, preparer, inventory) = {
                let outbox_state = app.state::<OutboxState>();
                (
                    outbox_state.policy_store,
                    outbox_state.preparer,
                    outbox_state.inventory,
                )
            };
            tauri::async_runtime::spawn_blocking(move || {
                inventory_whole_outbox(&workspace, store, preparer, inventory)
            })
            .await
            .map_err(|_| {
                ShellError::new(
                    ShellErrorCode::OutboxUnavailable,
                    "the outbox could not be listed",
                )
            })?
        }
    }
}

/// Run `f` against a locked handle to the managed `LaunchGate`, on a
/// blocking thread, so `decide` (which re-probes the filesystem) and
/// `approve_candidate`/`revoke` (which touch the approval store) never
/// stall the async executor.
///
/// # Panics
///
/// Panics if the blocking task itself panics, or if the gate's mutex was
/// poisoned by a prior panic while held.
async fn with_gate<T, F>(app: AppHandle, f: F) -> T
where
    F: FnOnce(&mut ShellLaunchGate) -> T + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ExecutableState>();
        let mut gate = state
            .gate
            .lock()
            .expect("launch gate mutex poisoned by a prior panic");
        f(&mut gate)
    })
    .await
    .expect("the blocking launch-gate task panicked")
}

/// Look up `candidate_id` in `candidates`, or the catalogue `no-candidate`
/// error if it is not known (never probed, or evicted from the bounded
/// table).
fn find_candidate(
    candidates: &CandidateTable,
    candidate_id: CandidateIdDto,
) -> Result<omnifrons_domain::executable::ExecutableIdentity, ShellError> {
    candidates
        .get(CandidateId::from_raw(candidate_id.0))
        .cloned()
        .ok_or_else(|| {
            ShellError::new(
                ShellErrorCode::NoCandidate,
                "no probed candidate with that id is known",
            )
        })
}

/// Open the native file picker and probe whatever the user selected.
///
/// The picker itself and the probe both run on a blocking thread
/// (`tauri::async_runtime::spawn_blocking`): `blocking_pick_file` would
/// deadlock if called directly from the async executor's own thread (it
/// dispatches the dialog back to the platform main thread and blocks
/// waiting for a response), and probing streams a file's content through a
/// synchronous hasher.
///
/// # Errors
///
/// Returns [`ShellError`] with [`ShellErrorCode::NoCandidate`] if no file
/// was selected or its path could not be resolved, or the mapped
/// [`ProbeOutcome`] failure otherwise.
#[tauri::command]
pub async fn executable_pick_and_probe(app: AppHandle) -> Result<ProbeResultDto, ShellError> {
    let dialog_app = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        dialog_app.dialog().file().blocking_pick_file()
    })
    .await
    .expect("the blocking file-picker task panicked");

    let Some(picked) = picked else {
        return Err(ShellError::new(
            ShellErrorCode::NoCandidate,
            "no file was selected",
        ));
    };
    let candidate_path = picked.into_path().map_err(|_| {
        ShellError::new(
            ShellErrorCode::NoCandidate,
            "the selected file's path could not be resolved",
        )
    })?;

    let outcome = tauri::async_runtime::spawn_blocking(move || {
        omnifrons_adapters::FsExecutableProber::new().probe(&candidate_path)
    })
    .await
    .expect("the blocking probe task panicked");

    match outcome {
        omnifrons_app::ProbeOutcome::Identity(executable) => {
            // Only the identity is kept in the candidate table -- the
            // handle this probe opened is dropped here. Approval re-probes
            // at launch time anyway (`LaunchGate::decide`), so a stale
            // handle held from pick-and-probe time would buy nothing.
            let identity = executable.identity;
            let state = app.state::<ExecutableState>();
            let candidate_id = state
                .candidates
                .lock()
                .expect("candidate table mutex poisoned by a prior panic")
                .insert(identity.clone());
            Ok(ProbeResultDto {
                candidate_id: CandidateIdDto(candidate_id.raw()),
                evidence: EvidenceDto::from_identity(&identity),
            })
        }
        other => {
            let failure = other
                .as_domain_failure()
                .expect("a non-Identity ProbeOutcome always maps to a domain failure");
            Err(ShellError::from_probe_failure(&failure))
        }
    }
}

/// Approve a previously probed candidate.
///
/// # Errors
///
/// Returns [`ShellError`] with [`ShellErrorCode::NoCandidate`] if
/// `candidate_id` is not known, or the mapped [`ApprovalStoreError`] if
/// the approval could not be recorded.
#[tauri::command]
pub async fn executable_approve(
    app: AppHandle,
    candidate_id: CandidateIdDto,
) -> Result<ApprovalDto, ShellError> {
    let identity = {
        let state = app.state::<ExecutableState>();
        let candidates = state
            .candidates
            .lock()
            .expect("candidate table mutex poisoned by a prior panic");
        find_candidate(&candidates, candidate_id)?
    };

    let record = with_gate(app, move |gate| gate.approve_candidate(identity))
        .await
        .map_err(ShellError::from)?;

    Ok(ApprovalDto::from_record(&record))
}

/// Revoke a previously recorded approval.
///
/// A no-op, not an error, if `approval_id` is not on record at all (see
/// `omnifrons_app::ApprovalStore::revoke`'s own contract).
///
/// # Errors
///
/// Returns the mapped [`ApprovalStoreError`] if the revocation could not
/// be written.
#[tauri::command]
pub async fn executable_revoke(
    app: AppHandle,
    approval_id: crate::ipc::dto::ApprovalIdDto,
) -> Result<(), ShellError> {
    with_gate(app, move |gate| gate.revoke(approval_id.into()))
        .await
        .map_err(ShellError::from)
}

/// List every approval on record.
///
/// # Errors
///
/// Returns the mapped [`ApprovalStoreError`] if the store could not be
/// read.
#[tauri::command]
pub async fn approvals_list(app: AppHandle) -> Result<Vec<ApprovalDto>, ShellError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ExecutableState>();
        let store = state.open_store()?;
        let records = ApprovalStore::list(&store)?;
        Ok::<_, ApprovalStoreError>(records.iter().map(ApprovalDto::from_record).collect())
    })
    .await
    .expect("the blocking approvals-list task panicked")
    .map_err(ShellError::from)
}

/// Open the native folder picker and, if a folder was selected, resolve it
/// into a [`WorkspaceRoot`] and make it this shell's single active
/// workspace, replacing any previously active one.
///
/// Run on a blocking thread (`tauri::async_runtime::spawn_blocking`):
/// `blocking_pick_folder` would deadlock if called directly from the async
/// executor's own thread, exactly like `executable_pick_and_probe`'s own
/// `blocking_pick_file` above.
///
/// # Errors
///
/// Returns [`ShellError`] with [`ShellErrorCode::NoWorkspace`] if no
/// folder was selected (canceled), or if the selected path could not be
/// resolved into a valid, existing directory.
#[tauri::command]
pub async fn workspace_pick(app: AppHandle) -> Result<WorkspaceDto, ShellError> {
    let dialog_app = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        dialog_app.dialog().file().blocking_pick_folder()
    })
    .await
    .expect("the blocking folder-picker task panicked");

    let no_workspace = || ShellError::new(ShellErrorCode::NoWorkspace, "no folder was selected");

    let picked = picked.ok_or_else(no_workspace)?;
    let candidate_path = picked.into_path().map_err(|_| no_workspace())?;
    let workspace = WorkspaceRoot::new(candidate_path).map_err(|_| no_workspace())?;

    Ok(activate_workspace(
        &app.state::<AdapterState>(),
        &app.state::<OutboxState>(),
        &app.state::<PublicationState>(),
        workspace,
    ))
}

/// The product work area's state against `workspace` (HAP-001-R7): checked
/// without creating anything, on every registration and on every
/// `workspace_current` read.
pub(crate) fn work_area_state(
    publication: &PublicationState,
    workspace: &WorkspaceRoot,
) -> WorkAreaStateTag {
    match WorkAreaRoot::check_configured(&publication.work_area, &[workspace]) {
        Ok(()) => WorkAreaStateTag::Valid,
        Err(error) => {
            tracing::debug!(
                %error,
                "the product work area fails its check against the active workspace"
            );
            WorkAreaStateTag::WorkAreaInvalid
        }
    }
}

/// Make `workspace` the single active workspace. When it differs from the
/// one active so far, every remembered run of the previous workspace is
/// forgotten and its held handles released (spike slice 5, R1-010): the
/// outbox run table is per active workspace within a shell session, so one
/// project's runs never count against the next project's handle and
/// record budget. Picking the workspace that is already active keeps its
/// runs. The product work area is re-checked against the workspace being
/// registered (HAP-001-R7; spike slice 5b) and the outcome rides the DTO:
/// the registration itself proceeds -- the operation R7 refuses is the
/// work area's use, and every publication command re-checks it again.
fn activate_workspace(
    adapter_state: &AdapterState,
    outbox_state: &OutboxState,
    publication_state: &PublicationState,
    workspace: WorkspaceRoot,
) -> WorkspaceDto {
    let dto = WorkspaceDto::new(&workspace, work_area_state(publication_state, &workspace));
    let mut slot = adapter_state
        .active_workspace
        .lock()
        .expect("active workspace mutex poisoned by a prior panic");
    if slot.as_ref() != Some(&workspace) {
        let forgotten = outbox_state.forget_runs();
        tracing::debug!(
            forgotten,
            "the active workspace changed; the previous workspace's run records were forgotten \
             and their handles released"
        );
    }
    *slot = Some(workspace);
    dto
}

/// The currently active workspace, if any.
///
/// `app` is taken by value, not `&AppHandle`, even though this function
/// only ever borrows it: that is the parameter shape `#[tauri::command]`'s
/// own argument-extraction expects for an injected `AppHandle`.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn workspace_current(app: AppHandle) -> Option<WorkspaceDto> {
    let state = app.state::<AdapterState>();
    let publication = app.state::<PublicationState>();
    state
        .active_workspace
        .lock()
        .expect("active workspace mutex poisoned by a prior panic")
        .as_ref()
        .map(|workspace| WorkspaceDto::new(workspace, work_area_state(&publication, workspace)))
}

/// List every built-in adapter's descriptor -- metadata only, never argv
/// or resolved environment values (`docs/spike-log.md` § Slice 3).
///
/// `app` is taken by value for the same reason as [`workspace_current`]'s
/// own `app` parameter.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn adapters_list(app: AppHandle) -> Vec<AdapterDescriptorDto> {
    let state = app.state::<AdapterState>();
    state
        .catalog
        .descriptors()
        .iter()
        .map(AdapterDescriptorDto::from_descriptor)
        .collect()
}

/// Stop a spawned demo harness, gracefully then forcefully, within
/// `deadline_ms`.
///
/// # Errors
///
/// Returns [`ShellError`] with [`ShellErrorCode::InvalidRequest`] if
/// `deadline_ms` is `0` or exceeds `30_000`, or with
/// [`ShellErrorCode::UnknownProcess`] if `id` is not known to the
/// supervisor.
#[tauri::command]
pub async fn harness_stop(
    app: AppHandle,
    id: ProcessIdDto,
    deadline_ms: u64,
) -> Result<ProcessTerminalStateDto, ShellError> {
    let deadline = validate_deadline_ms(deadline_ms)?;
    with_supervisor(app, move |supervisor| {
        supervisor
            .stop(id.into(), deadline)
            .map(ProcessTerminalStateDto::from)
            .map_err(ShellError::from)
    })
    .await
}

/// Observe a spawned demo harness's current status.
///
/// # Errors
///
/// Returns [`ShellError`] with [`ShellErrorCode::UnknownProcess`] if `id`
/// is not known to the supervisor.
#[tauri::command]
pub async fn harness_observe(
    app: AppHandle,
    id: ProcessIdDto,
) -> Result<ProcessStatusDto, ShellError> {
    with_supervisor(app, move |supervisor| {
        supervisor
            .observe(id.into())
            .map(ProcessStatusDto::from)
            .ok_or_else(|| ShellError::from(SupervisorError::UnknownProcess))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{
        AdapterWireSequencer, PtyForwarder, ShellError, adapter_wire_frames, find_candidate,
        pty_wire_frames,
    };
    use crate::executable_state::CandidateTable;
    use crate::ipc::dto::{
        CandidateIdDto, HarnessFrame, ProcessIdDto, ShellErrorCode, ShellErrorDetail,
    };
    use omnifrons_app::{EnvPlan, LineAssembler, StdinPlan, SupervisorError};

    /// R1-001: a `kind: "approved"` launch must never inherit this shell's
    /// own environment. Its plan carries the same base allowlist an
    /// adapter launch with no declared extras gets -- structurally
    /// `EnvPlan::Allowlist`, with exactly the base key set -- plus empty
    /// argv, `StdinPlan::Null`, and no prompt.
    #[test]
    fn default_approved_plan_uses_the_base_allowlist_never_inherit() {
        let plan = super::default_approved_plan().expect("the shell's own cwd must resolve");

        assert!(
            matches!(plan.env, EnvPlan::Allowlist(_)),
            "the approved plan's env must be an allowlist, got {:?}",
            plan.env
        );
        assert_eq!(
            plan.env,
            EnvPlan::new(&[]).expect("an empty declared-key list can never be secret-shaped"),
            "the approved plan's allowlist must be exactly the base key set"
        );
        assert!(plan.argv.is_empty());
        assert_eq!(plan.stdin, StdinPlan::Null);
        assert!(plan.prompt.is_none());
    }
    use omnifrons_domain::executable::{DenialReason, Sha256Digest};

    #[test]
    fn spawn_error_maps_to_a_catalogue_message_with_no_path() {
        let secret_path_error = SupervisorError::Spawn(
            "No such file or directory (os error 2): /home/user/secret/path".to_string(),
        );

        let mapped = ShellError::from(secret_path_error);

        assert_eq!(mapped.code, ShellErrorCode::SpawnFailed);
        assert!(
            !mapped.message.contains('/'),
            "the catalogue message must never echo the underlying error's path, got: {}",
            mapped.message
        );
    }

    #[test]
    fn unknown_process_maps_to_its_own_code() {
        let mapped = ShellError::from(SupervisorError::UnknownProcess);
        assert_eq!(mapped.code, ShellErrorCode::UnknownProcess);
    }

    #[test]
    fn already_subscribed_maps_to_its_own_code() {
        let mapped = ShellError::from(SupervisorError::AlreadySubscribed);
        assert_eq!(mapped.code, ShellErrorCode::AlreadySubscribed);
    }

    #[test]
    fn too_many_processes_maps_to_its_own_code() {
        let mapped = ShellError::from(SupervisorError::TooManyProcesses);
        assert_eq!(mapped.code, ShellErrorCode::TooManyProcesses);
    }

    /// Slice 4: the typed Windows refusal of a PTY launch maps to its own
    /// closed code with a fixed, slash-free catalogue message.
    #[test]
    fn pty_unsupported_maps_to_its_own_code() {
        let mapped = ShellError::from(SupervisorError::PtyUnsupported);
        assert_eq!(mapped.code, ShellErrorCode::PtyUnsupported);
        assert!(!mapped.message.contains('/'));
    }

    /// A `pty-cli` prompt the line discipline would interpret maps to its
    /// own closed code with a fixed, slash-free catalogue message.
    #[test]
    fn prompt_not_typeable_maps_to_its_own_code() {
        let mapped = ShellError::from(omnifrons_app::LaunchPlanError::PromptNotTypeable);
        assert_eq!(mapped.code, ShellErrorCode::PromptNotTypeable);
        assert_eq!(
            mapped.message,
            "prompt contains control characters a terminal would interpret"
        );
        assert!(!mapped.message.contains('/'));
    }

    /// The live forwarder branches on the adapter's transport class alone:
    /// `pty` takes the PTY branch, the structured transport the line
    /// branch.
    #[test]
    fn forwarder_kind_follows_the_transport_class() {
        use omnifrons_domain::adapter::TransportClass;
        assert_eq!(
            super::forwarder_for(TransportClass::Pty),
            super::ForwarderKind::Pty
        );
        assert_eq!(
            super::forwarder_for(TransportClass::StructuredStreamingCli),
            super::ForwarderKind::Line
        );
    }

    /// The catalog the live forwarder consults yields the expected
    /// transport class for each built-in id, so a `pty-cli` launch takes
    /// the PTY branch and a `stream-json-cli` launch the line branch.
    #[test]
    fn catalog_lookup_yields_the_expected_transport_class_per_adapter_id() {
        use omnifrons_domain::adapter::{AdapterId, TransportClass};
        let state = crate::adapter_state::AdapterState::new();
        for (id, transport, kind) in [
            (
                AdapterId::pty_cli(),
                TransportClass::Pty,
                super::ForwarderKind::Pty,
            ),
            (
                AdapterId::stream_json_cli(),
                TransportClass::StructuredStreamingCli,
                super::ForwarderKind::Line,
            ),
            (
                AdapterId::claude_code(),
                TransportClass::StructuredStreamingCli,
                super::ForwarderKind::Line,
            ),
        ] {
            let descriptor = state
                .catalog
                .get(&id)
                .unwrap_or_else(|| panic!("{id} must be in the catalog"))
                .describe();
            assert_eq!(descriptor.transport_class, transport, "for {id}");
            assert_eq!(
                super::forwarder_for(descriptor.transport_class),
                kind,
                "for {id}"
            );
        }
    }

    #[test]
    fn deadline_ms_zero_is_rejected_as_invalid_request() {
        let error = super::validate_deadline_ms(0).unwrap_err();
        assert_eq!(error.code, ShellErrorCode::InvalidRequest);
    }

    #[test]
    fn deadline_ms_above_30_000_is_rejected_as_invalid_request() {
        let error = super::validate_deadline_ms(30_001).unwrap_err();
        assert_eq!(error.code, ShellErrorCode::InvalidRequest);
    }

    #[test]
    fn deadline_ms_boundaries_are_accepted() {
        assert!(super::validate_deadline_ms(1).is_ok());
        assert!(super::validate_deadline_ms(30_000).is_ok());
    }

    #[test]
    fn approve_with_unknown_candidate_is_no_candidate() {
        let candidates = CandidateTable::default();

        let error = find_candidate(&candidates, CandidateIdDto(999)).unwrap_err();

        assert_eq!(error.code, ShellErrorCode::NoCandidate);
    }

    /// R3-009: `executable_approve` (via `find_candidate`) on a candidate
    /// id that has since been evicted from the bounded table must report
    /// the same `no-candidate` code as one that was never probed at all.
    #[test]
    fn approve_with_an_evicted_candidate_is_no_candidate() {
        use omnifrons_domain::executable::{ExecutableIdentity, PlatformEvidence};
        use std::path::PathBuf;

        fn identity(n: u8) -> ExecutableIdentity {
            ExecutableIdentity {
                canonical_path: PathBuf::from(format!("/opt/tool/app-{n}")),
                size: 4096,
                sha256: Sha256Digest([n; 32]),
                modified_at: None,
                platform: PlatformEvidence::Unix { mode: 0o755 },
            }
        }

        let mut candidates = CandidateTable::default();
        let evicted_id = candidates.insert(identity(0));
        // One more than the table's own bound (32) guarantees the first
        // insert above has been evicted by the time this loop finishes.
        for n in 1..=32u8 {
            candidates.insert(identity(n));
        }

        let error = find_candidate(&candidates, CandidateIdDto(evicted_id.raw())).unwrap_err();

        assert_eq!(error.code, ShellErrorCode::NoCandidate);
    }

    // -- Slice 3: the adapter forwarder's pure frame mapping --

    /// A fake adapter for the forwarder tests: the line `"two"` yields a
    /// `Message` then a `ToolCall` (two events from one line); any other
    /// line yields a single `Message` echoing it.
    struct TwoEventAdapter;

    impl omnifrons_app::HarnessAdapter for TwoEventAdapter {
        fn describe(&self) -> omnifrons_app::AdapterDescriptor {
            omnifrons_app::AdapterDescriptor {
                id: omnifrons_domain::adapter::AdapterId::stream_json_cli(),
                display_name: "Two-event fake".to_string(),
                transport_class: omnifrons_domain::adapter::TransportClass::StructuredStreamingCli,
                prompt_channel: omnifrons_domain::adapter::PromptChannel::StdinThenClose,
                argv_template: vec![],
                declared_env: vec![],
                scope_mode: omnifrons_domain::scope::ScopeMode::Advisory,
                notes: String::new(),
            }
        }

        fn build_launch(
            &self,
            request: &omnifrons_app::LaunchRequest,
        ) -> Result<omnifrons_app::LaunchPlan, omnifrons_app::LaunchPlanError> {
            Ok(omnifrons_app::LaunchPlan {
                argv: vec![],
                env: omnifrons_app::EnvPlan::new(&[])?,
                cwd: request.workspace.clone(),
                stdin: omnifrons_app::StdinPlan::PipePromptThenClose,
                prompt: Some(request.prompt.clone()),
                scope_mode: omnifrons_domain::scope::ScopeMode::Advisory,
                transport: omnifrons_domain::adapter::TransportClass::StructuredStreamingCli,
                output_dir: None,
            })
        }

        fn parse_line(&self, line: &str) -> Vec<omnifrons_domain::adapter::AdapterEvent> {
            use omnifrons_domain::adapter::{AdapterEvent, ToolCallProposal};
            if line == "two" {
                vec![
                    AdapterEvent::Message {
                        text: "first".to_string(),
                    },
                    AdapterEvent::ToolCall(ToolCallProposal {
                        name: "write_file".to_string(),
                        arguments_text: "{}".to_string(),
                    }),
                ]
            } else {
                vec![AdapterEvent::Message {
                    text: line.to_string(),
                }]
            }
        }
    }

    fn stdout_frame(
        seq: u64,
        dropped_before: u64,
        text: &str,
        continued: bool,
    ) -> omnifrons_app::OutputFrame {
        omnifrons_app::OutputFrame::with_dropped_before(
            seq,
            dropped_before,
            omnifrons_app::FramePayload::Text {
                stream: omnifrons_app::OutputStream::Stdout,
                text: text.to_string(),
                continued,
            },
        )
    }

    fn stderr_frame(seq: u64, dropped_before: u64, text: &str) -> omnifrons_app::OutputFrame {
        omnifrons_app::OutputFrame::with_dropped_before(
            seq,
            dropped_before,
            omnifrons_app::FramePayload::Text {
                stream: omnifrons_app::OutputStream::Stderr,
                text: text.to_string(),
                continued: false,
            },
        )
    }

    fn state_frame(seq: u64, dropped_before: u64) -> omnifrons_app::OutputFrame {
        omnifrons_app::OutputFrame::with_dropped_before(
            seq,
            dropped_before,
            omnifrons_app::FramePayload::State(omnifrons_app::ProcessTerminalState::Exited {
                code: Some(0),
            }),
        )
    }

    /// `(seq, droppedBefore, kind)` of an emitted wire frame.
    fn summarize(frame: &HarnessFrame) -> (u64, u64, &'static str) {
        use crate::ipc::dto::AdapterEventDto;
        match frame {
            HarnessFrame::Event {
                seq,
                dropped_before,
                event,
                ..
            } => {
                let kind = match event {
                    AdapterEventDto::State { .. } => "state",
                    AdapterEventDto::Message { .. } => "message",
                    AdapterEventDto::ToolCall { .. } => "tool-call",
                    AdapterEventDto::Diagnostic { .. } => "diagnostic",
                    AdapterEventDto::Unknown { .. } => "unknown",
                    AdapterEventDto::TerminalText { .. } => "terminal-text",
                    AdapterEventDto::TerminalAction { .. } => "terminal-action",
                    AdapterEventDto::TerminalDrops { .. } => "terminal-drops",
                    AdapterEventDto::ArtifactPublish { .. } => "artifact-publish",
                    AdapterEventDto::Candidates { .. } => "candidates",
                };
                (*seq, *dropped_before, kind)
            }
            HarnessFrame::State {
                seq,
                dropped_before,
                ..
            } => (*seq, *dropped_before, "terminal"),
            HarnessFrame::Stdout { .. } | HarnessFrame::Stderr { .. } => {
                panic!("an adapter launch must never emit a raw stdout/stderr frame, got {frame:?}")
            }
        }
    }

    /// R3-002: the summed `dropped_before` of every raw frame that fed a
    /// coalesced line reaches the wire, not only the last frame's own.
    #[test]
    fn adapter_forwarder_sums_dropped_before_across_coalesced_frames() {
        use crate::ipc::dto::AdapterEventDto;
        let mut assembler = LineAssembler::new();
        let mut sequencer = AdapterWireSequencer::new(ProcessIdDto(7));

        let first = adapter_wire_frames(
            &TwoEventAdapter,
            &mut assembler,
            &mut sequencer,
            stdout_frame(0, 2, "abc", true),
        );
        assert!(first.is_empty(), "a continued frame completes nothing yet");

        let second = adapter_wire_frames(
            &TwoEventAdapter,
            &mut assembler,
            &mut sequencer,
            stdout_frame(1, 0, "def", false),
        );
        assert_eq!(second.len(), 1);
        assert_eq!(summarize(&second[0]), (0, 2, "message"));
        match &second[0] {
            HarnessFrame::Event {
                event: AdapterEventDto::Message { text },
                ..
            } => assert_eq!(text, "abcdef"),
            other => panic!("expected the assembled message, got {other:?}"),
        }
    }

    /// R3-004 at the wire: one line yielding two events becomes two wire
    /// frames, `seq` contiguous per launch (never duplicated -- the
    /// renderer keys entries by it), `droppedBefore` on the first only;
    /// stderr and state frames continue the same sequence.
    #[test]
    fn adapter_forwarder_emits_one_wire_frame_per_event_with_a_contiguous_seq() {
        let mut assembler = LineAssembler::new();
        let mut sequencer = AdapterWireSequencer::new(ProcessIdDto(7));

        let mut emitted = adapter_wire_frames(
            &TwoEventAdapter,
            &mut assembler,
            &mut sequencer,
            stdout_frame(0, 1, "two", false),
        );
        emitted.extend(adapter_wire_frames(
            &TwoEventAdapter,
            &mut assembler,
            &mut sequencer,
            stderr_frame(1, 3, "diagnostic: hello"),
        ));
        emitted.extend(adapter_wire_frames(
            &TwoEventAdapter,
            &mut assembler,
            &mut sequencer,
            state_frame(2, 0),
        ));

        let summary: Vec<_> = emitted.iter().map(summarize).collect();
        assert_eq!(
            summary,
            vec![
                (0, 1, "message"),
                (1, 0, "tool-call"),
                (2, 3, "diagnostic"),
                (3, 0, "terminal"),
            ]
        );
    }

    /// A partial line still pending when the process reaches its terminal
    /// state is flushed as `Unknown{truncated: true}` before the `state`
    /// frame, never silently dropped.
    #[test]
    fn adapter_forwarder_flushes_a_pending_partial_line_before_the_state_frame() {
        use crate::ipc::dto::AdapterEventDto;
        let mut assembler = LineAssembler::new();
        let mut sequencer = AdapterWireSequencer::new(ProcessIdDto(7));

        let pending = adapter_wire_frames(
            &TwoEventAdapter,
            &mut assembler,
            &mut sequencer,
            stdout_frame(0, 0, "head", true),
        );
        assert!(pending.is_empty());

        let emitted = adapter_wire_frames(
            &TwoEventAdapter,
            &mut assembler,
            &mut sequencer,
            state_frame(1, 0),
        );
        let summary: Vec<_> = emitted.iter().map(summarize).collect();
        assert_eq!(summary, vec![(0, 0, "unknown"), (1, 0, "terminal")]);
        match &emitted[0] {
            HarnessFrame::Event {
                event: AdapterEventDto::Unknown { raw, truncated },
                ..
            } => {
                assert_eq!(raw, "head");
                assert!(truncated);
            }
            other => panic!("expected the flushed partial line, got {other:?}"),
        }
    }

    // -- Slice 4: the PTY forwarder's pure frame mapping --

    /// Simulate the supervisor's own line framing over `bytes` -- one raw
    /// `stdout` frame per line, a `\r` immediately before the `\n`
    /// stripped, a final unterminated line still delivered -- so the
    /// forwarder is driven exactly as it is by a real PTY launch (no
    /// corpus line reaches the 8 KiB cap, so nothing is `continued`).
    fn framed_lines(bytes: &[u8]) -> Vec<omnifrons_app::OutputFrame> {
        let mut frames = Vec::new();
        let mut seq = 0;
        let mut push = |line: &[u8]| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            frames.push(stdout_frame(seq, 0, &String::from_utf8_lossy(line), false));
            seq += 1;
        };
        let mut rest = bytes;
        while let Some(index) = rest.iter().position(|&b| b == b'\n') {
            push(&rest[..index]);
            rest = &rest[index + 1..];
        }
        if !rest.is_empty() {
            push(rest);
        }
        frames
    }

    /// The `(kind, payload)` of every emitted `event` frame plus the
    /// terminal `state`, as JSON, for exact sequence assertions.
    fn pty_kinds(frames: &[HarnessFrame]) -> Vec<&'static str> {
        frames.iter().map(|frame| summarize(frame).2).collect()
    }

    fn terminal_texts(frames: &[HarnessFrame]) -> String {
        use crate::ipc::dto::AdapterEventDto;
        frames
            .iter()
            .filter_map(|frame| match frame {
                HarnessFrame::Event {
                    event: AdapterEventDto::TerminalText { text },
                    ..
                } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn terminal_actions(frames: &[HarnessFrame]) -> Vec<(String, String)> {
        use crate::ipc::dto::AdapterEventDto;
        frames
            .iter()
            .filter_map(|frame| match frame {
                HarnessFrame::Event {
                    event: AdapterEventDto::TerminalAction { action, text },
                    ..
                } => Some((
                    serde_json::to_value(action)
                        .expect("serializes")
                        .as_str()
                        .expect("a string token")
                        .to_string(),
                    text.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    fn summed_drops(frames: &[HarnessFrame]) -> omnifrons_domain::terminal::DropCounts {
        use crate::ipc::dto::AdapterEventDto;
        let mut total = omnifrons_domain::terminal::DropCounts::default();
        for frame in frames {
            if let HarnessFrame::Event {
                event:
                    AdapterEventDto::TerminalDrops {
                        layout,
                        hyperlink,
                        clipboard,
                        file_transfer,
                        string,
                        unknown,
                        malformed,
                    },
                ..
            } = frame
            {
                total += omnifrons_domain::terminal::DropCounts {
                    layout: *layout,
                    hyperlink: *hyperlink,
                    clipboard: *clipboard,
                    file_transfer: *file_transfer,
                    string: *string,
                    unknown: *unknown,
                    malformed: *malformed,
                };
            }
        }
        total
    }

    /// The shared corpus, framed as the supervisor frames a PTY launch's
    /// output and driven through the forwarder, yields exactly the
    /// expected text (CRLF normalized by the framing, plus the newline the
    /// forwarder appends after the trailing bare ESC), exactly the expected
    /// actions in order, exactly the expected drop totals, a contiguous
    /// per-launch `seq`, and never the clipboard sentinel anywhere in the
    /// serialized wire frames.
    #[test]
    fn pty_forwarder_maps_the_corpus_to_the_expected_chunks_actions_and_drops() {
        use omnifrons_app::terminal_normalizer::corpus;
        let mut forwarder = PtyForwarder::new();
        let mut sequencer = AdapterWireSequencer::new(ProcessIdDto(7));

        let mut emitted = Vec::new();
        for frame in framed_lines(&corpus::bytes()) {
            emitted.extend(pty_wire_frames(&mut forwarder, &mut sequencer, frame));
        }
        emitted.extend(pty_wire_frames(
            &mut forwarder,
            &mut sequencer,
            state_frame(999, 0),
        ));

        let mut expected_text = corpus::expected_text().replace("\r\n", "\n");
        expected_text.push('\n');
        assert_eq!(terminal_texts(&emitted), expected_text);

        let expected_actions: Vec<(String, String)> = corpus::expected_actions()
            .into_iter()
            .map(|action| match action {
                omnifrons_domain::terminal::TerminalAction::Title(text) => {
                    ("title".to_string(), text)
                }
                omnifrons_domain::terminal::TerminalAction::Notification(text) => {
                    ("notification".to_string(), text)
                }
            })
            .collect();
        assert_eq!(terminal_actions(&emitted), expected_actions);

        assert_eq!(summed_drops(&emitted), corpus::expected_drops());

        let seqs: Vec<u64> = emitted.iter().map(|frame| summarize(frame).0).collect();
        assert_eq!(seqs, (0..seqs.len() as u64).collect::<Vec<_>>());
        assert_eq!(pty_kinds(&emitted).last(), Some(&"terminal"));

        let wire = serde_json::to_string(&emitted).expect("frames serialize");
        assert!(!wire.contains(corpus::CLIPBOARD_SENTINEL));
        assert!(!wire.contains('\u{1b}'));
        assert!(!wire.contains("example.invalid"));
    }

    /// One raw frame yields, in order: its text (adjacent text chunks
    /// coalesced into one `terminal-text`), each action where it occurred,
    /// and at most one `terminal-drops` -- only when the counts changed.
    #[test]
    fn pty_forwarder_emits_text_then_actions_then_at_most_one_drops_per_raw_frame() {
        let mut forwarder = PtyForwarder::new();
        let mut sequencer = AdapterWireSequencer::new(ProcessIdDto(7));

        let plain = pty_wire_frames(
            &mut forwarder,
            &mut sequencer,
            stdout_frame(0, 0, "plain", false),
        );
        assert_eq!(pty_kinds(&plain), vec!["terminal-text"]);
        assert_eq!(terminal_texts(&plain), "plain\n");

        let mixed = pty_wire_frames(
            &mut forwarder,
            &mut sequencer,
            stdout_frame(1, 0, "a\x1b[1mb\x1b]0;t\x07c\x1b]8;;x\x07d", false),
        );
        assert_eq!(
            pty_kinds(&mixed),
            vec![
                "terminal-text",
                "terminal-action",
                "terminal-text",
                "terminal-drops"
            ]
        );
        assert_eq!(terminal_texts(&mixed), "abcd\n");
        assert_eq!(
            summed_drops(&mixed),
            omnifrons_domain::terminal::DropCounts {
                layout: 1,
                hyperlink: 1,
                ..Default::default()
            }
        );
    }

    /// `droppedBefore` rides on the first wire frame a raw frame produces,
    /// `0` on the rest; a raw frame that produces no wire frame at all (an
    /// escape sequence still being collected) carries its count forward to
    /// the next wire frame, never losing it.
    #[test]
    fn pty_forwarder_carries_dropped_before_onto_the_first_wire_frame_and_across_silent_frames() {
        let mut forwarder = PtyForwarder::new();
        let mut sequencer = AdapterWireSequencer::new(ProcessIdDto(7));

        // A frame that only opens an OSC (the newline the forwarder appends
        // aborts it -- but drops are reported, so this frame is not silent;
        // use a continued frame to keep the sequence open).
        let silent = pty_wire_frames(
            &mut forwarder,
            &mut sequencer,
            stdout_frame(0, 3, "\x1b]0;pending", true),
        );
        assert!(silent.is_empty(), "nothing completes yet, got {silent:?}");

        let completed = pty_wire_frames(
            &mut forwarder,
            &mut sequencer,
            stdout_frame(1, 4, " title\x07after", false),
        );
        let summary: Vec<_> = completed.iter().map(summarize).collect();
        assert_eq!(
            summary,
            vec![(0, 7, "terminal-action"), (1, 0, "terminal-text")],
            "the summed count lands on the first wire frame only"
        );
    }

    /// A `State` frame first finishes the normalizer (a sequence still
    /// open is malformed and reported as drops) and then becomes the
    /// terminal `state` frame -- always last.
    #[test]
    fn pty_forwarder_finishes_the_normalizer_before_the_state_frame() {
        let mut forwarder = PtyForwarder::new();
        let mut sequencer = AdapterWireSequencer::new(ProcessIdDto(7));

        let pending = pty_wire_frames(
            &mut forwarder,
            &mut sequencer,
            stdout_frame(0, 0, "tail\x1b]0;never terminated", true),
        );
        assert_eq!(pty_kinds(&pending), vec!["terminal-text"]);
        assert_eq!(terminal_texts(&pending), "tail");

        let ended = pty_wire_frames(&mut forwarder, &mut sequencer, state_frame(1, 5));
        let summary: Vec<_> = ended.iter().map(summarize).collect();
        assert_eq!(summary, vec![(1, 5, "terminal-drops"), (2, 0, "terminal")]);
        assert_eq!(
            summed_drops(&ended),
            omnifrons_domain::terminal::DropCounts {
                malformed: 1,
                ..Default::default()
            }
        );
    }

    /// The supervisor's synthetic `stderr` frame (`stdin write failed`) on
    /// the PTY path maps to a `diagnostic` event exactly as on the line
    /// path: a terminal has no stderr, so this is the only stderr text a
    /// PTY launch can ever carry.
    #[test]
    fn pty_forwarder_maps_a_stderr_frame_to_a_diagnostic_event() {
        let mut forwarder = PtyForwarder::new();
        let mut sequencer = AdapterWireSequencer::new(ProcessIdDto(7));
        let emitted = pty_wire_frames(
            &mut forwarder,
            &mut sequencer,
            stderr_frame(0, 1, "stdin write failed"),
        );
        assert_eq!(
            emitted.iter().map(summarize).collect::<Vec<_>>(),
            vec![(0, 1, "diagnostic")]
        );
    }

    // -- Slice 5: the outbox --

    /// A drop-guard temp project for the outbox tests.
    struct TempProject(std::path::PathBuf);

    impl TempProject {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "omnifrons-shell-outbox-test-{}-{label}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("fixture dir");
            Self(dir)
        }

        fn workspace(&self) -> omnifrons_app::WorkspaceRoot {
            omnifrons_app::WorkspaceRoot::new(&self.0).expect("valid workspace")
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Every `PrepareError` renders `outbox-unavailable` at launch
    /// (HAP-001-R10, D14), with a fixed, slash-free message.
    #[test]
    fn prepare_error_maps_to_outbox_unavailable_with_a_slash_free_message() {
        use omnifrons_app::run_outbox::{
            OutboxDeclarationError, PrepareError, VerificationFailure,
        };
        for error in [
            PrepareError::Declaration(OutboxDeclarationError::IsLink),
            PrepareError::Declaration(OutboxDeclarationError::OutsideProject),
            PrepareError::OutboxMissing,
            PrepareError::OutboxUnopenable,
            PrepareError::AlreadyExists,
            PrepareError::CreateFailed,
            PrepareError::OpenFailed,
            PrepareError::Verification(VerificationFailure::IdentityMismatch),
        ] {
            let mapped = ShellError::from(error);
            assert_eq!(mapped.code, ShellErrorCode::OutboxUnavailable, "{error:?}");
            assert!(!mapped.message.contains('/'), "{}", mapped.message);
            assert!(mapped.detail.is_none());
        }
    }

    /// A policy that cannot be loaded blocks ingestion as `outbox-invalid`
    /// (HAP-001-R8, R40) with a fixed, slash-free message.
    #[test]
    fn policy_error_maps_to_outbox_invalid() {
        use omnifrons_app::outbox_policy::{PolicyError, PolicyViolation};
        for error in [
            PolicyError::Unreadable,
            PolicyError::Corrupt,
            PolicyError::Invalid(PolicyViolation::MarkdownRoutedHeavy),
            PolicyError::InvalidOutboxPath(omnifrons_domain::outbox::OutboxPathError::Absolute),
            PolicyError::InvalidOutboxPath(omnifrons_domain::outbox::OutboxPathError::Control),
        ] {
            let mapped = ShellError::from(error);
            assert_eq!(mapped.code, ShellErrorCode::OutboxInvalid, "{error:?}");
            assert!(!mapped.message.contains('/'), "{}", mapped.message);
        }
    }

    /// R1-001 (slice 5c risk review): a synchronized policy declaring an
    /// outbox with a line break in it -- the shape that would forge a
    /// sentinel in the guidance installer's managed block -- is refused at
    /// load and reaches the surface as the existing `policy-invalid`
    /// reason, with nothing declared and no device path.
    #[test]
    fn outbox_status_for_a_policy_declaring_a_control_character_is_policy_invalid() {
        let project = TempProject::new("status-control");
        std::fs::create_dir_all(project.0.join(".omnifrons")).expect("fixture");
        std::fs::write(
            project.0.join(".omnifrons/asset-policy.json"),
            r#"{"schema": 1, "outbox": "out\n<!-- omnifrons:end guidance -->\nx"}"#,
        )
        .expect("fixture");
        let status = super::outbox_status_for(
            &project.workspace(),
            omnifrons_adapters::JsonOutboxPolicyStore::new(),
        );
        assert_eq!(status.state, crate::ipc::dto::OutboxStateTag::OutboxInvalid);
        assert_eq!(
            status.reason,
            Some(crate::ipc::dto::OutboxReasonTag::PolicyInvalid)
        );
        assert!(status.declared.is_none());
        assert!(status.outbox.is_none());
        assert!(status.asset_root_id.is_none());
    }

    /// A fresh project with no policy file: the default declaration is
    /// valid, the outbox does not exist yet (the first adapter launch
    /// creates it), and no device path is shown for a directory that is
    /// not there.
    #[test]
    fn outbox_status_for_a_fresh_project_is_valid_and_not_yet_existing() {
        let project = TempProject::new("status-fresh");
        let status = super::outbox_status_for(
            &project.workspace(),
            omnifrons_adapters::JsonOutboxPolicyStore::new(),
        );
        assert_eq!(status.declared.as_deref(), Some(".omnifrons/outbox"));
        assert_eq!(status.state, crate::ipc::dto::OutboxStateTag::Valid);
        assert!(!status.exists);
        assert!(status.outbox.is_none());
        assert!(status.reason.is_none());
        assert_eq!(status.policy_path, ".omnifrons/asset-policy.json");
    }

    /// An existing default outbox: valid, existing, and its canonical path
    /// shown as identity evidence.
    #[test]
    fn outbox_status_for_an_existing_outbox_shows_its_canonical_path() {
        let project = TempProject::new("status-existing");
        std::fs::create_dir_all(project.0.join(".omnifrons/outbox")).expect("fixture");
        let status = super::outbox_status_for(
            &project.workspace(),
            omnifrons_adapters::JsonOutboxPolicyStore::new(),
        );
        assert_eq!(status.state, crate::ipc::dto::OutboxStateTag::Valid);
        assert!(status.exists);
        let expected =
            std::fs::canonicalize(project.0.join(".omnifrons/outbox")).expect("canonical");
        assert_eq!(
            status.outbox.as_deref(),
            Some(expected.to_string_lossy().as_ref())
        );
    }

    /// A corrupt policy: `outbox-unavailable` for the launch side, the
    /// reason named, nothing declared, no path.
    #[test]
    fn outbox_status_for_a_corrupt_policy_names_the_reason_and_declares_nothing() {
        let project = TempProject::new("status-corrupt");
        std::fs::create_dir_all(project.0.join(".omnifrons")).expect("fixture");
        std::fs::write(project.0.join(".omnifrons/asset-policy.json"), b"{ nope").expect("fixture");
        let status = super::outbox_status_for(
            &project.workspace(),
            omnifrons_adapters::JsonOutboxPolicyStore::new(),
        );
        assert_eq!(status.state, crate::ipc::dto::OutboxStateTag::OutboxInvalid);
        assert_eq!(
            status.reason,
            Some(crate::ipc::dto::OutboxReasonTag::PolicyCorrupt)
        );
        assert!(status.declared.is_none());
        assert!(status.outbox.is_none());
    }

    /// Unix only (symlink fixture): a declared outbox that is a link is
    /// `outbox-invalid` with reason `link`, the declared name shown, and
    /// no device path.
    #[cfg(unix)]
    #[test]
    fn outbox_status_for_a_linked_outbox_is_invalid_with_no_device_path() {
        let project = TempProject::new("status-link");
        std::fs::create_dir_all(project.0.join("real")).expect("fixture");
        std::fs::create_dir_all(project.0.join(".omnifrons")).expect("fixture");
        std::os::unix::fs::symlink(project.0.join("real"), project.0.join(".omnifrons/outbox"))
            .expect("fixture symlink");
        let status = super::outbox_status_for(
            &project.workspace(),
            omnifrons_adapters::JsonOutboxPolicyStore::new(),
        );
        assert_eq!(status.state, crate::ipc::dto::OutboxStateTag::OutboxInvalid);
        assert_eq!(status.reason, Some(crate::ipc::dto::OutboxReasonTag::Link));
        assert_eq!(status.declared.as_deref(), Some(".omnifrons/outbox"));
        assert!(status.outbox.is_none());
    }

    /// R1-002 (renderer risk review; HAP-001-R22): `outbox_status` carries
    /// the policy's asset root identity -- the destination shown before the
    /// decision -- as a token, never a path, and `null` when the policy
    /// declares none or cannot be loaded.
    #[test]
    fn outbox_status_reports_the_policys_asset_root_identity() {
        let project = TempProject::new("status-asset-root");
        let store = omnifrons_adapters::JsonOutboxPolicyStore::new();
        let status = super::outbox_status_for(&project.workspace(), store);
        assert_eq!(
            status.asset_root_id, None,
            "the default policy declares none"
        );

        let policy_path = project
            .0
            .join(omnifrons_app::outbox_policy::POLICY_FILE_PATH);
        std::fs::create_dir_all(policy_path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&policy_path, r#"{"schema": 1, "assetRootId": "main"}"#).expect("policy");
        let status = super::outbox_status_for(&project.workspace(), store);
        assert_eq!(status.asset_root_id.as_deref(), Some("main"));
        assert_eq!(status.state, crate::ipc::dto::OutboxStateTag::Valid);

        std::fs::write(&policy_path, "not json").expect("corrupt the policy");
        let status = super::outbox_status_for(&project.workspace(), store);
        assert_eq!(
            status.asset_root_id, None,
            "a policy that cannot be loaded declares nothing"
        );
    }

    /// Write `content` as `name` inside `dir` and return its digest as the
    /// prober computes it -- the shell has no hashing dependency of its
    /// own, and the prober is what the inventory uses anyway.
    fn write_and_digest(
        dir: &omnifrons_app::PreparedRunSubdirectory,
        name: &str,
        content: &[u8],
    ) -> Sha256Digest {
        use omnifrons_app::run_outbox::{CandidateProbe, CandidateProber as _};
        std::fs::write(dir.path.join(name), content).expect("fixture file");
        match omnifrons_adapters::FsCandidateProber::new().probe(
            &dir.handle,
            &dir.path,
            std::ffi::OsStr::new(name),
        ) {
            CandidateProbe::Regular(regular) => regular.digest,
            other => panic!("the fixture file must probe Regular, got {other:?}"),
        }
    }

    /// Prepare a run for `project` under the default outbox and register
    /// it in a fresh run table under `id`.
    fn prepared_run(
        project: &TempProject,
        id: omnifrons_app::ProcessId,
        token: &str,
    ) -> (
        std::sync::Arc<std::sync::Mutex<crate::outbox_state::RunTable>>,
        std::path::PathBuf,
    ) {
        use omnifrons_app::run_outbox::RunOutboxPreparer as _;
        let run_id = omnifrons_domain::outbox::RunId::new(token).expect("valid");
        let prepared = omnifrons_adapters::FsRunOutboxPreparer::new()
            .prepare(
                &project.workspace(),
                &omnifrons_domain::outbox::OutboxPath::default_path(),
                &run_id,
            )
            .expect("prepare");
        let path = prepared.path.clone();
        let mut table = crate::outbox_state::RunTable::default();
        table.insert(
            id,
            crate::outbox_state::RunRecord::with_provenance(
                prepared,
                omnifrons_app::outbox_policy::OutboxPolicy::default_policy(),
                None,
                None,
            ),
        );
        (std::sync::Arc::new(std::sync::Mutex::new(table)), path)
    }

    /// A fake adapter whose line `publish:<hex>` yields an
    /// `ArtifactPublish` naming `report.pdf` by that digest; any other
    /// line is a `Message`.
    struct PublishingAdapter;

    impl omnifrons_app::HarnessAdapter for PublishingAdapter {
        fn describe(&self) -> omnifrons_app::AdapterDescriptor {
            TwoEventAdapter.describe()
        }

        fn build_launch(
            &self,
            request: &omnifrons_app::LaunchRequest,
        ) -> Result<omnifrons_app::LaunchPlan, omnifrons_app::LaunchPlanError> {
            TwoEventAdapter.build_launch(request)
        }

        fn parse_line(&self, line: &str) -> Vec<omnifrons_domain::adapter::AdapterEvent> {
            use omnifrons_domain::adapter::AdapterEvent;
            use omnifrons_domain::outbox::{ProposedEntry, PublishProposal};
            if let Some(hex) = line.strip_prefix("publish:") {
                return vec![AdapterEvent::ArtifactPublish(PublishProposal {
                    entries: vec![ProposedEntry {
                        name: "report.pdf".to_string(),
                        sha256: Sha256Digest::from_hex(hex).expect("the test passes valid hex"),
                    }],
                })];
            }
            vec![AdapterEvent::Message {
                text: line.to_string(),
            }]
        }
    }

    /// Drive `frames` through the line forwarder with `PublishingAdapter`,
    /// in order, over one assembler.
    fn drive_publishing(
        sequencer: &mut AdapterWireSequencer,
        frames: Vec<omnifrons_app::OutputFrame>,
    ) -> Vec<HarnessFrame> {
        let mut assembler = LineAssembler::new();
        let mut emitted = Vec::new();
        for frame in frames {
            emitted.extend(adapter_wire_frames(
                &PublishingAdapter,
                &mut assembler,
                sequencer,
                frame,
            ));
        }
        emitted
    }

    /// HAP-001-R11/R12 at the wire: the proposal a run surfaces is
    /// recorded; at the terminal state the run subdirectory is inventoried
    /// and a `candidates` event summarizing it precedes the `state` frame
    /// (`seq` contiguous, `state` still last); the stored candidates carry
    /// the attribution the digest match decided -- the proposed file is
    /// the run's, the unproposed one is unattributed even though it sits
    /// in the same subdirectory.
    #[test]
    fn run_end_inventory_emits_candidates_before_the_state_frame_and_records_attribution() {
        use crate::ipc::dto::AdapterEventDto;
        use omnifrons_domain::outbox::{Attribution, CandidateState};

        let project = TempProject::new("run-end");
        let id = omnifrons_app::ProcessId(7);
        let (runs, run_path) = prepared_run(&project, id, "run-end-1");
        let prepared_handle = {
            let table = runs.lock().expect("table");
            table
                .get(id)
                .expect("record")
                .subdirectory_clone()
                .expect("clone the held handle")
        };
        let report = write_and_digest(&prepared_handle, "report.pdf", b"%PDF-1.7\nreport");
        write_and_digest(
            &prepared_handle,
            "stray.png",
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        );

        let tracker = super::RunEndTracker::new(
            std::sync::Arc::clone(&runs),
            id,
            omnifrons_adapters::FsOutboxInventory::new(),
        );
        let mut sequencer = AdapterWireSequencer::with_outbox(ProcessIdDto(7), tracker);
        let emitted = drive_publishing(
            &mut sequencer,
            vec![
                stdout_frame(0, 0, &format!("publish:{}", report.to_hex()), false),
                stdout_frame(1, 0, "working", false),
                state_frame(2, 0),
            ],
        );

        let summary: Vec<_> = emitted.iter().map(summarize).collect();
        assert_eq!(
            summary,
            vec![
                (0, 0, "artifact-publish"),
                (1, 0, "message"),
                (2, 0, "candidates"),
                (3, 0, "terminal"),
            ],
            "the candidates event rides before the terminal state frame with a contiguous seq"
        );
        match &emitted[2] {
            HarnessFrame::Event {
                event:
                    AdapterEventDto::Candidates {
                        run_id,
                        total,
                        candidate,
                        outbox_escape,
                        outbox_linked,
                        attributed,
                        unattributed,
                        unreadable,
                        unmatched_proposals,
                    },
                ..
            } => {
                assert_eq!(run_id, "run-end-1");
                assert_eq!(
                    (*total, *candidate, *outbox_escape, *outbox_linked),
                    (2, 2, 0, 0)
                );
                assert_eq!((*attributed, *unattributed), (1, 1));
                assert_eq!((*unreadable, *unmatched_proposals), (0, 0));
            }
            other => panic!("expected the candidates event, got {other:?}"),
        }
        let wire = serde_json::to_string(&emitted).expect("frames serialize");
        assert!(
            !wire.contains(run_path.to_string_lossy().as_ref()),
            "no frame carries the run subdirectory's path"
        );

        let table = runs.lock().expect("table");
        let record = table.get(id).expect("record");
        let candidates = record.candidates().expect("the inventory was stored");
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].entry.name, "run-end-1/report.pdf");
        assert_eq!(
            candidates[0].entry.attribution,
            Attribution::Run(omnifrons_domain::outbox::RunId::new("run-end-1").expect("valid"))
        );
        assert_eq!(candidates[0].state, CandidateState::Candidate);
        assert!(
            candidates[0].handle.is_some(),
            "within the cap, the handle is held"
        );
        assert_eq!(candidates[1].entry.name, "run-end-1/stray.png");
        assert_eq!(candidates[1].entry.attribution, Attribution::Unattributed);
    }

    /// A proposal naming a digest no entry carries is surfaced as a
    /// `diagnostic` event before the `candidates` summary, naming the
    /// proposed name and the digest's short prefix only.
    #[test]
    fn an_unmatched_proposal_is_surfaced_as_a_diagnostic_before_the_summary() {
        use crate::ipc::dto::AdapterEventDto;

        let project = TempProject::new("unmatched");
        let id = omnifrons_app::ProcessId(8);
        let (runs, _run_path) = prepared_run(&project, id, "run-unmatched");
        let ghost = Sha256Digest([0x77; 32]);

        let tracker = super::RunEndTracker::new(
            std::sync::Arc::clone(&runs),
            id,
            omnifrons_adapters::FsOutboxInventory::new(),
        );
        let mut sequencer = AdapterWireSequencer::with_outbox(ProcessIdDto(8), tracker);
        let mut assembler = LineAssembler::new();
        let mut emitted = adapter_wire_frames(
            &PublishingAdapter,
            &mut assembler,
            &mut sequencer,
            stdout_frame(0, 0, &format!("publish:{}", ghost.to_hex()), false),
        );
        emitted.extend(adapter_wire_frames(
            &PublishingAdapter,
            &mut assembler,
            &mut sequencer,
            state_frame(1, 0),
        ));
        let summary: Vec<_> = emitted.iter().map(summarize).collect();
        assert_eq!(
            summary,
            vec![
                (0, 0, "artifact-publish"),
                (1, 0, "diagnostic"),
                (2, 0, "candidates"),
                (3, 0, "terminal"),
            ]
        );
        match &emitted[1] {
            HarnessFrame::Event {
                event: AdapterEventDto::Diagnostic { text },
                ..
            } => {
                assert!(text.contains("report.pdf"), "got {text}");
                assert!(text.contains(&ghost.short_hex()), "got {text}");
                assert!(
                    !text.contains(&ghost.to_hex()),
                    "only the short prefix, got {text}"
                );
            }
            other => panic!("expected the diagnostic, got {other:?}"),
        }
        match &emitted[2] {
            HarnessFrame::Event {
                event:
                    AdapterEventDto::Candidates {
                        total,
                        unmatched_proposals,
                        ..
                    },
                ..
            } => {
                assert_eq!((*total, *unmatched_proposals), (0, 1));
            }
            other => panic!("expected the candidates event, got {other:?}"),
        }
    }

    /// A sequencer without a run tracker (a launch that prepared no
    /// outbox) emits the `state` frame alone, as before.
    #[test]
    fn a_sequencer_without_a_tracker_emits_only_the_state_frame() {
        let mut sequencer = AdapterWireSequencer::new(ProcessIdDto(9));
        let mut assembler = LineAssembler::new();
        let emitted = adapter_wire_frames(
            &PublishingAdapter,
            &mut assembler,
            &mut sequencer,
            state_frame(0, 0),
        );
        assert_eq!(
            emitted.iter().map(summarize).collect::<Vec<_>>(),
            vec![(0, 0, "terminal")]
        );
    }

    /// The whole-outbox inventory (HAP-001-R12, the session-start
    /// proposal): entries at the outbox root and one level down are all
    /// unattributed, named relative to the outbox, and no handle is held.
    #[test]
    fn whole_outbox_inventory_lists_root_and_run_entries_unattributed() {
        use crate::ipc::dto::AttributionDto;

        let project = TempProject::new("whole-outbox");
        let id = omnifrons_app::ProcessId(10);
        let (runs, _run_path) = prepared_run(&project, id, "run-old");
        let prepared_handle = {
            let table = runs.lock().expect("table");
            table
                .get(id)
                .expect("record")
                .subdirectory_clone()
                .expect("clone the held handle")
        };
        write_and_digest(&prepared_handle, "later.pdf", b"%PDF-1.7\nlater");
        std::fs::write(
            project.0.join(".omnifrons/outbox/dropped.pdf"),
            b"%PDF-1.7\ndropped",
        )
        .expect("fixture");

        let listed = super::inventory_whole_outbox(
            &project.workspace(),
            omnifrons_adapters::JsonOutboxPolicyStore::new(),
            omnifrons_adapters::FsRunOutboxPreparer::new(),
            omnifrons_adapters::FsOutboxInventory::new(),
        )
        .expect("the inventory must succeed");
        let names: Vec<&str> = listed.iter().map(|dto| dto.name.as_str()).collect();
        assert_eq!(names, vec!["dropped.pdf", "run-old/later.pdf"]);
        for dto in &listed {
            assert_eq!(
                dto.attribution,
                AttributionDto::Unattributed,
                "{}",
                dto.name
            );
            assert_eq!(dto.class.as_deref(), Some("generated-heavy"));
        }
    }

    /// The whole-outbox inventory of a project whose outbox does not exist
    /// yet is empty, not an error; a corrupt policy is `outbox-invalid`.
    #[test]
    fn whole_outbox_inventory_of_a_missing_outbox_is_empty_and_a_corrupt_policy_is_invalid() {
        let project = TempProject::new("whole-outbox-missing");
        let listed = super::inventory_whole_outbox(
            &project.workspace(),
            omnifrons_adapters::JsonOutboxPolicyStore::new(),
            omnifrons_adapters::FsRunOutboxPreparer::new(),
            omnifrons_adapters::FsOutboxInventory::new(),
        )
        .expect("a missing outbox is an empty inventory");
        assert!(listed.is_empty());

        std::fs::create_dir_all(project.0.join(".omnifrons")).expect("fixture");
        std::fs::write(project.0.join(".omnifrons/asset-policy.json"), b"{ nope").expect("fixture");
        let error = super::inventory_whole_outbox(
            &project.workspace(),
            omnifrons_adapters::JsonOutboxPolicyStore::new(),
            omnifrons_adapters::FsRunOutboxPreparer::new(),
            omnifrons_adapters::FsOutboxInventory::new(),
        )
        .unwrap_err();
        assert_eq!(error.code, ShellErrorCode::OutboxInvalid);
    }

    #[test]
    fn changed_since_approval_denial_carries_both_short_digests() {
        let recorded = Sha256Digest([1; 32]);
        let observed = Sha256Digest([2; 32]);

        let mapped = ShellError::from(DenialReason::ChangedSinceApproval { recorded, observed });

        assert_eq!(mapped.code, ShellErrorCode::ChangedSinceApproval);
        let detail = mapped
            .detail
            .expect("changed-since-approval must carry a structured detail");
        assert_eq!(
            detail,
            ShellErrorDetail::ChangedSinceApproval {
                recorded_sha256_short: recorded.short_hex(),
                observed_sha256_short: observed.short_hex(),
            }
        );
    }

    // -- Review fixes: the project-wide handle cap (R1-001), the extracted
    // launch sequence and its cleanup (R1-002/R3-002), the proposal cap
    // (R1-006), `candidates_for_run` (R3-003), the PTY run (R3-004), and
    // the oversized policy (R1-004) --

    /// Register a second prepared run in an existing table and return a
    /// duplicate of its held handle for writing fixtures.
    fn register_run(
        runs: &std::sync::Arc<std::sync::Mutex<crate::outbox_state::RunTable>>,
        project: &TempProject,
        id: omnifrons_app::ProcessId,
        token: &str,
    ) -> omnifrons_app::PreparedRunSubdirectory {
        use omnifrons_app::run_outbox::RunOutboxPreparer as _;
        let prepared = omnifrons_adapters::FsRunOutboxPreparer::new()
            .prepare(
                &project.workspace(),
                &omnifrons_domain::outbox::OutboxPath::default_path(),
                &omnifrons_domain::outbox::RunId::new(token).expect("valid"),
            )
            .expect("prepare");
        let clone = prepared.try_clone().expect("clone the held handle");
        runs.lock().expect("table").insert(
            id,
            crate::outbox_state::RunRecord::with_provenance(
                prepared,
                omnifrons_app::outbox_policy::OutboxPolicy::default_policy(),
                None,
                None,
            ),
        );
        clone
    }

    fn tracker_for(
        runs: &std::sync::Arc<std::sync::Mutex<crate::outbox_state::RunTable>>,
        id: omnifrons_app::ProcessId,
    ) -> super::RunEndTracker {
        super::RunEndTracker::new(
            std::sync::Arc::clone(runs),
            id,
            omnifrons_adapters::FsOutboxInventory::new(),
        )
    }

    /// R1-001: the D22 cap of 32 held handles is enforced across every
    /// remembered run of the project, not per run. Run A ends holding 20;
    /// run B ends with 20 entries of its own and keeps only 12 handles --
    /// every one of its entries still `candidate`.
    #[test]
    fn held_handles_are_capped_across_runs_not_per_run() {
        use omnifrons_domain::outbox::CandidateState;
        let project = TempProject::new("cross-run-cap");
        let id_a = omnifrons_app::ProcessId(21);
        let id_b = omnifrons_app::ProcessId(22);
        let (runs, _path_a) = prepared_run(&project, id_a, "run-cap-a");
        let handle_a = runs
            .lock()
            .expect("table")
            .get(id_a)
            .expect("record")
            .subdirectory_clone()
            .expect("clone");
        let handle_b = register_run(&runs, &project, id_b, "run-cap-b");
        for n in 0..20u8 {
            write_and_digest(
                &handle_a,
                &format!("a{n}.pdf"),
                &[b"%PDF-a".as_slice(), &[n]].concat(),
            );
            write_and_digest(
                &handle_b,
                &format!("b{n}.pdf"),
                &[b"%PDF-b".as_slice(), &[n]].concat(),
            );
        }

        let _ = tracker_for(&runs, id_a).finish();
        assert_eq!(runs.lock().expect("table").held_handles(), 20);

        let _ = tracker_for(&runs, id_b).finish();
        let table = runs.lock().expect("table");
        assert_eq!(table.held_handles(), 32, "the cap is project-wide");
        let candidates_b = table
            .get(id_b)
            .expect("record")
            .candidates()
            .expect("stored");
        assert_eq!(candidates_b.len(), 20);
        assert!(
            candidates_b
                .iter()
                .all(|candidate| candidate.state == CandidateState::Candidate),
            "an entry beyond the cap stays candidate"
        );
        assert_eq!(
            candidates_b
                .iter()
                .filter(|candidate| candidate.handle.is_some())
                .count(),
            12
        );
    }

    /// R1-002/R3-002: the extracted launch sequence -- prepare, declare,
    /// spawn -- removes the empty run subdirectory when the declaration
    /// fails, before anything is spawned.
    ///
    /// `EnvPlan::Inherit` is the only way to make `declare_output_dir` fail
    /// today, and no wired adapter ever produces it (every built-in
    /// `build_launch` returns an allowlist), so this branch is structurally
    /// unreachable from the product: the test proves the function's
    /// totality on that branch, not a product scenario.
    #[test]
    fn a_failing_declaration_removes_the_empty_run_subdirectory_and_spawns_nothing() {
        let project = TempProject::new("launch-declare-fails");
        let workspace = project.workspace();
        let run_id = omnifrons_domain::outbox::RunId::new("run-declare").expect("valid");
        let mut plan = super::default_approved_plan().expect("plan");
        plan.env = EnvPlan::Inherit;
        let spawned = std::cell::Cell::new(false);

        let error = super::launch_with_run_subdirectory(
            &omnifrons_adapters::JsonOutboxPolicyStore::new(),
            &omnifrons_adapters::FsRunOutboxPreparer::new(),
            &workspace,
            &run_id,
            plan,
            |_plan| {
                spawned.set(true);
                Ok::<_, SupervisorError>(omnifrons_app::ProcessId(1))
            },
        )
        .unwrap_err();

        assert_eq!(error.code, ShellErrorCode::InvalidRequest);
        assert!(
            !spawned.get(),
            "nothing may be spawned after a failed declaration"
        );
        assert!(
            !project.0.join(".omnifrons/outbox/run-declare").exists(),
            "the empty run subdirectory is removed"
        );
        assert!(
            project.0.join(".omnifrons/outbox").is_dir(),
            "the outbox itself stays"
        );
    }

    /// R1-002/R3-002: a failing spawn removes the empty run subdirectory;
    /// the plan handed to the spawn carried the declaration.
    #[test]
    fn a_failing_spawn_removes_the_empty_run_subdirectory() {
        let project = TempProject::new("launch-spawn-fails");
        let workspace = project.workspace();
        let run_id = omnifrons_domain::outbox::RunId::new("run-spawn").expect("valid");
        let plan = super::default_approved_plan().expect("plan");
        let expected_dir = project.0.join(".omnifrons/outbox/run-spawn");

        let error = super::launch_with_run_subdirectory(
            &omnifrons_adapters::JsonOutboxPolicyStore::new(),
            &omnifrons_adapters::FsRunOutboxPreparer::new(),
            &workspace,
            &run_id,
            plan,
            |plan| {
                let declared = plan
                    .output_dir
                    .as_ref()
                    .expect("the plan declares the output dir");
                assert_eq!(
                    declared,
                    &std::fs::canonicalize(&expected_dir).expect("the created dir canonicalizes")
                );
                assert!(
                    plan.env
                        .assignment_for(omnifrons_app::OUTPUT_DIR_ENV_KEY)
                        .is_some()
                );
                assert!(
                    declared.is_dir(),
                    "the subdirectory exists when the spawn runs"
                );
                Err::<omnifrons_app::ProcessId, _>(SupervisorError::Spawn("refused".to_string()))
            },
        )
        .unwrap_err();

        assert_eq!(error.code, ShellErrorCode::SpawnFailed);
        assert!(
            !expected_dir.exists(),
            "the empty run subdirectory is removed"
        );
    }

    /// R1-002/R3-002: a run subdirectory something was dropped into is
    /// never removed -- `remove_dir` refuses a non-empty directory, so a
    /// stranger's file survives the failed launch untouched.
    #[test]
    fn a_non_empty_run_subdirectory_is_never_removed_on_a_failed_launch() {
        let project = TempProject::new("launch-non-empty");
        let workspace = project.workspace();
        let run_id = omnifrons_domain::outbox::RunId::new("run-stranger").expect("valid");
        let plan = super::default_approved_plan().expect("plan");

        let error = super::launch_with_run_subdirectory(
            &omnifrons_adapters::JsonOutboxPolicyStore::new(),
            &omnifrons_adapters::FsRunOutboxPreparer::new(),
            &workspace,
            &run_id,
            plan,
            |plan| {
                let declared = plan.output_dir.as_ref().expect("declared");
                std::fs::write(declared.join("stranger.bin"), b"dropped in").expect("fixture");
                Err::<omnifrons_app::ProcessId, _>(SupervisorError::Spawn("refused".to_string()))
            },
        )
        .unwrap_err();

        assert_eq!(error.code, ShellErrorCode::SpawnFailed);
        let dir = project.0.join(".omnifrons/outbox/run-stranger");
        assert!(dir.is_dir(), "a non-empty subdirectory is left where it is");
        assert!(
            dir.join("stranger.bin").is_file(),
            "nothing a stranger dropped in is deleted"
        );
    }

    /// R1-002/R3-002: a successful spawn keeps the subdirectory and returns
    /// the id, the prepared subdirectory, and the policy in force.
    #[test]
    fn a_successful_launch_keeps_the_subdirectory_and_returns_its_record_parts() {
        let project = TempProject::new("launch-ok");
        let workspace = project.workspace();
        let run_id = omnifrons_domain::outbox::RunId::new("run-ok").expect("valid");
        let plan = super::default_approved_plan().expect("plan");
        let expected_dir = project.0.join(".omnifrons/outbox/run-ok");

        let launched = super::launch_with_run_subdirectory(
            &omnifrons_adapters::JsonOutboxPolicyStore::new(),
            &omnifrons_adapters::FsRunOutboxPreparer::new(),
            &workspace,
            &run_id,
            plan,
            |plan| {
                // The plan reaching `spawn` carries the declaration both
                // ways: the field and the environment assignment.
                let canonical =
                    std::fs::canonicalize(&expected_dir).expect("the created dir canonicalizes");
                assert_eq!(plan.output_dir.as_deref(), Some(canonical.as_path()));
                let assignment = plan
                    .env
                    .assignment_for(omnifrons_app::OUTPUT_DIR_ENV_KEY)
                    .expect("the plan reaching spawn carries the OMNIFRONS_OUTPUT_DIR assignment");
                assert_eq!(assignment.value, canonical.into_os_string());
                Ok::<_, SupervisorError>(omnifrons_app::ProcessId(5))
            },
        )
        .expect("the launch succeeds");

        assert_eq!(launched.id, omnifrons_app::ProcessId(5));
        assert_eq!(launched.subdirectory.run_id, run_id);
        assert!(launched.subdirectory.path.is_dir());
        assert_eq!(
            launched.policy,
            omnifrons_app::outbox_policy::OutboxPolicy::default_policy()
        );
    }

    /// R1-006: proposal entries beyond the per-run cap are counted and
    /// surfaced as exactly one diagnostic at run end, first; the recorded
    /// entries still take part in attribution (here: 1024 unmatched
    /// proposals, each its own diagnostic, then the summary, then state).
    #[test]
    fn proposal_entries_beyond_the_cap_are_counted_and_surfaced_as_one_diagnostic() {
        use crate::ipc::dto::AdapterEventDto;
        use crate::outbox_state::MAX_PROPOSED_ENTRIES;
        use omnifrons_domain::outbox::{ProposedEntry, PublishProposal};

        let project = TempProject::new("proposal-cap");
        let id = omnifrons_app::ProcessId(23);
        let (runs, _path) = prepared_run(&project, id, "run-proposals");
        {
            let mut table = runs.lock().expect("table");
            let record = table.get_mut(id).expect("record");
            for n in 0..=MAX_PROPOSED_ENTRIES {
                record.push_proposal(PublishProposal {
                    entries: vec![ProposedEntry {
                        name: format!("f{n}"),
                        sha256: Sha256Digest([u8::try_from(n % 251).expect("fits"); 32]),
                    }],
                });
            }
            assert_eq!(record.recorded_proposal_entries(), MAX_PROPOSED_ENTRIES);
            assert_eq!(record.dropped_proposal_entries(), 1);
        }

        let mut sequencer =
            AdapterWireSequencer::with_outbox(ProcessIdDto(23), tracker_for(&runs, id));
        let emitted = drive_publishing(&mut sequencer, vec![state_frame(0, 0)]);

        assert_eq!(emitted.len(), 1 + MAX_PROPOSED_ENTRIES + 2);
        match &emitted[0] {
            HarnessFrame::Event {
                event: AdapterEventDto::Diagnostic { text },
                ..
            } => {
                assert!(
                    text.contains("1 "),
                    "the dropped count is named, got {text}"
                );
                assert!(text.contains("beyond"), "got {text}");
            }
            other => panic!("expected the cap diagnostic first, got {other:?}"),
        }
        assert_eq!(summarize(emitted.last().expect("frames")).2, "terminal");
    }

    /// R3-003: `candidates_for_run` refuses an invalid token, an unknown
    /// run, and a run whose inventory has not been taken, each as
    /// `invalid-request`, and lists a finished run's candidates.
    #[test]
    fn candidates_for_run_refuses_invalid_unknown_and_unfinished_runs() {
        let project = TempProject::new("candidates-for-run");
        let id = omnifrons_app::ProcessId(24);
        let (runs, _path) = prepared_run(&project, id, "run-listed");
        let handle = runs
            .lock()
            .expect("table")
            .get(id)
            .expect("record")
            .subdirectory_clone()
            .expect("clone");
        write_and_digest(&handle, "one.pdf", b"%PDF-1.7\none");

        {
            let table = runs.lock().expect("table");
            for token in ["../escape", "", "a b"] {
                let error = super::candidates_for_run(&table, token).unwrap_err();
                assert_eq!(error.code, ShellErrorCode::InvalidRequest, "{token:?}");
                assert!(!error.message.contains('/'));
            }
            let unknown = super::candidates_for_run(&table, "run-unknown").unwrap_err();
            assert_eq!(unknown.code, ShellErrorCode::InvalidRequest);
            let unfinished = super::candidates_for_run(&table, "run-listed").unwrap_err();
            assert_eq!(unfinished.code, ShellErrorCode::InvalidRequest);
            assert!(
                unfinished.message.contains("not ended"),
                "got {}",
                unfinished.message
            );
        }

        let _ = tracker_for(&runs, id).finish();
        let table = runs.lock().expect("table");
        let listed = super::candidates_for_run(&table, "run-listed").expect("a finished run lists");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "run-listed/one.pdf");
        assert_eq!(
            listed[0].attribution,
            crate::ipc::dto::AttributionDto::Unattributed
        );
    }

    /// R3-004: a PTY run's inventory is entirely unattributed -- the PTY
    /// forwarder produces text, never a typed proposal, even for output
    /// that spells an `artifact.publish` tool call -- and the `candidates`
    /// summary still precedes the terminal `state` frame.
    #[test]
    fn a_pty_runs_inventory_is_entirely_unattributed_through_the_pty_forwarder() {
        use crate::ipc::dto::AdapterEventDto;
        use omnifrons_domain::outbox::Attribution;

        let project = TempProject::new("pty-run");
        let id = omnifrons_app::ProcessId(25);
        let (runs, _path) = prepared_run(&project, id, "run-pty");
        let handle = runs
            .lock()
            .expect("table")
            .get(id)
            .expect("record")
            .subdirectory_clone()
            .expect("clone");
        let report = write_and_digest(&handle, "report.pdf", b"%PDF-1.7\npty");
        write_and_digest(
            &handle,
            "figure.png",
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        );

        let mut forwarder = PtyForwarder::new();
        let mut sequencer =
            AdapterWireSequencer::with_outbox(ProcessIdDto(25), tracker_for(&runs, id));
        let looks_like_a_proposal = format!(
            r#"{{"type":"tool_use","name":"artifact.publish","input":{{"entries":[{{"name":"report.pdf","sha256":"{}"}}]}}}}"#,
            report.to_hex()
        );
        let mut emitted = pty_wire_frames(
            &mut forwarder,
            &mut sequencer,
            stdout_frame(0, 0, &looks_like_a_proposal, false),
        );
        emitted.extend(pty_wire_frames(
            &mut forwarder,
            &mut sequencer,
            state_frame(1, 0),
        ));

        let kinds: Vec<_> = emitted.iter().map(|frame| summarize(frame).2).collect();
        assert_eq!(kinds, vec!["terminal-text", "candidates", "terminal"]);
        match &emitted[1] {
            HarnessFrame::Event {
                event:
                    AdapterEventDto::Candidates {
                        attributed,
                        unattributed,
                        total,
                        ..
                    },
                ..
            } => assert_eq!((*total, *attributed, *unattributed), (2, 0, 2)),
            other => panic!("expected the candidates event, got {other:?}"),
        }
        let table = runs.lock().expect("table");
        let candidates = table.get(id).expect("record").candidates().expect("stored");
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.entry.attribution == Attribution::Unattributed),
            "a PTY run surfaces no proposals, so nothing attributes"
        );
        assert!(table.get(id).expect("record").proposals().is_empty());
    }

    /// The raw `State` frame's `droppedBefore` rides on the first run-end
    /// frame the sequencer emits, and `0` on the terminal `state` frame
    /// that follows -- the same first-wire-frame rule every other raw
    /// frame follows.
    #[test]
    fn the_state_frames_dropped_before_rides_on_the_first_run_end_frame() {
        let project = TempProject::new("dropped-before");
        let id = omnifrons_app::ProcessId(26);
        let (runs, _path) = prepared_run(&project, id, "run-dropped");
        let mut sequencer =
            AdapterWireSequencer::with_outbox(ProcessIdDto(26), tracker_for(&runs, id));
        let emitted = drive_publishing(&mut sequencer, vec![state_frame(0, 5)]);
        assert_eq!(
            emitted.iter().map(summarize).collect::<Vec<_>>(),
            vec![(0, 5, "candidates"), (1, 0, "terminal")]
        );
    }

    /// R1-004: an oversized policy file is refused at load and reported
    /// with the fixed reason `policy-invalid`; nothing is declared.
    #[test]
    fn outbox_status_for_an_oversized_policy_is_invalid_with_a_fixed_reason() {
        let project = TempProject::new("status-oversized");
        std::fs::create_dir_all(project.0.join(".omnifrons")).expect("fixture");
        let oversized =
            vec![
                b' ';
                usize::try_from(omnifrons_app::outbox_policy::POLICY_MAX_BYTES).expect("fits") + 1
            ];
        std::fs::write(project.0.join(".omnifrons/asset-policy.json"), oversized).expect("fixture");
        let status = super::outbox_status_for(
            &project.workspace(),
            omnifrons_adapters::JsonOutboxPolicyStore::new(),
        );
        assert_eq!(status.state, crate::ipc::dto::OutboxStateTag::OutboxInvalid);
        assert_eq!(
            status.reason,
            Some(crate::ipc::dto::OutboxReasonTag::PolicyInvalid)
        );
        assert!(status.declared.is_none());
    }

    // -- Re-review leftovers: the run table follows the active workspace
    // (R1-010) and an eviction between the inventory's two phases is never
    // silent (R1-011) --

    /// R1-010: the run table is per active workspace within a shell
    /// session. Picking a *different* workspace forgets every remembered
    /// run and releases its handles, so a previous project's runs never
    /// count against the next project's 32-handle / 64-record budget.
    #[test]
    fn picking_a_different_workspace_forgets_the_previous_workspaces_runs() {
        let project_a = TempProject::new("activate-a");
        let project_b = TempProject::new("activate-b");
        let device = TempProject::new("activate-device");
        let adapter_state = crate::adapter_state::AdapterState::new();
        let outbox_state = crate::outbox_state::OutboxState::new();
        let publication_state = crate::publication_state::PublicationState::under(&device.0);

        let dto = super::activate_workspace(
            &adapter_state,
            &outbox_state,
            &publication_state,
            project_a.workspace(),
        );
        assert_eq!(
            dto.display_path,
            project_a.workspace().path().to_string_lossy()
        );
        let id = omnifrons_app::ProcessId(31);
        let handle = register_run(&outbox_state.runs, &project_a, id, "run-under-a");
        for n in 0..3u8 {
            write_and_digest(
                &handle,
                &format!("f{n}.pdf"),
                &[b"%PDF-".as_slice(), &[n]].concat(),
            );
        }
        let _ = tracker_for(&outbox_state.runs, id).finish();
        assert_eq!(outbox_state.runs.lock().expect("table").held_handles(), 3);

        super::activate_workspace(
            &adapter_state,
            &outbox_state,
            &publication_state,
            project_b.workspace(),
        );

        let table = outbox_state.runs.lock().expect("table");
        assert_eq!(
            table.held_handles(),
            0,
            "the previous workspace's held handles are released"
        );
        assert!(
            table
                .find_by_run_id(
                    &omnifrons_domain::outbox::RunId::new("run-under-a").expect("valid")
                )
                .is_none(),
            "its run records are forgotten"
        );
        assert_eq!(
            *adapter_state
                .active_workspace
                .lock()
                .expect("active workspace"),
            Some(project_b.workspace())
        );
    }

    /// R1-010: picking the workspace that is already active is not a
    /// change -- its runs and handles are kept.
    #[test]
    fn re_picking_the_active_workspace_keeps_its_runs() {
        let project = TempProject::new("activate-same");
        let device = TempProject::new("activate-same-device");
        let adapter_state = crate::adapter_state::AdapterState::new();
        let outbox_state = crate::outbox_state::OutboxState::new();
        let publication_state = crate::publication_state::PublicationState::under(&device.0);
        super::activate_workspace(
            &adapter_state,
            &outbox_state,
            &publication_state,
            project.workspace(),
        );
        let id = omnifrons_app::ProcessId(33);
        let handle = register_run(&outbox_state.runs, &project, id, "run-kept");
        write_and_digest(&handle, "kept.pdf", b"%PDF-1.7\nkept");
        let _ = tracker_for(&outbox_state.runs, id).finish();
        assert_eq!(outbox_state.runs.lock().expect("table").held_handles(), 1);

        super::activate_workspace(
            &adapter_state,
            &outbox_state,
            &publication_state,
            project.workspace(),
        );

        let table = outbox_state.runs.lock().expect("table");
        assert_eq!(table.held_handles(), 1, "a re-pick keeps the held handle");
        assert!(table.get(id).is_some(), "and the record");
    }

    /// R1-002 (HAP-001-R7 on every workspace registration): activating a
    /// workspace re-checks the configured work area against it and reports
    /// the outcome on the `WorkspaceDto`, so the next publication command
    /// is not the first to notice; the check creates nothing.
    #[test]
    fn activating_a_workspace_over_the_work_area_reports_work_area_invalid() {
        let project = TempProject::new("activate-work-area");
        let device = TempProject::new("activate-work-area-device");
        let adapter_state = crate::adapter_state::AdapterState::new();
        let outbox_state = crate::outbox_state::OutboxState::new();

        let inside =
            crate::publication_state::PublicationState::under(&project.0.join(".omnifrons"));
        let dto =
            super::activate_workspace(&adapter_state, &outbox_state, &inside, project.workspace());
        assert_eq!(
            dto.work_area,
            crate::ipc::dto::WorkAreaStateTag::WorkAreaInvalid
        );
        assert!(
            !project.0.join(".omnifrons/work-area").exists(),
            "the check creates nothing"
        );

        let outside = crate::publication_state::PublicationState::under(&device.0);
        let dto =
            super::activate_workspace(&adapter_state, &outbox_state, &outside, project.workspace());
        assert_eq!(dto.work_area, crate::ipc::dto::WorkAreaStateTag::Valid);
        assert!(
            !device.0.join("work-area").exists(),
            "a valid check creates nothing either"
        );
    }

    /// R1-011: a run record evicted between the inventory's two phases --
    /// the snapshot taken under one lock, the store under another -- never
    /// loses its candidates silently: the store reports no record, the
    /// handles are released explicitly, and a diagnostic names the run id
    /// and how many candidates were not retained (counts only).
    #[test]
    fn a_record_evicted_during_the_inventory_is_reported_not_silently_dropped() {
        use crate::ipc::dto::AdapterEventDto;

        let project = TempProject::new("evicted-mid-inventory");
        let id = omnifrons_app::ProcessId(32);
        let (runs, _path) = prepared_run(&project, id, "run-evicted");
        let handle = runs
            .lock()
            .expect("table")
            .get(id)
            .expect("record")
            .subdirectory_clone()
            .expect("clone");
        for n in 0..3u8 {
            write_and_digest(
                &handle,
                &format!("f{n}.pdf"),
                &[b"%PDF-".as_slice(), &[n]].concat(),
            );
        }
        let tracker = tracker_for(&runs, id);
        let input = match tracker.snapshot() {
            super::RunEndPhase::Ready(input) => *input,
            other => panic!("the record is remembered before the eviction, got {other:?}"),
        };

        // Evict the record by driving the table directly: a full table of
        // newer records pushes the oldest one out.
        for n in 0..crate::outbox_state::MAX_RUN_RECORDS {
            register_run(
                &runs,
                &project,
                omnifrons_app::ProcessId(1000 + u32::try_from(n).expect("fits")),
                &format!("run-newer-{n}"),
            );
        }
        assert!(runs.lock().expect("table").get(id).is_none(), "evicted");

        let events = tracker.conclude(input);
        let kinds: Vec<&str> = events
            .iter()
            .map(|event| match event {
                AdapterEventDto::Candidates { .. } => "candidates",
                AdapterEventDto::Diagnostic { .. } => "diagnostic",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["candidates", "diagnostic"]);
        match &events[1] {
            AdapterEventDto::Diagnostic { text } => {
                assert!(text.contains("run-evicted"), "names the run id, got {text}");
                assert!(text.contains("3 candidates"), "names the count, got {text}");
                assert!(!text.contains('/'), "never a path, got {text}");
            }
            other => panic!("expected the retention diagnostic, got {other:?}"),
        }
        assert_eq!(
            runs.lock().expect("table").held_handles(),
            0,
            "nothing of the evicted run is held"
        );
    }
}
