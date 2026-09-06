import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { ApprovalSurface } from './ApprovalSurface'
import type { Approval, Evidence } from './ipc/harness'

// The jsdom crypto polyfill and React Testing Library's `cleanup()` are
// installed once for every test file by `testSupport/setup.ts`
// (`vite.config.ts`'s `test.setupFiles`) -- not repeated here.

afterEach(() => {
  clearMocks()
})

const SAMPLE_EVIDENCE: Evidence = {
  canonicalPath: '/opt/tool/<b>x</b>',
  size: 4096,
  sha256: 'a'.repeat(64),
  sha256Short: 'aaaaaaaa',
  modifiedAt: 1_732_000_000_000,
  platform: { os: 'unix', mode: 0o755 },
}

function sampleApproval(overrides: Partial<Approval> = {}): Approval {
  return {
    approvalId: 42,
    evidence: SAMPLE_EVIDENCE,
    approvedAt: 500,
    status: 'active',
    revokedAt: null,
    ...overrides,
  }
}

/** Mocks `approvals_list` (empty by default) plus whatever `extra` handles. */
function mockWithEmptyApprovalsList(
  extra?: (cmd: string, args: Record<string, unknown>) => unknown,
) {
  mockIPC((cmd, args) => {
    if (cmd === 'approvals_list') return []
    if (extra) return extra(cmd, args as Record<string, unknown>)
    throw new Error(`unexpected command: ${cmd}`)
  })
}

describe('ApprovalSurface', () => {
  it('renders as a standalone labelled region', async () => {
    mockWithEmptyApprovalsList()

    render(<ApprovalSurface />)

    const region = await screen.findByRole('region', { name: 'Executable approval' })
    expect(region).toBeTruthy()
  })

  it('calls approvalsList on mount and renders a generic error banner for a non-ShellError rejection', async () => {
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') {
        return Promise.reject(new Error('boom: /home/someone/secret/path'))
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('unexpected error')
  })

  it('shows a quiet status, not an alert, when the picker is cancelled (no-candidate)', async () => {
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return Promise.reject({ code: 'no-candidate', message: 'no file was selected' })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))

    const status = await screen.findByRole('status')
    expect(status.textContent).toBe('No file selected')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('shows an alert for a no-candidate rejection from executable_approve (a stale candidate)', async () => {
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return { candidateId: 7, evidence: SAMPLE_EVIDENCE }
      }
      if (cmd === 'executable_approve') {
        return Promise.reject({ code: 'no-candidate', message: 'unknown candidate id' })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))
    await screen.findByRole('heading', { name: 'Candidate evidence' })
    fireEvent.change(screen.getByLabelText(/type the short digest/i), {
      target: { value: SAMPLE_EVIDENCE.sha256Short },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toContain('no-candidate')
    expect(banner.textContent).toContain('unknown candidate id')
  })

  it('renders every evidence field as text after Pick executable, with no markup interpretation', async () => {
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return { candidateId: 7, evidence: SAMPLE_EVIDENCE }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    const region = await screen.findByRole('region', { name: 'Executable approval' })
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))

    await screen.findByRole('heading', { name: 'Candidate evidence' })

    expect(region.querySelector('b')).toBeNull()
    expect(region.textContent).toContain('/opt/tool/<b>x</b>')
    expect(region.textContent).toContain('4096')
    expect(region.textContent).toContain('a'.repeat(64))
    expect(region.textContent).toContain('aaaaaaaa')
    expect(region.textContent).toContain('unix')
  })

  it('renders "unreported" for a null modifiedAt', async () => {
    const evidence: Evidence = { ...SAMPLE_EVIDENCE, modifiedAt: null }
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return { candidateId: 7, evidence }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))
    await screen.findByRole('heading', { name: 'Candidate evidence' })

    expect(screen.getByText(/Modified:/).textContent).toContain('unreported')
  })

  it('renders the windows platform evidence variant with extension and attributes', async () => {
    const evidence: Evidence = {
      ...SAMPLE_EVIDENCE,
      platform: { os: 'windows', extension: '.exe', attributes: 32 },
    }
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return { candidateId: 7, evidence }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))
    await screen.findByRole('heading', { name: 'Candidate evidence' })

    const platformLine = screen.getByText(/Platform:/).textContent
    expect(platformLine).toContain('windows')
    expect(platformLine).toContain('extension .exe')
    expect(platformLine).toContain('attributes 32')
  })

  it('renders the formatted modified time and the unix platform line with the octal mode (R3-015)', async () => {
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return { candidateId: 7, evidence: SAMPLE_EVIDENCE }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))
    await screen.findByRole('heading', { name: 'Candidate evidence' })

    expect(screen.getByText(/Modified:/).textContent).toBe(
      `Modified: ${new Date(SAMPLE_EVIDENCE.modifiedAt as number).toISOString()}`,
    )
    // SAMPLE_EVIDENCE.platform is { os: 'unix', mode: 0o755 } -- 0o755 in
    // octal is the literal '755' asserted here.
    expect(screen.getByText(/Platform:/).textContent).toBe('Platform: unix (mode 755)')
  })

  it('keeps Approve disabled until the short digest is typed exactly', async () => {
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return { candidateId: 7, evidence: SAMPLE_EVIDENCE }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))
    await screen.findByRole('heading', { name: 'Candidate evidence' })

    const approveButton = screen.getByRole('button', { name: 'Approve' }) as HTMLButtonElement
    const confirmInput = screen.getByLabelText(/type the short digest/i)

    expect(approveButton.disabled).toBe(true)

    fireEvent.change(confirmInput, { target: { value: 'aaaaaaa' } })
    expect(approveButton.disabled).toBe(true)

    fireEvent.change(confirmInput, { target: { value: 'aaaaaaaa' } })
    expect(approveButton.disabled).toBe(false)
  })

  it('rejects the fixed phrase "approve" as a confirmation shortcut -- only the exact short digest enables Approve', async () => {
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return { candidateId: 7, evidence: SAMPLE_EVIDENCE }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))
    await screen.findByRole('heading', { name: 'Candidate evidence' })

    const approveButton = screen.getByRole('button', { name: 'Approve' }) as HTMLButtonElement
    const confirmInput = screen.getByLabelText(/type the short digest/i)

    fireEvent.change(confirmInput, { target: { value: 'approve' } })
    expect(approveButton.disabled).toBe(true)

    fireEvent.change(confirmInput, { target: { value: SAMPLE_EVIDENCE.sha256Short } })
    expect(approveButton.disabled).toBe(false)
  })

  it('leaves Approve disabled when the short digest is typed in a different case (R3-012)', async () => {
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return { candidateId: 7, evidence: SAMPLE_EVIDENCE }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))
    await screen.findByRole('heading', { name: 'Candidate evidence' })

    const approveButton = screen.getByRole('button', { name: 'Approve' }) as HTMLButtonElement
    const confirmInput = screen.getByLabelText(/type the short digest/i)

    fireEvent.change(confirmInput, {
      target: { value: SAMPLE_EVIDENCE.sha256Short.toUpperCase() },
    })
    expect(approveButton.disabled).toBe(true)
  })

  it('renders the act-as identity line before the confirmation input (TM-001-R7)', async () => {
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return { candidateId: 7, evidence: SAMPLE_EVIDENCE }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))
    await screen.findByRole('heading', { name: 'Candidate evidence' })

    const actAsLine = screen.getByText("Approving as: this device's local user")
    const confirmInput = screen.getByLabelText(/type the short digest/i)
    expect(
      actAsLine.compareDocumentPosition(confirmInput) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()
  })

  it('approving invokes executable_approve with the candidateId and lists the record', async () => {
    let capturedArgs: Record<string, unknown> | undefined
    mockIPC((cmd, args) => {
      if (cmd === 'approvals_list') return []
      if (cmd === 'executable_pick_and_probe') {
        return { candidateId: 7, evidence: SAMPLE_EVIDENCE }
      }
      if (cmd === 'executable_approve') {
        capturedArgs = args as Record<string, unknown>
        return sampleApproval({ approvalId: 42 })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))
    await screen.findByRole('heading', { name: 'Candidate evidence' })

    fireEvent.change(screen.getByLabelText(/type the short digest/i), {
      target: { value: 'aaaaaaaa' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }))

    await waitFor(() => {
      expect(capturedArgs).toEqual({ candidateId: 7 })
    })

    const approvalsList = await screen.findByRole('list', { name: 'Approvals list' })
    expect(approvalsList.textContent).toContain('/opt/tool/<b>x</b>')
  })

  it('Revoke invokes executable_revoke for that record', async () => {
    let capturedArgs: Record<string, unknown> | undefined
    let listCallCount = 0
    mockIPC((cmd, args) => {
      if (cmd === 'approvals_list') {
        listCallCount += 1
        if (listCallCount === 1) return [sampleApproval({ approvalId: 42 })]
        return [sampleApproval({ approvalId: 42, status: 'revoked', revokedAt: 600 })]
      }
      if (cmd === 'executable_revoke') {
        capturedArgs = args as Record<string, unknown>
        return null
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    await screen.findByText('/opt/tool/<b>x</b>')

    fireEvent.click(screen.getByRole('button', { name: 'Revoke' }))

    await waitFor(() => {
      expect(capturedArgs).toEqual({ approvalId: 42 })
    })
    await waitFor(() => {
      expect(screen.queryByRole('button', { name: 'Revoke' })).toBeNull()
    })
  })

  it('disables Revoke while a revoke is pending, so a double-click fires only one invoke', async () => {
    let revokeCallCount = 0
    let listCallCount = 0
    let resolveRevoke: (() => void) | undefined
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') {
        listCallCount += 1
        if (listCallCount === 1) return [sampleApproval({ approvalId: 42 })]
        return [sampleApproval({ approvalId: 42, status: 'revoked', revokedAt: 600 })]
      }
      if (cmd === 'executable_revoke') {
        revokeCallCount += 1
        return new Promise<null>((resolve) => {
          resolveRevoke = () => resolve(null)
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    await screen.findByText('/opt/tool/<b>x</b>')

    const revokeButton = screen.getByRole('button', { name: 'Revoke' }) as HTMLButtonElement
    fireEvent.click(revokeButton)
    expect(revokeButton.disabled).toBe(true)

    fireEvent.click(revokeButton)
    expect(revokeCallCount).toBe(1)

    resolveRevoke?.()
    await waitFor(() => {
      expect(screen.queryByRole('button', { name: 'Revoke' })).toBeNull()
    })
  })

  it('shows both digests and a Renew action, and no proceed affordance, for a changed-since-approval error', async () => {
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_approve') {
        return Promise.reject({
          code: 'changed-since-approval',
          message: "the executable's content has changed since it was approved",
          detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
        })
      }
      if (cmd === 'executable_pick_and_probe') {
        return { candidateId: 7, evidence: SAMPLE_EVIDENCE }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))
    await screen.findByRole('heading', { name: 'Candidate evidence' })
    fireEvent.change(screen.getByLabelText(/type the short digest/i), {
      target: { value: SAMPLE_EVIDENCE.sha256Short },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toContain('aaaaaaaa')
    expect(banner.textContent).toContain('bbbbbbbb')
    expect(screen.getByRole('button', { name: 'Renew approval' })).toBeTruthy()
    expect(screen.queryByText(/proceed anyway/i)).toBeNull()
    expect(screen.queryByRole('button', { name: /proceed/i })).toBeNull()
  })

  it('Renew approval re-runs pick-and-probe then approve', async () => {
    let pickCount = 0
    let approveCallCount = 0
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        pickCount += 1
        return { candidateId: pickCount, evidence: SAMPLE_EVIDENCE }
      }
      if (cmd === 'executable_approve') {
        approveCallCount += 1
        if (approveCallCount === 1) {
          return Promise.reject({
            code: 'changed-since-approval',
            message: "the executable's content has changed since it was approved",
            detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
          })
        }
        return sampleApproval({ approvalId: 99 })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))
    await screen.findByRole('heading', { name: 'Candidate evidence' })
    fireEvent.change(screen.getByLabelText(/type the short digest/i), {
      target: { value: SAMPLE_EVIDENCE.sha256Short },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }))
    await screen.findByRole('button', { name: 'Renew approval' })

    fireEvent.click(screen.getByRole('button', { name: 'Renew approval' }))

    await waitFor(() => {
      expect(pickCount).toBe(2)
    })
    await waitFor(() => {
      expect(approveCallCount).toBe(2)
    })
    await waitFor(() => {
      expect(screen.queryByRole('alert')).toBeNull()
    })
  })

  it('shows the code for a too-large error', async () => {
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return Promise.reject({
          code: 'too-large',
          message: 'the executable exceeds the size limit',
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toContain('too-large')
  })

  it('shows the code for a not-executable error', async () => {
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return Promise.reject({
          code: 'not-executable',
          message: 'the target is not a regular, executable file',
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toContain('not-executable')
  })

  it('no-ops after unmount: a pending pick-and-probe resolving post-unmount throws nothing', async () => {
    let resolvePick: (value: unknown) => void = () => {}
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return new Promise((resolve) => {
          resolvePick = resolve
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const { unmount } = render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    unmount()

    expect(() => {
      resolvePick({ candidateId: 7, evidence: SAMPLE_EVIDENCE })
    }).not.toThrow()

    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('no-ops after unmount: a pending executable_approve resolving post-unmount throws nothing', async () => {
    let resolveApprove: (value: unknown) => void = () => {}
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return { candidateId: 7, evidence: SAMPLE_EVIDENCE }
      }
      if (cmd === 'executable_approve') {
        return new Promise((resolve) => {
          resolveApprove = resolve
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const { unmount } = render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))
    await screen.findByRole('heading', { name: 'Candidate evidence' })
    fireEvent.change(screen.getByLabelText(/type the short digest/i), {
      target: { value: SAMPLE_EVIDENCE.sha256Short },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }))

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    unmount()

    expect(() => {
      resolveApprove(sampleApproval({ approvalId: 42 }))
    }).not.toThrow()

    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('no-ops after unmount: a pending executable_revoke resolving post-unmount throws nothing', async () => {
    let resolveRevoke: (value: unknown) => void = () => {}
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') return [sampleApproval({ approvalId: 42 })]
      if (cmd === 'executable_revoke') {
        return new Promise((resolve) => {
          resolveRevoke = resolve
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const { unmount } = render(<ApprovalSurface />)
    await screen.findByText('/opt/tool/<b>x</b>')
    fireEvent.click(screen.getByRole('button', { name: 'Revoke' }))

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    unmount()

    expect(() => {
      resolveRevoke(null)
    }).not.toThrow()

    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('no-ops after unmount: a pending renew re-approve resolving post-unmount throws nothing', async () => {
    let approveCallCount = 0
    let resolveSecondApprove: (value: unknown) => void = () => {}
    mockWithEmptyApprovalsList((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return { candidateId: 7, evidence: SAMPLE_EVIDENCE }
      }
      if (cmd === 'executable_approve') {
        approveCallCount += 1
        if (approveCallCount === 1) {
          return Promise.reject({
            code: 'changed-since-approval',
            message: "the executable's content has changed since it was approved",
            detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
          })
        }
        return new Promise((resolve) => {
          resolveSecondApprove = resolve
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const { unmount } = render(<ApprovalSurface />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick executable' }))
    await screen.findByRole('heading', { name: 'Candidate evidence' })
    fireEvent.change(screen.getByLabelText(/type the short digest/i), {
      target: { value: SAMPLE_EVIDENCE.sha256Short },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }))
    await screen.findByRole('button', { name: 'Renew approval' })

    fireEvent.click(screen.getByRole('button', { name: 'Renew approval' }))
    await waitFor(() => {
      expect(approveCallCount).toBe(2)
    })

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    unmount()

    expect(() => {
      resolveSecondApprove(sampleApproval({ approvalId: 99 }))
    }).not.toThrow()

    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })
})
