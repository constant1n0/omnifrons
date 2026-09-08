import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { AgentPanel } from './AgentPanel'
import { ApprovalSurface } from './ApprovalSurface'
import { HarnessPanel } from './HarnessPanel'
import type {
  AdapterDescriptor,
  Approval,
  ArtifactApproval,
  ArtifactStateFrame,
  Candidate,
  Evidence,
  HarnessFrame,
  OutboxReason,
  OutboxStatus,
  Publication,
} from './ipc/harness'

// The jsdom crypto polyfill and React Testing Library's `cleanup()` are
// installed once for every test file by `testSupport/setup.ts` -- not
// repeated here.

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

const SAMPLE_ADAPTER: AdapterDescriptor = {
  id: 'claude-code',
  displayName: 'Claude Code',
  transportClass: 'structured-streaming-cli',
  promptChannel: 'stdin-then-close',
  scopeMode: 'advisory',
  notes: 'credentials are harness-owned; not exercised in CI',
}

/**
 * The spike slice-4 pseudo-terminal fallback adapter, mirroring the
 * `pty-cli` descriptor `adapters_list` returns (`docs/spike-log.md` § Slice
 * 4, IPC shapes); `notes` is the Rust side's own disclosure text verbatim.
 */
const PTY_ADAPTER: AdapterDescriptor = {
  id: 'pty-cli',
  displayName: 'PTY CLI',
  transportClass: 'pty',
  promptChannel: 'pty-typed',
  scopeMode: 'advisory',
  notes:
    'degraded fallback with no structured events: the executable runs inside a pseudo-terminal and its output is rendered as plain text with layout controls dropped and counted; a title or notification it asks for is sanitized and shown as text only; hyperlinks, clipboard access and file transfer are disabled; the terminal is not a sandbox and grants no authority; not available on Windows in this slice',
}

/**
 * `outbox_status` for a valid, existing outbox under the sample workspace
 * (`docs/spike-log.md` § Slice 5, IPC shapes). `outbox` is the canonical
 * outbox path -- the third explicit RCS-001-R14 exception: identity
 * evidence of where a run's output lands, display-only, never sent back.
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

/**
 * What `outbox_status` answers with no active workspace: the same
 * `workspace-unavailable` rejection an adapter launch gets (`docs/spike-log.md`
 * § Slice 5, IPC shapes), which the panel meets with no status line and no
 * alert -- the missing workspace is already visible.
 */
const OUTBOX_NO_WORKSPACE = {
  code: 'workspace-unavailable',
  message: 'no workspace has been picked yet',
}

/**
 * The default mock: no active workspace (so `outbox_status` rejects
 * `workspace-unavailable`, as the shell does), two adapters (the slice 3
 * line agent and the slice 4 pseudo-terminal fallback), one active
 * approval. Individual tests override `onCommand` to add `harness_spawn`/
 * `workspace_pick` handling.
 */
/**
 * Overrides for the default mock (slice 5b): `outbox` answers `outbox_status`
 * with a status instead of the no-workspace rejection -- the approval block
 * reads its destination from it -- and `adapters` replaces the adapter list.
 */
interface MockOptions {
  outbox?: OutboxStatus
  adapters?: AdapterDescriptor[]
}

function defaultHandlers(
  onCommand?: (cmd: string, args: Record<string, unknown>) => unknown,
  options: MockOptions = {},
) {
  return (cmd: string, args: unknown) => {
    if (cmd === 'workspace_current') return null
    if (cmd === 'outbox_status') return options.outbox ?? Promise.reject(OUTBOX_NO_WORKSPACE)
    if (cmd === 'publications_list') return Promise.reject(OUTBOX_NO_WORKSPACE)
    if (cmd === 'adapters_list') return options.adapters ?? [SAMPLE_ADAPTER, PTY_ADAPTER]
    if (cmd === 'approvals_list') return [sampleApproval({ approvalId: 42 })]
    if (onCommand) return onCommand(cmd, args as Record<string, unknown>)
    throw new Error(`unexpected command: ${cmd}`)
  }
}

/**
 * Selects `value` in the labelled select only once that option exists.
 * Both option lists are populated by a mount-time IPC fetch, so changing
 * the select before its list has arrived silently selects nothing (the
 * value falls back to "") and leaves Start disabled -- the race behind an
 * intermittent CI failure in the deferred-spawn tests.
 */
async function selectOption(labelText: string, value: string): Promise<void> {
  const select = (await screen.findByLabelText(labelText)) as HTMLSelectElement
  await waitFor(() => {
    expect(Array.from(select.options).map((option) => option.value)).toContain(value)
  })
  fireEvent.change(select, { target: { value } })
  expect(select.value).toBe(value)
}

async function renderReady(
  onCommand?: (cmd: string, args: Record<string, unknown>) => unknown,
  adapterId = 'claude-code',
  options?: MockOptions,
) {
  mockIPC(defaultHandlers(onCommand, options))
  render(<AgentPanel />)
  await selectOption('Adapter', adapterId)
  await selectOption('Approval', '42')
  fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: 'do the thing' } })
}

/** Fills in the required selects, starts a run, and returns the live Channel. */
async function startAndCaptureChannel(
  onCommand?: (cmd: string, args: Record<string, unknown>) => unknown,
  adapterId = 'claude-code',
  options?: MockOptions,
): Promise<{ onmessage: (frame: HarnessFrame) => void }> {
  let channelRef: { onmessage: (frame: HarnessFrame) => void } | undefined
  await renderReady(
    (cmd, args) => {
      if (cmd === 'harness_spawn') {
        channelRef = (args as { onFrame: { onmessage: (frame: HarnessFrame) => void } }).onFrame
        return 7
      }
      if (onCommand) return onCommand(cmd, args)
      throw new Error(`unexpected command: ${cmd}`)
    },
    adapterId,
    options,
  )

  fireEvent.click(screen.getByRole('button', { name: 'Start' }))
  await waitFor(() => {
    expect(channelRef).toBeDefined()
  })
  await screen.findByText('running')

  if (!channelRef) throw new Error('harness_spawn was not called')
  return channelRef
}

beforeEach(() => {
  window.sessionStorage.clear()
})

afterEach(() => {
  clearMocks()
})

describe('AgentPanel', () => {
  it('renders the workspace, adapter, approval, and prompt controls', async () => {
    await renderReady()

    expect(screen.getByRole('button', { name: 'Pick workspace' })).toBeTruthy()
    expect(screen.getByLabelText('Adapter')).toBeTruthy()
    expect(screen.getByLabelText('Approval')).toBeTruthy()
    expect(screen.getByLabelText('Prompt')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Start' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Stop' })).toBeTruthy()
  })

  it('calls workspace_current on mount and shows the display path through PlainTextLine', async () => {
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return { displayPath: '/home/user/project', workArea: 'valid' }
      if (cmd === 'outbox_status') return OUTBOX_STATUS_VALID
      if (cmd === 'publications_list') return []
      if (cmd === 'adapters_list') return [SAMPLE_ADAPTER]
      if (cmd === 'approvals_list') return [sampleApproval()]
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)

    await screen.findByText('/home/user/project')
  })

  it('shows the sent prompt echoed as an untrusted "you" entry, as text', async () => {
    const channel = await startAndCaptureChannel()
    void channel

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toContain('do the thing')
  })

  it('renders a message event with a <b> tag as text, producing no <b> element', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 0,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: '<b>bold</b>' },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.querySelector('b')).toBeNull()
    expect(transcript.textContent).toContain('<b>bold</b>')
  })

  it('renders a toolCall event inside a labelled "proposal, not executed" block with zero interactive elements, no interactive attributes, and no IPC on click (R3-007)', async () => {
    // Every IPC command is counted at the top of the mock, whatever it is,
    // so a click that reached any handler at all -- not only an unexpected
    // command -- would show up.
    let invokeCount = 0
    let channelRef: { onmessage: (frame: HarnessFrame) => void } | undefined
    const handlers = defaultHandlers((cmd, args) => {
      if (cmd === 'harness_spawn') {
        channelRef = (args as { onFrame: { onmessage: (frame: HarnessFrame) => void } }).onFrame
        return 7
      }
      return null
    })
    mockIPC((cmd, args) => {
      invokeCount += 1
      return handlers(cmd, args)
    })

    render(<AgentPanel />)
    await selectOption('Adapter', 'claude-code')
    await selectOption('Approval', '42')
    fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: 'do the thing' } })
    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await waitFor(() => {
      expect(channelRef).toBeDefined()
    })
    await screen.findByText('running')
    if (!channelRef) throw new Error('harness_spawn was not called')

    act(() => {
      channelRef!.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 1,
          droppedBefore: 0,
          kind: 'tool-call',
          payload: { name: 'write_file', argumentsText: '{"path":"notes.md"}' },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toContain('proposal, not executed')
    expect(transcript.textContent).toContain('write_file')
    expect(transcript.textContent).toContain('{"path":"notes.md"}')

    const proposal = screen.getByTestId('tool-call-proposal')
    expect(
      proposal.querySelectorAll('button, a, input, textarea, select, details, summary').length,
    ).toBe(0)
    expect(
      proposal.querySelectorAll('[role], [tabindex], [href], [contenteditable], [onclick]')
        .length,
    ).toBe(0)

    const blockAndDescendants = [proposal, ...Array.from(proposal.querySelectorAll('*'))]
    for (const element of blockAndDescendants) {
      expect(element.getAttribute('role')).toBeNull()
      expect(element.getAttribute('tabindex')).toBeNull()
      expect(element.getAttribute('href')).toBeNull()
      expect(element.getAttribute('contenteditable')).toBeNull()
      expect(element.getAttribute('onclick')).toBeNull()
    }

    // React exposes no handler as a DOM attribute, so the only honest
    // check for an `onClick` is behavioral: click the block and every
    // descendant, then confirm no IPC command was invoked at all.
    const invokesBeforeClick = invokeCount
    for (const element of blockAndDescendants) {
      fireEvent.click(element)
    }
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })
    expect(invokeCount).toBe(invokesBeforeClick)
  })

  it('renders an unknown event raw line verbatim after control stripping', async () => {
    const channel = await startAndCaptureChannel()
    const esc = String.fromCharCode(0x1b)

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 2,
          droppedBefore: 0,
          kind: 'unknown',
          payload: { raw: `not${esc} json`, truncated: false },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toContain('not json')
    expect(transcript.textContent).not.toContain('(truncated)')
  })

  it('shows "(truncated)" for an unknown event flagged truncated', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 3,
          droppedBefore: 0,
          kind: 'unknown',
          payload: { raw: 'partial line', truncated: true },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toContain('partial line')
    expect(transcript.textContent).toContain('(truncated)')
  })

  it('renders a state event as a compact phase/subtype/observations line', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 4,
          droppedBefore: 0,
          kind: 'state',
          payload: {
            phase: 'init',
            subtype: null,
            observations: [
              { key: 'cwd', value: '/work' },
              { key: 'model', value: 'fake-model' },
            ],
          },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toContain('init')
    expect(transcript.textContent).toContain('cwd: /work')
    expect(transcript.textContent).toContain('model: fake-model')
  })

  it('renders a diagnostic event prefixed "diagnostic"', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 5,
          droppedBefore: 0,
          kind: 'diagnostic',
          payload: { text: 'hello' },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toContain('diagnostic')
    expect(transcript.textContent).toContain('hello')
  })

  it('shows the degraded marker when droppedBefore is nonzero', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 6,
          droppedBefore: 3,
          kind: 'message',
          payload: { text: 'hi' },
        },
      })
    })

    expect(
      screen.getByText('3 frames dropped before this entry — degraded, restart to replay'),
    ).toBeTruthy()
  })

  it('caps the transcript to the last 2000 entries and shows a cumulative trimmed marker', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      for (let seq = 0; seq < 2500; seq += 1) {
        channel.onmessage({
          stream: 'event',
          body: {
            id: 7,
            seq,
            droppedBefore: 0,
            kind: 'message',
            payload: { text: `line ${seq}` },
          },
        })
      }
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toContain('line 2499')
    expect(transcript.textContent).not.toContain('line 0')
    // The echoed prompt plus 2500 events is 2501 entries: the overflow K
    // is 501 (the prompt and lines 0-499), so the boundary sits between
    // line 499 (the last one evicted) and line 500 (the oldest survivor).
    expect(transcript.textContent).not.toContain('line 499')
    expect(transcript.textContent).toContain('line 500')
    // one marker row plus the 2000 surviving entry rows.
    expect(transcript.querySelectorAll('li').length).toBe(2001)
    expect(
      screen.getByText('showing the last 2000 entries; 501 earlier entries are not shown'),
    ).toBeTruthy()
  })

  it('shows the exact advisory scope badge text, and "sandbox" appears only inside it', async () => {
    await renderReady()

    const badge = screen.getByTestId('agent-scope-badge')
    expect(badge.textContent).toBe('advisory scope — not a sandbox')

    const sandboxMentions = Array.from(document.body.querySelectorAll('*')).filter(
      (el) => el.children.length === 0 && (el.textContent ?? '').includes('sandbox'),
    )
    expect(sandboxMentions).toEqual([badge])
  })

  it('shows the producer line', async () => {
    await renderReady()

    expect(screen.getByText('producer: unsigned (unknown)')).toBeTruthy()
  })

  it('disables Start and shows an inline message when the prompt exceeds 16 KiB', async () => {
    await renderReady()

    const promptInput = screen.getByLabelText('Prompt') as HTMLTextAreaElement
    fireEvent.change(promptInput, { target: { value: 'a'.repeat(16 * 1024 + 1) } })

    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    expect(screen.getByTestId('agent-prompt-size-validation')).toBeTruthy()
  })

  it('re-enables Start once the prompt is brought back under the 16 KiB cap', async () => {
    await renderReady()

    const promptInput = screen.getByLabelText('Prompt') as HTMLTextAreaElement
    fireEvent.change(promptInput, { target: { value: 'a'.repeat(16 * 1024 + 1) } })
    fireEvent.change(promptInput, { target: { value: 'short prompt' } })

    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
    expect(screen.queryByTestId('agent-prompt-size-validation')).toBeNull()
  })

  it('shows the workspace display path after a successful pick', async () => {
    mockIPC(
      defaultHandlers((cmd) => {
        if (cmd === 'workspace_pick') return { displayPath: '/home/user/project', workArea: 'valid' }
        throw new Error(`unexpected command: ${cmd}`)
      }),
    )

    render(<AgentPanel />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))

    await screen.findByText('/home/user/project')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('shows a quiet status, without an alert, when the workspace pick is canceled', async () => {
    mockIPC(
      defaultHandlers((cmd) => {
        if (cmd === 'workspace_pick') {
          return Promise.reject({ code: 'no-workspace', message: 'no folder was selected' })
        }
        throw new Error(`unexpected command: ${cmd}`)
      }),
    )

    render(<AgentPanel />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))

    await screen.findByText('No workspace selected')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('renders the secret-shaped-env code when harness_spawn rejects with it', async () => {
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'secret-shaped-env',
          message: 'the adapter declared an environment variable that looks secret-shaped',
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toContain('secret-shaped-env')
  })

  it('renders the workspace-unavailable code when harness_spawn rejects with it', async () => {
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'workspace-unavailable',
          message: 'no workspace has been picked yet',
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toContain('workspace-unavailable')
  })

  it('shows a denial with the public "untrusted" wording for a revoked approval', async () => {
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'revoked',
          message: 'the approval for this executable was revoked',
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toContain('untrusted')
    expect(banner.textContent).toContain('revoked')
  })

  it('no-ops after unmount: a frame delivered post-unmount throws nothing and logs no console.error', async () => {
    let channelRef: { onmessage: (frame: HarnessFrame) => void } | undefined
    mockIPC(
      defaultHandlers((cmd, args) => {
        if (cmd === 'harness_spawn') {
          channelRef = (args as { onFrame: { onmessage: (frame: HarnessFrame) => void } }).onFrame
          return 7
        }
        throw new Error(`unexpected command: ${cmd}`)
      }),
    )

    const { unmount } = render(<AgentPanel />)
    await selectOption('Adapter', 'claude-code')
    await selectOption('Approval', '42')
    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await waitFor(() => {
      expect(channelRef).toBeDefined()
    })
    await screen.findByText('running')
    if (!channelRef) throw new Error('harness_spawn was not called')

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    unmount()

    expect(() => {
      channelRef!.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 99,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'after unmount' },
        },
      })
    }).not.toThrow()

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('the workspace-pick continuation no-ops after unmount', async () => {
    let pickCalled = false
    let resolvePick: (value: unknown) => void = () => {}
    mockIPC(
      defaultHandlers((cmd) => {
        if (cmd === 'workspace_pick') {
          pickCalled = true
          return new Promise((resolve) => {
            resolvePick = resolve
          })
        }
        throw new Error(`unexpected command: ${cmd}`)
      }),
    )

    const { unmount } = render(<AgentPanel />)
    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    await waitFor(() => {
      expect(pickCalled).toBe(true)
    })
    unmount()

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    resolvePick({ displayPath: '/home/user/project', workArea: 'valid' })
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('never renders the approval region inside HarnessPanel\'s output log, and no proposal block there either (RCS-001-R6)', async () => {
    mockIPC(defaultHandlers())

    render(
      <>
        <HarnessPanel />
        <AgentPanel />
        <ApprovalSurface />
      </>,
    )

    const region = await screen.findByRole('region', { name: 'Executable approval' })
    const outputLog = screen.getByLabelText('Output log')
    expect(outputLog.contains(region)).toBe(false)
    expect(outputLog.querySelector('[data-testid="tool-call-proposal"]')).toBeNull()
    expect(document.body.contains(region)).toBe(true)
  })

  it('never renders the ApprovalSurface region inside the AgentPanel transcript', async () => {
    mockIPC(defaultHandlers())

    render(
      <>
        <AgentPanel />
        <ApprovalSurface />
      </>,
    )

    const region = await screen.findByRole('region', { name: 'Executable approval' })
    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.contains(region)).toBe(false)
  })

  it('keeps a single active run: Start stays disabled once running', async () => {
    await startAndCaptureChannel()

    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
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

  it('lists only active approvals in the approval select', async () => {
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return null
      if (cmd === 'outbox_status') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'publications_list') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'adapters_list') return [SAMPLE_ADAPTER]
      if (cmd === 'approvals_list') {
        return [
          sampleApproval({ approvalId: 1, status: 'active' }),
          sampleApproval({ approvalId: 2, status: 'revoked', revokedAt: 600 }),
        ]
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)

    const approvalSelect = (await screen.findByLabelText('Approval')) as HTMLSelectElement
    await waitFor(() => {
      const values = Array.from(approvalSelect.options).map((option) => option.value)
      expect(values).toContain('1')
      expect(values).not.toContain('2')
    })
  })
})

type LiveChannel = { onmessage: (frame: HarnessFrame) => void }

/**
 * Renders the panel ready to start, with a `harness_spawn` mock that hands
 * out incrementing ids (7, 8, ...) and records every live Channel it
 * receives, so a test can drive more than one run and address each run's
 * own channel. `harness_stop` resolves `killed` unless `onCommand`
 * intercepts it first.
 */
async function renderMultiRun(
  onCommand?: (cmd: string, args: Record<string, unknown>) => unknown,
  adapterId = 'claude-code',
  options?: MockOptions,
): Promise<{ channels: LiveChannel[]; spawnCount: () => number }> {
  const channels: LiveChannel[] = []
  let nextId = 7
  await renderReady(
    (cmd, args) => {
      if (cmd === 'harness_spawn') {
        channels.push((args as { onFrame: LiveChannel }).onFrame)
        const id = nextId
        nextId += 1
        return id
      }
      if (onCommand) return onCommand(cmd, args)
      if (cmd === 'harness_stop') return { state: 'killed', code: null }
      throw new Error(`unexpected command: ${cmd}`)
    },
    adapterId,
    options,
  )
  return { channels, spawnCount: () => channels.length }
}

describe('AgentPanel run identity (R1-001 / R3-001)', () => {
  it('ignores a frame carrying a foreign run id, event and state kinds alike: nothing appended, badge unchanged', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 99,
          seq: 0,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'foreign event' },
        },
      })
      channel.onmessage({
        stream: 'state',
        body: { id: 99, seq: 1, droppedBefore: 0, state: 'killed', code: null },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).not.toContain('foreign event')
    expect(screen.getByText('running')).toBeTruthy()
    expect(screen.queryByText('killed')).toBeNull()
    expect((screen.getByRole('button', { name: 'Stop' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
  })

  it('once a second run has started, a trailing frame with the first run id is ignored while a frame with the new id is applied', async () => {
    const { channels } = await renderMultiRun()

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await screen.findByText('killed')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    expect(channels).toHaveLength(2)

    act(() => {
      channels[0]!.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 5,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'late from run A' },
        },
      })
      channels[1]!.onmessage({
        stream: 'event',
        body: {
          id: 8,
          seq: 0,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'fresh from run B' },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).not.toContain('late from run A')
    expect(transcript.textContent).toContain('fresh from run B')
  })
})

describe('AgentPanel terminal state frame (R3-002)', () => {
  it('ends the run: Start re-enabled, Stop disabled, and a fresh Start spawns a new run', async () => {
    const { channels, spawnCount } = await renderMultiRun()

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')

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

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    expect(spawnCount()).toBe(2)
  })

  it('shows the degraded marker for a state frame with nonzero droppedBefore (R3-003)', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'state',
        body: { id: 7, seq: 9, droppedBefore: 3, state: 'killed', code: null },
      })
    })

    expect(
      screen.getByText('3 frames dropped before this entry — degraded, restart to replay'),
    ).toBeTruthy()
  })
})

/**
 * Renders the panel ready to start, with a `harness_spawn` mock whose
 * promise stays pending until the test resolves it -- so frames can be
 * delivered on the live Channel *before* the panel learns the run's id.
 */
async function renderWithDeferredSpawn(): Promise<{
  channels: LiveChannel[]
  resolveSpawn: (id: number) => void
}> {
  const channels: LiveChannel[] = []
  let resolveSpawn: (id: number) => void = () => {}
  await renderReady((cmd, args) => {
    if (cmd === 'harness_spawn') {
      channels.push((args as { onFrame: LiveChannel }).onFrame)
      return new Promise<number>((resolve) => {
        resolveSpawn = resolve
      })
    }
    if (cmd === 'harness_stop') return { state: 'killed', code: null }
    throw new Error(`unexpected command: ${cmd}`)
  })
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

describe('AgentPanel spawn generation: frames before harness_spawn resolves, and old channels', () => {
  it('applies a terminal state frame delivered before the spawn resolves: Start ends enabled, Stop disabled, state line in the transcript', async () => {
    const { channels, resolveSpawn } = await renderWithDeferredSpawn()

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
    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toContain('state: exited (code 0)')
    // The echoed prompt still precedes the run's own output.
    expect(transcript.textContent?.indexOf('you: do the thing')).toBeLessThan(
      transcript.textContent?.indexOf('state: exited (code 0)') ?? -1,
    )
  })

  it('applies a non-terminal frame delivered before the spawn resolves, then runs normally and still drops a foreign id once the id is known', async () => {
    const { channels, resolveSpawn } = await renderWithDeferredSpawn()

    await clickStartAndAwaitChannel(channels, 1)
    act(() => {
      channels[0]!.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 0,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'early but ours' },
        },
      })
    })
    await settleSpawn(resolveSpawn, 7)

    expect(screen.getByText('running')).toBeTruthy()
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    expect((screen.getByRole('button', { name: 'Stop' }) as HTMLButtonElement).disabled).toBe(
      false,
    )

    act(() => {
      channels[0]!.onmessage({
        stream: 'event',
        body: {
          id: 99,
          seq: 1,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'foreign after id known' },
        },
      })
      channels[0]!.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 1,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'later and ours' },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toContain('early but ours')
    expect(transcript.textContent).toContain('later and ours')
    expect(transcript.textContent).not.toContain('foreign after id known')
  })

  it('rejects a frame from a previous run\'s channel while the next spawn is still pending, even with the new id unknown', async () => {
    const { channels, resolveSpawn } = await renderWithDeferredSpawn()

    await clickStartAndAwaitChannel(channels, 1)
    await settleSpawn(resolveSpawn, 7)
    await screen.findByText('running')
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await screen.findByText('killed')

    await clickStartAndAwaitChannel(channels, 2)

    // Run A's channel speaks up while run B's id is still unknown: a
    // frame carrying A's own id, and one carrying the id B will get.
    act(() => {
      channels[0]!.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 9,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'stale from run A' },
        },
      })
      channels[0]!.onmessage({
        stream: 'event',
        body: {
          id: 8,
          seq: 10,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'stale channel, forged id' },
        },
      })
      channels[1]!.onmessage({
        stream: 'event',
        body: {
          id: 8,
          seq: 0,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'early from run B' },
        },
      })
    })
    await settleSpawn(resolveSpawn, 8)

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).not.toContain('stale from run A')
    expect(transcript.textContent).not.toContain('stale channel, forged id')
    expect(transcript.textContent).toContain('early from run B')
    expect(screen.getByText('running')).toBeTruthy()
  })

  it('chained: an early terminal frame, then the spawn resolves, then a same-channel foreign id is dropped with the badge held, then the run id is applied', async () => {
    const { channels, resolveSpawn } = await renderWithDeferredSpawn()

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
        stream: 'event',
        body: {
          id: 99,
          seq: 1,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'foreign after early end' },
        },
      })
      channels[0]!.onmessage({
        stream: 'state',
        body: { id: 99, seq: 2, droppedBefore: 0, state: 'orphan-risk/uncertain', code: null },
      })
    })
    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).not.toContain('foreign after early end')
    expect(screen.getByText('exited (code 0)')).toBeTruthy()
    expect(document.body.textContent).not.toContain('orphan-risk/uncertain')

    act(() => {
      channels[0]!.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 1,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'trailing and ours' },
        },
      })
    })
    expect(transcript.textContent).toContain('trailing and ours')
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
    expect((screen.getByRole('button', { name: 'Stop' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
  })
})

describe('AgentPanel run id after the run ends (R1-011 / R3-011)', () => {
  /** Delivers, on `channel`, a foreign-id event, a foreign-id state frame, and a trailing event carrying the run's own id. */
  function deliverForeignThenOwn(channel: LiveChannel) {
    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 99,
          seq: 4,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'foreign after end' },
        },
      })
      channel.onmessage({
        stream: 'state',
        body: { id: 99, seq: 5, droppedBefore: 0, state: 'orphan-risk/uncertain', code: null },
      })
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 4,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'trailing and ours' },
        },
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

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).not.toContain('foreign after end')
    expect(transcript.textContent).toContain('trailing and ours')
    expect(screen.getByText('exited (code 0)')).toBeTruthy()
    expect(screen.queryByText('orphan-risk/uncertain')).toBeNull()
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

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).not.toContain('foreign after end')
    expect(transcript.textContent).toContain('trailing and ours')
    expect(screen.getByText('killed')).toBeTruthy()
    expect(screen.queryByText('orphan-risk/uncertain')).toBeNull()
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
  })
})

describe('AgentPanel rejected spawn and the echoed prompt (R1-012 / R3-012)', () => {
  const SPAWN_FAILED = { code: 'spawn-failed', message: 'failed to start the requested process' }

  it('withdraws the echoed prompt when harness_spawn rejects', async () => {
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') return Promise.reject(SPAWN_FAILED)
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByRole('alert')

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).not.toContain('you: do the thing')
    expect(transcript.querySelectorAll('li').length).toBe(0)
  })

  it('a rejection followed by a retry echoes the prompt once and logs no console.error (no duplicate key)', async () => {
    let spawnCount = 0
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') {
        spawnCount += 1
        if (spawnCount === 1) return Promise.reject(SPAWN_FAILED)
        return 7
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByRole('alert')
    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')

    expect(spawnCount).toBe(2)
    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toBe('you: do the thing')
    expect(transcript.querySelectorAll('li').length).toBe(1)
    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('keeps a frame that arrived on the current channel before the rejection, withdrawing only the echoed prompt', async () => {
    let channelRef: LiveChannel | undefined
    let rejectSpawn: (reason: unknown) => void = () => {}
    await renderReady((cmd, args) => {
      if (cmd === 'harness_spawn') {
        channelRef = (args as { onFrame: LiveChannel }).onFrame
        return new Promise<number>((_resolve, reject) => {
          rejectSpawn = reject
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await waitFor(() => {
      expect(channelRef).toBeDefined()
    })
    if (!channelRef) throw new Error('harness_spawn was not called')
    act(() => {
      channelRef!.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 0,
          droppedBefore: 0,
          kind: 'diagnostic',
          payload: { text: 'spawn diagnostics' },
        },
      })
    })
    await act(async () => {
      rejectSpawn(SPAWN_FAILED)
      await new Promise((resolve) => {
        setTimeout(resolve, 0)
      })
    })
    await screen.findByRole('alert')

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toContain('diagnostic: spawn diagnostics')
    expect(transcript.textContent).not.toContain('you: do the thing')
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
  })
})

describe('AgentPanel catalogue codes with fixed copy (R3-013)', () => {
  const STRAY_DETAIL = { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' }

  it('renders unknown-adapter as a plain catalogue code with its message, no "untrusted" and no detail', async () => {
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'unknown-adapter',
          message: 'the requested adapter is not a built-in adapter',
          detail: STRAY_DETAIL,
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('unknown-adapter: the requested adapter is not a built-in adapter')
    expect(banner.textContent).not.toContain('untrusted')
    expect(banner.textContent).not.toContain('aaaaaaaa')
    expect(banner.textContent).not.toContain('bbbbbbbb')
  })

  it('renders prompt-too-large as a plain catalogue code with its message, no "untrusted" and no detail', async () => {
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'prompt-too-large',
          message: 'the prompt exceeds the 16 KiB size limit',
          detail: STRAY_DETAIL,
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('prompt-too-large: the prompt exceeds the 16 KiB size limit')
    expect(banner.textContent).not.toContain('untrusted')
    expect(banner.textContent).not.toContain('aaaaaaaa')
    expect(banner.textContent).not.toContain('bbbbbbbb')
  })
})

const PROMPT_MAX_BYTES = 16 * 1024

describe('AgentPanel prompt byte cap (R3-004)', () => {
  it('keeps Start enabled at exactly 16384 bytes and the spawn payload carries the full prompt', async () => {
    let capturedPrompt: string | undefined
    await renderReady((cmd, args) => {
      if (cmd === 'harness_spawn') {
        capturedPrompt = (args.kind as { prompt: string }).prompt
        return 7
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const prompt = 'a'.repeat(PROMPT_MAX_BYTES)
    expect(new TextEncoder().encode(prompt).length).toBe(16384)
    fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: prompt } })

    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
    expect(screen.queryByTestId('agent-prompt-size-validation')).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')

    expect(capturedPrompt).toBe(prompt)
    expect(new TextEncoder().encode(capturedPrompt!).length).toBe(16384)
  })

  it('disables Start at 16385 bytes and never spawns', async () => {
    let spawnCount = 0
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') {
        spawnCount += 1
        return 7
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    const prompt = 'a'.repeat(PROMPT_MAX_BYTES + 1)
    expect(new TextEncoder().encode(prompt).length).toBe(16385)
    fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: prompt } })

    const startButton = screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement
    expect(startButton.disabled).toBe(true)
    expect(screen.getByTestId('agent-prompt-size-validation').textContent).toContain(
      '16385 bytes',
    )

    fireEvent.click(startButton)
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })
    expect(spawnCount).toBe(0)
  })

  it('measures the cap in UTF-8 bytes, not code units: 6000 three-byte characters disable Start', async () => {
    let spawnCount = 0
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') {
        spawnCount += 1
        return 7
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    // U+20AC (euro sign) is one UTF-16 code unit but three UTF-8 bytes.
    const prompt = '€'.repeat(6000)
    expect(prompt.length).toBe(6000)
    expect(prompt.length).toBeLessThan(PROMPT_MAX_BYTES)
    expect(new TextEncoder().encode(prompt).length).toBe(18000)
    fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: prompt } })

    const startButton = screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement
    expect(startButton.disabled).toBe(true)
    expect(screen.getByTestId('agent-prompt-size-validation').textContent).toContain(
      '18000 bytes',
    )

    fireEvent.click(startButton)
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })
    expect(spawnCount).toBe(0)
  })
})

describe('AgentPanel unexpected fallback (R3-005)', () => {
  const LEAKY_MESSAGE = 'boom: /home/someone/secret/path'

  it('shows only "unexpected error" when harness_spawn rejects with a plain Error, never its message', async () => {
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') return Promise.reject(new Error(LEAKY_MESSAGE))
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('unexpected error')
    expect(document.body.textContent).not.toContain(LEAKY_MESSAGE)
  })

  it('shows only "unexpected error" when harness_spawn rejects with a plain non-ShellError object', async () => {
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') return Promise.reject({ reason: LEAKY_MESSAGE })
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('unexpected error')
    expect(document.body.textContent).not.toContain(LEAKY_MESSAGE)
  })

  it('shows only "unexpected error" when the mount-time adapters_list rejects with a plain Error', async () => {
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return null
      if (cmd === 'outbox_status') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'publications_list') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'adapters_list') return Promise.reject(new Error(LEAKY_MESSAGE))
      if (cmd === 'approvals_list') return [sampleApproval()]
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('unexpected error')
    expect(document.body.textContent).not.toContain(LEAKY_MESSAGE)
  })
})

describe('AgentPanel failure paths (R3-006)', () => {
  it('workspace_current rejecting with a ShellError shows the banner and leaves the panel usable', async () => {
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') {
        return Promise.reject({ code: 'invalid-request', message: 'bad request' })
      }
      if (cmd === 'outbox_status') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'publications_list') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'adapters_list') return [SAMPLE_ADAPTER]
      if (cmd === 'approvals_list') return [sampleApproval({ approvalId: 42 })]
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('invalid-request: bad request')

    await selectOption('Adapter', 'claude-code')
    await selectOption('Approval', '42')
    fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: 'still usable' } })
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
    expect(
      (screen.getByRole('button', { name: 'Pick workspace' }) as HTMLButtonElement).disabled,
    ).toBe(false)
  })

  it('adapters_list rejecting with a ShellError shows the banner and leaves the panel usable', async () => {
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return null
      if (cmd === 'outbox_status') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'publications_list') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'adapters_list') {
        return Promise.reject({ code: 'invalid-request', message: 'adapter catalog unavailable' })
      }
      if (cmd === 'approvals_list') return [sampleApproval({ approvalId: 42 })]
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('invalid-request: adapter catalog unavailable')

    const approvalSelect = (await screen.findByLabelText('Approval')) as HTMLSelectElement
    await waitFor(() => {
      expect(Array.from(approvalSelect.options).map((option) => option.value)).toContain('42')
    })
    expect(
      (screen.getByRole('button', { name: 'Pick workspace' }) as HTMLButtonElement).disabled,
    ).toBe(false)
  })

  it('approvals_list rejecting with approval-store-unavailable shows the banner and leaves the panel usable', async () => {
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return null
      if (cmd === 'outbox_status') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'publications_list') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'adapters_list') return [SAMPLE_ADAPTER]
      if (cmd === 'approvals_list') {
        return Promise.reject({
          code: 'approval-store-unavailable',
          message: 'the approval store could not be opened',
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe(
      'approval-store-unavailable: the approval store could not be opened',
    )

    const adapterSelect = (await screen.findByLabelText('Adapter')) as HTMLSelectElement
    await waitFor(() => {
      expect(Array.from(adapterSelect.options).map((option) => option.value)).toContain(
        'claude-code',
      )
    })
    expect(
      (screen.getByRole('button', { name: 'Pick workspace' }) as HTMLButtonElement).disabled,
    ).toBe(false)
  })

  it('harness_stop rejecting keeps the run active: banner shown, Stop still enabled, Start still disabled, frames still applied', async () => {
    const channel = await startAndCaptureChannel((cmd) => {
      if (cmd === 'harness_stop') {
        return Promise.reject({ code: 'invalid-request', message: 'stop was rejected' })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('invalid-request: stop was rejected')
    expect(screen.getByText('running')).toBeTruthy()
    expect((screen.getByRole('button', { name: 'Stop' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      true,
    )

    // The run's own frames keep applying: the active id was not cleared.
    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 1,
          droppedBefore: 0,
          kind: 'message',
          payload: { text: 'still streaming' },
        },
      })
    })
    expect(screen.getByLabelText('Agent transcript').textContent).toContain('still streaming')
  })
})

describe('AgentPanel control stripping per harness string (R3-008)', () => {
  const esc = String.fromCharCode(0x1b)

  it('strips control bytes from a state observation key, value, and subtype, and keeps a <b> tag literal', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 0,
          droppedBefore: 0,
          kind: 'state',
          payload: {
            phase: 'finished',
            subtype: `succ${esc}ess`,
            observations: [{ key: `cw${esc}d`, value: `<b>/wo${esc}rk</b>` }],
          },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.querySelector('b')).toBeNull()
    expect(transcript.textContent).toContain('finished, success, cwd: <b>/work</b>')
    expect(transcript.textContent).not.toContain(esc)
  })

  it('strips control bytes from a diagnostic text and keeps a <b> tag literal', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 0,
          droppedBefore: 0,
          kind: 'diagnostic',
          payload: { text: `warn${esc}ing <b>x</b>` },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.querySelector('b')).toBeNull()
    expect(transcript.textContent).toContain('diagnostic: warning <b>x</b>')
    expect(transcript.textContent).not.toContain(esc)
  })

  it('strips control bytes from a tool-call name and argumentsText and keeps a <b> tag literal', async () => {
    const channel = await startAndCaptureChannel()

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 0,
          droppedBefore: 0,
          kind: 'tool-call',
          payload: {
            name: `write${esc}_file<b>`,
            argumentsText: `{"path":"<b>no${esc}tes.md</b>"}`,
          },
        },
      })
    })

    const proposal = screen.getByTestId('tool-call-proposal')
    expect(proposal.querySelector('b')).toBeNull()
    expect(proposal.textContent).toContain('write_file<b>')
    expect(proposal.textContent).toContain('{"path":"<b>notes.md</b>"}')
    expect(proposal.textContent).not.toContain(esc)
  })

  it('strips control bytes from an adapter displayName and notes and keeps a <b> tag literal', async () => {
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return null
      if (cmd === 'outbox_status') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'publications_list') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'adapters_list') {
        return [
          {
            ...SAMPLE_ADAPTER,
            displayName: `Claude${esc} <b>Code</b>`,
            notes: `harness${esc}-owned <b>notes</b>`,
          },
        ]
      }
      if (cmd === 'approvals_list') return [sampleApproval()]
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)

    const adapterSelect = (await screen.findByLabelText('Adapter')) as HTMLSelectElement
    await waitFor(() => {
      expect(Array.from(adapterSelect.options).map((option) => option.value)).toContain(
        'claude-code',
      )
    })
    const option = Array.from(adapterSelect.options).find(
      (candidate) => candidate.value === 'claude-code',
    )
    expect(option?.textContent).toBe('Claude <b>Code</b>')
    expect(adapterSelect.querySelector('b')).toBeNull()

    fireEvent.change(adapterSelect, { target: { value: 'claude-code' } })
    const notes = await screen.findByText('harness-owned <b>notes</b>')
    expect(notes.querySelector('b')).toBeNull()
    expect(document.body.textContent).not.toContain(esc)
  })

  it('strips control bytes from the approval option label and keeps a <b> tag literal', async () => {
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return null
      if (cmd === 'outbox_status') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'publications_list') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'adapters_list') return [SAMPLE_ADAPTER]
      if (cmd === 'approvals_list') {
        return [
          sampleApproval({
            approvalId: 42,
            evidence: { ...SAMPLE_EVIDENCE, canonicalPath: `/opt/tool${esc}/<b>app</b>` },
          }),
        ]
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)

    const approvalSelect = (await screen.findByLabelText('Approval')) as HTMLSelectElement
    await waitFor(() => {
      expect(Array.from(approvalSelect.options).map((option) => option.value)).toContain('42')
    })
    const option = Array.from(approvalSelect.options).find((candidate) => candidate.value === '42')
    expect(option?.textContent).toBe('/opt/tool/<b>app</b>')
    expect(approvalSelect.querySelector('b')).toBeNull()
    expect(document.body.textContent).not.toContain(esc)
  })
})

describe('AgentPanel double-click before spawn resolves (R3-010)', () => {
  it('disables Start while a spawn is pending, and a second click does not spawn twice', async () => {
    let spawnCount = 0
    let resolveSpawn: (id: number) => void = () => {}
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') {
        spawnCount += 1
        return new Promise<number>((resolve) => {
          resolveSpawn = resolve
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
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

    // One active run per panel: Start stays disabled once running, not
    // just while the spawn itself is pending.
    expect(startButton.disabled).toBe(true)
  })
})

// -- Slice 4: the pty-cli adapter's transport disclosure and the three
// terminal event kinds (`docs/spike-log.md` § Slice 4) --

const PTY_DISCLOSURE = 'transport: pty — degraded fallback; plain text, layout controls dropped'

/** Leaf elements anywhere in the document whose text contains `needle`. */
function leavesContaining(needle: string): Element[] {
  return Array.from(document.body.querySelectorAll('*')).filter(
    (el) => el.children.length === 0 && (el.textContent ?? '').includes(needle),
  )
}

/** Delivers one `terminal-text` event carrying `text` on `channel`, as run 7's frame `seq`. */
function deliverTerminalText(
  channel: { onmessage: (frame: HarnessFrame) => void },
  text: string,
  seq = 0,
) {
  act(() => {
    channel.onmessage({
      stream: 'event',
      body: { id: 7, seq, droppedBefore: 0, kind: 'terminal-text', payload: { text } },
    })
  })
}

/** The rendered lines of the single `terminal-text` entry in the transcript, as text. */
function terminalLines(): string[] {
  const transcript = screen.getByLabelText('Agent transcript')
  const entries = transcript.querySelectorAll('[data-testid="terminal-text"]')
  expect(entries.length).toBe(1)
  return Array.from(entries[0]!.querySelectorAll('[data-testid="terminal-line"]')).map(
    (line) => line.textContent ?? '',
  )
}

describe('AgentPanel pty transport disclosure (slice 4)', () => {
  it('shows the fixed transport disclosure line once the selected adapter\'s transportClass is pty, and "pty" appears nowhere else in the panel', async () => {
    await renderReady(undefined, 'pty-cli')

    const disclosure = screen.getByTestId('agent-transport-disclosure')
    expect(disclosure.textContent).toBe(PTY_DISCLOSURE)
    expect(leavesContaining('pty')).toEqual([disclosure])
  })

  it('never shows the transport disclosure for a non-pty adapter, and removes it when the selection moves off pty-cli', async () => {
    await renderReady()
    expect(screen.queryByTestId('agent-transport-disclosure')).toBeNull()
    expect(document.body.textContent).not.toContain('transport: pty')

    await selectOption('Adapter', 'pty-cli')
    expect(screen.getByTestId('agent-transport-disclosure').textContent).toBe(PTY_DISCLOSURE)

    await selectOption('Adapter', 'claude-code')
    expect(screen.queryByTestId('agent-transport-disclosure')).toBeNull()
    expect(document.body.textContent).not.toContain('transport: pty')
  })
})

describe('AgentPanel terminal-text (slice 4)', () => {
  it('renders one transcript entry per terminal-text event, one line per newline-separated segment, a trailing newline adding no empty line', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    deliverTerminalText(channel, 'first\nsecond\n')

    // Two separate line elements, each holding exactly one segment: the
    // newline was split before PlainTextLine, never stripped inside it (a
    // single span would read "firstsecond"). Asserted per element rather
    // than on `textContent`, which concatenates adjacent blocks unseparated.
    expect(terminalLines()).toEqual(['first', 'second'])
    const transcript = screen.getByLabelText('Agent transcript')
    // The echoed prompt row plus exactly one row for the event.
    expect(transcript.querySelectorAll('li').length).toBe(2)
  })

  it('renders a terminal-text with no newline (a continued chunk) as exactly one line', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    deliverTerminalText(channel, 'partial')

    expect(terminalLines()).toEqual(['partial'])
  })

  it('keeps an interior empty line: "a\\n\\nb\\n" renders three lines with the middle one empty', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    deliverTerminalText(channel, 'a\n\nb\n')

    expect(terminalLines()).toEqual(['a', '', 'b'])
  })

  it('labels the entry visually as terminal output, plain text', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    deliverTerminalText(channel, 'hello\n')

    const entry = screen.getByLabelText('Agent transcript').querySelector(
      '[data-testid="terminal-text"]',
    )
    expect(entry?.textContent).toContain('terminal output, plain text')
  })

  it('renders two terminal-text events as two separate entries, in order', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    deliverTerminalText(channel, 'one\n', 0)
    deliverTerminalText(channel, 'two\n', 1)

    const transcript = screen.getByLabelText('Agent transcript')
    const entries = transcript.querySelectorAll('[data-testid="terminal-text"]')
    expect(entries.length).toBe(2)
    expect(entries[0]?.textContent).toContain('one')
    expect(entries[1]?.textContent).toContain('two')
    expect(transcript.textContent?.indexOf('one')).toBeLessThan(
      transcript.textContent?.indexOf('two') ?? -1,
    )
  })
})

describe('AgentPanel terminal-action (slice 4)', () => {
  // The title test sets `document.title` as its oracle; restore whatever
  // was there so no later test inherits it (slice 4 review, R3-007).
  let previousTitle = ''
  beforeEach(() => {
    previousTitle = document.title
  })
  afterEach(() => {
    document.title = previousTitle
  })

  it('renders a title action as a "title:" transcript line and never touches document.title, the header, or anything outside the transcript', async () => {
    document.title = 'Omnifrons test'
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 9,
          droppedBefore: 0,
          kind: 'terminal-action',
          payload: { action: 'title', text: 'build ok' },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toContain('title: build ok')
    expect(document.title).toBe('Omnifrons test')
    expect(document.head.textContent).not.toContain('build ok')
    const outside = leavesContaining('build ok').filter((el) => !transcript.contains(el))
    expect(outside).toEqual([])
    expect(screen.getByRole('heading', { name: 'Agent' }).textContent).toBe('Agent')
  })

  it('renders a notification action as a "notification:" transcript line, calling no Notification API, no IPC, and moving no focus', async () => {
    const notificationCtor = vi.fn()
    const requestPermission = vi.fn()
    Object.assign(notificationCtor, { requestPermission })
    vi.stubGlobal('Notification', notificationCtor)
    try {
      let invokeCount = 0
      let channelRef: { onmessage: (frame: HarnessFrame) => void } | undefined
      const handlers = defaultHandlers((cmd, args) => {
        if (cmd === 'harness_spawn') {
          channelRef = (args as { onFrame: { onmessage: (frame: HarnessFrame) => void } }).onFrame
          return 7
        }
        throw new Error(`unexpected command: ${cmd}`)
      })
      mockIPC((cmd, args) => {
        invokeCount += 1
        return handlers(cmd, args)
      })

      render(<AgentPanel />)
      await selectOption('Adapter', 'pty-cli')
      await selectOption('Approval', '42')
      fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: 'do the thing' } })
      fireEvent.click(screen.getByRole('button', { name: 'Start' }))
      await waitFor(() => {
        expect(channelRef).toBeDefined()
      })
      await screen.findByText('running')
      if (!channelRef) throw new Error('harness_spawn was not called')

      const focusedBefore = document.activeElement
      const invokesBefore = invokeCount
      act(() => {
        channelRef!.onmessage({
          stream: 'event',
          body: {
            id: 7,
            seq: 10,
            droppedBefore: 2,
            kind: 'terminal-action',
            payload: { action: 'notification', text: 'done' },
          },
        })
      })
      await new Promise((resolve) => {
        setTimeout(resolve, 0)
      })

      const transcript = screen.getByLabelText('Agent transcript')
      expect(transcript.textContent).toContain('notification: done')
      expect(
        screen.getByText('2 frames dropped before this entry — degraded, restart to replay'),
      ).toBeTruthy()
      expect(notificationCtor).not.toHaveBeenCalled()
      expect(requestPermission).not.toHaveBeenCalled()
      expect(invokeCount).toBe(invokesBefore)
      expect(document.activeElement).toBe(focusedBefore)
    } finally {
      vi.unstubAllGlobals()
    }
  })
})

describe('AgentPanel terminal-drops (slice 4)', () => {
  it('renders a degraded marker line with all seven counts in the fixed order', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
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
      })
    })

    const marker = screen.getByText(
      'dropped terminal controls: layout 21, hyperlink 2, clipboard 2, file transfer 3, string 3, unknown 1, malformed 3',
    )
    expect(marker.tagName).toBe('MARK')
    expect(screen.getByLabelText('Agent transcript').contains(marker)).toBe(true)
  })

  it('still renders every zero count as 0', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 11,
          droppedBefore: 0,
          kind: 'terminal-drops',
          payload: {
            layout: 0,
            hyperlink: 0,
            clipboard: 0,
            fileTransfer: 0,
            string: 0,
            unknown: 0,
            malformed: 0,
          },
        },
      })
    })

    expect(
      screen.getByText(
        'dropped terminal controls: layout 0, hyperlink 0, clipboard 0, file transfer 0, string 0, unknown 0, malformed 0',
      ),
    ).toBeTruthy()
  })
})

describe('AgentPanel pty-unsupported banner (slice 4, R3-013 analogue)', () => {
  it('renders pty-unsupported as a plain catalogue code with its fixed message, no "untrusted" and no stray detail', async () => {
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'pty-unsupported',
          message: 'pseudo-terminal launches are not available on this platform',
          detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    }, 'pty-cli')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe(
      'pty-unsupported: pseudo-terminal launches are not available on this platform',
    )
    expect(banner.textContent).not.toContain('untrusted')
    expect(banner.textContent).not.toContain('aaaaaaaa')
    expect(banner.textContent).not.toContain('bbbbbbbb')
  })

  it('renders prompt-not-typeable as a plain catalogue code with its fixed message, no "untrusted" and no stray detail', async () => {
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'prompt-not-typeable',
          message: 'prompt contains control characters a terminal would interpret',
          detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    }, 'pty-cli')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe(
      'prompt-not-typeable: prompt contains control characters a terminal would interpret',
    )
    expect(banner.textContent).not.toContain('untrusted')
    expect(banner.textContent).not.toContain('aaaaaaaa')
    expect(banner.textContent).not.toContain('bbbbbbbb')
  })
})

describe('AgentPanel content security on the pty path (slice 4, RCS-001)', () => {
  const esc = String.fromCharCode(0x1b)
  const bel = String.fromCharCode(0x07)
  /** U+202E RIGHT-TO-LEFT OVERRIDE, built from its code point so no bidi control sits in this source file. */
  const rlo = String.fromCodePoint(0x202e)

  it('renders a terminal-text carrying a literal OSC 8 hyperlink residue as text with no anchor and no ESC/BEL', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    deliverTerminalText(channel, `${esc}]8;;https://example.invalid${bel}label\n`)

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.querySelector('a')).toBeNull()
    expect(transcript.querySelector('[href]')).toBeNull()
    expect(transcript.textContent).toContain(']8;;https://example.invalidlabel')
    expect(transcript.textContent).not.toContain(esc)
    expect(transcript.textContent).not.toContain(bel)
  })

  it('renders a terminal-text containing a <b> tag literally, producing no <b> element', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    deliverTerminalText(channel, '<b>bold</b>\n')

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.querySelector('b')).toBeNull()
    expect(terminalLines()).toEqual(['<b>bold</b>'])
  })

  it('renders a title text carrying a C0 byte and a bidi override stripped of both', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 9,
          droppedBefore: 0,
          kind: 'terminal-action',
          payload: { action: 'title', text: `Ti${esc}tle ${rlo}rev` },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toContain('title: Title rev')
    expect(transcript.textContent).not.toContain(esc)
    expect(transcript.textContent).not.toContain(rlo)
  })

  it('renders a terminal-text containing SENTINEL-CLIPBOARD as text: the renderer never filters content, core\'s normalizer is what keeps a real OSC 52 payload off the wire', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    deliverTerminalText(channel, 'SENTINEL-CLIPBOARD\n')

    expect(terminalLines()).toEqual(['SENTINEL-CLIPBOARD'])
  })
})

// -- Slice 4 review (reliability lens) --

describe('AgentPanel bare CR on the pty path (slice 4 review, R3-001; recorded debt)', () => {
  const cr = String.fromCharCode(0x0d)

  it('debt: a bare CR is a C0 control PlainTextLine strips, so "a\\rb\\n" renders as one line "ab" -- a CR-updated progress bar renders concatenated', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    deliverTerminalText(channel, `a${cr}b\n`)

    expect(terminalLines()).toEqual(['ab'])
    expect(screen.getByLabelText('Agent transcript').textContent).not.toContain(cr)
  })

  it('a CRLF line end "a\\r\\nb\\n" renders as two lines "a" and "b" with no CR residue', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    deliverTerminalText(channel, `a${cr}\nb\n`)

    expect(terminalLines()).toEqual(['a', 'b'])
    expect(screen.getByLabelText('Agent transcript').textContent).not.toContain(cr)
  })
})

describe('AgentPanel terminal-drops count magnitude (slice 4 review, R3-002)', () => {
  /** Delivers one `terminal-drops` event whose counts are `counts`, on run 7's channel. */
  function deliverDrops(
    channel: LiveChannel,
    counts: {
      layout: number
      hyperlink: number
      clipboard: number
      fileTransfer: number
      string: number
      unknown: number
      malformed: number
    },
  ) {
    act(() => {
      channel.onmessage({
        stream: 'event',
        body: { id: 7, seq: 11, droppedBefore: 0, kind: 'terminal-drops', payload: counts },
      })
    })
  }

  it('renders a count of Number.MAX_SAFE_INTEGER as plain digits, never an exponent', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    deliverDrops(channel, {
      layout: Number.MAX_SAFE_INTEGER,
      hyperlink: 0,
      clipboard: 0,
      fileTransfer: 0,
      string: 0,
      unknown: 0,
      malformed: 0,
    })

    const marker = screen.getByText(/^dropped terminal controls: /)
    expect(marker.textContent).toContain('layout 9007199254740991,')
    expect(marker.textContent).not.toContain('e+')
  })

  it('renders a u64-max count (18446744073709551615 on the wire, 2^64 after JSON.parse) as plain digits: precision above 2^53 is lost in the renderer and accepted', async () => {
    // The literal never appears in source (eslint's no-loss-of-precision
    // would flag it); it reaches the renderer the way the wire does, by
    // parsing JSON, and lands on the nearest double, exactly 2^64.
    const wire = JSON.parse('{"hyperlink":18446744073709551615}') as { hyperlink: number }
    expect(wire.hyperlink).toBe(2 ** 64)
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    deliverDrops(channel, {
      layout: 0,
      hyperlink: wire.hyperlink,
      clipboard: 0,
      fileTransfer: 0,
      string: 0,
      unknown: 0,
      malformed: 0,
    })

    const marker = screen.getByText(/^dropped terminal controls: /)
    expect(marker.textContent).toContain('hyperlink 18446744073709552000,')
    expect(marker.textContent).not.toContain('e+')
  })
})

describe('AgentPanel terminal events under the run guards (slice 4 review, R3-004)', () => {
  it('drops a terminal-text frame from a previous run\'s channel once a second run has started, while the new run\'s own terminal-text is applied', async () => {
    const { channels } = await renderMultiRun(undefined, 'pty-cli')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await screen.findByText('killed')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    expect(channels).toHaveLength(2)

    act(() => {
      channels[0]!.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 5,
          droppedBefore: 0,
          kind: 'terminal-text',
          payload: { text: 'late from run A\n' },
        },
      })
      channels[1]!.onmessage({
        stream: 'event',
        body: {
          id: 8,
          seq: 0,
          droppedBefore: 0,
          kind: 'terminal-text',
          payload: { text: 'fresh from run B\n' },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).not.toContain('late from run A')
    expect(transcript.textContent).toContain('fresh from run B')
    expect(transcript.querySelectorAll('[data-testid="terminal-text"]').length).toBe(1)
  })

  it('drops terminal-text and terminal-drops frames carrying a foreign run id: nothing appended, badge unchanged', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 99,
          seq: 0,
          droppedBefore: 0,
          kind: 'terminal-text',
          payload: { text: 'foreign text\n' },
        },
      })
      channel.onmessage({
        stream: 'event',
        body: {
          id: 99,
          seq: 1,
          droppedBefore: 0,
          kind: 'terminal-drops',
          payload: {
            layout: 5,
            hyperlink: 0,
            clipboard: 0,
            fileTransfer: 0,
            string: 0,
            unknown: 0,
            malformed: 0,
          },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).not.toContain('foreign text')
    expect(screen.queryByText(/^dropped terminal controls: /)).toBeNull()
    // The echoed prompt row only.
    expect(transcript.querySelectorAll('li').length).toBe(1)
    expect(screen.getByText('running')).toBeTruthy()
  })

  it('a terminal state frame after terminal-text and terminal-drops events ends the run, with the state line last in the transcript', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    deliverTerminalText(channel, 'out\n', 0)
    act(() => {
      channel.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 1,
          droppedBefore: 0,
          kind: 'terminal-drops',
          payload: {
            layout: 1,
            hyperlink: 0,
            clipboard: 0,
            fileTransfer: 0,
            string: 0,
            unknown: 0,
            malformed: 0,
          },
        },
      })
      channel.onmessage({
        stream: 'state',
        body: { id: 7, seq: 2, droppedBefore: 0, state: 'exited', code: 0 },
      })
    })

    await screen.findByText('exited (code 0)')
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
    expect((screen.getByRole('button', { name: 'Stop' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    const transcript = screen.getByLabelText('Agent transcript')
    const rows = Array.from(transcript.querySelectorAll('li'))
    // prompt, terminal-text, terminal-drops, state -- in delivery order.
    expect(rows.length).toBe(4)
    expect(rows[1]?.textContent).toContain('out')
    expect(rows[2]?.textContent).toContain('dropped terminal controls: layout 1,')
    expect(rows[3]?.textContent).toBe('state: exited (code 0)')
  })
})

/** The three selection controls a run must freeze: workspace pick, adapter, approval. */
function selectionControls() {
  return {
    pick: screen.getByRole('button', { name: 'Pick workspace' }) as HTMLButtonElement,
    adapter: screen.getByLabelText('Adapter') as HTMLSelectElement,
    approval: screen.getByLabelText('Approval') as HTMLSelectElement,
  }
}

function expectSelectionControlsDisabled(disabled: boolean) {
  const { pick, adapter, approval } = selectionControls()
  expect([pick.disabled, adapter.disabled, approval.disabled]).toEqual([
    disabled,
    disabled,
    disabled,
  ])
}

describe('AgentPanel selection controls frozen during a run (slice 4 review, R3-005)', () => {
  it('disables the workspace, adapter and approval controls while a run is active and re-enables them once the terminal state frame ends it', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')
    expectSelectionControlsDisabled(true)

    act(() => {
      channel.onmessage({
        stream: 'state',
        body: { id: 7, seq: 1, droppedBefore: 0, state: 'exited', code: 0 },
      })
    })
    await screen.findByText('exited (code 0)')

    expectSelectionControlsDisabled(false)
  })

  it('disables the selection controls from the Start click on, while the spawn is still pending, and keeps them disabled once running', async () => {
    const { channels, resolveSpawn } = await renderWithDeferredSpawn()
    expectSelectionControlsDisabled(false)

    await clickStartAndAwaitChannel(channels, 1)
    expectSelectionControlsDisabled(true)

    await settleSpawn(resolveSpawn, 7)
    await screen.findByText('running')
    expectSelectionControlsDisabled(true)
  })

  it('re-enables the selection controls after a successful Stop', async () => {
    const channel = await startAndCaptureChannel((cmd) => {
      if (cmd === 'harness_stop') return { state: 'killed', code: null }
      throw new Error(`unexpected command: ${cmd}`)
    }, 'pty-cli')
    void channel
    expectSelectionControlsDisabled(true)

    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await screen.findByText('killed')

    expectSelectionControlsDisabled(false)
  })

  it('keeps the selection controls disabled after a rejected Stop, since the run is still active', async () => {
    const channel = await startAndCaptureChannel((cmd) => {
      if (cmd === 'harness_stop') {
        return Promise.reject({ code: 'invalid-request', message: 'stop was rejected' })
      }
      throw new Error(`unexpected command: ${cmd}`)
    }, 'pty-cli')
    void channel

    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await screen.findByRole('alert')

    expect(screen.getByText('running')).toBeTruthy()
    expectSelectionControlsDisabled(true)
  })

  it('re-enables the selection controls after a rejected spawn, since no run started', async () => {
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') {
        return Promise.reject({ code: 'spawn-failed', message: 'failed to start the requested process' })
      }
      throw new Error(`unexpected command: ${cmd}`)
    }, 'pty-cli')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByRole('alert')

    expectSelectionControlsDisabled(false)
  })

  it('keeps the pty transport disclosure through a run started with pty-cli: a change fired at the disabled adapter select is ignored while output streams, and takes effect only after the run ends', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')
    deliverTerminalText(channel, 'streaming\n', 0)
    expect(screen.getByTestId('agent-transport-disclosure').textContent).toBe(PTY_DISCLOSURE)

    const adapterSelect = screen.getByLabelText('Adapter') as HTMLSelectElement
    fireEvent.change(adapterSelect, { target: { value: 'claude-code' } })

    expect(adapterSelect.value).toBe('pty-cli')
    expect(screen.getByTestId('agent-transport-disclosure').textContent).toBe(PTY_DISCLOSURE)
    deliverTerminalText(channel, 'still streaming\n', 1)
    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.querySelectorAll('[data-testid="terminal-text"]').length).toBe(2)
    expect(screen.getByTestId('agent-transport-disclosure').textContent).toBe(PTY_DISCLOSURE)

    act(() => {
      channel.onmessage({
        stream: 'state',
        body: { id: 7, seq: 2, droppedBefore: 0, state: 'exited', code: 0 },
      })
    })
    await screen.findByText('exited (code 0)')

    await selectOption('Adapter', 'claude-code')
    expect(screen.queryByTestId('agent-transport-disclosure')).toBeNull()
  })
})

describe('AgentPanel empty terminal-text (slice 4 review, R3-006)', () => {
  it('renders no transcript entry for an empty terminal-text payload with nothing dropped before it', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    deliverTerminalText(channel, '')

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.querySelectorAll('[data-testid="terminal-text"]').length).toBe(0)
    // The echoed prompt row only.
    expect(transcript.querySelectorAll('li').length).toBe(1)
  })

  it('keeps the degraded marker of an empty terminal-text carrying a nonzero droppedBefore, rendering no line for it', async () => {
    const channel = await startAndCaptureChannel(undefined, 'pty-cli')

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: { id: 7, seq: 0, droppedBefore: 3, kind: 'terminal-text', payload: { text: '' } },
      })
    })

    expect(
      screen.getByText('3 frames dropped before this entry — degraded, restart to replay'),
    ).toBeTruthy()
    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.querySelectorAll('[data-testid="terminal-line"]').length).toBe(0)
  })
})

// -- Slice 5: the outbox status line, the artifact-publish proposal, the
// candidates summary and table, and the two outbox error codes
// (`docs/spike-log.md` § Slice 5) --

const OUTBOX_LINE_VALID =
  'outbox: /home/user/project/.omnifrons/outbox (declared .omnifrons/outbox)'

/**
 * Mounts the panel's IPC with an active workspace whose `outbox_status`
 * answers whatever `answer` returns (a status, or a rejected promise --
 * built lazily inside the handler, so no rejection exists before it is
 * handled).
 */
function mockMountWithOutbox(answer: () => unknown) {
  mockIPC((cmd) => {
    if (cmd === 'workspace_current') return { displayPath: '/home/user/project', workArea: 'valid' }
    if (cmd === 'adapters_list') return [SAMPLE_ADAPTER, PTY_ADAPTER]
    if (cmd === 'approvals_list') return [sampleApproval({ approvalId: 42 })]
    if (cmd === 'outbox_status') return answer()
    if (cmd === 'publications_list') return []
    throw new Error(`unexpected command: ${cmd}`)
  })
}

describe('AgentPanel outbox status line (slice 5)', () => {
  it('fetches outbox_status on mount and shows "outbox: <path> (declared <declared>)" for a valid, existing outbox', async () => {
    mockMountWithOutbox(() => OUTBOX_STATUS_VALID)

    render(<AgentPanel />)

    const line = await screen.findByRole('status', { name: 'Outbox' })
    expect(line.textContent).toBe(OUTBOX_LINE_VALID)
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('shows "outbox: <declared> (created at the first launch)" when the declaration is valid but the directory does not exist yet, with no device path anywhere', async () => {
    mockMountWithOutbox(() => ({ ...OUTBOX_STATUS_VALID, outbox: null, exists: false }))

    render(<AgentPanel />)

    const line = await screen.findByRole('status', { name: 'Outbox' })
    expect(line.textContent).toBe('outbox: .omnifrons/outbox (created at the first launch)')
    expect(document.body.textContent).not.toContain('/home/user/project/.omnifrons')
  })

  it('shows "outbox invalid: <declared> (<reason>)" when the policy declared a path, and "outbox invalid: <reason>" when the policy itself could not be loaded, each reason token mapped to fixed English and never rendered raw (R3-017)', async () => {
    const cases: { reason: OutboxReason; declared: string | null; expected: string }[] = [
      {
        reason: 'outside-project',
        declared: 'elsewhere/outbox',
        expected: 'outbox invalid: elsewhere/outbox (the declared path resolves outside the project)',
      },
      // The declared name is project-originated text: a <b> in it stays literal.
      {
        reason: 'link',
        declared: 'else<b>where</b>/outbox',
        expected: 'outbox invalid: else<b>where</b>/outbox (the declared path is a link)',
      },
      {
        reason: 'policy-unreadable',
        declared: null,
        expected: 'outbox invalid: the classification policy could not be read',
      },
      {
        reason: 'policy-corrupt',
        declared: null,
        expected: 'outbox invalid: the classification policy is corrupt',
      },
      {
        reason: 'policy-invalid',
        declared: null,
        expected: 'outbox invalid: the classification policy is invalid',
      },
    ]
    for (const { reason, declared, expected } of cases) {
      mockMountWithOutbox(() => ({
        declared,
        outbox: null,
        exists: declared !== null,
        state: 'outbox-invalid',
        reason,
        policyPath: '.omnifrons/asset-policy.json',
        assetRootId: declared === null ? null : 'main',
      }))
      const { unmount } = render(<AgentPanel />)

      const line = await screen.findByRole('status', { name: 'Outbox' })
      expect(line.textContent).toBe(expected)
      expect(line.querySelector('b')).toBeNull()
      // `link` is the one token that is also a word of its own English line.
      if (reason !== 'link') expect(document.body.textContent).not.toContain(reason)
      if (declared === null) expect(document.body.textContent).not.toContain('elsewhere')

      unmount()
      clearMocks()
    }
  })

  it('shows "outbox unavailable: <declared> (<reason>)" for the outbox-unavailable state the DTO carries (not-a-directory, unreadable), never the raw token', async () => {
    const cases: [OutboxReason, string][] = [
      [
        'not-a-directory',
        'outbox unavailable: .omnifrons/outbox (the declared path is not a directory)',
      ],
      ['unreadable', 'outbox unavailable: .omnifrons/outbox (the declared path could not be read)'],
    ]
    for (const [reason, expected] of cases) {
      mockMountWithOutbox(() => ({
        declared: '.omnifrons/outbox',
        outbox: null,
        exists: true,
        state: 'outbox-unavailable',
        reason,
        policyPath: '.omnifrons/asset-policy.json',
        assetRootId: 'main',
      }))
      const { unmount } = render(<AgentPanel />)

      const line = await screen.findByRole('status', { name: 'Outbox' })
      expect(line.textContent).toBe(expected)
      expect(document.body.textContent).not.toContain(reason)

      unmount()
      clearMocks()
    }
  })

  it('shows "outbox invalid: reason unreported" when a non-valid state carries a null reason', async () => {
    mockMountWithOutbox(() => ({
      declared: null,
      outbox: null,
      exists: false,
      state: 'outbox-invalid',
      reason: null,
      policyPath: '.omnifrons/asset-policy.json',
      assetRootId: null,
    }))

    render(<AgentPanel />)

    const line = await screen.findByRole('status', { name: 'Outbox' })
    expect(line.textContent).toBe('outbox invalid: reason unreported')
  })

  it('calls outbox_status on mount and renders no status line and no alert when it rejects workspace-unavailable (no workspace picked)', async () => {
    let statusCalls = 0
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return null
      if (cmd === 'adapters_list') return [SAMPLE_ADAPTER]
      if (cmd === 'approvals_list') return [sampleApproval()]
      if (cmd === 'outbox_status') {
        statusCalls += 1
        return Promise.reject(OUTBOX_NO_WORKSPACE)
      }
      if (cmd === 'publications_list') return Promise.reject(OUTBOX_NO_WORKSPACE)
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)

    await waitFor(() => {
      expect(statusCalls).toBe(1)
    })
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })
    expect(screen.queryByRole('status', { name: 'Outbox' })).toBeNull()
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('fetches outbox_status again after a successful workspace pick: no line before the pick, the valid line after it', async () => {
    let picked = false
    let statusCalls = 0
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return null
      if (cmd === 'adapters_list') return [SAMPLE_ADAPTER]
      if (cmd === 'approvals_list') return [sampleApproval()]
      if (cmd === 'outbox_status') {
        statusCalls += 1
        return picked ? OUTBOX_STATUS_VALID : Promise.reject(OUTBOX_NO_WORKSPACE)
      }
      if (cmd === 'publications_list') return picked ? [] : Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'workspace_pick') {
        picked = true
        return { displayPath: '/home/user/project', workArea: 'valid' }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)
    await waitFor(() => {
      expect(statusCalls).toBe(1)
    })
    expect(screen.queryByRole('status', { name: 'Outbox' })).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))

    await screen.findByText('/home/user/project')
    const line = await screen.findByRole('status', { name: 'Outbox' })
    expect(line.textContent).toBe(OUTBOX_LINE_VALID)
    expect(statusCalls).toBe(2)
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('drops a stale mount-time outbox_status response that resolves after the pick-time response (sequenced fetch)', async () => {
    let resolveMountStatus: (status: OutboxStatus) => void = () => {}
    let statusCalls = 0
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return null
      if (cmd === 'adapters_list') return [SAMPLE_ADAPTER]
      if (cmd === 'approvals_list') return [sampleApproval()]
      if (cmd === 'outbox_status') {
        statusCalls += 1
        if (statusCalls === 1) {
          return new Promise<OutboxStatus>((resolve) => {
            resolveMountStatus = resolve
          })
        }
        return OUTBOX_STATUS_VALID
      }
      if (cmd === 'publications_list') return []
      if (cmd === 'workspace_pick') return { displayPath: '/home/user/project', workArea: 'valid' }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)
    await waitFor(() => {
      expect(statusCalls).toBe(1)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    const line = await screen.findByRole('status', { name: 'Outbox' })
    expect(line.textContent).toBe(OUTBOX_LINE_VALID)

    // The mount-time response arrives last, carrying a different status:
    // it is stale and must not overwrite the pick-time line.
    await act(async () => {
      resolveMountStatus({ ...OUTBOX_STATUS_VALID, outbox: null, exists: false })
      await new Promise((resolve) => {
        setTimeout(resolve, 0)
      })
    })
    expect(screen.getByRole('status', { name: 'Outbox' }).textContent).toBe(OUTBOX_LINE_VALID)
  })

  it('renders the outbox path with a bidi override stripped, and a <b> tag in the declared name literally with no <b> element (RCS-001)', async () => {
    /** U+202E RIGHT-TO-LEFT OVERRIDE, built from its code point so no bidi control sits in this source file. */
    const rlo = String.fromCodePoint(0x202e)
    mockMountWithOutbox(() => ({
      ...OUTBOX_STATUS_VALID,
      outbox: `/home/user/project/.omnifrons/${rlo}xobtuo`,
      declared: '.omnifrons/<b>outbox</b>',
    }))

    render(<AgentPanel />)

    const line = await screen.findByRole('status', { name: 'Outbox' })
    expect(line.textContent).toBe(
      'outbox: /home/user/project/.omnifrons/xobtuo (declared .omnifrons/<b>outbox</b>)',
    )
    expect(line.querySelector('b')).toBeNull()
    expect(document.body.textContent).not.toContain(rlo)
  })

  it('shows the banner for a ShellError other than workspace-unavailable from outbox_status, as a plain catalogue code with its message, no "untrusted", and leaves the panel usable', async () => {
    mockMountWithOutbox(() =>
      Promise.reject({
        code: 'outbox-unavailable',
        message: 'the outbox status could not be computed',
        detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
      }),
    )

    render(<AgentPanel />)

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('outbox-unavailable: the outbox status could not be computed')
    expect(banner.textContent).not.toContain('untrusted')
    expect(banner.textContent).not.toContain('aaaaaaaa')
    expect(screen.queryByRole('status', { name: 'Outbox' })).toBeNull()

    await selectOption('Adapter', 'claude-code')
    await selectOption('Approval', '42')
    fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: 'still usable' } })
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
  })

  it('shows only "unexpected error" when outbox_status rejects with a plain Error, never its message (R3-005 analogue)', async () => {
    const leaky = 'boom: /home/someone/secret/outbox'
    mockMountWithOutbox(() => Promise.reject(new Error(leaky)))

    render(<AgentPanel />)

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('unexpected error')
    expect(document.body.textContent).not.toContain(leaky)
  })

  it('the outbox_status continuation no-ops after unmount', async () => {
    let resolveStatus: (status: OutboxStatus) => void = () => {}
    let statusCalled = false
    mockMountWithOutbox(() => {
      statusCalled = true
      return new Promise<OutboxStatus>((resolve) => {
        resolveStatus = resolve
      })
    })

    const { unmount } = render(<AgentPanel />)
    await waitFor(() => {
      expect(statusCalled).toBe(true)
    })
    unmount()

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    resolveStatus(OUTBOX_STATUS_VALID)
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })
})

/**
 * Asserts `block` is inert data: no interactive element, no interactive
 * attribute on it or any descendant -- the same assertions the tool-call
 * proposal block is held to (R3-007).
 */
function expectNoInteractiveElements(block: HTMLElement): HTMLElement[] {
  expect(block.querySelectorAll('button, a, input, textarea, select, details, summary').length).toBe(
    0,
  )
  expect(
    block.querySelectorAll('[role], [tabindex], [href], [contenteditable], [onclick]').length,
  ).toBe(0)
  const blockAndDescendants = [block, ...Array.from(block.querySelectorAll<HTMLElement>('*'))]
  for (const element of blockAndDescendants) {
    expect(element.getAttribute('role')).toBeNull()
    expect(element.getAttribute('tabindex')).toBeNull()
    expect(element.getAttribute('href')).toBeNull()
    expect(element.getAttribute('contenteditable')).toBeNull()
    expect(element.getAttribute('onclick')).toBeNull()
  }
  return blockAndDescendants
}

/**
 * Renders the panel with every IPC command counted, starts a run on
 * `claude-code`, and returns the live Channel plus a reader of the count --
 * so a test can click a block and prove no IPC command was invoked at all
 * (React exposes no handler as a DOM attribute, so this behavioral check
 * is the only honest one for an `onClick`).
 */
async function startWithCountedInvokes(
  onCommand?: (cmd: string, args: Record<string, unknown>) => unknown,
): Promise<{ channel: LiveChannel; invokeCount: () => number }> {
  let invokeCount = 0
  let channelRef: LiveChannel | undefined
  const handlers = defaultHandlers((cmd, args) => {
    if (cmd === 'harness_spawn') {
      channelRef = (args as { onFrame: LiveChannel }).onFrame
      return 7
    }
    if (onCommand) return onCommand(cmd, args)
    return null
  })
  mockIPC((cmd, args) => {
    invokeCount += 1
    return handlers(cmd, args)
  })

  render(<AgentPanel />)
  await selectOption('Adapter', 'claude-code')
  await selectOption('Approval', '42')
  fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: 'do the thing' } })
  fireEvent.click(screen.getByRole('button', { name: 'Start' }))
  await waitFor(() => {
    expect(channelRef).toBeDefined()
  })
  await screen.findByText('running')
  if (!channelRef) throw new Error('harness_spawn was not called')
  return { channel: channelRef, invokeCount: () => invokeCount }
}

/** Delivers one `artifact-publish` event naming `entries` by full digest, as run 7's frame `seq`. */
function deliverPublishProposal(
  channel: LiveChannel,
  entries: { name: string; sha256: string }[],
  seq = 6,
) {
  act(() => {
    channel.onmessage({
      stream: 'event',
      body: { id: 7, seq, droppedBefore: 0, kind: 'artifact-publish', payload: { entries } },
    })
  })
}

describe('AgentPanel artifact-publish proposal (slice 5)', () => {
  it('renders an artifact-publish event as a "publish proposal:" entry listing each entry as its name and the first 8 characters of its sha256, never the full digest', async () => {
    const channel = await startAndCaptureChannel()

    deliverPublishProposal(channel, [
      { name: 'report.pdf', sha256: 'ab'.repeat(32) },
      { name: 'data.zip', sha256: '0123456789abcdef'.repeat(4) },
    ])

    const proposal = screen.getByTestId('publish-proposal')
    expect(proposal.textContent).toContain('publish proposal:')
    const entries = Array.from(proposal.querySelectorAll('[data-testid="publish-entry"]')).map(
      (entry) => entry.textContent,
    )
    expect(entries).toEqual(['report.pdf abababab', 'data.zip 01234567'])
    expect(proposal.textContent).not.toContain('ab'.repeat(32))
    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.contains(proposal)).toBe(true)
    // The echoed prompt row plus exactly one row for the event.
    expect(transcript.querySelectorAll('li').length).toBe(2)
  })

  it('renders an artifact-publish event with no entries as the bare label and no entry lines', async () => {
    const channel = await startAndCaptureChannel()

    deliverPublishProposal(channel, [])

    const proposal = screen.getByTestId('publish-proposal')
    expect(proposal.textContent).toBe('publish proposal:')
    expect(proposal.querySelectorAll('[data-testid="publish-entry"]').length).toBe(0)
  })

  it('the publish proposal block has zero interactive elements, no interactive attributes, and no IPC on click (same assertions as the tool-call block)', async () => {
    const { channel, invokeCount } = await startWithCountedInvokes()

    deliverPublishProposal(channel, [{ name: 'report.pdf', sha256: 'ab'.repeat(32) }])

    const proposal = screen.getByTestId('publish-proposal')
    const blockAndDescendants = expectNoInteractiveElements(proposal)

    const invokesBeforeClick = invokeCount()
    for (const element of blockAndDescendants) {
      fireEvent.click(element)
    }
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })
    expect(invokeCount()).toBe(invokesBeforeClick)
  })

  it('strips control bytes from a proposal entry name and keeps a <b> tag literal; a sha256 shorter than 8 characters renders whole', async () => {
    const channel = await startAndCaptureChannel()
    const esc = String.fromCharCode(0x1b)

    deliverPublishProposal(channel, [{ name: `re${esc}port<b>.pdf`, sha256: 'abc' }])

    const proposal = screen.getByTestId('publish-proposal')
    expect(proposal.querySelector('b')).toBeNull()
    expect(proposal.textContent).toContain('report<b>.pdf abc')
    expect(proposal.textContent).not.toContain(esc)
  })
})

const RUN_ID = 'run-1725782401-000000001-0'

/**
 * The `candidates` summary the shell emits once at run end for the fixture
 * run of `docs/spike-log.md` § Slice 5 (IPC shapes): counts only, plus the
 * run id the follow-up `candidates_list` takes.
 */
const CANDIDATES_SUMMARY = {
  runId: RUN_ID,
  total: 5,
  candidate: 3,
  outboxEscape: 1,
  outboxLinked: 1,
  attributed: 2,
  unattributed: 3,
  unreadable: 0,
  unmatchedProposals: 1,
}

const CANDIDATES_LINE =
  'candidates: 5 total, 3 candidate, 1 escape, 1 linked, 2 attributed, 3 unattributed, 0 unreadable, 1 unmatched proposals'

/**
 * The full digests of the two validated fixture candidates
 * (`docs/spike-log.md` § Slice 5, IPC shapes): 64 lowercase hex characters,
 * `sha256Short` their first eight. Identity evidence on the row since slice
 * 5b, so an approval is made from the row itself.
 */
const REPORT_SHA256 = 'ab'.repeat(32)
const STRAY_SHA256 = '1'.repeat(64)

/**
 * `candidates_list { runId }` for that run: two validated candidates (one
 * attributed by the run's own proposal, one not) and two refused entries
 * carrying no digest facts at all (HAP-001-R20).
 */
const SAMPLE_CANDIDATES: Candidate[] = [
  {
    name: `${RUN_ID}/report.pdf`,
    size: 4096,
    sha256: REPORT_SHA256,
    sha256Short: 'abababab',
    detectedType: 'pdf',
    class: 'generated-heavy',
    attribution: { kind: 'run', runId: RUN_ID },
    state: 'candidate',
  },
  {
    name: `${RUN_ID}/stray.png`,
    size: 8,
    sha256: STRAY_SHA256,
    sha256Short: '11111111',
    detectedType: 'png',
    class: 'generated-heavy',
    attribution: { kind: 'unattributed' },
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
  {
    name: `${RUN_ID}/escape-link`,
    size: null,
    sha256: null,
    sha256Short: null,
    detectedType: null,
    class: null,
    attribution: { kind: 'unattributed' },
    state: 'outbox-escape',
  },
]

const ZERO_SUMMARY = {
  runId: RUN_ID,
  total: 0,
  candidate: 0,
  outboxEscape: 0,
  outboxLinked: 0,
  attributed: 0,
  unattributed: 0,
  unreadable: 0,
  unmatchedProposals: 0,
}

/** Delivers one `candidates` event carrying `payload`, as run 7's frame `seq`. */
function deliverCandidates(channel: LiveChannel, payload = CANDIDATES_SUMMARY, seq = 9) {
  act(() => {
    channel.onmessage({
      stream: 'event',
      body: { id: 7, seq, droppedBefore: 0, kind: 'candidates', payload },
    })
  })
}

/**
 * Starts a run whose `candidates_list` answers whatever `answer` returns
 * (rows, or a rejection built lazily inside the handler), recording every
 * args object it was called with.
 */
async function startWithCandidatesList(
  answer: () => unknown,
): Promise<{ channel: LiveChannel; calls: Record<string, unknown>[] }> {
  const calls: Record<string, unknown>[] = []
  const channel = await startAndCaptureChannel((cmd, args) => {
    if (cmd === 'candidates_list') {
      calls.push(args)
      return answer()
    }
    throw new Error(`unexpected command: ${cmd}`)
  })
  return { channel, calls }
}

/** The candidates table's data rows, each as its cells' text. */
function candidateRows(): string[][] {
  const region = screen.getByRole('region', { name: 'Candidates' })
  return Array.from(region.querySelectorAll('tbody tr')).map((row) =>
    Array.from(row.querySelectorAll('td')).map((cell) => cell.textContent ?? ''),
  )
}

describe('AgentPanel candidates summary line (slice 5)', () => {
  it('renders a candidates event as the fixed summary line with all eight counts in order, in its own transcript row', async () => {
    const { channel } = await startWithCandidatesList(() => [])

    deliverCandidates(channel)

    const transcript = screen.getByLabelText('Agent transcript')
    const rows = Array.from(transcript.querySelectorAll('li'))
    // The echoed prompt row plus exactly one row for the event.
    expect(rows.length).toBe(2)
    expect(rows[1]?.textContent).toBe(CANDIDATES_LINE)
  })

  it('renders every zero count as 0', async () => {
    const { channel } = await startWithCandidatesList(() => [])

    deliverCandidates(channel, ZERO_SUMMARY)

    expect(screen.getByLabelText('Agent transcript').textContent).toContain(
      'candidates: 0 total, 0 candidate, 0 escape, 0 linked, 0 attributed, 0 unattributed, 0 unreadable, 0 unmatched proposals',
    )
  })
})

describe('AgentPanel candidates count magnitude (slice 5 review, R3-015)', () => {
  it('renders a count of Number.MAX_SAFE_INTEGER as plain digits, never an exponent', async () => {
    const { channel } = await startWithCandidatesList(() => [])

    deliverCandidates(channel, { ...ZERO_SUMMARY, total: Number.MAX_SAFE_INTEGER })

    const line = screen.getByText(/^candidates: /)
    expect(line.textContent).toContain('candidates: 9007199254740991 total,')
    expect(line.textContent).not.toContain('e+')
  })

  it('renders a u32-max count (4294967295, the wire type of every count) and a u64-shaped 2^64 (parsed from JSON the way the wire arrives) as plain digits, never an exponent', async () => {
    // The u64 literal never appears in source (eslint's no-loss-of-precision
    // would flag it); parsed from JSON it lands on the nearest double, 2^64.
    const wire = JSON.parse('{"total":18446744073709551615}') as { total: number }
    expect(wire.total).toBe(2 ** 64)
    const { channel } = await startWithCandidatesList(() => [])

    deliverCandidates(channel, { ...ZERO_SUMMARY, total: 4_294_967_295, candidate: wire.total })

    const line = screen.getByText(/^candidates: /)
    expect(line.textContent).toContain('candidates: 4294967295 total, 18446744073709552000 candidate,')
    expect(line.textContent).not.toContain('e+')
  })
})

describe('AgentPanel candidates table (slice 5)', () => {
  it('after a candidates event, calls candidates_list with exactly { runId } and renders the table -- name, size, sha256, type, class, attribution, state, action -- with nulls as "—", every refused row marked, and an Approve button only on the generated-heavy candidate rows (slice 5b)', async () => {
    const { channel, calls } = await startWithCandidatesList(() => SAMPLE_CANDIDATES)

    deliverCandidates(channel)

    const region = await screen.findByRole('region', { name: 'Candidates' })
    expect(calls).toEqual([{ runId: RUN_ID }])
    expect(Array.from(region.querySelectorAll('th')).map((header) => header.textContent)).toEqual([
      'name',
      'size',
      'sha256',
      'type',
      'class',
      'attribution',
      'state',
      'action',
    ])
    expect(candidateRows()).toEqual([
      [
        `${RUN_ID}/report.pdf`,
        '4096',
        'abababab',
        'pdf',
        'generated-heavy',
        'run',
        'candidate',
        'Approve',
      ],
      [
        `${RUN_ID}/stray.png`,
        '8',
        '11111111',
        'png',
        'generated-heavy',
        'unattributed',
        'candidate',
        'Approve',
      ],
      [`${RUN_ID}/linked.bin`, '—', '—', '—', '—', 'unattributed', 'outbox-linked refused', ''],
      [`${RUN_ID}/escape-link`, '—', '—', '—', '—', 'unattributed', 'outbox-escape refused', ''],
    ])
    const rows = Array.from(region.querySelectorAll('tbody tr'))
    expect(rows.map((row) => row.querySelector('mark')?.textContent ?? null)).toEqual([
      null,
      null,
      'refused',
      'refused',
    ])
    // The table sits beside the transcript, never inside it.
    expect(screen.getByLabelText('Agent transcript').contains(region)).toBe(false)
  })

  it('renders no "publication not available in this slice" line: the slice 5b approval affordance replaces it', async () => {
    const { channel } = await startWithCandidatesList(() => SAMPLE_CANDIDATES)

    deliverCandidates(channel)

    const region = await screen.findByRole('region', { name: 'Candidates' })
    expect(screen.queryByText('publication not available in this slice')).toBeNull()
    expect(region.textContent).not.toContain('not available')
  })

  it('renders an empty candidates_list as the header row alone, with no data rows and no button', async () => {
    const { channel } = await startWithCandidatesList(() => [])

    deliverCandidates(channel, ZERO_SUMMARY)

    const region = await screen.findByRole('region', { name: 'Candidates' })
    expect(region.querySelectorAll('th').length).toBe(8)
    expect(candidateRows()).toEqual([])
    expect(region.querySelectorAll('button').length).toBe(0)
  })

  it("the candidates region's only interactive elements are the action column's Approve buttons: the header row and every data cell are inert (no interactive elements or attributes), and clicking anywhere in the region -- the Approve buttons included, disabled while the run is active -- triggers no IPC", async () => {
    const { channel, invokeCount } = await startWithCountedInvokes((cmd) => {
      if (cmd === 'candidates_list') return SAMPLE_CANDIDATES
      return null
    })

    deliverCandidates(channel)

    const region = await screen.findByRole('region', { name: 'Candidates' })
    const interactive = Array.from(
      region.querySelectorAll('button, a, input, textarea, select, details, summary'),
    )
    expect(interactive.map((element) => element.tagName)).toEqual(['BUTTON', 'BUTTON'])
    expect(interactive.map((element) => element.textContent)).toEqual(['Approve', 'Approve'])
    expect(interactive.every((element) => (element as HTMLButtonElement).disabled)).toBe(true)
    expectNoInteractiveElements(region.querySelector('thead') as HTMLElement)
    for (const cell of Array.from(
      region.querySelectorAll<HTMLElement>('tbody td:not(:last-child)'),
    )) {
      expectNoInteractiveElements(cell)
    }

    const invokesBeforeClick = invokeCount()
    for (const element of [region, ...Array.from(region.querySelectorAll<HTMLElement>('*'))]) {
      fireEvent.click(element)
    }
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })
    expect(invokeCount()).toBe(invokesBeforeClick)
  })

  it('renders outbox-invalid from candidates_list as a plain catalogue code with its fixed message, no "untrusted", no stray detail, and no table, while the summary line stands', async () => {
    const { channel } = await startWithCandidatesList(() =>
      Promise.reject({
        code: 'outbox-invalid',
        message: 'the classification policy could not be loaded',
        detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
      }),
    )

    deliverCandidates(channel)

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('outbox-invalid: the classification policy could not be loaded')
    expect(banner.textContent).not.toContain('untrusted')
    expect(banner.textContent).not.toContain('aaaaaaaa')
    expect(banner.textContent).not.toContain('bbbbbbbb')
    expect(screen.queryByRole('region', { name: 'Candidates' })).toBeNull()
    expect(screen.getByLabelText('Agent transcript').textContent).toContain(CANDIDATES_LINE)
  })

  it('shows only "unexpected error" when candidates_list rejects with a plain Error, never its message (R3-005 analogue)', async () => {
    const leaky = 'boom: /home/someone/secret/outbox'
    const { channel } = await startWithCandidatesList(() => Promise.reject(new Error(leaky)))

    deliverCandidates(channel)

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('unexpected error')
    expect(document.body.textContent).not.toContain(leaky)
  })

  it('drops a candidates_list response that resolves after a new run has started (generation guard): the new run shows no table', async () => {
    let resolveList: (rows: Candidate[]) => void = () => {}
    let listCalls = 0
    const { channels } = await renderMultiRun((cmd) => {
      if (cmd === 'candidates_list') {
        listCalls += 1
        return new Promise<Candidate[]>((resolve) => {
          resolveList = resolve
        })
      }
      if (cmd === 'harness_stop') return { state: 'killed', code: null }
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    deliverCandidates(channels[0]!)
    await waitFor(() => {
      expect(listCalls).toBe(1)
    })

    // Run A ends and run B starts while A's candidates_list is still pending.
    act(() => {
      channels[0]!.onmessage({
        stream: 'state',
        body: { id: 7, seq: 10, droppedBefore: 0, state: 'exited', code: 0 },
      })
    })
    await screen.findByText('exited (code 0)')
    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    expect(channels).toHaveLength(2)

    await act(async () => {
      resolveList(SAMPLE_CANDIDATES)
      await new Promise((resolve) => {
        setTimeout(resolve, 0)
      })
    })
    expect(screen.queryByRole('region', { name: 'Candidates' })).toBeNull()
    expect(listCalls).toBe(1)
  })

  it('a candidates event carrying a foreign run id never triggers candidates_list and appends nothing', async () => {
    const { channel, calls } = await startWithCandidatesList(() => SAMPLE_CANDIDATES)

    act(() => {
      channel.onmessage({
        stream: 'event',
        body: { id: 99, seq: 9, droppedBefore: 0, kind: 'candidates', payload: CANDIDATES_SUMMARY },
      })
    })
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(calls).toEqual([])
    expect(screen.getByLabelText('Agent transcript').textContent).not.toContain('candidates:')
    expect(screen.queryByRole('region', { name: 'Candidates' })).toBeNull()
  })

  it("keeps the previous run's candidates table when the next Start is rejected, since no run replaced it: the table returns as the echoed prompt is withdrawn (R3-019, R1-012 analogue)", async () => {
    let spawnCount = 0
    let channelRef: LiveChannel | undefined
    await renderReady((cmd, args) => {
      if (cmd === 'harness_spawn') {
        spawnCount += 1
        if (spawnCount === 1) {
          channelRef = (args as { onFrame: LiveChannel }).onFrame
          return 7
        }
        return Promise.reject({
          code: 'spawn-failed',
          message: 'failed to start the requested process',
        })
      }
      if (cmd === 'candidates_list') return SAMPLE_CANDIDATES
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await waitFor(() => {
      expect(channelRef).toBeDefined()
    })
    await screen.findByText('running')
    if (!channelRef) throw new Error('harness_spawn was not called')
    deliverCandidates(channelRef)
    await screen.findByRole('region', { name: 'Candidates' })
    act(() => {
      channelRef!.onmessage({
        stream: 'state',
        body: { id: 7, seq: 10, droppedBefore: 0, state: 'exited', code: 0 },
      })
    })
    await screen.findByText('exited (code 0)')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('spawn-failed: failed to start the requested process')
    expect(spawnCount).toBe(2)
    expect(screen.getByRole('region', { name: 'Candidates' })).toBeTruthy()
    expect(candidateRows()).toHaveLength(4)
    expect(screen.getByLabelText('Agent transcript').textContent).not.toContain('you: do the thing')
  })

  it("a new Start whose spawn resolves clears the previous run's candidates table along with the transcript", async () => {
    let listCalls = 0
    const { channels } = await renderMultiRun((cmd) => {
      if (cmd === 'candidates_list') {
        listCalls += 1
        return SAMPLE_CANDIDATES
      }
      if (cmd === 'harness_stop') return { state: 'killed', code: null }
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    deliverCandidates(channels[0]!)
    await screen.findByRole('region', { name: 'Candidates' })
    act(() => {
      channels[0]!.onmessage({
        stream: 'state',
        body: { id: 7, seq: 10, droppedBefore: 0, state: 'exited', code: 0 },
      })
    })
    await screen.findByText('exited (code 0)')

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')

    expect(screen.queryByRole('region', { name: 'Candidates' })).toBeNull()
    expect(listCalls).toBe(1)
  })

  it('the candidates_list continuation no-ops after unmount', async () => {
    let channelRef: LiveChannel | undefined
    let resolveList: (rows: Candidate[]) => void = () => {}
    let listCalled = false
    mockIPC(
      defaultHandlers((cmd, args) => {
        if (cmd === 'harness_spawn') {
          channelRef = (args as { onFrame: LiveChannel }).onFrame
          return 7
        }
        if (cmd === 'candidates_list') {
          listCalled = true
          return new Promise<Candidate[]>((resolve) => {
            resolveList = resolve
          })
        }
        throw new Error(`unexpected command: ${cmd}`)
      }),
    )

    const { unmount } = render(<AgentPanel />)
    await selectOption('Adapter', 'claude-code')
    await selectOption('Approval', '42')
    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await waitFor(() => {
      expect(channelRef).toBeDefined()
    })
    await screen.findByText('running')
    if (!channelRef) throw new Error('harness_spawn was not called')
    deliverCandidates(channelRef)
    await waitFor(() => {
      expect(listCalled).toBe(true)
    })
    unmount()

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    resolveList(SAMPLE_CANDIDATES)
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })
})

describe('AgentPanel continuations rejected after unmount (slice 5 review, R3-016)', () => {
  it('the outbox_status continuation no-ops when the fetch rejects after unmount: nothing thrown, no console.error, no alert', async () => {
    let rejectStatus: (reason: unknown) => void = () => {}
    let statusCalled = false
    mockMountWithOutbox(() => {
      statusCalled = true
      return new Promise<OutboxStatus>((_resolve, reject) => {
        rejectStatus = reject
      })
    })

    const { unmount } = render(<AgentPanel />)
    await waitFor(() => {
      expect(statusCalled).toBe(true)
    })
    unmount()

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    rejectStatus({ code: 'outbox-unavailable', message: 'the outbox status could not be computed' })
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    expect(document.body.querySelector('[role="alert"]')).toBeNull()
    consoleErrorSpy.mockRestore()
  })

  it('the candidates_list continuation no-ops when the fetch rejects after unmount: nothing thrown, no console.error, no alert', async () => {
    let channelRef: LiveChannel | undefined
    let rejectList: (reason: unknown) => void = () => {}
    let listCalled = false
    mockIPC(
      defaultHandlers((cmd, args) => {
        if (cmd === 'harness_spawn') {
          channelRef = (args as { onFrame: LiveChannel }).onFrame
          return 7
        }
        if (cmd === 'candidates_list') {
          listCalled = true
          return new Promise<Candidate[]>((_resolve, reject) => {
            rejectList = reject
          })
        }
        throw new Error(`unexpected command: ${cmd}`)
      }),
    )

    const { unmount } = render(<AgentPanel />)
    await selectOption('Adapter', 'claude-code')
    await selectOption('Approval', '42')
    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await waitFor(() => {
      expect(channelRef).toBeDefined()
    })
    await screen.findByText('running')
    if (!channelRef) throw new Error('harness_spawn was not called')
    deliverCandidates(channelRef)
    await waitFor(() => {
      expect(listCalled).toBe(true)
    })
    unmount()

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    rejectList({ code: 'invalid-request', message: 'no run with that id is remembered' })
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    expect(document.body.querySelector('[role="alert"]')).toBeNull()
    consoleErrorSpy.mockRestore()
  })
})

describe('AgentPanel candidates content security (slice 5, RCS-001)', () => {
  const esc = String.fromCharCode(0x1b)
  /** U+202E RIGHT-TO-LEFT OVERRIDE, built from its code point so no bidi control sits in this source file. */
  const rlo = String.fromCodePoint(0x202e)

  it('renders a candidate name carrying a literal <b>, a C0 byte, a bidi override and a "../" traversal sequence as text: no <b> element, controls stripped, the traversal kept as data the renderer never resolves', async () => {
    const { channel } = await startWithCandidatesList(() => [
      { ...SAMPLE_CANDIDATES[0]!, name: `../../<b>etc</b>/pass${esc}wd${rlo}` },
    ])

    deliverCandidates(channel)

    const region = await screen.findByRole('region', { name: 'Candidates' })
    expect(candidateRows()[0]?.[0]).toBe('../../<b>etc</b>/passwd')
    expect(region.querySelector('b')).toBeNull()
    expect(region.querySelector('a')).toBeNull()
    expect(region.querySelector('[href]')).toBeNull()
    expect(region.textContent).not.toContain(esc)
    expect(region.textContent).not.toContain(rlo)
  })

  it('renders a sha256Short with non-hex characters, and a detectedType carrying a <b> tag, as text', async () => {
    const { channel } = await startWithCandidatesList(() => [
      { ...SAMPLE_CANDIDATES[0]!, sha256Short: 'zz<b>!?', detectedType: '<b>pdf</b>' },
    ])

    deliverCandidates(channel)

    const region = await screen.findByRole('region', { name: 'Candidates' })
    expect(candidateRows()[0]?.[2]).toBe('zz<b>!?')
    expect(candidateRows()[0]?.[3]).toBe('<b>pdf</b>')
    expect(region.querySelector('b')).toBeNull()
  })
})

describe('AgentPanel outbox error codes (slice 5, R3-013 analogue)', () => {
  it('renders outbox-unavailable from harness_spawn as a plain catalogue code with its fixed message, no "untrusted" and no stray detail; nothing spawned, so the echoed prompt is withdrawn and Start re-enabled', async () => {
    await renderReady((cmd) => {
      if (cmd === 'harness_spawn') {
        return Promise.reject({
          code: 'outbox-unavailable',
          message: 'something already sits at the run subdirectory path; remove it and relaunch',
          detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe(
      'outbox-unavailable: something already sits at the run subdirectory path; remove it and relaunch',
    )
    expect(banner.textContent).not.toContain('untrusted')
    expect(banner.textContent).not.toContain('aaaaaaaa')
    expect(banner.textContent).not.toContain('bbbbbbbb')
    expect((screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
    expect(screen.getByLabelText('Agent transcript').querySelectorAll('li').length).toBe(0)
  })
})

// -- Slice 5b: approval and publication --

const PUBLICATION_ID = '2a91ea59fc5047831d97dbaebd763a3de37a5b382b683be59d10d6bdcdd92d4e'
const CATALOG_ID = `main/${PUBLICATION_ID}`

/**
 * `publications_list`'s registered record for the fixture publication
 * (`docs/spike-log.md` § Slice 5b, IPC shapes): the reference is AEC-001's
 * `ref` with the Catalog identity as locator, `names` the sanitized display
 * name plus an alias a later identical publication added, `availability`
 * this device's own observation. Never a device path.
 */
const REGISTERED_PUBLICATION: Publication = {
  publicationId: PUBLICATION_ID,
  state: 'registered',
  reference: { kind: 'artifact', id: PUBLICATION_ID, locator: CATALOG_ID },
  providerState: 'pending',
  catalogId: CATALOG_ID,
  names: ['report.pdf', 'report-copy.pdf'],
  availability: 'local',
}

/**
 * A registration that failed after a verified `published-local`: no
 * reference, provider state or Catalog identity yet (HAP-001-R24, R29).
 */
const PENDING_PUBLICATION: Publication = {
  publicationId: 'b'.repeat(64),
  state: 'registration-pending',
  reference: null,
  providerState: null,
  catalogId: null,
  names: ['stray.png'],
  availability: 'local',
}

const PUBLICATION_HEADERS = [
  'names',
  'state',
  'provider state',
  'reference',
  'catalog id',
  'availability',
]

/**
 * Mounts the panel's IPC with an active workspace and a valid outbox whose
 * `publications_list` answers whatever `answer` returns (records, or a
 * rejection built lazily inside the handler); `onCommand` handles anything
 * further.
 */
function mockMountWithPublications(
  answer: () => unknown,
  onCommand?: (cmd: string, args: Record<string, unknown>) => unknown,
) {
  mockIPC((cmd, args) => {
    if (cmd === 'workspace_current') return { displayPath: '/home/user/project', workArea: 'valid' }
    if (cmd === 'adapters_list') return [SAMPLE_ADAPTER, PTY_ADAPTER]
    if (cmd === 'approvals_list') return [sampleApproval({ approvalId: 42 })]
    if (cmd === 'outbox_status') return OUTBOX_STATUS_VALID
    if (cmd === 'publications_list') return answer()
    if (onCommand) return onCommand(cmd, args as Record<string, unknown>)
    throw new Error(`unexpected command: ${cmd}`)
  })
}

/** The publications table's data rows, each as its cells' text. */
function publicationRows(): string[][] {
  const region = screen.getByRole('region', { name: 'Publications' })
  return Array.from(region.querySelectorAll('tbody tr')).map((row) =>
    Array.from(row.querySelectorAll('td')).map((cell) => cell.textContent ?? ''),
  )
}

describe('AgentPanel publications table (slice 5b)', () => {
  it("calls publications_list on mount and renders the Publications table -- names, state, provider state, reference, catalog id, availability -- with the record's facts, the names joined, the catalog id short, and the table outside the transcript", async () => {
    mockMountWithPublications(() => [REGISTERED_PUBLICATION])

    render(<AgentPanel />)

    const region = await screen.findByRole('region', { name: 'Publications' })
    expect(Array.from(region.querySelectorAll('th')).map((header) => header.textContent)).toEqual(
      PUBLICATION_HEADERS,
    )
    expect(publicationRows()).toEqual([
      ['report.pdf, report-copy.pdf', 'registered', 'pending', CATALOG_ID, 'main/2a91ea59', 'local'],
    ])
    expect(screen.getByLabelText('Agent transcript').contains(region)).toBe(false)
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('renders a registration-pending record with "—" for its null provider state, reference and catalog id', async () => {
    mockMountWithPublications(() => [REGISTERED_PUBLICATION, PENDING_PUBLICATION])

    render(<AgentPanel />)

    await screen.findByRole('region', { name: 'Publications' })
    expect(publicationRows()[1]).toEqual([
      'stray.png',
      'registration-pending',
      '—',
      '—',
      '—',
      'local',
    ])
  })

  it('renders an empty publications_list as the section with the header row and no data rows', async () => {
    mockMountWithPublications(() => [])

    render(<AgentPanel />)

    const region = await screen.findByRole('region', { name: 'Publications' })
    expect(region.querySelectorAll('th').length).toBe(6)
    expect(publicationRows()).toEqual([])
  })

  it('calls publications_list on mount and renders no Publications section and no alert when it rejects workspace-unavailable (no workspace picked)', async () => {
    let listCalls = 0
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return null
      if (cmd === 'adapters_list') return [SAMPLE_ADAPTER]
      if (cmd === 'approvals_list') return [sampleApproval()]
      if (cmd === 'outbox_status') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'publications_list') {
        listCalls += 1
        return Promise.reject(OUTBOX_NO_WORKSPACE)
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)

    await waitFor(() => {
      expect(listCalls).toBe(1)
    })
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })
    expect(screen.queryByRole('region', { name: 'Publications' })).toBeNull()
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('fetches publications_list again after a successful workspace pick: no section before the pick, the table after it', async () => {
    let picked = false
    let listCalls = 0
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return null
      if (cmd === 'adapters_list') return [SAMPLE_ADAPTER]
      if (cmd === 'approvals_list') return [sampleApproval()]
      if (cmd === 'outbox_status') {
        return picked ? OUTBOX_STATUS_VALID : Promise.reject(OUTBOX_NO_WORKSPACE)
      }
      if (cmd === 'publications_list') {
        listCalls += 1
        return picked ? [REGISTERED_PUBLICATION] : Promise.reject(OUTBOX_NO_WORKSPACE)
      }
      if (cmd === 'workspace_pick') {
        picked = true
        return { displayPath: '/home/user/project', workArea: 'valid' }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)
    await waitFor(() => {
      expect(listCalls).toBe(1)
    })
    expect(screen.queryByRole('region', { name: 'Publications' })).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))

    await screen.findByText('/home/user/project')
    await screen.findByRole('region', { name: 'Publications' })
    expect(publicationRows()).toHaveLength(1)
    expect(listCalls).toBe(2)
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('drops a stale mount-time publications_list response that resolves after the pick-time response (sequenced fetch)', async () => {
    let resolveMountList: (records: Publication[]) => void = () => {}
    let listCalls = 0
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return null
      if (cmd === 'adapters_list') return [SAMPLE_ADAPTER]
      if (cmd === 'approvals_list') return [sampleApproval()]
      if (cmd === 'outbox_status') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'publications_list') {
        listCalls += 1
        if (listCalls === 1) {
          return new Promise<Publication[]>((resolve) => {
            resolveMountList = resolve
          })
        }
        return [REGISTERED_PUBLICATION]
      }
      if (cmd === 'workspace_pick') return { displayPath: '/home/user/project', workArea: 'valid' }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)
    await waitFor(() => {
      expect(listCalls).toBe(1)
    })
    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    await screen.findByRole('region', { name: 'Publications' })
    expect(publicationRows()).toHaveLength(1)
    expect(listCalls).toBe(2)

    await act(async () => {
      resolveMountList([PENDING_PUBLICATION])
      await new Promise((resolve) => {
        setTimeout(resolve, 0)
      })
    })

    expect(publicationRows()).toEqual([
      ['report.pdf, report-copy.pdf', 'registered', 'pending', CATALOG_ID, 'main/2a91ea59', 'local'],
    ])
  })

  it('shows the banner for catalog-unavailable from publications_list as a plain catalogue code with its fixed message, no "untrusted", no stray detail and no section, and leaves the panel usable', async () => {
    mockMountWithPublications(() =>
      Promise.reject({
        code: 'catalog-unavailable',
        message: 'the catalog is corrupt',
        detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
      }),
    )

    render(<AgentPanel />)

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('catalog-unavailable: the catalog is corrupt')
    expect(banner.textContent).not.toContain('untrusted')
    expect(banner.textContent).not.toContain('aaaaaaaa')
    expect(screen.queryByRole('region', { name: 'Publications' })).toBeNull()
    const approvalSelect = (await screen.findByLabelText('Approval')) as HTMLSelectElement
    await waitFor(() => {
      expect(Array.from(approvalSelect.options).map((option) => option.value)).toContain('42')
    })
    expect(
      (screen.getByRole('button', { name: 'Pick workspace' }) as HTMLButtonElement).disabled,
    ).toBe(false)
  })

  it('shows only "unexpected error" when publications_list rejects with a plain Error, never its message', async () => {
    mockMountWithPublications(() => Promise.reject(new Error('catalog path /secret/catalog.jsonl')))

    render(<AgentPanel />)

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('unexpected error')
    expect(document.body.textContent).not.toContain('/secret/catalog.jsonl')
  })

  it('the publications_list continuation no-ops after unmount: nothing thrown, no console.error', async () => {
    let resolveList: (records: Publication[]) => void = () => {}
    let listCalled = false
    mockMountWithPublications(() => {
      listCalled = true
      return new Promise<Publication[]>((resolve) => {
        resolveList = resolve
      })
    })

    const { unmount } = render(<AgentPanel />)
    await waitFor(() => {
      expect(listCalled).toBe(true)
    })
    unmount()

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    resolveList([REGISTERED_PUBLICATION])
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('the publications_list continuation no-ops when the fetch rejects after unmount: nothing thrown, no console.error, no alert', async () => {
    let rejectList: (reason: unknown) => void = () => {}
    let listCalled = false
    mockMountWithPublications(() => {
      listCalled = true
      return new Promise<Publication[]>((_resolve, reject) => {
        rejectList = reject
      })
    })

    const { unmount } = render(<AgentPanel />)
    await waitFor(() => {
      expect(listCalled).toBe(true)
    })
    unmount()

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    rejectList({ code: 'catalog-unavailable', message: 'the catalog is corrupt' })
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    expect(document.body.querySelector('[role="alert"]')).toBeNull()
    consoleErrorSpy.mockRestore()
  })
})

describe('AgentPanel publications content security (slice 5b, RCS-001)', () => {
  const esc = String.fromCharCode(0x1b)
  /** U+202E RIGHT-TO-LEFT OVERRIDE, built from its code point so no bidi control sits in this source file. */
  const rlo = String.fromCodePoint(0x202e)

  it('renders a display name carrying a literal <b>, a C0 byte and a bidi override as text with no <b> element, and the reference locator and catalog id as text with no anchor and no href', async () => {
    mockMountWithPublications(() => [
      { ...REGISTERED_PUBLICATION, names: [`<b>report</b>${esc}.pdf${rlo}`, 'plain.pdf'] },
    ])

    render(<AgentPanel />)

    const region = await screen.findByRole('region', { name: 'Publications' })
    expect(publicationRows()[0]?.[0]).toBe('<b>report</b>.pdf, plain.pdf')
    expect(publicationRows()[0]?.[3]).toBe(CATALOG_ID)
    expect(publicationRows()[0]?.[4]).toBe('main/2a91ea59')
    expect(region.querySelector('b')).toBeNull()
    expect(region.querySelector('a')).toBeNull()
    expect(region.querySelector('[href]')).toBeNull()
    expect(region.textContent).not.toContain(esc)
    expect(region.textContent).not.toContain(rlo)
  })
})

/**
 * The short digest of `report.pdf` as the row lists it -- what the user
 * types to approve (slice 2's shape); the row's own full `sha256` is what
 * the request carries, never the typed value.
 */
const REPORT_SHORT = 'abababab'

/**
 * `artifact_approve`'s payload for the fixture run's attributed entry
 * (`docs/spike-log.md` § Slice 5b, IPC shapes): the identity-bound facts,
 * the destination asset root identity, the act-as identity, the derived
 * ids. Never a device path.
 */
const SAMPLE_ARTIFACT_APPROVAL: ArtifactApproval = {
  approvalId: '23121521465ad9be',
  publicationId: PUBLICATION_ID,
  runId: RUN_ID,
  name: `${RUN_ID}/report.pdf`,
  displayName: 'report.pdf',
  sha256Short: 'abababab',
  size: 4096,
  detectedType: 'pdf',
  class: 'generated-heavy',
  attribution: { kind: 'run', runId: RUN_ID },
  assetRootId: 'main',
  actAs: 'device-local-user',
  approvedAt: 1725782401000,
}

const APPROVED_REPORT_CELL = 'approved 23121521465ad9be — report.pdf, asset root main Publish'

/** Ends run `id` with a terminal state frame, so the run guards release the approval controls. */
async function endRun(channel: LiveChannel, id = 7, seq = 10): Promise<void> {
  act(() => {
    channel.onmessage({
      stream: 'state',
      body: { id, seq, droppedBefore: 0, state: 'exited', code: 0 },
    })
  })
  await screen.findByText('exited (code 0)')
}

/** The candidates table's data row whose name cell reads `name`. */
function candidateRow(name: string): HTMLElement {
  const region = screen.getByRole('region', { name: 'Candidates' })
  const row = Array.from(region.querySelectorAll<HTMLElement>('tbody tr')).find(
    (candidate) => candidate.querySelector('td')?.textContent === name,
  )
  if (!row) throw new Error(`no candidates row named ${name}`)
  return row
}

/** The action cell of the row named `name`, as text. */
function actionCell(name: string): string {
  const cells = candidateRow(name).querySelectorAll('td')
  return cells[cells.length - 1]?.textContent ?? ''
}

/**
 * Starts a run whose inventory is the sample candidates, ends it (the
 * approval controls are frozen while a run is active), and returns the
 * live Channel plus every IPC call made from then on -- `onCommand`
 * answers the approval and publication commands a test drives.
 */
async function startInventoryAndEndRun(
  onCommand?: (cmd: string, args: Record<string, unknown>) => unknown,
  // A valid outbox whose policy declares the asset root `main` by default:
  // the approval block shows its destination from the outbox status and
  // will not approve toward an unknown or unconfigured one.
  options: MockOptions = { outbox: OUTBOX_STATUS_VALID },
  adapterId = 'claude-code',
): Promise<{ channel: LiveChannel; calls: { cmd: string; args: Record<string, unknown> }[] }> {
  const calls: { cmd: string; args: Record<string, unknown> }[] = []
  const channel = await startAndCaptureChannel(
    (cmd, args) => {
      calls.push({ cmd, args })
      if (cmd === 'candidates_list') return SAMPLE_CANDIDATES
      if (onCommand) return onCommand(cmd, args)
      throw new Error(`unexpected command: ${cmd}`)
    },
    adapterId,
    options,
  )
  deliverCandidates(channel)
  await screen.findByRole('region', { name: 'Candidates' })
  await endRun(channel)
  return { channel, calls }
}

const APPROVAL_INPUT_LABEL = /Type the short digest/

/**
 * A fixture adapter with a stricter scope mode than the two built-ins, so
 * the block's `scope:` line can be shown to follow the adapter the run was
 * started with rather than the current selection. Metadata only, like the
 * real descriptors.
 */
const SANDBOX_ADAPTER: AdapterDescriptor = {
  id: 'sandbox-cli',
  displayName: 'Sandbox CLI',
  transportClass: 'structured-streaming-cli',
  promptChannel: 'stdin-then-close',
  scopeMode: 'sandbox-enforced',
  notes: 'fixture: a sandbox-enforced scope mode',
}

/** Clicks the Approve button of the row named `name` and returns the approval block it opens. */
async function openApproval(name: string): Promise<HTMLElement> {
  fireEvent.click(within(candidateRow(name)).getByRole('button', { name: 'Approve' }))
  return screen.findByLabelText('Publication approval')
}

function approvalInput(): HTMLInputElement {
  return screen.getByLabelText(APPROVAL_INPUT_LABEL) as HTMLInputElement
}

function confirmButton(): HTMLButtonElement {
  return screen.getByRole('button', { name: 'Approve publication' }) as HTMLButtonElement
}

function typeDigest(value: string): void {
  fireEvent.change(approvalInput(), { target: { value } })
}

/** Types the report's full digest and clicks the final button. */
function confirmReportApproval(): void {
  typeDigest(REPORT_SHORT)
  fireEvent.click(confirmButton())
}

describe('AgentPanel approval affordance (slice 5b, HAP-001-R22, TM-001-R7)', () => {
  it("Approve opens a block, inside the Candidates region and outside the transcript, showing the candidate's identity-bound facts verbatim -- name, class, type, size, sha256, attribution with its run id -- then the destination (the outbox status's asset root) and the scope mode of the run's adapter, and the fixed act-as line; opening it invokes nothing, the input opens empty and the final button disabled", async () => {
    const { calls } = await startInventoryAndEndRun()
    const invokesBefore = calls.length

    const block = await openApproval(`${RUN_ID}/report.pdf`)

    expect(Array.from(block.querySelectorAll('p')).map((line) => line.textContent)).toEqual([
      `name: ${RUN_ID}/report.pdf`,
      'class: generated-heavy',
      'type: pdf',
      'size: 4096',
      `sha256: ${REPORT_SHA256}`,
      'sha256 short: abababab',
      `attribution: run ${RUN_ID}`,
      'destination: asset root main',
      'scope: advisory',
      'act as: device-local-user',
    ])
    expect(calls.length).toBe(invokesBefore)
    expect(approvalInput().value).toBe('')
    expect(confirmButton().disabled).toBe(true)
    expect(screen.getByRole('region', { name: 'Candidates' }).contains(block)).toBe(true)
    expect(screen.getByLabelText('Agent transcript').contains(block)).toBe(false)
  })

  it('shows the unattributed fact for an unattributed candidate: "attribution: unattributed", with no run id standing in for a producer', async () => {
    await startInventoryAndEndRun()

    const block = await openApproval(`${RUN_ID}/stray.png`)

    const lines = Array.from(block.querySelectorAll('p')).map((line) => line.textContent)
    expect(lines).toContain('attribution: unattributed')
    expect(lines).toContain(`sha256: ${STRAY_SHA256}`)
    expect(lines).toContain('sha256 short: 11111111')
    expect(lines).toContain(`name: ${RUN_ID}/stray.png`)
    expect(lines.filter((line) => line?.startsWith('attribution:'))).toEqual([
      'attribution: unattributed',
    ])
  })

  it('shows "destination: unconfigured" when the outbox status carries no asset root id, and keeps the final button disabled with the right short digest typed -- the shell would refuse with destination-invalid (HAP-001-R22, R1-002)', async () => {
    const { calls } = await startInventoryAndEndRun(undefined, {
      outbox: { ...OUTBOX_STATUS_VALID, assetRootId: null },
    })

    const block = await openApproval(`${RUN_ID}/report.pdf`)

    const lines = Array.from(block.querySelectorAll('p')).map((line) => line.textContent)
    expect(lines).toContain('destination: unconfigured')
    expect(lines).not.toContain('destination: asset root main')
    typeDigest(REPORT_SHORT)
    expect(confirmButton().disabled).toBe(true)
    fireEvent.click(confirmButton())
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })
    expect(calls.filter((call) => call.cmd === 'artifact_approve')).toHaveLength(0)
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('shows "destination: unknown" while no outbox status is known at all, and keeps the final button disabled with the right short digest typed', async () => {
    const { calls } = await startInventoryAndEndRun(undefined, {})

    const block = await openApproval(`${RUN_ID}/report.pdf`)

    const lines = Array.from(block.querySelectorAll('p')).map((line) => line.textContent)
    expect(lines).toContain('destination: unknown')
    typeDigest(REPORT_SHORT)
    expect(confirmButton().disabled).toBe(true)
    fireEvent.click(confirmButton())
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })
    expect(calls.filter((call) => call.cmd === 'artifact_approve')).toHaveLength(0)
  })

  it("shows the scope mode of the adapter the run was started with, from its descriptor and not from the current selection: a run started with a sandbox-enforced adapter keeps 'scope: sandbox-enforced' after the selection moves to an advisory one", async () => {
    await startInventoryAndEndRun(
      undefined,
      { outbox: OUTBOX_STATUS_VALID, adapters: [SAMPLE_ADAPTER, SANDBOX_ADAPTER] },
      'sandbox-cli',
    )
    // The run has ended, so the selection controls are live again.
    await selectOption('Adapter', 'claude-code')

    const block = await openApproval(`${RUN_ID}/report.pdf`)

    const lines = Array.from(block.querySelectorAll('p')).map((line) => line.textContent)
    expect(lines).toContain('scope: sandbox-enforced')
    expect(lines).not.toContain('scope: advisory')
  })

  it("enables the final button only when the typed value equals the row's sha256Short exactly -- never for the fixed phrase, uppercase hex, another row's short digest, a 7- or 9-character prefix, or the full digest", async () => {
    await startInventoryAndEndRun()
    await openApproval(`${RUN_ID}/report.pdf`)

    typeDigest('approve')
    expect(confirmButton().disabled).toBe(true)
    typeDigest('ABABABAB')
    expect(confirmButton().disabled).toBe(true)
    typeDigest('11111111')
    expect(confirmButton().disabled).toBe(true)
    typeDigest('abababa')
    expect(confirmButton().disabled).toBe(true)
    typeDigest('ababababa')
    expect(confirmButton().disabled).toBe(true)
    typeDigest(REPORT_SHA256)
    expect(confirmButton().disabled).toBe(true)
    typeDigest(REPORT_SHORT)
    expect(confirmButton().disabled).toBe(false)
  })

  it("no harness string pre-fills or enables the approval (TM-001-R1): a publish proposal naming the entry by its exact full digest leaves the input empty and the final button disabled, and setting the input's value programmatically with no input event leaves it disabled too", async () => {
    const channel = await startAndCaptureChannel(
      (cmd) => {
        if (cmd === 'candidates_list') return SAMPLE_CANDIDATES
        throw new Error(`unexpected command: ${cmd}`)
      },
      'claude-code',
      { outbox: OUTBOX_STATUS_VALID },
    )
    // The harness's own proposal, naming the very row by the very digest
    // the row carries: content, and never an approval input.
    deliverPublishProposal(channel, [{ name: 'report.pdf', sha256: REPORT_SHA256 }])
    deliverCandidates(channel)
    await screen.findByRole('region', { name: 'Candidates' })
    await endRun(channel)

    await openApproval(`${RUN_ID}/report.pdf`)

    expect(approvalInput().value).toBe('')
    expect(confirmButton().disabled).toBe(true)
    // What a script-driven pre-fill would do: set the DOM value with no
    // input event. React's controlled input never sees it -- its state,
    // and so the gate, stays on the empty string.
    approvalInput().value = REPORT_SHORT
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })
    expect(approvalInput().value).toBe(REPORT_SHORT)
    expect(confirmButton().disabled).toBe(true)
    // The user's own typing is what enables it. (React's value tracker
    // treats a change event carrying the very string already set on the
    // node as no change, so the typed sequence passes through another
    // value first -- a test-harness detail, not a property of the gate.)
    typeDigest('')
    expect(confirmButton().disabled).toBe(true)
    typeDigest(REPORT_SHORT)
    expect(confirmButton().disabled).toBe(false)
  })

  it('the final button calls artifact_approve with exactly { runId, name, sha256 } -- the inventory\'s run id, the row\'s name, the row\'s own full digest and never the typed value -- and on success the row\'s action cell shows "approved <approvalId> — <displayName>, asset root <assetRootId>" with a Publish button, its Approve button gone, the block closed, the other rows untouched', async () => {
    const { calls } = await startInventoryAndEndRun((cmd) => {
      if (cmd === 'artifact_approve') return SAMPLE_ARTIFACT_APPROVAL
      throw new Error(`unexpected command: ${cmd}`)
    })
    await openApproval(`${RUN_ID}/report.pdf`)

    confirmReportApproval()

    await waitFor(() => {
      expect(actionCell(`${RUN_ID}/report.pdf`)).toBe(APPROVED_REPORT_CELL)
    })
    const approveCalls = calls.filter((call) => call.cmd === 'artifact_approve')
    expect(approveCalls).toHaveLength(1)
    expect(approveCalls[0]?.args).toEqual({
      runId: RUN_ID,
      name: `${RUN_ID}/report.pdf`,
      sha256: REPORT_SHA256,
    })
    expect(Object.keys(approveCalls[0]!.args).sort()).toEqual(['name', 'runId', 'sha256'])
    expect(approveCalls[0]?.args.sha256).toHaveLength(64)
    expect(approveCalls[0]?.args.sha256).not.toBe(REPORT_SHORT)
    const report = candidateRow(`${RUN_ID}/report.pdf`)
    expect(within(report).queryByRole('button', { name: 'Approve' })).toBeNull()
    expect(within(report).getByRole('button', { name: 'Publish' })).toBeTruthy()
    expect(screen.queryByLabelText('Publication approval')).toBeNull()
    expect(actionCell(`${RUN_ID}/stray.png`)).toBe('Approve')
    expect(actionCell(`${RUN_ID}/linked.bin`)).toBe('')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('renders refused from artifact_approve as a plain catalogue code with its message, no "untrusted" and no stray detail; the row keeps its Approve button, the block stays open with the typed digest kept and the final button enabled again', async () => {
    await startInventoryAndEndRun((cmd) => {
      if (cmd === 'artifact_approve') {
        return Promise.reject({
          code: 'refused',
          message: "the candidate's digest does not match the approval request",
          detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    await openApproval(`${RUN_ID}/report.pdf`)

    confirmReportApproval()

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe(
      "refused: the candidate's digest does not match the approval request",
    )
    expect(banner.textContent).not.toContain('untrusted')
    expect(banner.textContent).not.toContain('aaaaaaaa')
    expect(actionCell(`${RUN_ID}/report.pdf`)).toBe('Approve')
    expect(screen.getByLabelText('Publication approval')).toBeTruthy()
    expect(approvalInput().value).toBe(REPORT_SHORT)
    await waitFor(() => {
      expect(confirmButton().disabled).toBe(false)
    })
  })

  it.each([
    ['destination-invalid', 'the project declares no asset root'],
    ['work-area-invalid', 'the product work area resolves inside a registered workspace root'],
    ['invalid-request', 'no run with that id is remembered'],
    ['outbox-invalid', 'the classification policy could not be loaded'],
    ['workspace-unavailable', 'no workspace has been picked yet'],
    // The backend mirror of the frozen surface: the shell refuses while any
    // supervised process runs, even a request the renderer never freezes.
    ['run-active', 'a run is active; approve or publish once it has ended'],
  ])(
    'renders %s from artifact_approve as a plain catalogue code with its fixed message, no "untrusted" and no stray detail; the row keeps its Approve button and the block stays usable with the typed digest kept and the final button enabled again (R3-021)',
    async (code, message) => {
      await startInventoryAndEndRun((cmd) => {
        if (cmd === 'artifact_approve') {
          return Promise.reject({
            code,
            message,
            detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
          })
        }
        throw new Error(`unexpected command: ${cmd}`)
      })
      await openApproval(`${RUN_ID}/report.pdf`)

      confirmReportApproval()

      const banner = await screen.findByRole('alert')
      expect(banner.textContent).toBe(`${code}: ${message}`)
      expect(banner.textContent).not.toContain('untrusted')
      expect(banner.textContent).not.toContain('aaaaaaaa')
      expect(actionCell(`${RUN_ID}/report.pdf`)).toBe('Approve')
      expect(screen.getByLabelText('Publication approval')).toBeTruthy()
      expect(approvalInput().value).toBe(REPORT_SHORT)
      await waitFor(() => {
        expect(confirmButton().disabled).toBe(false)
      })
    },
  )

  it('shows only "unexpected error" when artifact_approve rejects with a plain Error, never its message', async () => {
    await startInventoryAndEndRun((cmd) => {
      if (cmd === 'artifact_approve') return Promise.reject(new Error('journal path /secret/work-area'))
      throw new Error(`unexpected command: ${cmd}`)
    })
    await openApproval(`${RUN_ID}/report.pdf`)

    confirmReportApproval()

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('unexpected error')
    expect(document.body.textContent).not.toContain('/secret/work-area')
  })

  it('disables the final button and every Approve button while the approve is in flight, so a double-click fires one artifact_approve, and applies the response once it resolves', async () => {
    let resolveApprove: (approval: ArtifactApproval) => void = () => {}
    const { calls } = await startInventoryAndEndRun((cmd) => {
      if (cmd === 'artifact_approve') {
        return new Promise<ArtifactApproval>((resolve) => {
          resolveApprove = resolve
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    await openApproval(`${RUN_ID}/report.pdf`)
    typeDigest(REPORT_SHORT)

    fireEvent.click(confirmButton())
    fireEvent.click(confirmButton())

    await waitFor(() => {
      expect(confirmButton().disabled).toBe(true)
    })
    expect(
      (within(candidateRow(`${RUN_ID}/stray.png`)).getByRole('button', { name: 'Approve' }) as HTMLButtonElement)
        .disabled,
    ).toBe(true)
    expect(calls.filter((call) => call.cmd === 'artifact_approve')).toHaveLength(1)

    await act(async () => {
      resolveApprove(SAMPLE_ARTIFACT_APPROVAL)
      await new Promise((resolve) => {
        setTimeout(resolve, 0)
      })
    })
    expect(actionCell(`${RUN_ID}/report.pdf`)).toBe(APPROVED_REPORT_CELL)
    expect(
      (within(candidateRow(`${RUN_ID}/stray.png`)).getByRole('button', { name: 'Approve' }) as HTMLButtonElement)
        .disabled,
    ).toBe(false)
  })

  it('keeps every Approve button disabled while the run is active -- the inventory arrives before the terminal state frame -- and enables them once the run ends (controls frozen, spike default)', async () => {
    const { channel } = await startWithCandidatesList(() => SAMPLE_CANDIDATES)
    deliverCandidates(channel)
    await screen.findByRole('region', { name: 'Candidates' })

    const approveButtons = () =>
      Array.from(
        screen.getByRole('region', { name: 'Candidates' }).querySelectorAll<HTMLButtonElement>('button'),
      )
    expect(approveButtons().map((button) => button.disabled)).toEqual([true, true])

    await endRun(channel)

    expect(approveButtons().map((button) => button.disabled)).toEqual([false, false])
  })

  it('offers Approve only on a generated-heavy candidate carrying its digest: none on a generated-heavy candidate whose sha256 is null, none on a candidate of another class with a digest, none on an executable candidate without one (R3-024)', async () => {
    const { channel } = await startWithCandidatesList(() => [
      { ...SAMPLE_CANDIDATES[0]!, sha256: null },
      {
        ...SAMPLE_CANDIDATES[1]!,
        name: `${RUN_ID}/notes.md`,
        class: 'portable-text',
        detectedType: 'markdown',
      },
      { ...SAMPLE_CANDIDATES[1]!, name: `${RUN_ID}/tool`, class: 'executable', sha256: null },
      SAMPLE_CANDIDATES[1]!,
    ])
    deliverCandidates(channel)
    await screen.findByRole('region', { name: 'Candidates' })
    await endRun(channel)

    expect(actionCell(`${RUN_ID}/report.pdf`)).toBe('')
    expect(actionCell(`${RUN_ID}/notes.md`)).toBe('')
    expect(actionCell(`${RUN_ID}/tool`)).toBe('')
    expect(actionCell(`${RUN_ID}/stray.png`)).toBe('Approve')
  })

  it("clicking another row's Approve switches the block to that row's facts and clears the typed digest", async () => {
    await startInventoryAndEndRun()
    await openApproval(`${RUN_ID}/report.pdf`)
    typeDigest(REPORT_SHORT)
    expect(confirmButton().disabled).toBe(false)

    const block = await openApproval(`${RUN_ID}/stray.png`)

    expect(Array.from(block.querySelectorAll('p')).map((line) => line.textContent)).toContain(
      `name: ${RUN_ID}/stray.png`,
    )
    expect(block.textContent).not.toContain('report.pdf')
    expect(approvalInput().value).toBe('')
    expect(confirmButton().disabled).toBe(true)
    expect(screen.getAllByLabelText('Publication approval')).toHaveLength(1)
  })

  it('the approve continuation no-ops after unmount: nothing thrown, no console.error', async () => {
    let resolveApprove: (approval: ArtifactApproval) => void = () => {}
    let approveCalled = false
    await startInventoryAndEndRun((cmd) => {
      if (cmd === 'artifact_approve') {
        approveCalled = true
        return new Promise<ArtifactApproval>((resolve) => {
          resolveApprove = resolve
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    await openApproval(`${RUN_ID}/report.pdf`)
    confirmReportApproval()
    await waitFor(() => {
      expect(approveCalled).toBe(true)
    })

    // `render()` was called by the helpers; `cleanup()` unmounts everything
    // it mounted, the way the global `afterEach` does after each test.
    cleanup()

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    resolveApprove(SAMPLE_ARTIFACT_APPROVAL)
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('the approve continuation no-ops when artifact_approve rejects after unmount: nothing thrown, no console.error, no alert (R3-022)', async () => {
    let rejectApprove: (reason: unknown) => void = () => {}
    let approveCalled = false
    await startInventoryAndEndRun((cmd) => {
      if (cmd === 'artifact_approve') {
        approveCalled = true
        return new Promise<ArtifactApproval>((_resolve, reject) => {
          rejectApprove = reject
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    await openApproval(`${RUN_ID}/report.pdf`)
    confirmReportApproval()
    await waitFor(() => {
      expect(approveCalled).toBe(true)
    })

    cleanup()

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    rejectApprove({
      code: 'refused',
      message: "the candidate's digest does not match the approval request",
    })
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    expect(document.body.querySelector('[role="alert"]')).toBeNull()
    consoleErrorSpy.mockRestore()
  })

  it("drops an approve response that lands after a new run replaced the inventory (stale-response guard): the new run's row of the same name stays unapproved and no alert renders", async () => {
    let resolveApprove: (approval: ArtifactApproval) => void = () => {}
    const { channels } = await renderMultiRun(
      (cmd) => {
        if (cmd === 'candidates_list') return SAMPLE_CANDIDATES
        if (cmd === 'artifact_approve') {
          return new Promise<ArtifactApproval>((resolve) => {
            resolveApprove = resolve
          })
        }
        throw new Error(`unexpected command: ${cmd}`)
      },
      'claude-code',
      { outbox: OUTBOX_STATUS_VALID },
    )
    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    deliverCandidates(channels[0]!)
    await screen.findByRole('region', { name: 'Candidates' })
    await endRun(channels[0]!)
    await openApproval(`${RUN_ID}/report.pdf`)
    confirmReportApproval()
    await waitFor(() => {
      expect(confirmButton().disabled).toBe(true)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    act(() => {
      channels[1]!.onmessage({
        stream: 'event',
        body: { id: 8, seq: 9, droppedBefore: 0, kind: 'candidates', payload: CANDIDATES_SUMMARY },
      })
    })
    await screen.findByRole('region', { name: 'Candidates' })
    await endRun(channels[1]!, 8)

    await act(async () => {
      resolveApprove(SAMPLE_ARTIFACT_APPROVAL)
      await new Promise((resolve) => {
        setTimeout(resolve, 0)
      })
    })

    expect(actionCell(`${RUN_ID}/report.pdf`)).toBe('Approve')
    expect(screen.queryByLabelText('Publication approval')).toBeNull()
    expect(screen.queryByRole('alert')).toBeNull()
  })
})

describe('AgentPanel approval block content security (slice 5b, RCS-001)', () => {
  const esc = String.fromCharCode(0x1b)
  /** U+202E RIGHT-TO-LEFT OVERRIDE, built from its code point so no bidi control sits in this source file. */
  const rlo = String.fromCodePoint(0x202e)

  it('renders a candidate name carrying a literal <b>, a C0 byte, a bidi override and a "../" traversal in the block as text: no <b> element, no anchor, controls stripped, the traversal kept as data the renderer never resolves; the label shows the short digest as text', async () => {
    const channel = await startAndCaptureChannel((cmd) => {
      if (cmd === 'candidates_list') {
        return [
          {
            ...SAMPLE_CANDIDATES[0]!,
            name: `../../<b>etc</b>/pass${esc}wd${rlo}`,
            detectedType: '<b>pdf</b>',
          },
        ]
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    deliverCandidates(channel)
    await screen.findByRole('region', { name: 'Candidates' })
    await endRun(channel)

    const block = await openApproval('../../<b>etc</b>/passwd')

    const lines = Array.from(block.querySelectorAll('p')).map((line) => line.textContent)
    expect(lines[0]).toBe('name: ../../<b>etc</b>/passwd')
    expect(lines).toContain('type: <b>pdf</b>')
    expect(block.querySelector('b')).toBeNull()
    expect(block.querySelector('a')).toBeNull()
    expect(block.querySelector('[href]')).toBeNull()
    expect(block.textContent).not.toContain(esc)
    expect(block.textContent).not.toContain(rlo)
    expect(screen.getByText(APPROVAL_INPUT_LABEL).textContent).toContain('abababab')
  })
})

const WORK_AREA_LINE = 'work area invalid — publication refused until it is fixed'

describe('AgentPanel work area line (slice 5b, HAP-001-R7)', () => {
  it('renders the fixed work-area line as a status line named "Work area", beside the outbox line, when workspace_current reports work-area-invalid on mount; the raw token is not rendered and no alert', async () => {
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') {
        return { displayPath: '/home/user/project', workArea: 'work-area-invalid' }
      }
      if (cmd === 'adapters_list') return [SAMPLE_ADAPTER]
      if (cmd === 'approvals_list') return [sampleApproval()]
      if (cmd === 'outbox_status') return OUTBOX_STATUS_VALID
      if (cmd === 'publications_list') return []
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)

    const line = await screen.findByRole('status', { name: 'Work area' })
    expect(line.textContent).toBe(WORK_AREA_LINE)
    expect(document.body.textContent).not.toContain('work-area-invalid')
    const outboxLine = await screen.findByRole('status', { name: 'Outbox' })
    expect(line.parentElement).toBe(outboxLine.parentElement)
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('renders no work-area line for a valid workArea, on mount and after a pick', async () => {
    mockMountWithPublications(
      () => [],
      (cmd) => {
        if (cmd === 'workspace_pick') return { displayPath: '/home/user/other', workArea: 'valid' }
        throw new Error(`unexpected command: ${cmd}`)
      },
    )

    render(<AgentPanel />)

    await screen.findByRole('status', { name: 'Outbox' })
    expect(screen.queryByRole('status', { name: 'Work area' })).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))

    await screen.findByText('/home/user/other')
    expect(screen.queryByRole('status', { name: 'Work area' })).toBeNull()
  })

  it('renders the work-area line after a pick whose workspace reports work-area-invalid, and removes it after a later pick reporting valid', async () => {
    let pickCount = 0
    mockIPC((cmd) => {
      if (cmd === 'workspace_current') return null
      if (cmd === 'adapters_list') return [SAMPLE_ADAPTER]
      if (cmd === 'approvals_list') return [sampleApproval()]
      if (cmd === 'outbox_status') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'publications_list') return Promise.reject(OUTBOX_NO_WORKSPACE)
      if (cmd === 'workspace_pick') {
        pickCount += 1
        return pickCount === 1
          ? { displayPath: '/home/user/a', workArea: 'work-area-invalid' }
          : { displayPath: '/home/user/b', workArea: 'valid' }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<AgentPanel />)
    expect(screen.queryByRole('status', { name: 'Work area' })).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    await screen.findByText('/home/user/a')
    expect(screen.getByRole('status', { name: 'Work area' }).textContent).toBe(WORK_AREA_LINE)

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    await screen.findByText('/home/user/b')
    expect(screen.queryByRole('status', { name: 'Work area' })).toBeNull()
  })
})

type LiveStateChannel = { onmessage: (frame: ArtifactStateFrame) => void }
type StatePayload = ArtifactStateFrame['payload']

/** The publication `artifact_publish` returns for the report: a fresh record, one name. */
const PUBLISHED_REPORT: Publication = { ...REGISTERED_PUBLICATION, names: ['report.pdf'] }

/** The same publication left `registration-pending`: no reference, provider state or Catalog identity. */
const PENDING_REPORT: Publication = {
  ...PENDING_PUBLICATION,
  publicationId: PUBLICATION_ID,
  names: ['report.pdf'],
}

const REPORT_NAME = `${RUN_ID}/report.pdf`
const STRAY_NAME = `${RUN_ID}/stray.png`
const STRAY_PUBLICATION_ID = 'c'.repeat(64)

/** The unattributed row's approval (`docs/spike-log.md` § Slice 5b, IPC shapes): the unattributed fact recorded, nothing standing in for a producer. */
const STRAY_ARTIFACT_APPROVAL: ArtifactApproval = {
  approvalId: '5f0c5f0c5f0c5f0c',
  publicationId: STRAY_PUBLICATION_ID,
  runId: RUN_ID,
  name: STRAY_NAME,
  displayName: 'stray.png',
  sha256Short: '11111111',
  size: 8,
  detectedType: 'png',
  class: 'generated-heavy',
  attribution: { kind: 'unattributed' },
  assetRootId: 'main',
  actAs: 'device-local-user',
  approvedAt: 1725782402000,
}

/** The unattributed row's registered publication. */
const PUBLISHED_STRAY: Publication = {
  publicationId: STRAY_PUBLICATION_ID,
  state: 'registered',
  reference: {
    kind: 'artifact',
    id: STRAY_PUBLICATION_ID,
    locator: `main/${STRAY_PUBLICATION_ID}`,
  },
  providerState: 'pending',
  catalogId: `main/${STRAY_PUBLICATION_ID}`,
  names: ['stray.png'],
  availability: 'local',
}

const APPROVED_STRAY_CELL = 'approved 5f0c5f0c5f0c5f0c — stray.png, asset root main Publish'
const PUBLISHED_REPORT_ROW = [
  'report.pdf',
  'registered',
  'pending',
  CATALOG_ID,
  'main/2a91ea59',
  'local',
]
const PUBLISHED_STRAY_ROW = [
  'stray.png',
  'registered',
  'pending',
  `main/${STRAY_PUBLICATION_ID}`,
  'main/cccccccc',
  'local',
]

/** Delivers one `artifact-state` frame on the publish channel. */
function deliverArtifactState(
  channel: LiveStateChannel,
  state: StatePayload['state'],
  providerState: StatePayload['providerState'] = null,
  publicationId = PUBLICATION_ID,
): void {
  act(() => {
    channel.onmessage({ kind: 'artifact-state', payload: { publicationId, state, providerState } })
  })
}

/**
 * Starts a run on the sample inventory, ends it, approves `report.pdf`
 * with the fixture approval, and returns every IPC call from the start plus
 * a reader of the live publish channel once Publish has been clicked.
 * `onPublish` answers `artifact_publish` (the registered publication by
 * default; a deferred promise or a rejection when given).
 */
async function approveReport(
  onPublish?: () => unknown,
): Promise<{
  calls: { cmd: string; args: Record<string, unknown> }[]
  publishChannel: () => LiveStateChannel
}> {
  let publishChannelRef: LiveStateChannel | undefined
  const { calls } = await startInventoryAndEndRun((cmd, args) => {
    if (cmd === 'artifact_approve') return SAMPLE_ARTIFACT_APPROVAL
    if (cmd === 'artifact_publish') {
      publishChannelRef = (args as { onState: LiveStateChannel }).onState
      return onPublish ? onPublish() : PUBLISHED_REPORT
    }
    throw new Error(`unexpected command: ${cmd}`)
  })
  await openApproval(REPORT_NAME)
  confirmReportApproval()
  await waitFor(() => {
    expect(actionCell(REPORT_NAME)).toBe(APPROVED_REPORT_CELL)
  })
  return {
    calls,
    publishChannel: () => {
      if (!publishChannelRef) throw new Error('artifact_publish was not called')
      return publishChannelRef
    },
  }
}

function publishButton(name = REPORT_NAME): HTMLButtonElement {
  return within(candidateRow(name)).getByRole('button', { name: 'Publish' }) as HTMLButtonElement
}

/** Every transcript entry, as text, in order. */
function transcriptTexts(): string[] {
  return Array.from(screen.getByLabelText('Agent transcript').querySelectorAll('li')).map(
    (entry) => entry.textContent ?? '',
  )
}

/** Clicks Publish and waits for the one `artifact_publish` call. */
async function clickPublish(calls: { cmd: string }[]): Promise<void> {
  fireEvent.click(publishButton())
  await waitFor(() => {
    expect(calls.filter((call) => call.cmd === 'artifact_publish')).toHaveLength(1)
  })
}

describe('AgentPanel publish (slice 5b, HAP-001-R35)', () => {
  it('Publish calls artifact_publish with exactly { approvalId, onState } -- the approval id, a live Channel -- renders each artifact-state frame as the transcript line "publication <short id>: <state>" in order, and on resolution shows the row as "<state> <short id>" with no button and the publication as a Publications row', async () => {
    let resolvePublish: (publication: Publication) => void = () => {}
    const { calls, publishChannel } = await approveReport(
      () =>
        new Promise<Publication>((resolve) => {
          resolvePublish = resolve
        }),
    )
    const linesBefore = transcriptTexts()

    await clickPublish(calls)

    const publishCall = calls.find((call) => call.cmd === 'artifact_publish')!
    expect(Object.keys(publishCall.args).sort()).toEqual(['approvalId', 'onState'])
    expect(publishCall.args.approvalId).toBe('23121521465ad9be')
    expect(typeof (publishCall.args.onState as LiveStateChannel).onmessage).toBe('function')
    for (const key of Object.keys(publishCall.args)) {
      expect(key.toLowerCase()).not.toContain('path')
    }

    deliverArtifactState(publishChannel(), 'published-local')
    deliverArtifactState(publishChannel(), 'registered', 'pending')

    expect(transcriptTexts()).toEqual([
      ...linesBefore,
      'publication 2a91ea59: published-local',
      'publication 2a91ea59: registered',
    ])
    expect(screen.getByLabelText('Agent transcript').textContent).not.toContain(PUBLICATION_ID)

    await act(async () => {
      resolvePublish(PUBLISHED_REPORT)
      await new Promise((resolve) => {
        setTimeout(resolve, 0)
      })
    })

    expect(actionCell(REPORT_NAME)).toBe('registered 2a91ea59')
    expect(within(candidateRow(REPORT_NAME)).queryByRole('button')).toBeNull()
    await screen.findByRole('region', { name: 'Publications' })
    expect(publicationRows()).toEqual([
      ['report.pdf', 'registered', 'pending', CATALOG_ID, 'main/2a91ea59', 'local'],
    ])
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.queryByLabelText('Publication approval')).toBeNull()
  })

  it('a registration-pending result shows the row as "registration-pending <short id>" and a Publications row with "—" for the missing record facts, after its published-local line', async () => {
    let resolvePublish: (publication: Publication) => void = () => {}
    const { calls, publishChannel } = await approveReport(
      () =>
        new Promise<Publication>((resolve) => {
          resolvePublish = resolve
        }),
    )

    await clickPublish(calls)
    deliverArtifactState(publishChannel(), 'published-local')
    await act(async () => {
      resolvePublish(PENDING_REPORT)
      await new Promise((resolve) => {
        setTimeout(resolve, 0)
      })
    })

    expect(transcriptTexts().at(-1)).toBe('publication 2a91ea59: published-local')
    expect(actionCell(REPORT_NAME)).toBe('registration-pending 2a91ea59')
    await screen.findByRole('region', { name: 'Publications' })
    expect(publicationRows()).toEqual([
      ['report.pdf', 'registration-pending', '—', '—', '—', 'local'],
    ])
  })

  it("renders duplicate-publication from artifact_publish with its message and only the detail's two short ids -- never the full identities -- and no \"untrusted\"; the row keeps its approval with Publish enabled again, and the state frame's line stands", async () => {
    let rejectPublish: (reason: unknown) => void = () => {}
    const { calls, publishChannel } = await approveReport(
      () =>
        new Promise<Publication>((_resolve, reject) => {
          rejectPublish = reject
        }),
    )

    await clickPublish(calls)
    deliverArtifactState(publishChannel(), 'duplicate-publication', 'pending')
    await act(async () => {
      rejectPublish({
        code: 'duplicate-publication',
        message: 'an artifact with this content is already registered for this project',
        detail: { publicationId: PUBLICATION_ID, catalogId: CATALOG_ID },
      })
      await new Promise((resolve) => {
        setTimeout(resolve, 0)
      })
    })

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe(
      'duplicate-publication: an artifact with this content is already registered for this project (publication 2a91ea59, catalog main/2a91ea59)',
    )
    expect(banner.textContent).not.toContain(PUBLICATION_ID)
    expect(banner.textContent).not.toContain('untrusted')
    expect(transcriptTexts().at(-1)).toBe('publication 2a91ea59: duplicate-publication')
    expect(actionCell(REPORT_NAME)).toBe(APPROVED_REPORT_CELL)
    expect(publishButton().disabled).toBe(false)
    expect(screen.queryByRole('region', { name: 'Publications' })).toBeNull()
  })

  it.each([
    [
      'integrity-mismatch',
      'the published copy did not verify against the approved digest; it was discarded and the entry preserved',
      'integrity-mismatch',
    ],
    [
      'outbox-escape',
      "the entry's path no longer names the approved file; the approved bytes were kept as a recovery entry",
      'outbox-escape',
    ],
    ['outbox-linked', "the entry's link count is greater than one", 'outbox-linked'],
    ['refused', "the entry's identity facts changed since it was approved", 'refused'],
    ['catalog-unavailable', 'the catalog is corrupt', null],
    [
      'destination-invalid',
      'the device asset path resolves inside a registered workspace root',
      null,
    ],
    [
      'work-area-invalid',
      'the product work area resolves inside a registered workspace root',
      null,
    ],
    ['invalid-request', 'no approval with that id is recorded', null],
    ['workspace-unavailable', 'no workspace has been picked yet', null],
    // The backend mirror of the frozen surface (`docs/spike-log.md` § Slice 5b).
    ['run-active', 'a run is active; approve or publish once it has ended', null],
  ] as const)(
    'renders %s from artifact_publish as a plain catalogue code with its fixed message, no "untrusted" and no stray detail; the row keeps its approval with Publish enabled again',
    async (code, message, state) => {
      let rejectPublish: (reason: unknown) => void = () => {}
      const { calls, publishChannel } = await approveReport(
        () =>
          new Promise<Publication>((_resolve, reject) => {
            rejectPublish = reject
          }),
      )

      await clickPublish(calls)
      if (state !== null) deliverArtifactState(publishChannel(), state)
      await act(async () => {
        rejectPublish({
          code,
          message,
          detail: { recordedSha256Short: 'aaaaaaaa', observedSha256Short: 'bbbbbbbb' },
        })
        await new Promise((resolve) => {
          setTimeout(resolve, 0)
        })
      })

      const banner = await screen.findByRole('alert')
      expect(banner.textContent).toBe(`${code}: ${message}`)
      expect(banner.textContent).not.toContain('untrusted')
      expect(banner.textContent).not.toContain('aaaaaaaa')
      if (state !== null) {
        expect(transcriptTexts().at(-1)).toBe(`publication 2a91ea59: ${state}`)
      }
      expect(actionCell(REPORT_NAME)).toBe(APPROVED_REPORT_CELL)
      expect(publishButton().disabled).toBe(false)
    },
  )

  it('shows only "unexpected error" when artifact_publish rejects with a plain Error, never its message; the row keeps its approval with Publish enabled again', async () => {
    const { calls } = await approveReport(() =>
      Promise.reject(new Error('asset root path /secret/asset-roots/main')),
    )

    await clickPublish(calls)

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('unexpected error')
    expect(document.body.textContent).not.toContain('/secret/asset-roots/main')
    expect(actionCell(REPORT_NAME)).toBe(APPROVED_REPORT_CELL)
    await waitFor(() => {
      expect(publishButton().disabled).toBe(false)
    })
  })

  it('disables Publish while the publish is in flight, so a double-click fires one artifact_publish', async () => {
    let resolvePublish: (publication: Publication) => void = () => {}
    const { calls } = await approveReport(
      () =>
        new Promise<Publication>((resolve) => {
          resolvePublish = resolve
        }),
    )

    fireEvent.click(publishButton())
    fireEvent.click(publishButton())
    await waitFor(() => {
      expect(publishButton().disabled).toBe(true)
    })
    fireEvent.click(publishButton())
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(calls.filter((call) => call.cmd === 'artifact_publish')).toHaveLength(1)

    await act(async () => {
      resolvePublish(PUBLISHED_REPORT)
      await new Promise((resolve) => {
        setTimeout(resolve, 0)
      })
    })
    expect(actionCell(REPORT_NAME)).toBe('registered 2a91ea59')
  })

  it("drops publish frames that arrive after a new Start (generation guard): no publication line enters the new run's transcript, while the resolved publication still reaches the Publications table, the project's own truth", async () => {
    let resolvePublish: (publication: Publication) => void = () => {}
    const { calls, publishChannel } = await approveReport(
      () =>
        new Promise<Publication>((resolve) => {
          resolvePublish = resolve
        }),
    )
    await clickPublish(calls)

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    expect(screen.queryByRole('region', { name: 'Candidates' })).toBeNull()
    const linesBefore = transcriptTexts()

    deliverArtifactState(publishChannel(), 'published-local')
    deliverArtifactState(publishChannel(), 'registered', 'pending')
    expect(transcriptTexts()).toEqual(linesBefore)

    await act(async () => {
      resolvePublish(PUBLISHED_REPORT)
      await new Promise((resolve) => {
        setTimeout(resolve, 0)
      })
    })

    expect(transcriptTexts()).toEqual(linesBefore)
    await screen.findByRole('region', { name: 'Publications' })
    expect(publicationRows()).toHaveLength(1)
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it("a rejected Start restores the candidates table with its approvals and its open approval block intact, since no run replaced the inventory (R3-019 extended to slice 5b's row state)", async () => {
    let spawnCount = 0
    let channelRef: LiveChannel | undefined
    await renderReady((cmd, args) => {
      if (cmd === 'harness_spawn') {
        spawnCount += 1
        if (spawnCount === 1) {
          channelRef = (args as { onFrame: LiveChannel }).onFrame
          return 7
        }
        return Promise.reject({
          code: 'spawn-failed',
          message: 'failed to start the requested process',
        })
      }
      if (cmd === 'candidates_list') return SAMPLE_CANDIDATES
      if (cmd === 'artifact_approve') return SAMPLE_ARTIFACT_APPROVAL
      throw new Error(`unexpected command: ${cmd}`)
    }, 'claude-code', { outbox: OUTBOX_STATUS_VALID })
    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    await screen.findByText('running')
    if (!channelRef) throw new Error('harness_spawn was not called')
    deliverCandidates(channelRef)
    await screen.findByRole('region', { name: 'Candidates' })
    await endRun(channelRef)
    await openApproval(REPORT_NAME)
    confirmReportApproval()
    await waitFor(() => {
      expect(actionCell(REPORT_NAME)).toBe(APPROVED_REPORT_CELL)
    })
    await openApproval(`${RUN_ID}/stray.png`)

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('spawn-failed: failed to start the requested process')
    expect(spawnCount).toBe(2)
    expect(actionCell(REPORT_NAME)).toBe(APPROVED_REPORT_CELL)
    expect(publishButton().disabled).toBe(false)
    const block = screen.getByLabelText('Publication approval')
    expect(block.textContent).toContain(`name: ${RUN_ID}/stray.png`)
  })

  it('the publish continuation no-ops when artifact_publish rejects after unmount: nothing thrown, no console.error, no alert (R3-022)', async () => {
    let rejectPublish: (reason: unknown) => void = () => {}
    const { calls } = await approveReport(
      () =>
        new Promise<Publication>((_resolve, reject) => {
          rejectPublish = reject
        }),
    )
    await clickPublish(calls)

    cleanup()

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    rejectPublish({
      code: 'integrity-mismatch',
      message:
        'the published copy did not verify against the approved digest; it was discarded and the entry preserved',
    })
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    expect(document.body.querySelector('[role="alert"]')).toBeNull()
    consoleErrorSpy.mockRestore()
  })

  /**
   * Answers `artifact_approve` and `artifact_publish` for both approvable
   * fixture rows, by name and by approval id respectively.
   */
  function twoRowHandlers(cmd: string, args: Record<string, unknown>): unknown {
    if (cmd === 'artifact_approve') {
      return (args as { name: string }).name === REPORT_NAME
        ? SAMPLE_ARTIFACT_APPROVAL
        : STRAY_ARTIFACT_APPROVAL
    }
    if (cmd === 'artifact_publish') {
      return (args as { approvalId: string }).approvalId === SAMPLE_ARTIFACT_APPROVAL.approvalId
        ? PUBLISHED_REPORT
        : PUBLISHED_STRAY
    }
    throw new Error(`unexpected command: ${cmd}`)
  }

  /** Approves and publishes the row named `name`, waiting for each cell in turn. */
  async function approveAndPublish(
    name: string,
    short: string,
    approvedCell: string,
    publishedCell: string,
  ): Promise<void> {
    await openApproval(name)
    typeDigest(short)
    fireEvent.click(confirmButton())
    await waitFor(() => {
      expect(actionCell(name)).toBe(approvedCell)
    })
    fireEvent.click(publishButton(name))
    await waitFor(() => {
      expect(actionCell(name)).toBe(publishedCell)
    })
  }

  it("two rows, report then stray: the first row's published cell and Publications entry survive the second row's approve-and-publish cycle, and both rows end published with two Publications rows (R3-020)", async () => {
    await startInventoryAndEndRun(twoRowHandlers)

    await approveAndPublish(REPORT_NAME, REPORT_SHORT, APPROVED_REPORT_CELL, 'registered 2a91ea59')
    expect(publicationRows()).toEqual([PUBLISHED_REPORT_ROW])
    await approveAndPublish(STRAY_NAME, '11111111', APPROVED_STRAY_CELL, 'registered cccccccc')

    expect(actionCell(REPORT_NAME)).toBe('registered 2a91ea59')
    expect(actionCell(STRAY_NAME)).toBe('registered cccccccc')
    expect(publicationRows()).toEqual([PUBLISHED_REPORT_ROW, PUBLISHED_STRAY_ROW])
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.queryByLabelText('Publication approval')).toBeNull()
  })

  it("two rows, stray then report: the reverse order leaves both rows published, the first row's state intact through the second cycle, and the Publications rows in publication order (R3-020)", async () => {
    await startInventoryAndEndRun(twoRowHandlers)

    await approveAndPublish(STRAY_NAME, '11111111', APPROVED_STRAY_CELL, 'registered cccccccc')
    expect(publicationRows()).toEqual([PUBLISHED_STRAY_ROW])
    await approveAndPublish(REPORT_NAME, REPORT_SHORT, APPROVED_REPORT_CELL, 'registered 2a91ea59')

    expect(actionCell(STRAY_NAME)).toBe('registered cccccccc')
    expect(actionCell(REPORT_NAME)).toBe('registered 2a91ea59')
    expect(publicationRows()).toEqual([PUBLISHED_STRAY_ROW, PUBLISHED_REPORT_ROW])
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('the publish continuations no-op after unmount: a frame and the resolution after unmount throw nothing and log no console.error', async () => {
    let resolvePublish: (publication: Publication) => void = () => {}
    const { calls, publishChannel } = await approveReport(
      () =>
        new Promise<Publication>((resolve) => {
          resolvePublish = resolve
        }),
    )
    await clickPublish(calls)
    const channel = publishChannel()

    cleanup()

    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    channel.onmessage({
      kind: 'artifact-state',
      payload: { publicationId: PUBLICATION_ID, state: 'published-local', providerState: null },
    })
    resolvePublish(PUBLISHED_REPORT)
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })

    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })
})
