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
use std::time::Duration;

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
use omnifrons_domain::scope::ScopeMode;
use omnifrons_supervisor::TokioProcessSupervisor;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::adapter_state::AdapterState;
use crate::executable_state::{CandidateId, CandidateTable, ExecutableState, ShellLaunchGate};
use crate::ipc::dto::{
    AdapterDescriptorDto, ApprovalDto, CandidateIdDto, EvidenceDto, HarnessFrame, HarnessKindDto,
    ProbeResultDto, ProcessIdDto, ProcessStatusDto, ProcessTerminalStateDto, ShellError,
    ShellErrorCode, ShellErrorDetail, WorkspaceDto,
};

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
                ShellErrorDetail {
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

        let mut sequencer = AdapterWireSequencer::new(id_dto);
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
        frames.push(sequencer.state(dropped_before, state));
    }
    frames
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
struct AdapterWireSequencer {
    id: ProcessIdDto,
    next_seq: u64,
}

impl AdapterWireSequencer {
    fn new(id: ProcessIdDto) -> Self {
        Self { id, next_seq: 0 }
    }

    fn next_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    fn event(&mut self, dropped_before: u64, event: AdapterEvent) -> HarnessFrame {
        let seq = self.next_seq();
        HarnessFrame::event(self.id, seq, dropped_before, event)
    }

    fn state(&mut self, dropped_before: u64, state: ProcessTerminalState) -> HarnessFrame {
        let seq = self.next_seq();
        HarnessFrame::State {
            id: self.id,
            seq,
            dropped_before,
            terminal: state.into(),
        }
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
            frames.push(sequencer.state(frame.dropped_before, state));
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
        let request = LaunchRequest { prompt, workspace };
        adapter.build_launch(&request)?
    };

    let display_path = executable.identity.canonical_path.clone();
    let (id, receiver) = with_supervisor(app.clone(), move |supervisor| {
        let id = supervisor.spawn_approved(executable.handle, display_path, &plan)?;
        let receiver = supervisor.subscribe(id)?;
        Ok::<_, SupervisorError>((id, receiver))
    })
    .await?;

    forward_adapter_output(id, adapter_id, receiver, on_frame, app);
    Ok(ProcessIdDto::from(id))
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

    let dto = WorkspaceDto::from_workspace(&workspace);
    let state = app.state::<AdapterState>();
    *state
        .active_workspace
        .lock()
        .expect("active workspace mutex poisoned by a prior panic") = Some(workspace);

    Ok(dto)
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
    state
        .active_workspace
        .lock()
        .expect("active workspace mutex poisoned by a prior panic")
        .as_ref()
        .map(WorkspaceDto::from_workspace)
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
    use crate::ipc::dto::{CandidateIdDto, HarnessFrame, ProcessIdDto, ShellErrorCode};
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

    #[test]
    fn changed_since_approval_denial_carries_both_short_digests() {
        let recorded = Sha256Digest([1; 32]);
        let observed = Sha256Digest([2; 32]);

        let mapped = ShellError::from(DenialReason::ChangedSinceApproval { recorded, observed });

        assert_eq!(mapped.code, ShellErrorCode::ChangedSinceApproval);
        let detail = mapped
            .detail
            .expect("changed-since-approval must carry a structured detail");
        assert_eq!(detail.recorded_sha256_short, recorded.short_hex());
        assert_eq!(detail.observed_sha256_short, observed.short_hex());
    }
}
