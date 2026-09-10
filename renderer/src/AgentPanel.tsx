import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'

import { stripControlCharacters } from './controlCharacters'
import {
  adaptersList,
  approvalsList,
  artifactApprove,
  artifactPublish,
  candidatesList,
  guidanceApply,
  guidancePin,
  guidancePreview,
  guidanceRemove,
  guidanceRestore,
  guidanceSnapshots,
  guidanceStatus,
  harnessSpawn,
  harnessStop,
  isShellError,
  outboxStatus,
  publicationsList,
  workspaceCurrent,
  workspacePick,
  type AdapterDescriptor,
  type AgentEvent,
  type AgentPhaseTag,
  type Approval,
  type ApprovalId,
  type ArtifactApproval,
  type ArtifactState,
  type ArtifactStateFrame,
  type Attribution,
  type Availability,
  type Candidate,
  type CandidateState,
  type CandidatesSummary,
  type GuidanceAction,
  type GuidanceApplied,
  type GuidanceKind,
  type GuidanceStatus,
  type HarnessFrame,
  type Observation,
  type OutboxReason,
  type OutboxStatus,
  type ProcessId,
  type ProcessTerminalState,
  type ProviderState,
  type Publication,
  type ScopeMode,
  type ShellError,
  type ShellErrorCode,
  type Snapshot,
  type TerminalActionKind,
  type TerminalDropCounts,
  type WorkAreaState,
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

/**
 * The status line for the product work area's re-check against the active
 * workspace (`docs/spike-log.md` § Slice 5b, HAP-001-R7): fixed copy per
 * closed token through an exhaustive switch, never the raw token echoed --
 * `null` for `valid`, since a valid work area renders nothing. Every
 * publication command refuses with `work-area-invalid` until the area is
 * reconfigured, which is what the line says.
 */
function formatWorkAreaLine(state: WorkAreaState): string | null {
  switch (state) {
    case 'valid':
      return null
    case 'work-area-invalid':
      return 'work area invalid — publication refused until it is fixed'
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
 * An artifact state (`docs/spike-log.md` § Slice 5b): each of HAP-001's ten
 * closed tokens transcribed verbatim (`docs/target-architecture.md`
 * invariant 8), through an exhaustive switch so a new state cannot render
 * unlabelled -- the same discipline as {@link formatCandidateState}.
 */
function formatArtifactState(state: ArtifactState): string {
  switch (state) {
    case 'candidate':
      return 'candidate'
    case 'published-local':
      return 'published-local'
    case 'registered':
      return 'registered'
    case 'provider-synced':
      return 'provider-synced'
    case 'registration-pending':
      return 'registration-pending'
    case 'refused':
      return 'refused'
    case 'integrity-mismatch':
      return 'integrity-mismatch'
    case 'duplicate-publication':
      return 'duplicate-publication'
    case 'outbox-escape':
      return 'outbox-escape'
    case 'outbox-linked':
      return 'outbox-linked'
  }
}

/** A record's provider state, transcribed verbatim through an exhaustive switch; `null` (no record yet) is a null fact. */
function formatProviderState(state: ProviderState | null): string {
  switch (state) {
    case 'pending':
      return 'pending'
    case 'synced':
      return 'synced'
    case 'failed':
      return 'failed'
    case 'unavailable':
      return 'unavailable'
    case null:
      return NULL_FACT
  }
}

/** This device's own availability observation (HAP-001-R27), transcribed verbatim through an exhaustive switch. */
function formatAvailability(availability: Availability): string {
  switch (availability) {
    case 'local':
      return 'local'
    case 'unknown':
      return 'unknown'
  }
}

/**
 * The short form of a Catalog identity `<asset root id>/<publication hex>`:
 * the asset root id kept whole, the publication hex cut to its first eight
 * characters by code point like {@link shortDigest} -- `main/2a91ea59` for
 * the fixture record. Display-only, like the identity it abbreviates: never
 * resolved as a path, a link, or an address. A value with no `/` (not the
 * shape the shell issues) is cut whole, as the text it is.
 */
function shortCatalogId(catalogId: string): string {
  const slash = catalogId.indexOf('/')
  if (slash === -1) return shortDigest(catalogId)
  return `${catalogId.slice(0, slash + 1)}${shortDigest(catalogId.slice(slash + 1))}`
}

/**
 * One row of the publications table (`docs/spike-log.md` § Slice 5b), every
 * cell through {@link PlainTextLine}: the sanitized display names joined
 * (producer-supplied text re-sanitized by the shell on read, HAP-001-R24,
 * and portable content, HAP-001-R40 -- plain text here regardless), the
 * state token, the record's provider state, the reference's locator, the
 * Catalog identity's short form, and this device's availability. A `null`
 * fact -- no record before registration -- renders as `—`. The locator is
 * display-only: nothing here builds a link, a path, or an address from it.
 * Nothing in a row is interactive.
 */
function PublicationRow({ publication }: { publication: Publication }) {
  return (
    <tr>
      <td>
        <PlainTextLine text={publication.names.join(', ')} />
      </td>
      <td>
        <PlainTextLine text={formatArtifactState(publication.state)} />
      </td>
      <td>
        <PlainTextLine text={formatProviderState(publication.providerState)} />
      </td>
      <td>
        <PlainTextLine text={publication.reference?.locator ?? NULL_FACT} />
      </td>
      <td>
        <PlainTextLine
          text={publication.catalogId === null ? NULL_FACT : shortCatalogId(publication.catalogId)}
        />
      </td>
      <td>
        <PlainTextLine text={formatAvailability(publication.availability)} />
      </td>
    </tr>
  )
}

/**
 * The attribution fact of the publication approval block (HAP-001-R22: "the
 * producing run or the unattributed fact"): the run named by its id -- the
 * shell's own minted token, never a path -- or the bare unattributed fact,
 * with nothing standing in for a producer. Exhaustive over the closed kinds.
 */
function formatAttributionFact(attribution: Attribution): string {
  switch (attribution.kind) {
    case 'run':
      return `run ${attribution.runId}`
    case 'unattributed':
      return 'unattributed'
  }
}

/**
 * The destination fact of the approval block (HAP-001-R22: "destination by
 * display name and asset root identity"): the policy's asset root identity
 * token from the current outbox status -- `asset root <id>`; `unconfigured`
 * when the policy declares none, in which case the shell would refuse the
 * approval with `destination-invalid`; `unknown` while no status is known at
 * all. The two latter cases keep the final button disabled: no approval
 * toward a destination the surface cannot show. One token, never a path.
 */
function formatDestinationFact(outbox: OutboxStatus | null): string {
  if (outbox === null) return 'unknown'
  return outbox.assetRootId === null ? 'unconfigured' : `asset root ${outbox.assetRootId}`
}

/**
 * The scope fact of the approval block: the scope mode of the adapter the
 * run was started with, from its descriptor -- no publication DTO carries a
 * scope, so this is renderer-side, and it follows the producing launch rather
 * than the selection at approval time. Fixed copy per closed token through an
 * exhaustive switch; `null` (no descriptor found) reads as unreported.
 */
function formatScopeMode(mode: ScopeMode | null): string {
  switch (mode) {
    case 'sandbox-enforced':
      return 'sandbox-enforced'
    case 'harness-enforced':
      return 'harness-enforced'
    case 'advisory':
      return 'advisory'
    case null:
      return 'unreported'
  }
}

/**
 * The scope fact of the approval block for `inventory`: `none` for the
 * whole-outbox listing (slice 5c) -- no run produced it, so there is no
 * adapter whose scope mode could apply, and nothing enforces anything over
 * an unmediated producer (HAP-001 § Unmediated producers) -- else the scope
 * mode of the adapter the run was started with, through
 * {@link formatScopeMode}.
 */
function formatScopeFact(inventory: CandidatesInventory, adapter: AdapterDescriptor | null): string {
  if (inventory.runId === null) return 'none'
  return formatScopeMode(adapter?.scopeMode ?? null)
}

/**
 * The marker beside an approved row whose handle is not held (slice 5c;
 * HAP-001-R17, D22): the cap left no room for it, the approval stands on
 * the facts verified from the re-opened handle, and the publication re-opens
 * the entry under the single-handle discipline when its turn comes -- a
 * degraded fact said on the surface rather than left silent, like the
 * `refused` and dropped-frames markers. Fixed copy, never wire text.
 */
const HANDLE_NOT_HELD_MARK = 'handle: not held (cap reached)'

/**
 * Whether a candidates row offers approval (spike default): a validated
 * `candidate` of the `generated-heavy` class carrying its full digest --
 * attributed or unattributed alike, HAP-001 admitting an unattributed entry
 * to explicit human approval -- and the shell refuses every other state or
 * class as `refused`, so the panel offers no button it knows the shell
 * would refuse. A refused row (`outbox-escape`, `outbox-linked`) carries no
 * digest at all, and a row with no `sha256` has nothing to name an approval
 * by.
 */
function isApprovable(candidate: Candidate): boolean {
  return (
    candidate.state === 'candidate' &&
    candidate.class === 'generated-heavy' &&
    candidate.sha256 !== null
  )
}

/**
 * Whether `input` confirms `candidate`: an exact, case-sensitive match
 * against the row's own short digest, the fact the block shows -- slice 2's
 * shape and its R1-001 discipline (no fixed phrase accepted in its place, so
 * the act cannot be completed without having read the evidence it stands
 * in for). The typed value is a gate only: the request carries the row's
 * full `sha256`, never anything typed.
 */
function isApprovalConfirmed(candidate: Candidate, input: string): boolean {
  return candidate.sha256Short !== null && input === candidate.sha256Short
}

/**
 * TM-001-R7's act-as identity, stated on the approval surface before the
 * decision: fixed copy -- `device-local-user` is the one `ActAs` token the
 * shell can bind (`ActAsTag` has one variant), so this is the renderer's
 * own constant, never wire text, like `ApprovalSurface`'s "Approving as"
 * line.
 */
const APPROVAL_ACT_AS_LINE = 'act as: device-local-user'

/**
 * A row's publication progress, tracked by the panel per candidate name
 * from the approval on: approved (Publish offered), publishing (Publish in
 * flight), or published (the transaction's own result). Reset whenever a
 * new inventory replaces the rows.
 */
type RowPublication =
  | { phase: 'approved'; approval: ArtifactApproval }
  | { phase: 'publishing'; approval: ArtifactApproval }
  | { phase: 'published'; publication: Publication }

const EMPTY_ROW_PUBLICATIONS: ReadonlyMap<string, RowPublication> = new Map()

/**
 * The action cell once approved: the approval id, the sanitized display
 * name (HAP-001-R24), and the destination asset root identity --
 * HAP-001-R22's destination, which this wire contract discloses only in
 * the approval's response, so it is shown here, once known.
 */
function formatApprovedCell(approval: ArtifactApproval): string {
  return `approved ${approval.approvalId} — ${approval.displayName}, asset root ${approval.assetRootId}`
}

/**
 * One inventory as the panel keeps it: the run id the shell minted (the key
 * of an approval request, never a path) and the rows -- or, for the
 * whole-outbox listing (slice 5c), no run id at all: `candidates_list {}`
 * lists without a run, every row unattributed, and an approval made from
 * it names no run (`artifact_approve` with `runId` null).
 */
interface CandidatesInventory {
  runId: string | null
  rows: Candidate[]
  /**
   * The adapter the run was started with, for the approval block's scope
   * line; `null` only if the launch's adapter is unknown.
   */
  adapterId: string | null
}

/**
 * One row of the candidates table, every cell through {@link PlainTextLine}:
 * the name is producer-supplied text -- HAP-001-R24's display-name
 * sanitization happens in the shell at approval, so a traversal sequence
 * in it is data here and never resolved -- the facts come from the entry's
 * handle and are `null` for a refused entry, where nothing was digested
 * (rendered as `—`), and a refused row (`outbox-escape`, `outbox-linked`)
 * is marked `refused` beside its state token. The data cells hold nothing
 * interactive; `action` is the panel's own affordance for the row (slice
 * 5b: Approve, the approval's progress and Publish, or nothing).
 */
function CandidateRow({ candidate, action }: { candidate: Candidate; action: ReactNode }) {
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
      <td>{action}</td>
    </tr>
  )
}

/**
 * The transcript line for one `artifact-state` frame of a publication
 * (`docs/spike-log.md` § Slice 5b, HAP-001-R35): `publication <first eight
 * characters of the publication identity>: <state token>`, the token
 * transcribed verbatim through {@link formatArtifactState}. The provider
 * state the frame also carries is the Publications table's column, not
 * this line's. Fixed copy around shell-minted values; never a path.
 */
function formatPublicationStateLine(payload: ArtifactStateFrame['payload']): string {
  return `publication ${shortDigest(payload.publicationId)}: ${formatArtifactState(payload.state)}`
}

type TranscriptItem =
  | { type: 'prompt'; key: string; text: string }
  | { type: 'agent-event'; key: string; droppedBefore: number; event: AgentEvent }
  /** The run's own terminal `state` frame, recorded in the transcript like `HarnessPanel` logs it. */
  | { type: 'terminal-state'; key: string; droppedBefore: number; token: string }
  /**
   * One transition of a publication this panel started (slice 5b): the
   * shell's own state frame, rendered as a fixed line -- not agent output,
   * and not an approval (RCS-001-R6 keeps those out of any terminal pane;
   * the transcript is a plain-text list, and the approval block lives
   * beside the candidates table, never here).
   */
  | { type: 'publication-state'; key: string; text: string }

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

  if (item.type === 'publication-state') {
    return (
      <li key={item.key}>
        <PlainTextLine text={item.text} />
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

// -- Slice 5c: the guidance-note installer (HAP-001 D18, R42) --

/**
 * The guidance file the installer proposes by default, mirroring the
 * shell's own `DEFAULT_GUIDANCE_FILE`. The user may name another Markdown
 * file at the workspace root; the shell alone decides what it accepts, and
 * refuses anything else with `guidance-file-invalid`.
 */
const DEFAULT_GUIDANCE_FILE = 'AGENTS.md'

/** The one file the `ignore` kind manages, fixed by the shell -- shown, never typed. */
const IGNORE_FILE = '.gitignore'

/**
 * The section's standing disclaimer (HAP-001 § Agent guidance note): the
 * note asks a producer to write its heavy output into the outbox and the
 * ignore rule keeps that outbox out of the project's history -- neither
 * confines anything. Containment, if it ever exists, is the sandbox's job,
 * never a sentence in a Markdown file. Fixed copy, never wire text.
 */
const GUIDANCE_ADVISORY_LINE =
  'advisory: the note and the ignore rule set expectations and enforce nothing — not containment'

/**
 * Stated on the write surface before the decision: every guidance write is
 * preceded by a snapshot of the file as it stands, so the act is
 * reversible from the Snapshots table beside it. Fixed copy.
 */
const GUIDANCE_SNAPSHOT_SENTENCE = 'a snapshot of the current file is taken before any write'

/**
 * Stated on the removal surface before the decision (slice 5c review,
 * R1-005): unlike an apply, opening a Remove previews nothing, so the
 * surface says in words what `guidance_remove` takes out -- exactly the
 * lines between the sentinels, and the file itself only when nothing else
 * is left in it and Omnifrons is the one that created it (the shell's own
 * `plan_remove`). Nothing outside the block is ever removed, so no
 * user-authored bytes are lost, and {@link GUIDANCE_SNAPSHOT_SENTENCE}
 * beside it says a snapshot precedes the act either way. Fixed copy.
 */
const GUIDANCE_REMOVE_SCOPE_LINE =
  'removes the managed block only; the file itself goes only when nothing else remains in it and Omnifrons created it'

/**
 * Stated on the restore surface before the decision (slice 5c review,
 * R1-002): a restore is the one write here into user-owned, version-tracked
 * content whose bytes the surface never shows. The block reports what the
 * manifest records about the snapshot -- its digest, its size, whether the
 * file existed -- and there is no command on the wire that returns a
 * snapshot's content, so the surface says so rather than implying the facts
 * beside it are the bytes. Fixed copy.
 */
const GUIDANCE_RESTORE_BYTES_LINE =
  "the snapshot's bytes are not shown here: only what the manifest records about them"

/**
 * Stated on the restore surface, before the gate, whenever the snapshot
 * records that the file did not exist (slice 5c review, R1-002): restoring
 * it writes no bytes back at all -- it removes the file, whatever the file
 * now holds. Neither the `restore` action token nor the `Restore snapshot`
 * button says that, so this line does. Fixed copy.
 */
const GUIDANCE_RESTORE_REMOVES_LINE =
  'this snapshot recorded no file: restoring it REMOVES the file rather than writing bytes back'

/** The visible label of a managed kind's block, chosen by an exhaustive switch over the closed set. */
function guidanceBlockLabel(kind: GuidanceKind): string {
  switch (kind) {
    case 'guidance':
      return 'Guidance note'
    case 'ignore':
      return 'Ignore rule'
  }
}

/**
 * The managed block's state, fixed copy per closed `managed` token through
 * an exhaustive switch -- never the raw token echoed, except where the
 * token itself is the fact (`absent`, `current`, `outdated`). `outdated`
 * also names the template version an apply would write, which is the only
 * wire value in this line besides the file name and the digest.
 * `modified` and `malformed` say what the user must do, since Omnifrons
 * refuses to overwrite either.
 */
function formatManagedBlockState(status: GuidanceStatus): string {
  switch (status.managed) {
    case 'absent':
      return 'block absent'
    case 'current':
      return 'block current'
    case 'outdated':
      return `block outdated (template ${status.templateVersion})`
    case 'modified':
      return 'block modified — resolve by hand or restore'
    case 'malformed':
      return 'block malformed — resolve by hand'
  }
}

/**
 * One managed file's status line: the file the shell named, the block's
 * state, the file's short digest -- the fact the typed gate stands in for
 * -- or its absence, and the snapshot counts. The whole line renders
 * through {@link PlainTextLine}: the file name is project-originated text.
 */
function formatGuidanceStatusLine(status: GuidanceStatus): string {
  const digest =
    status.fileSha256Short === null
      ? 'file absent'
      : `file sha256 short ${status.fileSha256Short}`
  return `${status.file}: ${formatManagedBlockState(status)}; ${digest}; snapshots ${status.snapshots}, pinned ${status.pinned}`
}

/**
 * A guidance action token, transcribed verbatim through an exhaustive
 * switch (`docs/target-architecture.md` invariant 8, as
 * {@link formatCandidateState} does), so a new action cannot render
 * unlabelled.
 */
function formatGuidanceAction(action: GuidanceAction): string {
  switch (action) {
    case 'insert':
      return 'insert'
    case 'replace':
      return 'replace'
    case 'no-op':
      return 'no-op'
    case 'remove':
      return 'remove'
    case 'restore':
      return 'restore'
  }
}

/** A boolean wire fact as the one-word cell the snapshots table shows. */
function formatYesNo(value: boolean): string {
  return value ? 'yes' : 'no'
}

/**
 * A snapshot's instant as an ISO 8601 time. A `takenAt` that is not a
 * representable instant -- which the shell never sends -- reads as a null
 * fact rather than throwing during render.
 */
function formatSnapshotInstant(takenAt: number): string {
  const instant = new Date(takenAt)
  return Number.isNaN(instant.getTime()) ? NULL_FACT : instant.toISOString()
}

/** The restore block's snapshot fact: everything the manifest records about the bytes it would write back. */
function formatSnapshotFact(snapshot: Snapshot): string {
  return `snapshot: ${snapshot.id}, taken at ${formatSnapshotInstant(snapshot.takenAt)}, sha256 short ${snapshot.sha256Short}, size ${snapshot.size}, existed ${formatYesNo(snapshot.existed)}`
}

/**
 * The result of a completed guidance write: the kind and the file it was
 * written to -- both carried by `GuidanceApplied`, and both named here
 * because the section has one result line shared by the two kinds (slice 5c
 * review, R1-003/R3-007) -- then what was done, the snapshot taken before it
 * (`none` for a no-op, which snapshots nothing), and the file's short digest
 * afterwards (`absent` once the file is gone). The whole line renders
 * through {@link PlainTextLine}: the file name is project-originated text.
 */
function formatGuidanceResultLine(applied: GuidanceApplied): string {
  const snapshot = applied.snapshotId ?? 'none'
  const result = applied.resultSha256Short ?? 'absent'
  return `${guidanceBlockLabel(applied.kind)} ${applied.file}: written ${formatGuidanceAction(applied.action)}, snapshot ${snapshot}, result ${result}`
}

/**
 * Whether Remove may be offered for this status: only an intact block the
 * shell wrote -- `current` or `outdated` -- and only when the file has a
 * short digest for the gate to name. There is nothing of Omnifrons' to
 * remove from an `absent` file, and a `modified` or `malformed` block is the
 * user's to resolve (or to restore from a snapshot), never Omnifrons' to
 * delete. The digest condition is the surface's own (slice 5c review,
 * R3-008): a `current` status with no short digest is a state the shell does
 * not produce, but were one to arrive, opening a block on it would show a
 * gate with nothing to type, no button that could ever be enabled and no
 * Cancel -- a dead end. It is refused at the offer instead.
 */
function isGuidanceBlockRemovable(status: GuidanceStatus): boolean {
  if (status.fileSha256Short === null) return false
  return status.managed === 'current' || status.managed === 'outdated'
}

/**
 * The proposed block's own lines, split *before* any of it reaches
 * {@link PlainTextLine} -- which strips every C0 control including `\n`, so
 * a newline passed through it would silently join two lines into one (the
 * same rule {@link splitTerminalLines} follows for framed terminal text). A
 * trailing newline ends the last line rather than opening an empty one; an
 * interior empty line is kept, since it is one; a `\r` of a CRLF file is
 * stripped as the control character it is.
 */
function splitProposedLines(proposed: string): string[] {
  if (proposed === '') return []
  const lines = proposed.split('\n')
  if (lines[lines.length - 1] === '') lines.pop()
  return lines
}

/** Whose short digest the write block's typed gate names. */
type GuidanceGateSubject = "file's" | "proposed block's" | "snapshot's"

/** The act the write block's typed gate stands in for. */
type GuidanceGateVerb = 'write' | 'create the file' | 'remove' | 'restore'

/**
 * The facts a pending guidance write is bound to, captured when the block
 * opens and never re-derived afterwards: the managed file as the shell
 * named it, the action, the full digest the request binds to (`null` for a
 * file the surface showed as absent) beside its short form, and the exact
 * value that must be typed before the final button does anything. The
 * request never carries the typed value -- it is a gate, standing in for
 * having read the evidence (TM-001-R1), exactly as the slice 2 approval
 * gate does.
 */
interface GuidanceWriteBase {
  kind: GuidanceKind
  file: string
  action: GuidanceAction
  fileSha256: string | null
  fileSha256Short: string | null
  gate: string
  gateSubject: GuidanceGateSubject
  gateVerb: GuidanceGateVerb
}

/**
 * One pending write, by the command its final button invokes: an apply
 * carries the proposed block it would write (display-only text, never
 * markup and never an instruction -- HAP-001-R22: no content path may
 * approve or trigger anything), a removal carries nothing extra, and a
 * restore carries the snapshot whose bytes it would write back.
 */
type GuidanceWrite =
  | (GuidanceWriteBase & { command: 'apply'; proposed: string })
  | (GuidanceWriteBase & { command: 'remove' })
  | (GuidanceWriteBase & { command: 'restore'; snapshot: Snapshot })

/** The final button's name, one per command through an exhaustive switch. */
function guidanceWriteButtonName(write: GuidanceWrite): string {
  switch (write.command) {
    case 'apply':
      return 'Write'
    case 'remove':
      return 'Remove block'
    case 'restore':
      return 'Restore snapshot'
  }
}

/**
 * Invokes the command this write stands for, with exactly the target the
 * block showed: the `file` key only for the `guidance` kind (the `ignore`
 * kind always manages `.gitignore` and takes no file), and the full digest
 * the surface bound to -- never anything the user typed.
 */
function invokeGuidanceWrite(write: GuidanceWrite): Promise<GuidanceApplied> {
  const file = write.kind === 'guidance' ? write.file : undefined
  switch (write.command) {
    case 'apply':
      return guidanceApply(write.kind, file, write.fileSha256)
    case 'remove':
      return guidanceRemove(write.kind, file, write.fileSha256)
    case 'restore':
      return guidanceRestore(write.snapshot.id, write.fileSha256)
  }
}

/**
 * Whether `input` confirms `write`: an exact, case-sensitive match against
 * the short digest the block shows, and nothing else -- not the fixed
 * phrase, not the full digest, not another kind's, and not the same digest
 * with the space a paste leaves on it. An empty gate confirms nothing; it
 * is kept as a last line, not as the defence -- a block with no digest to
 * type is refused at the offer instead ({@link isGuidanceBlockRemovable},
 * slice 5c review, R3-008), so it never opens.
 */
function isGuidanceConfirmed(write: GuidanceWrite, input: string): boolean {
  return write.gate !== '' && input === write.gate
}

/** One managed kind's fetched state: its status, and every snapshot recorded for it. */
interface ManagedFileState {
  status: GuidanceStatus | null
  snapshots: Snapshot[]
}

const EMPTY_MANAGED_FILE: ManagedFileState = { status: null, snapshots: [] }

const EMPTY_GUIDANCE: Record<GuidanceKind, ManagedFileState> = {
  guidance: EMPTY_MANAGED_FILE,
  ignore: EMPTY_MANAGED_FILE,
}

/**
 * Replaces one kind's state, written out per kind rather than with a
 * computed key so the record's type is preserved exactly.
 */
function withManagedFile(
  previous: Record<GuidanceKind, ManagedFileState>,
  kind: GuidanceKind,
  update: (state: ManagedFileState) => ManagedFileState,
): Record<GuidanceKind, ManagedFileState> {
  return kind === 'guidance'
    ? { ...previous, guidance: update(previous.guidance) }
    : { ...previous, ignore: update(previous.ignore) }
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

  /**
   * The active project's publications (`docs/spike-log.md` § Slice 5b):
   * every record `publications_list` returned after the shell's restart
   * replay, or `null` while unknown or while no workspace is active. A
   * publication this panel completes is merged in directly from
   * `artifact_publish`'s own response.
   */
  const [publications, setPublications] = useState<Publication[] | null>(null)

  /**
   * The sequence number of the most recently *started* `publications_list`
   * fetch -- the same sequenced-fetch pattern as `outbox_status` above
   * (R3-008): only the latest fetch's response is applied.
   */
  const latestPublicationsRequestRef = useRef(0)

  /**
   * Fetches the project's publications: on mount, and again after a
   * successful workspace pick (a new workspace is another project's
   * Catalog). A `workspace-unavailable` rejection is the shell's answer
   * while no workspace is active -- no table and no alert, as for the
   * outbox status; any other rejection reaches the banner.
   */
  const refreshPublications = useCallback(() => {
    const requestId = (latestPublicationsRequestRef.current += 1)
    publicationsList()
      .then((records) => {
        if (!mountedRef.current) return
        if (requestId !== latestPublicationsRequestRef.current) return
        setPublications(records)
      })
      .catch((listError: unknown) => {
        if (!mountedRef.current) return
        if (requestId !== latestPublicationsRequestRef.current) return
        if (isShellError(listError) && listError.code === 'workspace-unavailable') {
          setPublications(null)
          return
        }
        setError(isShellError(listError) ? listError : 'unexpected')
      })
  }, [])

  useEffect(() => {
    refreshPublications()
  }, [refreshPublications])

  /**
   * Each managed kind's state (slice 5c, HAP-001 D18): the guidance note
   * the user named at the workspace root and the fixed `.gitignore`, with
   * the snapshots recorded for each. `null` status means unknown -- no
   * workspace, or a fetch that did not answer -- and renders no status line.
   */
  const [managed, setManaged] = useState<Record<GuidanceKind, ManagedFileState>>(EMPTY_GUIDANCE)

  /**
   * The guidance file the section is currently about: the last name the
   * shell itself answered for, kept as a ref so the stable fetch callbacks
   * below never close over a stale name. It follows the shell's own answer
   * and never the typed draft (slice 5c review, R1-004/R3-005): a name the
   * shell refuses with `guidance-file-invalid` must not become the name the
   * automatic fetches reuse with `surfaceError: false`, where the same
   * rejection would be swallowed and the section's status line, Preview,
   * Remove and every Restore would silently vanish. The input's own draft is
   * `guidanceFileDraft` state; leaving the field or pressing Enter commits
   * it, and only an accepted commit moves this ref.
   */
  const guidanceFileRef = useRef(DEFAULT_GUIDANCE_FILE)
  const [guidanceFileDraft, setGuidanceFileDraft] = useState(DEFAULT_GUIDANCE_FILE)

  /**
   * The generation of the guidance section's context: the workspace it is
   * about and the file name it is about. Bumped by a workspace pick and by
   * a committed file name, the same generation guard `spawnGenerationRef`
   * puts on a run's continuations (slice 5c review, R1-001). A
   * `guidance_preview` that resolves after either has moved is dropped
   * rather than opening a write block bound to the previous project's file,
   * digest and gate -- facts the shell's own `fileSha256` binding cannot
   * tell apart when two projects share a byte-identical `AGENTS.md` from
   * one template.
   */
  const guidanceContextRef = useRef(0)

  /**
   * The sequence number of the most recently *started* `guidance_status`
   * and `guidance_snapshots` fetch, per kind -- the same sequenced-fetch
   * pattern as `outbox_status` above (R3-008), counted per kind so the two
   * kinds' fetches never invalidate each other. A response that is no
   * longer the latest for its kind is silently dropped.
   */
  const latestGuidanceStatusRef = useRef<Record<GuidanceKind, number>>({ guidance: 0, ignore: 0 })
  const latestGuidanceSnapshotsRef = useRef<Record<GuidanceKind, number>>({
    guidance: 0,
    ignore: 0,
  })

  /**
   * Fetches one kind's status. `surfaceError` separates the two callers:
   * an automatic refresh -- on mount, after a workspace pick, after a
   * write -- is not a user act, so a rejection only clears the line (no
   * workspace is the ordinary case, and the missing workspace is already
   * visible); a refresh the user asked for by committing a file name shows
   * the rule's own message in the banner and leaves the previous line
   * standing, since nothing about the previously shown file changed.
   */
  const fetchGuidanceStatus = useCallback(
    (kind: GuidanceKind, file: string | undefined, surfaceError: boolean) => {
      const requestId = (latestGuidanceStatusRef.current[kind] += 1)
      guidanceStatus(kind, file)
        .then((status) => {
          if (!mountedRef.current) return
          if (requestId !== latestGuidanceStatusRef.current[kind]) return
          // The committed name follows the shell's own answer, never the
          // typed draft (R1-004/R3-005): a refused name never gets this far,
          // so it can never become the name a later automatic fetch reuses.
          if (kind === 'guidance') guidanceFileRef.current = status.file
          setManaged((previous) => withManagedFile(previous, kind, (state) => ({ ...state, status })))
        })
        .catch((statusError: unknown) => {
          if (!mountedRef.current) return
          if (requestId !== latestGuidanceStatusRef.current[kind]) return
          if (!surfaceError) {
            setManaged((previous) =>
              withManagedFile(previous, kind, (state) => ({ ...state, status: null })),
            )
            return
          }
          setError(isShellError(statusError) ? statusError : 'unexpected')
        })
    },
    [],
  )

  /**
   * Fetches one kind's snapshots, newest first. Sequenced per kind like the
   * status fetch. Every caller is an automatic refresh -- the listing is
   * never a user act of its own -- so a rejection empties the table
   * silently rather than raising a banner the user cannot act on; a
   * command the user *did* ask for reports its own `snapshot-unavailable`.
   */
  const fetchGuidanceSnapshots = useCallback((kind: GuidanceKind) => {
    const requestId = (latestGuidanceSnapshotsRef.current[kind] += 1)
    guidanceSnapshots(kind)
      .then((snapshots) => {
        if (!mountedRef.current) return
        if (requestId !== latestGuidanceSnapshotsRef.current[kind]) return
        // A payload that is not the list the contract promises renders as
        // no table rather than throwing mid-render and taking the whole
        // panel down with it -- the same fail-safe-to-nothing discipline
        // `handleFrame` applies to a frame it does not recognize.
        const listed = Array.isArray(snapshots) ? snapshots : []
        setManaged((previous) =>
          withManagedFile(previous, kind, (state) => ({ ...state, snapshots: listed })),
        )
      })
      .catch(() => {
        if (!mountedRef.current) return
        if (requestId !== latestGuidanceSnapshotsRef.current[kind]) return
        setManaged((previous) => withManagedFile(previous, kind, (state) => ({ ...state, snapshots: [] })))
      })
  }, [])

  /**
   * Both kinds' status and snapshots: on mount, and again after a
   * successful workspace pick (the managed files are the project's, like
   * its outbox and its Catalog). The guidance kind is asked about the
   * committed file name; the ignore kind takes no file at all.
   */
  const refreshGuidance = useCallback(() => {
    fetchGuidanceStatus('guidance', guidanceFileRef.current, false)
    fetchGuidanceStatus('ignore', undefined, false)
    fetchGuidanceSnapshots('guidance')
    fetchGuidanceSnapshots('ignore')
  }, [fetchGuidanceStatus, fetchGuidanceSnapshots])

  useEffect(() => {
    refreshGuidance()
  }, [refreshGuidance])

  /** The pending guidance write the user is disposing of, or `null` while no block is open. */
  const [guidanceWrite, setGuidanceWrite] = useState<GuidanceWrite | null>(null)

  /**
   * The digest the user has typed into the guidance write block. Only ever
   * set from the input's own change events -- never from a proposal, a file
   * name, a candidate, or any other harness-originated string (TM-001-R1).
   */
  const [guidanceInput, setGuidanceInput] = useState('')

  /** The last completed write's result line, or `null` while none has completed since the section was reset. */
  const [guidanceResult, setGuidanceResult] = useState<string | null>(null)

  /** True while a guidance preview, apply, removal or restore is in flight (one at a time). */
  const [guidanceBusy, setGuidanceBusy] = useState(false)

  /**
   * The same flag as `guidanceBusy`, set synchronously so a second click
   * landing before React has re-rendered the disabled button is still
   * refused -- the write must happen once per confirmed gate, never twice.
   */
  const guidanceBusyRef = useRef(false)

  /**
   * True while a `guidance_pin` is in flight. Pin has its own flag rather
   * than sharing `guidanceBusy`, because it stays live while a run is active
   * and writes no project file -- but it is still one at a time (slice 5c
   * review, R3-013): a double-click must pin once, and with only one call in
   * flight no slow answer can land on top of a newer one. The ref is set
   * synchronously so the second click of a double-click is refused before
   * React has re-rendered the disabled button.
   */
  const guidancePinBusyRef = useRef(false)
  const [guidancePinBusy, setGuidancePinBusy] = useState(false)

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
   * `candidates` event named, with that run id, or `null` until then.
   * Cleared by the next Start along with the transcript, so a new run never
   * shows the previous run's inventory beside its own output.
   */
  const [candidates, setCandidates] = useState<CandidatesInventory | null>(null)

  /**
   * The sequence number of the most recently *started* `candidates_list`
   * fetch (R3-008 pattern): only the latest fetch's response is applied.
   */
  const latestCandidatesRequestRef = useRef(0)

  /**
   * Bumped every time a fetched inventory is applied (slice 5b): the
   * identity a pending `artifact_approve` or `artifact_publish` response
   * must still match before it touches a row -- a response that lands after
   * a new run's inventory replaced the rows is dropped, even when the new
   * inventory lists the same names (stale-response guard). A rejected Start
   * restores the same inventory, so a response landing then still applies.
   */
  const appliedInventoryRef = useRef(0)

  /**
   * Counts the `artifact-state` frames this panel has rendered (slice 5b):
   * the publish channel carries no sequence number, so each transcript
   * line is keyed by this counter instead.
   */
  const publicationLineSeqRef = useRef(0)

  /**
   * The adapter id the current generation's run was started with (slice 5b):
   * copied into the inventory when it is applied, so the approval block's
   * scope line follows the producing launch rather than the selection at
   * approval time.
   */
  const runAdapterIdRef = useRef<string | null>(null)

  /**
   * Each row's publication progress by candidate name (slice 5b), from the
   * approval on; reset with every applied inventory, so a new run's rows
   * never inherit an earlier run's approvals.
   */
  const [rowPublications, setRowPublications] =
    useState<ReadonlyMap<string, RowPublication>>(EMPTY_ROW_PUBLICATIONS)

  /** The name of the row whose approval block is open, or `null`. */
  const [approvingName, setApprovingName] = useState<string | null>(null)

  /**
   * The digest the user has typed into the approval block. Only ever set
   * from the input's own change events -- never from a candidate name, a
   * proposal, or any other harness-originated string (TM-001-R1).
   */
  const [approvalInput, setApprovalInput] = useState('')

  /** True while an `artifact_approve` call is in flight (one at a time). */
  const [isApproving, setIsApproving] = useState(false)

  /** True while a whole-outbox `candidates_list` is in flight (slice 5c). */
  const [isListingOutbox, setIsListingOutbox] = useState(false)

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
            // A new inventory: the rows, the run id an approval request
            // names, and a clean slate for the rows' publication progress
            // and the approval block (slice 5b).
            appliedInventoryRef.current += 1
            setCandidates({ runId, rows, adapterId: runAdapterIdRef.current })
            setRowPublications(EMPTY_ROW_PUBLICATIONS)
            setApprovingName(null)
            setApprovalInput('')
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
      // status, fetched afresh rather than carried over -- and the Catalog
      // is the project's too, so its publications are fetched afresh.
      refreshOutboxStatus()
      refreshPublications()
      // The managed files are the project's as well: the open write block
      // and the last result belong to the project that was active when
      // they were made, so the section is reset before both kinds are
      // fetched afresh (slice 5c). The context generation bumps with the
      // reset, so a preview still in flight for the previous project opens
      // no block over the new one (R1-001).
      guidanceContextRef.current += 1
      setGuidanceWrite(null)
      setGuidanceInput('')
      setGuidanceResult(null)
      refreshGuidance()
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
    runAdapterIdRef.current = adapterId
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

  /**
   * Lists the whole outbox (slice 5c; HAP-001-R11, R12): every entry at the
   * outbox root and one level down, as `candidates_list {}` inventories it
   * without a run -- unattributed, a proposal only -- applied as the
   * candidates table with no run id, so an approval made from it names no
   * run. Sequenced like the run-end fetch and gated on the spawn
   * generation, so a response landing after a Start -- which cleared the
   * table for the new run -- is dropped rather than dressing that run in an
   * unrelated listing. Frozen while a run is active, belt and braces with
   * the button's own `disabled`.
   */
  async function handleListOutbox() {
    if (runActive || isListingOutbox) return
    const generation = spawnGenerationRef.current
    const requestId = (latestCandidatesRequestRef.current += 1)
    setError(null)
    setIsListingOutbox(true)
    try {
      const rows = await candidatesList()
      if (!mountedRef.current) return
      if (generation !== spawnGenerationRef.current) return
      if (requestId !== latestCandidatesRequestRef.current) return
      // A new inventory, like a run's: a clean slate for the rows'
      // publication progress and the approval block. A payload that is not
      // the list the contract promises becomes no rows rather than throwing
      // mid-render and taking the whole panel down with it (slice 5c review,
      // R1-007) -- the fail-safe-to-nothing discipline `handleFrame` and
      // `fetchGuidanceSnapshots` already apply.
      appliedInventoryRef.current += 1
      setCandidates({ runId: null, rows: Array.isArray(rows) ? rows : [], adapterId: null })
      setRowPublications(EMPTY_ROW_PUBLICATIONS)
      setApprovingName(null)
      setApprovalInput('')
    } catch (listError: unknown) {
      if (!mountedRef.current) return
      if (generation !== spawnGenerationRef.current) return
      if (requestId !== latestCandidatesRequestRef.current) return
      setError(isShellError(listError) ? listError : 'unexpected')
    } finally {
      if (mountedRef.current) setIsListingOutbox(false)
    }
  }

  /**
   * The target one guidance command names: the committed file for the
   * `guidance` kind, nothing at all for `ignore` -- which always manages
   * `.gitignore` and whose request crosses IPC as exactly `{ kind }`.
   */
  function guidanceRequestFile(kind: GuidanceKind, file: string): string | undefined {
    return kind === 'guidance' ? file : undefined
  }

  /**
   * Commits the guidance file name the user typed (leaving the field, or
   * Enter): asks the shell about that file and shows what it answers. A
   * name the shell refuses reaches the banner with the rule's own message
   * -- never the name -- and leaves the previously shown file's status
   * standing; `guidanceFileRef` moves only when the shell answers, so a
   * refusal cannot poison the automatic fetches (R1-004/R3-005). An open
   * write block is bound to the file it was opened for, so a change of file
   * closes it rather than letting it write to a name whose facts are no
   * longer on screen -- and the context generation bumps with it, so a
   * preview still in flight for the previous name opens nothing either
   * (R1-001). The last write's receipt goes too: it names a file, and that
   * file is no longer the one the section is about (R1-003/R3-007).
   */
  function commitGuidanceFile() {
    const name = guidanceFileDraft
    if (name === guidanceFileRef.current) return
    guidanceContextRef.current += 1
    setGuidanceWrite(null)
    setGuidanceInput('')
    setGuidanceResult(null)
    setError(null)
    fetchGuidanceStatus('guidance', name, true)
  }

  /**
   * The proposal (HAP-001 D18: the system proposes, the user disposes):
   * asks the shell what an apply would write and opens the write block on
   * the answer. Read-only -- nothing is written or snapshotted -- and the
   * block it opens invokes nothing until the typed gate confirms. The gate
   * is the file's own short digest when the file exists, and the proposed
   * block's own when it does not (there is no file digest to have read).
   * Frozen while a run is active, belt and braces with the button's
   * `disabled`.
   *
   * The answer is applied only while the section is still about the same
   * workspace and the same file (R1-001): a preview in flight when the user
   * picks a workspace or commits another name resolves after the section has
   * been reset, and re-opening the block then would bind a write to the
   * previous project's file, digest and gate inside the new project's
   * section. The rejection path is dropped the same way -- a refusal that
   * belongs to a context the user has left names no act to retry.
   */
  async function handleGuidancePreview(kind: GuidanceKind) {
    if (runActive || guidanceBusyRef.current) return
    const status = managed[kind].status
    if (status === null) return

    const context = guidanceContextRef.current
    guidanceBusyRef.current = true
    setGuidanceBusy(true)
    setError(null)
    try {
      const preview = await guidancePreview(kind, guidanceRequestFile(kind, status.file))
      if (!mountedRef.current) return
      if (context !== guidanceContextRef.current) return
      // The file's own short digest is the gate whenever the file exists;
      // for a file that does not, there is no such digest to have read, so
      // the proposal's own result digest stands in for it.
      const short = preview.fileSha256Short
      setGuidanceWrite({
        command: 'apply',
        kind,
        file: preview.file,
        action: preview.action,
        proposed: preview.proposed,
        fileSha256: preview.fileSha256,
        fileSha256Short: short,
        gate: short ?? preview.resultSha256Short,
        gateSubject: short === null ? "proposed block's" : "file's",
        gateVerb: short === null ? 'create the file' : 'write',
      })
      setGuidanceInput('')
      setGuidanceResult(null)
    } catch (previewError: unknown) {
      if (!mountedRef.current) return
      if (context !== guidanceContextRef.current) return
      setGuidanceWrite(null)
      setError(isShellError(previewError) ? previewError : 'unexpected')
    } finally {
      guidanceBusyRef.current = false
      if (mountedRef.current) setGuidanceBusy(false)
    }
  }

  /**
   * Opens the removal block for `kind`, bound to the status the section
   * already shows -- the file, its full digest, its short form as the gate.
   * Invokes nothing: a removal has no proposal to fetch, since what it
   * removes is the block the status line already reports.
   */
  function openGuidanceRemove(kind: GuidanceKind) {
    if (runActive || guidanceBusyRef.current) return
    const status = managed[kind].status
    if (status === null || !isGuidanceBlockRemovable(status)) return
    setGuidanceWrite({
      command: 'remove',
      kind,
      file: status.file,
      action: 'remove',
      fileSha256: status.fileSha256,
      fileSha256Short: status.fileSha256Short,
      gate: status.fileSha256Short ?? '',
      gateSubject: "file's",
      gateVerb: 'remove',
    })
    setGuidanceInput('')
    setGuidanceResult(null)
  }

  /**
   * Opens the restore block for one snapshot, bound to the current state of
   * the file it recorded: the gate is that file's short digest, so the act
   * stands in for having read what is about to be overwritten -- or, for a
   * file the surface shows as absent, the snapshot's own short digest,
   * since there is no current file to have read. Only a snapshot of the
   * file the status line shows -- and of this very kind (R1-006) -- is
   * restorable, and it invokes nothing until the gate confirms.
   */
  function openGuidanceRestore(kind: GuidanceKind, snapshot: Snapshot) {
    if (runActive || guidanceBusyRef.current) return
    const status = managed[kind].status
    if (status === null || snapshot.kind !== kind || snapshot.file !== status.file) return
    const short = status.fileSha256Short
    setGuidanceWrite({
      command: 'restore',
      kind,
      file: status.file,
      action: 'restore',
      snapshot,
      fileSha256: status.fileSha256,
      fileSha256Short: short,
      gate: short ?? snapshot.sha256Short,
      gateSubject: short === null ? "snapshot's" : "file's",
      gateVerb: 'restore',
    })
    setGuidanceInput('')
    setGuidanceResult(null)
  }

  /**
   * The write itself (HAP-001 D18, R42; TM-001-R1/R7): only the user's
   * click on the final button reaches here, and only once the typed short
   * digest confirms the block -- re-checked here rather than trusted to the
   * button's `disabled`. The request carries the digest the shell itself
   * reported, never the typed value; the shell verifies it against the file
   * on disk and refuses with `guidance-file-changed` if it moved.
   *
   * A refusal keeps the block open with the typed gate intact, so the user
   * can read the banner and try again -- except `guidance-file-changed`,
   * where the facts the block was bound to are stale by definition: the
   * block closes and the section is fetched afresh.
   *
   * Both continuations carry the same context guard the preview does
   * (R1-001): a write in flight when the user picks a workspace or commits
   * another file name has already reached the shell for the project it was
   * bound to, but its receipt names a file, and posting it under the section
   * the user has moved to would read as a write into *that* project.
   */
  async function handleGuidanceWrite() {
    if (runActive || guidanceBusyRef.current) return
    const write = guidanceWrite
    if (write === null) return
    if (!isGuidanceConfirmed(write, guidanceInput)) return

    const context = guidanceContextRef.current
    guidanceBusyRef.current = true
    setGuidanceBusy(true)
    setError(null)
    try {
      const applied = await invokeGuidanceWrite(write)
      if (!mountedRef.current) return
      if (context !== guidanceContextRef.current) return
      setGuidanceWrite(null)
      setGuidanceInput('')
      setGuidanceResult(formatGuidanceResultLine(applied))
      // The file and the store both moved: this kind's status and
      // snapshots are fetched afresh, and only this kind's.
      fetchGuidanceStatus(write.kind, guidanceRequestFile(write.kind, write.file), false)
      fetchGuidanceSnapshots(write.kind)
    } catch (writeError: unknown) {
      if (!mountedRef.current) return
      if (context !== guidanceContextRef.current) return
      setError(isShellError(writeError) ? writeError : 'unexpected')
      if (isShellError(writeError) && writeError.code === 'guidance-file-changed') {
        setGuidanceWrite(null)
        setGuidanceInput('')
        fetchGuidanceStatus(write.kind, guidanceRequestFile(write.kind, write.file), false)
        fetchGuidanceSnapshots(write.kind)
      }
    } finally {
      guidanceBusyRef.current = false
      if (mountedRef.current) setGuidanceBusy(false)
    }
  }

  /**
   * Pins or unpins one snapshot: a pinned snapshot is never pruned. The
   * store's own answer replaces the row, and the kind's status is fetched
   * afresh for its pinned count -- the listing is not, so the answer is not
   * immediately overwritten by a re-read of the same rows. Unlike every
   * other guidance command this one writes no project file, so it stays
   * live while a run is active, as the shell's own `guidance_pin` does --
   * but only one at a time (R3-013), so a double-click pins once and no
   * slow answer lands on top of a newer one.
   */
  async function handleGuidancePin(kind: GuidanceKind, snapshot: Snapshot) {
    if (guidancePinBusyRef.current) return
    if (snapshot.kind !== kind) return
    guidancePinBusyRef.current = true
    setGuidancePinBusy(true)
    setError(null)
    try {
      const updated = await guidancePin(snapshot.id, !snapshot.pinned)
      if (!mountedRef.current) return
      setManaged((previous) =>
        withManagedFile(previous, kind, (state) => ({
          ...state,
          snapshots: state.snapshots.map((recorded) =>
            recorded.id === updated.id ? updated : recorded,
          ),
        })),
      )
      fetchGuidanceStatus(kind, guidanceRequestFile(kind, guidanceFileRef.current), false)
    } catch (pinError: unknown) {
      if (!mountedRef.current) return
      setError(isShellError(pinError) ? pinError : 'unexpected')
    } finally {
      guidancePinBusyRef.current = false
      if (mountedRef.current) setGuidancePinBusy(false)
    }
  }

  /**
   * Opens the approval block for the row named `name` (slice 5b): the
   * block shows that row's identity-bound facts and an empty input --
   * nothing pre-fills it. Frozen while a run is active or an approve is in
   * flight, belt and braces with the buttons' own `disabled`.
   */
  function openApproval(name: string) {
    if (runActive || isApproving) return
    setApprovingName(name)
    setApprovalInput('')
  }

  /**
   * The approval itself (HAP-001-R22; TM-001-R1/R7): only the user's click
   * on the final button reaches here, and only once the typed short digest
   * confirms the row (`isApprovalConfirmed`) -- re-checked here rather than
   * trusted to the button's `disabled`. Calls `artifact_approve` with the
   * inventory's run id, the row's name, and the row's own full `sha256` --
   * the identity fact the shell listed, never the typed value and never a
   * proposal's digest; the shell verifies it against its own inventory.
   * Both continuations are dropped once unmounted or once a new inventory
   * replaced the rows, and the banner also holds its silence for a run that
   * has since been replaced.
   */
  async function handleConfirmApproval() {
    if (runActive || isApproving) return
    if (candidates === null || approvingName === null) return
    const row = candidates.rows.find((candidate) => candidate.name === approvingName)
    if (row === undefined || row.sha256 === null) return
    if (!isApprovalConfirmed(row, approvalInput)) return
    // No approval toward a destination the surface could not show
    // (HAP-001-R22): the shell would refuse it as `destination-invalid`.
    if (destinationAssetRootId === null) return

    const { runId } = candidates
    const name = row.name
    const sha256 = row.sha256
    const inventory = appliedInventoryRef.current
    const generation = spawnGenerationRef.current
    setError(null)
    setIsApproving(true)
    try {
      const approval = await artifactApprove(runId, name, sha256)
      if (!mountedRef.current) return
      if (inventory !== appliedInventoryRef.current) return
      setRowPublications((previous) => new Map(previous).set(name, { phase: 'approved', approval }))
      setApprovingName(null)
      setApprovalInput('')
    } catch (approveError: unknown) {
      if (!mountedRef.current) return
      if (inventory !== appliedInventoryRef.current) return
      if (generation !== spawnGenerationRef.current) return
      setError(isShellError(approveError) ? approveError : 'unexpected')
    } finally {
      if (mountedRef.current) setIsApproving(false)
    }
  }

  /**
   * Publishes an approved row (slice 5b; HAP-001-R35): calls
   * `artifact_publish` with the approval id and a channel, renders every
   * `artifact-state` frame as a transcript line, and applies the resolved
   * publication -- to the Publications table always (the project's own
   * truth, unless a `publications_list` refresh started since, whose own
   * response then defines the table), and to the row while the same
   * inventory is still displayed. Transcript lines and the banner are held
   * to the run generation the click happened under, so a new run's
   * transcript never receives an earlier publication's lines. A failure
   * returns the row to `approved` with Publish enabled again: the shell
   * decides whether the same approval can be retried.
   */
  async function handlePublish(name: string) {
    if (runActive) return
    const progress = rowPublications.get(name)
    if (progress === undefined || progress.phase !== 'approved') return

    const { approval } = progress
    const inventory = appliedInventoryRef.current
    const generation = spawnGenerationRef.current
    const publicationsRequestId = latestPublicationsRequestRef.current
    setError(null)
    setRowPublications((previous) => new Map(previous).set(name, { phase: 'publishing', approval }))
    try {
      const publication = await artifactPublish(approval.approvalId, (frame) => {
        if (!mountedRef.current) return
        if (generation !== spawnGenerationRef.current) return
        const seq = (publicationLineSeqRef.current += 1)
        setTranscript((previous) =>
          appendTranscriptEntry(previous, {
            type: 'publication-state',
            key: `publication-${seq}`,
            text: formatPublicationStateLine(frame.payload),
          }),
        )
      })
      if (!mountedRef.current) return
      if (publicationsRequestId === latestPublicationsRequestRef.current) {
        setPublications((previous) => [
          ...(previous ?? []).filter(
            (existing) => existing.publicationId !== publication.publicationId,
          ),
          publication,
        ])
      }
      if (inventory !== appliedInventoryRef.current) return
      setRowPublications((previous) =>
        new Map(previous).set(name, { phase: 'published', publication }),
      )
    } catch (publishError: unknown) {
      if (!mountedRef.current) return
      if (inventory === appliedInventoryRef.current) {
        setRowPublications((previous) => {
          const current = previous.get(name)
          return current?.phase === 'publishing'
            ? new Map(previous).set(name, { phase: 'approved', approval: current.approval })
            : previous
        })
      }
      if (generation !== spawnGenerationRef.current) return
      setError(isShellError(publishError) ? publishError : 'unexpected')
    }
  }

  /** The row whose approval block is open, if it is still listed. */
  const approvingCandidate =
    candidates !== null && approvingName !== null
      ? (candidates.rows.find((candidate) => candidate.name === approvingName) ?? null)
      : null
  const approvalConfirmed =
    approvingCandidate !== null && isApprovalConfirmed(approvingCandidate, approvalInput)

  /** The asset root an approval would publish to, or `null` while unknown or unconfigured. */
  const destinationAssetRootId = outbox?.assetRootId ?? null

  /** The descriptor of the adapter the displayed inventory's run was started with, if listed. */
  const inventoryAdapter =
    candidates === null || candidates.adapterId === null
      ? null
      : (adapters.find((adapter) => adapter.id === candidates.adapterId) ?? null)

  /** The work area's status line, or `null` while no workspace is active or its work area is valid. */
  const workAreaLine = workspace === null ? null : formatWorkAreaLine(workspace.workArea)

  /** Whether the typed gate confirms the open guidance write block. */
  const guidanceConfirmed =
    guidanceWrite !== null && isGuidanceConfirmed(guidanceWrite, guidanceInput)

  /**
   * One managed kind's block (slice 5c): its own header -- the editable
   * file name for the guidance note, the fixed `.gitignore` line for the
   * ignore rule -- the status line, Preview and Remove, and the Snapshots
   * table once the kind has any. Every fact renders through
   * {@link PlainTextLine}; nothing here writes anything until the write
   * block's typed gate confirms it.
   */
  function renderGuidanceBlock(kind: GuidanceKind, header: ReactNode): ReactNode {
    const { status, snapshots } = managed[kind]
    const label = guidanceBlockLabel(kind)
    const frozen = runActive || guidanceBusy
    return (
      <div aria-label={label}>
        {header}
        {status && (
          <p role="status" aria-label={`${label} status`}>
            <PlainTextLine text={formatGuidanceStatusLine(status)} />
          </p>
        )}
        <button
          type="button"
          onClick={() => handleGuidancePreview(kind)}
          disabled={status === null || frozen}
        >
          Preview
        </button>
        <button
          type="button"
          onClick={() => openGuidanceRemove(kind)}
          disabled={status === null || !isGuidanceBlockRemovable(status) || frozen}
        >
          Remove
        </button>
        {snapshots.length > 0 && (
          <table aria-label={`${label} snapshots`}>
            <thead>
              <tr>
                <th>file</th>
                <th>id</th>
                <th>taken at</th>
                <th>sha256 short</th>
                <th>size</th>
                <th>existed</th>
                <th>pinned</th>
                <th>action</th>
              </tr>
            </thead>
            <tbody>
              {snapshots.map((snapshot) => (
                <tr key={snapshot.id}>
                  <td>
                    <PlainTextLine text={snapshot.file} />
                  </td>
                  <td>
                    <PlainTextLine text={snapshot.id} />
                  </td>
                  <td>
                    <PlainTextLine text={formatSnapshotInstant(snapshot.takenAt)} />
                  </td>
                  <td>
                    <PlainTextLine text={snapshot.sha256Short} />
                  </td>
                  <td>
                    <PlainTextLine text={String(snapshot.size)} />
                  </td>
                  <td>
                    <PlainTextLine text={formatYesNo(snapshot.existed)} />
                  </td>
                  <td>
                    <PlainTextLine text={formatYesNo(snapshot.pinned)} />
                  </td>
                  <td>
                    {/*
                      Every action here is offered only for a snapshot of
                      this very kind (slice 5c review, R1-006). The two file
                      spaces cannot collide today -- the shell lists each
                      kind's snapshots separately -- so this is defence in
                      depth on a surface whose whole point is that the
                      block's facts bind the write.
                    */}
                    {snapshot.kind === kind && (
                      <button
                        type="button"
                        onClick={() => handleGuidancePin(kind, snapshot)}
                        disabled={guidancePinBusy}
                      >
                        {snapshot.pinned ? 'Unpin' : 'Pin'}
                      </button>
                    )}
                    {/*
                      A snapshot of another file is listed -- it is this
                      kind's history -- but restoring it would write a file
                      whose current state the block cannot show, so it is
                      offered only once that file is the one the status line
                      reports.
                    */}
                    {snapshot.kind === kind && status !== null && snapshot.file === status.file && (
                      <>
                        {' '}
                        <button
                          type="button"
                          onClick={() => openGuidanceRestore(kind, snapshot)}
                          disabled={frozen}
                        >
                          Restore
                        </button>
                      </>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    )
  }

  /**
   * A row's action cell (slice 5b): Approve for an approvable row with no
   * progress yet; once approved, the approval's facts and Publish; once
   * published, the transaction's own state token and the publication's
   * short id; nothing for a row the shell would refuse. Every button is
   * frozen while a run is active (publication of a finished run's
   * candidates is allowed only when no run is active -- spike default).
   */
  function renderRowAction(candidate: Candidate): ReactNode {
    const progress = rowPublications.get(candidate.name)
    if (progress === undefined) {
      if (!isApprovable(candidate)) return null
      return (
        <button
          type="button"
          onClick={() => openApproval(candidate.name)}
          disabled={runActive || isApproving}
        >
          Approve
        </button>
      )
    }
    if (progress.phase === 'published') {
      return (
        <PlainTextLine
          text={`${formatArtifactState(progress.publication.state)} ${shortDigest(progress.publication.publicationId)}`}
        />
      )
    }
    return (
      <>
        <PlainTextLine text={formatApprovedCell(progress.approval)} />
        {!progress.approval.handleHeld && (
          <>
            {' '}
            <mark>{HANDLE_NOT_HELD_MARK}</mark>
          </>
        )}{' '}
        <button
          type="button"
          onClick={() => handlePublish(candidate.name)}
          disabled={runActive || progress.phase === 'publishing'}
        >
          Publish
        </button>
      </>
    )
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
              {error.code === 'duplicate-publication' && error.detail && (
                // HAP-001-R23's acknowledgement of the existing record: its
                // two identities in short form, and nothing else of the
                // detail (slice 5b).
                <span>
                  {' '}
                  (publication <PlainTextLine text={shortDigest(error.detail.publicationId)} />,
                  catalog <PlainTextLine text={shortCatalogId(error.detail.catalogId)} />)
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
        {workAreaLine !== null && (
          // The work area's own status line (slice 5b, HAP-001-R7), a third
          // named live region beside the outbox line: fixed copy, rendered
          // only while the check reports the area invalid.
          <p role="status" aria-label="Work area">
            {workAreaLine}
          </p>
        )}
        {/*
          The whole-outbox inventory (slice 5c): what unmediated producers
          left at the outbox root or under a run subdirectory the shell no
          longer remembers, listed without a run and approvable as
          unattributed. Live only over an active workspace and never during
          a run, since Start clears the table for the run's own inventory.
        */}
        <button
          type="button"
          onClick={handleListOutbox}
          disabled={runActive || workspace === null || isListingOutbox}
        >
          List outbox
        </button>
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
                <th>action</th>
              </tr>
            </thead>
            <tbody>
              {candidates.rows.map((candidate, index) => (
                // An index key is sound here: the list is replaced whole by
                // each fetch and is never reordered or edited in place.
                <CandidateRow
                  key={index}
                  candidate={candidate}
                  action={renderRowAction(candidate)}
                />
              ))}
            </tbody>
          </table>

          {approvingCandidate && (
            // The publication approval surface (HAP-001-R22; TM-001-R1/R7):
            // the row's identity-bound facts verbatim, each through
            // PlainTextLine, the act-as identity as fixed copy, and a
            // digest-typed confirmation the user fills -- beside the table,
            // outside the transcript, never inside a terminal pane
            // (RCS-001-R6). The destination asset root is not known before
            // the shell answers, so it is shown in the row once approved.
            <div aria-label="Publication approval" data-testid="publication-approval">
              <p>
                name: <PlainTextLine text={approvingCandidate.name} />
              </p>
              <p>
                class: <PlainTextLine text={approvingCandidate.class ?? NULL_FACT} />
              </p>
              <p>
                type: <PlainTextLine text={approvingCandidate.detectedType ?? NULL_FACT} />
              </p>
              <p>
                size:{' '}
                <PlainTextLine
                  text={
                    approvingCandidate.size === null ? NULL_FACT : String(approvingCandidate.size)
                  }
                />
              </p>
              <p>
                sha256: <PlainTextLine text={approvingCandidate.sha256 ?? NULL_FACT} />
              </p>
              <p>
                sha256 short: <PlainTextLine text={approvingCandidate.sha256Short ?? NULL_FACT} />
              </p>
              <p>
                attribution:{' '}
                <PlainTextLine text={formatAttributionFact(approvingCandidate.attribution)} />
              </p>
              <p>
                destination: <PlainTextLine text={formatDestinationFact(outbox)} />
              </p>
              <p>
                scope: <PlainTextLine text={formatScopeFact(candidates, inventoryAdapter)} />
              </p>
              <p>{APPROVAL_ACT_AS_LINE}</p>
              <label htmlFor="agent-approval-confirm-input">
                Type the short digest (
                <PlainTextLine text={approvingCandidate.sha256Short ?? NULL_FACT} />) to approve
              </label>
              <input
                id="agent-approval-confirm-input"
                value={approvalInput}
                disabled={runActive || isApproving}
                onChange={(event) => setApprovalInput(event.target.value)}
              />
              <button
                type="button"
                onClick={handleConfirmApproval}
                disabled={
                  !approvalConfirmed || destinationAssetRootId === null || runActive || isApproving
                }
              >
                Approve publication
              </button>
            </div>
          )}
        </section>
      )}

      {/*
        The guidance-note installer (slice 5c; HAP-001 D18, R42): a sibling
        of the transcript, the candidates table and the Publications table,
        never inside any of them (RCS-001-R6). The system proposes -- a
        preview of the exact block it would write -- and the user disposes,
        through a typed short-digest gate over the file's own digest. The
        proposed block is display-only text throughout: nothing here reads
        it as an instruction and nothing renders it as markup, so no content
        path can approve or trigger anything (HAP-001-R22, TM-001-R1).
      */}
      <section aria-label="Guidance">
        <p>{GUIDANCE_ADVISORY_LINE}</p>

        {renderGuidanceBlock(
          'guidance',
          <>
            <label htmlFor="agent-guidance-file">Guidance file</label>
            <input
              id="agent-guidance-file"
              value={guidanceFileDraft}
              onChange={(event) => setGuidanceFileDraft(event.target.value)}
              onBlur={commitGuidanceFile}
              onKeyDown={(event) => {
                if (event.key === 'Enter') commitGuidanceFile()
              }}
            />
          </>,
        )}

        {renderGuidanceBlock('ignore', <p>{`Ignore file: ${IGNORE_FILE}`}</p>)}

        {guidanceWrite && (
          // The write surface: the facts the request is bound to, the
          // proposed block one line per text line, the reversibility
          // sentence, the act-as identity, and a digest-typed confirmation
          // the user fills. Nothing here is pre-filled from any
          // harness-originated value (TM-001-R1), and the request carries
          // the shell's own digest, never the typed one.
          <div aria-label="Guidance write" data-testid="guidance-write">
            <p>
              file: <PlainTextLine text={guidanceWrite.file} />
            </p>
            <p>
              action: <PlainTextLine text={formatGuidanceAction(guidanceWrite.action)} />
            </p>
            {guidanceWrite.command === 'apply' && (
              <>
                <p>proposed:</p>
                <div data-testid="guidance-proposed">
                  {splitProposedLines(guidanceWrite.proposed).map((line, index) => (
                    // An index key is sound here: the list is derived once
                    // from an immutable proposal and is never reordered or
                    // edited.
                    <div key={index} data-testid="guidance-proposed-line">
                      <PlainTextLine text={line} />
                    </div>
                  ))}
                </div>
              </>
            )}
            {guidanceWrite.command === 'remove' && <p>{GUIDANCE_REMOVE_SCOPE_LINE}</p>}
            {guidanceWrite.command === 'restore' && (
              // The snapshot's recorded facts, then the two disclosures the
              // facts alone do not carry (R1-002): that its bytes are never
              // shown, and -- when it recorded no file -- that restoring it
              // deletes rather than writes. Both stand before the gate.
              <>
                <p>
                  <PlainTextLine text={formatSnapshotFact(guidanceWrite.snapshot)} />
                </p>
                <p>{GUIDANCE_RESTORE_BYTES_LINE}</p>
                {!guidanceWrite.snapshot.existed && <p>{GUIDANCE_RESTORE_REMOVES_LINE}</p>}
              </>
            )}
            {guidanceWrite.fileSha256Short === null ? (
              <p>file: absent</p>
            ) : (
              <p>
                file sha256 short: <PlainTextLine text={guidanceWrite.fileSha256Short} />
              </p>
            )}
            <p>{GUIDANCE_SNAPSHOT_SENTENCE}</p>
            <p>{APPROVAL_ACT_AS_LINE}</p>
            <label htmlFor="agent-guidance-gate">
              {`Type the ${guidanceWrite.gateSubject} short digest (`}
              <PlainTextLine text={guidanceWrite.gate} />
              {`) to ${guidanceWrite.gateVerb}`}
            </label>
            <input
              id="agent-guidance-gate"
              value={guidanceInput}
              disabled={runActive || guidanceBusy}
              onChange={(event) => setGuidanceInput(event.target.value)}
            />
            <button
              type="button"
              onClick={handleGuidanceWrite}
              disabled={!guidanceConfirmed || runActive || guidanceBusy}
            >
              {guidanceWriteButtonName(guidanceWrite)}
            </button>
          </div>
        )}

        {guidanceResult !== null && (
          <p role="status" aria-label="Guidance result">
            <PlainTextLine text={guidanceResult} />
          </p>
        )}
      </section>

      {publications && (
        // The project's Catalog as this device sees it (`docs/spike-log.md`
        // § Slice 5b): beside the transcript and the candidates table, never
        // inside either. Rendered once a list was received, empty or not.
        <section aria-label="Publications">
          <table>
            <thead>
              <tr>
                <th>names</th>
                <th>state</th>
                <th>provider state</th>
                <th>reference</th>
                <th>catalog id</th>
                <th>availability</th>
              </tr>
            </thead>
            <tbody>
              {publications.map((publication) => (
                <PublicationRow key={publication.publicationId} publication={publication} />
              ))}
            </tbody>
          </table>
        </section>
      )}
    </section>
  )
}

export default AgentPanel
