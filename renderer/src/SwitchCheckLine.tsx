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
export function SwitchCheckLine({ adapterId, disabled }: SwitchCheckLineProps) {
  const [choice, setChoice] = useState<{ adapterId: string; host: string } | null>(null)
  const [outcome, setOutcome] = useState<Outcome | null>(null)
  const [hosts, setHosts] = useState<DefinitionHost[] | null>(null)

  // A host chosen for another adapter does not carry over.
  const host = choice !== null && choice.adapterId === adapterId ? choice.host : ''
  const key = adapterId === null ? null : `${adapterId}\u0000${host}`

  useEffect(() => {
    if (adapterId === null || key === null) return
    let live = true
    definitionSwitchCheck(adapterId, host === '' ? undefined : host).then(
      (check) => {
        if (live) setOutcome({ key, check })
      },
      (error: unknown) => {
        if (live) setOutcome({ key, error: isShellError(error) ? error : 'unexpected' })
      },
    )
    return () => {
      live = false
    }
  }, [adapterId, host, key])

  const current = outcome !== null && outcome.key === key ? outcome : null
  const verdict = current !== null && 'check' in current ? current.check.verdict : null
  const needsHost =
    adapterId !== null &&
    (host !== '' || (verdict !== null && verdict.kind === 'uncertain' && verdict.reason === 'no-host-chosen'))

  useEffect(() => {
    if (!needsHost || hosts !== null) return
    let live = true
    definitionHosts().then(
      (rows) => {
        if (live) setHosts(rows)
      },
      () => {
        if (live) setHosts([])
      },
    )
    return () => {
      live = false
    }
  }, [needsHost, hosts])

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
              setChoice({ adapterId, host: event.target.value })
            }}
          >
            <option value="">Choose a host</option>
            {(hosts ?? []).map((row) => (
              <option key={row.name} value={row.name}>
                {row.name}
              </option>
            ))}
          </select>
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
