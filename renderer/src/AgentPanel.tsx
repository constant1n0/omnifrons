import { useCallback, useEffect, useRef, useState } from 'react'

import { stripControlCharacters } from './controlCharacters'
import {
  adaptersList,
  approvalsList,
  candidatesList,
  harnessSpawn,
  harnessStop,
  isShellError,
  outboxStatus,
  workspaceCurrent,
  workspacePick,
  type AdapterDescriptor,
  type AgentEvent,
  type AgentPhaseTag,
  type Approval,
  type ApprovalId,
  type Attribution,
  type Candidate,
  type CandidateState,
  type CandidatesSummary,
  type HarnessFrame,
  type Observation,
  type OutboxReason,
  type OutboxStatus,
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

/**
 * The fixed English for an `outbox_status` reason token (`docs/spike-log.md`
 * § Slice 5): chosen by an exhaustive switch over the closed token set,
 * never the raw payload echoed. `null` -- a non-`valid` state with no
 * reason, which the shell never produces -- reads as unreported rather
 * than as an empty line.
 */
function formatOutboxReason(reason: OutboxReason | null): string {
  switch (reason) {
    case 'outside-project':
      return 'the declared path resolves outside the project'
    case 'link':
      return 'the declared path is a link'
    case 'not-a-directory':
      return 'the declared path is not a directory'
    case 'unreadable':
      return 'the declared path could not be read'
    case 'policy-unreadable':
      return 'the classification policy could not be read'
    case 'policy-corrupt':
      return 'the classification policy is corrupt'
    case 'policy-invalid':
      return 'the classification policy is invalid'
    case null:
      return 'reason unreported'
  }
}

/** The cell text for a fact the wire carries as `null` (a refused entry was never digested). */
const NULL_FACT = '—'

/**
 * The tail of a non-`valid` status line: the declared project-relative
 * name shown as the thing that is invalid, with the reason in parentheses
 * (HAP-001's visible state for `outbox-invalid` is "the declared path shown
 * as invalid"; slice 5 review, R3-017), or the reason alone when the policy
 * itself could not be loaded and nothing was declared.
 */
function formatDeclaredReason(status: OutboxStatus): string {
  const reason = formatOutboxReason(status.reason)
  return status.declared === null ? reason : `${status.declared} (${reason})`
}

/**
 * The outbox status line's text, fixed copy per closed state token
 * (`docs/spike-log.md` § Slice 5): a valid, existing outbox shows its
 * canonical path (the third explicit RCS-001-R14 exception -- identity
 * evidence of where a run's output lands, display-only) beside the
 * declared project-relative name; a valid declaration whose directory does
 * not exist yet shows only the declared name, since the first adapter
 * launch creates it; an invalid or unavailable declaration shows the
 * declared name (when the policy declared one) and its reason in fixed
 * English, never a device path. The whole line renders through
 * {@link PlainTextLine}: the path and the declared name are
 * project-originated text.
 */
function formatOutboxStatusLine(status: OutboxStatus): string {
  switch (status.state) {
    case 'valid':
      return status.exists && status.outbox !== null
        ? `outbox: ${status.outbox} (declared ${status.declared ?? NULL_FACT})`
        : `outbox: ${status.declared ?? NULL_FACT} (created at the first launch)`
    case 'outbox-invalid':
      return `outbox invalid: ${formatDeclaredReason(status)}`
    case 'outbox-unavailable':
      return `outbox unavailable: ${formatDeclaredReason(status)}`
  }
}

/** The visible label on every `artifact-publish` transcript entry. */
const PUBLISH_PROPOSAL_LABEL = 'publish proposal:'

/**
 * The first eight characters of a proposal's full digest -- the same short
 * form `sha256Short` carries elsewhere on the wire -- taken by code point,
 * so a digest that is not the 64 hex characters the contract promises is
 * still cut cleanly and rendered as the text it is.
 */
function shortDigest(sha256: string): string {
  return Array.from(sha256).slice(0, 8).join('')
}

/**
 * The `candidates` summary line: the eight counts of the shell's run-end
 * inventory in a fixed order, zero counts included -- every count is always
 * accounted for, like {@link formatTerminalDrops}' families. The run id the
 * payload carries is the key of the follow-up `candidatesList` fetch, not
 * part of the line.
 */
function formatCandidatesSummary(summary: CandidatesSummary): string {
  const counts = [
    `${summary.total} total`,
    `${summary.candidate} candidate`,
    `${summary.outboxEscape} escape`,
    `${summary.outboxLinked} linked`,
    `${summary.attributed} attributed`,
    `${summary.unattributed} unattributed`,
    `${summary.unreadable} unreadable`,
    `${summary.unmatchedProposals} unmatched proposals`,
  ]
  return `candidates: ${counts.join(', ')}`
}

/**
 * The attribution cell: fixed copy chosen per closed kind, never the wire
 * token echoed -- the switch is exhaustive, so a new kind cannot render
 * unlabelled. The run id of an attributed entry is not repeated per row:
 * the table is one run's inventory, and location alone never attributes
 * (HAP-001-R11), so `run` here means the run's own proposal named the entry.
 */
function formatAttribution(attribution: Attribution): string {
  switch (attribution.kind) {
    case 'run':
      return 'run'
    case 'unattributed':
      return 'unattributed'
  }
}

/**
 * The state cell: each closed wire token transcribed verbatim
 * (`docs/target-architecture.md` invariant 8, as `formatTerminalToken`
 * does), through an exhaustive switch so a new state cannot render
 * unlabelled.
 */
function formatCandidateState(state: CandidateState): string {
  switch (state) {
    case 'candidate':
      return 'candidate'
    case 'outbox-escape':
      return 'outbox-escape'
    case 'outbox-linked':
      return 'outbox-linked'
  }
}

/**
 * The fixed line above the candidates table: no approval, no publication
 * and no ingestion exists in this slice (HAP-001's publication transaction
 * is slice 5b), so the table is inert data and says so.
 */
const PUBLICATION_UNAVAILABLE = 'publication not available in this slice'

/**
 * One row of the candidates table, every cell through {@link PlainTextLine}:
 * the name is producer-supplied text -- HAP-001-R24's display-name
 * sanitization is slice 5b, so a traversal sequence in it is data here and
 * never resolved -- the facts come from the entry's handle and are `null`
 * for a refused entry, where nothing was digested (rendered as `—`), and a
 * refused row (`outbox-escape`, `outbox-linked`) is marked `refused` beside
 * its state token. Nothing in a row is interactive.
 */
function CandidateRow({ candidate }: { candidate: Candidate }) {
  return (
    <tr>
      <td>
        <PlainTextLine text={candidate.name} />
      </td>
      <td>
        <PlainTextLine text={candidate.size === null ? NULL_FACT : String(candidate.size)} />
      </td>
      <td>
        <PlainTextLine text={candidate.sha256Short ?? NULL_FACT} />
      </td>
      <td>
        <PlainTextLine text={candidate.detectedType ?? NULL_FACT} />
      </td>
      <td>
        <PlainTextLine text={candidate.class ?? NULL_FACT} />
      </td>
      <td>
        <PlainTextLine text={formatAttribution(candidate.attribution)} />
      </td>
      <td>
        <PlainTextLine text={formatCandidateState(candidate.state)} />
        {candidate.state !== 'candidate' && (
          <>
            {' '}
            <mark>refused</mark>
          </>
        )}
      </td>
    </tr>
  )
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
 *
 * The two outbox kinds (`docs/spike-log.md` § Slice 5) render as inert data
 * too: `artifact-publish` as a labelled `publish proposal:` block listing
 * each entry as its name and the first eight characters of its digest --
 * a proposal the harness made and nothing here executes, held to the same
 * zero-interactive-elements bar as the tool-call block -- and `candidates`
 * as one fixed summary line of the eight counts (the table it announces is
 * rendered beside the transcript by the panel, not inside it).
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
      {event.kind === 'candidates' && (
        <PlainTextLine text={formatCandidatesSummary(event.payload)} />
      )}
      {event.kind === 'artifact-publish' && (
        <div data-testid="publish-proposal">
          <p>{PUBLISH_PROPOSAL_LABEL}</p>
          {event.payload.entries.map((entry, index) => (
            // An index key is sound here: the list is derived once from an
            // immutable payload and is never reordered or edited.
            <p key={index} data-testid="publish-entry">
              <PlainTextLine text={`${entry.name} ${shortDigest(entry.sha256)}`} />
            </p>
          ))}
        </div>
      )}
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

  /**
   * The outbox's status for the active workspace (`docs/spike-log.md` §
   * Slice 5), or `null` while unknown or while no workspace is active.
   */
  const [outbox, setOutbox] = useState<OutboxStatus | null>(null)

  /**
   * The sequence number of the most recently *started* `outbox_status`
   * fetch -- the same sequenced-fetch pattern as `approvals_list` below
   * (R3-008): the mount-time fetch and a pick-time fetch can be in flight
   * together, and only the latest one's response is applied.
   */
  const latestOutboxStatusRequestRef = useRef(0)

  /**
   * Fetches the outbox status: on mount, and again after a successful
   * workspace pick. A `workspace-unavailable` rejection is the shell's
   * answer while no workspace is active -- the panel already shows that no
   * workspace is picked, so it clears the line and raises no alert; any
   * other rejection reaches the banner like every other mount-time fetch.
   * A stable callback, so the mount effect below can list it as its one
   * dependency.
   */
  const refreshOutboxStatus = useCallback(() => {
    const requestId = (latestOutboxStatusRequestRef.current += 1)
    outboxStatus()
      .then((status) => {
        if (!mountedRef.current) return
        if (requestId !== latestOutboxStatusRequestRef.current) return
        setOutbox(status)
      })
      .catch((statusError: unknown) => {
        if (!mountedRef.current) return
        if (requestId !== latestOutboxStatusRequestRef.current) return
        if (isShellError(statusError) && statusError.code === 'workspace-unavailable') {
          setOutbox(null)
          return
        }
        setError(isShellError(statusError) ? statusError : 'unexpected')
      })
  }, [])

  useEffect(() => {
    refreshOutboxStatus()
  }, [refreshOutboxStatus])

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

  /**
   * The current run's candidates table (`docs/spike-log.md` § Slice 5):
   * the rows `candidates_list` returned for the run id the run's own
   * `candidates` event named, or `null` until then. Cleared by the next
   * Start along with the transcript, so a new run never shows the previous
   * run's inventory beside its own output.
   */
  const [candidates, setCandidates] = useState<Candidate[] | null>(null)

  /**
   * The sequence number of the most recently *started* `candidates_list`
   * fetch (R3-008 pattern): only the latest fetch's response is applied.
   */
  const latestCandidatesRequestRef = useRef(0)

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
      if (frame.body.kind === 'candidates') {
        // The run's subdirectory has been inventoried: fetch the rows behind
        // the summary, by the run id the shell minted (never a path).
        // Sequenced like every other fetch here, and gated on the generation
        // this channel's closure captured, so a response landing after the
        // next Start never dresses a new run in an old run's candidates.
        const { runId } = frame.body.payload
        const requestId = (latestCandidatesRequestRef.current += 1)
        candidatesList(runId)
          .then((rows) => {
            if (!mountedRef.current) return
            if (generation !== spawnGenerationRef.current) return
            if (requestId !== latestCandidatesRequestRef.current) return
            setCandidates(rows)
          })
          .catch((listError: unknown) => {
            if (!mountedRef.current) return
            if (generation !== spawnGenerationRef.current) return
            if (requestId !== latestCandidatesRequestRef.current) return
            setError(isShellError(listError) ? listError : 'unexpected')
          })
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
      // The outbox is declared per project: a new workspace means a new
      // status, fetched afresh rather than carried over.
      refreshOutboxStatus()
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
    // The previous run's candidates table, kept aside so a rejected spawn
    // can put it back (R3-019): the table is cleared below on the premise
    // that a new run replaces it, and a rejection means none did.
    const previousCandidates = candidates
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
    setCandidates(null)
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
      // Likewise the candidates table: no run replaced the previous run's
      // inventory, so the table this Start cleared comes back -- unless a
      // frame that reached the current channel before the rejection already
      // produced a new one, which is kept, like such a frame's transcript
      // entry is (R3-019, R1-012 analogue).
      setCandidates((current) => current ?? previousCandidates)
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
        {outbox && (
          // A status line like the workspace-pick status beside it, named
          // so the two live regions stay distinguishable (R3-018).
          <p role="status" aria-label="Outbox">
            <PlainTextLine text={formatOutboxStatusLine(outbox)} />
          </p>
        )}
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

      {candidates && (
        <section aria-label="Candidates">
          <p>{PUBLICATION_UNAVAILABLE}</p>
          <table>
            <thead>
              <tr>
                <th>name</th>
                <th>size</th>
                <th>sha256</th>
                <th>type</th>
                <th>class</th>
                <th>attribution</th>
                <th>state</th>
              </tr>
            </thead>
            <tbody>
              {candidates.map((candidate, index) => (
                // An index key is sound here: the list is replaced whole by
                // each fetch and is never reordered or edited in place.
                <CandidateRow key={index} candidate={candidate} />
              ))}
            </tbody>
          </table>
        </section>
      )}
    </section>
  )
}

export default AgentPanel
