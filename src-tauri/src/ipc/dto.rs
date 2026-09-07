//! Typed IPC data shapes: the renderer only ever sees these, never a
//! domain type directly (`docs/spike-log.md` § IPC contract). Every field
//! name is `camelCase` on the wire; every closed-set discriminant (a
//! harness kind, a terminal state, an error code, a frame's stream) is a
//! fixed `kebab-case` token.

use serde::{Deserialize, Serialize};

/// The closed set of harness kinds a caller may request, as it crosses
/// IPC.
///
/// Internally tagged (`{"type": "demo-lines", "rateHz": ..., "lines":
/// ...}`, `{"type": "approved", "approvalId": ...}`), each variant
/// bundling exactly the parameters that kind needs, rather than a flat
/// `kind` string plus separate top-level `rateHz`/`lines`/`approvalId`
/// parameters most of which would be meaningless for any given `kind` --
/// the tagged shape makes an invalid combination (e.g. `approved` with a
/// `rateHz`) inexpressible on the wire, instead of merely rejected after
/// the fact (`docs/spike-log.md` § Slice 2 records this choice).
///
/// Not `Copy` as of the spike slice-3 spike: `Adapter`'s `adapter_id` and
/// `prompt` fields are both `String`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HarnessKindDto {
    DemoLines {
        rate_hz: u16,
        lines: u32,
    },
    DemoIgnoresSigterm {
        rate_hz: u16,
        lines: u32,
    },
    /// Launch the real executable behind this approval id, subject to a
    /// `LaunchGate` decision immediately before launch. Carries no
    /// `rateHz`/`lines`: those parameters describe the synthetic demo
    /// harness's own emission pattern and mean nothing for a real,
    /// caller-supplied executable.
    Approved {
        approval_id: ApprovalIdDto,
    },
    /// Launch the built-in adapter named by `adapter_id` against the
    /// executable behind `approval_id`, delivering `prompt` -- added in
    /// the spike slice-3 spike (`docs/spike-log.md` § Slice 3).
    /// `adapter_id` is validated into a closed
    /// `omnifrons_domain::adapter::AdapterId` (-> `unknown-adapter` if
    /// unrecognized) and `prompt` into a size-capped `AgentPrompt` (->
    /// `prompt-too-large`/`invalid-request`) before either ever reaches
    /// the launch gate or an adapter's own `build_launch`.
    Adapter {
        adapter_id: String,
        approval_id: ApprovalIdDto,
        prompt: String,
    },
}

/// A workspace directory, as it crosses IPC. `display_path` is the
/// workspace's canonical filesystem path -- the second explicit exception
/// to RCS-001-R14's no-raw-path rule, alongside
/// [`EvidenceDto::canonical_path`] (`docs/spike-log.md` § Slice 3): both
/// are identity evidence flowing core -> renderer for display only (where
/// the agent actually runs), never a reference the renderer sends back,
/// and never sent to the harness process (which receives the workspace
/// only as its own working directory).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDto {
    pub display_path: String,
}

impl WorkspaceDto {
    #[must_use]
    pub fn from_workspace(workspace: &omnifrons_app::WorkspaceRoot) -> Self {
        Self {
            display_path: workspace.path().to_string_lossy().into_owned(),
        }
    }
}

/// [`omnifrons_domain::adapter::TransportClass`], as it crosses IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportClassDto {
    StructuredStreamingCli,
    Pty,
}

impl From<omnifrons_domain::adapter::TransportClass> for TransportClassDto {
    fn from(value: omnifrons_domain::adapter::TransportClass) -> Self {
        match value {
            omnifrons_domain::adapter::TransportClass::StructuredStreamingCli => {
                Self::StructuredStreamingCli
            }
            omnifrons_domain::adapter::TransportClass::Pty => Self::Pty,
        }
    }
}

/// [`omnifrons_domain::adapter::PromptChannel`], as it crosses IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptChannelDto {
    StdinThenClose,
    Argv,
}

impl From<omnifrons_domain::adapter::PromptChannel> for PromptChannelDto {
    fn from(value: omnifrons_domain::adapter::PromptChannel) -> Self {
        match value {
            omnifrons_domain::adapter::PromptChannel::StdinThenClose => Self::StdinThenClose,
            omnifrons_domain::adapter::PromptChannel::Argv => Self::Argv,
        }
    }
}

/// [`omnifrons_domain::scope::ScopeMode`], as it crosses IPC -- the scope
/// badge data `docs/spike-log.md` § Slice 3 keeps as `advisory` for every
/// built-in adapter in this slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScopeModeDto {
    SandboxEnforced,
    HarnessEnforced,
    Advisory,
}

impl From<omnifrons_domain::scope::ScopeMode> for ScopeModeDto {
    fn from(value: omnifrons_domain::scope::ScopeMode) -> Self {
        match value {
            omnifrons_domain::scope::ScopeMode::SandboxEnforced => Self::SandboxEnforced,
            omnifrons_domain::scope::ScopeMode::HarnessEnforced => Self::HarnessEnforced,
            omnifrons_domain::scope::ScopeMode::Advisory => Self::Advisory,
        }
    }
}

/// One built-in adapter's descriptor, as it crosses IPC -- metadata only:
/// deliberately never `argvTemplate` or `declaredEnv`'s resolved values,
/// which are this shell's own launch-time concern, never the renderer's
/// (`docs/spike-log.md` § Slice 3, mirroring the no-argv/no-path
/// discipline `docs/spike-log.md` § IPC contract already established for
/// every other command).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterDescriptorDto {
    pub id: String,
    pub display_name: String,
    pub transport_class: TransportClassDto,
    pub prompt_channel: PromptChannelDto,
    pub scope_mode: ScopeModeDto,
    pub notes: String,
}

impl AdapterDescriptorDto {
    #[must_use]
    pub fn from_descriptor(descriptor: &omnifrons_app::AdapterDescriptor) -> Self {
        Self {
            id: descriptor.id.as_str().to_string(),
            display_name: descriptor.display_name.clone(),
            transport_class: descriptor.transport_class.into(),
            prompt_channel: descriptor.prompt_channel.into(),
            scope_mode: descriptor.scope_mode.into(),
            notes: descriptor.notes.clone(),
        }
    }
}

/// A process identifier crossing IPC: transparently the platform process
/// id, matching [`omnifrons_app::ProcessId`]'s own transparency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdDto(pub u32);

impl From<omnifrons_app::ProcessId> for ProcessIdDto {
    fn from(id: omnifrons_app::ProcessId) -> Self {
        Self(id.0)
    }
}

impl From<ProcessIdDto> for omnifrons_app::ProcessId {
    fn from(id: ProcessIdDto) -> Self {
        Self(id.0)
    }
}

/// The closed set of terminal-state tokens, without the `exited` variant's
/// `code` -- see [`ProcessTerminalStateDto`], which pairs this with `code`
/// as an always-present sibling field rather than nesting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessTerminalStateTag {
    Exited,
    Killed,
    /// The literal wire token is `orphan-risk/uncertain` (with the slash),
    /// matching `docs/target-architecture.md` § Required failure states'
    /// own spelling of this outcome verbatim.
    #[serde(rename = "orphan-risk/uncertain")]
    OrphanRiskUncertain,
}

/// A process's terminal state, as it crosses IPC: `state` is always
/// present as one of the three closed tokens, and `code` is always
/// present too -- `null` except when `state` is `exited` and the platform
/// reported an exit code.
///
/// Deliberately flat (not an internally tagged enum with a per-variant
/// field set) so the exact same shape also flattens cleanly into
/// [`HarnessFrame::State`]'s `id`/`seq`/`droppedBefore` fields, and so
/// [`crate::ipc::commands::harness_stop`] can return it directly with no
/// extra wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProcessTerminalStateDto {
    pub state: ProcessTerminalStateTag,
    pub code: Option<i32>,
}

impl From<omnifrons_app::ProcessTerminalState> for ProcessTerminalStateDto {
    fn from(state: omnifrons_app::ProcessTerminalState) -> Self {
        use omnifrons_app::ProcessTerminalState as Domain;
        match state {
            Domain::Exited { code } => Self {
                state: ProcessTerminalStateTag::Exited,
                code,
            },
            Domain::Killed => Self {
                state: ProcessTerminalStateTag::Killed,
                code: None,
            },
            Domain::OrphanRiskUncertain => Self {
                state: ProcessTerminalStateTag::OrphanRiskUncertain,
                code: None,
            },
        }
    }
}

/// A process's observed status, as it crosses IPC: either still running,
/// or terminal with the same flat state/code shape as
/// [`ProcessTerminalStateDto`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ProcessStatusDto {
    Running,
    Terminal {
        #[serde(flatten)]
        terminal: ProcessTerminalStateDto,
    },
}

impl From<omnifrons_app::ProcessStatus> for ProcessStatusDto {
    fn from(status: omnifrons_app::ProcessStatus) -> Self {
        match status {
            omnifrons_app::ProcessStatus::Running => Self::Running,
            omnifrons_app::ProcessStatus::Terminal(state) => Self::Terminal {
                terminal: state.into(),
            },
        }
    }
}

/// [`omnifrons_domain::adapter::AgentPhase`]'s tag, as it crosses IPC --
/// paired with [`AdapterEventDto::State`]'s own `subtype` as an
/// always-present (`null` unless `finished`) sibling field, mirroring
/// [`ProcessTerminalStateDto`]'s own flat, always-present-`code` shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentPhaseTag {
    Init,
    Finished,
    Exited,
}

/// One named text observation accompanying an
/// [`AdapterEventDto::State`] event, as it crosses IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationDto {
    pub key: String,
    pub value: String,
}

/// [`omnifrons_domain::adapter::AdapterEvent`], as it crosses IPC.
/// Adjacently tagged (`kind` + `payload`), matching
/// [`HarnessFrame`]'s own outer `stream`/`body` shape one level down: the
/// literal `kind`/`payload` field names [`HarnessFrame::Event`] flattens
/// this into come from here.
///
/// `Unknown::raw` is rendered as a plain (lossily decoded) string, never
/// base64 or a byte array: every `raw` this slice's own adapters ever
/// produce originates from text that was already valid UTF-8 (a captured
/// line, or `LineAssembler`'s own buffer of concatenated valid-UTF-8
/// frames) -- lossy decoding is a defensive fallback, not the expected
/// path (`docs/spike-log.md` § Slice 3).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum AdapterEventDto {
    State {
        phase: AgentPhaseTag,
        subtype: Option<String>,
        observations: Vec<ObservationDto>,
    },
    Message {
        text: String,
    },
    ToolCall {
        name: String,
        arguments_text: String,
    },
    Diagnostic {
        text: String,
    },
    Unknown {
        raw: String,
        truncated: bool,
    },
}

impl AdapterEventDto {
    #[must_use]
    pub fn from_domain(event: omnifrons_domain::adapter::AdapterEvent) -> Self {
        use omnifrons_domain::adapter::{AdapterEvent, AgentPhase};
        match event {
            AdapterEvent::State {
                phase,
                observations,
            } => {
                let (phase_tag, subtype) = match phase {
                    AgentPhase::Init => (AgentPhaseTag::Init, None),
                    AgentPhase::Finished { subtype } => (AgentPhaseTag::Finished, Some(subtype)),
                    AgentPhase::Exited => (AgentPhaseTag::Exited, None),
                };
                Self::State {
                    phase: phase_tag,
                    subtype,
                    observations: observations
                        .into_iter()
                        .map(|(key, value)| ObservationDto { key, value })
                        .collect(),
                }
            }
            AdapterEvent::Message { text } => Self::Message { text },
            AdapterEvent::ToolCall(proposal) => Self::ToolCall {
                name: proposal.name,
                arguments_text: proposal.arguments_text,
            },
            AdapterEvent::Diagnostic { text } => Self::Diagnostic { text },
            AdapterEvent::Unknown { raw, truncated } => Self::Unknown {
                raw: String::from_utf8_lossy(&raw).into_owned(),
                truncated,
            },
        }
    }
}

/// One streamed frame delivered over `harness_spawn`'s `onFrame` channel.
///
/// Adjacently tagged: `{"stream": "stdout" | "stderr" | "state" | "event",
/// "body": {...}}`. `stdout`/`stderr` carry decoded text (never emitted
/// for an adapter launch -- see [`Self::event`]) plus `continued`, the
/// capturing side's own statement of whether more of the same logical
/// line follows in the next frame (a forced split at the 8 KiB per-frame
/// cap) -- additive as of spike slice 3, so a consumer that ignores it
/// keeps working (`docs/spike-log.md` § Slice 3); `state` carries the same
/// flat terminal-state shape as [`ProcessTerminalStateDto`], flattened in
/// alongside `id`/`seq`/`droppedBefore` rather than nested; `event`
/// (spike slice 3) carries an [`AdapterEventDto`], flattened the same way
/// -- its own `kind`/`payload` fields land directly on `event`'s body,
/// alongside `id`/`seq`/`droppedBefore`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "stream",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HarnessFrame {
    Stdout {
        id: ProcessIdDto,
        seq: u64,
        dropped_before: u64,
        text: String,
        continued: bool,
    },
    Stderr {
        id: ProcessIdDto,
        seq: u64,
        dropped_before: u64,
        text: String,
        continued: bool,
    },
    State {
        id: ProcessIdDto,
        seq: u64,
        dropped_before: u64,
        #[serde(flatten)]
        terminal: ProcessTerminalStateDto,
    },
    Event {
        id: ProcessIdDto,
        seq: u64,
        dropped_before: u64,
        #[serde(flatten)]
        event: AdapterEventDto,
    },
}

impl HarnessFrame {
    /// Build the wire frame for `id` from a captured
    /// [`omnifrons_app::OutputFrame`]. Used for a demo or approved-plain
    /// (non-adapter) launch, whose stdout/stderr are always sent raw,
    /// never parsed.
    #[must_use]
    pub fn from_domain(id: ProcessIdDto, frame: omnifrons_app::OutputFrame) -> Self {
        match frame.payload {
            omnifrons_app::FramePayload::Text {
                stream: omnifrons_app::OutputStream::Stdout,
                text,
                continued,
            } => Self::Stdout {
                id,
                seq: frame.seq,
                dropped_before: frame.dropped_before,
                text,
                continued,
            },
            omnifrons_app::FramePayload::Text {
                stream: omnifrons_app::OutputStream::Stderr,
                text,
                continued,
            } => Self::Stderr {
                id,
                seq: frame.seq,
                dropped_before: frame.dropped_before,
                text,
                continued,
            },
            omnifrons_app::FramePayload::State(state) => Self::State {
                id,
                seq: frame.seq,
                dropped_before: frame.dropped_before,
                terminal: state.into(),
            },
        }
    }

    /// Build an `event` wire frame for `id` from an
    /// [`omnifrons_domain::adapter::AdapterEvent`] an adapter's
    /// `parse_line` (or the shell's own stderr-to-`Diagnostic` mapping)
    /// produced. Used only for an adapter launch.
    #[must_use]
    pub fn event(
        id: ProcessIdDto,
        seq: u64,
        dropped_before: u64,
        event: omnifrons_domain::adapter::AdapterEvent,
    ) -> Self {
        Self::Event {
            id,
            seq,
            dropped_before,
            event: AdapterEventDto::from_domain(event),
        }
    }
}

/// The closed set of error codes a failed IPC command reports. Fixed and
/// exhaustive: every value the renderer can compare against structurally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShellErrorCode {
    UnknownProcess,
    SpawnFailed,
    AlreadySubscribed,
    InvalidRequest,
    TooManyProcesses,
    /// `executable_approve` was called for a candidate id this shell has
    /// no record of (never probed, or evicted from the bounded probed-
    /// candidate table), or `executable_pick_and_probe`'s file dialog
    /// returned nothing (canceled, or its path could not be resolved).
    NoCandidate,
    /// The re-probed target is not a regular, executable file.
    NotExecutable,
    /// The re-probed target could not be read at all.
    ProbeFailed,
    /// The re-probed target exceeds the probe's size cap.
    TooLarge,
    /// No approval is on record for the given id.
    Unapproved,
    /// The executable's content no longer matches what was approved. See
    /// [`ShellErrorDetail`] for the recorded/observed digests.
    ChangedSinceApproval,
    /// The approved path now resolves to a different canonical path.
    ShadowedPath,
    /// The approval on record for this id was revoked.
    Revoked,
    /// The approval store itself could not be read or written.
    ApprovalStoreUnavailable,
    /// `harness_spawn`'s `kind: "adapter"` named an `adapterId` outside
    /// the closed, built-in adapter set (spike slice 3).
    UnknownAdapter,
    /// An adapter launch's prompt exceeds the 16 KiB size cap.
    PromptTooLarge,
    /// An adapter launch was requested with no active workspace picked
    /// (`workspace_pick`), or an adapter's own `build_launch` reported the
    /// cwd it was given resolves outside the workspace.
    WorkspaceUnavailable,
    /// An adapter's own declared (or otherwise requested) environment
    /// variable name looks secret-shaped and was refused.
    SecretShapedEnv,
    /// `workspace_pick`'s folder dialog was canceled (no folder was
    /// selected).
    NoWorkspace,
}

/// Structured detail for a [`ShellError`], carrying values a fixed
/// catalogue `message` string must not (a full digest is not a secret,
/// but it does not belong interpolated into free text either) --
/// currently only [`ShellErrorCode::ChangedSinceApproval`] populates this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellErrorDetail {
    pub recorded_sha256_short: String,
    pub observed_sha256_short: String,
}

/// A failed IPC command's error payload. `message` is always a catalogue
/// string, never the underlying error's own text -- see
/// `crate::ipc::commands`' `SupervisorError`/`InvalidRequest`/
/// `DenialReason`/`ApprovalStoreError` mappings, which is where an
/// `io::Error` carrying a real path would otherwise leak
/// (`docs/spike-log.md` § IPC contract). `detail` is present only for the
/// handful of codes that carry structured, non-path data alongside the
/// message (see [`ShellErrorDetail`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShellError {
    pub code: ShellErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<ShellErrorDetail>,
}

impl ShellError {
    /// Build an error with no structured detail.
    pub(crate) fn new(code: ShellErrorCode, message: &str) -> Self {
        Self {
            code,
            message: message.to_string(),
            detail: None,
        }
    }

    /// Build an error carrying structured `detail`.
    pub(crate) fn with_detail(
        code: ShellErrorCode,
        message: &str,
        detail: ShellErrorDetail,
    ) -> Self {
        Self {
            code,
            message: message.to_string(),
            detail: Some(detail),
        }
    }

    /// Map a failed probe (never [`omnifrons_domain::executable::ProbeOutcome::Identity`],
    /// which is the success case) to its catalogue error. Shared by
    /// `executable_pick_and_probe`'s own probe and by
    /// [`omnifrons_domain::executable::DenialReason::ProbeFailed`] (a
    /// `LaunchGate::decide` re-probe failure), so the two surfaces never
    /// drift onto different codes for the same underlying outcome.
    pub(crate) fn from_probe_failure(outcome: &omnifrons_domain::executable::ProbeOutcome) -> Self {
        use omnifrons_domain::executable::ProbeOutcome;
        match outcome {
            ProbeOutcome::Unreadable => Self::new(
                ShellErrorCode::ProbeFailed,
                "the executable could not be read",
            ),
            ProbeOutcome::NotRegularFile | ProbeOutcome::NotExecutable => Self::new(
                ShellErrorCode::NotExecutable,
                "the target is not a regular, executable file",
            ),
            ProbeOutcome::TooLarge => Self::new(
                ShellErrorCode::TooLarge,
                "the executable exceeds the size limit",
            ),
            // Never actually produced by a real probe or a real
            // `DenialReason::ProbeFailed` (both only ever wrap a genuine
            // failure outcome) -- folded in defensively so this mapping
            // stays total over every value `ProbeOutcome` allows.
            ProbeOutcome::Identity(_) => Self::new(
                ShellErrorCode::ProbeFailed,
                "an unexpected probe result was returned",
            ),
        }
    }
}

/// A probed-but-not-yet-approved candidate's opaque handle. Valid only for
/// this shell process's own lifetime -- never persisted, never stable
/// across a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateIdDto(pub u64);

/// An approval id, as it crosses IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalIdDto(pub u64);

impl From<omnifrons_domain::executable::ApprovalId> for ApprovalIdDto {
    fn from(id: omnifrons_domain::executable::ApprovalId) -> Self {
        Self(id.0)
    }
}

impl From<ApprovalIdDto> for omnifrons_domain::executable::ApprovalId {
    fn from(id: ApprovalIdDto) -> Self {
        Self(id.0)
    }
}

/// [`omnifrons_domain::executable::PlatformEvidence`], as it crosses IPC.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "os", rename_all = "kebab-case")]
pub enum PlatformEvidenceDto {
    Unix { mode: u32 },
    Windows { extension: String, attributes: u32 },
}

impl From<&omnifrons_domain::executable::PlatformEvidence> for PlatformEvidenceDto {
    fn from(evidence: &omnifrons_domain::executable::PlatformEvidence) -> Self {
        use omnifrons_domain::executable::PlatformEvidence as Domain;
        match evidence {
            Domain::Unix { mode } => Self::Unix { mode: *mode },
            Domain::Windows {
                extension,
                attributes,
            } => Self::Windows {
                extension: extension.clone(),
                attributes: *attributes,
            },
        }
    }
}

/// Milliseconds since the Unix epoch. Saturates to `0` for a time before
/// the epoch (never expected from a real platform-reported timestamp, but
/// avoids a panic rather than asserting it can never happen) and to
/// [`u64::MAX`] if a duration since the epoch somehow overflowed `u64`
/// milliseconds (equally never expected before the year 292 million or
/// so).
fn system_time_to_millis(time: std::time::SystemTime) -> u64 {
    time.duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

/// An executable identity's evidence, as it crosses IPC.
///
/// `canonical_path` is one of exactly two places a filesystem path
/// reaches the renderer in this surface, both identity evidence shown to
/// the user, never a reference the renderer could send back: this one
/// (the executable's canonical path, `docs/spike-log.md` § Slice 2,
/// TM-001-R7 display) and [`WorkspaceDto::display_path`] (the workspace
/// the agent runs in, `docs/spike-log.md` § Slice 3). The raw path a user
/// picked in the OS file dialog (which may differ, e.g. a symlink) is
/// never returned.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceDto {
    pub canonical_path: String,
    pub size: u64,
    pub sha256: String,
    pub sha256_short: String,
    pub modified_at: Option<u64>,
    pub platform: PlatformEvidenceDto,
}

impl EvidenceDto {
    #[must_use]
    pub fn from_identity(identity: &omnifrons_domain::executable::ExecutableIdentity) -> Self {
        Self {
            canonical_path: identity.canonical_path.to_string_lossy().into_owned(),
            size: identity.size,
            sha256: identity.sha256.to_hex(),
            sha256_short: identity.sha256.short_hex(),
            modified_at: identity.modified_at.map(system_time_to_millis),
            platform: PlatformEvidenceDto::from(&identity.platform),
        }
    }
}

/// `executable_pick_and_probe`'s success payload.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResultDto {
    pub candidate_id: CandidateIdDto,
    pub evidence: EvidenceDto,
}

/// An approval's status tag, as it crosses IPC -- paired with
/// [`ApprovalDto::revoked_at`] as an always-present sibling field, mirroring
/// [`ProcessTerminalStateDto`]'s own flat, always-present-`code` shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalStatusTag {
    Active,
    Revoked,
}

/// One approval on record, as it crosses IPC.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDto {
    pub approval_id: ApprovalIdDto,
    pub evidence: EvidenceDto,
    pub approved_at: u64,
    pub status: ApprovalStatusTag,
    pub revoked_at: Option<u64>,
}

impl ApprovalDto {
    #[must_use]
    pub fn from_record(record: &omnifrons_domain::executable::ApprovalRecord) -> Self {
        use omnifrons_domain::executable::ApprovalStatus;
        let (status, revoked_at) = match record.status {
            ApprovalStatus::Active => (ApprovalStatusTag::Active, None),
            ApprovalStatus::Revoked { revoked_at } => (
                ApprovalStatusTag::Revoked,
                Some(system_time_to_millis(revoked_at)),
            ),
        };
        Self {
            approval_id: record.approval_id.into(),
            evidence: EvidenceDto::from_identity(&record.identity),
            approved_at: system_time_to_millis(record.approved_at),
            status,
            revoked_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AdapterDescriptorDto, ApprovalDto, ApprovalIdDto, ApprovalStatusTag, CandidateIdDto,
        EvidenceDto, HarnessFrame, HarnessKindDto, PlatformEvidenceDto, ProbeResultDto,
        ProcessIdDto, ProcessStatusDto, ProcessTerminalStateDto, ProcessTerminalStateTag,
        PromptChannelDto, ScopeModeDto, ShellError, ShellErrorCode, ShellErrorDetail,
        TransportClassDto, WorkspaceDto,
    };

    fn json(value: &impl serde::Serialize) -> serde_json::Value {
        serde_json::to_value(value).expect("serialization must succeed")
    }

    #[test]
    fn harness_kind_dto_deserializes_from_tagged_objects() {
        let demo_lines: HarnessKindDto =
            serde_json::from_str(r#"{"type":"demo-lines","rateHz":10,"lines":5}"#)
                .expect("demo-lines must deserialize");
        assert_eq!(
            demo_lines,
            HarnessKindDto::DemoLines {
                rate_hz: 10,
                lines: 5
            }
        );

        let ignores_sigterm: HarnessKindDto =
            serde_json::from_str(r#"{"type":"demo-ignores-sigterm","rateHz":20,"lines":10}"#)
                .expect("demo-ignores-sigterm must deserialize");
        assert_eq!(
            ignores_sigterm,
            HarnessKindDto::DemoIgnoresSigterm {
                rate_hz: 20,
                lines: 10
            }
        );

        let approved: HarnessKindDto =
            serde_json::from_str(r#"{"type":"approved","approvalId":42}"#)
                .expect("approved must deserialize");
        assert_eq!(
            approved,
            HarnessKindDto::Approved {
                approval_id: ApprovalIdDto(42)
            }
        );
    }

    /// R1-006 / R3-010: an unrecognized `type` tag must be rejected, not
    /// silently coerced into some default variant.
    #[test]
    fn harness_kind_dto_rejects_an_unknown_type_tag() {
        let result: Result<HarnessKindDto, _> =
            serde_json::from_str(r#"{"type":"not-a-real-kind","rateHz":10,"lines":5}"#);
        assert!(
            result.is_err(),
            "an unrecognized type tag must be rejected, got {result:?}"
        );
    }

    /// R1-006 / R3-010: `approved` without its required `approvalId`
    /// field must be rejected, not default to some placeholder id.
    #[test]
    fn harness_kind_dto_rejects_approved_missing_approval_id() {
        let result: Result<HarnessKindDto, _> = serde_json::from_str(r#"{"type":"approved"}"#);
        assert!(
            result.is_err(),
            "approved with no approvalId must be rejected, got {result:?}"
        );
    }

    /// R1-006 / R3-010: `#[serde(deny_unknown_fields)]` on `HarnessKindDto`
    /// means an extra, unexpected field on any variant is rejected too --
    /// serde's own default (silently ignoring an unknown field) would
    /// otherwise let a caller send e.g. `approved` with a stray `rateHz`
    /// and have it silently dropped rather than surfaced.
    #[test]
    fn harness_kind_dto_rejects_an_extra_field_on_approved() {
        let result: Result<HarnessKindDto, _> =
            serde_json::from_str(r#"{"type":"approved","approvalId":42,"rateHz":10}"#);
        assert!(
            result.is_err(),
            "an extra field on approved must be rejected, got {result:?}"
        );
    }

    #[test]
    fn harness_kind_dto_deserializes_the_adapter_variant() {
        let adapter: HarnessKindDto = serde_json::from_str(
            r#"{"type":"adapter","adapterId":"claude-code","approvalId":42,"prompt":"do the thing"}"#,
        )
        .expect("adapter must deserialize");
        assert_eq!(
            adapter,
            HarnessKindDto::Adapter {
                adapter_id: "claude-code".to_string(),
                approval_id: ApprovalIdDto(42),
                prompt: "do the thing".to_string(),
            }
        );
    }

    #[test]
    fn harness_kind_dto_rejects_an_extra_field_on_adapter() {
        let result: Result<HarnessKindDto, _> = serde_json::from_str(
            r#"{"type":"adapter","adapterId":"claude-code","approvalId":42,"prompt":"x","rateHz":10}"#,
        );
        assert!(
            result.is_err(),
            "an extra field on adapter must be rejected, got {result:?}"
        );
    }

    #[test]
    fn harness_kind_dto_rejects_adapter_missing_prompt() {
        let result: Result<HarnessKindDto, _> =
            serde_json::from_str(r#"{"type":"adapter","adapterId":"claude-code","approvalId":42}"#);
        assert!(
            result.is_err(),
            "adapter missing its required prompt field must be rejected, got {result:?}"
        );
    }

    #[test]
    fn process_id_dto_serializes_as_a_bare_number() {
        assert_eq!(json(&ProcessIdDto(42)), serde_json::json!(42));
    }

    #[test]
    fn process_terminal_state_dto_json_shapes() {
        assert_eq!(
            json(&ProcessTerminalStateDto {
                state: ProcessTerminalStateTag::Exited,
                code: Some(0),
            }),
            serde_json::json!({"state": "exited", "code": 0})
        );
        assert_eq!(
            json(&ProcessTerminalStateDto {
                state: ProcessTerminalStateTag::Killed,
                code: None,
            }),
            serde_json::json!({"state": "killed", "code": null})
        );
        assert_eq!(
            json(&ProcessTerminalStateDto {
                state: ProcessTerminalStateTag::OrphanRiskUncertain,
                code: None,
            }),
            serde_json::json!({"state": "orphan-risk/uncertain", "code": null}),
            "the orphan-risk/uncertain token must be transcribed with its slash intact"
        );
    }

    #[test]
    fn process_status_dto_json_shapes() {
        assert_eq!(
            json(&ProcessStatusDto::Running),
            serde_json::json!({"status": "running"})
        );
        assert_eq!(
            json(&ProcessStatusDto::Terminal {
                terminal: ProcessTerminalStateDto {
                    state: ProcessTerminalStateTag::Killed,
                    code: None,
                },
            }),
            serde_json::json!({"status": "terminal", "state": "killed", "code": null})
        );
    }

    /// R3-003: a text frame carries `continued` on the wire (camelCase,
    /// additive), `false` for a frame ending at a genuine line end.
    #[test]
    fn harness_frame_stdout_json_shape() {
        let frame = HarnessFrame::Stdout {
            id: ProcessIdDto(7),
            seq: 3,
            dropped_before: 0,
            text: "line 1 out".to_string(),
            continued: false,
        };
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "stdout",
                "body": {"id": 7, "seq": 3, "droppedBefore": 0, "text": "line 1 out", "continued": false}
            })
        );
    }

    #[test]
    fn harness_frame_with_nonzero_dropped_before() {
        let frame = HarnessFrame::Stderr {
            id: ProcessIdDto(7),
            seq: 41,
            dropped_before: 12,
            text: "line 205 err".to_string(),
            continued: false,
        };
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "stderr",
                "body": {"id": 7, "seq": 41, "droppedBefore": 12, "text": "line 205 err", "continued": false}
            })
        );
    }

    /// R3-003: a forced-split domain frame's `continued: true` reaches the
    /// raw (demo/approved) wire shape unchanged via `from_domain`.
    #[test]
    fn harness_frame_from_domain_carries_a_split_frames_continued_flag() {
        let frame = HarnessFrame::from_domain(
            ProcessIdDto(7),
            omnifrons_app::OutputFrame::new(
                4,
                omnifrons_app::FramePayload::Text {
                    stream: omnifrons_app::OutputStream::Stdout,
                    text: "first half of a long line".to_string(),
                    continued: true,
                },
            ),
        );
        assert_eq!(json(&frame)["body"]["continued"], serde_json::json!(true));
        assert_eq!(json(&frame)["stream"], serde_json::json!("stdout"));
    }

    #[test]
    fn harness_frame_state_json_shape() {
        let frame = HarnessFrame::State {
            id: ProcessIdDto(7),
            seq: 13,
            dropped_before: 0,
            terminal: ProcessTerminalStateDto {
                state: ProcessTerminalStateTag::Exited,
                code: Some(0),
            },
        };
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "state",
                "body": {"id": 7, "seq": 13, "droppedBefore": 0, "state": "exited", "code": 0}
            })
        );
    }

    #[test]
    fn too_many_processes_code_serializes_as_kebab_case() {
        let error = ShellError::new(
            ShellErrorCode::TooManyProcesses,
            "too many processes are already running",
        );
        assert_eq!(
            json(&error),
            serde_json::json!({
                "code": "too-many-processes",
                "message": "too many processes are already running"
            })
        );
    }

    #[test]
    fn shell_error_json_shape() {
        let error = ShellError::new(
            ShellErrorCode::SpawnFailed,
            "failed to start the requested process",
        );
        assert_eq!(
            json(&error),
            serde_json::json!({"code": "spawn-failed", "message": "failed to start the requested process"})
        );
    }

    #[test]
    fn shell_error_with_detail_includes_the_detail_object() {
        let error = ShellError::with_detail(
            ShellErrorCode::ChangedSinceApproval,
            "the executable's content has changed since it was approved",
            ShellErrorDetail {
                recorded_sha256_short: "aaaaaaaa".to_string(),
                observed_sha256_short: "bbbbbbbb".to_string(),
            },
        );
        assert_eq!(
            json(&error),
            serde_json::json!({
                "code": "changed-since-approval",
                "message": "the executable's content has changed since it was approved",
                "detail": {
                    "recordedSha256Short": "aaaaaaaa",
                    "observedSha256Short": "bbbbbbbb",
                }
            })
        );
    }

    /// Every catalogue message this slice introduces (via `ShellError::new`/
    /// `with_detail`/`from_probe_failure` -- the fixed catalogue strings
    /// defined in this module) must contain no `/`, matching the existing
    /// no-underlying-path-leak requirement `docs/spike-log.md` § Slice 2
    /// restates for these new codes.
    #[test]
    fn every_new_shell_error_code_message_has_no_slash() {
        let messages = [
            ShellError::new(ShellErrorCode::NoCandidate, "no file was selected").message,
            ShellError::from_probe_failure(&omnifrons_domain::executable::ProbeOutcome::Unreadable)
                .message,
            ShellError::from_probe_failure(
                &omnifrons_domain::executable::ProbeOutcome::NotRegularFile,
            )
            .message,
            ShellError::from_probe_failure(
                &omnifrons_domain::executable::ProbeOutcome::NotExecutable,
            )
            .message,
            ShellError::from_probe_failure(&omnifrons_domain::executable::ProbeOutcome::TooLarge)
                .message,
            ShellError::new(
                ShellErrorCode::Unapproved,
                "this executable has not been approved",
            )
            .message,
            ShellError::new(
                ShellErrorCode::Revoked,
                "the approval for this executable was revoked",
            )
            .message,
            ShellError::new(
                ShellErrorCode::ShadowedPath,
                "the approved path now resolves somewhere else",
            )
            .message,
            ShellError::new(
                ShellErrorCode::ApprovalStoreUnavailable,
                "the approval store is unavailable",
            )
            .message,
        ];

        for message in messages {
            assert!(
                !message.contains('/'),
                "catalogue message must contain no '/', got: {message}"
            );
        }
    }

    #[test]
    fn probe_result_dto_json_shape() {
        let dto = ProbeResultDto {
            candidate_id: CandidateIdDto(1),
            evidence: EvidenceDto {
                canonical_path: "/opt/tool/app".to_string(),
                size: 4096,
                sha256: "a".repeat(64),
                sha256_short: "aaaaaaaa".to_string(),
                modified_at: Some(1_000),
                platform: PlatformEvidenceDto::Unix { mode: 0o755 },
            },
        };
        assert_eq!(
            json(&dto),
            serde_json::json!({
                "candidateId": 1,
                "evidence": {
                    "canonicalPath": "/opt/tool/app",
                    "size": 4096,
                    "sha256": "a".repeat(64),
                    "sha256Short": "aaaaaaaa",
                    "modifiedAt": 1000,
                    "platform": {"os": "unix", "mode": 0o755},
                }
            })
        );
    }

    #[test]
    fn approval_dto_json_shape_for_active_and_revoked() {
        let evidence = EvidenceDto {
            canonical_path: "/opt/tool/app".to_string(),
            size: 4096,
            sha256: "a".repeat(64),
            sha256_short: "aaaaaaaa".to_string(),
            modified_at: None,
            platform: PlatformEvidenceDto::Unix { mode: 0o755 },
        };

        let active = ApprovalDto {
            approval_id: ApprovalIdDto(7),
            evidence: evidence.clone(),
            approved_at: 500,
            status: ApprovalStatusTag::Active,
            revoked_at: None,
        };
        assert_eq!(json(&active)["status"], serde_json::json!("active"));
        assert_eq!(json(&active)["revokedAt"], serde_json::json!(null));

        let revoked = ApprovalDto {
            approval_id: ApprovalIdDto(7),
            evidence,
            approved_at: 500,
            status: ApprovalStatusTag::Revoked,
            revoked_at: Some(600),
        };
        assert_eq!(json(&revoked)["status"], serde_json::json!("revoked"));
        assert_eq!(json(&revoked)["revokedAt"], serde_json::json!(600));
    }

    // -- Slice 3: workspace, adapters, and the "event" harness frame --

    #[test]
    fn workspace_dto_json_shape() {
        let dto = WorkspaceDto {
            display_path: "/home/user/project".to_string(),
        };
        assert_eq!(
            json(&dto),
            serde_json::json!({"displayPath": "/home/user/project"})
        );
    }

    #[test]
    fn adapter_descriptor_dto_json_shape_has_no_argv_or_env() {
        let dto = AdapterDescriptorDto {
            id: "claude-code".to_string(),
            display_name: "Claude Code".to_string(),
            transport_class: TransportClassDto::StructuredStreamingCli,
            prompt_channel: PromptChannelDto::StdinThenClose,
            scope_mode: ScopeModeDto::Advisory,
            notes: "credentials are harness-owned; not exercised in CI".to_string(),
        };
        let value = json(&dto);
        assert_eq!(
            value,
            serde_json::json!({
                "id": "claude-code",
                "displayName": "Claude Code",
                "transportClass": "structured-streaming-cli",
                "promptChannel": "stdin-then-close",
                "scopeMode": "advisory",
                "notes": "credentials are harness-owned; not exercised in CI",
            })
        );
        let keys: std::collections::BTreeSet<&str> = value
            .as_object()
            .expect("must be an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert!(
            !keys.contains("argvTemplate") && !keys.contains("declaredEnv"),
            "adapters_list's DTO must never carry argv or declared-env values, got keys {keys:?}"
        );
    }

    #[test]
    fn harness_frame_event_state_json_shape() {
        let event = super::AdapterEventDto::State {
            phase: super::AgentPhaseTag::Finished,
            subtype: Some("success".to_string()),
            observations: vec![],
        };
        let frame = HarnessFrame::Event {
            id: ProcessIdDto(7),
            seq: 3,
            dropped_before: 0,
            event,
        };
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "event",
                "body": {
                    "id": 7,
                    "seq": 3,
                    "droppedBefore": 0,
                    "kind": "state",
                    "payload": {"phase": "finished", "subtype": "success", "observations": []},
                }
            })
        );
    }

    #[test]
    fn harness_frame_event_via_constructor_from_domain_adapter_event() {
        let frame = HarnessFrame::event(
            ProcessIdDto(1),
            0,
            0,
            omnifrons_domain::adapter::AdapterEvent::Diagnostic {
                text: "hello".to_string(),
            },
        );
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "event",
                "body": {"id": 1, "seq": 0, "droppedBefore": 0, "kind": "diagnostic", "payload": {"text": "hello"}}
            })
        );
    }

    /// R3-006: the `message` event's documented wire shape.
    #[test]
    fn harness_frame_event_message_json_shape() {
        let frame = HarnessFrame::event(
            ProcessIdDto(42),
            4,
            0,
            omnifrons_domain::adapter::AdapterEvent::Message {
                text: "hello".to_string(),
            },
        );
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "event",
                "body": {
                    "id": 42, "seq": 4, "droppedBefore": 0,
                    "kind": "message", "payload": {"text": "hello"}
                }
            })
        );
    }

    /// R3-006: the `tool-call` event's documented wire shape, including
    /// the camelCase `argumentsText` and the kebab-case `kind` token.
    #[test]
    fn harness_frame_event_tool_call_json_shape() {
        let frame = HarnessFrame::event(
            ProcessIdDto(42),
            5,
            0,
            omnifrons_domain::adapter::AdapterEvent::ToolCall(
                omnifrons_domain::adapter::ToolCallProposal {
                    name: "write_file".to_string(),
                    arguments_text: r#"{"path":"notes.md"}"#.to_string(),
                },
            ),
        );
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "event",
                "body": {
                    "id": 42, "seq": 5, "droppedBefore": 0,
                    "kind": "tool-call",
                    "payload": {"name": "write_file", "argumentsText": "{\"path\":\"notes.md\"}"}
                }
            })
        );
    }

    #[test]
    fn harness_frame_event_unknown_carries_truncated_flag() {
        let frame = HarnessFrame::event(
            ProcessIdDto(1),
            0,
            0,
            omnifrons_domain::adapter::AdapterEvent::Unknown {
                raw: b"not json".to_vec(),
                truncated: true,
            },
        );
        assert_eq!(
            json(&frame)["body"]["payload"],
            serde_json::json!({"raw": "not json", "truncated": true})
        );
    }

    /// The five new slice-3 error codes each render as their documented
    /// kebab-case token.
    #[test]
    fn slice_3_error_codes_serialize_as_kebab_case() {
        let cases = [
            (ShellErrorCode::UnknownAdapter, "unknown-adapter"),
            (ShellErrorCode::PromptTooLarge, "prompt-too-large"),
            (
                ShellErrorCode::WorkspaceUnavailable,
                "workspace-unavailable",
            ),
            (ShellErrorCode::SecretShapedEnv, "secret-shaped-env"),
            (ShellErrorCode::NoWorkspace, "no-workspace"),
        ];
        for (code, token) in cases {
            let error = ShellError::new(code, "message");
            assert_eq!(json(&error)["code"], serde_json::json!(token));
        }
    }
}
