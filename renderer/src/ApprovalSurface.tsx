import { useCallback, useEffect, useRef, useState } from 'react'

import {
  approvalsList,
  executableApprove,
  executablePickAndProbe,
  executableRevoke,
  isShellError,
  type Approval,
  type ApprovalId,
  type PlatformEvidence,
  type ProbeResult,
  type ShellError,
} from './ipc/harness'
import { PlainTextLine } from './PlainTextLine'

/**
 * The banner's error state: a typed `ShellError`, or the sentinel
 * `'unexpected'` for any rejection that isn't `ShellError`-shaped. A
 * non-`ShellError` rejection's `message` is never rendered, matching
 * `HarnessPanel`'s own banner (`docs/spike-log.md` § Slice 1, R3-012).
 */
type PanelError = ShellError | 'unexpected'

function formatModifiedAt(modifiedAt: number | null): string {
  return modifiedAt === null ? 'unreported' : new Date(modifiedAt).toISOString()
}

function formatPlatform(platform: PlatformEvidence): string {
  if (platform.os === 'unix') {
    return `unix (mode ${platform.mode.toString(8)})`
  }
  return `windows (extension ${platform.extension}, attributes ${platform.attributes})`
}

/**
 * Executable approval: a standalone surface for picking, inspecting, and
 * approving (or revoking) a caller-supplied executable's identity.
 *
 * Deliberately never mounted inside `HarnessPanel`'s output log, or any
 * other terminal/output pane -- RCS-001-R6 ("An approval MUST NOT be
 * rendered inside a terminal pane") and `docs/renderer-content-security.md`'s
 * guarantee that no rendered content appears inside an approval surface.
 * `App.tsx` mounts this as a sibling of `HarnessPanel`, never a child.
 *
 * Every evidence field renders through {@link PlainTextLine} -- plain text
 * only, matching slice 1's own no-markup-sink guarantee -- and approval is
 * a distinct, deliberate act: the user must type the candidate's own short
 * digest, exactly, into a text field before the Approve button is enabled
 * at all -- no fixed phrase accepted in its place (R1-001), so the act
 * cannot be completed without having read the evidence it confirms
 * (TM-001-R7: identity evidence display, a human decision grounded in
 * what will actually run).
 */
export function ApprovalSurface() {
  const [approvals, setApprovals] = useState<Approval[]>([])
  const [candidate, setCandidate] = useState<ProbeResult | null>(null)
  const [confirmInput, setConfirmInput] = useState('')
  const [error, setError] = useState<PanelError | null>(null)
  /**
   * A quiet, non-error status for the picker being cancelled -- distinct
   * from `error`, which is reserved for outcomes that actually need the
   * user's attention (R3-001). Cleared at the start of every new pick
   * attempt.
   */
  const [pickStatus, setPickStatus] = useState<string | null>(null)
  const [isPicking, setIsPicking] = useState(false)
  const [isApproving, setIsApproving] = useState(false)
  const [isRenewing, setIsRenewing] = useState(false)
  /** Approval ids with a `handleRevoke` call in flight (R3-002). */
  const [revokingIds, setRevokingIds] = useState<ApprovalId[]>([])

  /** Guards every async continuation below against a post-unmount update. */
  const mountedRef = useRef(true)
  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  const reportError = useCallback((thrown: unknown) => {
    if (!mountedRef.current) return
    setError(isShellError(thrown) ? thrown : 'unexpected')
  }, [])

  /**
   * Routes a rejection from `executablePickAndProbe` specifically. A
   * `no-candidate` rejection there means the OS file dialog returned
   * nothing -- the user cancelled it -- a normal, expected outcome, not
   * an error: it renders as a quiet status line instead of the error
   * banner (R3-001). `executable_pick_and_probe` and `executable_approve`
   * currently share the one `no-candidate` code for two different
   * situations (an empty dialog result here vs. a stale/unknown
   * candidate id there, see `src-tauri/src/ipc/commands.rs`); a distinct
   * `pick-cancelled` code that separates the two at the type level,
   * rather than by which call produced it, is a Rust-side follow-up
   * (`docs/spike-log.md` § Slice 2 Renderer). Every other rejection --
   * from this call or from `executableApprove`/`executableRevoke` -- is
   * always a real error and always renders through the banner.
   */
  const reportPickRejection = useCallback(
    (thrown: unknown) => {
      if (isShellError(thrown) && thrown.code === 'no-candidate') {
        if (mountedRef.current) setPickStatus('No file selected')
        return
      }
      reportError(thrown)
    },
    [reportError],
  )

  useEffect(() => {
    approvalsList()
      .then((records) => {
        if (!mountedRef.current) return
        setApprovals(records)
      })
      .catch(reportError)
  }, [reportError])

  async function refreshApprovals() {
    try {
      const records = await approvalsList()
      if (!mountedRef.current) return
      setApprovals(records)
    } catch (listError: unknown) {
      reportError(listError)
    }
  }

  /** Merges `approved` into local state -- an insert or an in-place update. */
  function upsertApproval(approved: Approval): void {
    setApprovals((previous) => [
      ...previous.filter((existing) => existing.approvalId !== approved.approvalId),
      approved,
    ])
  }

  async function pickAndSetCandidate(): Promise<ProbeResult> {
    const result = await executablePickAndProbe()
    if (mountedRef.current) {
      setCandidate(result)
      setConfirmInput('')
    }
    return result
  }

  async function handlePick() {
    setError(null)
    setPickStatus(null)
    setIsPicking(true)
    try {
      await pickAndSetCandidate()
    } catch (pickError: unknown) {
      reportPickRejection(pickError)
    } finally {
      if (mountedRef.current) setIsPicking(false)
    }
  }

  async function handleApprove() {
    if (candidate === null) return
    setError(null)
    setIsApproving(true)
    try {
      const approved = await executableApprove(candidate.candidateId)
      if (!mountedRef.current) return
      upsertApproval(approved)
      setCandidate(null)
      setConfirmInput('')
    } catch (approveError: unknown) {
      reportError(approveError)
    } finally {
      if (mountedRef.current) setIsApproving(false)
    }
  }

  async function handleRevoke(approvalId: ApprovalId) {
    // Guards against a double-click firing a second `executable_revoke`
    // for the same id while the first is still in flight (R3-002); the
    // Revoke button is also disabled for this id below, so this is
    // belt-and-suspenders against any click that slips through before
    // the re-render commits.
    if (revokingIds.includes(approvalId)) return
    setError(null)
    setRevokingIds((previous) => [...previous, approvalId])
    try {
      await executableRevoke(approvalId)
      await refreshApprovals()
    } catch (revokeError: unknown) {
      reportError(revokeError)
    } finally {
      if (mountedRef.current) {
        setRevokingIds((previous) => previous.filter((id) => id !== approvalId))
      }
    }
  }

  /**
   * Renews an approval whose content has changed since it was last
   * approved: re-runs pick-and-probe (the user must select the file
   * again, confirming its current content) then approve -- never a
   * "proceed anyway" affordance that would launch against a stale
   * approval (`docs/threat-model.md` HAR-3: "material change requires
   * renewed approval").
   */
  async function handleRenew() {
    setError(null)
    setPickStatus(null)
    setIsRenewing(true)

    let result: ProbeResult
    try {
      result = await pickAndSetCandidate()
    } catch (pickError: unknown) {
      // Cancelling the re-pick is the same normal, non-error outcome as
      // cancelling the initial pick (R3-001) -- and with no fresh
      // candidate to approve, the renew attempt simply stops here.
      reportPickRejection(pickError)
      if (mountedRef.current) setIsRenewing(false)
      return
    }

    try {
      const approved = await executableApprove(result.candidateId)
      if (!mountedRef.current) return
      upsertApproval(approved)
      setCandidate(null)
      setConfirmInput('')
    } catch (approveError: unknown) {
      reportError(approveError)
    } finally {
      if (mountedRef.current) setIsRenewing(false)
    }
  }

  /**
   * Approval is gated on an exact, case-sensitive match against the
   * candidate's own short digest -- no fixed phrase accepted in its
   * place (R1-001: a fixed shortcut would let a caller confirm without
   * ever having read the evidence it stands in for).
   */
  const isConfirmed = candidate !== null && confirmInput === candidate.evidence.sha256Short

  return (
    <section role="region" aria-label="Executable approval">
      <h2>Executable approval</h2>

      {error && (
        <div role="alert" data-testid="approval-error-banner">
          {error === 'unexpected' ? (
            <strong>unexpected error</strong>
          ) : (
            <>
              <strong>
                <PlainTextLine text={error.code} />
              </strong>
              : <PlainTextLine text={error.message} />
              {error.code === 'changed-since-approval' && error.detail && (
                <div>
                  <p>
                    Recorded: <PlainTextLine text={error.detail.recordedSha256Short} />
                    {' vs. observed: '}
                    <PlainTextLine text={error.detail.observedSha256Short} />
                  </p>
                  <button type="button" onClick={handleRenew} disabled={isRenewing}>
                    Renew approval
                  </button>
                </div>
              )}
            </>
          )}
        </div>
      )}

      <div>
        <button type="button" onClick={handlePick} disabled={isPicking}>
          Pick executable
        </button>
      </div>

      {pickStatus && <p role="status">{pickStatus}</p>}

      {candidate && (
        <div aria-label="Candidate evidence">
          <h3>Candidate evidence</h3>
          <p>
            Canonical path: <PlainTextLine text={candidate.evidence.canonicalPath} />
          </p>
          <p>
            Size: <PlainTextLine text={`${candidate.evidence.size}`} />
          </p>
          <p>
            SHA-256: <PlainTextLine text={candidate.evidence.sha256} />
          </p>
          <p>
            SHA-256 (short): <PlainTextLine text={candidate.evidence.sha256Short} />
          </p>
          <p>
            Modified: <PlainTextLine text={formatModifiedAt(candidate.evidence.modifiedAt)} />
          </p>
          <p>
            Platform: <PlainTextLine text={formatPlatform(candidate.evidence.platform)} />
          </p>

          {/*
            TM-001-R7 ("act-as identity"): the one actor this surface can
            ever bind an approval to is the current OS user running this
            device -- there is no separate account/identity selection in
            this narrow slice, so this line states that plainly rather
            than leaving it implicit. Plain text constant only: no name,
            no OS user id, nothing that would need PlainTextLine's
            untrusted-content handling.
          */}
          <p>Approving as: this device&apos;s local user</p>

          <label htmlFor="approval-confirm-input">
            {`Type the short digest (${candidate.evidence.sha256Short}) to confirm`}
          </label>
          <input
            id="approval-confirm-input"
            value={confirmInput}
            onChange={(event) => setConfirmInput(event.target.value)}
          />
          <button type="button" onClick={handleApprove} disabled={!isConfirmed || isApproving}>
            Approve
          </button>
        </div>
      )}

      <h3>Approvals</h3>
      <ul aria-label="Approvals list">
        {approvals.map((approval) => (
          <li key={approval.approvalId}>
            <PlainTextLine text={approval.evidence.canonicalPath} />
            {' -- '}
            <span>{approval.status}</span>
            {approval.status === 'active' && (
              <button
                type="button"
                onClick={() => handleRevoke(approval.approvalId)}
                disabled={revokingIds.includes(approval.approvalId)}
              >
                Revoke
              </button>
            )}
          </li>
        ))}
      </ul>
    </section>
  )
}

export default ApprovalSurface
