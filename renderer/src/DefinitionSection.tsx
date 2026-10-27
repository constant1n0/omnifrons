import { useCallback, useEffect, useRef, useState } from 'react'

import {
  definitionApply,
  definitionPreview,
  definitionStatus,
  definitionVerify,
  isShellError,
  type DefinitionAgent,
  type DefinitionAncestor,
  type DefinitionApply,
  type DefinitionAppliedFile,
  type DefinitionFingerprint,
  type DefinitionIdentityFile,
  type DefinitionOperation,
  type DefinitionPlanRequest,
  type DefinitionPreview,
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

/** How many hex characters of a digest a line shows; the plan id is always shown whole. */
const SHORT_HEX_LENGTH = 12

function shortHex(hex: string): string {
  return hex.slice(0, SHORT_HEX_LENGTH)
}

function fingerprintText(fingerprint: DefinitionFingerprint): string {
  switch (fingerprint.kind) {
    case 'sha256':
      return `sha256 ${shortHex(fingerprint.hex)}`
    case 'symlink':
      return `symlink ${fingerprint.target}`
    case 'irregular':
    case 'absent':
      return fingerprint.kind
  }
}

function operationText(operation: DefinitionOperation): string {
  const before = fingerprintText(operation.before)
  const after = shortHex(operation.after)
  switch (operation.op) {
    case 'write-agents':
      return `${operation.op} ${operation.path} from ${operation.source}; before: ${before}; after: ${after}`
    case 'write-wrapper':
      return `${operation.op} ${operation.path}; before: ${before}; after: ${after}`
    case 'write-block':
      return `${operation.op} ${operation.path} (${operation.mode}); before: ${before}; after: ${after}`
  }
}

function appliedFileText(file: DefinitionAppliedFile): string {
  const after = file.after === null ? 'none' : shortHex(file.after)
  const snapshot = file.snapshot === null ? 'no snapshot' : `snapshot ${file.snapshot}`
  return `${file.path}: ${fingerprintText(file.before)} -> ${after} (${snapshot})`
}

/** The hint a stale plan gets in place of the shell's own wording. */
const PLAN_STALE_HINT = 'the plan no longer matches the definition; preview again'

interface DefinitionPreviewPanelProps {
  target: string | null
  preview: DefinitionPreview
  /** Disables Apply: a scan, verify, preview or apply is in flight. */
  busy: boolean
  onApply: () => void
}

function DefinitionPreviewPanel({ target, preview, busy, onApply }: DefinitionPreviewPanelProps) {
  const { outcome } = preview
  return (
    <div aria-label="Preview">
      {target !== null && (
        <p>
          target: <PlainTextLine text={target} /> (<PlainTextLine text={preview.agent} />)
        </p>
      )}
      {outcome.kind === 'refused' ? (
        <p>
          <strong>
            <PlainTextLine text={outcome.code} />
          </strong>
          {': '}
          <PlainTextLine text={outcome.message} />
        </p>
      ) : (
        <>
          <p>
            plan id: <code>{outcome.planId}</code>
          </p>
          <ul aria-label="Operations">
            {outcome.operations.map((operation, index) => (
              <li key={index}>
                <PlainTextLine text={operationText(operation)} />
              </li>
            ))}
          </ul>
          {outcome.blockDiff !== null && (
            <>
              <p>
                block diff: <PlainTextLine text={outcome.blockDiff.path} />
              </p>
              <p>before</p>
              <ul aria-label="Block before">
                {outcome.blockDiff.before.map((line, index) => (
                  <li key={index}>
                    <PlainTextLine text={line} />
                  </li>
                ))}
              </ul>
              <p>after</p>
              <ul aria-label="Block after">
                {outcome.blockDiff.after.map((line, index) => (
                  <li key={index}>
                    <PlainTextLine text={line} />
                  </li>
                ))}
              </ul>
            </>
          )}
          {outcome.notices.length > 0 && (
            <ul aria-label="Notices">
              {outcome.notices.map((notice, index) => (
                <li key={index}>
                  <PlainTextLine text={`portable agent folder: ${notice.path}`} />
                </li>
              ))}
            </ul>
          )}
          <button type="button" onClick={onApply} disabled={busy}>
            Apply
          </button>
        </>
      )}
    </div>
  )
}

function DefinitionApplyPanel({ result }: { result: DefinitionApply }) {
  const { outcome } = result
  return (
    <div aria-label="Apply">
      {outcome.kind === 'applied' && (
        <>
          <p>
            applied plan <code>{outcome.planId}</code>
          </p>
          <p>
            record: <PlainTextLine text={outcome.record} />
          </p>
          <ul aria-label="Applied files">
            {outcome.files.map((file, index) => (
              <li key={index}>
                <PlainTextLine text={appliedFileText(file)} />
              </li>
            ))}
          </ul>
        </>
      )}
      {outcome.kind === 'noop' && (
        <p>
          nothing to apply: the definition already matches plan <code>{outcome.planId}</code>
        </p>
      )}
      {outcome.kind === 'refused' && (
        <p>
          <strong>
            <PlainTextLine text={outcome.code} />
          </strong>
          {': '}
          <PlainTextLine text={outcome.message} />
        </p>
      )}
    </div>
  )
}

/** The preview and apply state the report renders, and the handlers that change it. */
interface DefinitionPlanView {
  withGemini: boolean
  /** Paths of the nested rows whose own consent box is checked. */
  consented: ReadonlySet<string>
  /** True while a scan, verify, preview or apply is in flight: disables every Preview and Apply. */
  busy: boolean
  /** True while an apply is in flight: a rescan would hide what it is writing. */
  applying: boolean
  /** The relative target the shown preview (or its error) was planned for. */
  previewTarget: string | null
  preview: DefinitionPreview | null
  previewError: DefinitionSectionError | null
  applyResult: DefinitionApply | null
  applyError: DefinitionSectionError | null
  onWithGeminiChange: (checked: boolean) => void
  onConsentChange: (path: string, checked: boolean) => void
  onPreview: (target: string) => void
  onApply: () => void
}

interface DefinitionAgentRowProps {
  agent: DefinitionAgent
  verifyPending: boolean
  onVerify: (target: string) => void
  plan: DefinitionPlanView
}

function DefinitionAgentRow({ agent, verifyPending, onVerify, plan }: DefinitionAgentRowProps) {
  const consented = plan.consented.has(agent.path)
  return (
    <li aria-label="Agent root">
      <p>
        <PlainTextLine text={agent.path} /> <PlainTextLine text={agent.state} />{' '}
        <button
          type="button"
          aria-label={`Verify ${agent.path}`}
          onClick={() => onVerify(agent.path)}
          disabled={verifyPending}
        >
          Verify
        </button>{' '}
        <button
          type="button"
          aria-label={`Preview ${agent.path}`}
          onClick={() => plan.onPreview(agent.path)}
          disabled={plan.busy || (agent.consentRequired && !consented)}
        >
          Preview
        </button>
        {agent.consentRequired && (
          <>
            {' '}
            <label>
              <input
                type="checkbox"
                checked={consented}
                onChange={(event) => plan.onConsentChange(agent.path, event.target.checked)}
              />{' '}
              Adapt <PlainTextLine text={agent.path} /> on its own
            </label>
          </>
        )}
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
  /** The relative target the shown verification (or error) was run for. */
  verifyTarget: string | null
  /**
   * Disables every row's "Verify" button: true while a verify is already in flight, or
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
  onVerify: (target: string) => void
  plan: DefinitionPlanView
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
  verifyTarget,
  verifyPending,
  onRescan,
  onVerify,
  plan,
}: DefinitionReportProps) {
  return (
    <section aria-label="Definition">
      <button type="button" onClick={onRescan} disabled={loading || plan.applying}>
        Rescan
      </button>{' '}
      <label>
        <input
          type="checkbox"
          checked={plan.withGemini}
          onChange={(event) => plan.onWithGeminiChange(event.target.checked)}
        />{' '}
        Also write GEMINI.md
      </label>
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
            {!status.agents.some((agent) => agent.path === '.') && (
              <>
                {' '}
                <button type="button" aria-label="Verify ." onClick={() => onVerify('.')} disabled={verifyPending}>
                  Verify
                </button>{' '}
                <button type="button" aria-label="Preview ." onClick={() => plan.onPreview('.')} disabled={plan.busy}>
                  Preview
                </button>
              </>
            )}
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
              <DefinitionAgentRow
                key={agent.path}
                agent={agent}
                verifyPending={verifyPending}
                onVerify={onVerify}
                plan={plan}
              />
            ))}
          </ul>
        </>
      )}
      {verifyError && (
        <div role="alert" data-testid="definition-verify-error">
          <DefinitionErrorLine error={verifyError} />
        </div>
      )}
      {verify && (
        <div aria-label="Verification">
          {verifyTarget !== null && (
            <p>
              target: <PlainTextLine text={verifyTarget} /> (<PlainTextLine text={verify.agent} />)
            </p>
          )}
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
      {plan.previewError && (
        <div role="alert" data-testid="definition-preview-error">
          <DefinitionErrorLine error={plan.previewError} />
        </div>
      )}
      {plan.preview && (
        <DefinitionPreviewPanel
          target={plan.previewTarget}
          preview={plan.preview}
          busy={plan.busy}
          onApply={plan.onApply}
        />
      )}
      {plan.applyError && (
        <div role="alert" data-testid="definition-apply-error">
          {plan.applyError !== 'unexpected' && plan.applyError.code === 'plan-stale' ? (
            PLAN_STALE_HINT
          ) : (
            <DefinitionErrorLine error={plan.applyError} />
          )}
        </div>
      )}
      {plan.applyResult && <DefinitionApplyPanel result={plan.applyResult} />}
    </section>
  )
}

/**
 * The container half: fetches the active workspace's agent-definition scan
 * on mount and on "Rescan," and runs a verification of a
 * chosen agent directory on demand (a row's "Verify"). Holds every loading/error/data
 * state; {@link DefinitionReport} only renders it.
 */
export function DefinitionSection() {
  const [status, setStatus] = useState<DefinitionStatus | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<DefinitionSectionError | null>(null)
  const [verify, setVerify] = useState<DefinitionVerify | null>(null)
  const [verifyError, setVerifyError] = useState<DefinitionSectionError | null>(null)
  const [verifying, setVerifying] = useState(false)
  const [verifyTarget, setVerifyTarget] = useState<string | null>(null)
  const [withGemini, setWithGemini] = useState(false)
  const [consented, setConsented] = useState<ReadonlySet<string>>(() => new Set())
  const [preview, setPreview] = useState<DefinitionPreview | null>(null)
  const [previewError, setPreviewError] = useState<DefinitionSectionError | null>(null)
  const [previewing, setPreviewing] = useState(false)
  /** The request the shown preview was planned from: an apply repeats it, whatever the boxes say now. */
  const [previewRequest, setPreviewRequest] = useState<DefinitionPlanRequest | null>(null)
  const [applyResult, setApplyResult] = useState<DefinitionApply | null>(null)
  const [applyError, setApplyError] = useState<DefinitionSectionError | null>(null)
  const [applying, setApplying] = useState(false)

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
   * One preview and one apply at a time, and neither beside a verify or a
   * scan. Refs, for the reason `scanningRef` and `verifyingRef` are: two
   * clicks in one tick both read the last committed state. Like a verify,
   * a preview superseded by a rescan still clears its own flag when it
   * settles; only its result is dropped. An apply is never superseded --
   * `rescan` refuses while one runs, so its outcome is always shown.
   */
  const previewingRef = useRef(false)
  const applyingRef = useRef(false)

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
   * an in-flight verify could bump `generationRef` past a *scan*
   * that was still genuinely pending, so the scan's own settle would find
   * itself superseded and leave `loading` stuck `true` forever, with
   * "Rescan" and the row's "Verify" disabled until remount. It is safe again
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
   * them there would leave the row's "Verify" refused forever instead.
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
   * same way it would during a later rescan. In practice the row's "Verify" is
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
    if (scanningRef.current || applyingRef.current) return
    scanningRef.current = true
    setLoading(true)
    setError(null)
    setVerify(null)
    setVerifyError(null)
    setVerifyTarget(null)
    setConsented(new Set())
    setPreview(null)
    setPreviewError(null)
    setApplyResult(null)
    setApplyError(null)
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
   * The explicit row "Verify" click, run for one agent path. Mirrors `rescan`'s own in-flight
   * handling: `verifyingRef` guards this handler itself against a second
   * click arriving before the first verify settles, so at most one
   * `definitionVerify` call is ever in flight; `verifying` (state) only
   * disables the button below and drives the pending UI.
   *
   * Refusing synchronously when `scanningRef.current` is also set is what
   * each "Verify" button's own `disabled` attribute already intends
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
  const runVerify = useCallback((target: string) => {
    if (verifyingRef.current || scanningRef.current || previewingRef.current || applyingRef.current) return
    verifyingRef.current = true
    const generation = ++generationRef.current
    setVerifying(true)
    setVerify(null)
    setVerifyError(null)
    setVerifyTarget(target)
    definitionVerify(target).then(
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

  /**
   * A preview of one target. Refused while anything else is in flight, like
   * `runVerify`; replaces the shown preview, and drops the apply result of
   * the previous one. Takes a generation like a verify does, so a rescan
   * that lands first wins and the late preview is never shown beside the
   * report it was not planned against.
   */
  const runPreview = useCallback((request: DefinitionPlanRequest) => {
    if (previewingRef.current || applyingRef.current || verifyingRef.current || scanningRef.current) return
    previewingRef.current = true
    const generation = ++generationRef.current
    setPreviewing(true)
    setPreview(null)
    setPreviewError(null)
    setPreviewRequest(request)
    setApplyResult(null)
    setApplyError(null)
    definitionPreview(request).then(
      (result) => {
        previewingRef.current = false
        if (!mountedRef.current) return
        setPreviewing(false)
        if (generationRef.current !== generation) return
        setPreview(result)
      },
      (reason: unknown) => {
        previewingRef.current = false
        if (!mountedRef.current) return
        setPreviewing(false)
        if (generationRef.current !== generation) return
        setPreviewError(toSectionError(reason))
      },
    )
  }, [])

  const onPreview = useCallback(
    (target: string) => {
      runPreview({ agent: target, nestedConsent: consented.has(target), canonicalFrom: null, withGemini })
    },
    [runPreview, consented, withGemini],
  )

  /**
   * The scan that follows an apply that wrote, or may have written: unlike a
   * user's "Rescan" it keeps the apply result or error on screen, and it
   * drops the verification and the consent boxes, which described the rows
   * as they were.
   */
  const rescanAfterApply = useCallback(() => {
    scanningRef.current = true
    setLoading(true)
    setError(null)
    setVerify(null)
    setVerifyError(null)
    setVerifyTarget(null)
    setConsented(new Set())
    scan(true)
  }, [scan])

  /**
   * The apply of the shown plan, bound to its id and to the request it was
   * planned from. After an applied outcome the section rescans so the rows
   * show the new state; unlike a user's "Rescan" that keeps the apply
   * result on screen. A stale plan is dropped (its Apply would only fail
   * again); any other failure keeps the preview so the user can retry.
   */
  const runApply = useCallback(() => {
    if (applyingRef.current || previewingRef.current || verifyingRef.current || scanningRef.current) return
    if (preview === null || preview.outcome.kind !== 'planned' || previewRequest === null) return
    const planId = preview.outcome.planId
    applyingRef.current = true
    const generation = ++generationRef.current
    setApplying(true)
    setApplyResult(null)
    setApplyError(null)
    definitionApply(previewRequest, planId).then(
      (result) => {
        applyingRef.current = false
        if (!mountedRef.current) return
        setApplying(false)
        if (generationRef.current !== generation) return
        setApplyResult(result)
        if (result.outcome.kind === 'refused') return
        setPreview(null)
        if (result.outcome.kind === 'noop') return
        rescanAfterApply()
      },
      (reason: unknown) => {
        applyingRef.current = false
        if (!mountedRef.current) return
        setApplying(false)
        if (generationRef.current !== generation) return
        setApplyError(toSectionError(reason))
        // `run-active` is refused before any write, so the plan still holds
        // and Apply can be retried. Any other failure may have happened after
        // some writes: the plan no longer holds, and the rows are re-read.
        if (isShellError(reason) && reason.code === 'run-active') return
        setPreview(null)
        rescanAfterApply()
      },
    )
  }, [preview, previewRequest, rescanAfterApply])

  const onConsentChange = useCallback((path: string, checked: boolean) => {
    setConsented((current) => {
      const next = new Set(current)
      if (checked) next.add(path)
      else next.delete(path)
      return next
    })
  }, [])

  const busy = loading || verifying || previewing || applying

  return (
    <DefinitionReport
      status={status}
      loading={loading}
      error={error}
      verify={verify}
      verifyError={verifyError}
      verifyTarget={verifyTarget}
      verifyPending={busy}
      onRescan={rescan}
      onVerify={runVerify}
      plan={{
        withGemini,
        consented,
        busy,
        applying,
        previewTarget: previewRequest?.agent ?? null,
        preview,
        previewError,
        applyResult,
        applyError,
        onWithGeminiChange: setWithGemini,
        onConsentChange,
        onPreview,
        onApply: runApply,
      }}
    />
  )
}

export default DefinitionSection
