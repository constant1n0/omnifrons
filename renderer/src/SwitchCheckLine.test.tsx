// ADR-0005 slice 4, sub-slice 4c: the switch check's verdict line. Mocked at
// the Tauri IPC boundary with `mockIPC`, the convention `DefinitionSection.test.tsx`
// documents.

import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { SwitchCheckLine } from './SwitchCheckLine'
import type { DefinitionHost, DefinitionSwitchCheck, DefinitionSwitchClaim, ShellError } from './ipc/harness'

const CLAIM: DefinitionSwitchClaim = {
  host: 'Claude Code',
  source: 'https://example.invalid/docs',
  accessed: '2026-09-25',
  status: 'Verified',
}

const REFERENCE = { path: 'AGENTS.md', digest: 'ab'.repeat(32) }

const REACHES: DefinitionSwitchCheck = {
  schema: 'omnifrons.definition-switch-check/1',
  adapter: 'claude-code',
  host: 'Claude Code',
  verdict: { kind: 'reaches', file: 'CLAUDE.md', line: 'Claude Code reaches the definition through CLAUDE.md', claim: CLAIM },
  reference: REFERENCE,
}

const UNREACHED: DefinitionSwitchCheck = {
  schema: 'omnifrons.definition-switch-check/1',
  adapter: 'claude-code',
  host: 'Claude Code',
  verdict: {
    kind: 'unreached',
    expected: ['CLAUDE.md'],
    shadowedBy: null,
    fallback: null,
    line: 'Claude Code does not reach the definition: no CLAUDE.md',
    claim: CLAIM,
  },
  reference: null,
}

const NO_HOST: DefinitionSwitchCheck = {
  schema: 'omnifrons.definition-switch-check/1',
  adapter: 'pty-cli',
  host: null,
  verdict: { kind: 'uncertain', reason: 'no-host-chosen', name: null, line: 'Choose the host this adapter runs' },
  reference: REFERENCE,
}

const NAMES = [
  'Claude Code',
  'Codex',
  'OpenCode',
  'Kimi',
  'Gemini CLI',
  'Cursor',
  'Copilot',
  'Windsurf',
  'Aider',
]

const HOSTS: DefinitionHost[] = NAMES.map((name) => ({
  name,
  files: ['AGENTS.md'],
  fallbackFiles: [],
  accessed: '2026-09-25',
  status: 'Verified',
  source: 'https://example.invalid/docs',
}))

const SHELL_ERROR: ShellError = { code: 'workspace-unavailable', message: 'no workspace is active' }

afterEach(() => {
  cleanup()
  clearMocks()
})

describe('SwitchCheckLine', () => {
  it('renders nothing without an adapter and calls nothing', async () => {
    const calls: string[] = []
    mockIPC((cmd) => {
      calls.push(cmd)
      throw new Error(`unexpected command: ${cmd}`)
    })
    const { container } = render(<SwitchCheckLine adapterId={null} disabled={false} />)
    await act(async () => {})
    expect(container.innerHTML).toBe('')
    expect(calls).toEqual([])
  })

  it('checks claude-code with a null host and renders the marker, the line and the reference', async () => {
    const requests: unknown[] = []
    mockIPC((cmd, args) => {
      if (cmd === 'definition_hosts') return HOSTS
      if (cmd === 'definition_switch_check') {
        requests.push(args)
        return REACHES
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(<SwitchCheckLine adapterId="claude-code" disabled={false} />)

    const line = await screen.findByLabelText('Switch check')
    expect((await within(line).findByText('reaches')).tagName).toBe('STRONG')
    expect(line.textContent).toContain('Claude Code reaches the definition through CLAUDE.md')
    expect(line.textContent).toContain('definition AGENTS.md@abababababab')
    expect(line.textContent).not.toContain('abababababab' + 'ab')
    expect(requests).toEqual([{ adapterId: 'claude-code', host: null }])
    expect(screen.queryByLabelText('Target host')).toBeNull()
  })

  it('renders an unreached verdict line without a reference', async () => {
    mockIPC((cmd) => {
      if (cmd === 'definition_hosts') return HOSTS
      if (cmd === 'definition_switch_check') return UNREACHED
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(<SwitchCheckLine adapterId="claude-code" disabled={false} />)

    const line = await screen.findByLabelText('Switch check')
    expect((await within(line).findByText('unreached')).tagName).toBe('STRONG')
    expect(line.textContent).toContain('Claude Code does not reach the definition: no CLAUDE.md')
    expect(line.textContent).not.toContain('definition AGENTS.md@')
  })

  it('offers the nine hosts for a generic adapter and checks again with the chosen one', async () => {
    const requests: Array<{ adapterId: string; host: string | null }> = []
    mockIPC((cmd, args) => {
      if (cmd === 'definition_hosts') return HOSTS
      if (cmd === 'definition_switch_check') {
        const request = args as { adapterId: string; host: string | null }
        requests.push(request)
        return request.host === null ? NO_HOST : { ...REACHES, adapter: 'pty-cli', host: request.host }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(<SwitchCheckLine adapterId="pty-cli" disabled={false} />)

    const select = (await screen.findByLabelText('Target host')) as HTMLSelectElement
    await waitFor(() => {
      expect(Array.from(select.options).map((option) => option.textContent)).toEqual(['Choose a host', ...NAMES])
    })
    expect(await screen.findByText('uncertain')).toBeTruthy()

    fireEvent.change(select, { target: { value: 'Gemini CLI' } })

    expect(await screen.findByText('reaches')).toBeTruthy()
    expect(requests).toEqual([
      { adapterId: 'pty-cli', host: null },
      { adapterId: 'pty-cli', host: 'Gemini CLI' },
    ])
  })

  it('discards a response overtaken by a change of adapter', async () => {
    let releaseFirst: (check: DefinitionSwitchCheck) => void = () => {}
    mockIPC((cmd, args) => {
      if (cmd === 'definition_hosts') return HOSTS
      if (cmd === 'definition_switch_check') {
        const { adapterId } = args as { adapterId: string }
        if (adapterId === 'claude-code') {
          return new Promise<DefinitionSwitchCheck>((resolve) => {
            releaseFirst = resolve
          })
        }
        return { ...UNREACHED, adapter: adapterId, host: 'Codex' }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    const { rerender } = render(<SwitchCheckLine adapterId="claude-code" disabled={false} />)
    await waitFor(() => {
      expect(screen.getByLabelText('Switch check').textContent).toContain('Checking')
    })

    rerender(<SwitchCheckLine adapterId="codex-cli" disabled={false} />)
    expect(await screen.findByText('unreached')).toBeTruthy()

    await act(async () => {
      releaseFirst(REACHES)
    })
    expect(screen.queryByText('reaches')).toBeNull()
    expect(screen.getByText('unreached')).toBeTruthy()
  })

  it('renders a ShellError as its code and message', async () => {
    mockIPC((cmd) => {
      if (cmd === 'definition_hosts') return HOSTS
      if (cmd === 'definition_switch_check') return Promise.reject(SHELL_ERROR)
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(<SwitchCheckLine adapterId="claude-code" disabled={false} />)

    const line = await screen.findByLabelText('Switch check')
    await waitFor(() => {
      expect(line.textContent).toContain('workspace-unavailable')
    })
    expect(line.textContent).toContain('no workspace is active')
  })

  it('renders a rejected host catalogue as an error line and refetches it on retry', async () => {
    let catalogueCalls = 0
    mockIPC((cmd, args) => {
      if (cmd === 'definition_hosts') {
        catalogueCalls += 1
        return catalogueCalls === 1 ? Promise.reject(SHELL_ERROR) : HOSTS
      }
      if (cmd === 'definition_switch_check') {
        const request = args as { adapterId: string; host: string | null }
        return request.host === null ? NO_HOST : REACHES
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(<SwitchCheckLine adapterId="pty-cli" disabled={false} />)

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toContain('workspace-unavailable')
    expect(alert.textContent).toContain('no workspace is active')
    const select = screen.getByLabelText('Target host') as HTMLSelectElement
    expect(select.options).toHaveLength(1)

    fireEvent.click(screen.getByRole('button', { name: 'Retry' }))

    await waitFor(() => {
      expect(select.options).toHaveLength(NAMES.length + 1)
    })
    expect(screen.queryByRole('alert')).toBeNull()
    expect(catalogueCalls).toBe(2)
  })

  it('renders a rejection that is not a ShellError as an unexpected error', async () => {
    mockIPC((cmd) => {
      if (cmd === 'definition_hosts') return HOSTS
      if (cmd === 'definition_switch_check') return Promise.reject(new Error('boom'))
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(<SwitchCheckLine adapterId="claude-code" disabled={false} />)

    const line = await screen.findByLabelText('Switch check')
    await waitFor(() => {
      expect(line.textContent).toContain('unexpected error')
    })
    expect(line.textContent).not.toContain('boom')
  })

  it('keeps the host select mounted while a reset to the placeholder is re-checked', async () => {
    let releaseSecond: (check: DefinitionSwitchCheck) => void = () => {}
    let calls = 0
    mockIPC((cmd, args) => {
      if (cmd === 'definition_hosts') return HOSTS
      if (cmd === 'definition_switch_check') {
        calls += 1
        const request = args as { host: string | null }
        if (request.host === null && calls > 1) {
          return new Promise<DefinitionSwitchCheck>((resolve) => {
            releaseSecond = resolve
          })
        }
        return request.host === null ? NO_HOST : REACHES
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(<SwitchCheckLine adapterId="pty-cli" disabled={false} />)
    const select = (await screen.findByLabelText('Target host')) as HTMLSelectElement
    await waitFor(() => {
      expect(select.options).toHaveLength(NAMES.length + 1)
    })
    fireEvent.change(select, { target: { value: 'Gemini CLI' } })
    await screen.findByText('reaches')

    fireEvent.change(select, { target: { value: '' } })

    await waitFor(() => {
      expect(screen.getByLabelText('Switch check').textContent).toContain('Checking')
    })
    expect(screen.getByLabelText('Target host')).toBe(select)
    await act(async () => {
      releaseSecond(NO_HOST)
    })
    expect(await screen.findByText('uncertain')).toBeTruthy()
    expect(screen.getByLabelText('Target host')).toBe(select)
  })

  it('freezes the host select while disabled and asks nothing more on a change', async () => {
    const requests: unknown[] = []
    mockIPC((cmd, args) => {
      if (cmd === 'definition_hosts') return HOSTS
      if (cmd === 'definition_switch_check') {
        requests.push(args)
        return NO_HOST
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(<SwitchCheckLine adapterId="pty-cli" disabled={true} />)

    const select = (await screen.findByLabelText('Target host')) as HTMLSelectElement
    await screen.findByText('uncertain')
    expect(select.disabled).toBe(true)

    fireEvent.change(select, { target: { value: 'Gemini CLI' } })
    await act(async () => {})

    expect(requests).toEqual([{ adapterId: 'pty-cli', host: null }])
    expect(select.value).toBe('')
  })
})
