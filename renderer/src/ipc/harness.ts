/**
 * The renderer's only import of `@tauri-apps/api/core`. Every other
 * renderer module reaches the demo-harness IPC surface through this
 * module's typed wrappers, never `invoke` directly (`docs/spike-log.md`
 * § IPC contract).
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

/** The closed set of demo harness kinds a caller may request. */
export type HarnessKind = 'demo-lines' | 'demo-ignores-sigterm'

/** A process identifier crossing IPC: a bare number on the wire. */
export type ProcessId = number

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

/** A failed IPC command's error payload: a catalogue code plus message. */
export interface ShellError {
  code: ShellErrorCode
  message: string
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

/** A `harness_spawn` request, before its `onFrame` channel is attached. */
export interface HarnessSpawnRequest {
  kind: HarnessKind
  rateHz: number
  lines: number
}

/**
 * Spawn a demo harness and stream its captured output to `onFrame`.
 *
 * Creates a fresh `Channel<HarnessFrame>`, wires its `onmessage` to
 * `onFrame`, and invokes `harness_spawn` with exactly `{ kind, rateHz,
 * lines, onFrame }` -- no program path or argument vector, matching the
 * IPC contract in `docs/spike-log.md`.
 */
export async function harnessSpawn(
  request: HarnessSpawnRequest,
  onFrame: (frame: HarnessFrame) => void,
): Promise<ProcessId> {
  const channel = new Channel<HarnessFrame>()
  channel.onmessage = onFrame

  return invoke('harness_spawn', {
    kind: request.kind,
    rateHz: request.rateHz,
    lines: request.lines,
    onFrame: channel,
  })
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
