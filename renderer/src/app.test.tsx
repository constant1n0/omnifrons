import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import App, { DEFAULT_UNTRUSTED_CONTENT } from './App'
import type { AdapterDescriptor } from './ipc/harness'

/**
 * The three built-in adapter descriptors `adapters_list` returns as of
 * spike slice 4 -- the two slice 3 line agents and the pseudo-terminal
 * fallback (`docs/spike-log.md` § Slice 4, IPC shapes) -- so `App`'s mount
 * exercises the real list shape, `pty-typed` channel included, rather than
 * an empty list. Metadata only, exactly as the wire carries it: no argv,
 * no env.
 */
const BUILT_IN_ADAPTERS: AdapterDescriptor[] = [
  {
    id: 'stream-json-cli',
    displayName: 'Stream-JSON CLI',
    transportClass: 'structured-streaming-cli',
    promptChannel: 'stdin-then-close',
    scopeMode: 'advisory',
    notes: 'generic stream-json CLI harness',
  },
  {
    id: 'claude-code',
    displayName: 'Claude Code',
    transportClass: 'structured-streaming-cli',
    promptChannel: 'stdin-then-close',
    scopeMode: 'advisory',
    notes: 'credentials are harness-owned; not exercised in CI',
  },
  {
    id: 'pty-cli',
    displayName: 'PTY CLI',
    transportClass: 'pty',
    promptChannel: 'pty-typed',
    scopeMode: 'advisory',
    notes: 'degraded fallback with no structured events',
  },
]

// `ApprovalSurface` and `AgentPanel` (both mounted by `App`) each call
// `approvals_list` on mount, and `AgentPanel` also calls `workspace_current`,
// `adapters_list`, `outbox_status`, `publications_list`, -- since slice
// 5c -- `guidance_status` and `guidance_snapshots` for both managed kinds,
// -- since slice 5d -- `wrongroot_status` and `misplaced_list`, and -- since
// slice 5e -- `recovery_list`
// -- mock all of them here, for every test, so this module's IPC surface is
// deterministic rather than leaving `invoke` unmocked, which throws a bare
// `TypeError` (no `__TAURI_INTERNALS__` in this jsdom environment) instead
// of a proper `ShellError` rejection (R3-003). Any other command is a real
// error in this file's tests, so it rejects with a fixed, catalogue
// `ShellError` rather than throwing.
beforeEach(() => {
  mockIPC((cmd) => {
    if (cmd === 'approvals_list') return []
    if (cmd === 'workspace_current') return null
    if (cmd === 'adapters_list') return BUILT_IN_ADAPTERS
    // No workspace is active in this mock, so the four project-scoped
    // fetches answer the way the shell does (`docs/spike-log.md` § Slice 5,
    // § Slice 5b and § Slice 5c, IPC shapes): the same `workspace-unavailable`
    // rejection an adapter launch gets, which `AgentPanel` meets with no
    // status line, no publications table, no guidance status line, and no
    // alert.
    if (
      cmd === 'outbox_status' ||
      cmd === 'publications_list' ||
      cmd === 'guidance_status' ||
      cmd === 'guidance_snapshots' ||
      // `recovery_list` is the fifth project-scoped mount-time fetch
      // (slice 5e): the work area's publication journal is read against the
      // active workspace, so with none the shell answers the same way.
      cmd === 'recovery_list'
    ) {
      return Promise.reject({
        code: 'workspace-unavailable',
        message: 'no workspace has been picked yet',
      })
    }
    // The two slice-5d fetches answer with no workspace active, unlike the
    // four above (`docs/spike-log.md` § Slice 5d, IPC shapes): the
    // output-discipline report is derived from the adapter catalog and the
    // findings live in shell state, so neither needs a project. The report
    // is the advisory one every built-in adapter's scope mode produces, and
    // its two disclosures are never collapsed to none.
    if (cmd === 'wrongroot_status') {
      return {
        outputDiscipline: 'advisory',
        scopeMode: 'advisory',
        disclosures: [
          'a write inside the project but outside the outbox is detected after the run, never prevented',
          'a write outside the project is possible and is detected after the run, not prevented',
        ],
        scanned: false,
        findings: 0,
      }
    }
    if (cmd === 'misplaced_list') return []
    return Promise.reject({ code: 'invalid-request', message: 'unexpected command in test' })
  })
})

afterEach(() => {
  clearMocks()
})

describe('App', () => {
  it('renders no alert on mount, with the wrong-root section showing the advisory report the shell answers with no workspace', async () => {
    render(<App />)

    // Flush every pending microtask in the mount-time approvalsList() ->
    // .then/.catch chain (each async-function hop schedules its own
    // microtask) before asserting nothing rendered an alert.
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(screen.queryByRole('alert')).toBeNull()
    // Unlike the four project-scoped fetches, `wrongroot_status` and
    // `misplaced_list` answer with no workspace active (slice 5d): the
    // report is the adapter catalog's, and the findings are shell state.
    expect(screen.getByRole('status', { name: 'Output discipline' }).textContent).toBe(
      'output discipline: advisory (scope advisory) — a write outside the project root is not prevented; not scanned yet',
    )
    expect(
      screen.getByRole('list', { name: 'Output discipline disclosures' }).querySelectorAll('li'),
    ).toHaveLength(2)
    // Slice 5e's two regions stand at mount too. The Catalog says it has
    // not been previewed rather than nothing at all -- previewing is a user
    // act, so no `catalog_repair_preview` is sent here -- and `recovery_list`
    // rejects `workspace-unavailable` with no project, which the panel meets
    // with the fault line and never an alert.
    expect(screen.getByRole('status', { name: 'Catalog plan' }).textContent).toBe(
      'catalog: not previewed — nothing here has read it yet; previewing reads the whole catalog and writes nothing',
    )
    // The recovery listing did answer -- with the rejection -- so the count
    // line is not rendered at all: `recovery entries: 0` is a claim about
    // this device that a read which was never made must not make.
    expect(screen.queryByRole('status', { name: 'Recovery count' })).toBeNull()
    // And it says *why* it was not made. `workspace-unavailable` at mount
    // is idle, not a failure: this test used to assert the panel claimed
    // the entries "could not be read" before a workspace had ever been
    // picked, which left a genuine read failure saying exactly what idle
    // says (R1-007).
    expect(screen.queryByRole('status', { name: 'Recovery entries unavailable' })).toBeNull()
    expect(screen.getByRole('status', { name: 'Recovery entries not read' }).textContent).toBe(
      'no workspace is active, so the recovery entries were not read; this is not a report that there are none',
    )
  })

  it('renders untrusted content as plain text by default', () => {
    const untrustedContent = 'Reported note: <b>bold</b> should stay literal.'

    const { container } = render(<App untrustedContent={untrustedContent} />)

    expect(container.textContent).toContain(untrustedContent)
    expect(container.querySelector('b')).toBeNull()
  })

  it('shows the default placeholder when no content was received', () => {
    const { container } = render(<App />)

    expect(container.textContent).toContain(DEFAULT_UNTRUSTED_CONTENT)
  })

  it('never interprets untrusted content as markup', () => {
    const payloads = [
      '<script>alert(1)</script>',
      '"><img src=x onerror=alert(1)>',
      '<a href="javascript:alert(1)">x</a>',
    ]

    for (const payload of payloads) {
      const { container, unmount } = render(<App untrustedContent={payload} />)

      expect(container.querySelector('script')).toBeNull()
      expect(container.querySelector('img')).toBeNull()
      expect(container.querySelector('a')).toBeNull()
      expect(container.textContent).toContain(payload)

      unmount()
    }
  })

  it('renders an explicit empty string as empty content, not the default placeholder', () => {
    const { container } = render(<App untrustedContent="" />)

    expect(container.textContent).not.toContain(DEFAULT_UNTRUSTED_CONTENT)
    expect(container.querySelector('p')?.textContent).toBe('')
  })

  it('keeps the approval surface standing when another panel throws during render: the three panels are bare siblings, and React unmounts the whole tree on an uncaught render throw', async () => {
    // The coupling this proves is gone: a value the agent panel could not
    // format used to throw mid-render, and with no boundary above these
    // siblings that throw took the page with it -- the approval surface,
    // the product's consent gate, disappearing because a table cell
    // somewhere else received a token it did not recognize.
    //
    // `adapters_list` answering a non-array is the cheapest way to make one
    // panel throw from outside it; React logs the caught error itself,
    // which is the boundary working.
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    clearMocks()
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') return []
      if (cmd === 'workspace_current') return null
      if (cmd === 'adapters_list') return 'not a list at all'
      if (cmd === 'misplaced_list') return []
      return Promise.reject({ code: 'workspace-unavailable', message: 'no workspace' })
    })

    render(<App />)
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    // The agent panel is closed and says so, by name -- and the region
    // keeps that name, so the landmark it occupied is still there to
    // navigate to (R3-040).
    expect(screen.getByRole('region', { name: 'Agent' })).toBeTruthy()
    const fallback = screen.getByRole('alert').textContent ?? ''
    expect(fallback).toContain('the Agent panel could not be displayed')
    // It also says what the closure costs beyond the pixels, and what the
    // user can do about it (R3-038, R3-039).
    expect(fallback).toContain('Anything it had already started keeps running')
    expect(fallback).toContain('Reload the page')
    // The approval surface and the harness panel are untouched, and the
    // names in play are the ones the page shows: `Demo harness` and
    // `Executable approval`, never `Harness` and `Approvals`.
    expect(screen.getByRole('heading', { name: 'Executable approval' })).toBeTruthy()
    expect(screen.getByRole('heading', { name: 'Demo harness' })).toBeTruthy()
    expect(screen.queryByText(/^the Approvals panel/)).toBeNull()
    expect(screen.queryByText(/^the Harness panel/)).toBeNull()
    expect(screen.getAllByRole('alert')).toHaveLength(1)
    consoleErrorSpy.mockRestore()
  })

  it("names each closed panel by the name that panel carries on the page -- `Demo harness`, `Executable approval`, `Agent` -- and leaves each region under its own name, so the landmark a user navigates by does not move when a panel closes", async () => {
    // The fallback's whole job is to say which surface is gone, and it was
    // saying `Harness` and `Approvals` while the page said `Demo harness`
    // and `Executable approval`: right for one of three (R3-040). One bad
    // `approvals_list` closes all three at once, which is what makes the
    // three names assertable together.
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    clearMocks()
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') return 'not a list at all'
      if (cmd === 'workspace_current') return null
      if (cmd === 'adapters_list') return BUILT_IN_ADAPTERS
      if (cmd === 'misplaced_list') return []
      return Promise.reject({ code: 'workspace-unavailable', message: 'no workspace' })
    })

    render(<App />)
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })
    // The harness panel fetches the approvals lazily, only once the
    // `approved` kind is selected, so it is closed by the same bad payload
    // one interaction later rather than at mount.
    fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'approved' } })
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    const named = screen
      .getAllByRole('alert')
      .map((alert) => (alert.textContent ?? '').replace(/ panel could not be displayed.*$/, ''))
    expect(named.sort()).toEqual([
      'the Agent',
      'the Demo harness',
      'the Executable approval',
    ])
    // Each region keeps the panel's own accessible name rather than
    // becoming `<name> unavailable`, so the landmark list is unchanged.
    expect(screen.getByRole('region', { name: 'Demo harness' })).toBeTruthy()
    expect(screen.getByRole('region', { name: 'Executable approval' })).toBeTruthy()
    expect(screen.getByRole('region', { name: 'Agent' })).toBeTruthy()
    consoleErrorSpy.mockRestore()
  })
})
