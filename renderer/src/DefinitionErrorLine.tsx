import { PlainTextLine } from './PlainTextLine'
import type { ShellError } from './ipc/harness'

/** A scan or verify rejection: a typed shell error, or `'unexpected'` for any rejection that isn't `ShellError`-shaped -- mirrors `AgentPanel`'s own `PanelError`. */
export type DefinitionSectionError = ShellError | 'unexpected'

/**
 * Renders one {@link DefinitionSectionError} the way the rest of this panel
 * renders a `ShellError` (`AgentPanel`'s own error banner): the closed code
 * in `<strong>`, the message through {@link PlainTextLine} -- never the raw
 * object.
 */
export function DefinitionErrorLine({ error }: { error: DefinitionSectionError }) {
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
