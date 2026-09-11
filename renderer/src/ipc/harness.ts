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
 *
 * One kind (spike slice 5d, `docs/spike-log.md` § Slice 5d) is the
 * wrong-root scan's: `misplaced` is the shell's own run-end scan of the
 * active workspace root, emitted once per adapter launch **after
 * `candidates` and before the terminal `state` frame**, with the launch's
 * `seq` contiguous across all three. Counts only -- no name and no path
 * rides it. A scan that could not run emits the fixed `diagnostic` text
 * `the project could not be scanned for output written outside the outbox`
 * in its place, never a `misplaced` frame claiming nothing was found.
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
  | { kind: 'misplaced'; payload: ScanSummary }

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
  /**
   * A guidance command named a file outside RCS-001's file-name rule (a
   * path, a reserved device name, not Markdown), or the managed file is
   * not a regular file, exceeds the size bound, is not valid UTF-8, could
   * not be read or written, or did not read back as written (spike slice
   * 5c, HAP-001 D18). The rule's own fixed message, never the name.
   */
  | 'guidance-file-invalid'
  /**
   * The managed file's digest (or its absence) differs from the
   * `fileSha256` the request bound to: the surface is stale, nothing was
   * written (spike slice 5c).
   */
  | 'guidance-file-changed'
  /**
   * The managed block was edited inside its sentinels; Omnifrons neither
   * replaces nor removes it -- the user resolves by hand or restores a
   * snapshot (spike slice 5c).
   */
  | 'guidance-block-modified'
  /** The managed block's sentinels are not one intact pair (spike slice 5c). */
  | 'guidance-block-malformed'
  /**
   * `guidance_remove` found no managed block, or `guidance_restore` or
   * `guidance_pin` named a snapshot this project does not hold (spike
   * slice 5c).
   */
  | 'guidance-unmanaged'
  /**
   * The snapshot store under the work area could not be read, is corrupt
   * (a corrupt manifest fails the whole listing), or could not be written
   * (spike slice 5c).
   */
  | 'snapshot-unavailable'
  /**
   * `misplaced_remedy` named a finding this shell's last scan did not
   * report, or reported at a different digest, or nothing sits at that name
   * any more: the surface is stale and nothing was moved, copied or
   * recorded (spike slice 5d, HAP-001-R32). Scan again and retry.
   */
  | 'misplaced-unknown'
  /**
   * The quarantine directory resolves inside a registered workspace root,
   * could not be created owner-only, or could not be written; the file
   * stays exactly where it was found (spike slice 5d, RCS-001-R10).
   */
  | 'quarantine-unavailable'
  /**
   * The wrong-root scan could not walk the active workspace root, so no
   * verdict is claimed for it (spike slice 5d) -- never a claim that
   * nothing was misplaced.
   */
  | 'scan-failed'
  /**
   * The project's Catalog is corrupt: it has lines this version cannot
   * read, and {@link catalogRepairPreview} says which. Distinct from
   * `catalog-unavailable`, which is a catalog that could not be read or
   * written at all and has no repair to offer (spike slice 5e).
   *
   * This is a **wire change to a slice-5b code**: `CatalogStoreError::Corrupt`
   * rendered `catalog-unavailable` until this slice, so `publications_list`
   * and `artifact_publish` answer a corrupt catalog with this code now.
   */
  | 'catalog-corrupt'
  /**
   * {@link catalogRepair}'s `sha256` is not the catalog's own digest any
   * more: the preview is stale and nothing was rewritten (spike slice 5e).
   */
  | 'catalog-changed'
  /**
   * The catalog carries a line this repair may not drop -- a `record` whose
   * `catalogId` disagrees with its identity, which is the only line
   * registering that artifact (HAP-001-R39), or a line this version cannot
   * decode (HAP-001-R40) -- or there is nothing to drop at all, or the
   * rules would not yield a readable catalog. Nothing was rewritten (spike
   * slice 5e).
   */
  | 'repair-refused'
  /**
   * {@link recoveryApprove} named a digest no recovery entry of this
   * device's journal carries, or whose entry could not be opened at all
   * (HAP-001-R18; spike slice 5e).
   */
  | 'recovery-unknown'

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
  /**
   * The run whose run-end inventory listed the candidate; `null` for an
   * approval made from the whole-outbox inventory (`artifactApprove` with
   * `runId` null, spike slice 5c), which lists without a run -- the run
   * subdirectory such an entry sits under, if any, is a location fact its
   * `name` carries, never provenance (HAP-001-R11).
   */
  runId: string | null
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
  /**
   * The publication whose `outbox-escape` preserved these bytes
   * (HAP-001-R18), for an approval {@link recoveryApprove} made; `null` for
   * every outbox approval (spike slice 5e).
   *
   * A **location fact**, never provenance: the record this approval
   * registers carries `producer: unattributed`, because nothing about a
   * recovery entry attributes it to a run (HAP-001-R11, R36). It is also
   * not the identity being registered -- HAP-001-R23 derives that from the
   * bytes the shell read, which differ from the escaped publication's
   * whenever the file behind the held handle was rewritten in place before
   * the escape was detected. A surface shows it so the user knows what they
   * are re-publishing, and states nothing further about it.
   */
  recoveredFrom: PublicationId | null
  /**
   * Whether the entry's handle is held for the publication that follows
   * (HAP-001-R17): `false` when HAP-001 D22's cap left no room for it, in
   * which case the publication re-opens the entry under the single-handle
   * discipline when its turn comes -- said on the wire rather than left
   * silent (spike slice 5c). The approval stands either way.
   */
  handleHeld: boolean
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
 * Approve one candidate for publication, by the run id (or `null`), the
 * entry name exactly as `candidatesList` listed it, and the entry's full
 * 64-hex digest -- the row's own `Candidate.sha256`, the identity fact the
 * shell listed, never a typed value or a proposal's digest
 * (`docs/spike-log.md` § Slice 5b). With a run id the shell looks the entry
 * up in its own run-end inventory; with `runId` null (spike slice 5c) the
 * entry is one the whole-outbox inventory listed -- at the outbox root, or
 * under a run subdirectory no remembered run's inventory covers -- and the
 * shell re-opens it once under the single-handle discipline, requires the
 * digest taken from that handle to equal the request's, and approves it
 * as unattributed with no run and no launch provenance (HAP-001-R11, R15,
 * R16, R22). Either way the shell refuses a digest that does not match, a
 * refused entry, or a class other than `generated-heavy`, and the returned
 * approval carries the identity-bound facts the surface showed. Invokes
 * `artifact_approve` with exactly `{ runId, name, sha256 }` -- the `runId`
 * key present and `null` for the whole-outbox path; no path-shaped key.
 *
 * # Errors
 * Rejects with `refused` (an entry that cannot be approved, or a digest
 * that does not match its inventory), `destination-invalid` (the project
 * declares no asset root), `work-area-invalid`, `invalid-request` (an
 * unknown run or name, a digest that is not 64 hex characters, or, with
 * `runId` null, an entry of a run the shell still remembers with its
 * inventory: "approve it through that run id"), `outbox-invalid`,
 * `run-active` (a supervised process is still running), or
 * `workspace-unavailable`; and, with `runId` null, `integrity-mismatch`
 * (the re-opened entry's digest differs from the request's: it changed
 * since it was listed), `outbox-escape` (not a regular file inside the
 * outbox), or `outbox-linked` (a link count above one).
 */
export async function artifactApprove(
  runId: string | null,
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
 * `catalog-corrupt` (the Catalog has lines this version cannot read;
 * {@link catalogRepairPreview} says which -- since spike slice 5e, where a
 * corrupt catalog stopped rendering `catalog-unavailable`),
 * `catalog-unavailable` (a Catalog that could not be read or written at
 * all), `invalid-request` (a malformed or unrecorded approval id),
 * `run-active` (a supervised process is still running), or
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
 * Rejects with `catalog-corrupt` (the Catalog has lines this version cannot
 * read; spike slice 5e re-mapped this case off `catalog-unavailable`),
 * `catalog-unavailable` (it could not be read or written at all),
 * `work-area-invalid`, or `workspace-unavailable` if no workspace is
 * active.
 */
export async function publicationsList(): Promise<Publication[]> {
  return invoke('publications_list')
}

// -- Slice 5c: the guidance-note installer --

/**
 * The closed set of files the installer manages (`docs/spike-log.md` §
 * Slice 5c), mirroring `dto.rs`'s `ManagedFileKindDto`: `guidance` is the
 * project's agent guidance file -- a user-named Markdown file at the
 * workspace root, `AGENTS.md` by default -- carrying HAP-001's fixed note
 * (HAP-001-R42, D18); `ignore` is the fixed `.gitignore` at the root,
 * carrying the one outbox ignore rule.
 */
export type GuidanceKind = 'guidance' | 'ignore'

/**
 * The managed block's state in the file, mirroring `dto.rs`'s
 * `ManagedStatusTag` -- the token only: `absent` (no block, or no file),
 * `current` (this template version, intact), `outdated` (an earlier
 * version, or this version rendered for another outbox path, intact),
 * `modified` (edited inside its sentinels: neither replaced nor removed),
 * `malformed` (the sentinels are not one intact pair).
 */
export type ManagedStatus = 'absent' | 'current' | 'outdated' | 'modified' | 'malformed'

/**
 * What a guidance command did or would do, mirroring `dto.rs`'s
 * `GuidanceActionTag`: a preview or an apply reports `insert`, `replace`,
 * or `no-op`; a removal reports `remove` and a restore `restore`.
 */
export type GuidanceAction = 'insert' | 'replace' | 'no-op' | 'remove' | 'restore'

/** A snapshot id, as it crosses IPC: 16 lowercase hex characters. Opaque: never parsed by the renderer. */
export type SnapshotId = string

/**
 * `guidance_status`'s payload (`docs/spike-log.md` § Slice 5c): the managed
 * file's name at the workspace root, whether it exists, the block's state,
 * the template version an apply would write, the file's full 64-hex digest
 * -- what an apply or a removal binds to; `null` while the file does not
 * exist -- beside its short form, and the snapshot counts. Mirrors
 * `GuidanceStatusDto` field-for-field. Never a device path.
 */
export interface GuidanceStatus {
  kind: GuidanceKind
  file: string
  exists: boolean
  managed: ManagedStatus
  templateVersion: string
  fileSha256: string | null
  fileSha256Short: string | null
  snapshots: number
  pinned: number
}

/**
 * `guidance_preview`'s payload (`docs/spike-log.md` § Slice 5c): the
 * proposal the user disposes of (HAP-001 D18) -- the block exactly as it
 * would be written, in the file's own line ending (`proposed`: display-only
 * text, never inserted as markup and never read as an instruction), the
 * action an apply would take, the file's full digest an apply must bind
 * to (`null` for an absent file) beside its short form, and the short
 * digest of the whole file afterwards. Mirrors `GuidancePreviewDto`
 * field-for-field. Never a device path.
 */
export interface GuidancePreview {
  kind: GuidanceKind
  file: string
  action: GuidanceAction
  proposed: string
  fileSha256: string | null
  fileSha256Short: string | null
  resultSha256Short: string
}

/**
 * The payload of `guidance_apply`, `guidance_remove`, and
 * `guidance_restore` (`docs/spike-log.md` § Slice 5c): what was done, the
 * snapshot taken before the write (`null` for a no-op), and the short
 * digest of the whole file afterwards (`null` once the file was removed, or
 * a snapshot of an absent file was restored). Mirrors `GuidanceAppliedDto`
 * field-for-field. Never a device path.
 */
export interface GuidanceApplied {
  kind: GuidanceKind
  file: string
  action: GuidanceAction
  snapshotId: SnapshotId | null
  resultSha256Short: string | null
}

/**
 * One snapshot, as `guidance_snapshots` lists it (`docs/spike-log.md` §
 * Slice 5c): its id, the kind and file it recorded, whether the file
 * existed, the short digest and size of the bytes recorded, the instant as
 * ms since the epoch, and whether it is pinned. Mirrors `SnapshotDto`
 * field-for-field. Never a device path.
 */
export interface Snapshot {
  id: SnapshotId
  kind: GuidanceKind
  file: string
  existed: boolean
  sha256Short: string
  size: number
  takenAt: number
  pinned: boolean
}

/**
 * The target of a guidance command as it crosses IPC: the `file` key is
 * present only when a file was named, so a request for the `ignore` kind
 * -- which always manages `.gitignore` and takes no file -- crosses as
 * exactly `{ kind: 'ignore' }` (the `candidatesList` discipline: the key
 * absent, not `undefined`).
 */
function guidanceTarget(kind: GuidanceKind, file: string | undefined): Record<string, unknown> {
  return file === undefined ? { kind } : { kind, file }
}

/**
 * The managed file's state (`docs/spike-log.md` § Slice 5c). Read-only:
 * nothing is written or snapshotted. Invokes `guidance_status` with exactly
 * `{ kind, file }`, or `{ kind }` when no file is named (the shell defaults
 * the guidance file to `AGENTS.md`, and ignores `file` for `ignore`).
 *
 * # Errors
 * Rejects with `guidance-file-invalid` (a name outside the file-name rule,
 * or a file that is not a regular UTF-8 file within the size bound),
 * `outbox-invalid` (the policy declaring the outbox path could not be
 * loaded), `work-area-invalid`, `snapshot-unavailable`, or
 * `workspace-unavailable` if no workspace is active.
 */
export async function guidanceStatus(kind: GuidanceKind, file?: string): Promise<GuidanceStatus> {
  return invoke('guidance_status', guidanceTarget(kind, file))
}

/**
 * The proposal: what `guidanceApply` would write (HAP-001 D18; the system
 * proposes, the user disposes). Read-only: nothing is written or
 * snapshotted. Invokes `guidance_preview` with exactly `{ kind, file }`, or
 * `{ kind }` when no file is named.
 *
 * # Errors
 * Rejects with `guidance-file-invalid`, `guidance-block-modified`,
 * `guidance-block-malformed`, `outbox-invalid`, `work-area-invalid`, or
 * `workspace-unavailable` if no workspace is active.
 */
export async function guidancePreview(kind: GuidanceKind, file?: string): Promise<GuidancePreview> {
  return invoke('guidance_preview', guidanceTarget(kind, file))
}

/**
 * The approved write (HAP-001-R42, D18): the managed block inserted or
 * replaced, after a snapshot of the current file, bound to `fileSha256` --
 * the full 64-hex digest the surface showed, or `null` for a file it showed
 * as absent; never a typed value. Invokes `guidance_apply` with exactly
 * `{ kind, file, fileSha256 }`, or `{ kind, fileSha256 }` when no file is
 * named (the `ignore` kind).
 *
 * # Errors
 * Rejects with `guidance-file-changed` (the file's digest, or its absence,
 * differs from `fileSha256`: nothing written), `guidance-file-invalid`,
 * `guidance-block-modified`, `guidance-block-malformed`,
 * `snapshot-unavailable`, `outbox-invalid`, `work-area-invalid`,
 * `invalid-request` (a `fileSha256` that is not 64 hex characters),
 * `run-active` (a supervised process is still running), or
 * `workspace-unavailable`.
 */
export async function guidanceApply(
  kind: GuidanceKind,
  file: string | undefined,
  fileSha256: string | null,
): Promise<GuidanceApplied> {
  return invoke('guidance_apply', { ...guidanceTarget(kind, file), fileSha256 })
}

/**
 * The removal of the lines Omnifrons owns -- the managed block and one
 * adjacent blank line, and the file itself when Omnifrons created it --
 * after a snapshot, bound to `fileSha256` like `guidanceApply`. Invokes
 * `guidance_remove` with exactly `{ kind, file, fileSha256 }`, or
 * `{ kind, fileSha256 }` when no file is named.
 *
 * # Errors
 * Rejects with `guidance-unmanaged` (the file carries no managed block),
 * `guidance-file-changed`, `guidance-file-invalid`,
 * `guidance-block-modified`, `guidance-block-malformed`,
 * `snapshot-unavailable`, `work-area-invalid`, `invalid-request`,
 * `run-active`, or `workspace-unavailable`.
 */
export async function guidanceRemove(
  kind: GuidanceKind,
  file: string | undefined,
  fileSha256: string | null,
): Promise<GuidanceApplied> {
  return invoke('guidance_remove', { ...guidanceTarget(kind, file), fileSha256 })
}

/**
 * Every snapshot of `kind` for the active project, newest first
 * (`docs/spike-log.md` § Slice 5c). Read-only. Invokes `guidance_snapshots`
 * with exactly `{ kind }`.
 *
 * # Errors
 * Rejects with `snapshot-unavailable` (the store could not be read, or a
 * manifest is corrupt, which fails the whole listing), `work-area-invalid`,
 * or `workspace-unavailable` if no workspace is active.
 */
export async function guidanceSnapshots(kind: GuidanceKind): Promise<Snapshot[]> {
  return invoke('guidance_snapshots', { kind })
}

/**
 * Pin or unpin a snapshot: a pinned snapshot is never pruned. Invokes
 * `guidance_pin` with exactly `{ id, pinned }` and resolves with the
 * snapshot as it now stands.
 *
 * # Errors
 * Rejects with `guidance-unmanaged` (no snapshot with that id is recorded
 * for this project), `snapshot-unavailable`, `work-area-invalid`,
 * `invalid-request` (an id that is not 16 hex characters), or
 * `workspace-unavailable`.
 */
export async function guidancePin(id: SnapshotId, pinned: boolean): Promise<Snapshot> {
  return invoke('guidance_pin', { id, pinned })
}

/**
 * Restore a snapshot: the current state of the snapshot's file is
 * snapshotted first, then the recorded bytes are written back (or the file
 * removed, for a snapshot of an absent file), bound to `fileSha256` -- the
 * full digest of the snapshot's file as the surface showed it, or `null`
 * for a file it showed as absent. Invokes `guidance_restore` with exactly
 * `{ id, fileSha256 }`.
 *
 * # Errors
 * Rejects with `guidance-unmanaged` (no snapshot with that id),
 * `guidance-file-changed`, `guidance-file-invalid`, `snapshot-unavailable`
 * (the store could not be read, or the snapshot's bytes fail its manifest
 * digest), `work-area-invalid`, `invalid-request`, `run-active`, or
 * `workspace-unavailable`.
 */
export async function guidanceRestore(
  id: SnapshotId,
  fileSha256: string | null,
): Promise<GuidanceApplied> {
  return invoke('guidance_restore', { id, fileSha256 })
}

// -- Slice 5d: wrong-root detection, `misplaced`, and its three remedies --

/**
 * The output discipline the active scope reports (`docs/spike-log.md` §
 * Slice 5d, HAP-001-R33), mirroring `dto.rs`'s `OutputDisciplineTag`:
 * `enforced` only under `sandbox-enforced` scope whose declared write set
 * is the project root, `advisory` under every other mode.
 *
 * Neither value says a write inside the project but outside the outbox is
 * prevented -- it never is, in any mode. That is HAP-001-R34's disclosure,
 * carried as text on every report.
 */
export type OutputDiscipline = 'enforced' | 'advisory'

/**
 * `wrongroot_status`'s payload (`docs/spike-log.md` § Slice 5d): the
 * output-discipline label, the scope mode it was derived from -- the
 * weakest any registered adapter declares -- every disclosure HAP-001-R33
 * and R34 fix, and whether a scan has run in this session with how many
 * findings it left standing. Mirrors `WrongRootStatusDto` field-for-field.
 * Never a device path.
 *
 * `disclosures` is never empty: an `enforced` report carries HAP-001-R34's
 * line alone, an `advisory` one carries R33's beside it. The strings are
 * the contract's own honesty about what is *not* prevented, carried as
 * text so a consumer states them verbatim rather than composing its own.
 */
export interface WrongRootStatus {
  outputDiscipline: OutputDiscipline
  scopeMode: ScopeMode
  disclosures: string[]
  scanned: boolean
  findings: number
}

/**
 * The output-discipline report and whether a scan has run
 * (`docs/spike-log.md` § Slice 5d; HAP-001-R33, R34). Read-only: it starts
 * no scan and moves no file. Invokes `wrongroot_status` with no arguments.
 *
 * # Errors
 * Never rejects in the shell's own implementation -- the `Promise` matches
 * every other wrapper's shape -- so a rejection here is a transport
 * failure, not a catalogue code.
 */
export async function wrongRootStatus(): Promise<WrongRootStatus> {
  return invoke('wrongroot_status')
}

/**
 * `wrongroot_scan`'s payload (`docs/spike-log.md` § Slice 5d): what the
 * walk saw, counts only -- the findings themselves come from
 * {@link misplacedList}. Mirrors `MisplacedScanDto` field-for-field.
 * No name and no path rides this shape.
 *
 * `truncated` is `true` when the walk stopped at its own bound (the shell's
 * `MAX_SCANNED_ENTRIES`/`MAX_SCAN_DEPTH`) rather than reaching the end of
 * the project: the counts are then a partial view and the list is
 * incomplete, which a consumer must say rather than imply completeness.
 */
export interface ScanSummary {
  scanned: number
  findings: number
  ignored: number
  excluded: number
  unreadable: number
  truncated: boolean
}

/**
 * Scan the active workspace root for output written outside the outbox
 * (`docs/spike-log.md` § Slice 5d; HAP-001-R32, R43, R44) and remember the
 * findings for {@link misplacedList}. Observation only: nothing is moved,
 * copied, recorded or held, and a live run does not freeze it -- the
 * run-end scan is this same body. Invokes `wrongroot_scan` with no
 * arguments.
 *
 * # Errors
 * Rejects with `scan-failed` (the project root could not be walked, so no
 * verdict is claimed for it), `outbox-invalid` (the classification policy
 * declaring the outbox could not be loaded), `work-area-invalid` (the
 * ignore ledger could not be opened or read), or `workspace-unavailable`
 * if no workspace is active.
 */
export async function wrongRootScan(): Promise<ScanSummary> {
  return invoke('wrongroot_scan')
}

/**
 * [`omnifrons_domain::wrong_root::Remedy`], as it crosses IPC
 * (HAP-001-R32): the closed set of three, in the order HAP-001 §
 * Wrong-root detection and remedies lists them, and nothing else. Mirrors
 * `dto.rs`'s `RemedyTag`.
 *
 * Quarantine is the only one that removes the original.
 */
export type Remedy = 'quarantine' | 'publish' | 'ignore'

/**
 * Why a file is `misplaced` (HAP-001-R32), mirroring `dto.rs`'s wire token
 * for `WrongRootReason`: one arm in this slice -- the file sits inside the
 * project but outside the project's outbox. A *tracked* path is not a
 * reason here: a finding the project's classification policy calls
 * `git-tracked` is filtered out before a reason is ever assigned
 * (`docs/spike-log.md` § Slice 5d, D1).
 */
export type WrongRootReason = 'in-project-outside-outbox'

/**
 * One `misplaced` file, as `misplaced_list` returns it
 * (`docs/spike-log.md` § Slice 5d): the project-relative name it was found
 * at -- producer-supplied text, plain text only, never resolved as a path
 * and never used to build a link by the renderer -- the facts taken from
 * its own handle, why it is misplaced, and the remedies HAP-001-R32 offers
 * for this row. Mirrors `MisplacedDto` field-for-field. Never a device
 * path (RCS-001-R14).
 *
 * `remedies` is the row's own list: a consumer offers exactly what it
 * carries, never a set of its own.
 */
export interface MisplacedRow {
  name: string
  size: number
  /**
   * The full 64-hex content digest: the identity fact
   * {@link misplacedRemedy} names the finding by, so a remedy is chosen
   * from the row itself -- identity evidence, not a secret, under the same
   * TM-001-R7 display precedent as `Evidence.sha256`.
   */
  sha256: string
  sha256Short: string
  detectedType: string
  class: ArtifactClass
  reason: WrongRootReason
  remedies: Remedy[]
}

/**
 * Every finding the last scan left standing (`misplaced_list`), in walk
 * order; empty when no scan has run in this session. Read-only: listing
 * moves nothing. Invokes `misplaced_list` with no arguments.
 *
 * # Errors
 * Never rejects in the shell's own implementation -- the findings are read
 * from shell state, so a rejection here is a transport failure, not a
 * catalogue code.
 */
export async function misplacedList(): Promise<MisplacedRow[]> {
  return invoke('misplaced_list')
}

/**
 * What a remedy actually did, mirroring `dto.rs`'s `RemedyOutcomeTag`:
 * `quarantined` -- the file was moved into the product's quarantine
 * directory, outside any workspace (RCS-001-R10), and is gone from the
 * project; `copied-to-outbox` -- the file's bytes were copied into the
 * outbox as a new unattributed entry and the original was left exactly
 * where it was found (HAP-001-R28), with nothing published until that
 * entry is approved like any other; `ignored` -- the decision was recorded
 * against the file's name and digest, and the file was not touched.
 */
export type RemedyOutcome = 'quarantined' | 'copied-to-outbox' | 'ignored'

/**
 * The fixed token qualifying a remedy's outcome (`docs/spike-log.md` §
 * Slice 5d): which move ran, or why the original was kept. Never free text
 * and never a path.
 *
 * `renamed` is the handle-anchored rename on one volume; `renamed-unverified`
 * is that same rename after which the destination could not be re-opened and
 * digested, so the move happened and only its verification did not (it
 * carries `originalKept: false`, because a rename removes the original name
 * whatever the verification then found); `different-volume`,
 * `rename-unsupported` and `rename-failed` name the fallback the copy path
 * took, with the original removed after it; `unlink-failed`,
 * `original-already-gone`, `original-changed-during-the-move` and
 * `identity-check-unavailable` are the four cases where the copy stands and
 * the **original was kept** -- HAP-001-R19's residual, disclosed rather
 * than closed.
 *
 * Nine tokens, matching `quarantine_detail` in
 * `src-tauri/src/ipc/wrong_root.rs` arm for arm. The wire types this field
 * as a plain string, so a consumer must still have an answer for a token
 * this union does not name.
 */
export type RemedyDetail =
  | 'renamed'
  | 'renamed-unverified'
  | 'different-volume'
  | 'rename-unsupported'
  | 'rename-failed'
  | 'unlink-failed'
  | 'original-already-gone'
  | 'original-changed-during-the-move'
  | 'identity-check-unavailable'

/**
 * `misplaced_remedy`'s payload (`docs/spike-log.md` § Slice 5d): one shape
 * for all three remedies -- which one ran, what it did, and, for the two
 * that produce something, the logical name of what was produced beside its
 * digest. Mirrors `MisplacedRemedyDto` field-for-field.
 *
 * `name` is a quarantined file's sanitized name or the new outbox entry's
 * name, one component either way, and `null` for the ignore remedy, which
 * produces nothing.
 *
 * `originalKept` says whether **this remedy removed the original**, and
 * nothing beyond that. It is `false` for a quarantine that took the
 * original name away -- the handle-anchored `renamed`, `renamed-unverified`
 * (a rename removes the name whatever the verification then found), and
 * the `different-volume` / `rename-unsupported` / `rename-failed` copy
 * paths that unlink after copying -- and `true` for publish, for ignore,
 * and for each of {@link RemedyDetail}'s four residual details. `true` is
 * therefore **not** a report that the file is still in the project: three
 * of those four (`original-already-gone`,
 * `original-changed-during-the-move`, `identity-check-unavailable`) say
 * the opposite or say nothing at all, which is why the surface renders it
 * as "this remedy did not remove the original" rather than as "the
 * original is still in the project" (R1-004). The earlier wording here --
 * "whether the file is still in the project: `false` only for a quarantine
 * that completed the move" -- was the same claim the UI dropped, and it
 * contradicted both {@link RemedyDetail} above and `formatOriginalKept`
 * below it (R3-041).
 *
 * Every one of `name`, `sha256`, `originalKept` and `detail` is nullable
 * on the wire, so a consumer states the absence rather than assuming a
 * value. Never a device path (RCS-001-R14).
 */
export interface MisplacedRemedy {
  remedy: Remedy
  outcome: RemedyOutcome
  name: string | null
  sha256: string | null
  originalKept: boolean | null
  detail: RemedyDetail | null
}

/**
 * Run one of HAP-001-R32's three remedies over the finding named by `name`
 * and `sha256` -- the project-relative name and the full 64-hex digest the
 * last scan reported, taken from the row itself and never from anything
 * typed. Nothing happens without this explicit choice, and nothing outside
 * the row's own `remedies` is ever offered. Invokes `misplaced_remedy` with
 * exactly `{ name, sha256, remedy }`; never a path (RCS-001-R14).
 *
 * The digest binds the request exactly as `artifactApprove` binds a
 * candidate: a name the last scan never reported, or reported at other
 * bytes, is `misplaced-unknown` and nothing moves.
 *
 * # Errors
 * Rejects with `misplaced-unknown` (no scan reported that file at that
 * digest, or nothing sits at that name any more), `refused` (a fact bound
 * at scan time stopped holding: not a regular file, the name no longer
 * holds the opened file, the content changed), `quarantine-unavailable`
 * (the quarantine directory could not be used, resolves inside a
 * registered workspace root, could not be made owner-only, or the move
 * failed), `outbox-unavailable` (the publish remedy's copy-in could not
 * create its entry, or an entry of that name already holds other bytes),
 * `outbox-invalid` (the classification policy could not be loaded),
 * `outbox-linked` (the misplaced file's link count is greater than one, so
 * HAP-001-R20's hard-link refusal applies to it as it does to an outbox
 * entry -- reachable from **both** the quarantine and the publish remedy,
 * since each opens the file through `open_misplaced`, and additionally from
 * the publish remedy's own copy-in), `work-area-invalid` (the ignore ledger
 * could not be opened, read or written), `invalid-request` (a digest that is
 * not 64 hex characters, or a name that is not project-relative),
 * `run-active` (a supervised process is still running), or
 * `workspace-unavailable` if no workspace is active.
 *
 * Ten codes; `src-tauri/src/ipc/wrong_root.rs` reaches exactly these.
 */
export async function misplacedRemedy(
  name: string,
  sha256: string,
  remedy: Remedy,
): Promise<MisplacedRemedy> {
  return invoke('misplaced_remedy', { name, sha256, remedy })
}

// -- Slice 5e: catalog repair and recovery re-approval --

/**
 * Which repair rule would drop a Catalog line (`docs/spike-log.md` § Slice
 * 5e, D1), mirroring `dto.rs`'s `RepairRuleTag`: the two rules that delete
 * no registration, and nothing else.
 *
 * `duplicate-record` is the **later** of two `record` lines carrying one
 * publication identity -- the surviving line still registers it --
 * and `dangling-alias` is an `alias` no **earlier** `record` line carries,
 * which is a display name attached to nothing at the point the Catalog's
 * own reader meets it. The third rule, a `record` whose `catalogId`
 * disagrees with its identity, is **refused** and never applied
 * (HAP-001-R39): it is the only line registering that artifact, so it
 * arrives as a {@link RepairRefusal} instead.
 */
export type RepairRule = 'duplicate-record' | 'dangling-alias'

/**
 * Why a Catalog line refuses the whole repair (`docs/spike-log.md` § Slice
 * 5e), mirroring `dto.rs`'s `RepairRefusalTag`: **three** tokens, never
 * dropped, because the Catalog is synchronized, portable, untrusted
 * content (HAP-001-R40).
 *
 * - `catalog-id-mismatch`: a `record` whose `catalogId` is not
 *   `<assetRootId>/<publicationId>` (HAP-001-R39).
 * - `duplicate-across-asset-roots`: a `record` carrying a `publicationId`
 *   an earlier `record` already carries, under a **different**
 *   `assetRootId`. Two `catalogId`s, so two registered artifacts:
 *   dropping the later one deletes the second asset root's registration,
 *   which is the one thing a repair may never do. It is the token
 *   `duplicate-record` is *not*, and telling them apart is the whole
 *   point -- the rule drops the later of two lines that register **one**
 *   artifact; this refusal is the case where they register two.
 * - `unparsable`: a line this version cannot decode.
 *
 * This set is closed by `dto.rs` and not by this file. It drifted once
 * already -- the Rust added `duplicate-across-asset-roots`, this union did
 * not, and the surface's exhaustive switch fell through and rendered the
 * wire token raw (R1-001 / R1-002 / R1-011). `harness.test.ts` now reads
 * the enum out of `dto.rs` and fails on any difference, in either
 * direction, rather than switching this union over its own members.
 */
export type RefusalReason =
  | 'catalog-id-mismatch'
  | 'duplicate-across-asset-roots'
  | 'unparsable'

/**
 * One line {@link catalogRepair} would drop, named by its **one-based
 * number** and never by its content (HAP-001-R40, RCS-001-R14). Mirrors
 * `CatalogRepairDropDto` field-for-field.
 *
 * `publicationId` is the identity the line names when this version can
 * parse it as 64 hex characters, and `null` for a dangling alias whose
 * identity is not -- which is exactly why no record carries it.
 */
export interface RepairDrop {
  line: number
  rule: RepairRule
  publicationId: PublicationId | null
}

/** One line that refuses the repair, named the same way. Mirrors `CatalogRepairRefusalDto`. */
export interface RepairRefusal {
  line: number
  reason: RefusalReason
}

/**
 * `catalog_repair_preview`'s answer (`docs/spike-log.md` § Slice 5e): what
 * a repair would drop, what refuses it, and the digest the apply has to be
 * given back. Mirrors `CatalogRepairPreviewDto` field-for-field. Never a
 * device path, and never a line's content.
 *
 * `repairable` is `true` only when `refusals` is empty and `drops` is not:
 * the shell's own `RepairPlan::is_repairable`, carried as a fact rather
 * than recomputed by a consumer.
 */
export interface CatalogRepairPreview {
  sha256: string
  repairable: boolean
  drops: RepairDrop[]
  refusals: RepairRefusal[]
  keptRecords: number
}

/**
 * `catalog_repair`'s answer (`docs/spike-log.md` § Slice 5e): the binding
 * the request carried, the digest the catalog now has, and the two counts.
 * Mirrors `CatalogRepairedDto` field-for-field.
 *
 * Never a path: the pre-repair copy of `originalSha256`'s bytes lives in
 * the product work area, which is device configuration and never crosses
 * IPC (HAP-001-R5). Nothing lists or restores that copy either.
 */
export interface CatalogRepaired {
  originalSha256: string
  sha256: string
  droppedLines: number
  keptRecords: number
}

/**
 * One recovery entry (`docs/spike-log.md` § Slice 5e, HAP-001-R18): the
 * bytes a publication held when its outbox path stopped naming them, kept
 * in the product work area so that what was digested and approved
 * survives. Mirrors `RecoveryEntryDto` field-for-field. Never a path.
 *
 * `sha256` is the digest the entry is **filed under** -- a locator, not a
 * verified fact. `usable` is the probe's own answer: `false` when the entry
 * did not open as a regular file with a link count of one (in which case
 * `size` is `null` too), and `false` when the bytes at that name no longer
 * hash to it. {@link recoveryApprove} re-verifies from its own handle
 * whatever this said.
 *
 * `recoveredFrom` is the publication whose `outbox-escape` wrote the entry,
 * read from this device's journal and never inferred from the file name;
 * `recoveredAt` is when that step was journaled, in milliseconds since the
 * epoch.
 *
 * `displayName` is the name the re-publication would register and write
 * this under, and `class` the project policy's verdict on that name beside
 * the type and size read from the entry's own handle -- the two facts that
 * decide what lands on disk and whether the approval is allowed at all
 * (R1-004). `class` is `null` for an entry whose probe yielded nothing to
 * classify, the same fact `size: null` reports.
 *
 * **The listing is this device's, not the active project's** (R1-005). The
 * work area holding these entries is device configuration and its journal
 * is one per device, so `recoveryList` returns every entry the device
 * holds whatever workspace is active -- including entries an escape wrote
 * under another project. {@link recoveryApprove} then registers the bytes
 * into the project that is active **now**, under that project's identity
 * and asset root.
 */
export interface RecoveryEntry {
  sha256: string
  size: number | null
  recoveredFrom: PublicationId
  recoveredAt: number
  displayName: string
  class: ArtifactClass | null
  usable: boolean
}

/**
 * What a repair of the active project's Catalog would drop, and the digest
 * {@link catalogRepair} has to be given back (`docs/spike-log.md` § Slice
 * 5e; HAP-001-R23, R39, R40). Read-only: nothing is copied or rewritten,
 * and a live run does not freeze it -- seeing what is wrong is not a write.
 * Invokes `catalog_repair_preview` with no arguments.
 *
 * It **reads the whole Catalog**, under the publication surface lock, for
 * the length of the replay (R1-009): not against staleness but against
 * tearing, since a replay racing an append reads a half-written line and
 * reports the Catalog corrupt. Read-only is a statement about what it
 * writes, never about what it reads or what it holds while reading.
 *
 * # Errors
 * Rejects with `catalog-unavailable` (the project has no catalog, or it
 * could not be read) or `workspace-unavailable` if no workspace is active.
 * A **corrupt** catalog is not an error here: previewing it is the whole
 * point, and the refused lines come back in `refusals`.
 */
export async function catalogRepairPreview(): Promise<CatalogRepairPreview> {
  return invoke('catalog_repair_preview')
}

/**
 * Apply the repair to the bytes `sha256` names (`docs/spike-log.md` § Slice
 * 5e, D2): the preview's own digest, which the shell compares against the
 * catalog as it stands and refuses with `catalog-changed` when it has
 * moved. The catalog is copied into the product work area before a byte is
 * rewritten. Invokes `catalog_repair` with exactly `{ sha256 }`; never a
 * path.
 *
 * # Errors
 * Rejects with `run-active` (a supervised process is still running -- this
 * is a write to project content, so the shell refuses it like every other
 * one), `invalid-request` (a digest that is not 64 hex characters),
 * `work-area-invalid` (the work area failed its check, or the pre-repair
 * copy could not be written into it), `catalog-changed` (the file is no
 * longer those bytes: nothing was rewritten), `repair-refused` (a line may
 * not be dropped, nothing may be dropped, or the rules would not yield a
 * readable catalog), `catalog-unavailable` (the catalog could not be read
 * or written), or `workspace-unavailable` if no workspace is active.
 *
 * Seven codes; `src-tauri/src/ipc/catalog_repair.rs` reaches exactly these.
 */
export async function catalogRepair(sha256: string): Promise<CatalogRepaired> {
  return invoke('catalog_repair', { sha256 })
}

/**
 * Every recovery entry this device's journal knows about, earliest first
 * (`docs/spike-log.md` § Slice 5e, HAP-001-R18). Read-only: listing opens
 * each entry once to take its facts and holds **no handle** afterwards --
 * unlike {@link recoveryApprove}, which keeps one for the publication that
 * follows -- and a live run does not freeze it. Invokes `recovery_list`
 * with no arguments.
 *
 * It is not lock-free, and this used to read as though it were (R1-009):
 * `recovery_list_for` takes the publication surface lock for the length of
 * the journal replay, as every reader of that surface does, so a listing
 * is serialized against a publication in progress. "Holds nothing" was
 * about handles and was said as though it were about locks.
 *
 * An entry on disk that no `outbox-escape` step of this device's journal
 * names is not listed and is not approvable -- fail-closed rather than
 * attributing bytes nobody can account for.
 *
 * # Errors
 * Rejects with `work-area-invalid` (the work area failed its check, or the
 * publication journal could not be read), `outbox-invalid` (the
 * classification policy could not be loaded -- the listing carries each
 * entry's class, so it loads the policy the approval classifies against),
 * or `workspace-unavailable` if no workspace is active. The last one is
 * idle rather than a failure, and the panel says so separately (R1-007).
 */
export async function recoveryList(): Promise<RecoveryEntry[]> {
  return invoke('recovery_list')
}

/**
 * Approve one recovery entry for re-publication (`docs/spike-log.md` §
 * Slice 5e; HAP-001-R18, R22, D3): a fresh, explicit, per-artifact act that
 * no standing policy covers. `digest` is the digest the entry is filed
 * under, taken from the row itself and never from anything typed. Invokes
 * `recovery_approve` with exactly `{ digest }`; never a path.
 *
 * The shell opens the entry once without following a link, digests it from
 * that handle, and refuses with `integrity-mismatch` unless what it read is
 * the name the journal attests to.
 *
 * **One digest, not two** (slice 5e review, R1-006 / R3-069). This took a
 * second, `sha256`, described as "the digest the listing showed" -- but a
 * {@link RecoveryEntry} carries exactly one digest and it *is* the name, so
 * this function's only caller passed `entry.sha256` as both and the shell's
 * two comparisons were one comparison made twice. The shape was
 * {@link artifactApprove}'s, where `name` and `sha256` are independent
 * facts about an outbox entry; a recovery entry's name is its digest, so it
 * has no second fact to bind to. Staleness is caught where it is real: the
 * surface re-binds its open block to the entry's digest on every refetch,
 * and the shell re-derives from its own handle regardless.
 *
 * The approval that comes back goes to the **existing**
 * {@link artifactPublish}, unchanged. Its `publicationId` is derived from
 * the bytes that were read, which is not always the escaped publication's;
 * that publication travels as {@link ArtifactApproval.recoveredFrom}, a
 * location fact and never provenance.
 *
 * # Errors
 * Rejects with `run-active` (a supervised process is still running),
 * `invalid-request` (a digest that is not 64 hex characters, or a derived
 * approval id already recorded), `work-area-invalid` (the work area failed
 * its check, or the journal could not be read or written),
 * `recovery-unknown` (no `outbox-escape` of this device's journal wrote
 * that entry, or it could not be opened at all), `outbox-invalid` (the
 * classification policy could not be loaded -- reached from
 * `policy_store.load`, which `recovery_approve_for`'s own `# Errors` list
 * omits), `destination-invalid` (the project declares no asset root),
 * `refused` (the entry is not a regular file, its link count is greater
 * than one, or its class is not `generated-heavy`), `integrity-mismatch`
 * (the bytes read are not the digest the entry is filed under), or
 * `workspace-unavailable` if no workspace is active.
 *
 * Nine codes; `src-tauri/src/ipc/recovery.rs` reaches exactly these.
 */
export async function recoveryApprove(digest: string): Promise<ArtifactApproval> {
  return invoke('recovery_approve', { digest })
}
