import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { PanelErrorBoundary } from './PanelErrorBoundary'

/** A component that throws during render, the way a formatter returning `undefined` did. */
function Exploding(): never {
  throw new Error('Array.from(undefined) is not iterable')
}

/** A component whose thrown message carries markup, so the "no error text" assertions have something to catch. */
function ExplodingWithMarkup(): never {
  throw new Error('<b>boom</b> at <i>docs/report.pdf</i>')
}

/**
 * A throw two levels below the boundary's own child, which is where a
 * formatter actually throws: the boundary's child is a whole panel, and the
 * bad token reaches a table cell inside it.
 */
function NestedExploding() {
  return (
    <div>
      <p>a heading that rendered fine</p>
      <ul>
        <li>
          <Exploding />
        </li>
      </ul>
    </div>
  )
}

/** A panel whose click handler throws -- which a React error boundary does not catch. */
function ThrowsOnClick() {
  return (
    <button
      type="button"
      onClick={() => {
        throw new Error('a handler, not a render')
      }}
    >
      Quarantine file
    </button>
  )
}

function Quiet() {
  return <p>quiet panel content</p>
}

const AGENT_FALLBACK =
  'the Agent panel could not be displayed and has been closed; the other panels on this page are unaffected. Anything it had already started keeps running: a supervised process it launched is not stopped by this, and the control that would have stopped it has gone with the panel. Reload the page to bring the panel back.'

describe('PanelErrorBoundary', () => {
  it('renders its children untouched while nothing throws', () => {
    render(
      <PanelErrorBoundary name="Agent">
        <Quiet />
      </PanelErrorBoundary>,
    )

    expect(screen.getByText('quiet panel content')).toBeTruthy()
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('replaces only the panel that threw, names it, and says the others still stand -- a panel that silently vanished is how a user comes to believe a surface said nothing when it never rendered', () => {
    // React writes the caught error to `console.error` itself; that is the
    // boundary working, not a failure, so it is silenced here rather than
    // asserted absent.
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    render(
      <>
        <PanelErrorBoundary name="Agent">
          <Exploding />
        </PanelErrorBoundary>
        <PanelErrorBoundary name="Executable approval">
          <Quiet />
        </PanelErrorBoundary>
      </>,
    )

    expect(screen.getByRole('alert').textContent).toBe(AGENT_FALLBACK)
    // The sibling is untouched, which is the entire point: these panels are
    // bare siblings under one root, and React unmounts the whole tree on an
    // uncaught render throw.
    expect(screen.getByText('quiet panel content')).toBeTruthy()
    consoleErrorSpy.mockRestore()
  })

  it('says what the closure costs beyond the pixels: a panel can be holding a running process, and the control that would have stopped it goes with it', () => {
    // "The other panels on this page are unaffected" is true of the page
    // and false of the machine. A throw that lands mid-run takes Stop with
    // the panel, `harness_stop` is never called, and the child process
    // keeps running with nothing on the page able to reach it. The fallback
    // has to say so, because nothing else on the page can.
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    render(
      <PanelErrorBoundary name="Agent">
        <Exploding />
      </PanelErrorBoundary>,
    )

    const fallback = screen.getByRole('alert').textContent ?? ''
    expect(fallback).toContain('Anything it had already started keeps running')
    expect(fallback).toContain('the control that would have stopped it has gone with the panel')
    // And the one recovery that does work is named, because the boundary
    // deliberately offers no reset: `failed` never clears and no `key`
    // above it changes, so the panel stays closed for the session.
    expect(fallback).toContain('Reload the page')
    expect(screen.queryByRole('button')).toBeNull()
    consoleErrorSpy.mockRestore()
  })

  it("keeps the landmark a user navigates by: the fallback region carries the panel's own name, not a renamed one, so the region does not disappear from the landmark list when the panel closes", () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    render(
      <PanelErrorBoundary name="Executable approval">
        <Exploding />
      </PanelErrorBoundary>,
    )

    expect(screen.getByRole('region', { name: 'Executable approval' })).toBeTruthy()
    expect(screen.queryByRole('region', { name: 'Executable approval unavailable' })).toBeNull()
    // And the name in the sentence is the panel's own, so the surface it
    // names is one the user can actually find on the page.
    expect(screen.getByRole('alert').textContent).toContain(
      'the Executable approval panel could not be displayed',
    )
    consoleErrorSpy.mockRestore()
  })

  it('catches a throw from a descendant several levels below its own child, which is where a formatter actually throws: inside a table cell inside a panel', () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    render(
      <PanelErrorBoundary name="Agent">
        <NestedExploding />
      </PanelErrorBoundary>,
    )

    expect(screen.getByRole('alert').textContent).toBe(AGENT_FALLBACK)
    // The part of the panel that had already rendered goes too: the
    // boundary replaces its whole child, not the subtree that threw.
    expect(screen.queryByText('a heading that rendered fine')).toBeNull()
    consoleErrorSpy.mockRestore()
  })

  it('does not catch a throw from an event handler, which React never routes to a boundary: the panel stays mounted and the error escapes', () => {
    // Worth pinning because it is the boundary's most likely
    // misunderstanding. Every remedy on the agent panel acts from an
    // `onClick`, and a throw there does not close the panel -- it escapes
    // to the window. This test says which half the boundary owns.
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    render(
      <PanelErrorBoundary name="Agent">
        <ThrowsOnClick />
      </PanelErrorBoundary>,
    )

    // React 19 does not re-throw a handler error out of the dispatch; it
    // lets it reach the window, so that is where it is caught. The
    // `preventDefault` marks it handled so it does not also fail the run.
    const escaped: string[] = []
    const onWindowError = (event: ErrorEvent) => {
      escaped.push(event.message)
      event.preventDefault()
    }
    window.addEventListener('error', onWindowError)
    fireEvent.click(screen.getByRole('button', { name: 'Quarantine file' }))
    window.removeEventListener('error', onWindowError)

    expect(escaped.join(' ')).toContain('a handler, not a render')
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.getByRole('button', { name: 'Quarantine file' })).toBeTruthy()
    consoleErrorSpy.mockRestore()
  })

  it('renders nothing of the error itself: a thrown message can carry text the shell never chose, and the fallback is fixed copy only', () => {
    // The fixture throws markup and a file name on purpose. Asserting no
    // `<b>` against an error whose message had no markup in it proved
    // nothing at all -- the assertion could not have failed.
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    render(
      <PanelErrorBoundary name="Agent">
        <ExplodingWithMarkup />
      </PanelErrorBoundary>,
    )

    const fallback = screen.getByRole('alert')
    expect(fallback.textContent).toBe(AGENT_FALLBACK)
    expect(fallback.textContent).not.toContain('boom')
    expect(fallback.textContent).not.toContain('docs/report.pdf')
    expect(fallback.querySelector('b')).toBeNull()
    expect(fallback.querySelector('i')).toBeNull()
    consoleErrorSpy.mockRestore()
  })
})
