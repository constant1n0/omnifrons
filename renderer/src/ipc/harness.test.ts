import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { afterEach, describe, expect, it } from 'vitest'

import {
  adaptersList,
  approvalsList,
  artifactApprove,
  artifactPublish,
  candidatesList,
  executableApprove,
  executablePickAndProbe,
  executableRevoke,
  harnessObserve,
  harnessSpawn,
  harnessStop,
  isShellError,
  outboxStatus,
  publicationsList,
  workspaceCurrent,
  workspacePick,
  type AdapterDescriptor,
  type AgentEvent,
  type Approval,
  type ArtifactApproval,
  type ArtifactState,
  type ArtifactStateFrame,
  type Availability,
  type Candidate,
  type Evidence,
  type HarnessFrame,
  type OutboxStatus,
  type ProviderState,
  type Publication,
  type ShellError,
  type ShellErrorCode,
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
  it('invokes workspace_pick with no arguments and returns the workspace verbatim -- displayPath and, since slice 5b, its workArea check', async () => {
    mockIPC((cmd, args) => {
      if (cmd === 'workspace_pick') {
        expect(args).toEqual({})
        return { displayPath: '/home/user/project', workArea: 'valid' }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await workspacePick()

    expect(result).toEqual({ displayPath: '/home/user/project', workArea: 'valid' })
    expect(result.workArea).toBe('valid')
    expect(Object.keys(result).sort()).toEqual(['displayPath', 'workArea'])
  })

  it('returns a work-area-invalid workArea verbatim: the product work area re-checked against the picked workspace (slice 5b, HAP-001-R7)', async () => {
    mockIPC((cmd) => {
      if (cmd === 'workspace_pick') {
        return { displayPath: '/home/user/project', workArea: 'work-area-invalid' }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await workspacePick()

    expect(result.workArea).toBe('work-area-invalid')
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
  it('invokes workspace_current with no arguments and returns the active workspace, its workArea included', async () => {
    mockIPC((cmd, args) => {
      if (cmd === 'workspace_current') {
        expect(args).toEqual({})
        return { displayPath: '/home/user/project', workArea: 'work-area-invalid' }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await workspaceCurrent()

    expect(result).toEqual({ displayPath: '/home/user/project', workArea: 'work-area-invalid' })
    expect(result?.workArea).toBe('work-area-invalid')
  })

  it('compile-time guard, enforced by tsc -b in pnpm -r build and not by vitest: a switch over the workArea tokens with a never default compiles, and at runtime maps both tokens to themselves (slice 5b)', () => {
    type WorkArea = NonNullable<Awaited<ReturnType<typeof workspaceCurrent>>>['workArea']
    function workAreaLabel(state: WorkArea): string {
      switch (state) {
        case 'valid':
        case 'work-area-invalid':
          return state
        default: {
          const unreachable: never = state
          return unreachable
        }
      }
    }

    const states: WorkArea[] = ['valid', 'work-area-invalid']
    expect(states.map(workAreaLabel)).toEqual(states)
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

// -- Slice 4: the pty-cli descriptor, the three terminal event kinds, and
// the pty-unsupported error code (`docs/spike-log.md` § Slice 4) --

type LiveChannel = { onmessage: (frame: HarnessFrame) => void }

/**
 * Spawns the adapter kind named by `adapterId` through a mock that hands
 * back the live Channel, collecting every frame delivered to `onFrame`.
 */
async function spawnAdapterAndCapture(
  adapterId: 'claude-code' | 'pty-cli',
): Promise<{ channel: LiveChannel; received: HarnessFrame[] }> {
  let channelRef: LiveChannel | undefined
  mockIPC((cmd, args) => {
    if (cmd === 'harness_spawn') {
      channelRef = (args as Record<string, unknown>).onFrame as LiveChannel
      return 42
    }
    throw new Error(`unexpected command: ${cmd}`)
  })

  const received: HarnessFrame[] = []
  await harnessSpawn({ type: 'adapter', adapterId, approvalId: 1, prompt: 'hi' }, (frame) => {
    received.push(frame)
  })
  if (!channelRef) throw new Error('harness_spawn was not called')
  return { channel: channelRef, received }
}

/** Spawns the `pty-cli` adapter kind (slice 4) and captures its live Channel. */
function spawnPtyAndCapture(): Promise<{ channel: LiveChannel; received: HarnessFrame[] }> {
  return spawnAdapterAndCapture('pty-cli')
}

describe('HarnessFrame terminal event kinds (slice 4, pty-cli)', () => {
  it('delivers a terminal-text event frame verbatim, its newline kept', async () => {
    const { channel, received } = await spawnPtyAndCapture()

    const frame: HarnessFrame = {
      stream: 'event',
      body: {
        id: 42,
        seq: 8,
        droppedBefore: 0,
        kind: 'terminal-text',
        payload: { text: 'hello\n' },
      },
    }
    channel.onmessage(frame)

    expect(received).toEqual([frame])
    const delivered = received[0]
    expect(
      delivered?.stream === 'event' &&
        delivered.body.kind === 'terminal-text' &&
        delivered.body.payload.text,
    ).toBe('hello\n')
  })

  it('delivers terminal-action event frames for both closed actions, title and notification, verbatim', async () => {
    const { channel, received } = await spawnPtyAndCapture()

    const title: HarnessFrame = {
      stream: 'event',
      body: {
        id: 42,
        seq: 9,
        droppedBefore: 0,
        kind: 'terminal-action',
        payload: { action: 'title', text: 'build ok' },
      },
    }
    const notification: HarnessFrame = {
      stream: 'event',
      body: {
        id: 42,
        seq: 10,
        droppedBefore: 2,
        kind: 'terminal-action',
        payload: { action: 'notification', text: 'done' },
      },
    }
    channel.onmessage(title)
    channel.onmessage(notification)

    expect(received).toEqual([title, notification])
  })

  it('delivers a terminal-drops event frame with exactly the seven camelCase counts verbatim', async () => {
    const { channel, received } = await spawnPtyAndCapture()

    const frame: HarnessFrame = {
      stream: 'event',
      body: {
        id: 42,
        seq: 11,
        droppedBefore: 0,
        kind: 'terminal-drops',
        payload: {
          layout: 21,
          hyperlink: 2,
          clipboard: 2,
          fileTransfer: 3,
          string: 3,
          unknown: 1,
          malformed: 3,
        },
      },
    }
    channel.onmessage(frame)

    expect(received).toEqual([frame])
    const delivered = received[0]
    if (delivered?.stream !== 'event' || delivered.body.kind !== 'terminal-drops') {
      throw new Error('expected a terminal-drops event frame')
    }
    expect(Object.keys(delivered.body.payload).sort()).toEqual([
      'clipboard',
      'fileTransfer',
      'hyperlink',
      'layout',
      'malformed',
      'string',
      'unknown',
    ])
  })

  it('compile-time guard, enforced by tsc -b in pnpm -r build and not by vitest: a switch over AgentEvent kinds with a never default compiles, and at runtime maps each of the ten kinds to itself', () => {
    function kindLabel(kind: AgentEvent['kind']): string {
      switch (kind) {
        case 'state':
        case 'message':
        case 'tool-call':
        case 'diagnostic':
        case 'unknown':
        case 'terminal-text':
        case 'terminal-action':
        case 'terminal-drops':
        case 'artifact-publish':
        case 'candidates':
          return kind
        default: {
          const unreachable: never = kind
          return unreachable
        }
      }
    }

    const kinds: AgentEvent['kind'][] = [
      'state',
      'message',
      'tool-call',
      'diagnostic',
      'unknown',
      'terminal-text',
      'terminal-action',
      'terminal-drops',
      'artifact-publish',
      'candidates',
    ]
    expect(kinds.map(kindLabel)).toEqual(kinds)
  })
})

describe('adaptersList pty-cli descriptor (slice 4)', () => {
  it('returns the pty-cli descriptor with transportClass pty and promptChannel pty-typed verbatim, with no argv or env keys', async () => {
    const ptyCli: AdapterDescriptor = {
      id: 'pty-cli',
      displayName: 'PTY CLI',
      transportClass: 'pty',
      promptChannel: 'pty-typed',
      scopeMode: 'advisory',
      notes: 'degraded fallback with no structured events',
    }
    mockIPC((cmd, args) => {
      if (cmd === 'adapters_list') {
        expect(args).toEqual({})
        return [ptyCli]
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await adaptersList()

    expect(result).toEqual([ptyCli])
    expect(Object.keys(result[0]!)).not.toContain('argvTemplate')
    expect(Object.keys(result[0]!)).not.toContain('declaredEnv')
  })
})

describe('ShellErrorCode pty-unsupported (slice 4)', () => {
  it('harnessSpawn rejects with the typed pty-unsupported ShellError for a pty-cli launch on an unsupported platform', async () => {
    const error: ShellError = {
      code: 'pty-unsupported',
      message: 'pseudo-terminal launches are not available on this platform',
    }
    mockIPC((cmd) => {
      if (cmd === 'harness_spawn') return Promise.reject(error)
      throw new Error(`unexpected command: ${cmd}`)
    })

    await expect(
      harnessSpawn({ type: 'adapter', adapterId: 'pty-cli', approvalId: 1, prompt: 'hi' }, () => {}),
    ).rejects.toMatchObject(error)
    expect(isShellError(error)).toBe(true)
  })

  it('harnessSpawn rejects with the typed prompt-not-typeable ShellError for a pty-cli prompt carrying a C0 control a terminal would interpret', async () => {
    const error: ShellError = {
      code: 'prompt-not-typeable',
      message: 'prompt contains control characters a terminal would interpret',
    }
    mockIPC((cmd) => {
      if (cmd === 'harness_spawn') return Promise.reject(error)
      throw new Error(`unexpected command: ${cmd}`)
    })

    const esc = String.fromCharCode(0x1b)
    await expect(
      harnessSpawn(
        { type: 'adapter', adapterId: 'pty-cli', approvalId: 1, prompt: `hi${esc}[2J` },
        () => {},
      ),
    ).rejects.toMatchObject(error)
    expect(isShellError(error)).toBe(true)
  })
})

// -- Slice 5: the outbox status and candidates commands, the artifact-publish
// and candidates event kinds, and the two outbox error codes
// (`docs/spike-log.md` § Slice 5) --

/**
 * `outbox_status` for a valid, existing outbox. `outbox` is the canonical
 * outbox path -- the third explicit RCS-001-R14 exception: identity
 * evidence of where a run's output lands, display-only, never sent back
 * in any command.
 */
const OUTBOX_STATUS_VALID: OutboxStatus = {
  declared: '.omnifrons/outbox',
  outbox: '/home/user/project/.omnifrons/outbox',
  exists: true,
  state: 'valid',
  reason: null,
  policyPath: '.omnifrons/asset-policy.json',
  assetRootId: 'main',
}

const RUN_ID = 'run-1725782401-000000001-0'

/**
 * A validated, attributed candidate and a refused (`outbox-linked`) entry
 * carrying no digest facts at all (HAP-001-R20), as `candidates_list`
 * returns them.
 */
const SAMPLE_CANDIDATES: Candidate[] = [
  {
    name: `${RUN_ID}/report.pdf`,
    size: 4096,
    sha256: 'ab'.repeat(32),
    sha256Short: 'abababab',
    detectedType: 'pdf',
    class: 'generated-heavy',
    attribution: { kind: 'run', runId: RUN_ID },
    state: 'candidate',
  },
  {
    name: `${RUN_ID}/linked.bin`,
    size: null,
    sha256: null,
    sha256Short: null,
    detectedType: null,
    class: null,
    attribution: { kind: 'unattributed' },
    state: 'outbox-linked',
  },
]

describe('outboxStatus (slice 5)', () => {
  it('invokes outbox_status with no arguments and returns the status verbatim for a valid, existing outbox', async () => {
    mockIPC((cmd, args) => {
      if (cmd === 'outbox_status') {
        expect(args).toEqual({})
        return OUTBOX_STATUS_VALID
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await outboxStatus()

    expect(result).toEqual(OUTBOX_STATUS_VALID)
    // Since slice 5b the status carries the policy's asset root identity
    // token -- the destination the approval surface shows before the
    // decision (HAP-001-R22) -- never a path.
    expect(result.assetRootId).toBe('main')
    expect(Object.keys(result).sort()).toEqual([
      'assetRootId',
      'declared',
      'exists',
      'outbox',
      'policyPath',
      'reason',
      'state',
    ])
  })

  it('returns a valid, not-yet-created status and an outbox-invalid status with its reason token verbatim', async () => {
    const notYetCreated: OutboxStatus = {
      ...OUTBOX_STATUS_VALID,
      outbox: null,
      exists: false,
      assetRootId: null,
    }
    const invalid: OutboxStatus = {
      declared: 'elsewhere/outbox',
      outbox: null,
      exists: true,
      state: 'outbox-invalid',
      reason: 'link',
      policyPath: '.omnifrons/asset-policy.json',
      assetRootId: 'main',
    }
    const responses: OutboxStatus[] = [notYetCreated, invalid]
    mockIPC((cmd) => {
      if (cmd === 'outbox_status') return responses.shift()
      throw new Error(`unexpected command: ${cmd}`)
    })

    expect(await outboxStatus()).toEqual(notYetCreated)
    expect(await outboxStatus()).toEqual(invalid)
  })
})

describe('candidatesList (slice 5)', () => {
  it('invokes candidates_list with exactly { runId } and returns the candidates verbatim, a refused entry carrying null facts', async () => {
    let capturedArgs: Record<string, unknown> | undefined
    mockIPC((cmd, args) => {
      if (cmd === 'candidates_list') {
        capturedArgs = args as Record<string, unknown>
        return SAMPLE_CANDIDATES
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await candidatesList(RUN_ID)

    expect(capturedArgs).toEqual({ runId: RUN_ID })
    expect(Object.keys(capturedArgs!)).toEqual(['runId'])
    expect(result).toEqual(SAMPLE_CANDIDATES)
    // Since slice 5b the row carries its full digest beside the short one
    // (identity evidence, the `Evidence.sha256` precedent), `null` for a
    // refused entry like every other digested fact.
    expect(result[0]?.sha256).toBe('ab'.repeat(32))
    expect(result[0]?.sha256).toHaveLength(64)
    expect(Object.keys(result[0]!).sort()).toEqual([
      'attribution',
      'class',
      'detectedType',
      'name',
      'sha256',
      'sha256Short',
      'size',
      'state',
    ])
    expect(result[1]).toMatchObject({
      size: null,
      sha256: null,
      sha256Short: null,
      detectedType: null,
      class: null,
      state: 'outbox-linked',
    })
  })

  it('invokes candidates_list with exactly {} for the whole outbox when no runId is given, never a runId key', async () => {
    let capturedArgs: Record<string, unknown> | undefined
    mockIPC((cmd, args) => {
      if (cmd === 'candidates_list') {
        capturedArgs = args as Record<string, unknown>
        return []
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await candidatesList()

    expect(capturedArgs).toEqual({})
    expect(Object.keys(capturedArgs!)).toEqual([])
    expect(result).toEqual([])
  })

  it('rejects with the typed outbox-invalid ShellError when the policy cannot be loaded', async () => {
    const error: ShellError = {
      code: 'outbox-invalid',
      message: 'the classification policy could not be loaded',
    }
    mockIPC((cmd) => {
      if (cmd === 'candidates_list') return Promise.reject(error)
      throw new Error(`unexpected command: ${cmd}`)
    })

    await expect(candidatesList()).rejects.toMatchObject(error)
    expect(isShellError(error)).toBe(true)
  })
})

describe('HarnessFrame outbox event kinds (slice 5)', () => {
  it('delivers an artifact-publish event frame verbatim, its entries named by full digest', async () => {
    const { channel, received } = await spawnAdapterAndCapture('claude-code')

    const frame: HarnessFrame = {
      stream: 'event',
      body: {
        id: 42,
        seq: 6,
        droppedBefore: 0,
        kind: 'artifact-publish',
        payload: { entries: [{ name: 'report.pdf', sha256: 'ab'.repeat(32) }] },
      },
    }
    channel.onmessage(frame)

    expect(received).toEqual([frame])
    const delivered = received[0]
    if (delivered?.stream !== 'event' || delivered.body.kind !== 'artifact-publish') {
      throw new Error('expected an artifact-publish event frame')
    }
    expect(delivered.body.payload.entries).toHaveLength(1)
    expect(delivered.body.payload.entries[0]?.sha256).toHaveLength(64)
  })

  it('delivers a candidates event frame with exactly the nine camelCase fields verbatim', async () => {
    const { channel, received } = await spawnAdapterAndCapture('claude-code')

    const frame: HarnessFrame = {
      stream: 'event',
      body: {
        id: 42,
        seq: 9,
        droppedBefore: 0,
        kind: 'candidates',
        payload: {
          runId: RUN_ID,
          total: 5,
          candidate: 3,
          outboxEscape: 1,
          outboxLinked: 1,
          attributed: 2,
          unattributed: 3,
          unreadable: 0,
          unmatchedProposals: 1,
        },
      },
    }
    channel.onmessage(frame)

    expect(received).toEqual([frame])
    const delivered = received[0]
    if (delivered?.stream !== 'event' || delivered.body.kind !== 'candidates') {
      throw new Error('expected a candidates event frame')
    }
    expect(Object.keys(delivered.body.payload).sort()).toEqual([
      'attributed',
      'candidate',
      'outboxEscape',
      'outboxLinked',
      'runId',
      'total',
      'unattributed',
      'unmatchedProposals',
      'unreadable',
    ])
  })
})

describe('ShellErrorCode outbox codes (slice 5)', () => {
  it('harnessSpawn rejects with the typed outbox-unavailable ShellError for an adapter launch whose run subdirectory could not be prepared, nothing spawned', async () => {
    const error: ShellError = {
      code: 'outbox-unavailable',
      message: 'something already sits at the run subdirectory path; remove it and relaunch',
    }
    mockIPC((cmd) => {
      if (cmd === 'harness_spawn') return Promise.reject(error)
      throw new Error(`unexpected command: ${cmd}`)
    })

    await expect(
      harnessSpawn(
        { type: 'adapter', adapterId: 'claude-code', approvalId: 1, prompt: 'hi' },
        () => {},
      ),
    ).rejects.toMatchObject(error)
    expect(isShellError(error)).toBe(true)
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

// -- Slice 5b: approval, publication, and the Catalog listing --

const PUBLICATION_ID = '2a91ea59fc5047831d97dbaebd763a3de37a5b382b683be59d10d6bdcdd92d4e'

/** The full digest of the entry `candidates_list` lists as `061eb0a8`: 64 hex characters. */
const REPORT_DIGEST = `061eb0a8${'c'.repeat(56)}`

/**
 * `artifact_approve`'s payload for the fixture run's attributed entry
 * (`docs/spike-log.md` § Slice 5b, IPC shapes): every field a logical
 * value, the approval id 16 hex, the publication identity 64, the act-as
 * identity the device-local user. Never a device path.
 */
const SAMPLE_ARTIFACT_APPROVAL: ArtifactApproval = {
  approvalId: '23121521465ad9be',
  publicationId: PUBLICATION_ID,
  runId: RUN_ID,
  name: `${RUN_ID}/report.pdf`,
  displayName: 'report.pdf',
  sha256Short: '061eb0a8',
  size: 37,
  detectedType: 'pdf',
  class: 'generated-heavy',
  attribution: { kind: 'run', runId: RUN_ID },
  assetRootId: 'main',
  actAs: 'device-local-user',
  approvedAt: 1725782401000,
}

/** A `registered` publication: the reference is AEC-001's `ref`, the locator the Catalog identity. */
const REGISTERED_PUBLICATION: Publication = {
  publicationId: PUBLICATION_ID,
  state: 'registered',
  reference: { kind: 'artifact', id: PUBLICATION_ID, locator: `main/${PUBLICATION_ID}` },
  providerState: 'pending',
  catalogId: `main/${PUBLICATION_ID}`,
  names: ['report.pdf'],
  availability: 'local',
}

/** A registration that failed after a verified `published-local`: no reference before registration (HAP-001-R24). */
const PENDING_PUBLICATION: Publication = {
  publicationId: PUBLICATION_ID,
  state: 'registration-pending',
  reference: null,
  providerState: null,
  catalogId: null,
  names: ['report.pdf'],
  availability: 'local',
}

type LiveStateChannel = { onmessage: (frame: ArtifactStateFrame) => void }

describe('artifactApprove (slice 5b)', () => {
  it('invokes artifact_approve with exactly { runId, name, sha256 } -- the full 64-hex digest -- and returns the approval verbatim, with no path-shaped key', async () => {
    let capturedArgs: Record<string, unknown> | undefined
    mockIPC((cmd, args) => {
      if (cmd === 'artifact_approve') {
        capturedArgs = args as Record<string, unknown>
        return SAMPLE_ARTIFACT_APPROVAL
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const approval = await artifactApprove(RUN_ID, `${RUN_ID}/report.pdf`, REPORT_DIGEST)

    expect(approval).toEqual(SAMPLE_ARTIFACT_APPROVAL)
    expect(capturedArgs).toEqual({
      runId: RUN_ID,
      name: `${RUN_ID}/report.pdf`,
      sha256: REPORT_DIGEST,
    })
    expect(Object.keys(capturedArgs!).sort()).toEqual(['name', 'runId', 'sha256'])
    expect(capturedArgs!.sha256).toHaveLength(64)
    for (const key of Object.keys(capturedArgs!)) {
      expect(key.toLowerCase()).not.toContain('path')
    }
    expect(Object.keys(approval).sort()).toEqual([
      'actAs',
      'approvalId',
      'approvedAt',
      'assetRootId',
      'attribution',
      'class',
      'detectedType',
      'displayName',
      'name',
      'publicationId',
      'runId',
      'sha256Short',
      'size',
    ])
  })

  it('returns an unattributed approval verbatim: its attribution the bare unattributed kind, nothing standing in for a producer', async () => {
    const unattributed: ArtifactApproval = {
      ...SAMPLE_ARTIFACT_APPROVAL,
      approvalId: '5f0c5f0c5f0c5f0c',
      name: `${RUN_ID}/stray.png`,
      displayName: 'stray.png',
      sha256Short: '11111111',
      size: 11,
      detectedType: 'png',
      attribution: { kind: 'unattributed' },
    }
    mockIPC((cmd) => {
      if (cmd === 'artifact_approve') return unattributed
      throw new Error(`unexpected command: ${cmd}`)
    })

    const approval = await artifactApprove(RUN_ID, `${RUN_ID}/stray.png`, `11111111${'d'.repeat(56)}`)

    expect(approval).toEqual(unattributed)
    expect(approval.attribution).toEqual({ kind: 'unattributed' })
    expect(Object.keys(approval.attribution)).toEqual(['kind'])
  })

  it('rejects with the typed refused, destination-invalid and run-active ShellErrors', async () => {
    const refused: ShellError = {
      code: 'refused',
      message: 'only a generated-heavy candidate can be approved for publication',
    }
    const destination: ShellError = {
      code: 'destination-invalid',
      message: 'the project declares no asset root',
    }
    // The backend mirror of the renderer's frozen surface: refused while
    // any supervised process runs (`docs/spike-log.md` § Slice 5b).
    const runActive: ShellError = {
      code: 'run-active',
      message: 'a run is active; approve or publish once it has ended',
    }
    for (const error of [refused, destination, runActive]) {
      mockIPC((cmd) => {
        if (cmd === 'artifact_approve') return Promise.reject(error)
        throw new Error(`unexpected command: ${cmd}`)
      })

      await expect(
        artifactApprove(RUN_ID, `${RUN_ID}/notes.md`, REPORT_DIGEST),
      ).rejects.toMatchObject(error)
      expect(isShellError(error)).toBe(true)
      clearMocks()
    }
  })
})

describe('artifactPublish (slice 5b)', () => {
  it('invokes artifact_publish with exactly { approvalId, onState }, onState the live Channel, and returns the registered publication verbatim, its reference exactly { kind, id, locator }, no path-shaped key', async () => {
    let capturedArgs: Record<string, unknown> | undefined
    mockIPC((cmd, args) => {
      if (cmd === 'artifact_publish') {
        capturedArgs = args as Record<string, unknown>
        return REGISTERED_PUBLICATION
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const publication = await artifactPublish('23121521465ad9be', () => {})

    expect(publication).toEqual(REGISTERED_PUBLICATION)
    expect(Object.keys(capturedArgs!).sort()).toEqual(['approvalId', 'onState'])
    expect(capturedArgs!.approvalId).toBe('23121521465ad9be')
    expect(typeof (capturedArgs!.onState as LiveStateChannel).onmessage).toBe('function')
    for (const key of Object.keys(capturedArgs!)) {
      expect(key.toLowerCase()).not.toContain('path')
    }
    expect(Object.keys(publication).sort()).toEqual([
      'availability',
      'catalogId',
      'names',
      'providerState',
      'publicationId',
      'reference',
      'state',
    ])
    if (publication.reference === null) throw new Error('expected a reference')
    expect(Object.keys(publication.reference).sort()).toEqual(['id', 'kind', 'locator'])
    expect(publication.reference.kind).toBe('artifact')
  })

  it('delivers artifact-state frames sent on the live Channel to onState, in order, each payload exactly { publicationId, state, providerState }', async () => {
    let channelRef: LiveStateChannel | undefined
    mockIPC((cmd, args) => {
      if (cmd === 'artifact_publish') {
        channelRef = (args as Record<string, unknown>).onState as LiveStateChannel
        return REGISTERED_PUBLICATION
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const received: ArtifactStateFrame[] = []
    await artifactPublish('23121521465ad9be', (frame) => {
      received.push(frame)
    })
    if (!channelRef) throw new Error('artifact_publish was not called')
    const frames: ArtifactStateFrame[] = [
      {
        kind: 'artifact-state',
        payload: { publicationId: PUBLICATION_ID, state: 'published-local', providerState: null },
      },
      {
        kind: 'artifact-state',
        payload: { publicationId: PUBLICATION_ID, state: 'registered', providerState: 'pending' },
      },
    ]
    for (const frame of frames) channelRef.onmessage(frame)

    expect(received).toEqual(frames)
    expect(received.map((frame) => frame.payload.state)).toEqual(['published-local', 'registered'])
    expect(Object.keys(received[0]!.payload).sort()).toEqual([
      'providerState',
      'publicationId',
      'state',
    ])
  })

  it('returns a registration-pending publication verbatim: null reference, provider state and catalog id, its names and availability kept', async () => {
    mockIPC((cmd) => {
      if (cmd === 'artifact_publish') return PENDING_PUBLICATION
      throw new Error(`unexpected command: ${cmd}`)
    })

    const publication = await artifactPublish('23121521465ad9be', () => {})

    expect(publication).toEqual(PENDING_PUBLICATION)
    expect(publication.reference).toBeNull()
    expect(publication.providerState).toBeNull()
    expect(publication.catalogId).toBeNull()
  })

  it("rejects with the typed duplicate-publication ShellError carrying the existing record's publicationId and catalogId as detail, and with a typed integrity-mismatch carrying no detail", async () => {
    const duplicate: ShellError = {
      code: 'duplicate-publication',
      message: 'an artifact with this content is already registered for this project',
      detail: { publicationId: PUBLICATION_ID, catalogId: `main/${PUBLICATION_ID}` },
    }
    const mismatch: ShellError = {
      code: 'integrity-mismatch',
      message:
        'the published copy did not verify against the approved digest; it was discarded and the entry preserved',
    }
    const runActive: ShellError = {
      code: 'run-active',
      message: 'a run is active; approve or publish once it has ended',
    }
    for (const error of [duplicate, mismatch, runActive]) {
      mockIPC((cmd) => {
        if (cmd === 'artifact_publish') return Promise.reject(error)
        throw new Error(`unexpected command: ${cmd}`)
      })

      await expect(artifactPublish('23121521465ad9be', () => {})).rejects.toMatchObject(error)
      expect(isShellError(error)).toBe(true)
      clearMocks()
    }
    expect(Object.keys(duplicate.detail!).sort()).toEqual(['catalogId', 'publicationId'])
    expect(mismatch.detail).toBeUndefined()
  })
})

describe('publicationsList (slice 5b)', () => {
  it('invokes publications_list with no arguments and returns the records verbatim, a registered and a registration-pending one', async () => {
    const records = [
      { ...REGISTERED_PUBLICATION, names: ['report.pdf', 'report-copy.pdf'] },
      { ...PENDING_PUBLICATION, publicationId: 'b'.repeat(64) },
    ]
    mockIPC((cmd, args) => {
      if (cmd === 'publications_list') {
        expect(args).toEqual({})
        return records
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const result = await publicationsList()

    expect(result).toEqual(records)
    expect(result[0]?.names).toEqual(['report.pdf', 'report-copy.pdf'])
    expect(result[1]?.reference).toBeNull()
  })

  it('rejects with the typed catalog-unavailable and work-area-invalid ShellErrors', async () => {
    const catalog: ShellError = { code: 'catalog-unavailable', message: 'the catalog is corrupt' }
    const workArea: ShellError = {
      code: 'work-area-invalid',
      message: 'the product work area resolves inside a registered workspace root',
    }
    for (const error of [catalog, workArea]) {
      mockIPC((cmd) => {
        if (cmd === 'publications_list') return Promise.reject(error)
        throw new Error(`unexpected command: ${cmd}`)
      })

      await expect(publicationsList()).rejects.toMatchObject(error)
      expect(isShellError(error)).toBe(true)
      clearMocks()
    }
  })
})

describe('slice 5b closed sets', () => {
  it('compile-time guard, enforced by tsc -b in pnpm -r build and not by vitest: a switch over ArtifactState with a never default compiles, and at runtime maps each of the ten tokens to itself', () => {
    function stateLabel(state: ArtifactState): string {
      switch (state) {
        case 'candidate':
        case 'published-local':
        case 'registered':
        case 'provider-synced':
        case 'registration-pending':
        case 'refused':
        case 'integrity-mismatch':
        case 'duplicate-publication':
        case 'outbox-escape':
        case 'outbox-linked':
          return state
        default: {
          const unreachable: never = state
          return unreachable
        }
      }
    }

    const states: ArtifactState[] = [
      'candidate',
      'published-local',
      'registered',
      'provider-synced',
      'registration-pending',
      'refused',
      'integrity-mismatch',
      'duplicate-publication',
      'outbox-escape',
      'outbox-linked',
    ]
    expect(states.map(stateLabel)).toEqual(states)
  })

  it('compile-time guard, enforced by tsc -b in pnpm -r build and not by vitest: switches over ProviderState (four tokens) and Availability (two) with never defaults compile, and at runtime map each token to itself', () => {
    function providerLabel(state: ProviderState): string {
      switch (state) {
        case 'pending':
        case 'synced':
        case 'failed':
        case 'unavailable':
          return state
        default: {
          const unreachable: never = state
          return unreachable
        }
      }
    }
    function availabilityLabel(availability: Availability): string {
      switch (availability) {
        case 'local':
        case 'unknown':
          return availability
        default: {
          const unreachable: never = availability
          return unreachable
        }
      }
    }

    const providerStates: ProviderState[] = ['pending', 'synced', 'failed', 'unavailable']
    const availabilities: Availability[] = ['local', 'unknown']
    expect(providerStates.map(providerLabel)).toEqual(providerStates)
    expect(availabilities.map(availabilityLabel)).toEqual(availabilities)
  })

  it('compile-time guard, enforced by tsc -b in pnpm -r build and not by vitest: a switch over the whole ShellErrorCode union with a never default compiles -- the nine slice 5b codes among the thirty-two -- and at runtime maps each code to itself; artifact-state is the one closed frame kind of the publish channel', () => {
    function codeLabel(code: ShellErrorCode): string {
      switch (code) {
        case 'unknown-process':
        case 'spawn-failed':
        case 'already-subscribed':
        case 'invalid-request':
        case 'too-many-processes':
        case 'no-candidate':
        case 'not-executable':
        case 'probe-failed':
        case 'too-large':
        case 'unapproved':
        case 'changed-since-approval':
        case 'shadowed-path':
        case 'revoked':
        case 'approval-store-unavailable':
        case 'unknown-adapter':
        case 'prompt-too-large':
        case 'workspace-unavailable':
        case 'secret-shaped-env':
        case 'no-workspace':
        case 'pty-unsupported':
        case 'prompt-not-typeable':
        case 'outbox-invalid':
        case 'outbox-unavailable':
        case 'integrity-mismatch':
        case 'duplicate-publication':
        case 'work-area-invalid':
        case 'destination-invalid':
        case 'outbox-escape':
        case 'outbox-linked':
        case 'refused':
        case 'catalog-unavailable':
        case 'run-active':
          return code
        default: {
          const unreachable: never = code
          return unreachable
        }
      }
    }

    const codes: ShellErrorCode[] = [
      'unknown-process',
      'spawn-failed',
      'already-subscribed',
      'invalid-request',
      'too-many-processes',
      'no-candidate',
      'not-executable',
      'probe-failed',
      'too-large',
      'unapproved',
      'changed-since-approval',
      'shadowed-path',
      'revoked',
      'approval-store-unavailable',
      'unknown-adapter',
      'prompt-too-large',
      'workspace-unavailable',
      'secret-shaped-env',
      'no-workspace',
      'pty-unsupported',
      'prompt-not-typeable',
      'outbox-invalid',
      'outbox-unavailable',
      'integrity-mismatch',
      'duplicate-publication',
      'work-area-invalid',
      'destination-invalid',
      'outbox-escape',
      'outbox-linked',
      'refused',
      'catalog-unavailable',
      'run-active',
    ]
    expect(new Set(codes).size).toBe(32)
    expect(codes.map(codeLabel)).toEqual(codes)
    const kind: ArtifactStateFrame['kind'] = 'artifact-state'
    expect(kind).toBe('artifact-state')
  })
})
