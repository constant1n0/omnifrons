import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { HarnessPanel } from './HarnessPanel'
import type { HarnessFrame } from './ipc/harness'

// The jsdom crypto polyfill and React Testing Library's `cleanup()` are
// installed once for every test file by `testSupport/setup.ts`
// (`vite.config.ts`'s `test.setupFiles`) -- not repeated here.

const SESSION_STORAGE_KEY = 'omnifrons.harness.processIds'

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

  await screen.findByText('running')

  if (!channelRef) throw new Error('harness_spawn was not called')
  return channelRef
}

describe('HarnessPanel', () => {
  it('renders the kind select, bounded rate/lines inputs, and the controls', () => {
    render(<HarnessPanel />)

    const kindSelect = screen.getByLabelText('Kind') as HTMLSelectElement
    const options = Array.from(kindSelect.options).map((option) => option.value)
    expect(options).toEqual(['demo-lines', 'demo-ignores-sigterm'])

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
        body: { id: 7, seq: 0, droppedBefore: 0, text: '<b>bold</b>' },
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
        body: { id: 7, seq: 1, droppedBefore: 0, text: osc8 },
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
        body: { id: 7, seq: 12, droppedBefore: 7, text: 'line 13' },
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
          body: { id: 7, seq, droppedBefore: 0, text: `line ${seq}` },
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
    expect(spawnCount).toBe(1)

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
        body: { id: 7, seq: 0, droppedBefore: 0, text: 'after unmount' },
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
    let resolveObserve: (value: unknown) => void = () => {}
    mockIPC((cmd, args) => {
      if (cmd === 'harness_observe') {
        expect(args).toEqual({ id: 99 })
        return new Promise((resolve) => {
          resolveObserve = resolve
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const { unmount } = render(<HarnessPanel />)
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
})
