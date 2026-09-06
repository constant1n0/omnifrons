import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import App, { DEFAULT_UNTRUSTED_CONTENT } from './App'

// `ApprovalSurface` (mounted by `App`) calls `approvals_list` on mount in
// every test in this file -- mock it here, for every test, so this
// module's IPC surface is deterministic rather than leaving `invoke`
// unmocked, which throws a bare `TypeError` (no `__TAURI_INTERNALS__` in
// this jsdom environment) instead of a proper `ShellError` rejection
// (R3-003). Any other command is a real error in this file's tests, so
// it rejects with a fixed, catalogue `ShellError` rather than throwing.
beforeEach(() => {
  mockIPC((cmd) => {
    if (cmd === 'approvals_list') return []
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
