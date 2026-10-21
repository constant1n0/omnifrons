// ADR-0005 sub-slice 1i: the renderer surface for the agent-definition scan
// and verify. Mocked at the Tauri IPC boundary with `mockIPC`, exactly like
// `AgentPanel.test.tsx` and `ipc/harness.test.ts` -- this codebase has no
// `vi.mock('./ipc/harness')` precedent; every panel test drives the real
// wrapper functions against a mocked `invoke`.

import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { DefinitionSection } from './DefinitionSection'
import type { DefinitionStatus, DefinitionVerify, ShellError } from './ipc/harness'

const FIXTURE_STATUS: DefinitionStatus = {
  schema: 'omnifrons.definition-scan/1',
  root: '/workspace',
  ancestors: [{ path: '/outside', identityFiles: ['AGENTS.md'], hostExtras: ['CLAUDE.local.md'] }],
  agents: [
    {
      path: '.',
      state: 'agents-only',
      identityFiles: [{ kind: 'AGENTS.md', sha256: 'ab'.repeat(32), bytes: 42, symlink: null }],
      title: 'Fixture agent',
      parent: null,
      nested: ['sub'],
      consentRequired: true,
      skills: {
        discovered: ['skills/x/SKILL.md'],
        indexed: [],
        missing: ['skills/x/SKILL.md'],
        dangling: [],
        precedence: false,
      },
      warnings: [
        {
          code: 'extra-host-file',
          hosts: ['claude-code'],
          detail: 'a host-specific file sits alongside the identity files',
        },
      ],
    },
  ],
}

const FIXTURE_STATUS_RESCANNED: DefinitionStatus = { ...FIXTURE_STATUS, root: '/workspace-rescanned' }

const FIXTURE_VERIFY: DefinitionVerify = {
  schema: 'omnifrons.definition-verify/1',
  agent: '/workspace',
  state: 'none',
  canonical: false,
  problems: ['state is none'],
}

afterEach(() => {
  clearMocks()
})

describe('DefinitionSection (ADR-0005 sub-slice 1i)', () => {
  it('renders the ancestors, agents, warnings, and consent marker from the scanned status', async () => {
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return FIXTURE_STATUS
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)

    const section = await screen.findByLabelText('Definition')
    expect(within(section).getByText('/workspace')).toBeTruthy()
    expect(within(section).getByText('/outside')).toBeTruthy()
    // "AGENTS.md" appears twice: once in the ancestor's identity files and
    // once in the agent's own identity file row.
    expect(within(section).getAllByText('AGENTS.md').length).toBe(2)
    expect(within(section).getByText('extra-host-file')).toBeTruthy()
    expect(within(section).getByText(/consent required/)).toBeTruthy()
  })

  it('calls definition_status again when "Rescan" is clicked, replacing the shown report', async () => {
    let calls = 0
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        calls += 1
        return calls === 1 ? FIXTURE_STATUS : FIXTURE_STATUS_RESCANNED
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    fireEvent.click(screen.getByRole('button', { name: 'Rescan' }))

    await screen.findByText('/workspace-rescanned')
    expect(calls).toBe(2)
  })

  it('clears a previous verification result when "Rescan" is clicked', async () => {
    let statusCalls = 0
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        statusCalls += 1
        return statusCalls === 1 ? FIXTURE_STATUS : FIXTURE_STATUS_RESCANNED
      }
      if (cmd === 'definition_verify') return FIXTURE_VERIFY
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    fireEvent.click(screen.getByRole('button', { name: 'Verify root' }))
    await screen.findByLabelText('Verification')

    fireEvent.click(screen.getByRole('button', { name: 'Rescan' }))

    await screen.findByText('/workspace-rescanned')
    expect(screen.queryByLabelText('Verification')).toBeNull()
  })

  it('renders the verification\'s state, canonical flag, and problems when "Verify root" is clicked', async () => {
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return FIXTURE_STATUS
      if (cmd === 'definition_verify') return FIXTURE_VERIFY
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    fireEvent.click(screen.getByRole('button', { name: 'Verify root' }))

    const verification = await screen.findByLabelText('Verification')
    expect(within(verification).getByText('none')).toBeTruthy()
    expect(within(verification).getByText('canonical: no')).toBeTruthy()
    expect(within(verification).getByText('state is none')).toBeTruthy()
  })

  it('clears a stale verification result when a later verify rejects, showing only the new error', async () => {
    let shouldFail = false
    // `scan-failed` is the code `definition_verify`'s own definition-tree
    // read failure shares with `definition_status` (see `ShellErrorCode`'s
    // own doc comment on `scan-failed`), not a dedicated "verify" code.
    const error: ShellError = { code: 'scan-failed', message: 'the root could not be verified' }
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return FIXTURE_STATUS
      if (cmd === 'definition_verify') return shouldFail ? Promise.reject(error) : FIXTURE_VERIFY
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    fireEvent.click(screen.getByRole('button', { name: 'Verify root' }))
    await screen.findByLabelText('Verification')

    shouldFail = true
    fireEvent.click(screen.getByRole('button', { name: 'Verify root' }))

    const banner = await screen.findByTestId('definition-verify-error')
    expect(within(banner).getByText('scan-failed')).toBeTruthy()
    expect(within(banner).getByText(/the root could not be verified/)).toBeTruthy()
    expect(screen.queryByLabelText('Verification')).toBeNull()
  })

  it('ignores a second "Verify root" click while a verify is pending, calling definition_verify exactly once', async () => {
    let calls = 0
    let resolveVerify: ((value: DefinitionVerify) => void) | undefined
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return FIXTURE_STATUS
      if (cmd === 'definition_verify') {
        calls += 1
        return new Promise((resolve) => {
          resolveVerify = resolve as (value: DefinitionVerify) => void
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    const verifyButton = screen.getByRole('button', { name: 'Verify root' }) as HTMLButtonElement
    fireEvent.click(verifyButton)

    await waitFor(() => {
      expect(verifyButton.disabled).toBe(true)
    })
    fireEvent.click(verifyButton)

    if (!resolveVerify) throw new Error('definition_verify was not called')
    await act(async () => {
      resolveVerify!(FIXTURE_VERIFY)
    })

    await screen.findByLabelText('Verification')
    expect(calls).toBe(1)
  })

  it('calls definition_verify exactly once even when two "Verify root" clicks are dispatched before a re-render settles', async () => {
    let calls = 0
    let resolveVerify: ((value: DefinitionVerify) => void) | undefined
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return FIXTURE_STATUS
      if (cmd === 'definition_verify') {
        calls += 1
        return new Promise((resolve) => {
          resolveVerify = resolve as (value: DefinitionVerify) => void
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    const verifyButton = screen.getByRole('button', { name: 'Verify root' })
    // Both clicks are dispatched inside one `act` call, so React never gets
    // a chance to re-render between them -- unlike the double-click test
    // above, which awaits the `disabled` attribute first and so only
    // proves that attribute, never that `runVerify` itself is reentrant-safe
    // against two clicks racing on the pre-re-render `verifying` closure.
    act(() => {
      fireEvent.click(verifyButton)
      fireEvent.click(verifyButton)
    })
    expect(calls).toBe(1)

    if (!resolveVerify) throw new Error('definition_verify was not called')
    await act(async () => {
      resolveVerify!(FIXTURE_VERIFY)
    })
  })

  it('renders a ShellError from definition_status as its code and message, never the raw object', async () => {
    // The mount-time scan suppresses its own error (it meets a rejection
    // with an empty report, like `AgentPanel`'s own automatic fetches) --
    // only the explicit "Rescan" click surfaces it, so this clicks Rescan
    // once the quiet initial scan has settled.
    const error: ShellError = { code: 'scan-failed', message: 'the workspace could not be scanned' }
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return Promise.reject(error)
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await waitFor(() => {
      expect((screen.getByRole('button', { name: 'Rescan' }) as HTMLButtonElement).disabled).toBe(false)
    })
    fireEvent.click(screen.getByRole('button', { name: 'Rescan' }))

    const banner = await screen.findByRole('alert')
    expect(within(banner).getByText('scan-failed')).toBeTruthy()
    expect(within(banner).getByText(/the workspace could not be scanned/)).toBeTruthy()
    expect(banner.textContent).not.toContain('[object Object]')
  })

  it('clears the old report when a user-initiated "Rescan" fails, showing only the error', async () => {
    let shouldFail = false
    const error: ShellError = { code: 'scan-failed', message: 'the workspace could not be rescanned' }
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return shouldFail ? Promise.reject(error) : FIXTURE_STATUS
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    shouldFail = true
    fireEvent.click(screen.getByRole('button', { name: 'Rescan' }))

    const banner = await screen.findByRole('alert')
    expect(within(banner).getByText('scan-failed')).toBeTruthy()
    expect(within(banner).getByText(/the workspace could not be rescanned/)).toBeTruthy()
    expect(screen.queryByText('/workspace')).toBeNull()
  })

  it('shows a loading state before the scan resolves, then replaces it with the report', async () => {
    let resolveStatus: ((value: DefinitionStatus) => void) | undefined
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        return new Promise((resolve) => {
          resolveStatus = resolve as (value: DefinitionStatus) => void
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)

    await screen.findByRole('status')
    expect(screen.queryByText('/workspace')).toBeNull()

    if (!resolveStatus) throw new Error('definition_status was not called')
    await act(async () => {
      resolveStatus!(FIXTURE_STATUS)
    })

    await screen.findByText('/workspace')
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('drops a verify result that settles after a rescan has already replaced the report', async () => {
    let statusCalls = 0
    let resolveVerify: ((value: DefinitionVerify) => void) | undefined
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        statusCalls += 1
        return statusCalls === 1 ? FIXTURE_STATUS : FIXTURE_STATUS_RESCANNED
      }
      if (cmd === 'definition_verify') {
        return new Promise((resolve) => {
          resolveVerify = resolve as (value: DefinitionVerify) => void
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    fireEvent.click(screen.getByRole('button', { name: 'Verify root' }))
    if (!resolveVerify) throw new Error('definition_verify was not called')

    // The rescan's own `definition_status` call resolves while the verify
    // above is still pending -- a verification belongs to the report it
    // was run against, so this rescan invalidates it.
    fireEvent.click(screen.getByRole('button', { name: 'Rescan' }))
    await screen.findByText('/workspace-rescanned')

    await act(async () => {
      resolveVerify!(FIXTURE_VERIFY)
    })

    expect(screen.queryByLabelText('Verification')).toBeNull()
    expect(screen.getByText('/workspace-rescanned')).toBeTruthy()
  })

  it('calls definition_status exactly once even when two "Rescan" clicks are dispatched before a re-render settles', async () => {
    let calls = 0
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        calls += 1
        // The 1st call is the mount scan (resolved right away so the
        // button is enabled); later calls are left pending -- this test
        // only cares how many are dispatched, never how they resolve.
        if (calls === 1) return FIXTURE_STATUS
        return new Promise<DefinitionStatus>(() => {})
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    const rescanButton = screen.getByRole('button', { name: 'Rescan' })
    // Both clicks are dispatched inside one `act` call, so React never gets
    // a chance to re-render between them -- mirrors the existing same-tick
    // "Verify root" test above, this time proving `rescan`'s own ref guard
    // (not just the `disabled` attribute) keeps it to a single dispatch.
    act(() => {
      fireEvent.click(rescanButton)
      fireEvent.click(rescanButton)
    })

    expect(calls).toBe(2)
  })

  it('disables "Verify root" while the initial scan is loading, re-enabling once it settles', async () => {
    let resolveStatus: ((value: DefinitionStatus) => void) | undefined
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        return new Promise((resolve) => {
          resolveStatus = resolve as (value: DefinitionStatus) => void
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)

    const verifyButton = screen.getByRole('button', { name: 'Verify root' }) as HTMLButtonElement
    expect(verifyButton.disabled).toBe(true)

    if (!resolveStatus) throw new Error('definition_status was not called')
    await act(async () => {
      resolveStatus!(FIXTURE_STATUS)
    })

    expect(verifyButton.disabled).toBe(false)
  })
})
