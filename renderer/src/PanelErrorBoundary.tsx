import { Component, type ErrorInfo, type ReactNode } from 'react'

interface PanelErrorBoundaryProps {
  /**
   * The panel's own name, stated in the fallback so the user knows which
   * surface is gone -- and it must be the name that panel actually carries
   * on the page, not a shorthand for it. The three were `Harness`,
   * `Approvals` and `Agent` while the panels were labelled `Demo harness`,
   * `Executable approval` and `Agent`, so the one sentence whose whole job
   * is to say which surface closed named a surface the user could not find
   * for two of the three (R3-040).
   */
  name: string
  children: ReactNode
}

interface PanelErrorBoundaryState {
  failed: boolean
}

/**
 * Contains a render-time throw to the one panel it wraps.
 *
 * `App` mounts `HarnessPanel`, `ApprovalSurface` and `AgentPanel` as bare
 * siblings under one root. React unmounts the **whole tree** when a render
 * throws with no boundary above it, so until now any one of the three
 * taking a bad value from the wire took all three down with it: the
 * approval surface -- the product's consent gate -- and every freeze guard
 * disappeared because a table cell somewhere else received a token it did
 * not recognize. That coupling is the thing this boundary removes. The
 * blast radius of a render bug is now the panel that has it.
 *
 * It is deliberately minimal, and it is a **second** line rather than the
 * first. The wire-typed formatters carry their own fallbacks and the
 * findings listing validates each row before rendering it, so the failures
 * that were actually reachable are handled where they happen, with copy
 * that says something useful. This catches what nobody predicted, and it
 * does not try to be clever about it: no retry, no reset, no reload button.
 * A panel whose render just threw has state this component knows nothing
 * about, and offering to re-run that render would be offering a fix it
 * cannot deliver.
 *
 * The fallback says which panel is gone and that the others still stand,
 * because the alternative -- a panel silently vanishing from the page -- is
 * how a user comes to believe a surface said nothing when in fact it never
 * rendered. Fixed copy, and no part of the error object reaches the DOM: an
 * error's message can carry text the shell never chose, and this is not the
 * surface to start rendering that on.
 *
 * It also says the two things the first version left the user to discover
 * (R3-038, R3-039). **A closed panel can own live work.** A throw that
 * lands mid-run takes the Stop button with the panel: `harness_stop` is
 * never called, the supervised process keeps running, and "the other
 * panels are unaffected" is true of the page and false of the machine. And
 * **the closure is permanent for the session**: `failed` never resets, no
 * `key` above it changes, so good data arriving afterwards re-opens
 * nothing. Around the approval surface that is the session's only
 * revocation control gone. A reset is deliberately not offered -- a panel
 * whose render just threw has state this component knows nothing about,
 * and re-running that render would be offering a fix it cannot deliver --
 * so the fallback names the recovery that does work, which is a reload.
 *
 * A class component because that is the only form React gives this: there
 * is no hook equivalent of `getDerivedStateFromError` in React 19. It
 * catches **render**, not events: an error thrown inside an `onClick`
 * escapes it entirely, which is React's design and not something a
 * boundary can widen.
 */
export class PanelErrorBoundary extends Component<
  PanelErrorBoundaryProps,
  PanelErrorBoundaryState
> {
  state: PanelErrorBoundaryState = { failed: false }

  static getDerivedStateFromError(): PanelErrorBoundaryState {
    return { failed: true }
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    // Kept out of the DOM and out of `console.error`, which the tests treat
    // as a failure signal: the developer-facing detail goes to the debug
    // channel, and the user-facing surface says only the fixed sentence.
    console.debug('panel render failed', this.props.name, error, info.componentStack)
  }

  render(): ReactNode {
    if (!this.state.failed) return this.props.children
    return (
      // The region keeps the panel's own accessible name rather than
      // becoming `<name> unavailable`: the two never stand at once, so the
      // landmark a user navigates by is the same before and after the
      // panel closed, and what changed is what the region says, not what
      // it is called (R3-040).
      <section aria-label={this.props.name}>
        <p role="alert">
          {`the ${this.props.name} panel could not be displayed and has been closed; the other panels on this page are unaffected. Anything it had already started keeps running: a supervised process it launched is not stopped by this, and the control that would have stopped it has gone with the panel. Reload the page to bring the panel back.`}
        </p>
      </section>
    )
  }
}

export default PanelErrorBoundary
