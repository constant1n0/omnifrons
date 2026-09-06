import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { afterEach, describe, expect, it } from 'vitest'

import {
  harnessObserve,
  harnessSpawn,
  harnessStop,
  isShellError,
  type HarnessFrame,
} from './harness'

// The jsdom crypto polyfill is installed once for every test file by
// `testSupport/setup.ts` (`vite.config.ts`'s `test.setupFiles`).

afterEach(() => {
  clearMocks()
})

describe('harnessSpawn', () => {
  it('invokes harness_spawn with exactly the keys kind, rateHz, lines, onFrame', async () => {
    let capturedArgs: Record<string, unknown> | undefined
    mockIPC((cmd, args) => {
      if (cmd === 'harness_spawn') {
        capturedArgs = args as Record<string, unknown>
        return 42
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const id = await harnessSpawn({ kind: 'demo-lines', rateHz: 10, lines: 5 }, () => {})

    expect(id).toBe(42)
    expect(capturedArgs).toBeDefined()
    expect(Object.keys(capturedArgs!).sort()).toEqual(['kind', 'lines', 'onFrame', 'rateHz'])
    expect(capturedArgs!.kind).toBe('demo-lines')
    expect(capturedArgs!.rateHz).toBe(10)
    expect(capturedArgs!.lines).toBe(5)
    for (const key of Object.keys(capturedArgs!)) {
      expect(key.toLowerCase()).not.toContain('path')
    }
  })

  it('delivers frames sent on the live Channel instance to the onFrame callback', async () => {
    // The mock handler in @tauri-apps/api/mocks calls the real `invoke`'s
    // args straight through with no serialization -- `args.onFrame` here
    // is the exact live Channel instance harnessSpawn constructed, not a
    // serialized placeholder. See harness.ts's module doc for the write-up.
    let channelRef: { onmessage: (frame: HarnessFrame) => void } | undefined
    mockIPC((cmd, args) => {
      if (cmd === 'harness_spawn') {
        channelRef = (args as Record<string, unknown>).onFrame as {
          onmessage: (frame: HarnessFrame) => void
        }
        return 7
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const received: HarnessFrame[] = []
    await harnessSpawn({ kind: 'demo-lines', rateHz: 1, lines: 1 }, (frame) => {
      received.push(frame)
    })

    expect(channelRef).toBeDefined()
    const frame: HarnessFrame = {
      stream: 'stdout',
      body: { id: 7, seq: 0, droppedBefore: 0, text: 'hi' },
    }
    channelRef!.onmessage(frame)

    expect(received).toEqual([frame])
  })

  it('rejects with the ShellError payload when the command rejects', async () => {
    mockIPC((cmd) => {
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'spawn-failed',
          message: 'failed to start the requested process',
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    await expect(
      harnessSpawn({ kind: 'demo-lines', rateHz: 1, lines: 1 }, () => {}),
    ).rejects.toMatchObject({
      code: 'spawn-failed',
      message: 'failed to start the requested process',
    })
  })
})

describe('harnessStop', () => {
  it('invokes harness_stop with id and deadlineMs and returns the terminal state', async () => {
    mockIPC((cmd, args) => {
      if (cmd === 'harness_stop') {
        expect(args).toEqual({ id: 7, deadlineMs: 500 })
        return { state: 'killed', code: null }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await harnessStop(7, 500)

    expect(result).toEqual({ state: 'killed', code: null })
  })
})

describe('harnessObserve', () => {
  it('invokes harness_observe with id and returns the status', async () => {
    mockIPC((cmd, args) => {
      if (cmd === 'harness_observe') {
        expect(args).toEqual({ id: 7 })
        return { status: 'running' }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await harnessObserve(7)

    expect(result).toEqual({ status: 'running' })
  })

  it('returns a terminal orphan-risk/uncertain status verbatim', async () => {
    mockIPC((cmd) => {
      if (cmd === 'harness_observe') {
        return { status: 'terminal', state: 'orphan-risk/uncertain', code: null }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await harnessObserve(7)

    expect(result).toEqual({ status: 'terminal', state: 'orphan-risk/uncertain', code: null })
  })
})

describe('isShellError', () => {
  it('identifies a ShellError-shaped value', () => {
    expect(isShellError({ code: 'spawn-failed', message: 'x' })).toBe(true)
  })

  it('rejects values missing the shape', () => {
    expect(isShellError(null)).toBe(false)
    expect(isShellError(undefined)).toBe(false)
    expect(isShellError('error')).toBe(false)
    expect(isShellError({ message: 'x' })).toBe(false)
    expect(isShellError({ code: 1, message: 'x' })).toBe(false)
  })
})
