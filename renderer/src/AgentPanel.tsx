import { useCallback, useEffect, useRef, useState } from 'react'

import { stripControlCharacters } from './controlCharacters'
import {
  adaptersList,
  approvalsList,
  harnessSpawn,
  harnessStop,
  isShellError,
  workspaceCurrent,
  workspacePick,
  type AdapterDescriptor,
  type AgentEvent,
  type AgentPhaseTag,
  type Approval,
  type ApprovalId,
  type HarnessFrame,
  type Observation,
  type ProcessId,
  type ProcessTerminalState,
  type ShellError,
  type ShellErrorCode,
  type TerminalActionKind,
  type TerminalDropCounts,
  type Workspace,
} from './ipc/harness'
import { PlainTextLine } from './PlainTextLine'

/**
 * The same four `LaunchGate` denial codes {@link HarnessPanel} renders with
 * the shared public wording "untrusted" -- an adapter launch goes through
 * the identical gate decision for its own `approvalId`
 * (`docs/threat-model.md` HAR-3/HAR-4). Kept as its own copy here rather
 * than an import: `HarnessPanel` does not export this set, and each panel
 * owning its own copy of this small, fixed list matches how `PanelError`
 * is already duplicated across `HarnessPanel`/`ApprovalSurface`.
 */
const DENIAL_CODES: ReadonlySet<ShellErrorCode> = new Set([
  'unapproved',
  'changed-since-approval',
  'shadowed-path',
  'revoked',
])

/**
 * Advisory-only 16 KiB prompt cap, mirroring
 * `omnifrons_domain::adapter::AgentPrompt::MAX_BYTES`. A client-side hint
 * to disable Start early and explain why -- `AgentPrompt::new` on the Rust
 * side is the sole authority on what it accepts, and validates again
 * regardless of what this panel lets through (mirrors `HarnessPanel`'s own
 * `MIN_RATE_HZ`/`MAX_RATE_HZ` advisory-bounds comment, R3-006).
 */
const PROMPT_MAX_BYTES = 16 * 1024

/** UTF-8 byte length of `text` -- what the 16 KiB cap is actually measured in, not `text.length`. */
function promptByteLength(text: string): number {
  return new TextEncoder().encode(text).length
}

/** Deadline this panel asks `harness_stop` to honor, mirroring `HarnessPanel`'s own fixed default. */
const STOP_DEADLINE_MS = 2000

/**
 * Formats a terminal state as the exact badge token the panel shows,
 * mirroring `HarnessPanel`'s own `formatTerminalToken` verbatim (see that
 * function's doc comment for why every token here is one of the closed set
 * the wire contract defines, transcribed verbatim, per
 * `docs/target-architecture.md` invariant 8).
 */
function formatTerminalToken(state: ProcessTerminalState): string {
  if (state.state === 'exited') {
    return state.code === null ? 'exited (code unreported)' : `exited (code ${state.code})`
  }
  return state.state
}

function formatObservation(observation: Observation): string {
  return `${observation.key}: ${observation.value}`
}

/** A `state` event as one compact line: phase, subtype (if any), then each observation as `key: value`. */
function formatStateEventLine(payload: {
  phase: AgentPhaseTag
  subtype: string | null
  observations: Observation[]
}): string {
  const segments: string[] = [payload.phase]
  if (payload.subtype !== null) segments.push(payload.subtype)
  segments.push(...payload.observations.map(formatObservation))
  return segments.join(', ')
}

/**
 * The fixed disclosure shown while the selected adapter's `transportClass`
 * is `pty` (`docs/spike-log.md` § Slice 4): a pseudo-terminal launch is an
 * explicit degraded fallback (`docs/target-architecture.md` invariant 6),
 * rendered here as plain text with every layout control already dropped by
 * core's normalizer -- the strictest RCS-001 mode, not a terminal pane.
 * Fixed copy, never wire text.
 */
const PTY_TRANSPORT_DISCLOSURE =
  'transport: pty — degraded fallback; plain text, layout controls dropped'

/** The visible label on every `terminal-text` transcript entry. */
const TERMINAL_TEXT_LABEL = 'terminal output, plain text'

/**
 * Splits one `terminal-text` payload into the lines it carries, *before*
 * any of it reaches {@link PlainTextLine}: core keeps a `\n` per framed
 * line end, and `PlainTextLine` strips every C0 control including `\n`, so
 * a newline passed through it would silently join two lines into one. A
 * trailing `\n` ends the last line rather than opening an empty one; a
 * text with no `\n` (a continued chunk) is one line; an interior empty
 * line is kept, since it is one; an empty text is no line at all (slice 4
 * review, R3-006).
 */
function splitTerminalLines(text: string): string[] {
  if (text === '') return []
  const lines = text.split('\n')
  if (lines[lines.length - 1] === '') lines.pop()
  return lines
}

/**
 * The label of a `terminal-action` line: fixed copy chosen per closed
 * action token, never the wire token echoed -- the switch is exhaustive, so
 * a new action kind cannot render unlabelled.
 */
function formatTerminalActionLabel(action: TerminalActionKind): string {
  switch (action) {
    case 'title':
      return 'title'
    case 'notification':
      return 'notification'
  }
}

/**
 * The degraded marker for a `terminal-drops` event: the seven per-family
 * counts core's normalizer dropped since the previous marker, in RCS-001's
 * table order, zero counts included -- every family is always accounted
 * for, so a reader never has to guess whether an absent family meant zero
 * or unreported.
 */
function formatTerminalDrops(counts: TerminalDropCounts): string {
  const families = [
    `layout ${counts.layout}`,
    `hyperlink ${counts.hyperlink}`,
    `clipboard ${counts.clipboard}`,
    `file transfer ${counts.fileTransfer}`,
    `string ${counts.string}`,
    `unknown ${counts.unknown}`,
    `malformed ${counts.malformed}`,
  ]
  return `dropped terminal controls: ${families.join(', ')}`
}

type TranscriptItem =
  | { type: 'prompt'; key: string; text: string }
  | { type: 'agent-event'; key: string; droppedBefore: number; event: AgentEvent }
  /** The run's own terminal `state` frame, recorded in the transcript like `HarnessPanel` logs it. */
  | { type: 'terminal-state'; key: string; droppedBefore: number; token: string }

/**
 * Caps the rendered transcript to the most recent entries, mirroring
 * `HarnessPanel`'s own `MAX_LOG_ENTRIES`/`appendLogEntry` -- an unbounded
 * DOM list would eventually make this panel itself unusable on a
 * long-running agent (R3-007 analogue).
 */
const MAX_TRANSCRIPT_ENTRIES = 2000

interface TranscriptState {
  entries: TranscriptItem[]
  /** Cumulative count of entries ever trimmed from the front of the transcript. */
  trimmedCount: number
}

const EMPTY_TRANSCRIPT: TranscriptState = { entries: [], trimmedCount: 0 }

/** Appends `entry`, trimming from the front once the cap is exceeded. */
function appendTranscriptEntry(previous: TranscriptState, entry: TranscriptItem): TranscriptState {
  const entries = [...previous.entries, entry]
  if (entries.length <= MAX_TRANSCRIPT_ENTRIES) {
    return { entries, trimmedCount: previous.trimmedCount }
  }
  const overflow = entries.length - MAX_TRANSCRIPT_ENTRIES
  return { entries: entries.slice(overflow), trimmedCount: previous.trimmedCount + overflow }
}

/**
 * The banner's error state: a typed `ShellError`, or the sentinel
 * `'unexpected'` for any rejection that isn't `ShellError`-shaped --
 * mirrors `HarnessPanel`/`ApprovalSurface`'s own `PanelError` (R3-012).
 */
type PanelError = ShellError | 'unexpected'

/**
 * The degraded marker every transcript entry carrying a nonzero
 * `droppedBefore` is prefixed with -- used within this panel by adapter
 * events and the run's own terminal `state` frame alike (R3-003), so a
 * drop right before the process's exit is surfaced the same way as one
 * mid-stream. Local to `AgentPanel`: `HarnessPanel` keeps its own inline
 * marker with slice 1's wording ("N frames dropped before this line").
 */
function DroppedFramesMarker({ droppedBefore }: { droppedBefore: number }) {
  if (droppedBefore <= 0) return null
  return (
    <mark>
      {`${droppedBefore} frames dropped before this entry — degraded, restart to replay`}
    </mark>
  )
}

/**
 * Renders one transcript entry: the sent prompt as an untrusted "you"
 * entry, the run's terminal state as a `state: <token>` line, or one
 * adapter event by its `kind` -- `message`/`state`/`diagnostic`/`unknown`
 * as plain text through {@link PlainTextLine} (RCS-001-R1: no markup sink,
 * ever), and `tool-call` inside a labelled "proposal, not executed" block
 * containing zero interactive elements -- nothing this slice ships ever
 * executes a proposed tool call (`docs/spike-log.md` § Slice 3, VP-S18
 * notes).
 *
 * The three `terminal-*` kinds a `pty-cli` launch emits (`docs/spike-log.md`
 * § Slice 4) render as text too, and only text: `terminal-text` as one
 * labelled entry with a {@link PlainTextLine} per newline-split line,
 * `terminal-action` as a `title:`/`notification:` line that touches nothing
 * outside the transcript (never `document.title`, never a Notification
 * API, never focus), and `terminal-drops` as a `<mark>` marker line of the
 * seven drop counts. No terminal pane exists in this slice: plain text is
 * the strictest RCS-001 mode, and every layout control was dropped by core
 * before any of this reached the wire.
 */
function TranscriptEntryView({ item }: { item: TranscriptItem }) {
  if (item.type === 'prompt') {
    return (
      <li key={item.key}>
        you: <PlainTextLine text={item.text} />
      </li>
    )
  }

  if (item.type === 'terminal-state') {
    return (
      <li key={item.key}>
        <DroppedFramesMarker droppedBefore={item.droppedBefore} />
        <PlainTextLine text={`state: ${item.token}`} />
      </li>
    )
  }

  const { event, droppedBefore } = item
  return (
    <li key={item.key}>
      <DroppedFramesMarker droppedBefore={droppedBefore} />
      {event.kind === 'message' && <PlainTextLine text={event.payload.text} />}
      {event.kind === 'state' && <PlainTextLine text={formatStateEventLine(event.payload)} />}
      {event.kind === 'diagnostic' && (
        <PlainTextLine text={`diagnostic: ${event.payload.text}`} />
      )}
      {event.kind === 'unknown' && (
        <>
          <PlainTextLine text={event.payload.raw} />
          {event.payload.truncated && <span> (truncated)</span>}
        </>
      )}
      {event.kind === 'tool-call' && (
        <div data-testid="tool-call-proposal" aria-label="proposal, not executed">
          <p>proposal, not executed</p>
          <p>
            Tool: <PlainTextLine text={event.payload.name} />
          </p>
          <p>
            Arguments: <PlainTextLine text={event.payload.argumentsText} />
          </p>
        </div>
      )}
      {event.kind === 'terminal-text' && (
        <div data-testid="terminal-text">
          <span>{TERMINAL_TEXT_LABEL}</span>
          {splitTerminalLines(event.payload.text).map((line, index) => (
            // An index key is sound here: the list is derived once from an
            // immutable payload and is never reordered or edited.
            <div key={index} data-testid="terminal-line">
              <PlainTextLine text={line} />
            </div>
          ))}
        </div>
      )}
      {event.kind === 'terminal-action' && (
        <PlainTextLine
          text={`${formatTerminalActionLabel(event.payload.action)}: ${event.payload.text}`}
        />
      )}
      {event.kind === 'terminal-drops' && <mark>{formatTerminalDrops(event.payload)}</mark>}
    </li>
  )
}

/**
 * First built-in harness adapter control panel (`docs/spike-log.md` §
 * Slice 3): pick a workspace, pick a built-in adapter and an approved
 * executable, send a size-capped prompt, and watch the resulting
 * structured event transcript.
 *
 * A sibling of `HarnessPanel` and `ApprovalSurface`, never nested inside
 * either: `App.tsx` mounts all three side by side. Every rendered piece of
 * agent output -- the echoed prompt, every event's text -- goes through
 * {@link PlainTextLine}, matching `HarnessPanel`'s own no-markup-sink
 * guarantee; a `tool-call` proposal is rendered as inert data only, never
 * as anything a click could execute (RCS-001-R6/R16/R18,
 * `docs/renderer-content-security.md`).
 *
 * Spike slice 4 adds the `pty-cli` adapter's degraded-fallback path on the
 * same surface: a fixed transport disclosure while a `pty` adapter is
 * selected, and the three `terminal-*` event kinds rendered as plain text
 * in the same transcript (`docs/spike-log.md` § Slice 4).
 */
export function AgentPanel() {
  const [workspace, setWorkspace] = useState<Workspace | null>(null)
  const [workspaceStatus, setWorkspaceStatus] = useState<string | null>(null)
  const [isPickingWorkspace, setIsPickingWorkspace] = useState(false)

  const [adapters, setAdapters] = useState<AdapterDescriptor[]>([])
  const [adapterId, setAdapterId] = useState<string | null>(null)

  const [approvals, setApprovals] = useState<Approval[]>([])
  const [approvalId, setApprovalId] = useState<ApprovalId | null>(null)

  const [prompt, setPrompt] = useState('')

  const [activeId, setActiveId] = useState<ProcessId | null>(null)
  const [isSpawning, setIsSpawning] = useState(false)
  const [badge, setBadge] = useState('idle')
  const [transcript, setTranscript] = useState<TranscriptState>(EMPTY_TRANSCRIPT)
  const [error, setError] = useState<PanelError | null>(null)

  /**
   * True from the Start click until the run ends (its terminal `state`
   * frame, or a successful stop) or the spawn is rejected. While true, the
   * workspace, adapter and approval controls are disabled and their
   * handlers ignore input: the transport disclosure derives from the
   * selected adapter, so a selection change under a streaming run would
   * hide the disclosure while pty output keeps arriving (slice 4 review,
   * R3-005). A rejected stop leaves the run -- and this -- as it was.
   */
  const runActive = isSpawning || activeId !== null

  /**
   * Tracks whether this component instance is still mounted, guarding
   * every async continuation below against a post-unmount state update --
   * mirrors `HarnessPanel`/`ApprovalSurface`'s own `mountedRef` (R3-001).
   */
  const mountedRef = useRef(true)
  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  useEffect(() => {
    workspaceCurrent()
      .then((current) => {
        if (!mountedRef.current) return
        setWorkspace(current)
      })
      .catch((currentError: unknown) => {
        if (!mountedRef.current) return
        setError(isShellError(currentError) ? currentError : 'unexpected')
      })
  }, [])

  useEffect(() => {
    adaptersList()
      .then((list) => {
        if (!mountedRef.current) return
        setAdapters(list)
      })
      .catch((listError: unknown) => {
        if (!mountedRef.current) return
        setError(isShellError(listError) ? listError : 'unexpected')
      })
  }, [])

  /**
   * The sequence number of the most recently *started* `approvals_list`
   * fetch, mirroring `HarnessPanel`'s own sequenced-fetch pattern
   * (R3-008): a response is only applied if it is still the latest one in
   * flight, otherwise it is a stale response from an earlier fetch and is
   * silently dropped.
   */
  const latestApprovalsRequestRef = useRef(0)

  useEffect(() => {
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
  }, [])

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
   * active after the fact (R3-002).
   */
  const runEndedRef = useRef(true)

  const handleFrame = useCallback((generation: number, frame: HarnessFrame) => {
    if (!mountedRef.current) return
    // Stale-channel guard (R1-001/R3-001): every `harnessSpawn` wires a
    // fresh Channel to this same handler, so a trailing frame from an
    // earlier run's channel can still arrive after a new run has started
    // -- even before the new run's id is known. Only the current
    // generation's frames are applied; every other channel's are dropped
    // outright -- never appended, never allowed to move the badge.
    if (generation !== spawnGenerationRef.current) return
    // Belt and braces once the id is known: a frame on the current channel
    // carrying another run's id is dropped too, for the rest of this
    // generation -- after the run has ended as much as while it runs.
    // While the spawn is still in flight the generation alone decides, so
    // a frame that beats the spawn promise is applied, not dropped -- a
    // terminal frame arriving that early must still end the run (R3-002).
    if (runIdRef.current !== null && frame.body.id !== runIdRef.current) return
    if (frame.stream === 'state') {
      // Every `state` frame is terminal by contract (`HarnessStateFrameBody`
      // extends `ProcessTerminalState`, whose closed token set is exited/
      // killed/orphan-risk/uncertain), so the run is over: free the single
      // active slot here, not only from the Stop path (R3-002).
      runEndedRef.current = true
      setActiveId(null)
      const token = formatTerminalToken(frame.body)
      setBadge(token)
      setTranscript((previous) =>
        appendTranscriptEntry(previous, {
          type: 'terminal-state',
          key: `${frame.body.id}-${frame.body.seq}`,
          droppedBefore: frame.body.droppedBefore,
          token,
        }),
      )
      return
    }
    if (frame.stream === 'event') {
      // An empty `terminal-text` with nothing dropped before it carries
      // nothing to show: no entry, so it neither occupies a transcript row
      // nor a slot under the cap (slice 4 review, R3-006). One riding a
      // nonzero `droppedBefore` is kept for its degraded marker alone.
      if (
        frame.body.kind === 'terminal-text' &&
        frame.body.payload.text === '' &&
        frame.body.droppedBefore === 0
      ) {
        return
      }
      setTranscript((previous) =>
        appendTranscriptEntry(previous, {
          type: 'agent-event',
          key: `${frame.body.id}-${frame.body.seq}`,
          droppedBefore: frame.body.droppedBefore,
          event: frame.body,
        }),
      )
      return
    }
    // stdout/stderr raw frames are never emitted for an adapter launch
    // (`docs/spike-log.md` § Slice 3) -- ignored defensively rather than
    // asserted unreachable, so a future protocol change fails safe (no
    // rendering) instead of throwing.
  }, [])

  async function handlePickWorkspace() {
    // Belt and braces with the button's own `disabled`: never move the
    // workspace under an active run.
    if (runActive) return
    setError(null)
    setWorkspaceStatus(null)
    setIsPickingWorkspace(true)
    try {
      const picked = await workspacePick()
      if (!mountedRef.current) return
      setWorkspace(picked)
    } catch (pickError: unknown) {
      if (!mountedRef.current) return
      if (isShellError(pickError) && pickError.code === 'no-workspace') {
        setWorkspaceStatus('No workspace selected')
        return
      }
      setError(isShellError(pickError) ? pickError : 'unexpected')
    } finally {
      if (mountedRef.current) setIsPickingWorkspace(false)
    }
  }

  async function handleStart() {
    // Belt-and-suspenders: the Start button is already disabled while a
    // required selection is missing or the prompt exceeds the advisory
    // cap, but never spawn on a value this panel itself considers invalid.
    if (adapterId === null || approvalId === null) return
    if (promptByteLength(prompt) > PROMPT_MAX_BYTES) return

    const sentPrompt = prompt
    const generation = (spawnGenerationRef.current += 1)
    runEndedRef.current = false
    runIdRef.current = null
    setError(null)
    // The echoed prompt is entered before the spawn is even requested,
    // keyed by generation rather than by id, so a frame that beats the
    // spawn promise still lands after it in the transcript.
    setTranscript({
      entries: [{ type: 'prompt', key: `prompt-${generation}`, text: sentPrompt }],
      trimmedCount: 0,
    })
    setIsSpawning(true)
    try {
      const id = await harnessSpawn(
        { type: 'adapter', adapterId, approvalId, prompt: sentPrompt },
        (frame) => handleFrame(generation, frame),
      )
      if (!mountedRef.current) return
      if (generation !== spawnGenerationRef.current) return
      // The id is known from here on for the rest of this generation,
      // whether or not the run is still going.
      runIdRef.current = id
      // The run's terminal `state` frame may already have arrived and
      // ended it before the id was known: leave it ended, never mark it
      // active after the fact (R3-002).
      if (runEndedRef.current) return
      setActiveId(id)
      setBadge('running')
    } catch (spawnError: unknown) {
      if (!mountedRef.current) return
      if (generation !== spawnGenerationRef.current) return
      // No process, so no run to echo a prompt for: withdraw this
      // generation's own echoed prompt -- only that entry, so a frame that
      // already reached the current channel before the rejection is kept
      // rather than wiped (R1-012/R3-012).
      setTranscript((previous) => ({
        ...previous,
        entries: previous.entries.filter((entry) => entry.key !== `prompt-${generation}`),
      }))
      setError(isShellError(spawnError) ? spawnError : 'unexpected')
    } finally {
      if (mountedRef.current) setIsSpawning(false)
    }
  }

  async function handleStop() {
    if (activeId === null) return
    setError(null)
    try {
      const state = await harnessStop(activeId, STOP_DEADLINE_MS)
      if (!mountedRef.current) return
      setBadge(formatTerminalToken(state))
      runEndedRef.current = true
      setActiveId(null)
    } catch (stopError: unknown) {
      // A failed stop leaves the run exactly as it was -- active id kept,
      // Stop still enabled -- rather than pretending the process ended
      // (R3-006): the supervisor never confirmed a terminal state.
      if (!mountedRef.current) return
      setError(isShellError(stopError) ? stopError : 'unexpected')
    }
  }

  const activeApprovals = approvals.filter((approval) => approval.status === 'active')
  const selectedAdapter = adapters.find((adapter) => adapter.id === adapterId) ?? null
  const promptTooLarge = promptByteLength(prompt) > PROMPT_MAX_BYTES
  const startDisabled = runActive || adapterId === null || approvalId === null || promptTooLarge

  return (
    <section aria-label="Agent">
      <h2>Agent</h2>

      <p>
        <span data-testid="agent-scope-badge">advisory scope — not a sandbox</span>
      </p>
      <p data-testid="agent-producer-line">producer: unsigned (unknown)</p>

      {error && (
        <div role="alert" data-testid="agent-error-banner">
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
        <button
          type="button"
          onClick={handlePickWorkspace}
          disabled={isPickingWorkspace || runActive}
        >
          Pick workspace
        </button>
        {workspace && (
          <p>
            Workspace: <PlainTextLine text={workspace.displayPath} />
          </p>
        )}
        {workspaceStatus && <p role="status">{workspaceStatus}</p>}
      </div>

      <div>
        <label htmlFor="agent-adapter">Adapter</label>
        <select
          id="agent-adapter"
          value={adapterId ?? ''}
          disabled={runActive}
          onChange={(event) => {
            // A `change` dispatched at a disabled select still reaches
            // React; the guard keeps the selection frozen regardless.
            if (runActive) return
            setAdapterId(event.target.value === '' ? null : event.target.value)
          }}
        >
          <option value="">Select an adapter</option>
          {adapters.map((adapter) => (
            <option key={adapter.id} value={adapter.id}>
              {stripControlCharacters(adapter.displayName)}
            </option>
          ))}
        </select>
        {selectedAdapter && (
          <p>
            <PlainTextLine text={selectedAdapter.notes} />
          </p>
        )}
        {selectedAdapter?.transportClass === 'pty' && (
          <p data-testid="agent-transport-disclosure">{PTY_TRANSPORT_DISCLOSURE}</p>
        )}

        <label htmlFor="agent-approval">Approval</label>
        <select
          id="agent-approval"
          value={approvalId === null ? '' : String(approvalId)}
          disabled={runActive}
          onChange={(event) => {
            if (runActive) return
            setApprovalId(event.target.value === '' ? null : Number(event.target.value))
          }}
        >
          <option value="">Select an approval</option>
          {activeApprovals.map((approval) => (
            <option key={approval.approvalId} value={approval.approvalId}>
              {stripControlCharacters(approval.evidence.canonicalPath)}
            </option>
          ))}
        </select>

        <label htmlFor="agent-prompt">Prompt</label>
        <textarea
          id="agent-prompt"
          value={prompt}
          onChange={(event) => setPrompt(event.target.value)}
          aria-invalid={promptTooLarge}
        />
        {promptTooLarge && (
          <p data-testid="agent-prompt-size-validation">
            {`Prompt exceeds the 16 KiB size limit (${promptByteLength(prompt)} bytes); reduce it before starting.`}
          </p>
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

      <ul aria-label="Agent transcript">
        {transcript.trimmedCount > 0 && (
          <li>
            <mark>
              {`showing the last ${MAX_TRANSCRIPT_ENTRIES} entries; ${transcript.trimmedCount} earlier entries are not shown`}
            </mark>
          </li>
        )}
        {transcript.entries.map((item) => (
          <TranscriptEntryView key={item.key} item={item} />
        ))}
      </ul>
    </section>
  )
}

export default AgentPanel
