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
   * Scans the active workspace. `surfaceError` mirrors `AgentPanel`'s own
   * `fetchGuidanceStatus`: the automatic mount-time scan below passes
   * `false` -- a rejection just leaves the report empty, no alert -- and
   * the explicit "Rescan" click passes `true`, so a request the user made
   * directly is always answered, even by a failure.
   */
  // No synchronous `setState` here (react-hooks/set-state-in-effect):
  // `loading` starts `true` by its own initializer for the mount call
  // below, and every other transition happens inside `.then`/`.catch`,
  // never in the function body itself -- mirrors `AgentPanel`'s own
  // `refreshOutboxStatus`, which sets no state before its own fetch
  // settles either.
  const scan = useCallback((surfaceError: boolean) => {
    definitionStatus().then(
      (result) => {
        if (!mountedRef.current) return
        setStatus(result)
        setLoading(false)
      },
      (reason: unknown) => {
        if (!mountedRef.current) return
        setLoading(false)
        if (!surfaceError) {
          setStatus(null)
          return
        }
        setError(toSectionError(reason))
      },
    )
  }, [])

  useEffect(() => {
    scan(false)
  }, [scan])

  /** The explicit "Rescan" click: unlike the mount call, this sets `loading`/clears `error` itself -- an event handler, never an effect, so the lint rule above does not apply here. */
  const rescan = useCallback(() => {
    setLoading(true)
    setError(null)
    scan(true)
  }, [scan])

  /**
   * The explicit "Verify root" click. Mirrors `rescan`'s own in-flight
   * state: `verifying` both disables the button below and guards this
   * handler itself against a second click arriving before the first
   * verify settles, so at most one `definitionVerify` call is ever in
   * flight. Clearing `verify` here -- not only in the rejection branch
   * -- means a stale result is never shown beside either a new success
   * or a new error banner; only the latest outcome is ever on screen.
   */
  const runVerify = useCallback(() => {
    if (verifying) return
    setVerifying(true)
    setVerify(null)
    setVerifyError(null)
    definitionVerify().then(
      (result) => {
        if (!mountedRef.current) return
        setVerifying(false)
        setVerify(result)
      },
      (reason: unknown) => {
        if (!mountedRef.current) return
        setVerifying(false)
        setVerify(null)
        setVerifyError(toSectionError(reason))
      },
    )
  }, [verifying])

  return (
    <DefinitionReport
      status={status}
      loading={loading}
      error={error}
      verify={verify}
      verifyError={verifyError}
      verifyPending={verifying}
      onRescan={rescan}
      onVerify={runVerify}
    />
  )
}

export default DefinitionSection
