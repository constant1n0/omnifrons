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
    /// Whether the product work area still resolves outside this
    /// workspace: re-checked on every workspace registration (HAP-001-R7;
    /// spike slice 5b), so the next publication command is not the first
    /// to notice a workspace registered over it.
    pub work_area: WorkAreaStateTag,
}

impl WorkspaceDto {
    #[must_use]
    pub fn new(workspace: &omnifrons_app::WorkspaceRoot, work_area: WorkAreaStateTag) -> Self {
        Self {
            display_path: workspace.path().to_string_lossy().into_owned(),
            work_area,
        }
    }
}

/// The product work area's state against the active workspace, as it
/// crosses IPC (spike slice 5b): HAP-001's `work-area-invalid` token, or
/// `valid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkAreaStateTag {
    Valid,
    WorkAreaInvalid,
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
/// `pty-typed` (spike slice 4) is the `pty-cli` adapter's channel: the
/// prompt is typed into the child's controlling terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptChannelDto {
    StdinThenClose,
    Argv,
    PtyTyped,
}

impl From<omnifrons_domain::adapter::PromptChannel> for PromptChannelDto {
    fn from(value: omnifrons_domain::adapter::PromptChannel) -> Self {
        match value {
            omnifrons_domain::adapter::PromptChannel::StdinThenClose => Self::StdinThenClose,
            omnifrons_domain::adapter::PromptChannel::Argv => Self::Argv,
            omnifrons_domain::adapter::PromptChannel::PtyTyped => Self::PtyTyped,
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

/// The closed set of typed terminal actions a PTY launch's output may
/// produce (spike slice 4): the only two sequence families RCS-001's
/// terminal policy turns into a value, both already sanitized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalActionKindDto {
    Title,
    Notification,
}

/// One entry an `artifact.publish` proposal names, as it crosses IPC: the
/// harness's own claim -- a name and the full digest it asserts -- never
/// a fact Omnifrons verified (spike slice 5, HAP-001-R12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposedEntryDto {
    pub name: String,
    pub sha256: String,
}

/// [`omnifrons_domain::adapter::AdapterEvent`], as it crosses IPC.
/// Adjacently tagged (`kind` + `payload`), matching
/// [`HarnessFrame`]'s own outer `stream`/`body` shape one level down: the
/// literal `kind`/`payload` field names [`HarnessFrame::Event`] flattens
/// this into come from here.
///
/// Three kinds are the shell's own rather than an adapter's:
/// `artifact-publish` is the typed proposal a line agent recognized
/// (spike slice 5); `candidates` is emitted once per adapter launch at run
/// end, before the terminal `state` frame, summarizing the run
/// subdirectory's inventory by state and attribution; and `misplaced`
/// (spike slice 5d) follows it with the wrong-root scan's own counts --
/// HAP-001 D16's post-run scan. Both carry counts only, never a name and
/// never a path (`docs/spike-log.md` § Slice 5, § Slice 5d).
///
/// `Unknown::raw` is rendered as a plain (lossily decoded) string, never
/// base64 or a byte array: every `raw` this slice's own adapters ever
/// produce originates from text that was already valid UTF-8 (a captured
/// line, or `LineAssembler`'s own buffer of concatenated valid-UTF-8
/// frames) -- lossy decoding is a defensive fallback, not the expected
/// path (`docs/spike-log.md` § Slice 3).
///
/// The three `terminal-*` kinds (spike slice 4) are proposed AEC-001 kinds
/// for a PTY launch, emitted only by the shell's PTY forwarder: normalized
/// plain text (newlines kept), a sanitized title/notification shown as
/// text only, and the per-family counts of dropped sequences since the
/// previous `terminal-drops` of the same launch (`docs/spike-log.md` §
/// Slice 4).
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
    TerminalText {
        text: String,
    },
    TerminalAction {
        action: TerminalActionKindDto,
        text: String,
    },
    TerminalDrops {
        layout: u64,
        hyperlink: u64,
        clipboard: u64,
        file_transfer: u64,
        string: u64,
        unknown: u64,
        malformed: u64,
    },
    ArtifactPublish {
        entries: Vec<ProposedEntryDto>,
    },
    Candidates {
        run_id: String,
        total: u32,
        candidate: u32,
        outbox_escape: u32,
        outbox_linked: u32,
        attributed: u32,
        unattributed: u32,
        unreadable: u32,
        unmatched_proposals: u32,
    },
    Misplaced {
        scanned: u32,
        findings: u32,
        ignored: u32,
        excluded: u32,
        unreadable: u32,
        truncated: bool,
    },
}

impl AdapterEventDto {
    #[must_use]
    pub fn from_domain(event: omnifrons_domain::adapter::AdapterEvent) -> Self {
        use omnifrons_domain::adapter::{AdapterEvent, AgentPhase};
        match event {
            AdapterEvent::ArtifactPublish(proposal) => Self::ArtifactPublish {
                entries: proposal
                    .entries
                    .into_iter()
                    .map(|entry| ProposedEntryDto {
                        name: entry.name,
                        sha256: entry.sha256.to_hex(),
                    })
                    .collect(),
            },
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
            AdapterEvent::TerminalText { text } => Self::TerminalText { text },
            AdapterEvent::TerminalAction(action) => {
                use omnifrons_domain::terminal::TerminalAction;
                let (action, text) = match action {
                    TerminalAction::Title(text) => (TerminalActionKindDto::Title, text),
                    TerminalAction::Notification(text) => {
                        (TerminalActionKindDto::Notification, text)
                    }
                };
                Self::TerminalAction { action, text }
            }
            AdapterEvent::TerminalDrops(counts) => Self::TerminalDrops {
                layout: counts.layout,
                hyperlink: counts.hyperlink,
                clipboard: counts.clipboard,
                file_transfer: counts.file_transfer,
                string: counts.string,
                unknown: counts.unknown,
                malformed: counts.malformed,
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
        Self::event_dto(id, seq, dropped_before, AdapterEventDto::from_domain(event))
    }

    /// Build an `event` wire frame for `id` from an already-shaped
    /// [`AdapterEventDto`] -- the shell's own `candidates` summary and
    /// the diagnostics it emits at run end (spike slice 5), which have no
    /// domain `AdapterEvent` because no adapter produced them.
    #[must_use]
    pub fn event_dto(
        id: ProcessIdDto,
        seq: u64,
        dropped_before: u64,
        event: AdapterEventDto,
    ) -> Self {
        Self::Event {
            id,
            seq,
            dropped_before,
            event,
        }
    }
}

/// The outbox's state for the active workspace, as it crosses IPC
/// (`outbox_status`, spike slice 5): the three tokens HAP-001's signal
/// mapping assigns to the declaration and the pre-creation check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutboxStateTag {
    /// The declaration is valid; the outbox exists as a real directory
    /// inside the project, or does not exist yet and the first adapter
    /// launch creates it.
    Valid,
    /// The declared path resolves outside the project root or is a link,
    /// or the policy declaring it could not be loaded (HAP-001-R8).
    OutboxInvalid,
    /// Something that is not a directory sits at the declared path, or
    /// its metadata could not be read (HAP-001-R10).
    OutboxUnavailable,
}

/// Why `outbox_status` reports a non-`valid` state: a fixed token, never
/// the underlying error's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutboxReasonTag {
    OutsideProject,
    Link,
    NotADirectory,
    Unreadable,
    PolicyUnreadable,
    PolicyCorrupt,
    PolicyInvalid,
}

/// `outbox_status`'s payload. `outbox` is the outbox's canonical
/// filesystem path, present only when the declaration is valid and the
/// directory exists: identity evidence flowing core -> renderer for
/// display (where a run's output lands), the third explicit exception to
/// RCS-001-R14's no-raw-path rule alongside [`EvidenceDto::canonical_path`]
/// and [`WorkspaceDto::display_path`] -- never a reference the renderer
/// sends back, and never shown for a directory that failed validation.
/// `declared` and `policy_path` are project-relative names, not device
/// paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxStatusDto {
    pub declared: Option<String>,
    pub outbox: Option<String>,
    pub exists: bool,
    pub state: OutboxStateTag,
    pub reason: Option<OutboxReasonTag>,
    pub policy_path: String,
    /// The policy's asset root identity token (spike slice 5b): the
    /// destination HAP-001-R22 shows before the decision, never a path;
    /// `null` when the policy declares none or could not be loaded.
    pub asset_root_id: Option<String>,
}

/// [`omnifrons_domain::outbox::CandidateState`], as it crosses IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateStateTag {
    Candidate,
    OutboxEscape,
    OutboxLinked,
}

impl From<omnifrons_domain::outbox::CandidateState> for CandidateStateTag {
    fn from(state: omnifrons_domain::outbox::CandidateState) -> Self {
        use omnifrons_domain::outbox::CandidateState;
        match state {
            CandidateState::Candidate => Self::Candidate,
            CandidateState::OutboxEscape => Self::OutboxEscape,
            CandidateState::OutboxLinked => Self::OutboxLinked,
        }
    }
}

/// [`omnifrons_domain::outbox::Attribution`], as it crosses IPC:
/// `{"kind": "run", "runId": ...}` or `{"kind": "unattributed"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum AttributionDto {
    Run { run_id: String },
    Unattributed,
}

/// One candidate entry, as `candidates_list` returns it (spike slice 5):
/// its name relative to the outbox (a producer-supplied name, plain text
/// only), the facts taken from its handle -- `null` for a refused entry,
/// where nothing was digested (HAP-001-R20) -- its attribution, and its
/// state. Never a device path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateDto {
    pub name: String,
    pub size: Option<u64>,
    /// The full 64-hex content digest (spike slice 5b): the identity fact
    /// `artifact_approve` names the candidate by, so an approval is made
    /// from the row itself. Identity evidence, not a secret -- the
    /// executable evidence (`EvidenceDto::sha256`) already carries its own
    /// under the same TM-001-R7 display precedent -- and `null` for a
    /// refused entry, where nothing was digested.
    pub sha256: Option<String>,
    pub sha256_short: Option<String>,
    pub detected_type: Option<String>,
    pub class: Option<String>,
    pub attribution: AttributionDto,
    pub state: CandidateStateTag,
}

impl CandidateDto {
    #[must_use]
    pub fn from_candidate(
        entry: &omnifrons_domain::outbox::CandidateEntry,
        state: omnifrons_domain::outbox::CandidateState,
    ) -> Self {
        use omnifrons_domain::outbox::{Attribution, CandidateState};
        let digested = state == CandidateState::Candidate;
        Self {
            name: entry.name.clone(),
            size: digested.then_some(entry.size),
            sha256: digested.then(|| entry.digest.to_hex()),
            sha256_short: digested.then(|| entry.digest.short_hex()),
            detected_type: digested.then(|| entry.detected_type.as_str().to_string()),
            class: digested.then(|| entry.class.as_str().to_string()),
            attribution: match &entry.attribution {
                Attribution::Run(run_id) => AttributionDto::Run {
                    run_id: run_id.as_str().to_string(),
                },
                Attribution::Unattributed => AttributionDto::Unattributed,
            },
            state: state.into(),
        }
    }
}

/// [`omnifrons_domain::publication::ArtifactState`], as it crosses IPC
/// (spike slice 5b): the closed state tokens HAP-001's signal mapping
/// spells, exactly (HAP-001-R26).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactStateTag {
    Candidate,
    PublishedLocal,
    Registered,
    ProviderSynced,
    RegistrationPending,
    Refused,
    IntegrityMismatch,
    DuplicatePublication,
    OutboxEscape,
    OutboxLinked,
}

impl From<omnifrons_domain::publication::ArtifactState> for ArtifactStateTag {
    fn from(state: omnifrons_domain::publication::ArtifactState) -> Self {
        use omnifrons_domain::publication::ArtifactState;
        match state {
            ArtifactState::Candidate => Self::Candidate,
            ArtifactState::PublishedLocal => Self::PublishedLocal,
            ArtifactState::Registered => Self::Registered,
            ArtifactState::ProviderSynced => Self::ProviderSynced,
            ArtifactState::RegistrationPending => Self::RegistrationPending,
            ArtifactState::Refused => Self::Refused,
            ArtifactState::IntegrityMismatch => Self::IntegrityMismatch,
            ArtifactState::DuplicatePublication => Self::DuplicatePublication,
            ArtifactState::OutboxEscape => Self::OutboxEscape,
            ArtifactState::OutboxLinked => Self::OutboxLinked,
        }
    }
}

/// [`omnifrons_domain::publication::ProviderState`], as it crosses IPC:
/// the record's `provider_state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderStateTag {
    Pending,
    Synced,
    Failed,
    Unavailable,
}

impl From<omnifrons_domain::publication::ProviderState> for ProviderStateTag {
    fn from(state: omnifrons_domain::publication::ProviderState) -> Self {
        use omnifrons_domain::publication::ProviderState;
        match state {
            ProviderState::Pending => Self::Pending,
            ProviderState::Synced => Self::Synced,
            ProviderState::Failed => Self::Failed,
            ProviderState::Unavailable => Self::Unavailable,
        }
    }
}

/// This device's own availability observation for a publication
/// (HAP-001-R27): `local` when this device's journal shows the bytes
/// verified here, `unknown` otherwise. Never inferred from the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AvailabilityTag {
    Local,
    Unknown,
}

/// The act-as identity an approval binds (HAP-001-R22): the device-local
/// user, opaquely -- `omnifrons_domain::executable::DeviceLocalUser` as a
/// wire token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActAsTag {
    DeviceLocalUser,
}

/// The `kind` of an AEC-001 `ref` this shell issues: `artifact` only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReferenceKindTag {
    Artifact,
}

/// AEC-001's `ref` shape for an artifact (HAP-001 § Definitions,
/// "Portable reference"): `{ kind: "artifact", id: <publication identity>,
/// locator: <artifact Catalog identity> }`. Never a device path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableReferenceDto {
    pub kind: ReferenceKindTag,
    pub id: String,
    pub locator: String,
}

impl PortableReferenceDto {
    #[must_use]
    pub fn from_reference(reference: &omnifrons_domain::publication::PortableReference) -> Self {
        Self {
            kind: ReferenceKindTag::Artifact,
            id: reference.id().to_hex(),
            locator: reference.locator().to_string(),
        }
    }
}

/// `artifact_approve`'s payload (spike slice 5b): the identity-bound facts
/// the approval surface showed (HAP-001-R22), the destination asset root
/// identity, the act-as identity, and the derived ids -- the approval id
/// as 16 lowercase hex characters (a `u64` does not survive a JSON
/// number's 53-bit mantissa), the publication identity as 64. Never a
/// device path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactApprovalDto {
    pub approval_id: String,
    pub publication_id: String,
    /// The run whose run-end inventory listed the candidate; `null` for an
    /// approval made from the whole-outbox inventory (`artifact_approve`
    /// with `runId: null`, spike slice 5c), which lists without a run --
    /// the run subdirectory such an entry sits under, if any, is a location
    /// fact its `name` carries, never provenance (HAP-001-R11).
    pub run_id: Option<String>,
    pub name: String,
    pub display_name: String,
    pub sha256_short: String,
    pub size: u64,
    pub detected_type: String,
    pub class: String,
    pub attribution: AttributionDto,
    pub asset_root_id: String,
    pub act_as: ActAsTag,
    pub approved_at: u64,
    /// Whether the entry's handle is held for the publication that follows
    /// (HAP-001-R17): `false` when HAP-001 D22's cap left no room for it,
    /// in which case the publication re-opens the entry under the
    /// single-handle discipline when its turn comes -- said here rather
    /// than left silent (spike slice 5c).
    pub handle_held: bool,
}

impl ArtifactApprovalDto {
    #[must_use]
    pub fn from_approval(
        approval: &omnifrons_domain::publication::ArtifactApproval,
        handle_held: bool,
    ) -> Self {
        use omnifrons_domain::outbox::Attribution;
        Self {
            approval_id: approval.approval_id.to_hex(),
            publication_id: approval.publication_id.to_hex(),
            run_id: approval
                .run_id
                .as_ref()
                .map(|run_id| run_id.as_str().to_string()),
            name: approval.name.clone(),
            display_name: approval.display_name.as_str().to_string(),
            sha256_short: approval.digest.short_hex(),
            size: approval.size,
            detected_type: approval.detected_type.as_str().to_string(),
            class: approval.class.as_str().to_string(),
            attribution: match &approval.attribution {
                Attribution::Run(run_id) => AttributionDto::Run {
                    run_id: run_id.as_str().to_string(),
                },
                Attribution::Unattributed => AttributionDto::Unattributed,
            },
            asset_root_id: approval.asset_root_id.as_str().to_string(),
            act_as: ActAsTag::DeviceLocalUser,
            approved_at: system_time_to_millis(approval.approved_at),
            handle_held,
        }
    }
}

/// One publication, as `artifact_publish` returns it and
/// `publications_list` lists it (spike slice 5b): the publication
/// identity, its state, the portable reference once `registered`
/// (HAP-001-R24; `null` before), the record's `provider_state` and Catalog
/// identity once a record exists, the sanitized display names, and this
/// device's own availability observation. Never a device path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicationDto {
    pub publication_id: String,
    pub state: ArtifactStateTag,
    pub reference: Option<PortableReferenceDto>,
    pub provider_state: Option<ProviderStateTag>,
    pub catalog_id: Option<String>,
    pub names: Vec<String>,
    pub availability: AvailabilityTag,
}

impl PublicationDto {
    /// A registered record, as listed.
    #[must_use]
    pub fn from_record(
        record: &omnifrons_domain::publication::CatalogRecord,
        availability: AvailabilityTag,
    ) -> Self {
        Self {
            publication_id: record.publication_id.to_hex(),
            state: record.state.into(),
            reference: record
                .reference()
                .as_ref()
                .map(PortableReferenceDto::from_reference),
            provider_state: Some(record.provider.state.into()),
            catalog_id: Some(record.catalog_id.to_string()),
            names: record
                .names
                .iter()
                .map(|name| name.as_str().to_string())
                .collect(),
            availability,
        }
    }

    /// A publication as the transaction left it: `registered` with its
    /// record's facts, or `registration-pending` with no reference, no
    /// provider state, and no Catalog identity yet.
    #[must_use]
    pub fn from_published(
        published: &omnifrons_app::publication::Published,
        availability: AvailabilityTag,
    ) -> Self {
        use omnifrons_domain::publication::ArtifactState;
        let registered = published.state == ArtifactState::Registered;
        Self {
            publication_id: published.record.publication_id.to_hex(),
            state: published.state.into(),
            reference: published
                .reference
                .as_ref()
                .map(PortableReferenceDto::from_reference),
            provider_state: registered.then(|| published.record.provider.state.into()),
            catalog_id: registered.then(|| published.record.catalog_id.to_string()),
            names: published
                .record
                .names
                .iter()
                .map(|name| name.as_str().to_string())
                .collect(),
            availability,
        }
    }
}

/// The proposed AEC-001 kind `artifact.state`, as `artifact_publish`'s
/// `onState` channel carries it on every transition (HAP-001-R35):
/// adjacently tagged like [`AdapterEventDto`], `{ kind: "artifact-state",
/// payload: { publicationId, state, providerState } }`. Never a path; the
/// event observes and controls nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ArtifactStateFrame {
    ArtifactState {
        publication_id: String,
        state: ArtifactStateTag,
        provider_state: Option<ProviderStateTag>,
    },
}

impl ArtifactStateFrame {
    #[must_use]
    pub fn from_event(event: &omnifrons_app::publication::StateEvent) -> Self {
        Self::ArtifactState {
            publication_id: event.publication_id.to_hex(),
            state: event.state.into(),
            provider_state: event.provider_state.map(Into::into),
        }
    }
}

/// [`omnifrons_domain::guidance::ManagedFileKind`], as it crosses IPC
/// both ways (spike slice 5c): the `kind` of every guidance command,
/// `"guidance"` or `"ignore"` and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedFileKindDto {
    Guidance,
    Ignore,
}

impl From<omnifrons_domain::guidance::ManagedFileKind> for ManagedFileKindDto {
    fn from(kind: omnifrons_domain::guidance::ManagedFileKind) -> Self {
        use omnifrons_domain::guidance::ManagedFileKind;
        match kind {
            ManagedFileKind::Guidance => Self::Guidance,
            ManagedFileKind::Ignore => Self::Ignore,
        }
    }
}

impl From<ManagedFileKindDto> for omnifrons_domain::guidance::ManagedFileKind {
    fn from(kind: ManagedFileKindDto) -> Self {
        match kind {
            ManagedFileKindDto::Guidance => Self::Guidance,
            ManagedFileKindDto::Ignore => Self::Ignore,
        }
    }
}

/// [`omnifrons_domain::guidance::ManagedStatus`], as it crosses IPC: the
/// token only (an outdated block's own version stays inside the shell).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedStatusTag {
    Absent,
    Current,
    Outdated,
    Modified,
    Malformed,
}

impl From<&omnifrons_domain::guidance::ManagedStatus> for ManagedStatusTag {
    fn from(status: &omnifrons_domain::guidance::ManagedStatus) -> Self {
        use omnifrons_domain::guidance::ManagedStatus;
        match status {
            ManagedStatus::Absent => Self::Absent,
            ManagedStatus::Current => Self::Current,
            ManagedStatus::Outdated { .. } => Self::Outdated,
            ManagedStatus::Modified => Self::Modified,
            ManagedStatus::Malformed => Self::Malformed,
        }
    }
}

/// What a guidance command did or would do (spike slice 5c): the apply
/// plan's `insert`, `replace`, and `no-op`, plus the `remove` and
/// `restore` outcomes of the commands of those names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GuidanceActionTag {
    Insert,
    Replace,
    NoOp,
    Remove,
    Restore,
}

impl From<omnifrons_domain::guidance::ApplyAction> for GuidanceActionTag {
    fn from(action: omnifrons_domain::guidance::ApplyAction) -> Self {
        use omnifrons_domain::guidance::ApplyAction;
        match action {
            ApplyAction::Insert => Self::Insert,
            ApplyAction::Replace => Self::Replace,
            ApplyAction::NoOp => Self::NoOp,
        }
    }
}

/// `guidance_status`'s payload (spike slice 5c): the managed file's name
/// at the workspace root, whether it exists, the block's state, the
/// template version the product would write, the file's full digest
/// (what an apply or a removal binds to) beside its short form, and the
/// snapshot counts. Never a device path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidanceStatusDto {
    pub kind: ManagedFileKindDto,
    pub file: String,
    pub exists: bool,
    pub managed: ManagedStatusTag,
    pub template_version: String,
    pub file_sha256: Option<String>,
    pub file_sha256_short: Option<String>,
    pub snapshots: u32,
    pub pinned: u32,
}

impl GuidanceStatusDto {
    #[must_use]
    pub fn from_status(
        kind: omnifrons_domain::guidance::ManagedFileKind,
        status: &omnifrons_app::guidance::GuidanceStatus,
    ) -> Self {
        Self {
            kind: kind.into(),
            file: status.file.clone(),
            exists: status.exists,
            managed: ManagedStatusTag::from(&status.managed),
            template_version: omnifrons_domain::guidance::GUIDANCE_TEMPLATE_VERSION.to_string(),
            file_sha256: status.file_sha256.map(|digest| digest.to_hex()),
            file_sha256_short: status.file_sha256.map(|digest| digest.short_hex()),
            snapshots: u32::try_from(status.snapshots).unwrap_or(u32::MAX),
            pinned: u32::try_from(status.pinned).unwrap_or(u32::MAX),
        }
    }
}

/// `guidance_preview`'s payload (spike slice 5c): the proposal the user
/// disposes of (HAP-001 D18) -- the block as it would be written, the
/// action, the file digest an apply must bind to, and the short digest of
/// the whole file afterwards. Never a device path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidancePreviewDto {
    pub kind: ManagedFileKindDto,
    pub file: String,
    pub action: GuidanceActionTag,
    pub proposed: String,
    pub file_sha256: Option<String>,
    pub file_sha256_short: Option<String>,
    pub result_sha256_short: String,
}

impl GuidancePreviewDto {
    #[must_use]
    pub fn from_preview(
        kind: omnifrons_domain::guidance::ManagedFileKind,
        preview: &omnifrons_app::guidance::GuidancePreview,
    ) -> Self {
        Self {
            kind: kind.into(),
            file: preview.file.clone(),
            action: preview.action.into(),
            proposed: preview.proposed_block.clone(),
            file_sha256: preview.file_sha256.map(|digest| digest.to_hex()),
            file_sha256_short: preview.file_sha256.map(|digest| digest.short_hex()),
            result_sha256_short: preview.result_sha256.short_hex(),
        }
    }
}

/// The payload of `guidance_apply`, `guidance_remove`, and
/// `guidance_restore` (spike slice 5c): what was done, the snapshot taken
/// before the write (`null` for a no-op), and the short digest of the
/// whole file afterwards (`null` when the file was removed). Never a
/// device path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidanceAppliedDto {
    pub kind: ManagedFileKindDto,
    pub file: String,
    pub action: GuidanceActionTag,
    pub snapshot_id: Option<String>,
    pub result_sha256_short: Option<String>,
}

/// One snapshot, as `guidance_snapshots` lists it (spike slice 5c): its
/// 16-hex id, the kind and file it recorded, whether the file existed,
/// the short digest and size of the bytes recorded, the instant as ms
/// since the epoch, and whether it is pinned. Never a device path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDto {
    pub id: String,
    pub kind: ManagedFileKindDto,
    pub file: String,
    pub existed: bool,
    pub sha256_short: String,
    pub size: u64,
    pub taken_at: u64,
    pub pinned: bool,
}

impl SnapshotDto {
    #[must_use]
    pub fn from_manifest(manifest: &omnifrons_app::snapshot_store::SnapshotManifest) -> Self {
        Self {
            id: manifest.id.to_hex(),
            kind: manifest.kind.into(),
            file: manifest.file.clone(),
            existed: manifest.existed,
            sha256_short: manifest.sha256.short_hex(),
            size: manifest.size,
            taken_at: system_time_to_millis(manifest.taken_at),
            pinned: manifest.pinned,
        }
    }
}

/// [`omnifrons_domain::wrong_root::OutputDiscipline`], as it crosses IPC
/// (spike slice 5d, HAP-001-R33).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputDisciplineTag {
    Enforced,
    Advisory,
}

impl From<omnifrons_domain::wrong_root::OutputDiscipline> for OutputDisciplineTag {
    fn from(discipline: omnifrons_domain::wrong_root::OutputDiscipline) -> Self {
        use omnifrons_domain::wrong_root::OutputDiscipline;
        match discipline {
            OutputDiscipline::Enforced => Self::Enforced,
            OutputDiscipline::Advisory => Self::Advisory,
        }
    }
}

/// `wrongroot_status`'s payload (spike slice 5d): the output-discipline
/// label the active scope reports, the scope mode it was derived from,
/// every disclosure HAP-001-R33 and R34 fix -- carried as text so the
/// renderer states them verbatim rather than composing its own -- and
/// whether a scan has run in this session, with how many findings it left
/// standing. Never a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WrongRootStatusDto {
    pub output_discipline: OutputDisciplineTag,
    pub scope_mode: ScopeModeDto,
    pub disclosures: Vec<String>,
    pub scanned: bool,
    pub findings: u32,
}

impl WrongRootStatusDto {
    /// The status for `discipline` under `mode`, with the disclosures the
    /// domain fixes.
    #[must_use]
    pub fn of(
        discipline: omnifrons_domain::wrong_root::OutputDiscipline,
        mode: omnifrons_domain::scope::ScopeMode,
        scanned: bool,
        findings: u32,
    ) -> Self {
        Self {
            output_discipline: discipline.into(),
            scope_mode: mode.into(),
            disclosures: discipline
                .disclosures()
                .into_iter()
                .map(str::to_string)
                .collect(),
            scanned,
            findings,
        }
    }
}

/// One `misplaced` file, as `misplaced_list` returns it (spike slice 5d):
/// the project-relative name it was found at -- producer-supplied text a
/// consumer renders as plain text only -- the facts taken from its own
/// handle, why it is misplaced, and the three remedies HAP-001-R32 offers.
/// Never a device path (RCS-001-R14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MisplacedDto {
    pub name: String,
    pub size: u64,
    /// The full 64-hex content digest: the identity fact
    /// `misplaced_remedy` names the finding by, exactly as
    /// `artifact_approve` names a candidate.
    pub sha256: String,
    pub sha256_short: String,
    pub detected_type: String,
    pub class: String,
    pub reason: String,
    pub remedies: Vec<String>,
}

impl MisplacedDto {
    /// The row for `finding`.
    #[must_use]
    pub fn from_finding(finding: &omnifrons_domain::wrong_root::MisplacedFinding) -> Self {
        Self {
            name: finding.name().to_string(),
            size: finding.size,
            sha256: finding.digest.to_hex(),
            sha256_short: finding.digest.short_hex(),
            detected_type: finding.detected_type.as_str().to_string(),
            class: finding.class.as_str().to_string(),
            reason: finding.reason.as_str().to_string(),
            remedies: omnifrons_domain::wrong_root::Remedy::ALL
                .iter()
                .map(|remedy| remedy.as_str().to_string())
                .collect(),
        }
    }
}

/// `wrongroot_scan`'s payload (spike slice 5d): what the walk saw. Counts
/// only; the findings themselves come from `misplaced_list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MisplacedScanDto {
    pub scanned: u32,
    pub findings: u32,
    pub ignored: u32,
    pub excluded: u32,
    pub unreadable: u32,
    pub truncated: bool,
}

/// [`omnifrons_domain::wrong_root::Remedy`], as it crosses IPC: the
/// closed set of three, and nothing else (HAP-001-R32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemedyTag {
    Quarantine,
    Publish,
    Ignore,
}

impl From<RemedyTag> for omnifrons_domain::wrong_root::Remedy {
    fn from(tag: RemedyTag) -> Self {
        match tag {
            RemedyTag::Quarantine => Self::Quarantine,
            RemedyTag::Publish => Self::Publish,
            RemedyTag::Ignore => Self::Ignore,
        }
    }
}

/// What a remedy actually did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemedyOutcomeTag {
    /// The file was moved into the quarantine directory.
    Quarantined,
    /// The file's bytes were copied into the outbox as a new unattributed
    /// entry; nothing is published until that entry is approved.
    CopiedToOutbox,
    /// The decision was recorded against the file's name and digest.
    Ignored,
}

/// `misplaced_remedy`'s payload (spike slice 5d): which remedy ran, what
/// it did, and -- for the two that produce something -- the logical name
/// of what it produced and that thing's digest. `name` is a quarantined
/// file's sanitized name or the new outbox entry's name, one component
/// either way; never a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MisplacedRemedyDto {
    pub remedy: RemedyTag,
    pub outcome: RemedyOutcomeTag,
    pub name: Option<String>,
    pub sha256: Option<String>,
    /// Quarantine only: whether the original was kept because this
    /// platform could not prove the name still held it or could not
    /// remove it (HAP-001-R19's residual, stated rather than silent).
    pub original_kept: Option<bool>,
    /// A fixed token qualifying the outcome (why the original was kept,
    /// or which move ran); never free text and never a path.
    pub detail: Option<String>,
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
    /// `harness_spawn`'s `kind: "adapter"` named the `pty-cli` adapter on
    /// a platform where this slice implements no pseudo-terminal launch
    /// (Windows; spike slice 4). Nothing was spawned.
    PtyUnsupported,
    /// A `pty-cli` launch's prompt contains control characters the
    /// terminal's line discipline would interpret rather than type (a C0
    /// control other than newline and tab, or DEL); refused at
    /// plan-building time, nothing was spawned (spike slice 4).
    PromptNotTypeable,
    /// The project's outbox declaration is invalid -- the classification
    /// policy could not be loaded, or the declared path resolves outside
    /// the project or is a link -- so ingestion is blocked
    /// (`candidates_list`; HAP-001-R8, spike slice 5).
    OutboxInvalid,
    /// An adapter launch could not prepare its run subdirectory: the
    /// outbox failed its pre-creation check, something already sits at the
    /// subdirectory's path, or handle verification failed; nothing was
    /// spawned (HAP-001-R10, D14; spike slice 5).
    OutboxUnavailable,
    /// The published copy's digest did not verify, or the held handle's
    /// bytes changed since they were digested; the copy is discarded and
    /// the entry preserved (HAP-001-R21; spike slice 5b).
    IntegrityMismatch,
    /// The publication identity is already registered; the existing record
    /// is acknowledged in [`ShellErrorDetail::DuplicatePublication`] and
    /// nothing new was published (HAP-001-R23; spike slice 5b).
    DuplicatePublication,
    /// The product work area resolves inside a registered workspace root,
    /// or its journal cannot be used; the operation is refused
    /// (HAP-001-R7; spike slice 5b).
    WorkAreaInvalid,
    /// The project declares no asset root, or its device asset path
    /// resolves inside a registered workspace root or cannot be written;
    /// nothing is copied (HAP-001-R6, R14; spike slice 5b).
    DestinationInvalid,
    /// At publish time the entry's path no longer names the held handle's
    /// file, or the handle is not a regular file; nothing is published and
    /// the held bytes are kept as a recovery entry (HAP-001-R18; spike
    /// slice 5b).
    OutboxEscape,
    /// The held handle's link count is greater than one at publish time
    /// (HAP-001-R20; spike slice 5b).
    OutboxLinked,
    /// The candidate cannot be approved in its state or class, its digest
    /// does not match the request, or the re-opened entry's identity facts
    /// changed since approval (HAP-001-R22; spike slice 5b).
    Refused,
    /// The project's Catalog could not be read or written (spike slice
    /// 5b).
    CatalogUnavailable,
    /// `artifact_approve` or `artifact_publish` was called while a
    /// supervised process is running: the publication surface is frozen
    /// until every run has reached its terminal state -- a spike default
    /// mirroring the renderer's own guard so direct IPC cannot bypass it
    /// (spike slice 5b, renderer risk review R1-001).
    RunActive,
    /// A guidance command named a file outside RCS-001's file-name rule
    /// (a path, a reserved device name, not Markdown), or the managed file
    /// is not a regular file, exceeds the size bound, is not valid UTF-8,
    /// could not be read or written, or did not read back as written
    /// (spike slice 5c, HAP-001 D18).
    GuidanceFileInvalid,
    /// The managed file's digest (or its absence) differs from the
    /// `fileSha256` the request bound to: the surface is stale, nothing
    /// was written (spike slice 5c).
    GuidanceFileChanged,
    /// The managed block was edited inside its sentinels; Omnifrons
    /// neither replaces nor removes it -- the user resolves by hand or
    /// restores a snapshot (spike slice 5c).
    GuidanceBlockModified,
    /// The managed block's sentinels are not one intact pair (spike slice
    /// 5c).
    GuidanceBlockMalformed,
    /// `guidance_remove` found no managed block, or `guidance_restore` or
    /// `guidance_pin` named a snapshot this project does not hold (spike
    /// slice 5c).
    GuidanceUnmanaged,
    /// The snapshot store under the work area could not be read, is
    /// corrupt, or could not be written (spike slice 5c).
    SnapshotUnavailable,
    /// `misplaced_remedy` named a finding this shell's last scan did not
    /// report, or reported at a different digest: the surface is stale and
    /// nothing was moved, copied, or recorded (spike slice 5d,
    /// HAP-001-R32).
    MisplacedUnknown,
    /// The quarantine directory resolves inside a registered workspace
    /// root, could not be created owner-only, or could not be written; the
    /// file stays exactly where it was found (spike slice 5d,
    /// RCS-001-R10).
    QuarantineUnavailable,
    /// The wrong-root scan could not walk the active workspace root, so no
    /// verdict is claimed for it (spike slice 5d).
    ScanFailed,
}

/// Structured detail for a [`ShellError`], carrying values a fixed
/// catalogue `message` string must not (a full digest is not a secret,
/// but it does not belong interpolated into free text either). Untagged:
/// each carrying code has its own field set, and the wire shape of the
/// slice-2 `changed-since-approval` detail is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged, rename_all_fields = "camelCase")]
pub enum ShellErrorDetail {
    /// [`ShellErrorCode::ChangedSinceApproval`]: both digests as short hex
    /// prefixes.
    ChangedSinceApproval {
        recorded_sha256_short: String,
        observed_sha256_short: String,
    },
    /// [`ShellErrorCode::DuplicatePublication`]: the existing record's
    /// logical identities (HAP-001-R23 "acknowledge the existing record").
    DuplicatePublication {
        publication_id: String,
        catalog_id: String,
    },
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

    use omnifrons_domain::executable::Sha256Digest;

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
            ShellErrorDetail::ChangedSinceApproval {
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
            work_area: super::WorkAreaStateTag::Valid,
        };
        assert_eq!(
            json(&dto),
            serde_json::json!({"displayPath": "/home/user/project", "workArea": "valid"})
        );
        let invalid = WorkspaceDto {
            display_path: "/home/user/project".to_string(),
            work_area: super::WorkAreaStateTag::WorkAreaInvalid,
        };
        assert_eq!(
            json(&invalid)["workArea"],
            serde_json::json!("work-area-invalid")
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

    // -- Slice 4: the pty-cli descriptor, the three terminal event kinds,
    // and the pty-unsupported error code --

    #[test]
    fn prompt_channel_dto_pty_typed_and_transport_pty_serialize_as_kebab_case() {
        let dto = AdapterDescriptorDto {
            id: "pty-cli".to_string(),
            display_name: "PTY CLI".to_string(),
            transport_class: TransportClassDto::Pty,
            prompt_channel: PromptChannelDto::PtyTyped,
            scope_mode: ScopeModeDto::Advisory,
            notes: "degraded fallback".to_string(),
        };
        assert_eq!(
            json(&dto),
            serde_json::json!({
                "id": "pty-cli",
                "displayName": "PTY CLI",
                "transportClass": "pty",
                "promptChannel": "pty-typed",
                "scopeMode": "advisory",
                "notes": "degraded fallback",
            })
        );
    }

    /// Proposed AEC-001 kind `terminal-text`: the normalized text, newlines
    /// included.
    #[test]
    fn harness_frame_event_terminal_text_json_shape() {
        let frame = HarnessFrame::event(
            ProcessIdDto(42),
            8,
            0,
            omnifrons_domain::adapter::AdapterEvent::TerminalText {
                text: "hello\n".to_string(),
            },
        );
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "event",
                "body": {
                    "id": 42, "seq": 8, "droppedBefore": 0,
                    "kind": "terminal-text", "payload": {"text": "hello\n"}
                }
            })
        );
    }

    /// Proposed AEC-001 kind `terminal-action`: `action` is one of the two
    /// closed tokens, `text` the already-sanitized text.
    #[test]
    fn harness_frame_event_terminal_action_json_shapes() {
        use omnifrons_domain::terminal::TerminalAction;
        let title = HarnessFrame::event(
            ProcessIdDto(42),
            9,
            0,
            omnifrons_domain::adapter::AdapterEvent::TerminalAction(TerminalAction::Title(
                "build ok".to_string(),
            )),
        );
        assert_eq!(
            json(&title)["body"],
            serde_json::json!({
                "id": 42, "seq": 9, "droppedBefore": 0,
                "kind": "terminal-action", "payload": {"action": "title", "text": "build ok"}
            })
        );
        let notification = HarnessFrame::event(
            ProcessIdDto(42),
            10,
            2,
            omnifrons_domain::adapter::AdapterEvent::TerminalAction(TerminalAction::Notification(
                "done".to_string(),
            )),
        );
        assert_eq!(
            json(&notification)["body"],
            serde_json::json!({
                "id": 42, "seq": 10, "droppedBefore": 2,
                "kind": "terminal-action", "payload": {"action": "notification", "text": "done"}
            })
        );
    }

    /// Proposed AEC-001 kind `terminal-drops`: the seven per-family counts,
    /// camelCase on the wire.
    #[test]
    fn harness_frame_event_terminal_drops_json_shape() {
        let frame = HarnessFrame::event(
            ProcessIdDto(42),
            11,
            0,
            omnifrons_domain::adapter::AdapterEvent::TerminalDrops(
                omnifrons_domain::terminal::DropCounts {
                    layout: 21,
                    hyperlink: 2,
                    clipboard: 2,
                    file_transfer: 3,
                    string: 3,
                    unknown: 1,
                    malformed: 3,
                },
            ),
        );
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "event",
                "body": {
                    "id": 42, "seq": 11, "droppedBefore": 0,
                    "kind": "terminal-drops",
                    "payload": {
                        "layout": 21, "hyperlink": 2, "clipboard": 2, "fileTransfer": 3,
                        "string": 3, "unknown": 1, "malformed": 3
                    }
                }
            })
        );
    }

    #[test]
    fn pty_unsupported_code_serializes_as_kebab_case_with_a_slash_free_message() {
        let error = ShellError::new(
            ShellErrorCode::PtyUnsupported,
            "pseudo-terminal launches are not available on this platform",
        );
        assert_eq!(
            json(&error),
            serde_json::json!({
                "code": "pty-unsupported",
                "message": "pseudo-terminal launches are not available on this platform"
            })
        );
        assert!(!error.message.contains('/'));
    }

    #[test]
    fn prompt_not_typeable_code_serializes_as_kebab_case_with_a_slash_free_message() {
        let error = ShellError::new(
            ShellErrorCode::PromptNotTypeable,
            "prompt contains control characters a terminal would interpret",
        );
        assert_eq!(
            json(&error),
            serde_json::json!({
                "code": "prompt-not-typeable",
                "message": "prompt contains control characters a terminal would interpret"
            })
        );
        assert!(!error.message.contains('/'));
    }

    /// The two slice-4 error codes each render as their documented
    /// kebab-case token: the closed set the renderer's `ShellErrorCode`
    /// union mirrors.
    #[test]
    fn slice_4_error_codes_serialize_as_kebab_case() {
        let cases = [
            (ShellErrorCode::PtyUnsupported, "pty-unsupported"),
            (ShellErrorCode::PromptNotTypeable, "prompt-not-typeable"),
        ];
        for (code, token) in cases {
            let error = ShellError::new(code, "message");
            assert_eq!(json(&error)["code"], serde_json::json!(token));
        }
    }

    // -- Slice 5: the outbox status, candidates, the artifact-publish and
    // candidates event kinds, and the two outbox error codes --

    /// `outbox_status` for a valid, existing outbox: the canonical outbox
    /// path is identity evidence for display (the third RCS-001-R14
    /// exception), never a reference the renderer sends back.
    #[test]
    fn outbox_status_dto_json_shape_when_valid() {
        let dto = super::OutboxStatusDto {
            declared: Some(".omnifrons/outbox".to_string()),
            outbox: Some("/home/user/project/.omnifrons/outbox".to_string()),
            exists: true,
            state: super::OutboxStateTag::Valid,
            reason: None,
            policy_path: ".omnifrons/asset-policy.json".to_string(),
            asset_root_id: Some("main".to_string()),
        };
        assert_eq!(
            json(&dto),
            serde_json::json!({
                "declared": ".omnifrons/outbox",
                "outbox": "/home/user/project/.omnifrons/outbox",
                "exists": true,
                "state": "valid",
                "reason": null,
                "policyPath": ".omnifrons/asset-policy.json",
                "assetRootId": "main",
            })
        );
    }

    /// An invalid declaration carries the declared project-relative name
    /// (shown as invalid) and a fixed reason token, never a device path.
    #[test]
    fn outbox_status_dto_json_shape_when_invalid_carries_no_device_path() {
        let dto = super::OutboxStatusDto {
            declared: Some("elsewhere/outbox".to_string()),
            outbox: None,
            exists: true,
            state: super::OutboxStateTag::OutboxInvalid,
            reason: Some(super::OutboxReasonTag::Link),
            policy_path: ".omnifrons/asset-policy.json".to_string(),
            asset_root_id: None,
        };
        assert_eq!(
            json(&dto),
            serde_json::json!({
                "declared": "elsewhere/outbox",
                "outbox": null,
                "exists": true,
                "state": "outbox-invalid",
                "reason": "link",
                "policyPath": ".omnifrons/asset-policy.json",
                "assetRootId": null,
            })
        );
        let unavailable = super::OutboxStatusDto {
            declared: None,
            outbox: None,
            exists: false,
            state: super::OutboxStateTag::OutboxUnavailable,
            reason: Some(super::OutboxReasonTag::PolicyCorrupt),
            policy_path: ".omnifrons/asset-policy.json".to_string(),
            asset_root_id: None,
        };
        assert_eq!(
            json(&unavailable)["state"],
            serde_json::json!("outbox-unavailable")
        );
        assert_eq!(
            json(&unavailable)["reason"],
            serde_json::json!("policy-corrupt")
        );
    }

    /// Every reason token is kebab-case and slash-free.
    #[test]
    fn outbox_reason_tags_serialize_as_kebab_case() {
        let cases = [
            (super::OutboxReasonTag::OutsideProject, "outside-project"),
            (super::OutboxReasonTag::Link, "link"),
            (super::OutboxReasonTag::NotADirectory, "not-a-directory"),
            (super::OutboxReasonTag::Unreadable, "unreadable"),
            (
                super::OutboxReasonTag::PolicyUnreadable,
                "policy-unreadable",
            ),
            (super::OutboxReasonTag::PolicyCorrupt, "policy-corrupt"),
            (super::OutboxReasonTag::PolicyInvalid, "policy-invalid"),
        ];
        for (tag, token) in cases {
            assert_eq!(json(&tag), serde_json::json!(token));
        }
    }

    /// A `candidates_list` item for an attributed, validated candidate:
    /// the name relative to the outbox, the facts from its handle, the run
    /// that named it, and the `candidate` state -- never a device path.
    #[test]
    fn candidate_dto_json_shape_for_an_attributed_candidate() {
        use omnifrons_domain::outbox::{
            ArtifactClass, Attribution, CandidateEntry, CandidateState, DetectedType, RunId,
        };
        let entry = CandidateEntry {
            name: "run-1/report.pdf".to_string(),
            size: 4096,
            digest: Sha256Digest([0xab; 32]),
            detected_type: DetectedType::Pdf,
            attribution: Attribution::Run(RunId::new("run-1").expect("valid")),
            class: ArtifactClass::GeneratedHeavy,
        };
        let dto = super::CandidateDto::from_candidate(&entry, CandidateState::Candidate);
        assert_eq!(
            json(&dto),
            serde_json::json!({
                "name": "run-1/report.pdf",
                "size": 4096,
                "sha256": "ab".repeat(32),
                "sha256Short": "abababab",
                "detectedType": "pdf",
                "class": "generated-heavy",
                "attribution": {"kind": "run", "runId": "run-1"},
                "state": "candidate",
            })
        );
        // The full digest is identity evidence the approval names the
        // candidate by (HAP-001-R22), never a device path; the row's keys are
        // exactly these seven plus `sha256`, and none is a path.
        let value = json(&dto);
        let keys: std::collections::BTreeSet<&str> = value
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys.into_iter().collect::<Vec<_>>(),
            vec![
                "attribution",
                "class",
                "detectedType",
                "name",
                "sha256",
                "sha256Short",
                "size",
                "state"
            ]
        );
    }

    /// A refused entry (`outbox-escape`, `outbox-linked`) carries no digest
    /// facts at all: nothing was digested (HAP-001-R20).
    #[test]
    fn candidate_dto_json_shape_for_a_refused_entry_has_no_digest_facts() {
        use omnifrons_domain::outbox::{
            ArtifactClass, Attribution, CandidateEntry, CandidateState, DetectedType,
        };
        let entry = CandidateEntry {
            name: "run-1/linked.bin".to_string(),
            size: 0,
            digest: Sha256Digest([0; 32]),
            detected_type: DetectedType::Unknown,
            attribution: Attribution::Unattributed,
            class: ArtifactClass::Unclassified,
        };
        let dto = super::CandidateDto::from_candidate(&entry, CandidateState::OutboxLinked);
        assert_eq!(
            json(&dto),
            serde_json::json!({
                "name": "run-1/linked.bin",
                "size": null,
                "sha256": null,
                "sha256Short": null,
                "detectedType": null,
                "class": null,
                "attribution": {"kind": "unattributed"},
                "state": "outbox-linked",
            })
        );
        let escape = super::CandidateDto::from_candidate(&entry, CandidateState::OutboxEscape);
        assert_eq!(json(&escape)["state"], serde_json::json!("outbox-escape"));
    }

    /// Proposed AEC-001 kind `artifact-publish`: the harness's own claim,
    /// entries named by full digest -- a proposal only.
    #[test]
    fn harness_frame_event_artifact_publish_json_shape() {
        use omnifrons_domain::outbox::{ProposedEntry, PublishProposal};
        let frame = HarnessFrame::event(
            ProcessIdDto(42),
            6,
            0,
            omnifrons_domain::adapter::AdapterEvent::ArtifactPublish(PublishProposal {
                entries: vec![ProposedEntry {
                    name: "report.pdf".to_string(),
                    sha256: Sha256Digest([0xab; 32]),
                }],
            }),
        );
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "event",
                "body": {
                    "id": 42, "seq": 6, "droppedBefore": 0,
                    "kind": "artifact-publish",
                    "payload": {"entries": [{"name": "report.pdf", "sha256": "ab".repeat(32)}]}
                }
            })
        );
    }

    /// The `candidates` event the shell emits once at run end, before the
    /// terminal `state` frame: the inventory summary, counts per state and
    /// attribution, no path.
    #[test]
    fn harness_frame_event_candidates_json_shape() {
        let frame = HarnessFrame::event_dto(
            ProcessIdDto(42),
            9,
            0,
            super::AdapterEventDto::Candidates {
                run_id: "run-1".to_string(),
                total: 5,
                candidate: 3,
                outbox_escape: 1,
                outbox_linked: 1,
                attributed: 2,
                unattributed: 3,
                unreadable: 0,
                unmatched_proposals: 1,
            },
        );
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "event",
                "body": {
                    "id": 42, "seq": 9, "droppedBefore": 0,
                    "kind": "candidates",
                    "payload": {
                        "runId": "run-1", "total": 5, "candidate": 3, "outboxEscape": 1,
                        "outboxLinked": 1, "attributed": 2, "unattributed": 3,
                        "unreadable": 0, "unmatchedProposals": 1
                    }
                }
            })
        );
    }

    /// The two slice-5 error codes each render as their documented
    /// kebab-case token with slash-free messages.
    #[test]
    fn slice_5_error_codes_serialize_as_kebab_case() {
        let cases = [
            (ShellErrorCode::OutboxInvalid, "outbox-invalid"),
            (ShellErrorCode::OutboxUnavailable, "outbox-unavailable"),
        ];
        for (code, token) in cases {
            let error = ShellError::new(code, "message");
            assert_eq!(json(&error)["code"], serde_json::json!(token));
        }
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

    // -- Slice 5b: the artifact approval, the publication, the
    // artifact-state frame, and the publication error codes --

    /// `artifact_approve`'s payload: every identity-bound fact the approval
    /// surface showed (HAP-001-R22), the act-as identity, and the derived
    /// ids as hex strings -- never a device path.
    #[test]
    fn artifact_approval_dto_json_shape() {
        let dto = super::ArtifactApprovalDto {
            approval_id: "0123456789abcdef".to_string(),
            publication_id: "ab".repeat(32),
            run_id: Some("run-1".to_string()),
            name: "run-1/report.pdf".to_string(),
            display_name: "report.pdf".to_string(),
            sha256_short: "abababab".to_string(),
            size: 4096,
            detected_type: "pdf".to_string(),
            class: "generated-heavy".to_string(),
            attribution: super::AttributionDto::Run {
                run_id: "run-1".to_string(),
            },
            asset_root_id: "main".to_string(),
            act_as: super::ActAsTag::DeviceLocalUser,
            approved_at: 1_725_782_401_000,
            handle_held: true,
        };
        assert_eq!(
            json(&dto),
            serde_json::json!({
                "approvalId": "0123456789abcdef",
                "publicationId": "ab".repeat(32),
                "runId": "run-1",
                "name": "run-1/report.pdf",
                "displayName": "report.pdf",
                "sha256Short": "abababab",
                "size": 4096,
                "detectedType": "pdf",
                "class": "generated-heavy",
                "attribution": {"kind": "run", "runId": "run-1"},
                "assetRootId": "main",
                "actAs": "device-local-user",
                "approvedAt": 1_725_782_401_000u64,
                "handleHeld": true,
            })
        );
    }

    /// An approval made from the whole-outbox inventory (spike slice 5c)
    /// names no run: `runId` is `null`, the attribution is the
    /// unattributed fact, and `handleHeld` says whether the re-opened
    /// handle is held for the publication (false once the D22 cap is
    /// full). The slice-5b shape is unchanged when a run is present.
    #[test]
    fn artifact_approval_dto_json_shape_for_a_whole_outbox_approval_has_a_null_run() {
        let dto = super::ArtifactApprovalDto {
            approval_id: "0123456789abcdef".to_string(),
            publication_id: "ab".repeat(32),
            run_id: None,
            name: "dropped.pdf".to_string(),
            display_name: "dropped.pdf".to_string(),
            sha256_short: "abababab".to_string(),
            size: 15,
            detected_type: "pdf".to_string(),
            class: "generated-heavy".to_string(),
            attribution: super::AttributionDto::Unattributed,
            asset_root_id: "main".to_string(),
            act_as: super::ActAsTag::DeviceLocalUser,
            approved_at: 1_725_782_401_000,
            handle_held: false,
        };
        let value = json(&dto);
        assert_eq!(value["runId"], serde_json::Value::Null);
        assert_eq!(
            value["attribution"],
            serde_json::json!({"kind": "unattributed"})
        );
        assert_eq!(value["handleHeld"], serde_json::json!(false));
        assert_eq!(value["name"], serde_json::json!("dropped.pdf"));
        let keys: Vec<&str> = value
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys.len(),
            14,
            "the thirteen slice-5b keys plus handleHeld, none a path: {keys:?}"
        );
    }

    /// `artifact_publish`'s and `publications_list`'s payload: a registered
    /// publication carries AEC-001's `ref` shape `{ kind, id, locator }`
    /// with the Catalog identity as `locator`; a pending one carries no
    /// reference (HAP-001-R24). Never a device path.
    #[test]
    fn publication_dto_json_shapes_registered_and_pending() {
        let registered = super::PublicationDto {
            publication_id: "ab".repeat(32),
            state: super::ArtifactStateTag::Registered,
            reference: Some(super::PortableReferenceDto {
                kind: super::ReferenceKindTag::Artifact,
                id: "ab".repeat(32),
                locator: format!("main/{}", "ab".repeat(32)),
            }),
            provider_state: Some(super::ProviderStateTag::Pending),
            catalog_id: Some(format!("main/{}", "ab".repeat(32))),
            names: vec!["report.pdf".to_string()],
            availability: super::AvailabilityTag::Local,
        };
        assert_eq!(
            json(&registered),
            serde_json::json!({
                "publicationId": "ab".repeat(32),
                "state": "registered",
                "reference": {"kind": "artifact", "id": "ab".repeat(32), "locator": format!("main/{}", "ab".repeat(32))},
                "providerState": "pending",
                "catalogId": format!("main/{}", "ab".repeat(32)),
                "names": ["report.pdf"],
                "availability": "local",
            })
        );
        let pending = super::PublicationDto {
            publication_id: "cd".repeat(32),
            state: super::ArtifactStateTag::RegistrationPending,
            reference: None,
            provider_state: None,
            catalog_id: None,
            names: vec!["report.pdf".to_string()],
            availability: super::AvailabilityTag::Unknown,
        };
        assert_eq!(
            json(&pending)["state"],
            serde_json::json!("registration-pending")
        );
        assert_eq!(json(&pending)["reference"], serde_json::json!(null));
        assert_eq!(json(&pending)["providerState"], serde_json::json!(null));
        assert_eq!(json(&pending)["catalogId"], serde_json::json!(null));
        assert_eq!(json(&pending)["availability"], serde_json::json!("unknown"));
    }

    /// Every artifact state token the shell can put on the wire is spelled
    /// as HAP-001's signal mapping spells it (HAP-001-R26).
    #[test]
    fn artifact_state_and_provider_state_tags_serialize_as_kebab_case() {
        use omnifrons_domain::publication::{ArtifactState, ProviderState};
        for state in ArtifactState::ALL {
            assert_eq!(
                json(&super::ArtifactStateTag::from(state)),
                serde_json::json!(state.as_str())
            );
        }
        for state in ProviderState::ALL {
            assert_eq!(
                json(&super::ProviderStateTag::from(state)),
                serde_json::json!(state.as_str())
            );
        }
    }

    /// The proposed AEC-001 kind `artifact.state`, as `artifact_publish`'s
    /// `onState` channel carries it: `{ kind: "artifact-state", payload:
    /// { publicationId, state, providerState } }` on every transition.
    #[test]
    fn artifact_state_frame_json_shape() {
        let frame = super::ArtifactStateFrame::ArtifactState {
            publication_id: "ab".repeat(32),
            state: super::ArtifactStateTag::PublishedLocal,
            provider_state: None,
        };
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "kind": "artifact-state",
                "payload": {"publicationId": "ab".repeat(32), "state": "published-local", "providerState": null}
            })
        );
        let registered = super::ArtifactStateFrame::ArtifactState {
            publication_id: "ab".repeat(32),
            state: super::ArtifactStateTag::Registered,
            provider_state: Some(super::ProviderStateTag::Pending),
        };
        assert_eq!(
            json(&registered)["payload"]["providerState"],
            serde_json::json!("pending")
        );
    }

    /// The nine slice-5b error codes each render as their documented
    /// kebab-case token, with slash-free messages.
    #[test]
    fn slice_5b_error_codes_serialize_as_kebab_case() {
        let cases = [
            (ShellErrorCode::IntegrityMismatch, "integrity-mismatch"),
            (
                ShellErrorCode::DuplicatePublication,
                "duplicate-publication",
            ),
            (ShellErrorCode::WorkAreaInvalid, "work-area-invalid"),
            (ShellErrorCode::DestinationInvalid, "destination-invalid"),
            (ShellErrorCode::OutboxEscape, "outbox-escape"),
            (ShellErrorCode::OutboxLinked, "outbox-linked"),
            (ShellErrorCode::Refused, "refused"),
            (ShellErrorCode::CatalogUnavailable, "catalog-unavailable"),
            (ShellErrorCode::RunActive, "run-active"),
        ];
        for (code, token) in cases {
            let error = ShellError::new(code, "message");
            assert_eq!(json(&error)["code"], serde_json::json!(token));
        }
    }

    /// The six slice-5c error codes each render as their documented
    /// kebab-case token.
    #[test]
    fn slice_5c_error_codes_serialize_as_kebab_case() {
        let cases = [
            (ShellErrorCode::GuidanceFileInvalid, "guidance-file-invalid"),
            (ShellErrorCode::GuidanceFileChanged, "guidance-file-changed"),
            (
                ShellErrorCode::GuidanceBlockModified,
                "guidance-block-modified",
            ),
            (
                ShellErrorCode::GuidanceBlockMalformed,
                "guidance-block-malformed",
            ),
            (ShellErrorCode::GuidanceUnmanaged, "guidance-unmanaged"),
            (ShellErrorCode::SnapshotUnavailable, "snapshot-unavailable"),
        ];
        for (code, token) in cases {
            let error = ShellError::new(code, "message");
            assert_eq!(json(&error)["code"], serde_json::json!(token));
        }
    }

    /// `kind` on every guidance command is `"guidance"` or `"ignore"` and
    /// nothing else.
    #[test]
    fn managed_file_kind_dto_deserializes_from_kebab_case_only() {
        let guidance: super::ManagedFileKindDto =
            serde_json::from_str("\"guidance\"").expect("guidance");
        assert_eq!(guidance, super::ManagedFileKindDto::Guidance);
        let ignore: super::ManagedFileKindDto = serde_json::from_str("\"ignore\"").expect("ignore");
        assert_eq!(ignore, super::ManagedFileKindDto::Ignore);
        assert!(serde_json::from_str::<super::ManagedFileKindDto>("\"other\"").is_err());
        assert!(serde_json::from_str::<super::ManagedFileKindDto>("\"Guidance\"").is_err());
        for kind in omnifrons_domain::guidance::ManagedFileKind::ALL {
            assert_eq!(
                json(&super::ManagedFileKindDto::from(kind)),
                serde_json::json!(kind.as_str())
            );
            assert_eq!(
                omnifrons_domain::guidance::ManagedFileKind::from(super::ManagedFileKindDto::from(
                    kind
                )),
                kind
            );
        }
    }

    /// The status and preview payloads (spike slice 5c): camelCase keys,
    /// fixed tokens, the digests short except the full `fileSha256` a
    /// request binds to -- never a path.
    #[test]
    fn guidance_status_and_preview_dto_json_shapes() {
        let status = super::GuidanceStatusDto {
            kind: super::ManagedFileKindDto::Guidance,
            file: "AGENTS.md".to_string(),
            exists: true,
            managed: super::ManagedStatusTag::Outdated,
            template_version: "hap-001-guidance-v1".to_string(),
            file_sha256: Some("ab".repeat(32)),
            file_sha256_short: Some("abababab".to_string()),
            snapshots: 2,
            pinned: 1,
        };
        assert_eq!(
            json(&status),
            serde_json::json!({
                "kind": "guidance",
                "file": "AGENTS.md",
                "exists": true,
                "managed": "outdated",
                "templateVersion": "hap-001-guidance-v1",
                "fileSha256": "ab".repeat(32),
                "fileSha256Short": "abababab",
                "snapshots": 2,
                "pinned": 1,
            })
        );
        for (status, token) in [
            (omnifrons_domain::guidance::ManagedStatus::Absent, "absent"),
            (
                omnifrons_domain::guidance::ManagedStatus::Current,
                "current",
            ),
            (
                omnifrons_domain::guidance::ManagedStatus::Outdated {
                    version: "v0".to_string(),
                },
                "outdated",
            ),
            (
                omnifrons_domain::guidance::ManagedStatus::Modified,
                "modified",
            ),
            (
                omnifrons_domain::guidance::ManagedStatus::Malformed,
                "malformed",
            ),
        ] {
            assert_eq!(
                json(&super::ManagedStatusTag::from(&status)),
                serde_json::json!(token)
            );
        }

        let preview = super::GuidancePreviewDto {
            kind: super::ManagedFileKindDto::Ignore,
            file: ".gitignore".to_string(),
            action: super::GuidanceActionTag::Insert,
            proposed: "# omnifrons:begin ignore ...".to_string(),
            file_sha256: None,
            file_sha256_short: None,
            result_sha256_short: "cdcdcdcd".to_string(),
        };
        assert_eq!(
            json(&preview),
            serde_json::json!({
                "kind": "ignore",
                "file": ".gitignore",
                "action": "insert",
                "proposed": "# omnifrons:begin ignore ...",
                "fileSha256": null,
                "fileSha256Short": null,
                "resultSha256Short": "cdcdcdcd",
            })
        );
        for (action, token) in [
            (super::GuidanceActionTag::Insert, "insert"),
            (super::GuidanceActionTag::Replace, "replace"),
            (super::GuidanceActionTag::NoOp, "no-op"),
            (super::GuidanceActionTag::Remove, "remove"),
            (super::GuidanceActionTag::Restore, "restore"),
        ] {
            assert_eq!(json(&action), serde_json::json!(token));
        }
    }

    /// The applied and snapshot payloads (spike slice 5c): ids as 16 hex,
    /// short digests, instants as ms since the epoch, a removed file's
    /// digest `null` -- never a path.
    #[test]
    fn guidance_applied_and_snapshot_dto_json_shapes() {
        let applied = super::GuidanceAppliedDto {
            kind: super::ManagedFileKindDto::Guidance,
            file: "AGENTS.md".to_string(),
            action: super::GuidanceActionTag::Remove,
            snapshot_id: Some("0123456789abcdef".to_string()),
            result_sha256_short: None,
        };
        assert_eq!(
            json(&applied),
            serde_json::json!({
                "kind": "guidance",
                "file": "AGENTS.md",
                "action": "remove",
                "snapshotId": "0123456789abcdef",
                "resultSha256Short": null,
            })
        );

        let snapshot = super::SnapshotDto {
            id: "0123456789abcdef".to_string(),
            kind: super::ManagedFileKindDto::Guidance,
            file: "AGENTS.md".to_string(),
            existed: false,
            sha256_short: "e3b0c442".to_string(),
            size: 0,
            taken_at: 1_725_782_401_000,
            pinned: true,
        };
        assert_eq!(
            json(&snapshot),
            serde_json::json!({
                "id": "0123456789abcdef",
                "kind": "guidance",
                "file": "AGENTS.md",
                "existed": false,
                "sha256Short": "e3b0c442",
                "size": 0,
                "takenAt": 1_725_782_401_000u64,
                "pinned": true,
            })
        );
    }

    /// `duplicate-publication` acknowledges the existing record in
    /// `detail`: its publication identity and Catalog identity (logical
    /// ids, never a path); the slice-2 `changed-since-approval` detail
    /// keeps its exact shape.
    #[test]
    fn shell_error_detail_shapes_for_both_carrying_codes() {
        let duplicate = ShellError::with_detail(
            ShellErrorCode::DuplicatePublication,
            "an artifact with this content is already registered for this project",
            ShellErrorDetail::DuplicatePublication {
                publication_id: "ab".repeat(32),
                catalog_id: format!("main/{}", "ab".repeat(32)),
            },
        );
        assert_eq!(
            json(&duplicate)["detail"],
            serde_json::json!({
                "publicationId": "ab".repeat(32),
                "catalogId": format!("main/{}", "ab".repeat(32)),
            })
        );
        let changed = ShellError::with_detail(
            ShellErrorCode::ChangedSinceApproval,
            "the executable's content has changed since it was approved",
            ShellErrorDetail::ChangedSinceApproval {
                recorded_sha256_short: "aaaaaaaa".to_string(),
                observed_sha256_short: "bbbbbbbb".to_string(),
            },
        );
        assert_eq!(
            json(&changed)["detail"],
            serde_json::json!({
                "recordedSha256Short": "aaaaaaaa",
                "observedSha256Short": "bbbbbbbb",
            })
        );
    }

    // -- Slice 5d: wrong-root detection, the three remedies, and the
    // run-end `misplaced` event --

    /// `wrongroot_status`'s payload: the output-discipline label, every
    /// disclosure HAP-001-R33 and R34 fix, and whether a scan has run.
    /// Never a path.
    #[test]
    fn wrong_root_status_dto_json_shape() {
        let dto = super::WrongRootStatusDto::of(
            omnifrons_domain::wrong_root::OutputDiscipline::Advisory,
            omnifrons_domain::scope::ScopeMode::Advisory,
            false,
            0,
        );
        assert_eq!(
            json(&dto),
            serde_json::json!({
                "outputDiscipline": "advisory",
                "scopeMode": "advisory",
                "disclosures": [
                    "a write inside the project but outside the outbox is detected after the \
                     run, never prevented",
                    "a write outside the project is possible and is detected after the run, \
                     not prevented",
                ],
                "scanned": false,
                "findings": 0,
            })
        );
        let enforced = super::WrongRootStatusDto::of(
            omnifrons_domain::wrong_root::OutputDiscipline::Enforced,
            omnifrons_domain::scope::ScopeMode::SandboxEnforced,
            true,
            2,
        );
        assert_eq!(
            json(&enforced)["outputDiscipline"],
            serde_json::json!("enforced")
        );
        assert_eq!(
            json(&enforced)["disclosures"]
                .as_array()
                .expect("array")
                .len(),
            1,
            "HAP-001-R34's disclosure is carried even under sandbox-enforced scope"
        );
    }

    /// `misplaced_list`'s row: the project-relative name, the facts taken
    /// from the file's own handle, the reason, and the three remedies --
    /// never a device path (RCS-001-R14).
    #[test]
    fn misplaced_dto_json_shape() {
        let finding = omnifrons_domain::wrong_root::MisplacedFinding::new(
            "docs/report.pdf",
            4096,
            omnifrons_domain::executable::Sha256Digest([0xab; 32]),
            omnifrons_domain::outbox::DetectedType::Pdf,
            omnifrons_domain::outbox::ArtifactClass::GeneratedHeavy,
            omnifrons_domain::wrong_root::WrongRootReason::InProjectOutsideOutbox,
        )
        .expect("a project-relative name");
        assert_eq!(
            json(&super::MisplacedDto::from_finding(&finding)),
            serde_json::json!({
                "name": "docs/report.pdf",
                "size": 4096,
                "sha256": "ab".repeat(32),
                "sha256Short": "abababab",
                "detectedType": "pdf",
                "class": "generated-heavy",
                "reason": "in-project-outside-outbox",
                "remedies": ["quarantine", "publish", "ignore"],
            })
        );
    }

    /// The run-end `misplaced` event: counts only, never a name or a path.
    #[test]
    fn misplaced_event_frame_json_shape() {
        let frame = HarnessFrame::event_dto(
            ProcessIdDto(42),
            10,
            0,
            super::AdapterEventDto::Misplaced {
                scanned: 12,
                findings: 2,
                ignored: 1,
                excluded: 3,
                unreadable: 0,
                truncated: false,
            },
        );
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "event",
                "body": {
                    "id": 42, "seq": 10, "droppedBefore": 0,
                    "kind": "misplaced",
                    "payload": {
                        "scanned": 12, "findings": 2, "ignored": 1,
                        "excluded": 3, "unreadable": 0, "truncated": false
                    }
                }
            })
        );
    }

    /// `misplaced_remedy`'s payload, one shape per remedy.
    ///
    /// Each fixture is the shape the command actually emits (R3-021): a
    /// quarantine always carries its fixed `detail` token, an ignore always
    /// carries the digest it bound to and `originalKept: true`, and a
    /// publish carries the new entry's name with a null `detail`. A fixture
    /// that contradicts its producer pins a wire nothing sends.
    #[test]
    fn misplaced_remedy_dto_json_shape() {
        let quarantined = super::MisplacedRemedyDto {
            remedy: super::RemedyTag::Quarantine,
            outcome: super::RemedyOutcomeTag::Quarantined,
            name: Some("abababab-report.pdf".to_string()),
            sha256: Some("ab".repeat(32)),
            original_kept: Some(false),
            detail: Some("renamed".to_string()),
        };
        assert_eq!(
            json(&quarantined),
            serde_json::json!({
                "remedy": "quarantine",
                "outcome": "quarantined",
                "name": "abababab-report.pdf",
                "sha256": "ab".repeat(32),
                "originalKept": false,
                "detail": "renamed",
            })
        );
        let published = super::MisplacedRemedyDto {
            remedy: super::RemedyTag::Publish,
            outcome: super::RemedyOutcomeTag::CopiedToOutbox,
            name: Some("abababab-report.pdf".to_string()),
            sha256: Some("ab".repeat(32)),
            original_kept: Some(true),
            detail: None,
        };
        assert_eq!(
            json(&published),
            serde_json::json!({
                "remedy": "publish",
                "outcome": "copied-to-outbox",
                "name": "abababab-report.pdf",
                "sha256": "ab".repeat(32),
                "originalKept": true,
                "detail": null,
            })
        );
        let ignored = super::MisplacedRemedyDto {
            remedy: super::RemedyTag::Ignore,
            outcome: super::RemedyOutcomeTag::Ignored,
            name: None,
            sha256: Some("ab".repeat(32)),
            original_kept: Some(true),
            detail: None,
        };
        assert_eq!(
            json(&ignored),
            serde_json::json!({
                "remedy": "ignore",
                "outcome": "ignored",
                "name": null,
                "sha256": "ab".repeat(32),
                "originalKept": true,
                "detail": null,
            })
        );
    }

    /// `misplaced_remedy`'s `remedy` field parses the three tokens and
    /// nothing else.
    #[test]
    fn remedy_tag_deserializes_from_kebab_case_only() {
        for remedy in omnifrons_domain::wrong_root::Remedy::ALL {
            let parsed: super::RemedyTag =
                serde_json::from_str(&format!("\"{}\"", remedy.as_str())).expect("token");
            assert_eq!(json(&parsed), serde_json::json!(remedy.as_str()));
        }
        assert!(serde_json::from_str::<super::RemedyTag>("\"delete\"").is_err());
        assert!(serde_json::from_str::<super::RemedyTag>("\"Quarantine\"").is_err());
    }

    /// The three slice-5d error codes each render as their documented
    /// kebab-case token.
    #[test]
    fn slice_5d_error_codes_serialize_as_kebab_case() {
        let cases = [
            (ShellErrorCode::MisplacedUnknown, "misplaced-unknown"),
            (
                ShellErrorCode::QuarantineUnavailable,
                "quarantine-unavailable",
            ),
            (ShellErrorCode::ScanFailed, "scan-failed"),
        ];
        for (code, token) in cases {
            let error = ShellError::new(code, "message");
            assert_eq!(json(&error)["code"], serde_json::json!(token));
            assert!(
                !error.message.contains('/') && !error.message.contains('\\'),
                "a catalogue message never carries a path separator"
            );
        }
    }
}
