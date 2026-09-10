import { AgentPanel } from './AgentPanel'
import { ApprovalSurface } from './ApprovalSurface'
import { HarnessPanel } from './HarnessPanel'
import { PanelErrorBoundary } from './PanelErrorBoundary'
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
      {/* Each panel gets its own boundary, so a render-time throw in one
          costs that one panel and not the page. These three are siblings
          under a single root, and React unmounts the whole tree on an
          uncaught render throw: without a boundary here, a bad token in the
          agent panel's tables took the approval surface -- the consent gate
          -- down with it. The boundary is per panel rather than one around
          all three for exactly that reason; a shared one would keep the
          page alive and still lose all three surfaces at once. */}
      {/* The name each boundary carries is the name its panel carries on
          the page -- `Demo harness`, `Executable approval`, `Agent` -- and
          not a shorthand for it: the fallback's whole job is to tell the
          user which surface is gone, and it cannot do that with a name the
          page never showed them (R3-040). */}
      <PanelErrorBoundary name="Demo harness">
        <HarnessPanel />
      </PanelErrorBoundary>
      {/* A sibling of HarnessPanel, never a child -- RCS-001-R6 and
          docs/renderer-content-security.md's guarantee that no rendered
          content appears inside an approval surface. The boundary wraps the
          panel and is itself a sibling, so nothing about that containment
          changes. */}
      <PanelErrorBoundary name="Executable approval">
        <ApprovalSurface />
      </PanelErrorBoundary>
      {/* A sibling of HarnessPanel and ApprovalSurface, never nested inside
          either -- spike slice 3's own adapter launch surface keeps the
          same containment guarantee (`docs/spike-log.md` § Slice 3
          renderer). */}
      <PanelErrorBoundary name="Agent">
        <AgentPanel />
      </PanelErrorBoundary>
    </>
  )
}

export default App
