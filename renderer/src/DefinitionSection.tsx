import { useCallback, useEffect, useRef, useState } from 'react'

import {
  definitionStatus,
  definitionVerify,
  isShellError,
  type DefinitionAgent,
  type DefinitionAncestor,
  type DefinitionIdentityFile,
  type DefinitionStatus,
  type DefinitionVerify,
  type DefinitionWarning,
  type ShellError,
} from './ipc/harness'
import { PlainTextLine } from './PlainTextLine'

/**
 * ADR-0005 sub-slice 1i: the renderer surface for the agent-definition
 * scan and verify (sub-slice 1h's `definition_status`/`definition_verify`
 * commands). Split container/presentational: {@link DefinitionSection}
 * owns the fetches and all loading/error/data state; {@link
 * DefinitionReport} only renders what it is given.
 */

/** A scan or verify rejection: a typed shell error, or `'unexpected'` for any rejection that isn't `ShellError`-shaped -- mirrors `AgentPanel`'s own `PanelError`. */
type DefinitionSectionError = ShellError | 'unexpected'

function toSectionError(reason: unknown): DefinitionSectionError {
  return isShellError(reason) ? reason : 'unexpected'
}

/**
 * Renders one {@link DefinitionSectionError} the way the rest of this panel
 * renders a `ShellError` (`AgentPanel`'s own error banner): the closed code
 * in `<strong>`, the message through {@link PlainTextLine} -- never the raw
 * object.
 */
function DefinitionErrorLine({ error }: { error: DefinitionSectionError }) {
  if (error === 'unexpected') return <strong>unexpected error</strong>
  return (
    <>
      <strong>
        <PlainTextLine text={error.code} />
      </strong>
      {': '}
      <PlainTextLine text={error.message} />
    </>
  )
}

function DefinitionIdentityFileRow({ file }: { file: DefinitionIdentityFile }) {
  return (
    <li>
      <PlainTextLine text={file.kind} />
      {file.symlink !== null && (
        <>
          {' -> '}
          <PlainTextLine text={file.symlink} />
        </>
      )}
    </li>
  )
}

function DefinitionWarningRow({ warning }: { warning: DefinitionWarning }) {
  return (
    <li>
      <strong>
        <PlainTextLine text={warning.code} />
      </strong>
      {' ('}
      <PlainTextLine text={warning.hosts.join(', ')} />
      {'): '}
      <PlainTextLine text={warning.detail} />
    </li>
  )
}

function DefinitionAgentRow({ agent }: { agent: DefinitionAgent }) {
  return (
    <li aria-label="Agent root">
      <p>
        <PlainTextLine text={agent.path} /> <PlainTextLine text={agent.state} />
      </p>
      {agent.title !== null && (
        <p>
          title: <PlainTextLine text={agent.title} />
        </p>
      )}
      <p>
        parent:{' '}
        {agent.parent === null ? (
          'none (scan root)'
        ) : (
          <>
            <PlainTextLine text={agent.parent.kind} /> <PlainTextLine text={agent.parent.path} />
          </>
        )}
      </p>
      {agent.consentRequired && <p>consent required</p>}
      <p>
        skills: discovered {agent.skills.discovered.length}, indexed {agent.skills.indexed.length}, missing{' '}
        {agent.skills.missing.length}, dangling {agent.skills.dangling.length}
        {agent.skills.precedence && ' (precedence observed)'}
      </p>
      {agent.identityFiles.length > 0 && (
        <ul aria-label="Identity files">
          {agent.identityFiles.map((file) => (
            <DefinitionIdentityFileRow key={file.kind} file={file} />
          ))}
        </ul>
      )}
      {agent.nested.length > 0 && (
        <ul aria-label="Nested agents">
          {agent.nested.map((path) => (
            <li key={path}>
              <PlainTextLine text={path} />
            </li>
          ))}
        </ul>
      )}
      {agent.warnings.length > 0 && (
        <ul aria-label="Warnings">
          {agent.warnings.map((warning, index) => (
            <DefinitionWarningRow key={index} warning={warning} />
          ))}
        </ul>
      )}
    </li>
  )
}

function DefinitionAncestorRow({ ancestor }: { ancestor: DefinitionAncestor }) {
  return (
    <li>
      <PlainTextLine text={ancestor.path} />
      {ancestor.identityFiles.length > 0 && (
        <>
          {' -- identity files: '}
          <PlainTextLine text={ancestor.identityFiles.join(', ')} />
        </>
      )}
      {ancestor.hostExtras.length > 0 && (
        <>
          {' -- host extras: '}
          <PlainTextLine text={ancestor.hostExtras.join(', ')} />
        </>
      )}
    </li>
  )
}

interface DefinitionReportProps {
  status: DefinitionStatus | null
  loading: boolean
  error: DefinitionSectionError | null
  verify: DefinitionVerify | null
  verifyError: DefinitionSectionError | null
  /**
   * Disables "Verify root": true while a verify is already in flight, or
   * while any scan is loading. A verification belongs to the report on
   * screen, so it cannot be started against one that is still being
   * fetched: `runVerify` itself refuses synchronously while `scanningRef`
   * shows a scan in flight (mount or rescan), and this prop is what keeps
   * the button visibly disabled for that same window. The container's
   * `generationRef` covers the one case neither of those catches -- a
   * verify that was already running *before* a scan started -- by
   * dropping its result once that scan supersedes it.
   */
  verifyPending: boolean
  onRescan: () => void
  onVerify: () => void
}

/**
 * The presentational half: the scanned root, its ancestors, every
 * discovered agent, and a verification's outcome. Carries no fetch logic
 * of its own.
 */
export function DefinitionReport({
  status,
  loading,
  error,
  verify,
  verifyError,
  verifyPending,
  onRescan,
  onVerify,
}: DefinitionReportProps) {
  return (
    <section aria-label="Definition">
      <button type="button" onClick={onRescan} disabled={loading}>
        Rescan
      </button>
      {loading && <p role="status">scanning</p>}
      {error && (
        <div role="alert" data-testid="definition-error">
          <DefinitionErrorLine error={error} />
        </div>
      )}
      {status && (
        <>
          <p>
            root: <PlainTextLine text={status.root} />
          </p>
          {status.ancestors.length > 0 && (
            <ul aria-label="Ancestors">
              {status.ancestors.map((ancestor) => (
                <DefinitionAncestorRow key={ancestor.path} ancestor={ancestor} />
              ))}
            </ul>
          )}
          <ul aria-label="Agents">
            {status.agents.map((agent) => (
              <DefinitionAgentRow key={agent.path} agent={agent} />
            ))}
          </ul>
        </>
      )}
      <button type="button" onClick={onVerify} disabled={verifyPending}>
        Verify root
      </button>
      {verifyError && (
        <div role="alert" data-testid="definition-verify-error">
          <DefinitionErrorLine error={verifyError} />
        </div>
      )}
      {verify && (
        <div aria-label="Verification">
          <p>
            state: <PlainTextLine text={verify.state} />
          </p>
          <p>canonical: {verify.canonical ? 'yes' : 'no'}</p>
          {verify.problems.length > 0 && (
            <ul aria-label="Problems">
              {verify.problems.map((problem, index) => (
                <li key={index}>
                  <PlainTextLine text={problem} />
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  )
}

/**
 * The container half: fetches the active workspace's agent-definition scan
 * on mount and on "Rescan," and runs a verification of the active
 * workspace root on demand ("Verify root"). Holds every loading/error/data
 * state; {@link DefinitionReport} only renders it.
 */
export function DefinitionSection() {
  const [status, setStatus] = useState<DefinitionStatus | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<DefinitionSectionError | null>(null)
  const [verify, setVerify] = useState<DefinitionVerify | null>(null)
  const [verifyError, setVerifyError] = useState<DefinitionSectionError | null>(null)
  const [verifying, setVerifying] = useState(false)

  /** Guards every async continuation below against a post-unmount state update (mirrors `AgentPanel`'s own `mountedRef`). */
  const mountedRef = useRef(true)
  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  /**
   * One shared request counter for every scan *and* verify below. Each
   * dispatch bumps it and captures the post-bump value as its own
   * "generation"; a settle handler that finds the ref has since moved on
   * belongs to a request a later one has already superseded, so it skips
   * every state update tied to the result -- not just the display. This is
   * what makes a rescan invalidate a verify still in flight against the
   * report it was run against (a verification belongs to that report,
   * never a newer one), and what makes a scan whose settle handler runs
   * late lose to one dispatched more recently, regardless of settle order.
   */
  const generationRef = useRef(0)

  /**
   * Guards both the mount-time scan and `rescan` against a reentrant
   * dispatch, and tells `runVerify` below when a scan -- either one -- is
   * in flight. The reentrancy guard matters for the same reason as
   * `runVerify`'s own `verifyingRef`: `loading` (state) only reflects the
   * last completed render, so two clicks fired in the same tick (no
   * `await` between them) both see `loading === false` and both would
   * call `scan`. A ref is read and written synchronously, so it is
   * current for every call in the same tick, even back-to-back ones --
   * which is also why `runVerify` reads it directly instead of relying on
   * the `disabled` attribute its own click cannot yet observe mid-tick: a
   * verify dispatched in the same tick as a scan, before React has
   * committed the `disabled` attribute that scan just set, must still see
   * that scan in flight.
   */
  const scanningRef = useRef(false)

  /**
   * Scans the active workspace. `surfaceError` mirrors `AgentPanel`'s own
   * `fetchGuidanceStatus`: the automatic mount-time scan below passes
   * `false` -- a rejection just leaves the report empty, no alert -- and
   * the explicit "Rescan" click passes `true`, so a request the user made
   * directly is always answered, even by a failure. Either way, a
   * rejection clears `status`: a rescan that fails never leaves the prior
   * report on screen beside the new error, symmetric with the quiet
   * mount-time failure.
   *
   * `setLoading(false)` and `scanningRef.current = false` both wait on
   * the same generation check as `setStatus`/`setError` below: only the
   * settle whose captured `generation` still matches `generationRef.current`
   * clears anything at all. A scan that has already been superseded --
   * by React StrictMode's development-only effect replay dispatching a
   * second mount-time scan (see the mount effect below), or by a later
   * "Rescan" -- does nothing when it settles, pending flags included.
   *
   * Gating `loading`'s own clearing like this used to be exactly the bug:
   * an in-flight "Verify root" could bump `generationRef` past a *scan*
   * that was still genuinely pending, so the scan's own settle would find
   * itself superseded and leave `loading` stuck `true` forever, with
   * "Rescan" and "Verify root" disabled until remount. It is safe again
   * now because `scanningRef` -- set the moment a scan dispatches, in
   * both callers below -- stays `true` for the *entire* window any
   * relevant scan is in flight, not just until the first of two racing
   * dispatches happens to settle, precisely because its own clearing now
   * waits on this same check instead of running from an unconditional
   * `.finally`. `runVerify` already refuses synchronously while
   * `scanningRef.current` is set, so no verify can ever supersede a scan
   * that is genuinely still pending; the only thing that can ever move
   * `generationRef` past a pending scan's own value is another scan, and
   * that scan clears these same two flags when *it* settles. No scan, in
   * flight or superseded, is ever left with nothing left to clear them.
   *
   * `verifyingRef`/`verifying` are deliberately not given this same
   * treatment in `runVerify` below: a "Rescan" can still supersede a
   * verify that is genuinely in flight (see `rescan`'s own doc comment),
   * and nothing clears a superseded verify's *own* pending flags on its
   * behalf the way a winning scan clears a superseded scan's -- gating
   * them there would leave "Verify root" refused forever instead.
   * `setVerify`/`setVerifyError` keep their pre-existing generation gate:
   * only the *result* a superseded verify carries is ever dropped.
   */
  // No synchronous `setState` here (react-hooks/set-state-in-effect):
  // `loading` starts `true` by its own initializer for the mount call
  // below, and every other transition happens inside `.then`/`.catch`,
  // never in the function body itself -- mirrors `AgentPanel`'s own
  // `refreshOutboxStatus`, which sets no state before its own fetch
  // settles either.
  const scan = useCallback((surfaceError: boolean) => {
    const generation = ++generationRef.current
    return definitionStatus().then(
      (result) => {
        if (!mountedRef.current) return
        if (generationRef.current !== generation) return
        setLoading(false)
        scanningRef.current = false
        setStatus(result)
      },
      (reason: unknown) => {
        if (!mountedRef.current) return
        if (generationRef.current !== generation) return
        setLoading(false)
        scanningRef.current = false
        setStatus(null)
        if (surfaceError) {
          setError(toSectionError(reason))
        }
      },
    )
  }, [])

  /**
   * The automatic mount-time scan is still a scan, so it sets
   * `scanningRef` exactly like `rescan` below does -- a verify clicked
   * before this very first fetch has settled must see one in flight, the
   * same way it would during a later rescan. In practice "Verify root" is
   * already disabled for this entire window regardless (`verifyPending`
   * includes `loading`, which starts `true` from this component's own
   * state initializer, not from this effect), so setting it here too is
   * belt and suspenders for that part: it costs nothing and keeps
   * `scanningRef`'s own invariant -- true whenever a scan is in flight --
   * true regardless of how that scan was triggered, rather than true only
   * for `rescan`.
   *
   * The cleanup below is not belt and suspenders: it is what keeps this
   * effect correct under React StrictMode's development-only behavior of
   * running a mount effect, its cleanup, and the same mount effect again
   * -- precisely to surface the kind of missing-cleanup bug this one used
   * to have. Both replayed invocations call this same `scan`, so both
   * dispatches share this one instance's `generationRef`/`scanningRef`.
   * Bumping `generationRef` here, the instant React tears the first
   * invocation down, guarantees the second invocation's own call to
   * `scan` captures a strictly higher generation -- so the first scan's
   * eventual settle always finds itself superseded and does nothing (see
   * `scan`'s own doc comment above for exactly what that means now).
   * Resetting `scanningRef` to `false` here is not load-bearing by
   * itself -- the second invocation's own setup sets it `true` again
   * immediately after, in the same synchronous replay -- but it keeps the
   * ref an honest reflection of "this effect's own scan is in flight" at
   * every point in between, the same discipline `rescan` below follows.
   *
   * Outside StrictMode's development-only replay, this cleanup only ever
   * runs once, on a genuine unmount, after `mountedRef`'s own cleanup
   * (above) has already turned off every further state update from this
   * instance -- bumping a `generationRef` nothing will read again is
   * harmless.
   */
  useEffect(() => {
    scanningRef.current = true
    scan(false)
    return () => {
      generationRef.current += 1
      scanningRef.current = false
    }
  }, [scan])

  /**
   * The explicit "Rescan" click: unlike the mount call, this sets
   * `loading`/clears `error` itself -- an event handler, never an effect,
   * so the lint rule above does not apply here. A verification belongs
   * to the report it was run against, so a user-initiated rescan also
   * clears any prior `verify`/`verifyError` -- otherwise a stale
   * verification would sit beside a freshly scanned report it was never
   * run against. Bumping `generationRef` (inside `scan`) is what makes
   * that true even for a verify that is still in flight: its settle
   * handler finds the ref has moved on and skips `setVerify` entirely, so
   * a verification result that arrives after this rescan never appears
   * beside the newly scanned report, no matter when it settles.
   *
   * `scan` now clears `scanningRef` itself, once its own settle's
   * generation is still current (see `scan`'s own doc comment above), so
   * this click no longer chains a `.finally` to do it. The guard `if
   * (scanningRef.current) return` above is what keeps this function from
   * ever dispatching a second scan while this one, or the mount scan, is
   * still in flight.
   */
  const rescan = useCallback(() => {
    if (scanningRef.current) return
    scanningRef.current = true
    setLoading(true)
    setError(null)
    setVerify(null)
    setVerifyError(null)
    scan(true)
  }, [scan])

  /**
   * Guards `runVerify` against two clicks dispatched before React
   * re-renders between them. The `verifying` *state* only reflects the
   * last completed render, so a click handler's closure captured before
   * that render still reads the old value -- two clicks fired in the same
   * tick (no `await` between them) both see `verifying === false` and
   * both would call `definitionVerify`. A ref is read and written
   * synchronously, so it is current for every call in the same tick, even
   * back-to-back ones. `verifying` (state) is kept only to drive the
   * button's `disabled` attribute and the pending UI below.
   */
  const verifyingRef = useRef(false)

  /**
   * The explicit "Verify root" click. Mirrors `rescan`'s own in-flight
   * handling: `verifyingRef` guards this handler itself against a second
   * click arriving before the first verify settles, so at most one
   * `definitionVerify` call is ever in flight; `verifying` (state) only
   * disables the button below and drives the pending UI.
   *
   * Refusing synchronously when `scanningRef.current` is also set is what
   * "Verify root"'s own `disabled` attribute already intends
   * (`verifyPending` includes `loading`): a verification belongs to the
   * report on screen, so none starts while that report is still being
   * fetched -- fetched by either the mount-time scan or a rescan, both of
   * which set `scanningRef`. Without this check, a verify dispatched in
   * the same tick as a scan -- before React has committed that scan's own
   * `disabled` attribute -- would still start, bump `generationRef`, and
   * supersede the scan it raced; see `scan`'s own doc comment for what
   * that did to `loading`.
   *
   * Clearing `verify` here -- not only in the rejection branch -- means a
   * stale result is never shown beside either a new success or a new
   * error banner; only the latest outcome is ever on screen. Capturing
   * `generationRef`'s post-bump value means a result that settles after a
   * rescan has already moved the ref on is dropped instead of displayed:
   * `verifyingRef` and `verifying` are still cleared unconditionally (the
   * request is no longer in flight either way), but `setVerify`/
   * `setVerifyError` only run when this is still the latest generation.
   */
  const runVerify = useCallback(() => {
    if (verifyingRef.current || scanningRef.current) return
    verifyingRef.current = true
    const generation = ++generationRef.current
    setVerifying(true)
    setVerify(null)
    setVerifyError(null)
    definitionVerify().then(
      (result) => {
        verifyingRef.current = false
        if (!mountedRef.current) return
        setVerifying(false)
        if (generationRef.current !== generation) return
        setVerify(result)
      },
      (reason: unknown) => {
        verifyingRef.current = false
        if (!mountedRef.current) return
        setVerifying(false)
        if (generationRef.current !== generation) return
        setVerify(null)
        setVerifyError(toSectionError(reason))
      },
    )
  }, [])

  return (
    <DefinitionReport
      status={status}
      loading={loading}
      error={error}
      verify={verify}
      verifyError={verifyError}
      verifyPending={verifying || loading}
      onRescan={rescan}
      onVerify={runVerify}
    />
  )
}

export default DefinitionSection
