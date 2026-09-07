import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { afterEach, describe, expect, it } from 'vitest'

import {
  adaptersList,
  approvalsList,
  executableApprove,
  executablePickAndProbe,
  executableRevoke,
  harnessObserve,
  harnessSpawn,
  harnessStop,
  isShellError,
  workspaceCurrent,
  workspacePick,
  type Approval,
  type Evidence,
  type HarnessFrame,
} from './harness'

// The jsdom crypto polyfill is installed once for every test file by
// `testSupport/setup.ts` (`vite.config.ts`'s `test.setupFiles`).

afterEach(() => {
  clearMocks()
})

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

describe('harnessSpawn', () => {
  it('invokes harness_spawn with exactly the keys kind and onFrame, kind carrying rateHz/lines for a demo kind', async () => {
    let capturedArgs: Record<string, unknown> | undefined
    mockIPC((cmd, args) => {
      if (cmd === 'harness_spawn') {
        capturedArgs = args as Record<string, unknown>
        return 42
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const id = await harnessSpawn({ type: 'demo-lines', rateHz: 10, lines: 5 }, () => {})

    expect(id).toBe(42)
    expect(capturedArgs).toBeDefined()
    expect(Object.keys(capturedArgs!).sort()).toEqual(['kind', 'onFrame'])
    expect(capturedArgs!.kind).toEqual({ type: 'demo-lines', rateHz: 10, lines: 5 })
    for (const key of Object.keys(capturedArgs!)) {
      expect(key.toLowerCase()).not.toContain('path')
    }
    for (const key of Object.keys(capturedArgs!.kind as Record<string, unknown>)) {
      expect(key.toLowerCase()).not.toContain('path')
    }
  })

  it('invokes harness_spawn for the approved kind with exactly { kind: { type: "approved", approvalId }, onFrame } and no path-shaped key', async () => {
    let capturedArgs: Record<string, unknown> | undefined
    mockIPC((cmd, args) => {
      if (cmd === 'harness_spawn') {
        capturedArgs = args as Record<string, unknown>
        return 99
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const id = await harnessSpawn({ type: 'approved', approvalId: 42 }, () => {})

    expect(id).toBe(99)
    expect(capturedArgs).toBeDefined()
    expect(Object.keys(capturedArgs!).sort()).toEqual(['kind', 'onFrame'])
    expect(capturedArgs!.kind).toEqual({ type: 'approved', approvalId: 42 })
    expect(Object.keys(capturedArgs!.kind as Record<string, unknown>).sort()).toEqual([
      'approvalId',
      'type',
    ])
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
    await harnessSpawn({ type: 'demo-lines', rateHz: 1, lines: 1 }, (frame) => {
      received.push(frame)
    })

    expect(channelRef).toBeDefined()
    const frame: HarnessFrame = {
      stream: 'stdout',
      body: { id: 7, seq: 0, droppedBefore: 0, continued: false, text: 'hi' },
    }
    channelRef!.onmessage(frame)

    expect(received).toEqual([frame])
  })

  it('delivers a text frame flagged continued (more of the same line follows) verbatim', async () => {
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
    await harnessSpawn({ type: 'demo-lines', rateHz: 1, lines: 1 }, (frame) => {
      received.push(frame)
    })

    const head: HarnessFrame = {
      stream: 'stdout',
      body: { id: 7, seq: 0, droppedBefore: 0, continued: true, text: 'first half of a ' },
    }
    const tail: HarnessFrame = {
      stream: 'stdout',
      body: { id: 7, seq: 1, droppedBefore: 0, continued: false, text: 'long line' },
    }
    channelRef!.onmessage(head)
    channelRef!.onmessage(tail)

    expect(received).toEqual([head, tail])
    expect(received[0]?.stream === 'stdout' && received[0].body.continued).toBe(true)
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
      harnessSpawn({ type: 'demo-lines', rateHz: 1, lines: 1 }, () => {}),
    ).rejects.toMatchObject({
      code: 'spawn-failed',
      message: 'failed to start the requested process',
    })
  })

  it('invokes harness_spawn for the adapter kind with exactly { kind: { type: "adapter", adapterId, approvalId, prompt }, onFrame } and no path-shaped key', async () => {
    let capturedArgs: Record<string, unknown> | undefined
    mockIPC((cmd, args) => {
      if (cmd === 'harness_spawn') {
        capturedArgs = args as Record<string, unknown>
        return 123
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const id = await harnessSpawn(
      { type: 'adapter', adapterId: 'claude-code', approvalId: 42, prompt: 'do the thing' },
      () => {},
    )

    expect(id).toBe(123)
    expect(capturedArgs).toBeDefined()
    expect(Object.keys(capturedArgs!).sort()).toEqual(['kind', 'onFrame'])
    expect(capturedArgs!.kind).toEqual({
      type: 'adapter',
      adapterId: 'claude-code',
      approvalId: 42,
      prompt: 'do the thing',
    })
    expect(Object.keys(capturedArgs!.kind as Record<string, unknown>).sort()).toEqual([
      'adapterId',
      'approvalId',
      'prompt',
      'type',
    ])
    for (const key of Object.keys(capturedArgs!)) {
      expect(key.toLowerCase()).not.toContain('path')
    }
    for (const key of Object.keys(capturedArgs!.kind as Record<string, unknown>)) {
      expect(key.toLowerCase()).not.toContain('path')
    }
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

describe('executablePickAndProbe', () => {
  it('invokes executable_pick_and_probe with no arguments and returns the probe result verbatim', async () => {
    mockIPC((cmd, args) => {
      if (cmd === 'executable_pick_and_probe') {
        expect(args).toEqual({})
        return { candidateId: 7, evidence: SAMPLE_EVIDENCE }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await executablePickAndProbe()

    expect(result).toEqual({ candidateId: 7, evidence: SAMPLE_EVIDENCE })
  })

  it('rejects with the ShellError payload when the command rejects', async () => {
    mockIPC((cmd) => {
      if (cmd === 'executable_pick_and_probe') {
        return Promise.reject({ code: 'no-candidate', message: 'no file was selected' })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    await expect(executablePickAndProbe()).rejects.toMatchObject({
      code: 'no-candidate',
      message: 'no file was selected',
    })
  })
})

describe('executableApprove', () => {
  it('invokes executable_approve with exactly { candidateId } and returns the approval', async () => {
    mockIPC((cmd, args) => {
      if (cmd === 'executable_approve') {
        expect(args).toEqual({ candidateId: 7 })
        return sampleApproval({ approvalId: 42 })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await executableApprove(7)

    expect(result).toEqual(sampleApproval({ approvalId: 42 }))
  })
})

describe('executableRevoke', () => {
  it('invokes executable_revoke with exactly { approvalId } and resolves to undefined', async () => {
    let capturedArgs: Record<string, unknown> | undefined
    mockIPC((cmd, args) => {
      if (cmd === 'executable_revoke') {
        capturedArgs = args as Record<string, unknown>
        return null
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    await expect(executableRevoke(42)).resolves.toBeUndefined()
    expect(capturedArgs).toEqual({ approvalId: 42 })
  })
})

describe('approvalsList', () => {
  it('invokes approvals_list with no arguments and returns the list verbatim', async () => {
    const approvals = [sampleApproval({ approvalId: 1 }), sampleApproval({ approvalId: 2 })]
    mockIPC((cmd, args) => {
      if (cmd === 'approvals_list') {
        expect(args).toEqual({})
        return approvals
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await approvalsList()

    expect(result).toEqual(approvals)
  })
})

describe('adaptersList', () => {
  it('invokes adapters_list with no arguments and returns the descriptors verbatim, with no argv or env keys', async () => {
    const descriptors = [
      {
        id: 'claude-code',
        displayName: 'Claude Code',
        transportClass: 'structured-streaming-cli' as const,
        promptChannel: 'stdin-then-close' as const,
        scopeMode: 'advisory' as const,
        notes: 'credentials are harness-owned; not exercised in CI',
      },
    ]
    mockIPC((cmd, args) => {
      if (cmd === 'adapters_list') {
        expect(args).toEqual({})
        return descriptors
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await adaptersList()

    expect(result).toEqual(descriptors)
    for (const descriptor of result) {
      expect(Object.keys(descriptor)).not.toContain('argvTemplate')
      expect(Object.keys(descriptor)).not.toContain('declaredEnv')
    }
  })
})

describe('workspacePick', () => {
  it('invokes workspace_pick with no arguments and returns the workspace verbatim', async () => {
    mockIPC((cmd, args) => {
      if (cmd === 'workspace_pick') {
        expect(args).toEqual({})
        return { displayPath: '/home/user/project' }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await workspacePick()

    expect(result).toEqual({ displayPath: '/home/user/project' })
  })

  it('rejects with no-workspace when the folder dialog was canceled', async () => {
    mockIPC((cmd) => {
      if (cmd === 'workspace_pick') {
        return Promise.reject({ code: 'no-workspace', message: 'no folder was selected' })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    await expect(workspacePick()).rejects.toMatchObject({ code: 'no-workspace' })
  })
})

describe('workspaceCurrent', () => {
  it('invokes workspace_current with no arguments and returns the active workspace', async () => {
    mockIPC((cmd, args) => {
      if (cmd === 'workspace_current') {
        expect(args).toEqual({})
        return { displayPath: '/home/user/project' }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await workspaceCurrent()

    expect(result).toEqual({ displayPath: '/home/user/project' })
  })

  it('returns null when no workspace is active', async () => {
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return null
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await workspaceCurrent()

    expect(result).toBeNull()
  })
})

describe('HarnessFrame event stream', () => {
  it('delivers an adapter event frame to the onFrame callback verbatim', async () => {
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
    await harnessSpawn(
      { type: 'adapter', adapterId: 'claude-code', approvalId: 1, prompt: 'hi' },
      (frame) => {
        received.push(frame)
      },
    )

    expect(channelRef).toBeDefined()
    const frame: HarnessFrame = {
      stream: 'event',
      body: {
        id: 7,
        seq: 0,
        droppedBefore: 0,
        kind: 'message',
        payload: { text: 'hello' },
      },
    }
    channelRef!.onmessage(frame)

    expect(received).toEqual([frame])
  })
})

describe('isShellError', () => {
  it('identifies a ShellError-shaped value', () => {
    expect(isShellError({ code: 'spawn-failed', message: 'x' })).toBe(true)
  })

  it('identifies a ShellError-shaped value carrying a detail object', () => {
    expect(
      isShellError({
        code: 'changed-since-approval',
        message: 'x',
        detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
      }),
    ).toBe(true)
  })

  it('rejects values missing the shape', () => {
    expect(isShellError(null)).toBe(false)
    expect(isShellError(undefined)).toBe(false)
    expect(isShellError('error')).toBe(false)
    expect(isShellError({ message: 'x' })).toBe(false)
    expect(isShellError({ code: 1, message: 'x' })).toBe(false)
  })
})
