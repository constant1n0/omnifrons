import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

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
// `adapters_list`, `outbox_status`, `publications_list` and -- since slice
// 5c -- `guidance_status` and `guidance_snapshots` for both managed kinds
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
      cmd === 'guidance_snapshots'
    ) {
      return Promise.reject({
        code: 'workspace-unavailable',
        message: 'no workspace has been picked yet',
      })
    }
    return Promise.reject({ code: 'invalid-request', message: 'unexpected command in test' })
  })
})

afterEach(() => {
  clearMocks()
})

describe('App', () => {
  it('renders no alert on mount', async () => {
    render(<App />)

    // Flush every pending microtask in the mount-time approvalsList() ->
    // .then/.catch chain (each async-function hop schedules its own
    // microtask) before asserting nothing rendered an alert.
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(screen.queryByRole('alert')).toBeNull()
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
})
