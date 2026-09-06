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
export type HarnessKind =
  | { type: 'demo-lines'; rateHz: number; lines: number }
  | { type: 'demo-ignores-sigterm'; rateHz: number; lines: number }
  | { type: 'approved'; approvalId: ApprovalId }

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
  text: string
}

interface HarnessStateFrameBody extends ProcessTerminalState {
  id: ProcessId
  seq: number
  droppedBefore: number
}

/** One streamed frame delivered over `harness_spawn`'s `onFrame` channel. */
export type HarnessFrame =
  | { stream: 'stdout'; body: HarnessTextFrameBody }
  | { stream: 'stderr'; body: HarnessTextFrameBody }
  | { stream: 'state'; body: HarnessStateFrameBody }

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

/**
 * Structured detail for a {@link ShellError}, carrying values a fixed
 * catalogue `message` must not -- currently only populated for
 * `changed-since-approval`.
 */
export interface ShellErrorDetail {
  recordedSha256Short: string
  observedSha256Short: string
}

/**
 * A failed IPC command's error payload. `detail` is present only for the
 * handful of codes that carry structured, non-path data alongside the
 * message (see {@link ShellErrorDetail}).
 */
export interface ShellError {
  code: ShellErrorCode
  message: string
  detail?: ShellErrorDetail
}

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
