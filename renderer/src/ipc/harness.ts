/**
 * The renderer's only import of `@tauri-apps/api/core`. Every other
 * renderer module reaches the demo-harness and executable-approval IPC
 * surface through this module's typed wrappers, never `invoke` directly
 * (`docs/spike-log.md` § IPC contract).
 *
 * The types below mirror `src-tauri/src/ipc/dto.rs` field-for-field: the
 * renderer only ever sees these shapes, never a Rust domain type.
 *
 * ## Mocking `Channel` in tests
 *
 * `@tauri-apps/api/mocks`' `mockIPC(cb)` replaces
 * `window.__TAURI_INTERNALS__.invoke`, which is exactly what the real
 * `invoke()` in `@tauri-apps/api/core` calls with its `args` object
 * untouched -- no JSON serialization happens at that layer (serialization
 * to the wire only happens deeper, inside the real bridge `mockIPC`
 * replaces). So the `onFrame` key the mock handler receives in `args` is
 * the exact live `Channel` instance `harnessSpawn` constructed, not a
 * serialized placeholder string. Calling `args.onFrame.onmessage(frame)`
 * from inside the mock handler is therefore the approach that works, and
 * is what `harness.test.ts` does; no `Channel` mock or module-level hook
 * is needed.
 */
import { Channel, invoke } from '@tauri-apps/api/core'

/**
 * The closed set of harness kinds a caller may request, as it crosses IPC.
 *
 * Internally tagged, mirroring `src-tauri/src/ipc/dto.rs`'s
 * `HarnessKindDto` verbatim: each variant bundles exactly the parameters
 * that kind needs, rather than a flat `kind` string plus separate
 * `rateHz`/`lines`/`approvalId` fields most of which would be meaningless
 * for any given kind -- an invalid combination (e.g. `approved` with a
 * `rateHz`) is inexpressible on the wire, not merely rejected after the
 * fact (`docs/spike-log.md` § Slice 2).
 */
/**
 * Launch the built-in adapter named by `adapterId` against the executable
 * behind `approvalId`, delivering `prompt` -- added in the spike slice-3
 * spike (`docs/spike-log.md` § Slice 3). Carries no `rateHz`/`lines`, and no
 * program path or argument vector: `adapterId`'s own fixed `argvTemplate`
 * supplies the only argv this launch will ever use, entirely Rust-side.
 */
export type HarnessKind =
  | { type: 'demo-lines'; rateHz: number; lines: number }
  | { type: 'demo-ignores-sigterm'; rateHz: number; lines: number }
  | { type: 'approved'; approvalId: ApprovalId }
  | { type: 'adapter'; adapterId: string; approvalId: ApprovalId; prompt: string }

/** A process identifier crossing IPC: a bare number on the wire. */
export type ProcessId = number

/** A probed-but-not-yet-approved candidate's opaque handle. */
export type CandidateId = number

/** An approval id, as it crosses IPC. */
export type ApprovalId = number

/** The closed set of terminal-state tokens a process can report. */
export type ProcessTerminalStateTag = 'exited' | 'killed' | 'orphan-risk/uncertain'

/**
 * A process's terminal state: `state` is always one of the three closed
 * tokens, and `code` is always present -- `null` except when `state` is
 * `exited` and the platform reported an exit code.
 */
export interface ProcessTerminalState {
  state: ProcessTerminalStateTag
  code: number | null
}

/** A process's observed status, as `harness_observe` returns it. */
export type ProcessStatus =
  | { status: 'running' }
  | ({ status: 'terminal' } & ProcessTerminalState)

interface HarnessTextFrameBody {
  id: ProcessId
  seq: number
  droppedBefore: number
  /**
   * `true` when this frame's `text` is only the head of a line whose
   * remainder follows in the next text frame on the same stream -- the
   * supervisor splits a captured line exceeding its per-frame byte cap
   * across consecutive frames and flags every frame but the last. `false`
   * when `text` ends the line. Mirrors `src-tauri/src/ipc/dto.rs`'s text
   * frame body field-for-field (spike slice 3); no renderer surface
   * reassembles split lines yet, so this field is carried but not rendered.
   */
  continued: boolean
  text: string
}

interface HarnessStateFrameBody extends ProcessTerminalState {
  id: ProcessId
  seq: number
  droppedBefore: number
}

/**
 * [`omnifrons_domain::adapter::AgentPhase`]'s tag, as it crosses IPC --
 * paired with a `state` {@link AgentEvent}'s own `subtype` as an
 * always-present (`null` unless `finished`) sibling field.
 */
export type AgentPhaseTag = 'init' | 'finished' | 'exited'

/** One named text observation accompanying a `state` {@link AgentEvent}. */
export interface Observation {
  key: string
  value: string
}

/**
 * The closed set of typed terminal actions a `pty-cli` launch's output may
 * produce (spike slice 4), mirroring `dto.rs`'s `TerminalActionKindDto`:
 * the only two sequence families RCS-001's terminal policy turns into a
 * value (an OSC 0/2 title, an OSC 9 notification), both already sanitized
 * by core -- control and bidirectional-override characters stripped, at
 * most 256 characters. Data to show as text only: never applied to any
 * chrome, `document.title`, or a notification surface.
 */
export type TerminalActionKind = 'title' | 'notification'

/**
 * [`omnifrons_domain::terminal::DropCounts`], as it crosses IPC: per-family
 * counts of terminal control sequences core's normalizer dropped since the
 * previous `terminal-drops` event of the same launch, one field per row of
 * RCS-001's policy table (`fileTransfer` is `file_transfer` in camelCase,
 * like every other wire field).
 */
export interface TerminalDropCounts {
  layout: number
  hyperlink: number
  clipboard: number
  fileTransfer: number
  string: number
  unknown: number
  malformed: number
}

/**
 * One entry an `artifact.publish` proposal names, as it crosses IPC
 * (spike slice 5, HAP-001-R12): the harness's own claim -- a name and the
 * full 64-hex digest it asserts -- never a fact Omnifrons verified. Both
 * are harness-originated text: plain text only, `name` never resolved as
 * a path by the renderer.
 */
export interface ProposedEntry {
  name: string
  sha256: string
}

/**
 * The shell's own run-end inventory summary of a run's subdirectory, as
 * the `candidates` event carries it (spike slice 5): counts by state and
 * attribution, plus the run id `candidatesList` takes -- never a path.
 * `unreadable` counts entries that could not be opened at all (excluded
 * from the candidates), `unmatchedProposals` the proposal entries whose
 * digest no entry carried.
 */
export interface CandidatesSummary {
  runId: string
  total: number
  candidate: number
  outboxEscape: number
  outboxLinked: number
  attributed: number
  unattributed: number
  unreadable: number
  unmatchedProposals: number
}

/**
 * [`omnifrons_domain::adapter::AdapterEvent`], as it crosses IPC.
 * Adjacently tagged (`kind` + `payload`), mirroring
 * `src-tauri/src/ipc/dto.rs`'s `AdapterEventDto` verbatim -- added in the
 * spike slice-3 spike (`docs/spike-log.md` § Slice 3).
 *
 * `unknown`'s `raw` is a plain, already-control-strippable string, never
 * base64 or a byte array -- see `AdapterEventDto::Unknown`'s own doc
 * comment for why lossy decoding is the expected shape here, not a
 * defensive escape hatch this type needs to represent separately.
 *
 * The three `terminal-*` kinds (spike slice 4, `docs/spike-log.md` § Slice
 * 4) are proposed AEC-001 kinds a `pty-cli` launch emits: `terminal-text`
 * is normalized plain text whose `text` keeps a `\n` per framed line end
 * and may end without one (a continued chunk), so a consumer splits lines
 * itself; `terminal-action` is a sanitized title or notification, data to
 * show as text only; `terminal-drops` is the per-family count of dropped
 * control sequences since the previous `terminal-drops` of the same launch.
 *
 * Two kinds (spike slice 5, `docs/spike-log.md` § Slice 5) are the outbox's:
 * `artifact-publish` is the typed publish proposal a line agent recognized
 * -- the harness's claim, entries named by full digest, a proposal the
 * renderer only ever shows and never executes -- and `candidates` is the
 * shell's own run-end inventory summary, emitted once per adapter launch
 * before the terminal `state` frame.
 */
export type AgentEvent =
  | {
      kind: 'state'
      payload: { phase: AgentPhaseTag; subtype: string | null; observations: Observation[] }
    }
  | { kind: 'message'; payload: { text: string } }
  | { kind: 'tool-call'; payload: { name: string; argumentsText: string } }
  | { kind: 'diagnostic'; payload: { text: string } }
  | { kind: 'unknown'; payload: { raw: string; truncated: boolean } }
  | { kind: 'terminal-text'; payload: { text: string } }
  | { kind: 'terminal-action'; payload: { action: TerminalActionKind; text: string } }
  | { kind: 'terminal-drops'; payload: TerminalDropCounts }
  | { kind: 'artifact-publish'; payload: { entries: ProposedEntry[] } }
  | { kind: 'candidates'; payload: CandidatesSummary }

interface HarnessEventFrameBody {
  id: ProcessId
  seq: number
  droppedBefore: number
}

/** One streamed frame delivered over `harness_spawn`'s `onFrame` channel. */
export type HarnessFrame =
  | { stream: 'stdout'; body: HarnessTextFrameBody }
  | { stream: 'stderr'; body: HarnessTextFrameBody }
  | { stream: 'state'; body: HarnessStateFrameBody }
  | { stream: 'event'; body: HarnessEventFrameBody & AgentEvent }

/** The closed set of error codes a failed IPC command reports. */
export type ShellErrorCode =
  | 'unknown-process'
  | 'spawn-failed'
  | 'already-subscribed'
  | 'invalid-request'
  | 'too-many-processes'
  | 'no-candidate'
  | 'not-executable'
  | 'probe-failed'
  | 'too-large'
  | 'unapproved'
  | 'changed-since-approval'
  | 'shadowed-path'
  | 'revoked'
  | 'approval-store-unavailable'
  /** `harness_spawn`'s `kind: "adapter"` named an `adapterId` outside the closed, built-in adapter set. */
  | 'unknown-adapter'
  /** An adapter launch's prompt exceeds the 16 KiB size cap. */
  | 'prompt-too-large'
  /** No active workspace was picked, or an adapter's `cwd` resolves outside it. */
  | 'workspace-unavailable'
  /** An adapter's declared (or otherwise requested) environment variable name looks secret-shaped. */
  | 'secret-shaped-env'
  /** `workspace_pick`'s folder dialog was canceled (no folder was selected). */
  | 'no-workspace'
  /**
   * `harness_spawn`'s `kind: "adapter"` named the `pty-cli` adapter on a
   * platform where spike slice 4 implements no pseudo-terminal launch
   * (Windows). Nothing was spawned.
   */
  | 'pty-unsupported'
  /**
   * A `pty-cli` launch's prompt contains a C0 control character other than
   * newline and tab (or DEL) -- something a terminal's line discipline would
   * interpret rather than type; refused at plan-building time, nothing was
   * spawned (spike slice 4).
   */
  | 'prompt-not-typeable'
  /**
   * The project's outbox declaration is invalid -- the classification
   * policy could not be loaded, or the declared path resolves outside the
   * project or is a link -- so ingestion is blocked (`candidates_list`;
   * HAP-001-R8, spike slice 5).
   */
  | 'outbox-invalid'
  /**
   * An adapter launch could not prepare its run subdirectory: the outbox
   * failed its pre-creation check, something already sits at the
   * subdirectory's path, or handle verification failed; nothing was spawned
   * (HAP-001-R10, D14; spike slice 5).
   */
  | 'outbox-unavailable'
  /**
   * The published copy's digest did not verify against the approved
   * digest, or the held handle's bytes changed since they were digested;
   * the copy is discarded and the entry preserved (`artifact_publish`;
   * HAP-001-R21, spike slice 5b).
   */
  | 'integrity-mismatch'
  /**
   * The publication identity is already registered for this project; the
   * existing record is acknowledged by the two ids in
   * {@link DuplicatePublicationDetail} and nothing new was published
   * (`artifact_publish`; HAP-001-R23, spike slice 5b).
   */
  | 'duplicate-publication'
  /**
   * The product work area resolves inside a registered workspace root, or
   * its journal cannot be used; the operation is refused (HAP-001-R7,
   * spike slice 5b).
   */
  | 'work-area-invalid'
  /**
   * The project declares no asset root, or its device asset path resolves
   * inside a registered workspace root or cannot be written; nothing is
   * copied (HAP-001-R6, R14; spike slice 5b).
   */
  | 'destination-invalid'
  /**
   * At publish time the entry's path no longer names the held handle's
   * file, or the handle is not a regular file; nothing is published and
   * the held bytes are kept as a recovery entry (`artifact_publish`;
   * HAP-001-R18, spike slice 5b).
   */
  | 'outbox-escape'
  /**
   * The held handle's link count is greater than one at publish time
   * (`artifact_publish`; HAP-001-R20, spike slice 5b).
   */
  | 'outbox-linked'
  /**
   * The candidate cannot be approved in its state or class, its digest
   * does not match the approval request, or the re-opened entry's identity
   * facts changed since approval (`artifact_approve`/`artifact_publish`;
   * HAP-001-R22, spike slice 5b).
   */
  | 'refused'
  /** The project's Catalog could not be read or written (spike slice 5b). */
  | 'catalog-unavailable'
  /**
   * `artifact_approve` or `artifact_publish` was called while a supervised
   * process is still running: the shell's mirror of the renderer's frozen
   * publication surface, so a request that bypasses the renderer's guard
   * meets the same answer (spike slice 5b, renderer risk review R1-001).
   * Open again once every run has reached its terminal state; a wait, not
   * a verdict on anything.
   */
  | 'run-active'

/** `changed-since-approval`'s detail: both digests as short hex prefixes. */
export interface ChangedSinceApprovalDetail {
  recordedSha256Short: string
  observedSha256Short: string
}

/**
 * `duplicate-publication`'s detail (spike slice 5b): the existing record's
 * logical identities -- HAP-001-R23's "acknowledge the existing record" --
 * a 64-hex publication identity and the `<asset root id>/<publication
 * hex>` Catalog identity. Display-only text, never a path.
 */
export interface DuplicatePublicationDetail {
  publicationId: string
  catalogId: string
}

/**
 * Structured detail for a {@link ShellError}, carrying values a fixed
 * catalogue `message` must not. Mirrors `dto.rs`'s untagged
 * `ShellErrorDetail`: one field set per carrying code, and the wire shape
 * of the slice-2 `changed-since-approval` detail is unchanged.
 */
export type ShellErrorDetail = ChangedSinceApprovalDetail | DuplicatePublicationDetail

/**
 * A failed IPC command's error payload. `detail` is present only for the
 * two codes that carry structured, non-path data alongside the message,
 * each with its own shape -- discriminated on `code` here, so a banner
 * that narrows on the code sees the matching detail type and no other;
 * every other code carries none.
 */
export type ShellError =
  | {
      code: Exclude<ShellErrorCode, 'changed-since-approval' | 'duplicate-publication'>
      message: string
      detail?: undefined
    }
  | { code: 'changed-since-approval'; message: string; detail?: ChangedSinceApprovalDetail }
  | { code: 'duplicate-publication'; message: string; detail?: DuplicatePublicationDetail }

/** Type guard for a rejected `invoke`'s error value being a {@link ShellError}. */
export function isShellError(error: unknown): error is ShellError {
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    'message' in error &&
    typeof (error as { code: unknown }).code === 'string' &&
    typeof (error as { message: unknown }).message === 'string'
  )
}

/** [`omnifrons_domain::executable::PlatformEvidence`], as it crosses IPC. */
export type PlatformEvidence =
  | { os: 'unix'; mode: number }
  | { os: 'windows'; extension: string; attributes: number }

/**
 * An executable identity's evidence, as it crosses IPC. `canonicalPath` is
 * the *one* place a filesystem path reaches the renderer at all in this
 * surface (TM-001-R7 display) -- the raw path a user picked in the OS
 * file dialog is never returned.
 */
export interface Evidence {
  canonicalPath: string
  size: number
  sha256: string
  sha256Short: string
  modifiedAt: number | null
  platform: PlatformEvidence
}

/** `executable_pick_and_probe`'s success payload. */
export interface ProbeResult {
  candidateId: CandidateId
  evidence: Evidence
}

/** An approval's status tag, as it crosses IPC. */
export type ApprovalStatus = 'active' | 'revoked'

/** One approval on record, as it crosses IPC. */
export interface Approval {
  approvalId: ApprovalId
  evidence: Evidence
  approvedAt: number
  status: ApprovalStatus
  /** Ms since epoch once `status` is `revoked`; `null` while `active`. */
  revokedAt: number | null
}

// -- Slice 3: workspace, adapters, and the "event" harness frame --

/**
 * A workspace directory, as it crosses IPC. `displayPath` is the
 * workspace's canonical filesystem path -- the same, deliberate exception
 * `Evidence.canonicalPath` already makes for TM-001-R7 display; a workspace
 * path is not secret, and an adapter's process runs with it as its own
 * working directory regardless.
 */
export interface Workspace {
  displayPath: string
  /**
   * Whether the product work area still resolves outside this workspace,
   * re-checked on every workspace registration (HAP-001-R7; spike slice 5b)
   * so the next publication command is not the first to notice a workspace
   * registered over it. Mirrors `dto.rs`'s `WorkspaceDto.work_area`.
   */
  workArea: WorkAreaState
}

/**
 * The product work area's state against the active workspace, as it
 * crosses IPC (`docs/spike-log.md` § Slice 5b): `valid`, or
 * `work-area-invalid` when the area resolves inside the workspace root --
 * every publication command refuses with the same-named `ShellErrorCode`
 * until it is fixed. Mirrors `dto.rs`'s `WorkAreaStateTag`.
 */
export type WorkAreaState = 'valid' | 'work-area-invalid'

/** [`omnifrons_domain::adapter::TransportClass`], as it crosses IPC. */
export type TransportClass = 'structured-streaming-cli' | 'pty'

/**
 * [`omnifrons_domain::adapter::PromptChannel`], as it crosses IPC.
 * `pty-typed` (spike slice 4) is the `pty-cli` adapter's channel: the
 * prompt is typed into the child's controlling terminal followed by a
 * carriage return; nothing is closed.
 */
export type PromptChannel = 'stdin-then-close' | 'argv' | 'pty-typed'

/**
 * [`omnifrons_domain::scope::ScopeMode`], as it crosses IPC -- every
 * built-in adapter in this slice reports `advisory` (`docs/spike-log.md`
 * § Slice 3).
 */
export type ScopeMode = 'sandbox-enforced' | 'harness-enforced' | 'advisory'

/**
 * One built-in adapter's descriptor, as it crosses IPC -- metadata only:
 * deliberately never `argvTemplate` or `declaredEnv`'s resolved values,
 * which are the shell's own launch-time concern, never the renderer's.
 */
export interface AdapterDescriptor {
  id: string
  displayName: string
  transportClass: TransportClass
  promptChannel: PromptChannel
  scopeMode: ScopeMode
  notes: string
}

/**
 * Spawn a harness or an approved executable, and stream its captured
 * output to `onFrame`.
 *
 * Creates a fresh `Channel<HarnessFrame>`, wires its `onmessage` to
 * `onFrame`, and invokes `harness_spawn` with exactly `{ kind, onFrame }`
 * -- `kind` alone carries every parameter that variant needs (`rateHz`/
 * `lines` for a demo kind, `approvalId` for `approved`), so no top-level
 * program path or argument vector ever crosses this call, matching the
 * IPC contract in `docs/spike-log.md`.
 */
export async function harnessSpawn(
  kind: HarnessKind,
  onFrame: (frame: HarnessFrame) => void,
): Promise<ProcessId> {
  const channel = new Channel<HarnessFrame>()
  channel.onmessage = onFrame

  return invoke('harness_spawn', { kind, onFrame: channel })
}

/** Stop a spawned demo harness, gracefully then forcefully, within `deadlineMs`. */
export async function harnessStop(
  id: ProcessId,
  deadlineMs: number,
): Promise<ProcessTerminalState> {
  return invoke('harness_stop', { id, deadlineMs })
}

/** Observe a spawned demo harness's current status. */
export async function harnessObserve(id: ProcessId): Promise<ProcessStatus> {
  return invoke('harness_observe', { id })
}

/**
 * Open the native OS file picker and probe whatever the user selected.
 * The raw path the user picked is never returned -- only
 * `evidence.canonicalPath`, the resolved path (TM-001-R7 display).
 */
export async function executablePickAndProbe(): Promise<ProbeResult> {
  return invoke('executable_pick_and_probe')
}

/** Approve a previously probed candidate. */
export async function executableApprove(candidateId: CandidateId): Promise<Approval> {
  return invoke('executable_approve', { candidateId })
}

/** Revoke a previously recorded approval. */
export async function executableRevoke(approvalId: ApprovalId): Promise<void> {
  await invoke('executable_revoke', { approvalId })
}

/** List every approval on record. */
export async function approvalsList(): Promise<Approval[]> {
  return invoke('approvals_list')
}

/**
 * Open the native OS folder picker and, on a selection, make it this
 * shell's single active workspace, replacing any previously active one.
 *
 * # Errors
 * Rejects with `no-workspace` if the dialog was canceled, or the selected
 * path could not be resolved into a valid, existing directory.
 */
export async function workspacePick(): Promise<Workspace> {
  return invoke('workspace_pick')
}

/** The currently active workspace, or `null` if none has been picked yet. */
export async function workspaceCurrent(): Promise<Workspace | null> {
  return invoke('workspace_current')
}

/**
 * List every built-in adapter's descriptor -- metadata only, never argv or
 * resolved environment values (`docs/spike-log.md` § Slice 3).
 */
export async function adaptersList(): Promise<AdapterDescriptor[]> {
  return invoke('adapters_list')
}

// -- Slice 5: the outbox status and the candidates inventory --

/**
 * The outbox's state for the active workspace, mirroring `dto.rs`'s
 * `OutboxStateTag`: `valid` (the declaration is valid; the outbox exists,
 * or does not exist yet and the first adapter launch creates it),
 * `outbox-invalid` (the declared path resolves outside the project or is a
 * link, or the policy declaring it could not be loaded; HAP-001-R8), or
 * `outbox-unavailable` (something that is not a directory sits at the
 * declared path, or it could not be read; HAP-001-R10).
 */
export type OutboxState = 'valid' | 'outbox-invalid' | 'outbox-unavailable'

/**
 * Why `outboxStatus` reports a non-`valid` state: a fixed token, never the
 * underlying error's text, mirroring `dto.rs`'s `OutboxReasonTag`.
 */
export type OutboxReason =
  | 'outside-project'
  | 'link'
  | 'not-a-directory'
  | 'unreadable'
  | 'policy-unreadable'
  | 'policy-corrupt'
  | 'policy-invalid'

/**
 * `outbox_status`'s payload. `outbox` is the outbox's canonical
 * filesystem path, present only when the declaration is valid and the
 * directory exists: identity evidence flowing core -> renderer for display
 * (where a run's output lands) -- the third explicit exception to
 * RCS-001-R14's no-raw-path rule alongside `Evidence.canonicalPath` and
 * `Workspace.displayPath` (`docs/spike-log.md` § Slice 5). Display-only:
 * never sent back in any command, and never shown for a directory that
 * failed validation. `declared` and `policyPath` are project-relative
 * names, not device paths; `declared` is `null` when the policy itself
 * could not be loaded.
 */
export interface OutboxStatus {
  declared: string | null
  outbox: string | null
  exists: boolean
  state: OutboxState
  reason: OutboxReason | null
  policyPath: string
  /**
   * The policy's asset root identity token (spike slice 5b): the destination
   * HAP-001-R22 shows on the approval surface before the decision -- one
   * token, never a path; `null` when the policy declares none or could not
   * be loaded, in which case every approval would fail `destination-invalid`.
   * Mirrors `OutboxStatusDto.asset_root_id`.
   */
  assetRootId: string | null
}

/**
 * [`omnifrons_domain::outbox::CandidateState`], as it crosses IPC: a
 * validated `candidate`, or a refused entry -- `outbox-escape` (a link or a
 * non-regular file, never dereferenced) or `outbox-linked` (a link count
 * above one) -- carrying no digest facts (HAP-001-R20).
 */
export type CandidateState = 'candidate' | 'outbox-escape' | 'outbox-linked'

/** [`omnifrons_domain::outbox::ArtifactClass`], as it crosses IPC. */
export type ArtifactClass =
  | 'generated-heavy'
  | 'git-tracked'
  | 'portable-text'
  | 'executable'
  | 'unclassified'

/**
 * [`omnifrons_domain::outbox::Attribution`], as it crosses IPC: attributed
 * to the run whose own publish proposal named the entry by digest, or
 * unattributed -- location alone never attributes (HAP-001-R11).
 */
export type Attribution = { kind: 'run'; runId: string } | { kind: 'unattributed' }

/**
 * One candidate entry, as `candidates_list` returns it: its name relative
 * to the outbox (a producer-supplied name -- plain text only, never
 * resolved as a path by the renderer), the facts taken from its handle --
 * every one `null` for a refused entry, where nothing was digested -- its
 * attribution, and its state. `detectedType` is a `DetectedType` token
 * (`pdf`, `png`, `markdown`, `plain-text`, `unknown`, ...), shown as text.
 * Never a device path.
 */
export interface Candidate {
  name: string
  size: number | null
  /**
   * The full 64-hex content digest (spike slice 5b): the identity fact
   * `artifactApprove` names the candidate by, so an approval is made from
   * the row itself -- identity evidence, not a secret, under the same
   * TM-001-R7 display precedent as `Evidence.sha256` -- and `null` for a
   * refused entry, where nothing was digested. Mirrors
   * `CandidateDto.sha256`.
   */
  sha256: string | null
  sha256Short: string | null
  detectedType: string | null
  class: ArtifactClass | null
  attribution: Attribution
  state: CandidateState
}

/**
 * The outbox's status for the active workspace (`docs/spike-log.md` §
 * Slice 5). Read-only: a status query never creates the outbox.
 *
 * # Errors
 * Rejects with `workspace-unavailable` if no workspace is active.
 */
export async function outboxStatus(): Promise<OutboxStatus> {
  return invoke('outbox_status')
}

/**
 * The candidate entries of one run (`runId` given: the inventory taken at
 * that run's end, attributed by the run's own proposals) or of the whole
 * outbox (`runId` omitted: every entry, unattributed, as a proposal only).
 * The argument object carries a `runId` key only when one was given, so
 * the whole-outbox request crosses IPC as exactly `{}`.
 *
 * # Errors
 * Rejects with `invalid-request` if `runId` names no remembered run or a
 * run that has not ended; `outbox-invalid`/`outbox-unavailable` if the
 * whole-outbox inventory cannot run; `workspace-unavailable` if no
 * workspace is active for a whole-outbox request.
 */
export async function candidatesList(runId?: string): Promise<Candidate[]> {
  return invoke('candidates_list', runId === undefined ? {} : { runId })
}

// -- Slice 5b: approval, publication, and the Catalog listing --

/**
 * [`omnifrons_domain::publication::ArtifactState`], as it crosses IPC
 * (`docs/spike-log.md` § Slice 5b): the ten closed state tokens HAP-001's
 * signal mapping spells, exactly, and nothing else (HAP-001-R26) --
 * mirroring `dto.rs`'s `ArtifactStateTag`. Transcribed verbatim wherever a
 * surface shows one (`docs/target-architecture.md` invariant 8).
 */
export type ArtifactState =
  | 'candidate'
  | 'published-local'
  | 'registered'
  | 'provider-synced'
  | 'registration-pending'
  | 'refused'
  | 'integrity-mismatch'
  | 'duplicate-publication'
  | 'outbox-escape'
  | 'outbox-linked'

/** [`omnifrons_domain::publication::ProviderState`], as it crosses IPC: the record's `provider_state`. */
export type ProviderState = 'pending' | 'synced' | 'failed' | 'unavailable'

/**
 * This device's own availability observation for a publication
 * (HAP-001-R27): `local` when this device's journal shows the bytes
 * verified here, `unknown` otherwise -- never inferred from the record.
 */
export type Availability = 'local' | 'unknown'

/**
 * The act-as identity an artifact approval binds (HAP-001-R22, TM-001-R7):
 * the device-local user, as an opaque wire token -- the one actor this
 * shell can bind an approval to.
 */
export type ActAs = 'device-local-user'

/**
 * An artifact approval id, as it crosses IPC: 16 lowercase hex characters
 * (a derived `u64` does not survive a JSON number's 53-bit mantissa, so it
 * is a string on the wire, unlike the slice-2 executable {@link ApprovalId}).
 * Opaque: never parsed by the renderer.
 */
export type ArtifactApprovalId = string

/** A publication identity, as it crosses IPC: 64 hex characters. Opaque. */
export type PublicationId = string

/**
 * AEC-001's `ref` for an artifact (HAP-001 § Definitions, "Portable
 * reference"): `{ kind: 'artifact', id: <publication identity>, locator:
 * <artifact Catalog identity> }`. `locator` is `<asset root id>/<publication
 * hex>` -- display-only text the renderer never resolves as a path, a link,
 * or an address, and never a device path.
 */
export interface PortableReference {
  kind: 'artifact'
  id: PublicationId
  locator: string
}

/**
 * `artifact_approve`'s payload (`docs/spike-log.md` § Slice 5b): the
 * identity-bound facts the approval surface showed (HAP-001-R22) --
 * `name` the entry as `candidates_list` listed it, `displayName` the
 * sanitized record name (HAP-001-R24), the digest's short form, size,
 * detected type, class, the attribution -- plus the destination asset
 * root identity, the act-as identity, and the derived ids. Mirrors
 * `ArtifactApprovalDto` field-for-field. Never a device path.
 */
export interface ArtifactApproval {
  approvalId: ArtifactApprovalId
  publicationId: PublicationId
  runId: string
  name: string
  displayName: string
  sha256Short: string
  size: number
  detectedType: string
  class: ArtifactClass
  attribution: Attribution
  assetRootId: string
  actAs: ActAs
  approvedAt: number
}

/**
 * One publication, as `artifact_publish` returns it and `publications_list`
 * lists it (`docs/spike-log.md` § Slice 5b): the publication identity, its
 * state, the portable reference once `registered` (HAP-001-R24; `null`
 * before), the record's provider state and Catalog identity once a record
 * exists, the sanitized display names (the request's, plus aliases later
 * identical publications added), and this device's own availability
 * observation. Mirrors `PublicationDto` field-for-field. Never a device
 * path.
 */
export interface Publication {
  publicationId: PublicationId
  state: ArtifactState
  reference: PortableReference | null
  providerState: ProviderState | null
  catalogId: string | null
  names: string[]
  availability: Availability
}

/**
 * The frame `artifact_publish`'s `onState` channel carries on every
 * transition, in order (proposed AEC-001 kind `artifact.state`;
 * HAP-001-R35): adjacently tagged like {@link AgentEvent}, mirroring
 * `dto.rs`'s `ArtifactStateFrame` -- a separate one-kind frame on the
 * publish command's own channel, never a member of the adapter channel's
 * `AgentEvent` union, which the Rust side never sends it on. Never a
 * path; the event observes and controls nothing.
 */
export type ArtifactStateFrame = {
  kind: 'artifact-state'
  payload: {
    publicationId: PublicationId
    state: ArtifactState
    providerState: ProviderState | null
  }
}

/**
 * Approve one candidate of a remembered run for publication, by the run
 * id, the entry name exactly as `candidatesList` listed it, and the
 * entry's full 64-hex digest -- the row's own `Candidate.sha256`, the
 * identity fact the shell listed, never a typed value or a proposal's
 * digest (`docs/spike-log.md` § Slice 5b). The shell looks the entry up in
 * its own run-end inventory and refuses a digest that does not match it,
 * a refused entry, or a class other than `generated-heavy`; the returned
 * approval carries the identity-bound facts the surface showed. Invokes
 * `artifact_approve` with exactly `{ runId, name, sha256 }` -- no
 * path-shaped key.
 *
 * # Errors
 * Rejects with `refused` (an entry that cannot be approved, or a digest
 * that does not match), `destination-invalid` (the project declares no
 * asset root), `work-area-invalid`, `invalid-request` (an unknown run or
 * name, or a digest that is not 64 hex characters), `outbox-invalid`,
 * `run-active` (a supervised process is still running), or
 * `workspace-unavailable`.
 */
export async function artifactApprove(
  runId: string,
  name: string,
  sha256: string,
): Promise<ArtifactApproval> {
  return invoke('artifact_approve', { runId, name, sha256 })
}

/**
 * Publish a previously approved candidate, streaming every state
 * transition to `onState` (`docs/spike-log.md` § Slice 5b, HAP-001-R35).
 *
 * Creates a fresh `Channel<ArtifactStateFrame>`, wires its `onmessage` to
 * `onState`, and invokes `artifact_publish` with exactly `{ approvalId,
 * onState }` -- the `harnessSpawn` pattern. Resolves with the publication
 * as the transaction left it: `registered` with its reference, or
 * `registration-pending` with none.
 *
 * # Errors
 * A failure emits its own state frame once, then rejects with the
 * matching code: `integrity-mismatch`, `duplicate-publication` (its
 * `detail` the existing record's two ids), `outbox-escape`,
 * `outbox-linked`, `refused`, `destination-invalid`, `work-area-invalid`,
 * `catalog-unavailable`, `invalid-request` (a malformed or unrecorded
 * approval id), `run-active` (a supervised process is still running), or
 * `workspace-unavailable`.
 */
export async function artifactPublish(
  approvalId: ArtifactApprovalId,
  onState: (frame: ArtifactStateFrame) => void,
): Promise<Publication> {
  const channel = new Channel<ArtifactStateFrame>()
  channel.onmessage = onState

  return invoke('artifact_publish', { approvalId, onState: channel })
}

/**
 * Every publication of the active project after the shell's restart
 * replay (`docs/spike-log.md` § Slice 5b, HAP-001-R29): the Catalog's
 * records with this device's own availability, plus any
 * `registration-pending` recovery could not complete. A fresh project is
 * `[]`.
 *
 * # Errors
 * Rejects with `catalog-unavailable`, `work-area-invalid`, or
 * `workspace-unavailable` if no workspace is active.
 */
export async function publicationsList(): Promise<Publication[]> {
  return invoke('publications_list')
}
