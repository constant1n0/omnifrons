import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { ApprovalSurface } from './ApprovalSurface'
import { HarnessPanel } from './HarnessPanel'
import type { Approval, Evidence, HarnessFrame } from './ipc/harness'

// The jsdom crypto polyfill and React Testing Library's `cleanup()` are
// installed once for every test file by `testSupport/setup.ts`
// (`vite.config.ts`'s `test.setupFiles`) -- not repeated here.

const SESSION_STORAGE_KEY = 'omnifrons.harness.processIds'

const SAMPLE_EVIDENCE: Evidence = {
  canonicalPath: '/opt/tool/app',
  size: 4096,
  sha256: 'a'.repeat(64),
  sha256Short: 'aaaaaaaa',
  modifiedAt: 1_000,
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

beforeEach(() => {
  window.sessionStorage.clear()
})

afterEach(() => {
  clearMocks()
})

/** Starts the harness and returns the live Channel captured from the mock. */
async function startAndCaptureChannel(
  onCommand?: (cmd: string, args: Record<string, unknown>) => unknown,
): Promise<{ onmessage: (frame: HarnessFrame) => void }> {
  let channelRef: { onmessage: (frame: HarnessFrame) => void } | undefined
  mockIPC((cmd, args) => {
    if (cmd === 'harness_spawn') {
      channelRef = (args as { onFrame: { onmessage: (frame: HarnessFrame) => void } }).onFrame
      return 7
    }
    if (onCommand) return onCommand(cmd, args as Record<string, unknown>)
    throw new Error(`unexpected command: ${cmd}`)
  })

  render(<HarnessPanel />)
  fireEvent.click(screen.getByRole('button', { name: 'Start' }))
  await waitFor(() => {
    expect(channelRef).toBeDefined()
  })
  await screen.findByText('running')

  if (!channelRef) throw new Error('harness_spawn was not called')
  return channelRef
}

/**
 * Switches Kind to `approved` and selects approval `value` only once its
 * option exists -- the approvals list is fetched lazily on that switch, so
 * changing the select before it arrives silently selects nothing (the
 * value falls back to "") and leaves Start disabled.
 */
async function selectApprovedKindWithApproval(value: string): Promise<void> {
  fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'approved' } })
  const approvalSelect = (await screen.findByLabelText('Approval')) as HTMLSelectElement
  await waitFor(() => {
    expect(Array.from(approvalSelect.options).map((option) => option.value)).toContain(value)
  })
  fireEvent.change(approvalSelect, { target: { value } })
  expect(approvalSelect.value).toBe(value)
}

describe('HarnessPanel', () => {
  it('renders the kind select, bounded rate/lines inputs, and the controls', () => {
    render(<HarnessPanel />)

    const kindSelect = screen.getByLabelText('Kind') as HTMLSelectElement
    const options = Array.from(kindSelect.options).map((option) => option.value)
    expect(options).toEqual(['demo-lines', 'demo-ignores-sigterm', 'approved'])

    const rateInput = screen.getByLabelText('Rate (Hz)') as HTMLInputElement
    expect(rateInput.min).toBe('1')
    expect(rateInput.max).toBe('1000')

    const linesInput = screen.getByLabelText('Lines') as HTMLInputElement
    expect(linesInput.min).toBe('1')
    expect(linesInput.max).toBe('100000')

    expect(screen.getByRole('button', { name: 'Start' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Stop' })).toBeTruthy()
  })

  it('disables Start and shows a validation message when Rate (Hz) is empty', () => {
    render(<HarnessPanel />)
    const rateInput = screen.getByLabelText('Rate (Hz)') as HTMLInputElement

    fireEvent.change(rateInput, { target: { value: '' } })

    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    expect(screen.getByTestId('rate-hz-validation')).toBeTruthy()
  })

  it('disables Start and shows a validation message when Rate (Hz) is negative', () => {
    render(<HarnessPanel />)
    const rateInput = screen.getByLabelText('Rate (Hz)') as HTMLInputElement

    fireEvent.change(rateInput, { target: { value: '-5' } })

    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    expect(screen.getByTestId('rate-hz-validation')).toBeTruthy()
  })

  it('disables Start and shows a validation message when Rate (Hz) is non-numeric', () => {
    render(<HarnessPanel />)
    const rateInput = screen.getByLabelText('Rate (Hz)') as HTMLInputElement

    fireEvent.change(rateInput, { target: { value: 'abc' } })

    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    expect(screen.getByTestId('rate-hz-validation')).toBeTruthy()
  })

  it('disables Start and shows a validation message when Lines is above the maximum', () => {
    render(<HarnessPanel />)
    const linesInput = screen.getByLabelText('Lines') as HTMLInputElement

    fireEvent.change(linesInput, { target: { value: '100001' } })

    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    expect(screen.getByTestId('lines-validation')).toBeTruthy()
  })

  it('disables Start and shows a validation message when Rate (Hz) has a decimal point', () => {
    render(<HarnessPanel />)
    const rateInput = screen.getByLabelText('Rate (Hz)') as HTMLInputElement

    fireEvent.change(rateInput, { target: { value: '1.5' } })

    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    expect(screen.getByTestId('rate-hz-validation')).toBeTruthy()
  })

  it('disables Start and shows a validation message when Rate (Hz) has trailing non-digit characters', () => {
    render(<HarnessPanel />)
    const rateInput = screen.getByLabelText('Rate (Hz)') as HTMLInputElement

    fireEvent.change(rateInput, { target: { value: '10abc' } })

    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    expect(screen.getByTestId('rate-hz-validation')).toBeTruthy()
  })

  it('re-enables Start once an invalid Rate (Hz) is corrected back into range', () => {
    render(<HarnessPanel />)
    const rateInput = screen.getByLabelText('Rate (Hz)') as HTMLInputElement

    fireEvent.change(rateInput, { target: { value: '' } })
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    )

    fireEvent.change(rateInput, { target: { value: '10' } })
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
    expect(screen.queryByTestId('rate-hz-validation')).toBeNull()
  })

  it('renders frame text literally: a <b> tag produces no element, text stays visible', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'stdout',
        body: { id: 7, seq: 0, droppedBefore: 0, continued: false, text: '<b>bold</b>' },
      })
    })

    const log = screen.getByLabelText('Output log')
    expect(log.querySelector('b')).toBeNull()
    expect(log.textContent).toContain('<b>bold</b>')
  })

  it('strips ESC/BEL from an OSC 8 hyperlink, produces no <a>, and keeps the rest of the bytes literally', async () => {
    const channel = await startAndCaptureChannel()

    const esc = String.fromCharCode(0x1b)
    const bel = String.fromCharCode(0x07)
    const osc8 = `${esc}]8;;https://example.invalid${bel}label${esc}]8;;${bel}`

    act(() => {
      channel.onmessage({
        stream: 'stdout',
        body: { id: 7, seq: 1, droppedBefore: 0, continued: false, text: osc8 },
      })
    })

    const log = screen.getByLabelText('Output log')
    expect(log.querySelector('a')).toBeNull()
    // ESC and BEL (both C0 control bytes) are stripped; every other byte,
    // including the OSC marker's own punctuation, survives literally --
    // there is no OSC interpretation, only per-byte control stripping.
    // Exact equality (not `toContain`): proves nothing extra -- no leading
    // marker, no stray whitespace -- sneaks into the rendered line either.
    expect(log.textContent).toBe('stdout: ]8;;https://example.invalidlabel]8;;')
  })

  it('renders a dropped-frames marker when droppedBefore is nonzero', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'stdout',
        body: { id: 7, seq: 12, droppedBefore: 7, continued: false, text: 'line 13' },
      })
    })

    const mark = screen.getByText('7 frames dropped before this line')
    expect(mark.tagName).toBe('MARK')
  })

  it('bounds the log to the last 2000 entries and shows a cumulative trimmed-lines marker', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      for (let seq = 0; seq < 2500; seq += 1) {
        channel.onmessage({
          stream: 'stdout',
          body: { id: 7, seq, droppedBefore: 0, continued: false, text: `line ${seq}` },
        })
      }
    })

    const log = screen.getByLabelText('Output log')
    const marker = screen.getByText(
      'showing the last 2000 lines; 500 earlier lines are not shown',
    )
    expect(marker.tagName).toBe('MARK')

    // one marker row plus the 2000 surviving entry rows.
    expect(log.querySelectorAll('li').length).toBe(2001)
    // lines 0-499 were trimmed; the oldest surviving line is 500.
    expect(log.textContent).toContain('stdout: line 500')
    expect(log.textContent).not.toContain('stdout: line 499')
  })

  it('shows the killed badge after a mocked harness_stop resolves', async () => {
    const channel = await startAndCaptureChannel((cmd, args) => {
      if (cmd === 'harness_stop') {
        expect(args).toEqual({ id: 7, deadlineMs: 2000 })
        return { state: 'killed', code: null }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    void channel

    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))

    await screen.findByText('killed')
  })

  it('resets activeId and disables Stop once harness_stop resolves', async () => {
    const channel = await startAndCaptureChannel((cmd) => {
      if (cmd === 'harness_stop') {
        return { state: 'killed', code: null }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    void channel

    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await screen.findByText('killed')

    const stopButton = screen.getByRole('button', { name: 'Stop' }) as HTMLButtonElement
    expect(stopButton.disabled).toBe(true)
    // The id is forgotten after a successful stop: nothing left to
    // reconnect to (R3-003), so it must not linger in storage (R3-015).
    expect(window.sessionStorage.getItem(SESSION_STORAGE_KEY)).toBe('[]')
  })

  it('disables Start while a spawn is pending, and a second click does not spawn twice', async () => {
    let spawnCount = 0
    let resolveSpawn: (id: number) => void = () => {}
    mockIPC((cmd) => {
      if (cmd === 'harness_spawn') {
        spawnCount += 1
        return new Promise<number>((resolve) => {
          resolveSpawn = resolve
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    const startButton = screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement

    fireEvent.click(startButton)
    expect(startButton.disabled).toBe(true)

    fireEvent.click(startButton)
    await waitFor(() => {
      expect(spawnCount).toBe(1)
    })

    await act(async () => {
      resolveSpawn(7)
      await Promise.resolve()
    })
    await screen.findByText('running')

    // One active harness per panel (R3-004): Start stays disabled once a
    // harness is running, not just while the spawn itself is pending.
    expect(startButton.disabled).toBe(true)
  })

  it('shows "exited (code N)" when the exit code is a number', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'state',
        body: { id: 7, seq: 4, droppedBefore: 0, state: 'exited', code: 0 },
      })
    })

    await screen.findByText('exited (code 0)')
  })

  it('shows "exited (code unreported)" when the exit code is null', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'state',
        body: { id: 7, seq: 4, droppedBefore: 0, state: 'exited', code: null },
      })
    })

    await screen.findByText('exited (code unreported)')
  })

  it('renders the orphan-risk/uncertain token verbatim from a state frame, never "done"', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'state',
        body: { id: 7, seq: 3, droppedBefore: 0, state: 'orphan-risk/uncertain', code: null },
      })
    })

    await screen.findByText('orphan-risk/uncertain')
    expect(screen.queryByText('done')).toBeNull()
    expect(document.body.textContent).not.toContain('done')
  })

  it('shows the error code and message when harness_spawn is rejected, with no path in the message', async () => {
    mockIPC((cmd) => {
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'spawn-failed',
          message: 'failed to start the requested process',
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    await screen.findByText('spawn-failed')
    const banner = screen.getByRole('alert')
    expect(banner.textContent).toContain('spawn-failed')
    expect(banner.textContent).toContain('failed to start the requested process')
    expect(banner.textContent).not.toContain('/')
  })

  it('strips control characters from the error banner code and message', async () => {
    const esc = String.fromCharCode(0x1b)
    mockIPC((cmd) => {
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: `spawn${esc}-failed`,
          message: `failed${esc} to start`,
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('spawn-failed: failed to start')
  })

  it('shows a generic "unexpected error" banner, with no message text, when harness_spawn rejects with a non-ShellError value', async () => {
    mockIPC((cmd) => {
      if (cmd === 'harness_spawn') {
        return Promise.reject(new Error('boom: /home/someone/secret/path'))
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('unexpected error')
  })

  it('no-ops after unmount: a frame delivered post-unmount throws nothing and logs no console.error', async () => {
    let channelRef: { onmessage: (frame: HarnessFrame) => void } | undefined
    mockIPC((cmd, args) => {
      if (cmd === 'harness_spawn') {
        channelRef = (args as { onFrame: { onmessage: (frame: HarnessFrame) => void } }).onFrame
        return 7
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const { unmount } = render(<HarnessPanel />)
    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    if (!channelRef) throw new Error('harness_spawn was not called')

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    unmount()

    expect(() => {
      channelRef!.onmessage({
        stream: 'stdout',
        body: { id: 7, seq: 0, droppedBefore: 0, continued: false, text: 'after unmount' },
      })
    }).not.toThrow()

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('shows a reconnected marker on mount for a remembered id still running', async () => {
    window.sessionStorage.setItem(SESSION_STORAGE_KEY, JSON.stringify([99]))
    mockIPC((cmd, args) => {
      if (cmd === 'harness_observe') {
        expect(args).toEqual({ id: 99 })
        return { status: 'running' }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)

    await screen.findByText('Process 99: stream reconnected; earlier output not replayed')
  })

  it('prunes a remembered id from storage when harness_observe reports a terminal status', async () => {
    window.sessionStorage.setItem(SESSION_STORAGE_KEY, JSON.stringify([99]))
    mockIPC((cmd) => {
      if (cmd === 'harness_observe') {
        return { status: 'terminal', state: 'killed', code: null }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)

    await waitFor(() => {
      expect(window.sessionStorage.getItem(SESSION_STORAGE_KEY)).toBe('[]')
    })
  })

  it('forgets a stale remembered id silently (no banner) when harness_observe reports unknown-process', async () => {
    window.sessionStorage.setItem(SESSION_STORAGE_KEY, JSON.stringify([99]))
    mockIPC((cmd) => {
      if (cmd === 'harness_observe') {
        return Promise.reject({ code: 'unknown-process', message: 'no such process' })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)

    await waitFor(() => {
      expect(window.sessionStorage.getItem(SESSION_STORAGE_KEY)).toBe('[]')
    })
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('shows the banner once and prunes the id when harness_observe rejects with a non-stale ShellError', async () => {
    window.sessionStorage.setItem(SESSION_STORAGE_KEY, JSON.stringify([99]))
    mockIPC((cmd) => {
      if (cmd === 'harness_observe') {
        return Promise.reject({ code: 'invalid-request', message: 'bad request' })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)

    const banners = await screen.findAllByTestId('harness-error-banner')
    expect(banners).toHaveLength(1)
    expect(banners[0]?.textContent).toBe('invalid-request: bad request')
    await waitFor(() => {
      expect(window.sessionStorage.getItem(SESSION_STORAGE_KEY)).toBe('[]')
    })
  })

  it('the mount-time observe effect no-ops after unmount: no sessionStorage write once the component is gone', async () => {
    window.sessionStorage.setItem(SESSION_STORAGE_KEY, JSON.stringify([99]))
    let observeCalled = false
    let resolveObserve: (value: unknown) => void = () => {}
    mockIPC((cmd, args) => {
      if (cmd === 'harness_observe') {
        expect(args).toEqual({ id: 99 })
        observeCalled = true
        return new Promise((resolve) => {
          resolveObserve = resolve
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const { unmount } = render(<HarnessPanel />)
    await waitFor(() => {
      expect(observeCalled).toBe(true)
    })
    unmount()

    resolveObserve({ status: 'terminal', state: 'killed', code: null })
    // Flush every pending microtask in the invoke -> harnessObserve -> .then
    // chain (each `async function` hop schedules its own microtask) before
    // asserting nothing changed. A macrotask boundary guarantees the
    // microtask queue is fully drained first.
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(window.sessionStorage.getItem(SESSION_STORAGE_KEY)).toBe(JSON.stringify([99]))
  })

  it('hides the Rate/Lines inputs and shows an approval select when Kind is "approved"', async () => {
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') {
        return [sampleApproval({ approvalId: 42 })]
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'approved' } })

    await screen.findByLabelText('Approval')
    expect(screen.queryByLabelText('Rate (Hz)')).toBeNull()
    expect(screen.queryByLabelText('Lines')).toBeNull()
  })

  it('lists only active approvals in the approval select', async () => {
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') {
        return [
          sampleApproval({ approvalId: 1, status: 'active' }),
          sampleApproval({ approvalId: 2, status: 'revoked', revokedAt: 600 }),
        ]
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'approved' } })

    const approvalSelect = (await screen.findByLabelText('Approval')) as HTMLSelectElement
    const values = Array.from(approvalSelect.options).map((option) => option.value)
    expect(values).toContain('1')
    expect(values).not.toContain('2')
  })

  it('renders the approval option label as the control-character-stripped canonical path (R3-013)', async () => {
    const esc = String.fromCharCode(0x1b)
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') {
        return [
          sampleApproval({
            approvalId: 42,
            evidence: { ...SAMPLE_EVIDENCE, canonicalPath: `/opt/tool${esc}/app` },
          }),
        ]
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'approved' } })

    const approvalSelect = (await screen.findByLabelText('Approval')) as HTMLSelectElement
    const option = Array.from(approvalSelect.options).find((candidate) => candidate.value === '42')
    expect(option?.textContent).toBe('/opt/tool/app')
  })

  it('spawns with exactly { kind: { type: "approved", approvalId }, onFrame } for the approved kind', async () => {
    let capturedArgs: Record<string, unknown> | undefined
    mockIPC((cmd, args) => {
      if (cmd === 'approvals_list') {
        return [sampleApproval({ approvalId: 42 })]
      }
      if (cmd === 'harness_spawn') {
        capturedArgs = args as Record<string, unknown>
        return 7
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    await selectApprovedKindWithApproval('42')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')

    expect(capturedArgs).toBeDefined()
    expect(Object.keys(capturedArgs!).sort()).toEqual(['kind', 'onFrame'])
    expect(capturedArgs!.kind).toEqual({ type: 'approved', approvalId: 42 })
  })

  it('shows a denial in the banner with the public "untrusted" wording for a revoked approval', async () => {
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') {
        return [sampleApproval({ approvalId: 42 })]
      }
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'revoked',
          message: 'the approval for this executable was revoked',
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    await selectApprovedKindWithApproval('42')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toContain('untrusted')
    expect(banner.textContent).toContain('revoked')
    expect(banner.textContent).toContain('the approval for this executable was revoked')
  })

  it('shows a denial in the banner with the public "untrusted" wording for an unapproved candidate (R3-014)', async () => {
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') {
        return [sampleApproval({ approvalId: 42 })]
      }
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'unapproved',
          message: 'this executable has not been approved',
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    await selectApprovedKindWithApproval('42')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toContain('untrusted')
    expect(banner.textContent).toContain('unapproved')
    expect(banner.textContent).toContain('this executable has not been approved')
  })

  it('shows a denial in the banner with the public "untrusted" wording for a shadowed path (R3-014)', async () => {
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') {
        return [sampleApproval({ approvalId: 42 })]
      }
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'shadowed-path',
          message: 'the approved path now resolves somewhere else',
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    await selectApprovedKindWithApproval('42')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toContain('untrusted')
    expect(banner.textContent).toContain('shadowed-path')
    expect(banner.textContent).toContain('the approved path now resolves somewhere else')
  })

  it('shows a changed-since-approval denial with both digests in the banner detail', async () => {
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') {
        return [sampleApproval({ approvalId: 42 })]
      }
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'changed-since-approval',
          message: "the executable's content has changed since it was approved",
          detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    await selectApprovedKindWithApproval('42')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toContain('untrusted')
    expect(banner.textContent).toContain('aaaaaaaa')
    expect(banner.textContent).toContain('bbbbbbbb')
  })

  it('clears the error banner and resets the approval selection when switching away from "approved" (R3-007/R3-011)', async () => {
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') return [sampleApproval({ approvalId: 42 })]
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'revoked',
          message: 'the approval for this executable was revoked',
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    await selectApprovedKindWithApproval('42')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByRole('alert')

    fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'demo-lines' } })
    expect(screen.queryByRole('alert')).toBeNull()

    fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'approved' } })
    const reselectedApprovalSelect = (await screen.findByLabelText(
      'Approval',
    )) as HTMLSelectElement
    expect(reselectedApprovalSelect.value).toBe('')
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
  })

  it('ignores a stale approvals_list response that resolves after a newer request (R3-008)', async () => {
    let callCount = 0
    const resolvers: Array<(records: Approval[]) => void> = []
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') {
        callCount += 1
        return new Promise<Approval[]>((resolve) => {
          resolvers[callCount - 1] = resolve
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'approved' } })
    await waitFor(() => {
      expect(callCount).toBe(1)
    })

    fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'demo-lines' } })
    fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'approved' } })
    await waitFor(() => {
      expect(callCount).toBe(2)
    })

    // The newer request (#2) resolves first...
    act(() => {
      resolvers[1]!([sampleApproval({ approvalId: 2 })])
    })
    await waitFor(() => {
      const approvalSelect = screen.getByLabelText('Approval') as HTMLSelectElement
      expect(Array.from(approvalSelect.options).map((option) => option.value)).toContain('2')
    })

    // ...then the stale older request (#1) resolves after it -- it must be
    // ignored, not overwrite the newer, already-applied response. Flushed
    // with a real macrotask tick (not just a synchronous act()) so the
    // stale response's own .then chain -- two async-function hops plus
    // the effect's own .then -- has fully settled before asserting.
    await act(async () => {
      resolvers[0]!([sampleApproval({ approvalId: 1 })])
      await new Promise((resolve) => {
        setTimeout(resolve, 0)
      })
    })

    const approvalSelect = screen.getByLabelText('Approval') as HTMLSelectElement
    const values = Array.from(approvalSelect.options).map((option) => option.value)
    expect(values).toContain('2')
    expect(values).not.toContain('1')
  })

  it('renders detail only for a changed-since-approval error, never for another code that happens to carry one (R3-009)', async () => {
    mockIPC((cmd) => {
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'invalid-request',
          message: 'bad request',
          detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toContain('invalid-request')
    expect(banner.textContent).not.toContain('aaaaaaaa')
    expect(banner.textContent).not.toContain('bbbbbbbb')
    expect(banner.textContent).not.toContain('recorded')
  })

  it('keeps Start disabled while the approvals list is still loading for the approved kind (R3-010)', async () => {
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') return new Promise<never[]>(() => {})
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<HarnessPanel />)
    fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'approved' } })
    await screen.findByLabelText('Approval')

    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
  })

  it('never renders the approval region inside the output log element (RCS-001-R6)', async () => {
    mockIPC((cmd) => {
      if (cmd === 'approvals_list') return []
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(
      <>
        <HarnessPanel />
        <ApprovalSurface />
      </>,
    )

    const region = await screen.findByRole('region', { name: 'Executable approval' })
    const log = screen.getByLabelText('Output log')
    expect(log.contains(region)).toBe(false)
    expect(document.body.contains(region)).toBe(true)
  })
})

type LiveChannel = { onmessage: (frame: HarnessFrame) => void }

/**
 * Renders the panel with a `harness_spawn` mock that hands out
 * incrementing ids (7, 8, ...) and records every live Channel it
 * receives, so a test can drive more than one run and address each run's
 * own channel. `harness_stop` resolves `killed`.
 */
function renderMultiRun(): { channels: LiveChannel[] } {
  const channels: LiveChannel[] = []
  let nextId = 7
  mockIPC((cmd, args) => {
    if (cmd === 'harness_spawn') {
      channels.push((args as { onFrame: LiveChannel }).onFrame)
      const id = nextId
      nextId += 1
      return id
    }
    if (cmd === 'harness_stop') return { state: 'killed', code: null }
    throw new Error(`unexpected command: ${cmd}`)
  })
  render(<HarnessPanel />)
  return { channels }
}

describe('HarnessPanel run identity (R1-001 / R3-001)', () => {
  it('ignores a frame carrying a foreign run id, text and state kinds alike: nothing appended, badge unchanged', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'stdout',
        body: { id: 99, seq: 0, droppedBefore: 0, continued: false, text: 'foreign text' },
      })
      channel.onmessage({
        stream: 'state',
        body: { id: 99, seq: 1, droppedBefore: 0, state: 'killed', code: null },
      })
    })

    const log = screen.getByLabelText('Output log')
    expect(log.querySelectorAll('li').length).toBe(0)
    expect(log.textContent).not.toContain('foreign text')
    expect(screen.getByText('running')).toBeTruthy()
    expect(screen.queryByText('killed')).toBeNull()
    expect((screen.getByRole('button', { name: 'Stop' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
  })

  it('once a second run has started, a trailing frame with the first run id is ignored while a frame with the new id is applied', async () => {
    const { channels } = renderMultiRun()

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await screen.findByText('killed')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    expect(channels).toHaveLength(2)

    act(() => {
      channels[0]!.onmessage({
        stream: 'stdout',
        body: { id: 7, seq: 5, droppedBefore: 0, continued: false, text: 'late from run A' },
      })
      channels[1]!.onmessage({
        stream: 'stdout',
        body: { id: 8, seq: 0, droppedBefore: 0, continued: false, text: 'fresh from run B' },
      })
    })

    const log = screen.getByLabelText('Output log')
    expect(log.textContent).not.toContain('late from run A')
    expect(log.textContent).toContain('stdout: fresh from run B')
  })
})

describe('HarnessPanel terminal state frame (R3-002)', () => {
  it('ends the run: Start re-enabled, Stop disabled, the id forgotten, and a fresh Start spawns a new run', async () => {
    const { channels } = renderMultiRun()

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    expect(window.sessionStorage.getItem(SESSION_STORAGE_KEY)).toBe('[7]')

    act(() => {
      channels[0]!.onmessage({
        stream: 'state',
        body: { id: 7, seq: 3, droppedBefore: 0, state: 'exited', code: 0 },
      })
    })

    await screen.findByText('exited (code 0)')
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
    expect((screen.getByRole('button', { name: 'Stop' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    // A terminal frame means nothing is left to reconnect to (R3-003), the
    // same as a terminal `harness_observe` status or a completed stop.
    expect(window.sessionStorage.getItem(SESSION_STORAGE_KEY)).toBe('[]')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    expect(channels).toHaveLength(2)
  })
})

describe('HarnessPanel run id after the run ends (R1-011 / R3-011)', () => {
  /** Delivers, on `channel`, a foreign-id text frame, a foreign-id state frame, and a trailing text frame carrying the run's own id. */
  function deliverForeignThenOwn(channel: LiveChannel) {
    act(() => {
      channel.onmessage({
        stream: 'stdout',
        body: { id: 99, seq: 4, droppedBefore: 0, continued: false, text: 'foreign after end' },
      })
      channel.onmessage({
        stream: 'state',
        body: { id: 99, seq: 5, droppedBefore: 0, state: 'orphan-risk/uncertain', code: null },
      })
      channel.onmessage({
        stream: 'stdout',
        body: { id: 7, seq: 4, droppedBefore: 0, continued: false, text: 'trailing and ours' },
      })
    })
  }

  it('after a terminal state frame, a same-channel foreign-id frame is dropped and the badge holds, while a trailing frame with the run id is applied', async () => {
    const channel = await startAndCaptureChannel()
    act(() => {
      channel.onmessage({
        stream: 'state',
        body: { id: 7, seq: 3, droppedBefore: 0, state: 'exited', code: 0 },
      })
    })
    await screen.findByText('exited (code 0)')

    deliverForeignThenOwn(channel)

    const log = screen.getByLabelText('Output log')
    expect(log.textContent).not.toContain('foreign after end')
    expect(log.textContent).not.toContain('orphan-risk/uncertain')
    expect(log.textContent).toContain('stdout: trailing and ours')
    expect(screen.getByText('exited (code 0)')).toBeTruthy()
    expect(document.body.textContent).not.toContain('orphan-risk/uncertain')
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
  })

  it('after a successful Stop, a same-channel foreign-id frame is dropped and the badge holds, while a trailing frame with the run id is applied', async () => {
    const channel = await startAndCaptureChannel((cmd) => {
      if (cmd === 'harness_stop') return { state: 'killed', code: null }
      throw new Error(`unexpected command: ${cmd}`)
    })
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await screen.findByText('killed')

    deliverForeignThenOwn(channel)

    const log = screen.getByLabelText('Output log')
    expect(log.textContent).not.toContain('foreign after end')
    expect(log.textContent).toContain('stdout: trailing and ours')
    expect(screen.getByText('killed')).toBeTruthy()
    expect(document.body.textContent).not.toContain('orphan-risk/uncertain')
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
  })
})

/**
 * Renders the panel with a `harness_spawn` mock whose promise stays
 * pending until the test resolves it -- so frames can be delivered on the
 * live Channel *before* the panel learns the run's id.
 */
function renderWithDeferredSpawn(): {
  channels: LiveChannel[]
  resolveSpawn: (id: number) => void
} {
  const channels: LiveChannel[] = []
  let resolveSpawn: (id: number) => void = () => {}
  mockIPC((cmd, args) => {
    if (cmd === 'harness_spawn') {
      channels.push((args as { onFrame: LiveChannel }).onFrame)
      return new Promise<number>((resolve) => {
        resolveSpawn = resolve
      })
    }
    if (cmd === 'harness_stop') return { state: 'killed', code: null }
    throw new Error(`unexpected command: ${cmd}`)
  })
  render(<HarnessPanel />)
  return { channels, resolveSpawn: (id) => resolveSpawn(id) }
}

/** Resolves a deferred spawn and drains the resolve handler's microtasks plus one macrotask. */
async function settleSpawn(resolveSpawn: (id: number) => void, id: number): Promise<void> {
  await act(async () => {
    resolveSpawn(id)
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })
  })
}

/** Clicks Start and waits until the mocked `harness_spawn` has handed over run number `expectedRuns`'s Channel. */
async function clickStartAndAwaitChannel(
  channels: LiveChannel[],
  expectedRuns: number,
): Promise<void> {
  fireEvent.click(screen.getByRole('button', { name: 'Start' }))
  await waitFor(() => {
    expect(channels).toHaveLength(expectedRuns)
  })
}

describe('HarnessPanel spawn generation: frames before harness_spawn resolves, and old channels', () => {
  it('applies a terminal state frame delivered before the spawn resolves: Start ends enabled, Stop disabled, state line in the log, id never persisted', async () => {
    const { channels, resolveSpawn } = renderWithDeferredSpawn()

    await clickStartAndAwaitChannel(channels, 1)

    act(() => {
      channels[0]!.onmessage({
        stream: 'state',
        body: { id: 7, seq: 0, droppedBefore: 0, state: 'exited', code: 0 },
      })
    })
    await settleSpawn(resolveSpawn, 7)

    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
    expect((screen.getByRole('button', { name: 'Stop' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    expect(screen.getByText('exited (code 0)')).toBeTruthy()
    expect(screen.queryByText('running')).toBeNull()
    expect(screen.getByLabelText('Output log').textContent).toContain('state: exited (code 0)')
    // Nothing is left to reconnect to, so the id must not be remembered.
    expect(JSON.parse(window.sessionStorage.getItem(SESSION_STORAGE_KEY) ?? '[]')).toEqual([])
  })

  it('applies a text frame delivered before the spawn resolves, then runs normally and still drops a foreign id once the id is known', async () => {
    const { channels, resolveSpawn } = renderWithDeferredSpawn()

    await clickStartAndAwaitChannel(channels, 1)
    act(() => {
      channels[0]!.onmessage({
        stream: 'stdout',
        body: { id: 7, seq: 0, droppedBefore: 0, continued: false, text: 'early but ours' },
      })
    })
    await settleSpawn(resolveSpawn, 7)

    expect(screen.getByText('running')).toBeTruthy()
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    expect(window.sessionStorage.getItem(SESSION_STORAGE_KEY)).toBe('[7]')

    act(() => {
      channels[0]!.onmessage({
        stream: 'stdout',
        body: { id: 99, seq: 1, droppedBefore: 0, continued: false, text: 'foreign after id known' },
      })
      channels[0]!.onmessage({
        stream: 'stdout',
        body: { id: 7, seq: 1, droppedBefore: 0, continued: false, text: 'later and ours' },
      })
    })

    const log = screen.getByLabelText('Output log')
    expect(log.textContent).toContain('stdout: early but ours')
    expect(log.textContent).toContain('stdout: later and ours')
    expect(log.textContent).not.toContain('foreign after id known')
  })

  it('rejects a frame from a previous run\'s channel while the next spawn is still pending, even with the new id unknown', async () => {
    const { channels, resolveSpawn } = renderWithDeferredSpawn()

    await clickStartAndAwaitChannel(channels, 1)
    await settleSpawn(resolveSpawn, 7)
    await screen.findByText('running')
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await screen.findByText('killed')

    await clickStartAndAwaitChannel(channels, 2)

    act(() => {
      channels[0]!.onmessage({
        stream: 'stdout',
        body: { id: 7, seq: 9, droppedBefore: 0, continued: false, text: 'stale from run A' },
      })
      channels[0]!.onmessage({
        stream: 'stdout',
        body: { id: 8, seq: 10, droppedBefore: 0, continued: false, text: 'stale channel, forged id' },
      })
      channels[1]!.onmessage({
        stream: 'stdout',
        body: { id: 8, seq: 0, droppedBefore: 0, continued: false, text: 'early from run B' },
      })
    })
    await settleSpawn(resolveSpawn, 8)

    const log = screen.getByLabelText('Output log')
    expect(log.textContent).not.toContain('stale from run A')
    expect(log.textContent).not.toContain('stale channel, forged id')
    expect(log.textContent).toContain('stdout: early from run B')
    expect(screen.getByText('running')).toBeTruthy()
  })

  it('chained: an early terminal frame, then the spawn resolves, then a same-channel foreign id is dropped with the badge held, then the run id is applied', async () => {
    const { channels, resolveSpawn } = renderWithDeferredSpawn()

    await clickStartAndAwaitChannel(channels, 1)
    act(() => {
      channels[0]!.onmessage({
        stream: 'state',
        body: { id: 7, seq: 0, droppedBefore: 0, state: 'exited', code: 0 },
      })
    })
    await settleSpawn(resolveSpawn, 7)
    expect(screen.getByText('exited (code 0)')).toBeTruthy()
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )

    // The id became known only after the run had already ended: it must
    // still gate every later frame on this channel.
    act(() => {
      channels[0]!.onmessage({
        stream: 'stdout',
        body: { id: 99, seq: 1, droppedBefore: 0, continued: false, text: 'foreign after early end' },
      })
      channels[0]!.onmessage({
        stream: 'state',
        body: { id: 99, seq: 2, droppedBefore: 0, state: 'orphan-risk/uncertain', code: null },
      })
    })
    const log = screen.getByLabelText('Output log')
    expect(log.textContent).not.toContain('foreign after early end')
    expect(screen.getByText('exited (code 0)')).toBeTruthy()
    expect(document.body.textContent).not.toContain('orphan-risk/uncertain')

    act(() => {
      channels[0]!.onmessage({
        stream: 'stdout',
        body: { id: 7, seq: 1, droppedBefore: 0, continued: false, text: 'trailing and ours' },
      })
    })
    expect(log.textContent).toContain('stdout: trailing and ours')
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
    expect((screen.getByRole('button', { name: 'Stop' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    expect(JSON.parse(window.sessionStorage.getItem(SESSION_STORAGE_KEY) ?? '[]')).toEqual([])
  })
})
