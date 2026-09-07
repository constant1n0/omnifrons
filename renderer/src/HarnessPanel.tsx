import { useCallback, useEffect, useRef, useState } from 'react'

import { stripControlCharacters } from './controlCharacters'
import {
  approvalsList,
  harnessObserve,
  harnessSpawn,
  harnessStop,
  isShellError,
  type Approval,
  type ApprovalId,
  type HarnessFrame,
  type HarnessKind,
  type ProcessId,
  type ProcessTerminalState,
  type ShellError,
  type ShellErrorCode,
} from './ipc/harness'
import { PlainTextLine } from './PlainTextLine'

/**
 * The tag alone of {@link HarnessKind} -- what the Kind select offers.
 * Excludes `'adapter'` (spike slice 3): the adapter launch surface is
 * `AgentPanel`, a distinct component -- this panel keeps its own slice
 * 1/2 kind set unchanged.
 */
type HarnessKindTag = Exclude<HarnessKind['type'], 'adapter'>

/** The three harness kinds the Kind select offers. */
const HARNESS_KIND_TAGS: HarnessKindTag[] = ['demo-lines', 'demo-ignores-sigterm', 'approved']

/**
 * The four `LaunchGate` denial codes `harness_spawn`'s `approved` kind can
 * reject with -- rendered in the banner with the shared public wording
 * "untrusted", since every one of them means the same thing to a caller:
 * this executable is not cleared to launch right now
 * (`docs/threat-model.md` HAR-3/HAR-4).
 */
const DENIAL_CODES: ReadonlySet<ShellErrorCode> = new Set([
  'unapproved',
  'changed-since-approval',
  'shadowed-path',
  'revoked',
])

/**
 * Bounds mirroring `omnifrons_app::harness_catalog`'s validated range.
 * These are advisory only here, a client-side hint to disable Start
 * early and explain why -- the Rust catalog is the sole authority on
 * what it accepts, and validates again regardless of what this panel
 * lets through (R3-006).
 */
const MIN_RATE_HZ = 1
const MAX_RATE_HZ = 1000
const MIN_LINES = 1
const MAX_LINES = 100_000

const DEFAULT_RATE_HZ = 10
const DEFAULT_LINES = 10

/**
 * Parses `raw` as a whole number in `[min, max]`, or `null` if it isn't
 * one: empty (after trimming), not a clean non-negative integer string
 * (anything `Number.parseInt` would only partially consume, such as
 * `"1.5"` or `"10abc"`, per R3-017), or outside the bounds.
 */
function parseCount(raw: string, min: number, max: number): number | null {
  const trimmed = raw.trim()
  if (!/^\d+$/.test(trimmed)) return null
  const parsed = Number.parseInt(trimmed, 10)
  if (parsed < min || parsed > max) return null
  return parsed
}

/**
 * Deadline this panel asks `harness_stop` to honor. Not exposed as its
 * own input -- the deliverable's UI surface only calls for kind, rate,
 * and lines controls plus Start/Stop -- so a fixed spike-appropriate
 * default is used instead.
 */
const STOP_DEADLINE_MS = 2000

/** The single key under which every spawned process id is remembered. */
const SESSION_STORAGE_KEY = 'omnifrons.harness.processIds'

function loadPersistedIds(): ProcessId[] {
  try {
    const raw = window.sessionStorage.getItem(SESSION_STORAGE_KEY)
    if (!raw) return []
    const parsed: unknown = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter((value): value is number => typeof value === 'number')
  } catch {
    // sessionStorage unavailable, disabled, or holding malformed data --
    // treat it as if nothing were remembered rather than failing to mount.
    return []
  }
}

function persistIds(ids: ProcessId[]): void {
  try {
    window.sessionStorage.setItem(SESSION_STORAGE_KEY, JSON.stringify(ids))
  } catch {
    // Best-effort only: persistence failing must not break the panel.
  }
}

/**
 * Forgets one remembered process id: a terminal status observed for it,
 * a stale (`unknown-process`) id, or a completed `harness_stop` all mean
 * there is nothing left to reconnect to (R3-003).
 */
function removePersistedId(id: ProcessId): void {
  persistIds(loadPersistedIds().filter((existing) => existing !== id))
}

/**
 * Formats a terminal state as the exact badge token the panel shows.
 * Per target-architecture.md invariant 8, an uncertain or partial state
 * (`orphan-risk/uncertain`) is never rendered as, or alongside, "done" --
 * every token here is one of the closed set the wire contract defines,
 * transcribed verbatim.
 */
function formatTerminalToken(state: ProcessTerminalState): string {
  if (state.state === 'exited') {
    return state.code === null ? 'exited (code unreported)' : `exited (code ${state.code})`
  }
  return state.state
}

/**
 * This panel never spawns `kind: "adapter"` (that surface is `AgentPanel`),
 * so a live `event` frame never actually reaches here -- the `event`
 * branch exists only so this function stays total over `HarnessFrame`'s
 * full, slice-3-widened union.
 */
function frameLineText(frame: HarnessFrame): string {
  if (frame.stream === 'state') {
    return `${frame.stream}: ${formatTerminalToken(frame.body)}`
  }
  if (frame.stream === 'event') {
    return `${frame.stream}: unexpected event frame`
  }
  return `${frame.stream}: ${frame.body.text}`
}

interface LogEntry {
  key: string
  droppedBefore: number
  text: string
}

/**
 * Caps the rendered log to the most recent frames -- an unbounded DOM
 * list would eventually make the panel itself unusable on a long-running
 * or high-rate harness (R3-007). 2000 is a spike-appropriate default,
 * comfortably above `MAX_RATE_HZ` seconds of output at a glance.
 */
const MAX_LOG_ENTRIES = 2000

interface LogState {
  entries: LogEntry[]
  /** Cumulative count of entries ever trimmed from the front of the log. */
  trimmedCount: number
}

const EMPTY_LOG: LogState = { entries: [], trimmedCount: 0 }

/** Appends `entry`, trimming from the front once the cap is exceeded. */
function appendLogEntry(previous: LogState, entry: LogEntry): LogState {
  const entries = [...previous.entries, entry]
  if (entries.length <= MAX_LOG_ENTRIES) {
    return { entries, trimmedCount: previous.trimmedCount }
  }
  const overflow = entries.length - MAX_LOG_ENTRIES
  return { entries: entries.slice(overflow), trimmedCount: previous.trimmedCount + overflow }
}

/**
 * The banner's error state: a typed `ShellError`, or the sentinel
 * `'unexpected'` for any rejection that isn't `ShellError`-shaped. A
 * non-`ShellError` rejection's `message` is never rendered -- unlike a
 * `ShellError`, it comes from no fixed catalogue and can carry a raw OS
 * error string with a real filesystem path (see the IPC contract in
 * `docs/spike-log.md`) -- so the banner shows only the fixed string
 * "unexpected error" instead of surfacing that text (R3-012).
 */
type PanelError = ShellError | 'unexpected'

/**
 * Demo-harness control panel: spawn one of the two synthetic harness
 * kinds, watch its streamed output, and stop it.
 *
 * Every piece of harness output is untrusted content with no declared
 * richer class, so it renders through {@link PlainTextLine} -- plain
 * text only, per RCS-001-R1 -- never as markup.
 */
export function HarnessPanel() {
  const [kindTag, setKindTag] = useState<HarnessKindTag>('demo-lines')
  // Held as raw strings, not numbers: an in-progress edit (empty, a bare
  // "-", a partial number) must stay representable in the input while
  // still being validated on every keystroke (R3-006).
  const [rateHzInput, setRateHzInput] = useState(String(DEFAULT_RATE_HZ))
  const [linesInput, setLinesInput] = useState(String(DEFAULT_LINES))
  const rateHz = parseCount(rateHzInput, MIN_RATE_HZ, MAX_RATE_HZ)
  const lines = parseCount(linesInput, MIN_LINES, MAX_LINES)
  /** Active approvals offered by the `approved` kind's own select. */
  const [approvals, setApprovals] = useState<Approval[]>([])
  const [approvedId, setApprovedId] = useState<ApprovalId | null>(null)
  const [activeId, setActiveId] = useState<ProcessId | null>(null)
  /** True from `handleStart`'s call until its spawn settles, one way or the other. */
  const [isSpawning, setIsSpawning] = useState(false)
  const [badge, setBadge] = useState('idle')
  const [log, setLog] = useState<LogState>(EMPTY_LOG)
  const [error, setError] = useState<PanelError | null>(null)
  const [reconnectedIds, setReconnectedIds] = useState<ProcessId[]>([])

  /**
   * Tracks whether this component instance is still mounted. `handleFrame`
   * and the mount-time observe promise handlers below check it before
   * doing any work, so a frame delivered -- or an observe promise settled
   * -- after unmount no-ops entirely: no state update, no sessionStorage
   * write (R3-001).
   */
  const mountedRef = useRef(true)
  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  /**
   * The sequence number of the most recently *started* `approvals_list`
   * fetch below. Compared against the request id each fetch captured at
   * its own start, so a response is only ever applied if it is still the
   * latest one in flight -- otherwise it is a stale response from an
   * earlier fetch (e.g. leaving `approved` and re-selecting it before the
   * first fetch settled) and is silently dropped (R3-008).
   */
  const latestApprovalsRequestRef = useRef(0)

  /**
   * Fetches the active-approvals list lazily, only once the `approved`
   * kind is actually selected -- every other test/usage of this panel
   * never triggers an `approvals_list` call at all, keeping the demo-kind
   * paths' own IPC surface unchanged from slice 1.
   */
  useEffect(() => {
    if (kindTag !== 'approved') return
    const requestId = (latestApprovalsRequestRef.current += 1)
    approvalsList()
      .then((records) => {
        if (!mountedRef.current) return
        if (requestId !== latestApprovalsRequestRef.current) return
        setApprovals(records)
      })
      .catch((listError: unknown) => {
        if (!mountedRef.current) return
        if (requestId !== latestApprovalsRequestRef.current) return
        setError(isShellError(listError) ? listError : 'unexpected')
      })
  }, [kindTag])

  /**
   * The id of the current generation's run once known, or `null` while
   * the spawn is still in flight. Set when `harnessSpawn` resolves and
   * cleared only when the next Start bumps the generation -- deliberately
   * *not* when the run ends, so a same-channel frame carrying a foreign
   * id is still dropped after the run's terminal `state` frame or a
   * successful stop, while a trailing frame carrying the run's own id is
   * still applied (R1-011/R3-011). A ref rather than state: `handleFrame`
   * is a stable `useCallback(..., [])` and must never close over a stale
   * value. Whether the run is *running* is `activeId` state plus
   * `runEndedRef`, not this.
   */
  const runIdRef = useRef<ProcessId | null>(null)

  /**
   * Per-spawn generation token (R1-001/R3-001). Incremented on every Start
   * before `harnessSpawn` is called, and captured in the closure of that
   * spawn's own Channel `onmessage` -- so a frame from an earlier run's
   * channel is rejected by its generation alone, whatever its timing and
   * whatever id it carries, and never by relying on the new run's id
   * already being known.
   */
  const spawnGenerationRef = useRef(0)

  /**
   * True once the current generation's run has ended: its terminal `state`
   * frame was applied, or a stop succeeded. A terminal frame can arrive
   * before `harnessSpawn` itself resolves (an instantly exiting process),
   * so the resolve handler consults this and never marks such a run
   * active -- nor remembers its id -- after the fact (R3-002).
   */
  const runEndedRef = useRef(true)

  const handleFrame = useCallback((generation: number, frame: HarnessFrame) => {
    if (!mountedRef.current) return
    // Stale-channel guard (R1-001/R3-001): every `harnessSpawn` wires a
    // fresh Channel to this same handler, so a trailing frame from an
    // earlier run's channel can still arrive after a new run has started
    // -- even before the new run's id is known. Only the current
    // generation's frames are applied; every other channel's are dropped
    // outright -- never logged, never allowed to move the badge.
    if (generation !== spawnGenerationRef.current) return
    // Belt and braces once the id is known: a frame on the current channel
    // carrying another run's id is dropped too, for the rest of this
    // generation -- after the run has ended as much as while it runs.
    // While the spawn is still in flight the generation alone decides, so
    // a frame that beats the spawn promise is applied, not dropped -- a
    // terminal frame arriving that early must still end the run (R3-002).
    if (runIdRef.current !== null && frame.body.id !== runIdRef.current) return
    setLog((previous) =>
      appendLogEntry(previous, {
        key: `${frame.body.id}-${frame.body.seq}`,
        droppedBefore: frame.body.droppedBefore,
        text: frameLineText(frame),
      }),
    )
    if (frame.stream === 'state') {
      // Every `state` frame is terminal by contract (`HarnessStateFrameBody`
      // extends `ProcessTerminalState`), so the run is over: free the
      // single active slot here, not only from the Stop path (R3-002), and
      // forget the id -- nothing is left to reconnect to (R3-003).
      setBadge(formatTerminalToken(frame.body))
      runEndedRef.current = true
      setActiveId(null)
      removePersistedId(frame.body.id)
    }
  }, [])

  useEffect(() => {
    const rememberedIds = loadPersistedIds()
    for (const id of rememberedIds) {
      harnessObserve(id)
        .then((status) => {
          if (!mountedRef.current) return
          if (status.status === 'running') {
            setReconnectedIds((previous) =>
              previous.includes(id) ? previous : [...previous, id],
            )
            return
          }
          // A terminal status observed for a remembered id: nothing left
          // to reconnect to -- forget it (R3-003).
          removePersistedId(id)
        })
        .catch((observeError: unknown) => {
          if (!mountedRef.current) return
          if (!isShellError(observeError)) return
          // `unknown-process` means the id is stale (e.g. a dev-mode
          // restart lost the supervisor's record of it) -- forget it
          // silently, it is not an error worth surfacing (R3-002). Any
          // other code is a real error: show it once, and still forget
          // the id, since this id cannot be reconnected to either way.
          if (observeError.code !== 'unknown-process') {
            setError(observeError)
          }
          removePersistedId(id)
        })
    }
  }, [])

  async function handleStart() {
    // Belt-and-suspenders: the Start button is already disabled while the
    // active kind's own required input is missing, but never spawn on a
    // value this panel itself considers out of range or unselected.
    let requestedKind: HarnessKind
    if (kindTag === 'approved') {
      if (approvedId === null) return
      requestedKind = { type: 'approved', approvalId: approvedId }
    } else {
      if (rateHz === null || lines === null) return
      requestedKind = { type: kindTag, rateHz, lines }
    }

    const generation = (spawnGenerationRef.current += 1)
    runEndedRef.current = false
    runIdRef.current = null
    setError(null)
    setLog(EMPTY_LOG)
    setReconnectedIds([])
    setIsSpawning(true)
    try {
      const id = await harnessSpawn(requestedKind, (frame) => handleFrame(generation, frame))
      if (generation !== spawnGenerationRef.current) return
      // The id is known from here on for the rest of this generation,
      // whether or not the run is still going.
      runIdRef.current = id
      // The run's terminal `state` frame may already have arrived and
      // ended it before the id was known: leave it ended, never mark it
      // active after the fact, and never remember an id there is nothing
      // left to reconnect to (R3-002/R3-003).
      if (runEndedRef.current) return
      setActiveId(id)
      setBadge('running')
      persistIds([...loadPersistedIds(), id])
    } catch (spawnError: unknown) {
      setError(isShellError(spawnError) ? spawnError : 'unexpected')
    } finally {
      setIsSpawning(false)
    }
  }

  async function handleStop() {
    if (activeId === null) return
    setError(null)
    try {
      const state = await harnessStop(activeId, STOP_DEADLINE_MS)
      setBadge(formatTerminalToken(state))
      // A stopped process has nothing left to reconnect to (R3-003), and
      // slice 1 allows only one active harness per panel (R3-004) -- once
      // stopped, the slot is free again.
      removePersistedId(activeId)
      runEndedRef.current = true
      setActiveId(null)
    } catch (stopError: unknown) {
      // A failed stop leaves the run exactly as it was -- active id kept,
      // Stop still enabled -- rather than pretending the process ended
      // (R3-006): the supervisor never confirmed a terminal state.
      setError(isShellError(stopError) ? stopError : 'unexpected')
    }
  }

  /**
   * Leaving the `approved` kind clears its own error banner and forgets
   * the previously selected approval, rather than letting either linger
   * once they no longer apply to the now-selected kind -- switching back
   * to `approved` later always requires re-selecting an approval
   * (R3-007/R3-011).
   */
  function handleKindChange(nextKind: HarnessKindTag) {
    if (kindTag === 'approved' && nextKind !== 'approved') {
      setError(null)
      setApprovedId(null)
    }
    setKindTag(nextKind)
  }

  const activeApprovals = approvals.filter((approval) => approval.status === 'active')
  const missingRequiredInput =
    kindTag === 'approved' ? approvedId === null : rateHz === null || lines === null
  const startDisabled = isSpawning || activeId !== null || missingRequiredInput

  return (
    <section aria-label="Demo harness">
      <h2>Demo harness</h2>

      {error && (
        <div role="alert" data-testid="harness-error-banner">
          {error === 'unexpected' ? (
            <strong>unexpected error</strong>
          ) : (
            <>
              {DENIAL_CODES.has(error.code) && (
                <>
                  <strong>untrusted</strong>{' '}
                </>
              )}
              <strong>
                <PlainTextLine text={error.code} />
              </strong>
              : <PlainTextLine text={error.message} />
              {error.code === 'changed-since-approval' && error.detail && (
                <span>
                  {' '}
                  (recorded <PlainTextLine text={error.detail.recordedSha256Short} />, observed{' '}
                  <PlainTextLine text={error.detail.observedSha256Short} />)
                </span>
              )}
            </>
          )}
        </div>
      )}

      <div>
        <label htmlFor="harness-kind">Kind</label>
        <select
          id="harness-kind"
          value={kindTag}
          onChange={(event) => handleKindChange(event.target.value as HarnessKindTag)}
        >
          {HARNESS_KIND_TAGS.map((value) => (
            <option key={value} value={value}>
              {value}
            </option>
          ))}
        </select>

        {kindTag === 'approved' ? (
          <>
            <label htmlFor="harness-approval">Approval</label>
            <select
              id="harness-approval"
              value={approvedId === null ? '' : String(approvedId)}
              onChange={(event) =>
                setApprovedId(event.target.value === '' ? null : Number(event.target.value))
              }
            >
              <option value="">Select an approval</option>
              {activeApprovals.map((approval) => (
                <option key={approval.approvalId} value={approval.approvalId}>
                  {stripControlCharacters(approval.evidence.canonicalPath)}
                </option>
              ))}
            </select>
          </>
        ) : (
          <>
            <label htmlFor="harness-rate-hz">Rate (Hz)</label>
            <input
              id="harness-rate-hz"
              type="number"
              min={MIN_RATE_HZ}
              max={MAX_RATE_HZ}
              value={rateHzInput}
              onChange={(event) => setRateHzInput(event.target.value)}
              aria-invalid={rateHz === null}
            />
            {rateHz === null && (
              <p data-testid="rate-hz-validation">
                {`Rate (Hz) must be a whole number from ${MIN_RATE_HZ} to ${MAX_RATE_HZ}.`}
              </p>
            )}

            <label htmlFor="harness-lines">Lines</label>
            <input
              id="harness-lines"
              type="number"
              min={MIN_LINES}
              max={MAX_LINES}
              value={linesInput}
              onChange={(event) => setLinesInput(event.target.value)}
              aria-invalid={lines === null}
            />
            {lines === null && (
              <p data-testid="lines-validation">
                {`Lines must be a whole number from ${MIN_LINES} to ${MAX_LINES}.`}
              </p>
            )}
          </>
        )}

        <button type="button" onClick={handleStart} disabled={startDisabled}>
          Start
        </button>
        <button type="button" onClick={handleStop} disabled={activeId === null}>
          Stop
        </button>
      </div>

      <p>
        State: <strong>{badge}</strong>
      </p>

      {reconnectedIds.map((id) => (
        <p key={id}>{`Process ${id}: stream reconnected; earlier output not replayed`}</p>
      ))}

      <ul aria-label="Output log">
        {log.trimmedCount > 0 && (
          <li>
            <mark>
              {`showing the last ${MAX_LOG_ENTRIES} lines; ${log.trimmedCount} earlier lines are not shown`}
            </mark>
          </li>
        )}
        {log.entries.map((entry) => (
          <li key={entry.key}>
            {entry.droppedBefore > 0 && (
              <mark>{entry.droppedBefore} frames dropped before this line</mark>
            )}
            <PlainTextLine text={entry.text} />
          </li>
        ))}
      </ul>
    </section>
  )
}

export default HarnessPanel
