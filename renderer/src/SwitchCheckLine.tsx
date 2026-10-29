import { useEffect, useState } from 'react'

import { DefinitionErrorLine, type DefinitionSectionError } from './DefinitionErrorLine'
import { PlainTextLine } from './PlainTextLine'
import {
  definitionHosts,
  definitionSwitchCheck,
  isShellError,
  type DefinitionHost,
  type DefinitionSwitchCheck,
} from './ipc/harness'

interface SwitchCheckLineProps {
  /** The picked adapter; `null` renders nothing. */
  adapterId: string | null
  /**
   * What the check is about besides the adapter: the active workspace's
   * generation (empty when none). A change asks again, like a new adapter.
   */
  contextKey: string
  /** A run is live or the picker is frozen: the host choice freezes with it. */
  disabled: boolean
}

/** What one finished call answered, keyed by the request it answered. */
type Outcome =
  | { key: string; check: DefinitionSwitchCheck }
  | { key: string; error: DefinitionSectionError }

/** The number of hex characters of the digest shown beside the path. */
const DIGEST_SHOWN = 12

/**
 * The switch check's one-line verdict beside the adapter picker: whether
 * the host a model switch to `adapterId` would run reaches the canonical
 * definition. An adapter with no host of its own (its first check is
 * `uncertain` with `no-host-chosen`) gets a host select over the loader
 * table, listed by `definition_hosts`. Nothing is stored; each change of
 * adapter or host asks the shell again, and an answer overtaken by a
 * later change is dropped.
 */
export function SwitchCheckLine({ adapterId, contextKey, disabled }: SwitchCheckLineProps) {
  const [choice, setChoice] = useState<{ scope: string; host: string } | null>(null)
  const [outcome, setOutcome] = useState<Outcome | null>(null)
  const [hosts, setHosts] = useState<DefinitionHost[] | null>(null)
  const [hostsError, setHostsError] = useState<DefinitionSectionError | null>(null)
  const [hostsAttempt, setHostsAttempt] = useState(0)
  // The adapter whose first check found no host of its own: its select stays
  // mounted from then on, whatever a re-check is doing.
  const [pickerFor, setPickerFor] = useState<string | null>(null) // a scope, see below

  // A host chosen for another adapter does not carry over.
  const scope = adapterId === null ? null : `${contextKey}\u0000${adapterId}`
  const host = choice !== null && choice.scope === scope ? choice.host : ''
  const key = scope === null ? null : `${scope}\u0000${host}`

  useEffect(() => {
    if (adapterId === null || scope === null || key === null) return
    let live = true
    definitionSwitchCheck(adapterId, host === '' ? undefined : host).then(
      (check) => {
        if (!live) return
        setOutcome({ key, check })
        if (check.verdict.kind === 'uncertain' && check.verdict.reason === 'no-host-chosen') setPickerFor(scope)
      },
      (error: unknown) => {
        if (live) setOutcome({ key, error: isShellError(error) ? error : 'unexpected' })
      },
    )
    return () => {
      live = false
    }
  }, [adapterId, host, key, scope])

  const current = outcome !== null && outcome.key === key ? outcome : null
  const needsHost = scope !== null && pickerFor === scope

  useEffect(() => {
    if (!needsHost || hosts !== null) return
    let live = true
    definitionHosts().then(
      (rows) => {
        if (!live) return
        setHostsError(null)
        setHosts(rows)
      },
      (error: unknown) => {
        if (live) setHostsError(isShellError(error) ? error : 'unexpected')
      },
    )
    return () => {
      live = false
    }
  }, [needsHost, hosts, hostsAttempt])

  if (adapterId === null) return null

  const reference = current !== null && 'check' in current ? current.check.reference : null

  return (
    <div>
      {needsHost && (
        <>
          <label htmlFor="agent-switch-host">Target host</label>
          <select
            id="agent-switch-host"
            value={host}
            disabled={disabled}
            onChange={(event) => {
              if (disabled) return
              setChoice({ scope: scope ?? '', host: event.target.value })
            }}
          >
            <option value="">Choose a host</option>
            {(hosts ?? []).map((row) => (
              <option key={row.name} value={row.name}>
                {row.name}
              </option>
            ))}
          </select>
          {hostsError !== null && (
            <>
              <span role="alert">
                <DefinitionErrorLine error={hostsError} />
              </span>{' '}
              <button type="button" onClick={() => setHostsAttempt((attempt) => attempt + 1)}>
                Retry
              </button>
            </>
          )}
        </>
      )}
      <p aria-label="Switch check">
        {current === null ? (
          'Checking the target host...'
        ) : 'error' in current ? (
          <DefinitionErrorLine error={current.error} />
        ) : (
          <>
            <strong>{current.check.verdict.kind}</strong>
            {': '}
            <PlainTextLine text={current.check.verdict.line} />
            {'claim' in current.check.verdict && (
              <>
                {' '}
                <PlainTextLine
                  text={`claim: ${current.check.verdict.claim.source} (accessed ${current.check.verdict.claim.accessed}, ${current.check.verdict.claim.status})`}
                />
              </>
            )}
            {reference !== null && (
              <>
                {' '}
                <PlainTextLine text={`definition ${reference.path}@${reference.digest.slice(0, DIGEST_SHOWN)}`} />
              </>
            )}
          </>
        )}
      </p>
    </div>
  )
}

export default SwitchCheckLine
