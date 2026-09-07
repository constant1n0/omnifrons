import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { AgentPanel } from './AgentPanel'
import { ApprovalSurface } from './ApprovalSurface'
import { HarnessPanel } from './HarnessPanel'
import type { AdapterDescriptor, Approval, Evidence, HarnessFrame } from './ipc/harness'

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
 * The default mock: no active workspace, two adapters (the slice 3 line
 * agent and the slice 4 pseudo-terminal fallback), one active approval.
 * Individual tests override `onCommand` to add `harness_spawn`/
 * `workspace_pick` handling.
 */
function defaultHandlers(onCommand?: (cmd: string, args: Record<string, unknown>) => unknown) {
  return (cmd: string, args: unknown) => {
    if (cmd === 'workspace_current') return null
    if (cmd === 'adapters_list') return [SAMPLE_ADAPTER, PTY_ADAPTER]
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
  adapterId: 'claude-code' | 'pty-cli' = 'claude-code',
) {
  mockIPC(defaultHandlers(onCommand))
  render(<AgentPanel />)
  await selectOption('Adapter', adapterId)
  await selectOption('Approval', '42')
  fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: 'do the thing' } })
}

/** Fills in the required selects, starts a run, and returns the live Channel. */
async function startAndCaptureChannel(
  onCommand?: (cmd: string, args: Record<string, unknown>) => unknown,
  adapterId: 'claude-code' | 'pty-cli' = 'claude-code',
): Promise<{ onmessage: (frame: HarnessFrame) => void }> {
  let channelRef: { onmessage: (frame: HarnessFrame) => void } | undefined
  await renderReady((cmd, args) => {
    if (cmd === 'harness_spawn') {
      channelRef = (args as { onFrame: { onmessage: (frame: HarnessFrame) => void } }).onFrame
      return 7
    }
    if (onCommand) return onCommand(cmd, args)
    throw new Error(`unexpected command: ${cmd}`)
  }, adapterId)

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
      if (cmd === 'workspace_current') return { displayPath: '/home/user/project' }
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
        if (cmd === 'workspace_pick') return { displayPath: '/home/user/project' }
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
    resolvePick({ displayPath: '/home/user/project' })
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
  adapterId: 'claude-code' | 'pty-cli' = 'claude-code',
): Promise<{ channels: LiveChannel[]; spawnCount: () => number }> {
  const channels: LiveChannel[] = []
  let nextId = 7
  await renderReady((cmd, args) => {
    if (cmd === 'harness_spawn') {
      channels.push((args as { onFrame: LiveChannel }).onFrame)
      const id = nextId
      nextId += 1
      return id
    }
    if (onCommand) return onCommand(cmd, args)
    if (cmd === 'harness_stop') return { state: 'killed', code: null }
    throw new Error(`unexpected command: ${cmd}`)
  }, adapterId)
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
