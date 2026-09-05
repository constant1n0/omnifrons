import { UntrustedText } from './UntrustedText'

/** Shown only when `untrustedContent` is omitted (`undefined`), never for an explicit empty string. */
export const DEFAULT_UNTRUSTED_CONTENT = 'No content received yet.'

interface AppProps {
  /**
   * Content from an untrusted source, rendered as plain text via UntrustedText.
   *
   * Omitting this prop (`undefined`) falls back to `DEFAULT_UNTRUSTED_CONTENT`,
   * since JavaScript default parameters only apply to `undefined`. An explicit
   * empty string is a deliberate, distinct value — it renders as empty content,
   * not as the placeholder.
   */
  untrustedContent?: string
}

export function App({ untrustedContent = DEFAULT_UNTRUSTED_CONTENT }: AppProps) {
  return (
    <>
      <h1>Omnifrons</h1>
      <UntrustedText content={untrustedContent} />
    </>
  )
}

export default App
