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
  GuidanceApplied,
  GuidancePreview,
  GuidanceStatus,
  HarnessFrame,
  MisplacedRow,
  OutboxReason,
  OutboxStatus,
  OutputDiscipline,
  Publication,
  Remedy,
  RemedyDetail,
  RemedyOutcome,
  ScanSummary,
  ScopeMode,
  Snapshot,
  Workspace,
  WrongRootReason,
  WrongRootStatus,
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

    const badge = screen.getByText('advisory scope — not a sandbox')
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
  handleHeld: true,
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
  handleHeld: true,
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

// -- Slice 5c: the whole-outbox listing and the unattributed approval --

type Call = { cmd: string; args: Record<string, unknown> }

/** The active workspace of the slice 5c mounts: the shape `workspace_current` returns since slice 5b. */
const WORKSPACE_ACTIVE: Workspace = { displayPath: '/home/user/project', workArea: 'valid' }

const TEMPLATE_VERSION = 'hap-001-guidance-v1'

/**
 * A fresh project's guidance answers (`docs/spike-log.md` § Slice 5c, IPC
 * shapes): both managed files absent, no snapshots -- what every slice 5c
 * mount over a workspace answers unless a test says otherwise. `undefined`
 * for any other command, so a caller can fall through.
 */
function answerGuidanceFresh(cmd: string, args: Record<string, unknown>): unknown {
  if (cmd === 'guidance_status') {
    const status: GuidanceStatus = {
      kind: args.kind as GuidanceStatus['kind'],
      file: args.kind === 'ignore' ? '.gitignore' : ((args.file as string | undefined) ?? 'AGENTS.md'),
      exists: false,
      managed: 'absent',
      templateVersion: TEMPLATE_VERSION,
      fileSha256: null,
      fileSha256Short: null,
      snapshots: 0,
      pinned: 0,
    }
    return status
  }
  if (cmd === 'guidance_snapshots') return []
  return undefined
}

/**
 * A fresh project's wrong-root answers (`docs/spike-log.md` § Slice 5d, IPC
 * shapes): the advisory report every built-in adapter's scope mode
 * produces, both of its disclosures, no scan run and nothing standing.
 * `undefined` for any other command, so a caller can fall through.
 */
function answerWrongRootFresh(cmd: string): unknown {
  if (cmd === 'wrongroot_status') {
    const status: WrongRootStatus = {
      outputDiscipline: 'advisory',
      scopeMode: 'advisory',
      disclosures: [
        'a write inside the project but outside the outbox is detected after the run, never prevented',
        'a write outside the project is possible and is detected after the run, not prevented',
      ],
      scanned: false,
      findings: 0,
    }
    return status
  }
  if (cmd === 'misplaced_list') return []
  return undefined
}

/**
 * Mounts the panel over an active workspace with a valid outbox whose
 * policy declares the asset root `main`, an empty Catalog, and a fresh
 * project's guidance and wrong-root answers. `onCommand` answers everything else -- a
 * `harness_spawn`, a `candidates_list`, an approval -- and returns
 * `undefined` to fall through to the defaults. Every IPC call is recorded,
 * mount-time ones included.
 */
function mountWithWorkspace(
  onCommand?: (cmd: string, args: Record<string, unknown>) => unknown,
  options: { outbox?: OutboxStatus; workspace?: Workspace | null } = {},
): Call[] {
  const calls: Call[] = []
  mockIPC((cmd, rawArgs) => {
    const args = (rawArgs ?? {}) as Record<string, unknown>
    calls.push({ cmd, args })
    if (onCommand) {
      const answer = onCommand(cmd, args)
      if (answer !== undefined) return answer
    }
    if (cmd === 'workspace_current') {
      return options.workspace === undefined ? WORKSPACE_ACTIVE : options.workspace
    }
    if (cmd === 'adapters_list') return [SAMPLE_ADAPTER, PTY_ADAPTER]
    if (cmd === 'approvals_list') return [sampleApproval({ approvalId: 42 })]
    if (cmd === 'outbox_status') return options.outbox ?? OUTBOX_STATUS_VALID
    if (cmd === 'publications_list') return []
    const guidance = answerGuidanceFresh(cmd, args)
    if (guidance !== undefined) return guidance
    const wrongRoot = answerWrongRootFresh(cmd)
    if (wrongRoot !== undefined) return wrongRoot
    throw new Error(`unexpected command: ${cmd}`)
  })
  render(<AgentPanel />)
  return calls
}

/** Flushes the pending microtasks and a macrotask inside `act`, so every settled continuation has run. */
async function flush(): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })
  })
}

const DROPPED_NAME = 'dropped.pdf'
const DROPPED_SHA256 = `9d3f2a10${'e'.repeat(56)}`
const DROPPED_SHORT = '9d3f2a10'
const FORGOTTEN_STRAY_NAME = 'run-old/stray.png'

/**
 * `candidates_list {}` -- the whole outbox (`docs/spike-log.md` § Slice 5c,
 * IPC shapes): an entry at the outbox root, one under a run subdirectory
 * the shell no longer remembers (a location fact the name carries, never
 * provenance), and a Markdown note the policy classes as portable text;
 * every one unattributed, since no run's own proposal named it.
 */
const OUTBOX_CANDIDATES: Candidate[] = [
  {
    name: DROPPED_NAME,
    size: 15,
    sha256: DROPPED_SHA256,
    sha256Short: DROPPED_SHORT,
    detectedType: 'pdf',
    class: 'generated-heavy',
    attribution: { kind: 'unattributed' },
    state: 'candidate',
  },
  {
    name: FORGOTTEN_STRAY_NAME,
    size: 8,
    sha256: STRAY_SHA256,
    sha256Short: '11111111',
    detectedType: 'png',
    class: 'generated-heavy',
    attribution: { kind: 'unattributed' },
    state: 'candidate',
  },
  {
    name: 'notes.md',
    size: 40,
    sha256: 'f'.repeat(64),
    sha256Short: 'ffffffff',
    detectedType: 'markdown',
    class: 'portable-text',
    attribution: { kind: 'unattributed' },
    state: 'candidate',
  },
]

/**
 * `artifact_approve { runId: null, … }`'s payload for the root entry
 * (`docs/spike-log.md` § Slice 5c, IPC shapes): no run, the unattributed
 * fact, nothing standing in for a producer, the re-opened handle held.
 */
const DROPPED_APPROVAL: ArtifactApproval = {
  approvalId: '5f0c3b2a9e1d7c44',
  publicationId: 'b'.repeat(64),
  runId: null,
  name: DROPPED_NAME,
  displayName: 'dropped.pdf',
  sha256Short: DROPPED_SHORT,
  size: 15,
  detectedType: 'pdf',
  class: 'generated-heavy',
  attribution: { kind: 'unattributed' },
  assetRootId: 'main',
  actAs: 'device-local-user',
  approvedAt: 1725782401000,
  handleHeld: true,
}

const APPROVED_DROPPED_CELL = 'approved 5f0c3b2a9e1d7c44 — dropped.pdf, asset root main Publish'
const NOT_HELD_MARK = 'handle: not held (cap reached)'

function listOutboxButton(): HTMLButtonElement {
  return screen.getByRole('button', { name: 'List outbox' }) as HTMLButtonElement
}

/**
 * Mounts over a workspace whose whole-outbox inventory is
 * `OUTBOX_CANDIDATES`, waits for List outbox to be live, clicks it, and
 * waits for the table. `onCommand` answers the approval commands a test
 * drives.
 */
async function listOutbox(
  onCommand?: (cmd: string, args: Record<string, unknown>) => unknown,
): Promise<Call[]> {
  const calls = mountWithWorkspace((cmd, args) => {
    if (cmd === 'candidates_list') return OUTBOX_CANDIDATES
    return onCommand?.(cmd, args)
  })
  await waitFor(() => {
    expect(listOutboxButton().disabled).toBe(false)
  })
  fireEvent.click(listOutboxButton())
  await screen.findByRole('region', { name: 'Candidates' })
  return calls
}

/** Selects the adapter and approval, fills the prompt, and clicks Start on a panel `mountWithWorkspace` mounted. */
async function startRunOverWorkspace(): Promise<void> {
  await selectOption('Adapter', 'claude-code')
  await selectOption('Approval', '42')
  fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: 'do the thing' } })
  fireEvent.click(screen.getByRole('button', { name: 'Start' }))
  await screen.findByText('running')
}

describe('AgentPanel whole-outbox listing (slice 5c, HAP-001-R11, R12)', () => {
  it('renders a "List outbox" button beside the outbox line: disabled while no workspace is active, enabled once one is, and invoking nothing until clicked', async () => {
    mockIPC(defaultHandlers())
    render(<AgentPanel />)
    await screen.findByLabelText('Adapter')
    expect(listOutboxButton().disabled).toBe(true)
    cleanup()
    clearMocks()

    const calls = mountWithWorkspace()
    await waitFor(() => {
      expect(listOutboxButton().disabled).toBe(false)
    })
    expect(calls.filter((call) => call.cmd === 'candidates_list')).toHaveLength(0)
    expect(screen.queryByRole('region', { name: 'Candidates' })).toBeNull()
  })

  it('clicking List outbox calls candidates_list with exactly {} -- no runId key -- and renders the Candidates table with the whole-outbox rows: a root entry, an entry under a forgotten run, every row unattributed, Approve only on the generated-heavy rows, and no summary line in the transcript', async () => {
    const calls = await listOutbox()

    const listings = calls.filter((call) => call.cmd === 'candidates_list')
    expect(listings).toHaveLength(1)
    expect(listings[0]?.args).toEqual({})
    expect(Object.keys(listings[0]!.args)).toEqual([])
    expect(candidateRows()).toEqual([
      [DROPPED_NAME, '15', DROPPED_SHORT, 'pdf', 'generated-heavy', 'unattributed', 'candidate', 'Approve'],
      [FORGOTTEN_STRAY_NAME, '8', '11111111', 'png', 'generated-heavy', 'unattributed', 'candidate', 'Approve'],
      ['notes.md', '40', 'ffffffff', 'markdown', 'portable-text', 'unattributed', 'candidate', ''],
    ])
    expect(screen.getByLabelText('Agent transcript').querySelectorAll('li')).toHaveLength(0)
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('Approve on a whole-outbox row opens the block with the row\'s facts, "attribution: unattributed", the destination from the outbox status, "scope: none" (no run, so no adapter), and the act-as line; the gate is the row\'s short digest, and the final button calls artifact_approve with exactly { runId: null, name, sha256 } -- the key present and null, the row\'s own full digest, never the typed value -- then the row shows the approved cell with Publish and the block closes', async () => {
    const calls = await listOutbox((cmd) => {
      if (cmd === 'artifact_approve') return DROPPED_APPROVAL
      return undefined
    })

    const block = await openApproval(DROPPED_NAME)

    expect(Array.from(block.querySelectorAll('p')).map((line) => line.textContent)).toEqual([
      `name: ${DROPPED_NAME}`,
      'class: generated-heavy',
      'type: pdf',
      'size: 15',
      `sha256: ${DROPPED_SHA256}`,
      `sha256 short: ${DROPPED_SHORT}`,
      'attribution: unattributed',
      'destination: asset root main',
      'scope: none',
      'act as: device-local-user',
    ])
    expect(confirmButton().disabled).toBe(true)
    typeDigest(DROPPED_SHA256)
    expect(confirmButton().disabled).toBe(true)
    typeDigest(DROPPED_SHORT)
    expect(confirmButton().disabled).toBe(false)
    fireEvent.click(confirmButton())

    await waitFor(() => {
      expect(actionCell(DROPPED_NAME)).toBe(APPROVED_DROPPED_CELL)
    })
    const approvals = calls.filter((call) => call.cmd === 'artifact_approve')
    expect(approvals).toHaveLength(1)
    expect(approvals[0]?.args).toEqual({ runId: null, name: DROPPED_NAME, sha256: DROPPED_SHA256 })
    expect(Object.keys(approvals[0]!.args).sort()).toEqual(['name', 'runId', 'sha256'])
    expect(approvals[0]?.args.runId).toBeNull()
    expect(approvals[0]?.args.sha256).toHaveLength(64)
    expect(screen.queryByLabelText('Publication approval')).toBeNull()
    expect(candidateRow(DROPPED_NAME).querySelector('mark')).toBeNull()
    expect(within(candidateRow(DROPPED_NAME)).getByRole('button', { name: 'Publish' })).toBeTruthy()
    expect(actionCell(FORGOTTEN_STRAY_NAME)).toBe('Approve')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('an entry under a forgotten run approves the same way, with runId null and its "<run id>/<file>" name sent exactly as listed -- the subdirectory a location fact, never a run id', async () => {
    const calls = await listOutbox((cmd) => {
      if (cmd === 'artifact_approve') {
        return {
          ...DROPPED_APPROVAL,
          approvalId: '6a6a6a6a6a6a6a6a',
          name: FORGOTTEN_STRAY_NAME,
          displayName: 'stray.png',
          sha256Short: '11111111',
          size: 8,
          detectedType: 'png',
        }
      }
      return undefined
    })

    const block = await openApproval(FORGOTTEN_STRAY_NAME)
    const lines = Array.from(block.querySelectorAll('p')).map((line) => line.textContent)
    expect(lines).toContain(`name: ${FORGOTTEN_STRAY_NAME}`)
    expect(lines).toContain('attribution: unattributed')
    expect(lines).toContain('scope: none')
    expect(lines.some((line) => line?.includes('run-old') && !line.startsWith('name:'))).toBe(false)
    typeDigest('11111111')
    fireEvent.click(confirmButton())

    await waitFor(() => {
      expect(actionCell(FORGOTTEN_STRAY_NAME)).toBe(
        'approved 6a6a6a6a6a6a6a6a — stray.png, asset root main Publish',
      )
    })
    expect(calls.filter((call) => call.cmd === 'artifact_approve')[0]?.args).toEqual({
      runId: null,
      name: FORGOTTEN_STRAY_NAME,
      sha256: STRAY_SHA256,
    })
  })

  it('renders "handle: not held (cap reached)" in a <mark> beside the approved cell when the approval reports handleHeld false, with Publish still offered (the publication re-opens the entry); nothing of the sort for handleHeld true', async () => {
    await listOutbox((cmd) => {
      if (cmd === 'artifact_approve') return { ...DROPPED_APPROVAL, handleHeld: false }
      return undefined
    })
    await openApproval(DROPPED_NAME)
    typeDigest(DROPPED_SHORT)
    fireEvent.click(confirmButton())

    await waitFor(() => {
      expect(actionCell(DROPPED_NAME)).toBe(
        `approved 5f0c3b2a9e1d7c44 — dropped.pdf, asset root main ${NOT_HELD_MARK} Publish`,
      )
    })
    const row = candidateRow(DROPPED_NAME)
    expect(row.querySelector('mark')?.textContent).toBe(NOT_HELD_MARK)
    expect(within(row).getByRole('button', { name: 'Publish' }).hasAttribute('disabled')).toBe(
      false,
    )
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it.each([
    {
      code: 'integrity-mismatch',
      message:
        "the entry's digest differs from the digest the request names; it changed since it was listed",
    },
    { code: 'outbox-escape', message: 'the entry is not a regular file inside the outbox' },
    { code: 'outbox-linked', message: "the entry's link count is greater than one" },
    {
      code: 'invalid-request',
      message: 'the entry belongs to a remembered run; approve it through that run id',
    },
  ])(
    'renders $code from a whole-outbox artifact_approve as a plain catalogue code with the shell\'s fixed message, no "untrusted"; the row keeps Approve, the block stays open with the typed digest kept and the final button enabled again',
    async ({ code, message }) => {
      await listOutbox((cmd) => {
        if (cmd === 'artifact_approve') return Promise.reject({ code, message })
        return undefined
      })
      await openApproval(DROPPED_NAME)
      typeDigest(DROPPED_SHORT)
      fireEvent.click(confirmButton())

      const alert = await screen.findByRole('alert')
      expect(alert.textContent).toBe(`${code}: ${message}`)
      expect(alert.textContent).not.toContain('untrusted')
      expect(actionCell(DROPPED_NAME)).toBe('Approve')
      expect(screen.getByLabelText('Publication approval')).toBeTruthy()
      expect(approvalInput().value).toBe(DROPPED_SHORT)
      await waitFor(() => {
        expect(confirmButton().disabled).toBe(false)
      })
    },
  )

  it('List outbox is disabled while a run is active and enabled once it ends, and a listing response landing after a Start is dropped: the run starts with no table, and its own inventory is applied normally afterwards', async () => {
    let resolveListing: (rows: Candidate[]) => void = () => {}
    let channel: LiveChannel | undefined
    const calls = mountWithWorkspace((cmd, args) => {
      if (cmd === 'harness_spawn') {
        channel = (args as { onFrame: LiveChannel }).onFrame
        return 7
      }
      if (cmd === 'candidates_list') {
        if (Object.keys(args).length === 0) {
          return new Promise<Candidate[]>((resolve) => {
            resolveListing = resolve
          })
        }
        return SAMPLE_CANDIDATES
      }
      return undefined
    })
    await waitFor(() => {
      expect(listOutboxButton().disabled).toBe(false)
    })
    fireEvent.click(listOutboxButton())
    await waitFor(() => {
      expect(calls.filter((call) => call.cmd === 'candidates_list')).toHaveLength(1)
    })

    await startRunOverWorkspace()
    expect(listOutboxButton().disabled).toBe(true)
    resolveListing(OUTBOX_CANDIDATES)
    await flush()
    expect(screen.queryByRole('region', { name: 'Candidates' })).toBeNull()

    if (!channel) throw new Error('harness_spawn was not called')
    deliverCandidates(channel)
    await screen.findByRole('region', { name: 'Candidates' })
    expect(candidateRows().map((row) => row[0])).toEqual(SAMPLE_CANDIDATES.map((row) => row.name))
    await endRun(channel)
    expect(listOutboxButton().disabled).toBe(false)
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('a second List outbox replaces the rows and clears the approvals and the open block: a new inventory', async () => {
    let listing = 0
    await listOutbox((cmd) => {
      if (cmd === 'artifact_approve') return DROPPED_APPROVAL
      return undefined
    })
    await openApproval(DROPPED_NAME)
    typeDigest(DROPPED_SHORT)
    fireEvent.click(confirmButton())
    await waitFor(() => {
      expect(actionCell(DROPPED_NAME)).toBe(APPROVED_DROPPED_CELL)
    })
    await openApproval(FORGOTTEN_STRAY_NAME)
    clearMocks()
    mockIPC((cmd) => {
      if (cmd === 'candidates_list') {
        listing += 1
        return [OUTBOX_CANDIDATES[0]!]
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    fireEvent.click(listOutboxButton())

    await waitFor(() => {
      expect(candidateRows()).toHaveLength(1)
    })
    expect(listing).toBe(1)
    expect(actionCell(DROPPED_NAME)).toBe('Approve')
    expect(screen.queryByLabelText('Publication approval')).toBeNull()
  })

  it('a listing rejection reaches the banner with no table (outbox-invalid, the shell\'s fixed message), and a rejection landing after unmount raises no console.error and renders nothing', async () => {
    mountWithWorkspace((cmd) => {
      if (cmd === 'candidates_list') {
        return Promise.reject({
          code: 'outbox-invalid',
          message: 'the classification policy could not be loaded',
        })
      }
      return undefined
    })
    await waitFor(() => {
      expect(listOutboxButton().disabled).toBe(false)
    })
    fireEvent.click(listOutboxButton())
    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toBe('outbox-invalid: the classification policy could not be loaded')
    expect(screen.queryByRole('region', { name: 'Candidates' })).toBeNull()
    cleanup()
    clearMocks()

    let rejectListing: (error: unknown) => void = () => {}
    mountWithWorkspace((cmd) => {
      if (cmd === 'candidates_list') {
        return new Promise((_resolve, reject) => {
          rejectListing = reject
        })
      }
      return undefined
    })
    await waitFor(() => {
      expect(listOutboxButton().disabled).toBe(false)
    })
    fireEvent.click(listOutboxButton())
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    cleanup()
    rejectListing({ code: 'outbox-invalid', message: 'the classification policy could not be loaded' })
    await new Promise((resolve) => {
      setTimeout(resolve, 0)
    })
    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })
})

// -- Slice 5c: the guidance-note installer --

const FILE_DIGEST = `3c1e9a07${'a'.repeat(56)}`
const FILE_SHORT = '3c1e9a07'
const IGNORE_DIGEST = `a71bc0d2${'c'.repeat(56)}`
const IGNORE_SHORT = 'a71bc0d2'

/** `guidance_status { kind: "guidance" }` with the note installed and current, one snapshot on record. */
function noteStatus(overrides: Partial<GuidanceStatus> = {}): GuidanceStatus {
  return {
    kind: 'guidance',
    file: 'AGENTS.md',
    exists: true,
    managed: 'current',
    templateVersion: TEMPLATE_VERSION,
    fileSha256: FILE_DIGEST,
    fileSha256Short: FILE_SHORT,
    snapshots: 1,
    pinned: 0,
    ...overrides,
  }
}

/** The fresh project's note status: no file at all. */
const NOTE_ABSENT = noteStatus({
  exists: false,
  managed: 'absent',
  fileSha256: null,
  fileSha256Short: null,
  snapshots: 0,
})

/** `.gitignore` exists (the project's own rules) but carries no managed block yet. */
const IGNORE_STATUS_CURRENT: GuidanceStatus = {
  kind: 'ignore',
  file: '.gitignore',
  exists: true,
  managed: 'current',
  templateVersion: TEMPLATE_VERSION,
  fileSha256: IGNORE_DIGEST,
  fileSha256Short: IGNORE_SHORT,
  snapshots: 0,
  pinned: 0,
}

/**
 * The note's block as the shell proposes it (`docs/spike-log.md` § Slice
 * 5c, IPC shapes), abridged to two of HAP-001's seven bullets: the
 * HTML-comment sentinels, the Markdown heading, a blank line, the bullets
 * with the outbox path substituted -- text the renderer shows line by line
 * and never interprets.
 */
const NOTE_LINES = [
  `<!-- omnifrons:begin guidance ${TEMPLATE_VERSION} sha256:${'d'.repeat(64)} -->`,
  '## Generated files',
  '',
  "- Write every generated file that is not a Markdown note — documents, images, audio, video, datasets, archives, exports — into `.omnifrons/outbox`, relative to this project's root. Create the directory if it does not exist.",
  '- Keep Markdown notes where this project already keeps its notes, never in `.omnifrons/outbox`.',
  '<!-- omnifrons:end guidance -->',
]

const IGNORE_LINES = [
  `# omnifrons:begin ignore ${TEMPLATE_VERSION} sha256:${'b'.repeat(64)}`,
  '/.omnifrons/outbox/',
  '# omnifrons:end ignore',
]

/** `guidance_preview { kind: "guidance", file: "AGENTS.md" }` for an absent file: an insert that creates it. */
function notePreview(overrides: Partial<GuidancePreview> = {}): GuidancePreview {
  return {
    kind: 'guidance',
    file: 'AGENTS.md',
    action: 'insert',
    proposed: NOTE_LINES.join('\n'),
    fileSha256: null,
    fileSha256Short: null,
    resultSha256Short: 'e5a1b2c3',
    ...overrides,
  }
}

/** The preview over an existing file carrying an outdated block: a replace, bound to the file's digest. */
const PREVIEW_REPLACE = notePreview({
  action: 'replace',
  fileSha256: FILE_DIGEST,
  fileSha256Short: FILE_SHORT,
  resultSha256Short: '7d7d7d7d',
})

const IGNORE_PREVIEW: GuidancePreview = {
  kind: 'ignore',
  file: '.gitignore',
  action: 'insert',
  proposed: IGNORE_LINES.join('\n'),
  fileSha256: IGNORE_DIGEST,
  fileSha256Short: IGNORE_SHORT,
  resultSha256Short: '0e9f4c31',
}

const APPLIED_REPLACE: GuidanceApplied = {
  kind: 'guidance',
  file: 'AGENTS.md',
  action: 'replace',
  snapshotId: '0123456789abcdef',
  resultSha256Short: '7d7d7d7d',
}

/** A removal that deleted the file Omnifrons created: no digest afterwards. */
const APPLIED_REMOVE: GuidanceApplied = {
  kind: 'guidance',
  file: 'AGENTS.md',
  action: 'remove',
  snapshotId: '89abcdef01234567',
  resultSha256Short: null,
}

const SNAPSHOT_NEWEST: Snapshot = {
  id: '89abcdef01234567',
  kind: 'guidance',
  file: 'AGENTS.md',
  existed: true,
  sha256Short: FILE_SHORT,
  size: 1512,
  takenAt: 1725782402000,
  pinned: false,
}

/** The snapshot of the file's absence, taken before the first insert, pinned. */
const SNAPSHOT_OLDEST: Snapshot = {
  id: '0123456789abcdef',
  kind: 'guidance',
  file: 'AGENTS.md',
  existed: false,
  sha256Short: 'e3b0c442',
  size: 0,
  takenAt: 1725782401000,
  pinned: true,
}

/** A snapshot of another guidance file: listed under the same kind, restorable only once that file is the one the status line shows. */
const SNAPSHOT_OTHER_FILE: Snapshot = {
  id: 'abcdefabcdefabcd',
  kind: 'guidance',
  file: 'CLAUDE.md',
  existed: true,
  sha256Short: '5e5e5e5e',
  size: 300,
  takenAt: 1725782400000,
  pinned: false,
}

const NOTE_SNAPSHOTS = [SNAPSHOT_NEWEST, SNAPSHOT_OLDEST, SNAPSHOT_OTHER_FILE]

const ADVISORY_LINE =
  'advisory: the note and the ignore rule set expectations and enforce nothing — not containment'
const SNAPSHOT_SENTENCE = 'a snapshot of the current file is taken before any write'
const ACT_AS_LINE = 'act as: device-local-user'

/** The removal surface's own disclosure of what a removal takes out (slice 5c review, R1-005). */
const REMOVE_SCOPE_LINE =
  'removes the managed block only; the file itself goes only when nothing else remains in it and Omnifrons created it'

/** The restore surface's disclosure that the snapshot's bytes are never shown (slice 5c review, R1-002). */
const RESTORE_BYTES_LINE =
  "the snapshot's bytes are not shown here: only what the manifest records about them"

/** The restore surface's disclosure that a snapshot of an absent file deletes (slice 5c review, R1-002). */
const RESTORE_REMOVES_LINE =
  'this snapshot recorded no file: restoring it REMOVES the file rather than writing bytes back'

/** What each guidance command answers: a value, or a thunk (for a rejection or a deferred promise built lazily). */
interface GuidanceMock {
  note?: GuidanceStatus | (() => unknown)
  ignore?: GuidanceStatus | (() => unknown)
  noteSnapshots?: Snapshot[] | (() => unknown)
  ignoreSnapshots?: Snapshot[] | (() => unknown)
}

function answerMock(answer: unknown): unknown {
  return typeof answer === 'function' ? (answer as () => unknown)() : answer
}

/**
 * Mounts over a workspace with the guidance answers in `mock` (a fresh
 * project's for anything unspecified) and `onCommand` answering the rest
 * -- previews, writes, pins, a spawn -- first.
 */
function mountGuidance(
  mock: GuidanceMock = {},
  onCommand?: (cmd: string, args: Record<string, unknown>) => unknown,
): Call[] {
  return mountWithWorkspace((cmd, args) => {
    const own = onCommand?.(cmd, args)
    if (own !== undefined) return own
    if (cmd === 'guidance_status') {
      const answer = args.kind === 'ignore' ? mock.ignore : mock.note
      return answer === undefined ? undefined : answerMock(answer)
    }
    if (cmd === 'guidance_snapshots') {
      const answer = args.kind === 'ignore' ? mock.ignoreSnapshots : mock.noteSnapshots
      return answer === undefined ? undefined : answerMock(answer)
    }
    return undefined
  })
}

function guidanceRegion(): HTMLElement {
  return screen.getByRole('region', { name: 'Guidance' })
}

function noteBlock(): HTMLElement {
  return screen.getByLabelText('Guidance note')
}

function ignoreBlock(): HTMLElement {
  return screen.getByLabelText('Ignore rule')
}

function noteStatusLine(): string | null {
  return screen.queryByRole('status', { name: 'Guidance note status' })?.textContent ?? null
}

function ignoreStatusLine(): string | null {
  return screen.queryByRole('status', { name: 'Ignore rule status' })?.textContent ?? null
}

function fileInput(): HTMLInputElement {
  return screen.getByLabelText('Guidance file') as HTMLInputElement
}

/** Types `name` into the guidance file input and commits it by leaving the field. */
function commitFile(name: string): void {
  fireEvent.change(fileInput(), { target: { value: name } })
  fireEvent.blur(fileInput())
}

function writeBlock(): HTMLElement {
  return screen.getByLabelText('Guidance write')
}

function writeBlockLines(): (string | null)[] {
  return Array.from(writeBlock().querySelectorAll('p')).map((line) => line.textContent)
}

/**
 * The proposed block's own lines, queried through the block's accessible
 * name rather than a test id (slice 5c review, R3-017): every child of the
 * labelled container is one rendered line, so the query asserts what the
 * test id did -- the lines, in order -- without a hook of its own.
 */
function proposedBlock(): HTMLElement {
  return within(writeBlock()).getByLabelText('proposed block')
}

function proposedLines(): (string | null)[] {
  return Array.from(proposedBlock().children).map((line) => line.textContent)
}

const GATE_LABEL = /short digest \(.*\) to (write|create the file|remove|restore)$/

function gateInput(): HTMLInputElement {
  return screen.getByLabelText(GATE_LABEL) as HTMLInputElement
}

function gateLabelText(): string {
  return writeBlock().querySelector('label')?.textContent ?? ''
}

function typeGate(value: string): void {
  fireEvent.change(gateInput(), { target: { value } })
}

function finalButton(name: 'Write' | 'Remove block' | 'Restore snapshot'): HTMLButtonElement {
  return screen.getByRole('button', { name }) as HTMLButtonElement
}

function resultLine(): string | null {
  return screen.queryByRole('status', { name: 'Guidance result' })?.textContent ?? null
}

/** Waits for both status lines, then clicks the note's Preview and waits for the block. */
async function openNotePreview(): Promise<HTMLElement> {
  await waitFor(() => {
    expect(noteStatusLine()).not.toBeNull()
  })
  fireEvent.click(within(noteBlock()).getByRole('button', { name: 'Preview' }))
  return screen.findByLabelText('Guidance write')
}

function snapshotRows(tableName = 'Guidance note snapshots'): string[][] {
  const table = screen.getByRole('table', { name: tableName })
  return Array.from(table.querySelectorAll('tbody tr')).map((row) =>
    Array.from(row.querySelectorAll('td')).map((cell) => cell.textContent ?? ''),
  )
}

function snapshotRow(id: string): HTMLElement {
  const table = screen.getByRole('table', { name: 'Guidance note snapshots' })
  const row = Array.from(table.querySelectorAll<HTMLElement>('tbody tr')).find(
    (candidate) => candidate.querySelectorAll('td')[1]?.textContent === id,
  )
  if (!row) throw new Error(`no snapshot row ${id}`)
  return row
}

function countCalls(calls: Call[], cmd: string): number {
  return calls.filter((call) => call.cmd === cmd).length
}

describe('AgentPanel guidance section (slice 5c, HAP-001 D18, R42)', () => {
  it('renders a Guidance region as a sibling of the transcript, the Candidates and the Publications regions -- never inside any of them -- with the fixed advisory line, the guidance file input defaulting to AGENTS.md, the fixed .gitignore line, Preview and Remove per kind, and on mount calls guidance_status for both kinds (exactly { kind: "guidance", file: "AGENTS.md" } and exactly { kind: "ignore" }) and guidance_snapshots for both', async () => {
    const calls = await listOutbox()

    const region = guidanceRegion()
    expect(region.textContent).toContain(ADVISORY_LINE)
    expect(fileInput().value).toBe('AGENTS.md')
    expect(ignoreBlock().textContent).toContain('Ignore file: .gitignore')
    for (const block of [noteBlock(), ignoreBlock()]) {
      expect(within(block).getByRole('button', { name: 'Preview' })).toBeTruthy()
      expect(within(block).getByRole('button', { name: 'Remove' })).toBeTruthy()
      expect(region.contains(block)).toBe(true)
    }
    const transcript = screen.getByLabelText('Agent transcript')
    const candidates = screen.getByRole('region', { name: 'Candidates' })
    const publications = screen.getByRole('region', { name: 'Publications' })
    for (const other of [transcript, candidates, publications]) {
      expect(other.contains(region)).toBe(false)
      expect(region.contains(other)).toBe(false)
    }
    expect(calls.filter((call) => call.cmd === 'guidance_status').map((call) => call.args)).toEqual(
      [{ kind: 'guidance', file: 'AGENTS.md' }, { kind: 'ignore' }],
    )
    expect(
      calls.filter((call) => call.cmd === 'guidance_snapshots').map((call) => call.args),
    ).toEqual([{ kind: 'guidance' }, { kind: 'ignore' }])
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it("renders no status line and no alert while no workspace is active (the shell's workspace-unavailable on both fetches), with Preview and Remove disabled for both kinds", async () => {
    mountWithWorkspace(
      (cmd) => {
        if (cmd === 'guidance_status' || cmd === 'guidance_snapshots') {
          return Promise.reject(OUTBOX_NO_WORKSPACE)
        }
        return undefined
      },
      { workspace: null },
    )
    await screen.findByLabelText('Adapter')
    await flush()

    expect(noteStatusLine()).toBeNull()
    expect(ignoreStatusLine()).toBeNull()
    expect(screen.queryByRole('alert')).toBeNull()
    for (const block of [noteBlock(), ignoreBlock()]) {
      expect(
        (within(block).getByRole('button', { name: 'Preview' }) as HTMLButtonElement).disabled,
      ).toBe(true)
      expect(
        (within(block).getByRole('button', { name: 'Remove' }) as HTMLButtonElement).disabled,
      ).toBe(true)
    }
  })

  it.each([
    {
      label: 'absent, no file',
      status: NOTE_ABSENT,
      line: 'AGENTS.md: block absent; file absent; snapshots 0, pinned 0',
    },
    {
      label: 'absent, the file exists without a block',
      status: noteStatus({ managed: 'absent', snapshots: 0 }),
      line: 'AGENTS.md: block absent; file sha256 short 3c1e9a07; snapshots 0, pinned 0',
    },
    {
      label: 'current',
      status: noteStatus(),
      line: 'AGENTS.md: block current; file sha256 short 3c1e9a07; snapshots 1, pinned 0',
    },
    {
      label: 'outdated',
      status: noteStatus({ managed: 'outdated', snapshots: 2, pinned: 1 }),
      line: 'AGENTS.md: block outdated (template hap-001-guidance-v1); file sha256 short 3c1e9a07; snapshots 2, pinned 1',
    },
    {
      label: 'modified',
      status: noteStatus({ managed: 'modified' }),
      line: 'AGENTS.md: block modified — resolve by hand or restore; file sha256 short 3c1e9a07; snapshots 1, pinned 0',
    },
    {
      label: 'malformed',
      status: noteStatus({ managed: 'malformed' }),
      line: 'AGENTS.md: block malformed — resolve by hand; file sha256 short 3c1e9a07; snapshots 1, pinned 0',
    },
  ])(
    'renders the note status line with fixed copy per managed token ($label): the file name, the block state, the short digest when the file exists, the snapshot and pinned counts',
    async ({ status, line }) => {
      mountGuidance({ note: status, ignore: IGNORE_STATUS_CURRENT })

      await waitFor(() => {
        expect(noteStatusLine()).toBe(line)
      })
      expect(ignoreStatusLine()).toBe(
        '.gitignore: block current; file sha256 short a71bc0d2; snapshots 0, pinned 0',
      )
      expect(screen.queryByRole('alert')).toBeNull()
    },
  )

  it('committing a new guidance file name (leaving the field, or Enter) refetches guidance_status with exactly { kind: "guidance", file: <name> } and shows that file\'s status; a response of an earlier fetch landing later is dropped, and the ignore kind is untouched', async () => {
    let resolveFirst: (status: GuidanceStatus) => void = () => {}
    let first = true
    const calls = mountGuidance({
      note: () => {
        if (first) {
          first = false
          return new Promise<GuidanceStatus>((resolve) => {
            resolveFirst = resolve
          })
        }
        return undefined
      },
    })
    // The second and later fetches answer through the fall-through: a fresh
    // status naming the requested file.
    await screen.findByLabelText('Guidance file')
    expect(noteStatusLine()).toBeNull()

    commitFile('CLAUDE.md')
    await waitFor(() => {
      expect(noteStatusLine()).toBe('CLAUDE.md: block absent; file absent; snapshots 0, pinned 0')
    })
    resolveFirst(noteStatus())
    await flush()
    expect(noteStatusLine()).toBe('CLAUDE.md: block absent; file absent; snapshots 0, pinned 0')

    fireEvent.change(fileInput(), { target: { value: 'NOTES.md' } })
    fireEvent.keyDown(fileInput(), { key: 'Enter' })
    await waitFor(() => {
      expect(noteStatusLine()).toBe('NOTES.md: block absent; file absent; snapshots 0, pinned 0')
    })
    expect(calls.filter((call) => call.cmd === 'guidance_status').map((call) => call.args)).toEqual(
      [
        { kind: 'guidance', file: 'AGENTS.md' },
        { kind: 'ignore' },
        { kind: 'guidance', file: 'CLAUDE.md' },
        { kind: 'guidance', file: 'NOTES.md' },
      ],
    )
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('Preview calls guidance_preview with exactly { kind: "guidance", file } -- and nothing else -- and opens the write block inside the Guidance region: file, action, the proposed block one line per text line verbatim through plain text (the HTML-comment sentinels and the Markdown heading as text: no comment node, no heading, no anchor, nothing interactive), "file sha256 short: <short>", the fixed snapshot sentence, the act-as line, the label naming the file\'s short digest, an empty input and a disabled Write', async () => {
    const calls = mountGuidance(
      { note: noteStatus({ managed: 'outdated' }) },
      (cmd) => (cmd === 'guidance_preview' ? PREVIEW_REPLACE : undefined),
    )
    await waitFor(() => {
      expect(noteStatusLine()).not.toBeNull()
    })
    const before = calls.length

    fireEvent.click(within(noteBlock()).getByRole('button', { name: 'Preview' }))
    const block = await screen.findByLabelText('Guidance write')

    expect(calls.slice(before).map((call) => call.cmd)).toEqual(['guidance_preview'])
    expect(calls[before]?.args).toEqual({ kind: 'guidance', file: 'AGENTS.md' })
    expect(Object.keys(calls[before]!.args).sort()).toEqual(['file', 'kind'])
    expect(guidanceRegion().contains(block)).toBe(true)
    expect(screen.getByLabelText('Agent transcript').contains(block)).toBe(false)
    expect(writeBlockLines()).toEqual([
      'file: AGENTS.md',
      'action: replace',
      'proposed:',
      'file sha256 short: 3c1e9a07',
      SNAPSHOT_SENTENCE,
      ACT_AS_LINE,
    ])
    expect(proposedLines()).toEqual(NOTE_LINES)
    expectNoInteractiveElements(proposedBlock())
    expect(block.querySelector('h1, h2, h3, a, script, ul, li, code, pre')).toBeNull()
    const walker = document.createTreeWalker(block, NodeFilter.SHOW_COMMENT)
    expect(walker.nextNode()).toBeNull()
    expect(block.textContent).toContain('<!-- omnifrons:begin guidance ')
    expect(block.textContent).toContain('## Generated files')
    expect(gateLabelText()).toBe("Type the file's short digest (3c1e9a07) to write")
    expect(gateInput().value).toBe('')
    expect(finalButton('Write').disabled).toBe(true)
  })

  it('for an absent file the block reads "file: absent" and the label names the proposed block\'s short digest ("to create the file"); typing it enables Write, whose click sends fileSha256 null', async () => {
    const calls = mountGuidance({ note: NOTE_ABSENT }, (cmd) => {
      if (cmd === 'guidance_preview') return notePreview()
      if (cmd === 'guidance_apply') {
        return { ...APPLIED_REPLACE, action: 'insert', resultSha256Short: 'e5a1b2c3' }
      }
      return undefined
    })
    await openNotePreview()

    expect(writeBlockLines()).toEqual([
      'file: AGENTS.md',
      'action: insert',
      'proposed:',
      'file: absent',
      SNAPSHOT_SENTENCE,
      ACT_AS_LINE,
    ])
    expect(gateLabelText()).toBe(
      "Type the proposed block's short digest (e5a1b2c3) to create the file",
    )
    typeGate('e5a1b2c3')
    expect(finalButton('Write').disabled).toBe(false)
    fireEvent.click(finalButton('Write'))

    await waitFor(() => {
      expect(resultLine()).toBe(
        'Guidance note AGENTS.md: written insert, snapshot 0123456789abcdef, result e5a1b2c3',
      )
    })
    const applies = calls.filter((call) => call.cmd === 'guidance_apply')
    expect(applies).toHaveLength(1)
    expect(applies[0]?.args).toEqual({ kind: 'guidance', file: 'AGENTS.md', fileSha256: null })
  })

  it("enables Write only on an exact match of the gate digest: never the fixed phrase, uppercase hex, a 7- or 9-character prefix, a leading or trailing space (the shape a paste produces), the result digest of a file that exists, the full digest, or the other kind's digest", async () => {
    mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, (cmd) =>
      cmd === 'guidance_preview' ? PREVIEW_REPLACE : undefined,
    )
    await openNotePreview()

    for (const wrong of [
      'write',
      '3C1E9A07',
      '3c1e9a0',
      '3c1e9a07a',
      ' 3c1e9a07',
      '3c1e9a07 ',
      ' 3c1e9a07 ',
      '7d7d7d7d',
      FILE_DIGEST,
      IGNORE_SHORT,
    ]) {
      typeGate(wrong)
      expect(finalButton('Write').disabled).toBe(true)
    }
    typeGate(FILE_SHORT)
    expect(finalButton('Write').disabled).toBe(false)
  })

  it('no harness- or content-originated value pre-fills or enables the gate (TM-001-R1): a publish proposal and a candidate row naming the very short digest, the digest shown in the label itself, and a programmatic input.value with no input event all leave the input empty and Write disabled; only typing enables it', async () => {
    let channel: LiveChannel | undefined
    mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, (cmd, args) => {
      if (cmd === 'harness_spawn') {
        channel = (args as { onFrame: LiveChannel }).onFrame
        return 7
      }
      if (cmd === 'candidates_list') return [{ ...OUTBOX_CANDIDATES[0]!, name: FILE_SHORT }]
      if (cmd === 'guidance_preview') return PREVIEW_REPLACE
      return undefined
    })
    await startRunOverWorkspace()
    if (!channel) throw new Error('harness_spawn was not called')
    // The harness's own proposal, naming the very digest the gate wants.
    deliverPublishProposal(channel, [{ name: FILE_SHORT, sha256: FILE_DIGEST }])
    await endRun(channel)
    fireEvent.click(listOutboxButton())
    await screen.findByRole('region', { name: 'Candidates' })
    expect(candidateRows()[0]?.[0]).toBe(FILE_SHORT)

    await openNotePreview()

    expect(gateInput().value).toBe('')
    expect(finalButton('Write').disabled).toBe(true)
    gateInput().value = FILE_SHORT
    await flush()
    expect(gateInput().value).toBe(FILE_SHORT)
    expect(finalButton('Write').disabled).toBe(true)
    // (React's value tracker reports a change event carrying the string
    // already set on the node as no change, so the typed sequence passes
    // through another value first -- a test-harness detail.)
    typeGate('')
    expect(finalButton('Write').disabled).toBe(true)
    typeGate(FILE_SHORT)
    expect(finalButton('Write').disabled).toBe(false)
  })

  it('Write calls guidance_apply with exactly { kind: "guidance", file, fileSha256 } -- the preview\'s file and full 64-hex digest, never the typed value -- once per double-click, then shows "<kind label> <file>: written <action>, snapshot <id>, result <short>", closes the block, and refetches guidance_status and guidance_snapshots for that kind only', async () => {
    let resolveApply: (applied: GuidanceApplied) => void = () => {}
    let written = false
    const calls = mountGuidance(
      { note: () => noteStatus({ managed: written ? 'current' : 'outdated' }) },
      (cmd) => {
        if (cmd === 'guidance_preview') return PREVIEW_REPLACE
        if (cmd === 'guidance_apply') {
          return new Promise<GuidanceApplied>((resolve) => {
            resolveApply = resolve
          })
        }
        return undefined
      },
    )
    await openNotePreview()
    const statusesBefore = countCalls(calls, 'guidance_status')
    const snapshotsBefore = countCalls(calls, 'guidance_snapshots')
    typeGate(FILE_SHORT)

    fireEvent.click(finalButton('Write'))
    fireEvent.click(finalButton('Write'))

    await waitFor(() => {
      expect(finalButton('Write').disabled).toBe(true)
    })
    expect(gateInput().disabled).toBe(true)
    const applies = calls.filter((call) => call.cmd === 'guidance_apply')
    expect(applies).toHaveLength(1)
    expect(applies[0]?.args).toEqual({
      kind: 'guidance',
      file: 'AGENTS.md',
      fileSha256: FILE_DIGEST,
    })
    expect(Object.keys(applies[0]!.args).sort()).toEqual(['file', 'fileSha256', 'kind'])
    expect(applies[0]?.args.fileSha256).toHaveLength(64)
    expect(applies[0]?.args.fileSha256).not.toBe(FILE_SHORT)

    written = true
    resolveApply(APPLIED_REPLACE)
    await waitFor(() => {
      expect(resultLine()).toBe(
        'Guidance note AGENTS.md: written replace, snapshot 0123456789abcdef, result 7d7d7d7d',
      )
    })
    expect(screen.queryByLabelText('Guidance write')).toBeNull()
    await waitFor(() => {
      expect(noteStatusLine()).toContain('block current')
    })
    expect(countCalls(calls, 'guidance_status')).toBe(statusesBefore + 1)
    expect(countCalls(calls, 'guidance_snapshots')).toBe(snapshotsBefore + 1)
    expect(
      calls.filter((call) => call.cmd === 'guidance_status').at(-1)?.args,
    ).toEqual({ kind: 'guidance', file: 'AGENTS.md' })
    expect(calls.filter((call) => call.cmd === 'guidance_snapshots').at(-1)?.args).toEqual({
      kind: 'guidance',
    })
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('a no-op write shows "<kind label> <file>: written no-op, snapshot none, result <short>"', async () => {
    mountGuidance({ note: noteStatus() }, (cmd) => {
      if (cmd === 'guidance_preview') {
        return { ...PREVIEW_REPLACE, action: 'no-op', resultSha256Short: FILE_SHORT }
      }
      if (cmd === 'guidance_apply') {
        return { ...APPLIED_REPLACE, action: 'no-op', snapshotId: null, resultSha256Short: FILE_SHORT }
      }
      return undefined
    })
    await openNotePreview()
    expect(writeBlockLines()).toContain('action: no-op')
    typeGate(FILE_SHORT)
    fireEvent.click(finalButton('Write'))

    await waitFor(() => {
      expect(resultLine()).toBe(
        'Guidance note AGENTS.md: written no-op, snapshot none, result 3c1e9a07',
      )
    })
  })

  it('the ignore kind: Preview sends exactly { kind: "ignore" } with no file key, the block shows .gitignore and the three proposed lines verbatim, Write sends exactly { kind: "ignore", fileSha256 } with no file key, and the receipt names that kind and that file', async () => {
    const calls = mountGuidance({ ignore: IGNORE_STATUS_CURRENT }, (cmd) => {
      if (cmd === 'guidance_preview') return IGNORE_PREVIEW
      if (cmd === 'guidance_apply') {
        return {
          kind: 'ignore',
          file: '.gitignore',
          action: 'insert',
          snapshotId: 'fedcba9876543210',
          resultSha256Short: '0e9f4c31',
        }
      }
      return undefined
    })
    await waitFor(() => {
      expect(ignoreStatusLine()).not.toBeNull()
    })

    fireEvent.click(within(ignoreBlock()).getByRole('button', { name: 'Preview' }))
    await screen.findByLabelText('Guidance write')

    const previews = calls.filter((call) => call.cmd === 'guidance_preview')
    expect(previews).toHaveLength(1)
    expect(previews[0]?.args).toEqual({ kind: 'ignore' })
    expect(Object.keys(previews[0]!.args)).toEqual(['kind'])
    expect(writeBlockLines()).toEqual([
      'file: .gitignore',
      'action: insert',
      'proposed:',
      'file sha256 short: a71bc0d2',
      SNAPSHOT_SENTENCE,
      ACT_AS_LINE,
    ])
    expect(proposedLines()).toEqual(IGNORE_LINES)
    expect(gateLabelText()).toBe("Type the file's short digest (a71bc0d2) to write")
    typeGate(IGNORE_SHORT)
    fireEvent.click(finalButton('Write'))

    await waitFor(() => {
      expect(resultLine()).toBe(
        'Ignore rule .gitignore: written insert, snapshot fedcba9876543210, result 0e9f4c31',
      )
    })
    const applies = calls.filter((call) => call.cmd === 'guidance_apply')
    expect(applies[0]?.args).toEqual({ kind: 'ignore', fileSha256: IGNORE_DIGEST })
    expect(Object.keys(applies[0]!.args).sort()).toEqual(['fileSha256', 'kind'])
  })

  it('Remove opens a remove block bound to the status -- file, "action: remove", what a removal takes out, the file\'s short digest, the snapshot sentence, the act-as line, the label "…to remove", a disabled "Remove block" -- invoking nothing; typing the digest enables it, and the click calls guidance_remove with exactly { kind, file, fileSha256 } from the status, shows the removal receipt and refetches', async () => {
    let removed = false
    const calls = mountGuidance(
      { note: () => (removed ? NOTE_ABSENT : noteStatus()) },
      (cmd) => (cmd === 'guidance_remove' ? APPLIED_REMOVE : undefined),
    )
    await waitFor(() => {
      expect(noteStatusLine()).not.toBeNull()
    })
    const before = calls.length

    fireEvent.click(within(noteBlock()).getByRole('button', { name: 'Remove' }))
    await screen.findByLabelText('Guidance write')

    expect(calls.length).toBe(before)
    expect(writeBlockLines()).toEqual([
      'file: AGENTS.md',
      'action: remove',
      REMOVE_SCOPE_LINE,
      'file sha256 short: 3c1e9a07',
      SNAPSHOT_SENTENCE,
      ACT_AS_LINE,
    ])
    expect(gateLabelText()).toBe("Type the file's short digest (3c1e9a07) to remove")
    expect(finalButton('Remove block').disabled).toBe(true)
    typeGate(FILE_SHORT)
    expect(finalButton('Remove block').disabled).toBe(false)
    removed = true
    fireEvent.click(finalButton('Remove block'))

    await waitFor(() => {
      expect(resultLine()).toBe(
        'Guidance note AGENTS.md: written remove, snapshot 89abcdef01234567, result absent',
      )
    })
    const removes = calls.filter((call) => call.cmd === 'guidance_remove')
    expect(removes).toHaveLength(1)
    expect(removes[0]?.args).toEqual({ kind: 'guidance', file: 'AGENTS.md', fileSha256: FILE_DIGEST })
    expect(Object.keys(removes[0]!.args).sort()).toEqual(['file', 'fileSha256', 'kind'])
    expect(screen.queryByLabelText('Guidance write')).toBeNull()
    await waitFor(() => {
      expect(noteStatusLine()).toBe('AGENTS.md: block absent; file absent; snapshots 0, pinned 0')
    })
  })

  it.each([
    { label: 'absent', status: NOTE_ABSENT, offered: false },
    { label: 'current', status: noteStatus(), offered: true },
    { label: 'outdated', status: noteStatus({ managed: 'outdated' }), offered: true },
    { label: 'modified', status: noteStatus({ managed: 'modified' }), offered: false },
    { label: 'malformed', status: noteStatus({ managed: 'malformed' }), offered: false },
  ])(
    'offers Remove only for a block the shell would remove ($label: $offered): current or outdated; never for an absent, modified or malformed block',
    async ({ status, offered }) => {
      mountGuidance({ note: status })
      await waitFor(() => {
        expect(noteStatusLine()).not.toBeNull()
      })

      const remove = within(noteBlock()).getByRole('button', { name: 'Remove' }) as HTMLButtonElement
      expect(remove.disabled).toBe(!offered)
    },
  )
})

describe('AgentPanel guidance snapshots (slice 5c, HAP-001 D18)', () => {
  it('renders a Snapshots table per kind that has any -- file, id, taken at, sha256 short, size, existed, pinned, action -- rows as listed (newest first), taken at as an ISO time, existed and pinned as yes/no, every cell plain text, Pin or Unpin by the row\'s flag, and Restore only on the snapshots of the file the status line shows; no table for an empty list', async () => {
    mountGuidance({ note: noteStatus({ snapshots: 3, pinned: 1 }), noteSnapshots: NOTE_SNAPSHOTS })
    await waitFor(() => {
      expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).not.toBeNull()
    })

    const table = screen.getByRole('table', { name: 'Guidance note snapshots' })
    expect(noteBlock().contains(table)).toBe(true)
    expect(Array.from(table.querySelectorAll('th')).map((header) => header.textContent)).toEqual([
      'file',
      'id',
      'taken at',
      'sha256 short',
      'size',
      'existed',
      'pinned',
      'action',
    ])
    expect(snapshotRows()).toEqual([
      [
        'AGENTS.md',
        '89abcdef01234567',
        new Date(1725782402000).toISOString(),
        '3c1e9a07',
        '1512',
        'yes',
        'no',
        'Pin Restore',
      ],
      [
        'AGENTS.md',
        '0123456789abcdef',
        new Date(1725782401000).toISOString(),
        'e3b0c442',
        '0',
        'no',
        'yes',
        'Unpin Restore',
      ],
      [
        'CLAUDE.md',
        'abcdefabcdefabcd',
        new Date(1725782400000).toISOString(),
        '5e5e5e5e',
        '300',
        'yes',
        'no',
        'Pin',
      ],
    ])
    for (const cell of Array.from(table.querySelectorAll<HTMLElement>('tbody td:not(:last-child)'))) {
      expectNoInteractiveElements(cell)
    }
    expect(screen.queryByRole('table', { name: 'Ignore rule snapshots' })).toBeNull()
  })

  it('Pin and Unpin call guidance_pin with exactly { id, pinned } -- the opposite of the row\'s flag -- apply the returned snapshot to the row, and refetch the status for the pinned count', async () => {
    let pinnedCount = 1
    const calls = mountGuidance(
      {
        note: () => noteStatus({ snapshots: 3, pinned: pinnedCount }),
        noteSnapshots: NOTE_SNAPSHOTS,
      },
      (cmd, args) => {
        if (cmd === 'guidance_pin') {
          const snapshot = NOTE_SNAPSHOTS.find((candidate) => candidate.id === args.id)!
          pinnedCount += args.pinned ? 1 : -1
          return { ...snapshot, pinned: args.pinned }
        }
        return undefined
      },
    )
    await waitFor(() => {
      expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).not.toBeNull()
    })

    fireEvent.click(within(snapshotRow('89abcdef01234567')).getByRole('button', { name: 'Pin' }))
    await waitFor(() => {
      expect(snapshotRows()[0]?.slice(6)).toEqual(['yes', 'Unpin Restore'])
    })
    await waitFor(() => {
      expect(noteStatusLine()).toContain('pinned 2')
    })
    fireEvent.click(within(snapshotRow('0123456789abcdef')).getByRole('button', { name: 'Unpin' }))
    await waitFor(() => {
      expect(snapshotRows()[1]?.slice(6)).toEqual(['no', 'Pin Restore'])
    })
    await waitFor(() => {
      expect(noteStatusLine()).toContain('pinned 1')
    })

    const pins = calls.filter((call) => call.cmd === 'guidance_pin').map((call) => call.args)
    expect(pins).toEqual([
      { id: '89abcdef01234567', pinned: true },
      { id: '0123456789abcdef', pinned: false },
    ])
    expect(Object.keys(pins[0]!).sort()).toEqual(['id', 'pinned'])
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('Restore opens a restore block with the snapshot\'s facts, the two restore disclosures and the gate against the current file digest ("…to restore"); the click calls guidance_restore with exactly { id, fileSha256 } -- the status\'s full digest -- shows the restore receipt and refetches', async () => {
    const calls = mountGuidance(
      { note: noteStatus({ snapshots: 3, pinned: 1 }), noteSnapshots: NOTE_SNAPSHOTS },
      (cmd) =>
        cmd === 'guidance_restore'
          ? {
              kind: 'guidance',
              file: 'AGENTS.md',
              action: 'restore',
              snapshotId: 'fedcba9876543210',
              resultSha256Short: null,
            }
          : undefined,
    )
    await waitFor(() => {
      expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).not.toBeNull()
    })
    const before = calls.length

    fireEvent.click(within(snapshotRow('0123456789abcdef')).getByRole('button', { name: 'Restore' }))
    await screen.findByLabelText('Guidance write')

    expect(calls.length).toBe(before)
    expect(writeBlockLines()).toEqual([
      'file: AGENTS.md',
      'action: restore',
      `snapshot: 0123456789abcdef, taken at ${new Date(1725782401000).toISOString()}, sha256 short e3b0c442, size 0, existed no`,
      RESTORE_BYTES_LINE,
      RESTORE_REMOVES_LINE,
      'file sha256 short: 3c1e9a07',
      SNAPSHOT_SENTENCE,
      ACT_AS_LINE,
    ])
    expect(gateLabelText()).toBe("Type the file's short digest (3c1e9a07) to restore")
    expect(finalButton('Restore snapshot').disabled).toBe(true)
    typeGate(FILE_SHORT)
    expect(finalButton('Restore snapshot').disabled).toBe(false)
    const statusesBefore = countCalls(calls, 'guidance_status')
    fireEvent.click(finalButton('Restore snapshot'))

    await waitFor(() => {
      expect(resultLine()).toBe(
        'Guidance note AGENTS.md: written restore, snapshot fedcba9876543210, result absent',
      )
    })
    const restores = calls.filter((call) => call.cmd === 'guidance_restore')
    expect(restores).toHaveLength(1)
    expect(restores[0]?.args).toEqual({ id: '0123456789abcdef', fileSha256: FILE_DIGEST })
    expect(Object.keys(restores[0]!.args).sort()).toEqual(['fileSha256', 'id'])
    expect(screen.queryByLabelText('Guidance write')).toBeNull()
    await waitFor(() => {
      expect(countCalls(calls, 'guidance_status')).toBe(statusesBefore + 1)
    })
  })

  it("for an absent file the restore gate is the snapshot's own short digest and the request carries fileSha256 null", async () => {
    const calls = mountGuidance(
      { note: noteStatus({ exists: false, managed: 'absent', fileSha256: null, fileSha256Short: null }), noteSnapshots: [SNAPSHOT_NEWEST] },
      (cmd) =>
        cmd === 'guidance_restore'
          ? {
              kind: 'guidance',
              file: 'AGENTS.md',
              action: 'restore',
              snapshotId: 'fedcba9876543210',
              resultSha256Short: FILE_SHORT,
            }
          : undefined,
    )
    await waitFor(() => {
      expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).not.toBeNull()
    })

    fireEvent.click(within(snapshotRow('89abcdef01234567')).getByRole('button', { name: 'Restore' }))
    await screen.findByLabelText('Guidance write')

    expect(writeBlockLines()).toContain('file: absent')
    expect(gateLabelText()).toBe("Type the snapshot's short digest (3c1e9a07) to restore")
    typeGate(FILE_SHORT)
    fireEvent.click(finalButton('Restore snapshot'))

    await waitFor(() => {
      expect(resultLine()).toBe(
        'Guidance note AGENTS.md: written restore, snapshot fedcba9876543210, result 3c1e9a07',
      )
    })
    expect(calls.filter((call) => call.cmd === 'guidance_restore')[0]?.args).toEqual({
      id: '89abcdef01234567',
      fileSha256: null,
    })
  })
})

describe('AgentPanel guidance codes and guards (slice 5c)', () => {
  it.each([
    {
      code: 'guidance-file-invalid',
      message: 'the guidance file must end in .md',
    },
    {
      code: 'guidance-file-changed',
      message: 'the managed file changed since it was shown; read its status again and retry',
    },
    {
      code: 'guidance-block-modified',
      message:
        'the managed block was modified inside its sentinels; resolve it by hand or restore a snapshot',
    },
    {
      code: 'guidance-block-malformed',
      message:
        "the managed block's sentinels are not one intact pair; resolve it by hand or restore a snapshot",
    },
    { code: 'guidance-unmanaged', message: 'the file carries no managed block' },
    { code: 'snapshot-unavailable', message: 'the snapshot store could not be written' },
    { code: 'run-active', message: 'a run is active; approve or publish once it has ended' },
  ])(
    'renders $code from guidance_apply through the banner as "<code>: <message>" with no "untrusted" and no detail; the block stays open with the typed digest kept and Write enabled again -- except guidance-file-changed, which refetches the status and snapshots and closes the block',
    async ({ code, message }) => {
      const calls = mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, (cmd) => {
        if (cmd === 'guidance_preview') return PREVIEW_REPLACE
        if (cmd === 'guidance_apply') return Promise.reject({ code, message })
        return undefined
      })
      await openNotePreview()
      const statusesBefore = countCalls(calls, 'guidance_status')
      const snapshotsBefore = countCalls(calls, 'guidance_snapshots')
      typeGate(FILE_SHORT)
      fireEvent.click(finalButton('Write'))

      const alert = await screen.findByRole('alert')
      expect(alert.textContent).toBe(`${code}: ${message}`)
      expect(alert.textContent).not.toContain('untrusted')
      if (code === 'guidance-file-changed') {
        expect(screen.queryByLabelText('Guidance write')).toBeNull()
        await waitFor(() => {
          expect(countCalls(calls, 'guidance_status')).toBe(statusesBefore + 1)
        })
        expect(countCalls(calls, 'guidance_snapshots')).toBe(snapshotsBefore + 1)
      } else {
        expect(screen.getByLabelText('Guidance write')).toBeTruthy()
        expect(gateInput().value).toBe(FILE_SHORT)
        await waitFor(() => {
          expect(finalButton('Write').disabled).toBe(false)
        })
        expect(countCalls(calls, 'guidance_status')).toBe(statusesBefore)
        expect(countCalls(calls, 'guidance_snapshots')).toBe(snapshotsBefore)
      }
    },
  )

  it('a guidance-file-invalid rejection of the status fetch for a committed file name reaches the banner with the rule\'s own message, never the name, and leaves the previous status line standing', async () => {
    mountGuidance({
      note: () => undefined,
    }, (cmd, args) => {
      if (cmd === 'guidance_status' && args.file === 'notes.txt') {
        return Promise.reject({
          code: 'guidance-file-invalid',
          message: 'the guidance file must end in .md',
        })
      }
      return undefined
    })
    await waitFor(() => {
      expect(noteStatusLine()).toBe('AGENTS.md: block absent; file absent; snapshots 0, pinned 0')
    })

    commitFile('notes.txt')

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toBe('guidance-file-invalid: the guidance file must end in .md')
    expect(alert.textContent).not.toContain('notes.txt')
    expect(noteStatusLine()).toBe('AGENTS.md: block absent; file absent; snapshots 0, pinned 0')
  })

  it('shows only "unexpected error" when a guidance command rejects with a plain Error, never its message', async () => {
    mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, (cmd) => {
      if (cmd === 'guidance_preview') return Promise.reject(new Error('secret detail'))
      return undefined
    })
    await waitFor(() => {
      expect(noteStatusLine()).not.toBeNull()
    })

    fireEvent.click(within(noteBlock()).getByRole('button', { name: 'Preview' }))

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toBe('unexpected error')
    expect(screen.queryByLabelText('Guidance write')).toBeNull()
  })

  it('while a run is active Preview, Remove, Restore, an open block\'s input and its final button are frozen, while the file input and Pin stay live; everything is live again once the run ends', async () => {
    let channel: LiveChannel | undefined
    mountGuidance(
      { note: noteStatus({ snapshots: 3, pinned: 1 }), noteSnapshots: NOTE_SNAPSHOTS },
      (cmd, args) => {
        if (cmd === 'harness_spawn') {
          channel = (args as { onFrame: LiveChannel }).onFrame
          return 7
        }
        if (cmd === 'guidance_preview') return { ...PREVIEW_REPLACE, action: 'no-op' }
        return undefined
      },
    )
    await openNotePreview()
    typeGate(FILE_SHORT)
    expect(finalButton('Write').disabled).toBe(false)

    await startRunOverWorkspace()

    const controls = () => ({
      preview: (within(noteBlock()).getByRole('button', { name: 'Preview' }) as HTMLButtonElement)
        .disabled,
      remove: (within(noteBlock()).getByRole('button', { name: 'Remove' }) as HTMLButtonElement)
        .disabled,
      restore: (
        within(snapshotRow('89abcdef01234567')).getByRole('button', {
          name: 'Restore',
        }) as HTMLButtonElement
      ).disabled,
      pin: (within(snapshotRow('89abcdef01234567')).getByRole('button', { name: 'Pin' }) as HTMLButtonElement)
        .disabled,
      gate: gateInput().disabled,
      write: finalButton('Write').disabled,
      file: fileInput().disabled,
    })
    expect(controls()).toEqual({
      preview: true,
      remove: true,
      restore: true,
      pin: false,
      gate: true,
      write: true,
      file: false,
    })

    if (!channel) throw new Error('harness_spawn was not called')
    await endRun(channel)
    expect(controls()).toEqual({
      preview: false,
      remove: false,
      restore: false,
      pin: false,
      gate: false,
      write: false,
      file: false,
    })
  })

  /**
   * The seven guidance commands, each reached the way the surface reaches
   * it: a fetch on mount, a button, or a final button through a confirmed
   * gate. Mounts, drives the panel until `command` is in flight, and hands
   * back the recorded calls and that one deferred promise's settle
   * functions (slice 5c review, R3-001).
   */
  async function driveGuidanceInFlight(command: string): Promise<{
    calls: Call[]
    resolve: (value: unknown) => void
    reject: (reason: unknown) => void
  }> {
    let resolve: (value: unknown) => void = () => {}
    let reject: (reason: unknown) => void = () => {}
    const deferred = (): Promise<never> =>
      new Promise((settle, refuse) => {
        resolve = settle as (value: unknown) => void
        reject = refuse
      })
    const answer = (cmd: string): unknown => (cmd === command ? deferred() : undefined)

    if (command === 'guidance_status') {
      const calls = mountGuidance({ note: () => deferred(), ignore: IGNORE_STATUS_CURRENT })
      await waitFor(() => {
        expect(ignoreStatusLine()).not.toBeNull()
      })
      return { calls, resolve, reject }
    }
    if (command === 'guidance_snapshots') {
      const calls = mountGuidance({ note: noteStatus(), noteSnapshots: () => deferred() })
      await waitFor(() => {
        expect(noteStatusLine()).not.toBeNull()
      })
      return { calls, resolve, reject }
    }
    if (command === 'guidance_preview') {
      const calls = mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, answer)
      await waitFor(() => {
        expect(noteStatusLine()).not.toBeNull()
      })
      fireEvent.click(within(noteBlock()).getByRole('button', { name: 'Preview' }))
      await waitFor(() => {
        expect(countCalls(calls, 'guidance_preview')).toBe(1)
      })
      return { calls, resolve, reject }
    }
    if (command === 'guidance_apply') {
      const calls = mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, (cmd) =>
        cmd === 'guidance_preview' ? PREVIEW_REPLACE : answer(cmd),
      )
      await openNotePreview()
      typeGate(FILE_SHORT)
      fireEvent.click(finalButton('Write'))
      await waitFor(() => {
        expect(countCalls(calls, 'guidance_apply')).toBe(1)
      })
      return { calls, resolve, reject }
    }
    if (command === 'guidance_remove') {
      const calls = mountGuidance({ note: noteStatus() }, answer)
      await waitFor(() => {
        expect(noteStatusLine()).not.toBeNull()
      })
      fireEvent.click(within(noteBlock()).getByRole('button', { name: 'Remove' }))
      await screen.findByLabelText('Guidance write')
      typeGate(FILE_SHORT)
      fireEvent.click(finalButton('Remove block'))
      await waitFor(() => {
        expect(countCalls(calls, 'guidance_remove')).toBe(1)
      })
      return { calls, resolve, reject }
    }
    const calls = mountGuidance(
      { note: noteStatus({ snapshots: 3, pinned: 1 }), noteSnapshots: NOTE_SNAPSHOTS },
      answer,
    )
    await waitFor(() => {
      expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).not.toBeNull()
    })
    if (command === 'guidance_restore') {
      fireEvent.click(within(snapshotRow('89abcdef01234567')).getByRole('button', { name: 'Restore' }))
      await screen.findByLabelText('Guidance write')
      typeGate(FILE_SHORT)
      fireEvent.click(finalButton('Restore snapshot'))
      await waitFor(() => {
        expect(countCalls(calls, 'guidance_restore')).toBe(1)
      })
      return { calls, resolve, reject }
    }
    fireEvent.click(within(snapshotRow('89abcdef01234567')).getByRole('button', { name: 'Pin' }))
    await waitFor(() => {
      expect(countCalls(calls, 'guidance_pin')).toBe(1)
    })
    return { calls, resolve, reject }
  }

  /** What each guidance command answers when its continuation is settled after the unmount. */
  const GUIDANCE_UNMOUNT_ANSWERS: Record<string, unknown> = {
    guidance_status: noteStatus(),
    guidance_snapshots: NOTE_SNAPSHOTS,
    guidance_preview: PREVIEW_REPLACE,
    guidance_apply: APPLIED_REPLACE,
    guidance_remove: APPLIED_REMOVE,
    guidance_restore: { ...APPLIED_REMOVE, action: 'restore' },
    guidance_pin: { ...SNAPSHOT_NEWEST, pinned: true },
  }

  it.each([
    'guidance_status',
    'guidance_snapshots',
    'guidance_preview',
    'guidance_apply',
    'guidance_remove',
    'guidance_restore',
    'guidance_pin',
  ])(
    "%s's continuation no-ops after unmount, whether it resolves or rejects: nothing thrown, no console.error, and no IPC call after the unmount (the success paths of the writing commands and of pin would otherwise refetch)",
    async (command) => {
      for (const outcome of ['resolve', 'reject'] as const) {
        const { calls, resolve, reject } = await driveGuidanceInFlight(command)
        const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
        const before = calls.length

        cleanup()
        if (outcome === 'resolve') resolve(GUIDANCE_UNMOUNT_ANSWERS[command])
        else reject({ code: 'snapshot-unavailable', message: 'a snapshot manifest is corrupt' })
        await new Promise((settled) => {
          setTimeout(settled, 0)
        })

        expect(consoleErrorSpy).not.toHaveBeenCalled()
        expect(calls.length).toBe(before)
        consoleErrorSpy.mockRestore()
        clearMocks()
      }
    },
  )

  it("a workspace pick clears the completed write's result line: the receipt belongs to the project that was active when it was made", async () => {
    mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, (cmd) => {
      if (cmd === 'guidance_preview') return PREVIEW_REPLACE
      if (cmd === 'guidance_apply') return APPLIED_REPLACE
      if (cmd === 'workspace_pick') return { displayPath: '/home/user/other', workArea: 'valid' }
      return undefined
    })
    await openNotePreview()
    typeGate(FILE_SHORT)
    fireEvent.click(finalButton('Write'))
    await waitFor(() => {
      expect(resultLine()).toBe(
        'Guidance note AGENTS.md: written replace, snapshot 0123456789abcdef, result 7d7d7d7d',
      )
    })

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))

    await waitFor(() => {
      expect(resultLine()).toBeNull()
    })
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('a workspace pick resets the section: the open block goes, both kinds\' status and snapshots are fetched again, and a pre-pick status response landing after the pick is dropped in favour of the post-pick one', async () => {
    let resolveFirst: (status: GuidanceStatus) => void = () => {}
    let fetches = 0
    const calls = mountGuidance(
      {
        note: () => {
          fetches += 1
          if (fetches === 1) {
            return new Promise<GuidanceStatus>((resolve) => {
              resolveFirst = resolve
            })
          }
          return noteStatus({ managed: 'outdated' })
        },
        ignore: IGNORE_STATUS_CURRENT,
      },
      (cmd) => {
        if (cmd === 'guidance_preview') return IGNORE_PREVIEW
        if (cmd === 'workspace_pick') return { displayPath: '/home/user/other', workArea: 'valid' }
        return undefined
      },
    )
    await waitFor(() => {
      expect(ignoreStatusLine()).not.toBeNull()
    })
    fireEvent.click(within(ignoreBlock()).getByRole('button', { name: 'Preview' }))
    await screen.findByLabelText('Guidance write')
    const statusesBefore = countCalls(calls, 'guidance_status')
    const snapshotsBefore = countCalls(calls, 'guidance_snapshots')

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))

    await waitFor(() => {
      expect(noteStatusLine()).toContain('block outdated')
    })
    expect(screen.queryByLabelText('Guidance write')).toBeNull()
    expect(countCalls(calls, 'guidance_status')).toBe(statusesBefore + 2)
    expect(countCalls(calls, 'guidance_snapshots')).toBe(snapshotsBefore + 2)
    resolveFirst(NOTE_ABSENT)
    await flush()
    expect(noteStatusLine()).toContain('block outdated')
    expect(screen.queryByRole('alert')).toBeNull()
  })
})

describe('AgentPanel guidance content security (slice 5c, RCS-001, TM-001-R1)', () => {
  const esc = String.fromCharCode(0x1b)
  /** U+202E RIGHT-TO-LEFT OVERRIDE, built from its code point so no bidi control sits in this source file. */
  const rlo = String.fromCodePoint(0x202e)

  it('renders a managed file name carrying a literal <b>, a C0 byte and a bidi override in the status line, the block and the label as text: no <b>, no anchor, controls stripped; the proposed block carrying a script tag, a Markdown link, an HTML anchor and an HTML comment renders as text with no script, anchor, href or comment node anywhere in the region', async () => {
    const hostile = `<b>AGENTS</b>${esc}x${rlo}.md`
    const hostileProposed = [
      '<!-- omnifrons:begin guidance hap-001-guidance-v1 sha256:00 -->',
      '<script>alert(1)</script>',
      '[click](http://example.invalid) <a href="http://example.invalid">x</a>',
      `line${esc}[31mwith${rlo}controls`,
      '<!-- omnifrons:end guidance -->',
    ].join('\n')
    mountGuidance({ note: noteStatus({ file: hostile }) }, (cmd) =>
      cmd === 'guidance_preview'
        ? { ...PREVIEW_REPLACE, file: hostile, proposed: hostileProposed }
        : undefined,
    )
    await openNotePreview()

    expect(noteStatusLine()).toBe(
      '<b>AGENTS</b>x.md: block current; file sha256 short 3c1e9a07; snapshots 1, pinned 0',
    )
    expect(writeBlockLines()[0]).toBe('file: <b>AGENTS</b>x.md')
    expect(proposedLines()).toEqual([
      '<!-- omnifrons:begin guidance hap-001-guidance-v1 sha256:00 -->',
      '<script>alert(1)</script>',
      '[click](http://example.invalid) <a href="http://example.invalid">x</a>',
      'line[31mwithcontrols',
      '<!-- omnifrons:end guidance -->',
    ])
    const region = guidanceRegion()
    expect(region.querySelector('b, a, script, [href]')).toBeNull()
    const walker = document.createTreeWalker(region, NodeFilter.SHOW_COMMENT)
    expect(walker.nextNode()).toBeNull()
    expect(region.textContent).not.toContain(esc)
    expect(region.textContent).not.toContain(rlo)
    expect(gateLabelText()).toBe("Type the file's short digest (3c1e9a07) to write")
  })
})

describe('AgentPanel guidance stale context (slice 5c review, R1-001)', () => {
  it('drops a guidance_preview that resolves after a workspace pick: no block re-opens bound to the previous project\'s file, digest and gate', async () => {
    let resolvePreview: (preview: GuidancePreview) => void = () => {}
    mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, (cmd) => {
      if (cmd === 'guidance_preview') {
        return new Promise<GuidancePreview>((resolve) => {
          resolvePreview = resolve
        })
      }
      if (cmd === 'workspace_pick') return { displayPath: '/home/user/other', workArea: 'valid' }
      return undefined
    })
    await waitFor(() => {
      expect(noteStatusLine()).not.toBeNull()
    })

    fireEvent.click(within(noteBlock()).getByRole('button', { name: 'Preview' }))
    await flush()
    expect(screen.queryByLabelText('Guidance write')).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    await flush()
    resolvePreview(PREVIEW_REPLACE)
    await flush()

    expect(screen.queryByLabelText('Guidance write')).toBeNull()
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('drops a guidance_preview that resolves after a different guidance file is committed, honouring the rule that a change of file closes a block bound to the previous one', async () => {
    let resolvePreview: (preview: GuidancePreview) => void = () => {}
    mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, (cmd, args) => {
      if (cmd === 'guidance_preview') {
        return new Promise<GuidancePreview>((resolve) => {
          resolvePreview = resolve
        })
      }
      if (cmd === 'guidance_status' && args.file === 'CLAUDE.md') {
        return noteStatus({ file: 'CLAUDE.md', managed: 'outdated' })
      }
      return undefined
    })
    await waitFor(() => {
      expect(noteStatusLine()).not.toBeNull()
    })

    fireEvent.click(within(noteBlock()).getByRole('button', { name: 'Preview' }))
    await flush()

    commitFile('CLAUDE.md')
    await waitFor(() => {
      expect(noteStatusLine()).toContain('CLAUDE.md')
    })
    resolvePreview(PREVIEW_REPLACE)
    await flush()

    expect(screen.queryByLabelText('Guidance write')).toBeNull()
  })

  it('drops a guidance_preview that rejects after a workspace pick: the previous project\'s refusal raises no banner over the new one', async () => {
    let rejectPreview: (reason: unknown) => void = () => {}
    mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, (cmd) => {
      if (cmd === 'guidance_preview') {
        return new Promise<GuidancePreview>((_resolve, reject) => {
          rejectPreview = reject
        })
      }
      if (cmd === 'workspace_pick') return { displayPath: '/home/user/other', workArea: 'valid' }
      return undefined
    })
    await waitFor(() => {
      expect(noteStatusLine()).not.toBeNull()
    })

    fireEvent.click(within(noteBlock()).getByRole('button', { name: 'Preview' }))
    await flush()
    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    await flush()
    rejectPreview({ code: 'guidance-block-malformed', message: 'the managed block is malformed' })
    await flush()

    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.queryByLabelText('Guidance write')).toBeNull()
    expect(
      (within(noteBlock()).getByRole('button', { name: 'Preview' }) as HTMLButtonElement).disabled,
    ).toBe(false)
  })

  it('drops a guidance_apply that resolves after a workspace pick: the previous project\'s receipt never appears under the new one, and no fetch is made for the project the section has left', async () => {
    let resolveApply: (applied: GuidanceApplied) => void = () => {}
    const calls = mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, (cmd) => {
      if (cmd === 'guidance_preview') return PREVIEW_REPLACE
      if (cmd === 'guidance_apply') {
        return new Promise<GuidanceApplied>((resolve) => {
          resolveApply = resolve
        })
      }
      if (cmd === 'workspace_pick') return { displayPath: '/home/user/other', workArea: 'valid' }
      return undefined
    })
    await openNotePreview()
    typeGate(FILE_SHORT)
    fireEvent.click(finalButton('Write'))
    await waitFor(() => {
      expect(countCalls(calls, 'guidance_apply')).toBe(1)
    })

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    await waitFor(() => {
      expect(screen.queryByLabelText('Guidance write')).toBeNull()
    })
    const statusesAfterPick = countCalls(calls, 'guidance_status')
    const snapshotsAfterPick = countCalls(calls, 'guidance_snapshots')
    resolveApply(APPLIED_REPLACE)
    await flush()

    expect(resultLine()).toBeNull()
    expect(countCalls(calls, 'guidance_status')).toBe(statusesAfterPick)
    expect(countCalls(calls, 'guidance_snapshots')).toBe(snapshotsAfterPick)
  })
})

describe('AgentPanel guidance committed file name (slice 5c review, R1-004 / R3-005)', () => {
  it('never lets a guidance file name the shell refused become the name a later automatic fetch reuses: after the refusal a Pin still asks about the committed file, and the status line, Preview, Remove and Restore all stand', async () => {
    const calls = mountGuidance(
      { note: noteStatus({ snapshots: 3, pinned: 1 }), noteSnapshots: NOTE_SNAPSHOTS },
      (cmd, args) => {
        if (cmd === 'guidance_status' && args.file === '') {
          return Promise.reject({
            code: 'guidance-file-invalid',
            message: 'the guidance file must end in .md',
          })
        }
        if (cmd === 'guidance_pin') return { ...SNAPSHOT_NEWEST, pinned: true }
        return undefined
      },
    )
    await waitFor(() => {
      expect(noteStatusLine()).not.toBeNull()
    })

    commitFile('')
    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toBe('guidance-file-invalid: the guidance file must end in .md')
    expect(noteStatusLine()).toContain('AGENTS.md: block current')

    fireEvent.click(within(snapshotRow('89abcdef01234567')).getByRole('button', { name: 'Pin' }))
    await waitFor(() => {
      expect(countCalls(calls, 'guidance_pin')).toBe(1)
    })
    await flush()

    expect(calls.filter((call) => call.cmd === 'guidance_status').at(-1)?.args).toEqual({
      kind: 'guidance',
      file: 'AGENTS.md',
    })
    expect(noteStatusLine()).toContain('AGENTS.md: block current')
    expect(
      (within(noteBlock()).getByRole('button', { name: 'Preview' }) as HTMLButtonElement).disabled,
    ).toBe(false)
    expect(
      (within(noteBlock()).getByRole('button', { name: 'Remove' }) as HTMLButtonElement).disabled,
    ).toBe(false)
    expect(
      within(snapshotRow('89abcdef01234567')).queryByRole('button', { name: 'Restore' }),
    ).not.toBeNull()
  })

  it('follows the shell\'s own answer for the committed name: once a name is accepted the automatic fetches use it, and the refused draft left in the field never reaches the wire', async () => {
    const calls = mountGuidance(
      { note: noteStatus({ snapshots: 3, pinned: 1 }), noteSnapshots: NOTE_SNAPSHOTS },
      (cmd, args) => {
        if (cmd === 'guidance_status' && args.file === 'notes.txt') {
          return Promise.reject({
            code: 'guidance-file-invalid',
            message: 'the guidance file must end in .md',
          })
        }
        if (cmd === 'guidance_status' && args.file === 'CLAUDE.md') {
          return noteStatus({ file: 'CLAUDE.md', snapshots: 3, pinned: 1 })
        }
        if (cmd === 'guidance_pin') return { ...SNAPSHOT_NEWEST, pinned: true }
        return undefined
      },
    )
    await waitFor(() => {
      expect(noteStatusLine()).not.toBeNull()
    })

    commitFile('CLAUDE.md')
    await waitFor(() => {
      expect(noteStatusLine()).toContain('CLAUDE.md')
    })
    commitFile('notes.txt')
    await screen.findByRole('alert')

    fireEvent.click(within(snapshotRow('89abcdef01234567')).getByRole('button', { name: 'Pin' }))
    await waitFor(() => {
      expect(countCalls(calls, 'guidance_pin')).toBe(1)
    })
    await flush()

    expect(calls.filter((call) => call.cmd === 'guidance_status').at(-1)?.args).toEqual({
      kind: 'guidance',
      file: 'CLAUDE.md',
    })
    expect(noteStatusLine()).toContain('CLAUDE.md')
    expect(fileInput().value).toBe('notes.txt')
  })
})

describe('AgentPanel guidance dead-end blocks (slice 5c review, R3-008)', () => {
  it('offers no Remove for a current block whose status carries no short digest: the gate could never be typed and the block would have no way out', async () => {
    mountGuidance({ note: noteStatus({ fileSha256: null, fileSha256Short: null }) })
    await waitFor(() => {
      expect(noteStatusLine()).not.toBeNull()
    })

    expect(
      (within(noteBlock()).getByRole('button', { name: 'Remove' }) as HTMLButtonElement).disabled,
    ).toBe(true)
    expect(screen.queryByLabelText('Guidance write')).toBeNull()
  })
})

describe('AgentPanel guidance write receipt (slice 5c review, R1-003 / R3-007)', () => {
  it('names the kind and the file in the receipt, and clears it when a guidance file name is committed, so a receipt can never sit under another file\'s status line', async () => {
    mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, (cmd, args) => {
      if (cmd === 'guidance_preview') return PREVIEW_REPLACE
      if (cmd === 'guidance_apply') return APPLIED_REPLACE
      if (cmd === 'guidance_status' && args.file === 'CLAUDE.md') {
        return noteStatus({ file: 'CLAUDE.md' })
      }
      return undefined
    })
    await openNotePreview()
    typeGate(FILE_SHORT)
    fireEvent.click(finalButton('Write'))

    await waitFor(() => {
      expect(resultLine()).toBe(
        'Guidance note AGENTS.md: written replace, snapshot 0123456789abcdef, result 7d7d7d7d',
      )
    })

    commitFile('CLAUDE.md')
    await waitFor(() => {
      expect(noteStatusLine()).toContain('CLAUDE.md')
    })
    expect(resultLine()).toBeNull()
  })
})

describe('AgentPanel guidance write disclosures (slice 5c review, R1-002 / R1-005)', () => {
  it('states on the removal surface what a removal takes out: the managed block, and the file itself only when nothing else remains in it and Omnifrons created it', async () => {
    mountGuidance({ note: noteStatus() })
    await waitFor(() => {
      expect(noteStatusLine()).not.toBeNull()
    })

    fireEvent.click(within(noteBlock()).getByRole('button', { name: 'Remove' }))
    await screen.findByLabelText('Guidance write')

    expect(writeBlockLines()).toEqual([
      'file: AGENTS.md',
      'action: remove',
      REMOVE_SCOPE_LINE,
      'file sha256 short: 3c1e9a07',
      SNAPSHOT_SENTENCE,
      ACT_AS_LINE,
    ])
  })

  it('states on the restore surface that the snapshot\'s bytes are not shown, and, before the gate, that a snapshot recording no file REMOVES the file', async () => {
    mountGuidance({ note: noteStatus({ snapshots: 3, pinned: 1 }), noteSnapshots: NOTE_SNAPSHOTS })
    await waitFor(() => {
      expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).not.toBeNull()
    })

    fireEvent.click(within(snapshotRow('89abcdef01234567')).getByRole('button', { name: 'Restore' }))
    await screen.findByLabelText('Guidance write')
    expect(writeBlockLines()).toEqual([
      'file: AGENTS.md',
      'action: restore',
      `snapshot: 89abcdef01234567, taken at ${new Date(1725782402000).toISOString()}, sha256 short 3c1e9a07, size 1512, existed yes`,
      RESTORE_BYTES_LINE,
      'file sha256 short: 3c1e9a07',
      SNAPSHOT_SENTENCE,
      ACT_AS_LINE,
    ])

    fireEvent.click(within(snapshotRow('0123456789abcdef')).getByRole('button', { name: 'Restore' }))
    await flush()
    expect(writeBlockLines()).toEqual([
      'file: AGENTS.md',
      'action: restore',
      `snapshot: 0123456789abcdef, taken at ${new Date(1725782401000).toISOString()}, sha256 short e3b0c442, size 0, existed no`,
      RESTORE_BYTES_LINE,
      RESTORE_REMOVES_LINE,
      'file sha256 short: 3c1e9a07',
      SNAPSHOT_SENTENCE,
      ACT_AS_LINE,
    ])
    const removes = Array.from(writeBlock().querySelectorAll('p')).find(
      (line) => line.textContent === RESTORE_REMOVES_LINE,
    )!
    const label = writeBlock().querySelector('label')!
    expect(removes.compareDocumentPosition(label) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })
})

describe('AgentPanel guidance snapshot kind (slice 5c review, R1-006)', () => {
  it('offers neither Pin nor Restore on a snapshot whose kind is not the block\'s: the block\'s facts bind the write, so a foreign kind is offered no action', async () => {
    const foreign: Snapshot = { ...SNAPSHOT_NEWEST, id: 'ffffffffffffffff', kind: 'ignore' }
    mountGuidance({
      note: noteStatus({ snapshots: 2, pinned: 1 }),
      noteSnapshots: [foreign, SNAPSHOT_OLDEST],
    })
    await waitFor(() => {
      expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).not.toBeNull()
    })

    expect(snapshotRows().map((row) => row.at(-1))).toEqual(['', 'Unpin Restore'])
  })
})

describe('AgentPanel whole-outbox listing shape (slice 5c review, R1-007)', () => {
  it('renders a candidates_list payload that is not the list the contract promises as no rows, rather than throwing mid-render and taking the panel down', async () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    mountWithWorkspace((cmd) => (cmd === 'candidates_list' ? null : undefined))
    await waitFor(() => {
      expect(listOutboxButton().disabled).toBe(false)
    })

    fireEvent.click(listOutboxButton())
    await screen.findByRole('region', { name: 'Candidates' })

    expect(candidateRows()).toEqual([])
    expect(screen.getByLabelText('Agent transcript')).toBeTruthy()
    expect(guidanceRegion()).toBeTruthy()
    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })
})

describe('AgentPanel guidance pin in flight (slice 5c review, R3-013)', () => {
  it('fires one guidance_pin per double-click: the second click lands on a pin already in flight and is refused', async () => {
    let resolvePin: (snapshot: Snapshot) => void = () => {}
    const calls = mountGuidance(
      { note: noteStatus({ snapshots: 3, pinned: 1 }), noteSnapshots: NOTE_SNAPSHOTS },
      (cmd) => {
        if (cmd === 'guidance_pin') {
          return new Promise<Snapshot>((resolve) => {
            resolvePin = resolve
          })
        }
        return undefined
      },
    )
    await waitFor(() => {
      expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).not.toBeNull()
    })

    const pin = within(snapshotRow('89abcdef01234567')).getByRole('button', { name: 'Pin' })
    fireEvent.click(pin)
    fireEvent.click(pin)
    await flush()

    expect(countCalls(calls, 'guidance_pin')).toBe(1)
    resolvePin({ ...SNAPSHOT_NEWEST, pinned: true })
    await waitFor(() => {
      expect(snapshotRows()[0]?.slice(6)).toEqual(['yes', 'Unpin Restore'])
    })
    expect(
      (within(snapshotRow('89abcdef01234567')).getByRole('button', {
        name: 'Unpin',
      }) as HTMLButtonElement).disabled,
    ).toBe(false)
  })
})

describe('AgentPanel guidance rejections beyond apply (slice 5c review, R3-003 / R3-004)', () => {
  it.each([
    {
      command: 'guidance_remove',
      button: 'Remove block' as const,
      code: 'guidance-block-modified',
      message:
        'the managed block was modified inside its sentinels; resolve it by hand or restore a snapshot',
      closes: false,
    },
    {
      command: 'guidance_remove',
      button: 'Remove block' as const,
      code: 'guidance-file-changed',
      message: 'the managed file changed since it was shown; read its status again and retry',
      closes: true,
    },
    {
      command: 'guidance_restore',
      button: 'Restore snapshot' as const,
      code: 'snapshot-unavailable',
      message: 'the snapshot store could not be written',
      closes: false,
    },
    {
      command: 'guidance_restore',
      button: 'Restore snapshot' as const,
      code: 'guidance-file-changed',
      message: 'the managed file changed since it was shown; read its status again and retry',
      closes: true,
    },
  ])(
    'renders $code from $command through the banner as "<code>: <message>" with no "untrusted"; the block stays open with the typed digest kept and the button enabled again -- except guidance-file-changed, which closes it and refetches that kind\'s status and snapshots',
    async ({ command, button, code, message, closes }) => {
      const calls = mountGuidance(
        { note: noteStatus({ snapshots: 3, pinned: 1 }), noteSnapshots: NOTE_SNAPSHOTS },
        (cmd) => (cmd === command ? Promise.reject({ code, message }) : undefined),
      )
      await waitFor(() => {
        expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).not.toBeNull()
      })

      if (command === 'guidance_remove') {
        fireEvent.click(within(noteBlock()).getByRole('button', { name: 'Remove' }))
      } else {
        fireEvent.click(
          within(snapshotRow('89abcdef01234567')).getByRole('button', { name: 'Restore' }),
        )
      }
      await screen.findByLabelText('Guidance write')
      const statusesBefore = countCalls(calls, 'guidance_status')
      const snapshotsBefore = countCalls(calls, 'guidance_snapshots')
      typeGate(FILE_SHORT)
      fireEvent.click(finalButton(button))

      const alert = await screen.findByRole('alert')
      expect(alert.textContent).toBe(`${code}: ${message}`)
      expect(alert.textContent).not.toContain('untrusted')
      if (closes) {
        expect(screen.queryByLabelText('Guidance write')).toBeNull()
        await waitFor(() => {
          expect(countCalls(calls, 'guidance_status')).toBe(statusesBefore + 1)
        })
        expect(countCalls(calls, 'guidance_snapshots')).toBe(snapshotsBefore + 1)
      } else {
        expect(screen.getByLabelText('Guidance write')).toBeTruthy()
        expect(gateInput().value).toBe(FILE_SHORT)
        await waitFor(() => {
          expect(finalButton(button).disabled).toBe(false)
        })
        expect(countCalls(calls, 'guidance_status')).toBe(statusesBefore)
        expect(countCalls(calls, 'guidance_snapshots')).toBe(snapshotsBefore)
      }
    },
  )

  it('renders a guidance_pin rejection through the banner as "<code>: <message>", leaves the row\'s flag as it was, refetches nothing, and offers Pin again', async () => {
    const calls = mountGuidance(
      { note: noteStatus({ snapshots: 3, pinned: 1 }), noteSnapshots: NOTE_SNAPSHOTS },
      (cmd) =>
        cmd === 'guidance_pin'
          ? Promise.reject({
              code: 'guidance-unmanaged',
              message: 'no snapshot with that id is recorded for this project',
            })
          : undefined,
    )
    await waitFor(() => {
      expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).not.toBeNull()
    })
    const statusesBefore = countCalls(calls, 'guidance_status')
    const snapshotsBefore = countCalls(calls, 'guidance_snapshots')

    fireEvent.click(within(snapshotRow('89abcdef01234567')).getByRole('button', { name: 'Pin' }))

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toBe(
      'guidance-unmanaged: no snapshot with that id is recorded for this project',
    )
    expect(alert.textContent).not.toContain('untrusted')
    expect(snapshotRows()[0]?.slice(6)).toEqual(['no', 'Pin Restore'])
    expect(countCalls(calls, 'guidance_status')).toBe(statusesBefore)
    expect(countCalls(calls, 'guidance_snapshots')).toBe(snapshotsBefore)
    expect(
      (within(snapshotRow('89abcdef01234567')).getByRole('button', {
        name: 'Pin',
      }) as HTMLButtonElement).disabled,
    ).toBe(false)
  })

  it('renders a guidance_preview rejected with a ShellError as "<code>: <message>" and leaves no block open', async () => {
    mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, (cmd) =>
      cmd === 'guidance_preview'
        ? Promise.reject({
            code: 'guidance-block-malformed',
            message:
              "the managed block's sentinels are not one intact pair; resolve it by hand or restore a snapshot",
          })
        : undefined,
    )
    await waitFor(() => {
      expect(noteStatusLine()).not.toBeNull()
    })

    fireEvent.click(within(noteBlock()).getByRole('button', { name: 'Preview' }))

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toBe(
      "guidance-block-malformed: the managed block's sentinels are not one intact pair; resolve it by hand or restore a snapshot",
    )
    expect(alert.textContent).not.toContain('untrusted')
    expect(screen.queryByLabelText('Guidance write')).toBeNull()
    await waitFor(() => {
      expect(
        (within(noteBlock()).getByRole('button', { name: 'Preview' }) as HTMLButtonElement).disabled,
      ).toBe(false)
    })
  })
})

describe('AgentPanel guidance proposed-block lines (slice 5c review, R3-006)', () => {
  it.each([
    {
      label: 'a trailing newline ends the last line rather than opening an empty one',
      proposed: 'first\nsecond\n',
      lines: ['first', 'second'],
    },
    {
      label: 'an interior empty line is kept, since it is one',
      proposed: 'first\n\nthird\n',
      lines: ['first', '', 'third'],
    },
    {
      label: 'a proposal with no trailing newline keeps its last line',
      proposed: 'first\nsecond',
      lines: ['first', 'second'],
    },
    { label: 'an empty proposal yields no lines at all', proposed: '', lines: [] },
    {
      label: "a CRLF file's carriage return is stripped by PlainTextLine, the control it is",
      proposed: 'first\r\nsecond\r\n',
      lines: ['first', 'second'],
    },
  ])('splits the proposed block into display lines: $label', async ({ proposed, lines }) => {
    mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, (cmd) =>
      cmd === 'guidance_preview' ? { ...PREVIEW_REPLACE, proposed } : undefined,
    )
    await openNotePreview()

    expect(proposedLines()).toEqual(lines)
    expect(writeBlock().textContent).not.toContain('\r')
  })
})

describe('AgentPanel guidance per-kind independence (slice 5c review, R3-009)', () => {
  const IGNORE_SNAPSHOT: Snapshot = {
    id: '1111222233334444',
    kind: 'ignore',
    file: '.gitignore',
    existed: true,
    sha256Short: IGNORE_SHORT,
    size: 90,
    takenAt: 1725782403000,
    pinned: false,
  }

  it("renders both kinds' snapshot tables at once and keeps them independent: a pin on one kind rewrites that kind's row and refetches that kind's status alone, leaving the other kind's table and status untouched", async () => {
    const calls = mountGuidance(
      {
        note: noteStatus({ snapshots: 3, pinned: 1 }),
        noteSnapshots: NOTE_SNAPSHOTS,
        ignore: IGNORE_STATUS_CURRENT,
        ignoreSnapshots: [IGNORE_SNAPSHOT],
      },
      (cmd, args) =>
        cmd === 'guidance_pin'
          ? { ...IGNORE_SNAPSHOT, pinned: args.pinned as boolean }
          : undefined,
    )
    await waitFor(() => {
      expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).not.toBeNull()
    })
    await waitFor(() => {
      expect(screen.queryByRole('table', { name: 'Ignore rule snapshots' })).not.toBeNull()
    })

    expect(snapshotRows('Guidance note snapshots')).toHaveLength(3)
    expect(snapshotRows('Ignore rule snapshots')).toHaveLength(1)
    const noteRowsBefore = snapshotRows('Guidance note snapshots')
    const noteLineBefore = noteStatusLine()
    const statuses = () => calls.filter((call) => call.cmd === 'guidance_status')
    const before = statuses().length

    fireEvent.click(
      within(screen.getByRole('table', { name: 'Ignore rule snapshots' })).getByRole('button', {
        name: 'Pin',
      }),
    )

    await waitFor(() => {
      expect(snapshotRows('Ignore rule snapshots')[0]?.[6]).toBe('yes')
    })
    expect(snapshotRows('Guidance note snapshots')).toEqual(noteRowsBefore)
    expect(noteStatusLine()).toBe(noteLineBefore)
    expect(statuses().slice(before).map((call) => call.args)).toEqual([{ kind: 'ignore' }])
  })
})

describe('AgentPanel guidance same-file Restore (slice 5c review, R3-012)', () => {
  it("gives a snapshot of another guidance file its Restore once that file is the one the status line shows, and takes it from the rows of the file the section has left", async () => {
    mountGuidance(
      { note: noteStatus({ snapshots: 3, pinned: 1 }), noteSnapshots: NOTE_SNAPSHOTS },
      (cmd, args) =>
        cmd === 'guidance_status' && args.file === 'CLAUDE.md'
          ? noteStatus({ file: 'CLAUDE.md', snapshots: 3, pinned: 1 })
          : undefined,
    )
    await waitFor(() => {
      expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).not.toBeNull()
    })
    expect(snapshotRows().map((row) => row.at(-1))).toEqual([
      'Pin Restore',
      'Unpin Restore',
      'Pin',
    ])

    commitFile('CLAUDE.md')

    await waitFor(() => {
      expect(noteStatusLine()).toContain('CLAUDE.md')
    })
    expect(snapshotRows().map((row) => row.at(-1))).toEqual(['Pin', 'Unpin', 'Pin Restore'])
  })
})

describe('AgentPanel guidance absent-file gate (slice 5c review, R3-016)', () => {
  it("enables Write for a file the preview showed as absent only on an exact match of the proposed block's short digest: never the gate verb, uppercase hex, a 7- or 9-character prefix, a leading or trailing space, the empty string, or the digest of some other file", async () => {
    mountGuidance({ note: NOTE_ABSENT }, (cmd) =>
      cmd === 'guidance_preview' ? notePreview() : undefined,
    )
    await openNotePreview()
    expect(gateLabelText()).toBe(
      "Type the proposed block's short digest (e5a1b2c3) to create the file",
    )

    for (const wrong of [
      'create the file',
      'E5A1B2C3',
      'e5a1b2c',
      'e5a1b2c3a',
      ' e5a1b2c3',
      'e5a1b2c3 ',
      '',
      FILE_SHORT,
      IGNORE_SHORT,
    ]) {
      typeGate(wrong)
      expect(finalButton('Write').disabled).toBe(true)
    }
    typeGate('e5a1b2c3')
    expect(finalButton('Write').disabled).toBe(false)
  })
})

// -- Slice 5d: wrong roots, `misplaced`, and its three remedies --

/** HAP-001-R34's disclosure, carried in every mode including `sandbox-enforced`. */
const IN_PROJECT_DISCLOSURE =
  'a write inside the project but outside the outbox is detected after the run, never prevented'

/** HAP-001-R33's additional disclosure, carried under `advisory` scope only. */
const OUTSIDE_PROJECT_DISCLOSURE =
  'a write outside the project is possible and is detected after the run, not prevented'

/** `wrongroot_status` for a project no scan has run over yet, under the advisory scope every built-in adapter declares. */
function wrongRootStatusAdvisory(overrides: Partial<WrongRootStatus> = {}): WrongRootStatus {
  return {
    outputDiscipline: 'advisory',
    scopeMode: 'advisory',
    disclosures: [IN_PROJECT_DISCLOSURE, OUTSIDE_PROJECT_DISCLOSURE],
    scanned: false,
    findings: 0,
    ...overrides,
  }
}

/** The report a hypothetical `sandbox-enforced` catalog produces: HAP-001-R34's disclosure alone, never zero. */
const WRONG_ROOT_STATUS_ENFORCED: WrongRootStatus = {
  outputDiscipline: 'enforced',
  scopeMode: 'sandbox-enforced',
  disclosures: [IN_PROJECT_DISCLOSURE],
  scanned: true,
  findings: 2,
}

function wrongRootsRegion(): HTMLElement {
  return screen.getByRole('region', { name: 'Wrong roots' })
}

function outputDisciplineLine(): string | null {
  return screen.queryByRole('status', { name: 'Output discipline' })?.textContent ?? null
}

function disclosureLines(): (string | null)[] {
  const list = screen.queryByRole('list', { name: 'Output discipline disclosures' })
  if (!list) return []
  return Array.from(list.querySelectorAll('li')).map((line) => line.textContent)
}

/**
 * Mounts over a workspace whose wrong-root answers are `mock`'s, with a
 * fresh project's for anything unspecified, and `onCommand` answering the
 * rest -- a scan, a remedy, a spawn -- first.
 */
function mountWrongRoots(
  mock: { status?: WrongRootStatus | (() => unknown); rows?: MisplacedRow[] | (() => unknown) } = {},
  onCommand?: (cmd: string, args: Record<string, unknown>) => unknown,
): Call[] {
  return mountWithWorkspace((cmd, args) => {
    const own = onCommand?.(cmd, args)
    if (own !== undefined) return own
    if (cmd === 'wrongroot_status') {
      return mock.status === undefined ? undefined : answerMock(mock.status)
    }
    if (cmd === 'misplaced_list') {
      return mock.rows === undefined ? undefined : answerMock(mock.rows)
    }
    return undefined
  })
}

describe('AgentPanel wrong roots section (slice 5d, HAP-001-R32, R33, R34)', () => {
  it('renders a Wrong roots region beside the transcript, the Candidates region and the Guidance region -- never inside any of them (RCS-001-R6) -- opening with the advisory output-discipline line and every disclosure, one line each', async () => {
    mountWrongRoots()

    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })
    expect(outputDisciplineLine()).toBe(
      'output discipline: advisory (scope advisory) — a write outside the project root is not prevented; not scanned yet',
    )
    expect(disclosureLines()).toEqual([IN_PROJECT_DISCLOSURE, OUTSIDE_PROJECT_DISCLOSURE])

    const region = wrongRootsRegion()
    expect(screen.getByLabelText('Agent transcript').contains(region)).toBe(false)
    expect(screen.getByRole('region', { name: 'Guidance' }).contains(region)).toBe(false)
    expect(region.contains(screen.getByRole('region', { name: 'Guidance' }))).toBe(false)
  })

  it("renders the enforced report as a declaration the product does not verify -- never as a prevention it has confirmed, which would contradict this panel's own advisory badge -- with HAP-001-R34's disclosure alone beside the scan counts", async () => {
    mountWrongRoots({ status: WRONG_ROOT_STATUS_ENFORCED })

    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })
    expect(outputDisciplineLine()).toBe(
      'output discipline: enforced (scope sandbox-enforced) — the adapter declares that a write outside the project root is prevented; the product does not verify that claim; scanned, findings 2',
    )
    // The claim the old copy made, and the reason it could not stand: the
    // discipline is derived from what adapters *declare*, nothing here
    // checks it, and the panel's own badge says the opposite two regions
    // above. Both halves are asserted -- that the surface no longer claims
    // prevention, and that the badge it would have contradicted is present.
    expect(outputDisciplineLine()).not.toContain('is prevented by the sandbox')
    expect(screen.getByText('advisory scope — not a sandbox')).toBeTruthy()
    expect(disclosureLines()).toEqual([IN_PROJECT_DISCLOSURE])
  })

  it('renders every disclosure the report carries as text, however many and whatever they say: none is collapsed, truncated or hidden behind a toggle, and markup in one stays literal', async () => {
    const extra = 'a third disclosure a later contract adds <b>bold</b>'
    mountWrongRoots({
      status: wrongRootStatusAdvisory({
        disclosures: [IN_PROJECT_DISCLOSURE, OUTSIDE_PROJECT_DISCLOSURE, extra],
      }),
    })

    await waitFor(() => {
      expect(disclosureLines()).toHaveLength(3)
    })
    expect(disclosureLines()).toEqual([IN_PROJECT_DISCLOSURE, OUTSIDE_PROJECT_DISCLOSURE, extra])
    const region = wrongRootsRegion()
    expect(region.querySelector('b')).toBeNull()
    expect(region.querySelector('details')).toBeNull()
  })
})

function outputDisciplineUnavailableLine(): string | null {
  return (
    screen.queryByRole('status', { name: 'Output discipline unavailable' })?.textContent ?? null
  )
}

describe('AgentPanel disclosures do not depend on a fetch (slice 5d review, R1-010 / R3-026)', () => {
  it('states HAP-001-R33 and R34 even when wrongroot_status rejects: the report is the thing that failed, and a disclosure a failed fetch can remove is not a disclosure', async () => {
    // Measured before the fix, with the status rejecting: no discipline
    // line, no disclosures, and no banner -- an automatic refresh is not a
    // user act, so the rejection is silent by design -- while Scan stayed
    // enabled and Quarantine stayed offered. The product went on offering
    // the acts and stopped saying what it does not prevent.
    mountWrongRoots({ status: () => Promise.reject({ code: 'unexpected', message: 'gone' }) })

    await waitFor(() => {
      expect(outputDisciplineUnavailableLine()).not.toBeNull()
    })
    expect(disclosureLines()).toEqual([IN_PROJECT_DISCLOSURE, OUTSIDE_PROJECT_DISCLOSURE])
    // The absence of the report is itself stated, rather than left as a
    // blank space that reads like a clean report.
    expect(outputDisciplineUnavailableLine()).toBe(
      'output discipline: not reported — the report could not be read; the disclosures below hold in every mode',
    )
    expect(outputDisciplineLine()).toBeNull()
    // And the surface is still offering the acts, which is exactly why the
    // disclosures have to be there.
    expect(scanButton().disabled).toBe(false)
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('falls back to the advisory pair -- never to silence, and never to the enforced line alone -- for a report that arrives carrying no disclosures at all', async () => {
    mountWrongRoots({ status: wrongRootStatusAdvisory({ disclosures: [] }) })

    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })
    // With no report to go on, the product cannot claim an enforcement
    // nobody told it about, so it states the weaker, safer pair.
    expect(disclosureLines()).toEqual([IN_PROJECT_DISCLOSURE, OUTSIDE_PROJECT_DISCLOSURE])
  })

  it('offers no quarantine without the disclosures standing beside it: with the status rejecting, a finding is still listed and remediable, and both disclosures are on the page', async () => {
    mountWrongRoots({
      status: () => Promise.reject({ code: 'unexpected', message: 'gone' }),
      rows: [MISPLACED_REPORT],
    })
    await screen.findByRole('table', { name: 'Misplaced files' })

    expect(remedyButtons('docs/report.pdf')).toEqual(['Quarantine', 'Publish to outbox', 'Ignore'])
    expect(disclosureLines()).toEqual([IN_PROJECT_DISCLOSURE, OUTSIDE_PROJECT_DISCLOSURE])
  })

  it('strips control and bidi-override characters from a disclosure the report carries, which React\'s own escaping would not have done', async () => {
    // The earlier content-security assertion here could not tell
    // `PlainTextLine` from React's default escaping, because markup is all
    // it tested and React escapes markup on its own. An ESC byte and a
    // U+202E override are the fixture that separates them -- the same one
    // the row-name test uses.
    const esc = String.fromCharCode(0x1b)
    const rlo = String.fromCodePoint(0x202e)
    const hostile = `a later contract's <b>disclosure</b>${esc}[31m${rlo}denrevog`
    mountWrongRoots({ status: wrongRootStatusAdvisory({ disclosures: [hostile] }) })

    await waitFor(() => {
      expect(disclosureLines()).toHaveLength(1)
    })
    const list = screen.getByRole('list', { name: 'Output discipline disclosures' })
    expect(list.textContent).toContain("a later contract's <b>disclosure</b>")
    expect(list.querySelector('b')).toBeNull()
    expect(list.textContent).not.toContain(esc)
    expect(list.textContent).not.toContain(rlo)
  })
})

/** One `misplaced_list` row: a project-relative name, the facts from its own handle, the three remedies. */
function misplacedRow(overrides: Partial<MisplacedRow> = {}): MisplacedRow {
  return {
    name: 'docs/report.pdf',
    size: 4096,
    sha256: 'ab'.repeat(32),
    sha256Short: 'abababab',
    detectedType: 'pdf',
    class: 'generated-heavy',
    reason: 'in-project-outside-outbox',
    remedies: ['quarantine', 'publish', 'ignore'],
    ...overrides,
  }
}

const MISPLACED_REPORT = misplacedRow()
const MISPLACED_BUNDLE = misplacedRow({
  name: 'build/bundle.zip',
  size: 90210,
  sha256: 'cd'.repeat(32),
  sha256Short: 'cdcdcdcd',
  detectedType: 'zip',
})

/** `wrongroot_scan`'s answer for a walk that saw twelve files and left two findings standing. */
const SCAN_SUMMARY: ScanSummary = {
  scanned: 12,
  findings: 2,
  ignored: 1,
  excluded: 3,
  unreadable: 0,
  truncated: false,
}

function scanButton(): HTMLButtonElement {
  return screen.getByRole('button', { name: 'Scan' }) as HTMLButtonElement
}

function scanLine(): string | null {
  return screen.queryByRole('status', { name: 'Wrong roots scan' })?.textContent ?? null
}

describe('AgentPanel wrong roots scan (slice 5d)', () => {
  it('clicking Scan calls wrongroot_scan with exactly {} and renders every count of the answer in a fixed order, zeros included, then refetches the status and the findings', async () => {
    const calls = mountWrongRoots({ rows: [] }, (cmd) =>
      cmd === 'wrongroot_scan' ? SCAN_SUMMARY : undefined,
    )
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })
    expect(scanLine()).toBeNull()
    const before = calls.length

    fireEvent.click(scanButton())

    await waitFor(() => {
      expect(scanLine()).not.toBeNull()
    })
    expect(scanLine()).toBe('scan: 12 scanned, 2 findings, 1 ignored, 3 excluded, 0 unreadable')
    const after = calls.slice(before).map((call) => call.cmd)
    expect(after[0]).toBe('wrongroot_scan')
    expect(calls[before]?.args).toEqual({})
    expect(after).toContain('misplaced_list')
    expect(after).toContain('wrongroot_status')
  })

  it('says in fixed copy that a truncated scan stopped early and its list is incomplete, never letting a partial walk read as a complete one', async () => {
    mountWrongRoots({ rows: [] }, (cmd) =>
      cmd === 'wrongroot_scan' ? { ...SCAN_SUMMARY, scanned: 20000, truncated: true } : undefined,
    )
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })

    fireEvent.click(scanButton())

    await waitFor(() => {
      expect(scanLine()).not.toBeNull()
    })
    expect(scanLine()).toBe(
      'scan: 20000 scanned, 2 findings, 1 ignored, 3 excluded, 0 unreadable — the scan stopped early; the list is incomplete',
    )
  })

  it('stays live while a run is active -- the scan observes, holds no handle and takes no lock, exactly as the shell\'s own run-end scan does -- and its click actually calls wrongroot_scan then', async () => {
    let channel: LiveChannel | undefined
    const calls = mountWrongRoots({ rows: [] }, (cmd, args) => {
      if (cmd === 'harness_spawn') {
        channel = (args as { onFrame: LiveChannel }).onFrame
        return 7
      }
      if (cmd === 'wrongroot_scan') return SCAN_SUMMARY
      return undefined
    })
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })

    await startRunOverWorkspace()

    expect(scanButton().disabled).toBe(false)
    const before = countCalls(calls, 'wrongroot_scan')
    fireEvent.click(scanButton())

    await waitFor(() => {
      expect(countCalls(calls, 'wrongroot_scan')).toBe(before + 1)
    })
    expect(scanLine()).toBe('scan: 12 scanned, 2 findings, 1 ignored, 3 excluded, 0 unreadable')

    if (!channel) throw new Error('harness_spawn was not called')
    await endRun(channel)
  })

  it('fires one wrongroot_scan per double-click: the second click lands on a scan already in flight and is refused', async () => {
    let settle: (value: unknown) => void = () => {}
    const calls = mountWrongRoots({ rows: [] }, (cmd) =>
      cmd === 'wrongroot_scan'
        ? new Promise((resolve) => {
            settle = resolve as (value: unknown) => void
          })
        : undefined,
    )
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })

    fireEvent.click(scanButton())
    fireEvent.click(scanButton())

    await waitFor(() => {
      expect(countCalls(calls, 'wrongroot_scan')).toBe(1)
    })
    await act(async () => {
      settle(SCAN_SUMMARY)
      await Promise.resolve()
    })
    expect(countCalls(calls, 'wrongroot_scan')).toBe(1)
  })
})

function misplacedTable(): HTMLElement {
  return screen.getByRole('table', { name: 'Misplaced files' })
}

/** The Misplaced table's data rows, each as its cells' text. */
function misplacedRows(): string[][] {
  return Array.from(misplacedTable().querySelectorAll('tbody tr')).map((row) =>
    Array.from(row.querySelectorAll('td')).map((cell) => cell.textContent ?? ''),
  )
}

/** The Misplaced table's data row whose name cell reads `name`. */
function misplacedTableRow(name: string): HTMLElement {
  const row = Array.from(misplacedTable().querySelectorAll<HTMLElement>('tbody tr')).find(
    (candidate) => candidate.querySelector('td')?.textContent === name,
  )
  if (!row) throw new Error(`no misplaced row named ${name}`)
  return row
}

/** The remedy buttons offered for the row named `name`, in the order the row offers them. */
function remedyButtons(name: string): string[] {
  return Array.from(misplacedTableRow(name).querySelectorAll('button')).map(
    (button) => button.textContent ?? '',
  )
}

describe('AgentPanel misplaced table (slice 5d, HAP-001-R32)', () => {
  it('renders one row per finding in walk order -- name, class, type, size, short digest, reason as fixed copy -- with the full digest nowhere in the table and no table at all while nothing is standing', async () => {
    mountWrongRoots({ rows: [] })
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })
    expect(screen.queryByRole('table', { name: 'Misplaced files' })).toBeNull()
    cleanup()
    clearMocks()

    mountWrongRoots({ rows: [MISPLACED_REPORT, MISPLACED_BUNDLE] })
    await screen.findByRole('table', { name: 'Misplaced files' })

    expect(misplacedRows().map((cells) => cells.slice(0, 6))).toEqual([
      [
        'docs/report.pdf',
        'generated-heavy',
        'pdf',
        '4096',
        'abababab',
        'inside the project, outside the outbox',
      ],
      [
        'build/bundle.zip',
        'generated-heavy',
        'zip',
        '90210',
        'cdcdcdcd',
        'inside the project, outside the outbox',
      ],
    ])
    expect(misplacedTable().textContent).not.toContain('ab'.repeat(32))
  })

  it("never resolves a finding's name as a path or a link: a name carrying markup, a C0 control and a bidi override renders as the text it is, with no anchor and no href anywhere in the region", async () => {
    const esc = String.fromCharCode(0x1b)
    const rlo = String.fromCodePoint(0x202e)
    const hostile = `docs/<b>x</b>${esc}[31m${rlo}fdp.report`
    mountWrongRoots({ rows: [misplacedRow({ name: hostile })] })
    await screen.findByRole('table', { name: 'Misplaced files' })

    const region = wrongRootsRegion()
    expect(region.textContent).toContain('docs/<b>x</b>')
    expect(region.querySelector('b')).toBeNull()
    expect(region.querySelector('a')).toBeNull()
    expect(region.querySelector('[href]')).toBeNull()
    expect(region.textContent).not.toContain(esc)
    expect(region.textContent).not.toContain(rlo)
  })

  it("offers exactly the remedies the row's own remedies array carries, in its order, never a set of its own: a row offering one offers one button and a row offering none offers none", async () => {
    mountWrongRoots({
      rows: [
        MISPLACED_REPORT,
        misplacedRow({ name: 'a.pdf', sha256Short: '11111111', remedies: ['ignore'] }),
        misplacedRow({ name: 'b.pdf', sha256Short: '22222222', remedies: [] }),
      ],
    })
    await screen.findByRole('table', { name: 'Misplaced files' })

    expect(remedyButtons('docs/report.pdf')).toEqual(['Quarantine', 'Publish to outbox', 'Ignore'])
    expect(remedyButtons('a.pdf')).toEqual(['Ignore'])
    expect(remedyButtons('b.pdf')).toEqual([])
  })
})

const QUARANTINE_SCOPE_LINE =
  'the file will be MOVED out of the project into the quarantine directory, outside any workspace; this product then offers no way to list, open or restore it'

const COPY_NEEDS_APPROVAL_SENTENCE =
  'the copy is not published: approve it in the outbox listing like any other entry'

const IGNORE_IS_PERMANENT_SENTENCE =
  'this decision cannot be undone or reviewed from this product: nothing here lists what has been ignored'

function quarantineBlock(): HTMLElement {
  return screen.getByLabelText('Quarantine confirmation')
}

function quarantineBlockLines(): (string | null)[] {
  return Array.from(quarantineBlock().querySelectorAll('p')).map((line) => line.textContent)
}

function quarantineGate(): HTMLInputElement {
  return screen.getByLabelText(/^Type the short digest \(.*\) to quarantine$/) as HTMLInputElement
}

function quarantineButton(): HTMLButtonElement {
  return screen.getByRole('button', { name: 'Quarantine file' }) as HTMLButtonElement
}

function remedyResultLine(): string | null {
  return screen.queryByRole('status', { name: 'Remedy result' })?.textContent ?? null
}

/** Why the findings table is empty, when it is empty for a reason other than "nothing is misplaced". */
function misplacedUnavailableLine(): string | null {
  return screen.queryByRole('status', { name: 'Misplaced files unavailable' })?.textContent ?? null
}

/**
 * Why the findings table that *is* on the screen is short of the listing --
 * a separate accessible name from the one above, because "there is no
 * table" and "this table is incomplete" are different facts and a screen
 * reader is told which one it has (R3-047).
 */
function misplacedIncompleteLine(): string | null {
  return screen.queryByRole('status', { name: 'Misplaced files incomplete' })?.textContent ?? null
}

/** Mounts over one standing finding and clicks its Quarantine, returning the recorded calls. */
async function openQuarantineBlock(
  onCommand?: (cmd: string, args: Record<string, unknown>) => unknown,
): Promise<Call[]> {
  const calls = mountWrongRoots({ rows: [MISPLACED_REPORT] }, onCommand)
  await screen.findByRole('table', { name: 'Misplaced files' })
  fireEvent.click(within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Quarantine' }))
  await screen.findByLabelText('Quarantine confirmation')
  return calls
}

describe('AgentPanel quarantine remedy (slice 5d, HAP-001-R32; TM-001-R1/R7)', () => {
  it("opens a confirmation block bound to the row -- its identity facts, the sentence saying the file will be MOVED out of the project and that the product then offers no way back to it, the act-as line, a gate naming the short digest, an empty input and a disabled button -- invoking nothing", async () => {
    const calls = await openQuarantineBlock()

    expect(calls.filter((call) => call.cmd === 'misplaced_remedy')).toHaveLength(0)
    // The claim the line used to make. `docs/spike-log.md` § Slice 5d
    // records reveal-only as honored by omission -- no flag is set, no
    // permission is changed, and nothing lists a quarantined file -- so the
    // block must not promise the file was left in a particular state.
    expect(quarantineBlock().textContent).not.toContain('reveal-only')
    expect(quarantineBlock().textContent).not.toContain('left unexecuted')
    // Nor any claim about the file's own permissions: the copy path creates
    // its destination `0o600` on unix while the rename path keeps whatever
    // mode the file had, so no single sentence about them is true of a
    // quarantine in general.
    expect(quarantineBlock().textContent).not.toContain('permits')
    expect(quarantineBlockLines()).toEqual([
      'name: docs/report.pdf',
      'class: generated-heavy',
      'type: pdf',
      'size: 4096',
      `sha256: ${'ab'.repeat(32)}`,
      'sha256 short: abababab',
      'reason: inside the project, outside the outbox',
      QUARANTINE_SCOPE_LINE,
      'act as: device-local-user',
    ])
    expect(wrongRootsRegion().contains(quarantineBlock())).toBe(true)
    expect(screen.getByLabelText('Agent transcript').contains(quarantineBlock())).toBe(false)
    expect(quarantineGate().value).toBe('')
    expect(quarantineButton().disabled).toBe(true)
  })

  it("enables the button only on an exact, case-sensitive match of the row's own short digest, and the click then calls misplaced_remedy with exactly { name, sha256, remedy } carrying the row's full digest -- never the typed value", async () => {
    const calls = await openQuarantineBlock((cmd) =>
      cmd === 'misplaced_remedy'
        ? {
            remedy: 'quarantine',
            outcome: 'quarantined',
            name: 'abababab-report.pdf',
            sha256: 'ab'.repeat(32),
            originalKept: false,
            detail: 'renamed',
          }
        : undefined,
    )

    for (const wrong of [
      'quarantine',
      'ABABABAB',
      'abababa',
      'abababab1',
      ' abababab',
      'abababab ',
      '',
      'ab'.repeat(32),
      'cdcdcdcd',
    ]) {
      fireEvent.change(quarantineGate(), { target: { value: wrong } })
      expect(quarantineButton().disabled).toBe(true)
    }
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
    expect(quarantineButton().disabled).toBe(false)

    fireEvent.click(quarantineButton())

    await waitFor(() => {
      expect(countCalls(calls, 'misplaced_remedy')).toBe(1)
    })
    const request = calls.find((call) => call.cmd === 'misplaced_remedy')
    expect(request?.args).toEqual({
      name: 'docs/report.pdf',
      sha256: 'ab'.repeat(32),
      remedy: 'quarantine',
    })
    expect(Object.keys(request!.args).sort()).toEqual(['name', 'remedy', 'sha256'])
  })

  it('nothing harness- or project-originated pre-fills or enables the gate (TM-001-R1): the digest the label itself names, a finding whose own name is that digest, and a programmatic input.value with no input event all leave the input empty and the button disabled; only typing enables it', async () => {
    const calls = mountWrongRoots({
      rows: [MISPLACED_REPORT, misplacedRow({ name: 'abababab', sha256Short: '33333333' })],
    })
    await screen.findByRole('table', { name: 'Misplaced files' })
    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Quarantine' }),
    )
    await screen.findByLabelText('Quarantine confirmation')

    // The gate's own label names the digest it wants, and a row beside it
    // is named the very same string: neither reaches the input.
    expect(quarantineBlock().querySelector('label')?.textContent).toBe(
      'Type the short digest (abababab) to quarantine',
    )
    expect(quarantineGate().value).toBe('')
    expect(quarantineButton().disabled).toBe(true)

    quarantineGate().value = 'abababab'
    await flush()
    expect(quarantineGate().value).toBe('abababab')
    expect(quarantineButton().disabled).toBe(true)
    // (React's value tracker reports a change event carrying the string
    // already set on the node as no change, so the typed sequence passes
    // through another value first -- a test-harness detail.)
    fireEvent.change(quarantineGate(), { target: { value: '' } })
    expect(quarantineButton().disabled).toBe(true)
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
    expect(quarantineButton().disabled).toBe(false)
    expect(countCalls(calls, 'misplaced_remedy')).toBe(0)
  })
})

describe('AgentPanel remedy outcomes (slice 5d, HAP-001-R28, R32)', () => {
  it("makes plain that a completed quarantine took the original out of the project, names what it produced and says which move ran, then refetches the findings and the status", async () => {
    const calls = await openQuarantineBlock((cmd) =>
      cmd === 'misplaced_remedy'
        ? {
            remedy: 'quarantine',
            outcome: 'quarantined',
            name: 'abababab-report.pdf',
            sha256: 'ab'.repeat(32),
            originalKept: false,
            detail: 'renamed',
          }
        : undefined,
    )
    const before = calls.length
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
    fireEvent.click(quarantineButton())

    await waitFor(() => {
      expect(remedyResultLine()).not.toBeNull()
    })
    expect(remedyResultLine()).toBe(
      'quarantine: moved into quarantine, outside any workspace; name abababab-report.pdf; the original is gone from the project; moved by rename',
    )
    expect(screen.queryByLabelText('Quarantine confirmation')).toBeNull()
    const after = calls.slice(before).map((call) => call.cmd)
    expect(after).toContain('misplaced_list')
    expect(after).toContain('wrongroot_status')
  })

  it("says only that the remedy did not remove the original for each of HAP-001-R19's four residual details -- never that the file is still in the project, which three of the four contradict -- so the receipt and its own detail line cannot disagree", async () => {
    // `originalKept: true` does not mean the file is where the user left
    // it. The shell sets it for `original-already-gone` (the original is
    // gone), `original-changed-during-the-move` (something else is at that
    // name now) and `identity-check-unavailable` (nobody knows), because
    // what it records is that *this remedy* did not remove it. The receipt
    // used to read `the original is still in the project` beside each of
    // those, contradicting the detail sentence on the very same line.
    const expected: Record<string, string> = {
      'unlink-failed': 'the copy stands and the original could not be removed',
      'original-already-gone': 'the copy stands and the original was already gone',
      'original-changed-during-the-move':
        'the copy stands and the original changed during the move',
      'identity-check-unavailable':
        'the copy stands and this platform cannot prove the name still holds it',
    }
    for (const [detail, copy] of Object.entries(expected)) {
      await openQuarantineBlock((cmd) =>
        cmd === 'misplaced_remedy'
          ? {
              remedy: 'quarantine',
              outcome: 'quarantined',
              name: 'abababab-report.pdf',
              sha256: 'ab'.repeat(32),
              originalKept: true,
              detail,
            }
          : undefined,
      )
      fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
      fireEvent.click(quarantineButton())

      await waitFor(() => {
        expect(remedyResultLine()).not.toBeNull()
      })
      expect(remedyResultLine()).toBe(
        `quarantine: moved into quarantine, outside any workspace; name abababab-report.pdf; this remedy did not remove the original; ${copy}`,
      )
      expect(remedyResultLine()).not.toContain('the original is still in the project')
      cleanup()
      clearMocks()
    }
  })

  it("says a renamed-unverified quarantine both moved the file out of the project and could not verify what arrived, so the delete remedy's failure branch cannot be read as a clean move", async () => {
    // The ninth `detail` token, produced on the live quarantine path when
    // the rename returned but the destination could not be re-opened and
    // digested (`quarantine_detail` in `src-tauri/src/ipc/wrong_root.rs`).
    // It carries `originalKept: false` -- a rename removes the original
    // name whatever the verification then found -- and the destination
    // name, so the receipt has to say the file is gone *and* that its
    // arrival is unchecked. Either half alone misleads.
    await openQuarantineBlock((cmd) =>
      cmd === 'misplaced_remedy'
        ? {
            remedy: 'quarantine',
            outcome: 'quarantined',
            name: 'abababab-report.pdf',
            sha256: 'ab'.repeat(32),
            originalKept: false,
            detail: 'renamed-unverified',
          }
        : undefined,
    )
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
    fireEvent.click(quarantineButton())

    await waitFor(() => {
      expect(remedyResultLine()).not.toBeNull()
    })
    expect(remedyResultLine()).toBe(
      'quarantine: moved into quarantine, outside any workspace; name abababab-report.pdf; the original is gone from the project; moved by rename, but its arrival in quarantine could not be verified: the file is out of the project and nothing here has checked what arrived',
    )
    // It must not read as the plain `renamed` receipt, which claims a
    // verified arrival this branch is exactly the absence of.
    expect(remedyResultLine()).not.toBe(
      'quarantine: moved into quarantine, outside any workspace; name abababab-report.pdf; the original is gone from the project; moved by rename',
    )
  })

  it('Publish to outbox acts on a single click -- it keeps the original where it was found -- and its receipt says the original is still there and that the copy is not published until it is approved like any other outbox entry', async () => {
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) =>
      cmd === 'misplaced_remedy'
        ? {
            remedy: 'publish',
            outcome: 'copied-to-outbox',
            name: 'abababab-report.pdf',
            sha256: 'ab'.repeat(32),
            originalKept: true,
            detail: null,
          }
        : undefined,
    )
    await screen.findByRole('table', { name: 'Misplaced files' })

    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', {
        name: 'Publish to outbox',
      }),
    )

    await waitFor(() => {
      expect(remedyResultLine()).not.toBeNull()
    })
    expect(screen.queryByLabelText('Quarantine confirmation')).toBeNull()
    expect(remedyResultLine()).toBe(
      `publish: copied into the outbox as a new unattributed entry; name abababab-report.pdf; this remedy did not remove the original; ${COPY_NEEDS_APPROVAL_SENTENCE}`,
    )
    expect(calls.find((call) => call.cmd === 'misplaced_remedy')?.args).toEqual({
      name: 'docs/report.pdf',
      sha256: 'ab'.repeat(32),
      remedy: 'publish',
    })
  })

  it('Ignore acts on a single click too, and its receipt says the decision was recorded, that the file is not offered again until its content changes, that this remedy removed nothing, and that the suppression cannot be undone or reviewed from this product', async () => {
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) =>
      cmd === 'misplaced_remedy'
        ? {
            remedy: 'ignore',
            outcome: 'ignored',
            name: null,
            sha256: 'ab'.repeat(32),
            originalKept: true,
            detail: null,
          }
        : undefined,
    )
    await screen.findByRole('table', { name: 'Misplaced files' })

    fireEvent.click(within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Ignore' }))

    await waitFor(() => {
      expect(remedyResultLine()).not.toBeNull()
    })
    expect(remedyResultLine()).toBe(
      `ignore: the decision was recorded, and the file is not offered again until its content changes; this remedy did not remove the original; ${IGNORE_IS_PERMANENT_SENTENCE}`,
    )
    // One unconfirmed click suppresses a detection for that content
    // permanently: `docs/spike-log.md` § Slice 5d records that the ignore
    // ledger has no listing and no revocation command anywhere in the
    // product. The remedy is not destructive to the file, which is why it
    // is not gated -- it is destructive to the detection, and the receipt
    // has to say so rather than let the user discover it by looking for an
    // undo that does not exist.
    expect(remedyResultLine()).toContain('cannot be undone')
    expect(calls.find((call) => call.cmd === 'misplaced_remedy')?.args).toEqual({
      name: 'docs/report.pdf',
      sha256: 'ab'.repeat(32),
      remedy: 'ignore',
    })
  })

  it('fires one misplaced_remedy per double-click: the second click lands on a remedy already in flight and is refused, whichever remedy it names', async () => {
    let settle: (value: unknown) => void = () => {}
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) =>
      cmd === 'misplaced_remedy'
        ? new Promise((resolve) => {
            settle = resolve as (value: unknown) => void
          })
        : undefined,
    )
    await screen.findByRole('table', { name: 'Misplaced files' })

    const row = misplacedTableRow('docs/report.pdf')
    fireEvent.click(within(row).getByRole('button', { name: 'Ignore' }))
    fireEvent.click(within(row).getByRole('button', { name: 'Ignore' }))
    fireEvent.click(within(row).getByRole('button', { name: 'Publish to outbox' }))

    await waitFor(() => {
      expect(countCalls(calls, 'misplaced_remedy')).toBe(1)
    })
    await act(async () => {
      settle({
        remedy: 'ignore',
        outcome: 'ignored',
        name: null,
        sha256: 'ab'.repeat(32),
        originalKept: true,
        detail: null,
      })
      await Promise.resolve()
    })
    expect(countCalls(calls, 'misplaced_remedy')).toBe(1)
  })
})

/** Delivers one run-end `misplaced` event carrying `payload`, as run 7's frame `seq`. */
function deliverMisplaced(channel: LiveChannel, payload: ScanSummary = SCAN_SUMMARY, seq = 10) {
  act(() => {
    channel.onmessage({
      stream: 'event',
      body: { id: 7, seq, droppedBefore: 0, kind: 'misplaced', payload },
    })
  })
}

/** Every transcript row's text, in order. */
function transcriptRows(): string[] {
  return Array.from(screen.getByLabelText('Agent transcript').querySelectorAll('li')).map(
    (row) => row.textContent ?? '',
  )
}

describe('AgentPanel run-end misplaced frame (slice 5d)', () => {
  it('renders a misplaced event as one transcript line carrying the same fixed copy as the scan summary, in its own row, and refetches the findings and the status the run-end scan produced', async () => {
    let channel: LiveChannel | undefined
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd, args) => {
      if (cmd === 'harness_spawn') {
        channel = (args as { onFrame: LiveChannel }).onFrame
        return 7
      }
      return undefined
    })
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })
    await startRunOverWorkspace()
    if (!channel) throw new Error('harness_spawn was not called')
    const before = calls.length

    deliverMisplaced(channel)

    expect(transcriptRows()).toEqual([
      'you: do the thing',
      'scan: 12 scanned, 2 findings, 1 ignored, 3 excluded, 0 unreadable',
    ])
    await waitFor(() => {
      expect(calls.slice(before).map((call) => call.cmd)).toContain('misplaced_list')
    })
    expect(calls.slice(before).map((call) => call.cmd)).toContain('wrongroot_status')

    await endRun(channel)
  })

  it('says a truncated run-end scan stopped early in the transcript too, with the same fixed copy the section uses, and renders every zero count as 0', async () => {
    let channel: LiveChannel | undefined
    mountWrongRoots({ rows: [] }, (cmd, args) => {
      if (cmd === 'harness_spawn') {
        channel = (args as { onFrame: LiveChannel }).onFrame
        return 7
      }
      return undefined
    })
    await startRunOverWorkspace()
    if (!channel) throw new Error('harness_spawn was not called')

    deliverMisplaced(channel, {
      scanned: 0,
      findings: 0,
      ignored: 0,
      excluded: 0,
      unreadable: 0,
      truncated: true,
    })

    expect(transcriptRows()[1]).toBe(
      'scan: 0 scanned, 0 findings, 0 ignored, 0 excluded, 0 unreadable — the scan stopped early; the list is incomplete',
    )
    await endRun(channel)
  })

  it('renders the fixed diagnostic a scan that could not run emits as a diagnostic, never as a misplaced line claiming nothing was found', async () => {
    let channel: LiveChannel | undefined
    mountWrongRoots({ rows: [] }, (cmd, args) => {
      if (cmd === 'harness_spawn') {
        channel = (args as { onFrame: LiveChannel }).onFrame
        return 7
      }
      return undefined
    })
    await startRunOverWorkspace()
    if (!channel) throw new Error('harness_spawn was not called')

    act(() => {
      channel!.onmessage({
        stream: 'event',
        body: {
          id: 7,
          seq: 10,
          droppedBefore: 0,
          kind: 'diagnostic',
          payload: {
            text: 'the project could not be scanned for output written outside the outbox',
          },
        },
      })
    })

    expect(transcriptRows()[1]).toBe(
      'diagnostic: the project could not be scanned for output written outside the outbox',
    )
    expect(transcriptRows()[1]).not.toContain('scan: 0 scanned')
    await endRun(channel)
  })
})

describe('AgentPanel wrong roots and the workspace (slice 5d; slice 5c review, R1-001)', () => {
  it("resets the section on a workspace pick -- a real receipt of a DELETION, the scan line, an open quarantine block and its typed gate all go, since they are the previous project's -- and fetches the new project's status and findings afresh", async () => {
    // The receipt has to be produced before the pick, not merely asserted
    // absent after it. Asserting `remedyResultLine()` is null when no
    // remedy ever ran is vacuous: it was null the whole time, and deleting
    // the reset entirely left this test green -- which is how a previous
    // project's DELETION receipt could survive under the next project with
    // the suite reporting no problem at all. So: run a quarantine, watch
    // the receipt say the file is gone, and only then pick.
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT, MISPLACED_BUNDLE] }, (cmd) => {
      if (cmd === 'workspace_pick') return { displayPath: '/home/user/other', workArea: 'valid' }
      if (cmd === 'wrongroot_scan') return SCAN_SUMMARY
      if (cmd === 'misplaced_remedy') {
        return {
          remedy: 'quarantine',
          outcome: 'quarantined',
          name: 'abababab-report.pdf',
          sha256: 'ab'.repeat(32),
          originalKept: false,
          detail: 'renamed',
        }
      }
      return undefined
    })
    await screen.findByRole('table', { name: 'Misplaced files' })
    fireEvent.click(scanButton())
    await waitFor(() => {
      expect(scanLine()).not.toBeNull()
    })

    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Quarantine' }),
    )
    await screen.findByLabelText('Quarantine confirmation')
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
    fireEvent.click(quarantineButton())
    await waitFor(() => {
      expect(remedyResultLine()).not.toBeNull()
    })
    expect(remedyResultLine()).toContain('the original is gone from the project')

    // A second block, open over the other finding, with its gate typed:
    // the pick has to take that too, not just the receipt.
    fireEvent.click(
      within(misplacedTableRow('build/bundle.zip')).getByRole('button', { name: 'Quarantine' }),
    )
    await screen.findByLabelText('Quarantine confirmation')
    fireEvent.change(quarantineGate(), { target: { value: 'cdcdcdcd' } })
    expect(quarantineButton().disabled).toBe(false)
    const before = calls.length

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    await flush()

    expect(scanLine()).toBeNull()
    expect(screen.queryByLabelText('Quarantine confirmation')).toBeNull()
    expect(remedyResultLine()).toBeNull()
    const after = calls.slice(before).map((call) => call.cmd)
    expect(after).toContain('wrongroot_status')
    expect(after).toContain('misplaced_list')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it("drops a wrongroot_scan that resolves after a workspace pick: the new project's section shows no scan line of the previous project's walk", async () => {
    let resolveScan: (summary: ScanSummary) => void = () => {}
    mountWrongRoots({ rows: [] }, (cmd) => {
      if (cmd === 'wrongroot_scan') {
        return new Promise<ScanSummary>((resolve) => {
          resolveScan = resolve
        })
      }
      if (cmd === 'workspace_pick') return { displayPath: '/home/user/other', workArea: 'valid' }
      return undefined
    })
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })

    fireEvent.click(scanButton())
    await flush()
    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    await flush()
    resolveScan(SCAN_SUMMARY)
    await flush()

    expect(scanLine()).toBeNull()
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('holds the workspace pick while a remedy is in flight, so a receipt for a file that has already left the project can never be stranded between two projects', async () => {
    // The window this closes was reachable by ordinary clicking: confirm a
    // quarantine, then pick another project while it runs. The file leaves
    // the project either way -- the shell is already past the point of no
    // return -- and the generation guard then drops the receipt, correctly,
    // since posting it under the new project would read as a move inside
    // *that* project. Correct and silent: the user is never told the file
    // left. Holding the pick for the length of one remedy is the only
    // answer that is neither wrong nor silent.
    let resolveRemedy: (result: unknown) => void = () => {}
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) => {
      if (cmd === 'misplaced_remedy') {
        return new Promise((resolve) => {
          resolveRemedy = resolve as (result: unknown) => void
        })
      }
      if (cmd === 'workspace_pick') return { displayPath: '/home/user/other', workArea: 'valid' }
      return undefined
    })
    await screen.findByRole('table', { name: 'Misplaced files' })

    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Ignore' }),
    )
    await flush()

    const pick = screen.getByRole('button', { name: 'Pick workspace' }) as HTMLButtonElement
    expect(pick.disabled).toBe(true)
    fireEvent.click(pick)
    await flush()
    expect(countCalls(calls, 'workspace_pick')).toBe(0)

    resolveRemedy({
      remedy: 'ignore',
      outcome: 'ignored',
      name: null,
      sha256: 'ab'.repeat(32),
      originalKept: true,
      detail: null,
    })
    await flush()

    // The receipt is posted, under the project it belongs to, and the pick
    // is live again the moment the remedy is done.
    expect(remedyResultLine()).toContain('ignore: the decision was recorded')
    expect((screen.getByRole('button', { name: 'Pick workspace' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('refuses a workspace pick that lands before React has re-rendered the disabled button: the ref, not the attribute, is what holds the pick during a remedy', async () => {
    // The `disabled` attribute is one render behind the click that starts
    // the remedy. What actually has to hold the pick is `remedyBusyRef`,
    // set synchronously in the same tick -- so the two clicks go out in a
    // single `act` with no re-render between them, which is the only way to
    // reach the handler's own guard rather than the attribute above it.
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) => {
      if (cmd === 'misplaced_remedy') {
        return new Promise<never>(() => {
          // Never settles: the remedy stays in flight for both clicks.
        })
      }
      if (cmd === 'workspace_pick') return { displayPath: '/home/user/other', workArea: 'valid' }
      return undefined
    })
    await screen.findByRole('table', { name: 'Misplaced files' })

    const ignore = within(misplacedTableRow('docs/report.pdf')).getByRole('button', {
      name: 'Ignore',
    })
    const pick = screen.getByRole('button', { name: 'Pick workspace' }) as HTMLButtonElement
    expect(pick.disabled).toBe(false)

    act(() => {
      ignore.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      pick.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    await flush()

    expect(countCalls(calls, 'misplaced_remedy')).toBe(1)
    expect(countCalls(calls, 'workspace_pick')).toBe(0)
  })

  it("drops a wrongroot_status that resolves after a workspace pick: a stale report never posts the previous project's scan counts under the new one", async () => {
    // The generation guard is implemented in all four wrong-root paths and
    // was proven in three of eight continuations -- `wrongroot_status` in
    // neither of its two. It is the one that carries `scanned, findings n`,
    // so a stale answer states, on the new project's own status line, how
    // many findings the *previous* project had.
    let resolveStatus: (status: WrongRootStatus) => void = () => {}
    let picked = false
    mountWrongRoots(
      {
        status: () =>
          picked
            ? wrongRootStatusAdvisory({ scanned: false, findings: 0 })
            : new Promise<WrongRootStatus>((resolve) => {
                resolveStatus = resolve
              }),
        rows: [],
      },
      (cmd) => {
        if (cmd === 'workspace_pick') {
          picked = true
          return { displayPath: '/home/user/other', workArea: 'valid' }
        }
        return undefined
      },
    )
    await waitFor(() => {
      expect(outputDisciplineUnavailableLine()).not.toBeNull()
    })

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    await flush()
    resolveStatus(wrongRootStatusAdvisory({ scanned: true, findings: 7 }))
    await flush()

    expect(outputDisciplineLine()).not.toContain('findings 7')
    expect(outputDisciplineLine()).toContain('not scanned yet')
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('drops a wrongroot_status that *rejects* after a workspace pick too, leaving the new project\'s own report standing rather than clearing it', async () => {
    let rejectStatus: (reason: unknown) => void = () => {}
    let picked = false
    mountWrongRoots(
      {
        status: () =>
          picked
            ? wrongRootStatusAdvisory({ scanned: true, findings: 1 })
            : new Promise<WrongRootStatus>((_resolve, reject) => {
                rejectStatus = reject
              }),
        rows: [],
      },
      (cmd) => {
        if (cmd === 'workspace_pick') {
          picked = true
          return { displayPath: '/home/user/other', workArea: 'valid' }
        }
        return undefined
      },
    )
    await waitFor(() => {
      expect(outputDisciplineUnavailableLine()).not.toBeNull()
    })

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    await flush()
    rejectStatus({ code: 'unexpected', message: 'gone' })
    await flush()

    // The new project's report answered; the old project's rejection must
    // not reach in and clear it.
    expect(outputDisciplineLine()).toContain('scanned, findings 1')
    expect(outputDisciplineUnavailableLine()).toBeNull()
  })

  it("drops a misplaced_list that *rejects* after a workspace pick: the previous project's failure never posts the 'could not be read' line over the new project's own answer", async () => {
    let rejectList: (reason: unknown) => void = () => {}
    let picked = false
    mountWrongRoots(
      {
        rows: () =>
          picked
            ? [MISPLACED_BUNDLE]
            : new Promise<MisplacedRow[]>((_resolve, reject) => {
                rejectList = reject
              }),
      },
      (cmd) => {
        if (cmd === 'workspace_pick') {
          picked = true
          return { displayPath: '/home/user/other', workArea: 'valid' }
        }
        return undefined
      },
    )
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    await flush()
    rejectList({ code: 'unexpected', message: 'gone' })
    await flush()

    await screen.findByRole('table', { name: 'Misplaced files' })
    expect(misplacedRows().map((cells) => cells[0])).toEqual(['build/bundle.zip'])
    expect(misplacedUnavailableLine()).toBeNull()
  })

  it("drops a misplaced_list that resolves after a workspace pick: one project's findings never stand in the next project's table", async () => {
    let resolveList: (rows: MisplacedRow[]) => void = () => {}
    let picked = false
    mountWrongRoots(
      {
        rows: () =>
          picked
            ? []
            : new Promise<MisplacedRow[]>((resolve) => {
                resolveList = resolve
              }),
      },
      (cmd) => {
        if (cmd === 'workspace_pick') {
          picked = true
          return { displayPath: '/home/user/other', workArea: 'valid' }
        }
        return undefined
      },
    )
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })

    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    await flush()
    resolveList([MISPLACED_REPORT])
    await flush()

    expect(screen.queryByRole('table', { name: 'Misplaced files' })).toBeNull()
  })
})

describe('AgentPanel wrong roots guards (slice 5d)', () => {
  it("freezes every remedy, the open quarantine block's input and its final button while a run is active -- the shell refuses each remedy with run-active in the same window -- while Scan stays live; everything is live again once the run ends, with the typed gate surviving the run", async () => {
    let channel: LiveChannel | undefined
    mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd, args) => {
      if (cmd === 'harness_spawn') {
        channel = (args as { onFrame: LiveChannel }).onFrame
        return 7
      }
      return undefined
    })
    await screen.findByRole('table', { name: 'Misplaced files' })
    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Quarantine' }),
    )
    await screen.findByLabelText('Quarantine confirmation')
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
    expect(quarantineButton().disabled).toBe(false)

    await startRunOverWorkspace()

    const controls = () => {
      const row = misplacedTableRow('docs/report.pdf')
      return {
        quarantine: (within(row).getByRole('button', { name: 'Quarantine' }) as HTMLButtonElement)
          .disabled,
        publish: (
          within(row).getByRole('button', { name: 'Publish to outbox' }) as HTMLButtonElement
        ).disabled,
        ignore: (within(row).getByRole('button', { name: 'Ignore' }) as HTMLButtonElement).disabled,
        gate: quarantineGate().disabled,
        confirm: quarantineButton().disabled,
        scan: scanButton().disabled,
      }
    }
    expect(controls()).toEqual({
      quarantine: true,
      publish: true,
      ignore: true,
      gate: true,
      confirm: true,
      scan: false,
    })

    if (!channel) throw new Error('harness_spawn was not called')
    await endRun(channel)
    expect(controls()).toEqual({
      quarantine: false,
      publish: false,
      ignore: false,
      gate: false,
      confirm: false,
      scan: false,
    })
    expect(quarantineGate().value).toBe('abababab')
  })

  it('refuses a remedy that lands in the same tick as Start, before React has re-rendered either the button or the handler: the run is active from the moment Start is clicked, and both halves of the freeze were one render behind it', async () => {
    // The handler-side run-active guard could not be reached by any click
    // while it read the render value: every entry point carries
    // `disabled={runActive || ...}`, and React does not dispatch onClick on
    // a disabled button, so deleting the guard left the whole suite green.
    // The window it is *for* is this one -- `handleStart` sets `isSpawning`
    // inside a click handler, so a second click batched into the same tick
    // sees a stale `false` on the button and in the closure alike. Both
    // clicks go out inside one `act`, which is the only way to keep the
    // render batched and reach the handler with the button still enabled.
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) => {
      if (cmd === 'harness_spawn') {
        return new Promise<number>(() => {
          // Never settles: the run stays in its spawning window, which is
          // exactly the window `handleStart` opens synchronously.
        })
      }
      return undefined
    })
    await screen.findByRole('table', { name: 'Misplaced files' })
    await selectOption('Adapter', 'claude-code')
    await selectOption('Approval', '42')
    fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: 'do the thing' } })

    const start = screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement
    const ignore = within(misplacedTableRow('docs/report.pdf')).getByRole('button', {
      name: 'Ignore',
    }) as HTMLButtonElement
    expect(start.disabled).toBe(false)
    expect(ignore.disabled).toBe(false)

    act(() => {
      start.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      ignore.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    await flush()

    expect(countCalls(calls, 'harness_spawn')).toBe(1)
    expect(countCalls(calls, 'misplaced_remedy')).toBe(0)
  })

  it('a click on a frozen remedy during a run invokes nothing, belt and braces with the buttons\' own disabled', async () => {
    let channel: LiveChannel | undefined
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd, args) => {
      if (cmd === 'harness_spawn') {
        channel = (args as { onFrame: LiveChannel }).onFrame
        return 7
      }
      return undefined
    })
    await screen.findByRole('table', { name: 'Misplaced files' })
    await startRunOverWorkspace()

    const row = misplacedTableRow('docs/report.pdf')
    fireEvent.click(within(row).getByRole('button', { name: 'Ignore' }))
    fireEvent.click(within(row).getByRole('button', { name: 'Publish to outbox' }))
    fireEvent.click(within(row).getByRole('button', { name: 'Quarantine' }))
    await flush()

    expect(countCalls(calls, 'misplaced_remedy')).toBe(0)
    expect(screen.queryByLabelText('Quarantine confirmation')).toBeNull()

    if (!channel) throw new Error('harness_spawn was not called')
    await endRun(channel)
  })
})

describe('AgentPanel wrong roots banners (slice 5d)', () => {
  const codes: { code: string; message: string }[] = [
    {
      code: 'misplaced-unknown',
      message: 'no scan reported that file at that digest; scan again and retry',
    },
    {
      code: 'quarantine-unavailable',
      message: 'the quarantine directory could not be used',
    },
    // `scan-failed` is not in this table: it is `wrongroot_scan`'s refusal,
    // not a remedy's, and it has its own banner test below. Mixing it in
    // here was part of why the reachable set was never actually counted.
    { code: 'invalid-request', message: 'the digest is not 64 hex characters' },
    { code: 'refused', message: "the file's content changed since it was found" },
    {
      code: 'outbox-unavailable',
      message: 'an outbox entry of that name already exists with other content',
    },
    { code: 'work-area-invalid', message: 'the ignore ledger could not be written' },
    { code: 'run-active', message: 'a run is active; approve or publish once it has ended' },
    // HAP-001-R20's hard-link refusal, applied to a misplaced file
    // (HAP-001-R32) -- reachable from **both** remedies, since
    // `open_misplaced` raises it for the quarantine path and the copy-in
    // raises it again for the publish path. It was on none of the three
    // lists that were supposed to enumerate this command's codes: not the
    // wrapper's `# Errors` JSDoc, not the wrapper tests, and not here.
    { code: 'outbox-linked', message: "the file's link count is greater than one" },
    { code: 'outbox-invalid', message: 'the outbox declaration could not be loaded' },
    { code: 'workspace-unavailable', message: 'no workspace is active' },
  ]

  it('covers every code misplaced_remedy reaches, which is ten', () => {
    // The number is the point: the wrapper's JSDoc named nine, the wrapper
    // tests asserted seven, and this table asserted a different seven. One
    // reachable refusal -- a security refusal -- was missing from all
    // three, so no list here disagreed with any other loudly enough to be
    // noticed.
    expect(codes).toHaveLength(10)
    expect(codes.map((entry) => entry.code).sort()).toEqual([
      'invalid-request',
      'misplaced-unknown',
      'outbox-invalid',
      'outbox-linked',
      'outbox-unavailable',
      'quarantine-unavailable',
      'refused',
      'run-active',
      'work-area-invalid',
      'workspace-unavailable',
    ])
  })

  it.each(codes)(
    'renders a $code rejection through the banner as "<code>: <message>" with no "untrusted" prefix and no detail rendered',
    async ({ code, message }) => {
      mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) =>
        cmd === 'misplaced_remedy'
          ? Promise.reject({ code, message, detail: { recordedSha256Short: 'deadbeef' } })
          : undefined,
      )
      await screen.findByRole('table', { name: 'Misplaced files' })

      fireEvent.click(
        within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Ignore' }),
      )

      const banner = await screen.findByRole('alert')
      expect(banner.textContent).toBe(`${code}: ${message}`)
      expect(banner.textContent).not.toContain('untrusted')
      expect(banner.textContent).not.toContain('deadbeef')
    },
  )

  it('renders a scan-failed rejection from Scan the same way, and leaves no scan line claiming a walk that never ran', async () => {
    mountWrongRoots({ rows: [] }, (cmd) =>
      cmd === 'wrongroot_scan'
        ? Promise.reject({ code: 'scan-failed', message: 'the project could not be scanned' })
        : undefined,
    )
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })

    fireEvent.click(scanButton())

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('scan-failed: the project could not be scanned')
    expect(scanLine()).toBeNull()
    expect(scanButton().disabled).toBe(false)
  })

  it('renders a non-ShellError rejection as "unexpected error" and nothing of its message', async () => {
    mountWrongRoots({ rows: [] }, (cmd) =>
      cmd === 'wrongroot_scan' ? Promise.reject(new Error('a stack trace nobody should read')) : undefined,
    )
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })

    fireEvent.click(scanButton())

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('unexpected error')
    expect(banner.textContent).not.toContain('stack trace')
  })

  it('keeps the table and the open block as they were on every rejection that is not misplaced-unknown, so the user can read the banner and retry the decision they had already made', async () => {
    // The claim "a rejection keeps the table and the block" was in the
    // spike log and untested: every banner case above clicks Ignore from
    // the table with no block open, so nothing there could have noticed a
    // rejection closing one.
    const calls = await openQuarantineBlock((cmd) =>
      cmd === 'misplaced_remedy'
        ? Promise.reject({
            code: 'quarantine-unavailable',
            message: 'the quarantine directory could not be used',
          })
        : undefined,
    )
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
    fireEvent.click(quarantineButton())

    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe(
      'quarantine-unavailable: the quarantine directory could not be used',
    )
    // The block is still open, still bound to the same finding, with the
    // typed gate intact and the button live -- everything the user needs to
    // retry without reading and typing the digest again.
    expect(screen.queryByLabelText('Quarantine confirmation')).not.toBeNull()
    expect(quarantineBlockLines()[0]).toBe('name: docs/report.pdf')
    expect(quarantineGate().value).toBe('abababab')
    expect(quarantineButton().disabled).toBe(false)
    expect(screen.queryByRole('table', { name: 'Misplaced files' })).not.toBeNull()

    // And it really can be retried: a second click sends a second request.
    const before = countCalls(calls, 'misplaced_remedy')
    fireEvent.click(quarantineButton())
    await waitFor(() => {
      expect(countCalls(calls, 'misplaced_remedy')).toBe(before + 1)
    })
  })

  it('clears the banner when a later act succeeds, so a refusal never outlives the attempt it refused', async () => {
    let fail = true
    mountWrongRoots({ rows: [] }, (cmd) => {
      if (cmd !== 'wrongroot_scan') return undefined
      return fail
        ? Promise.reject({ code: 'scan-failed', message: 'the project could not be scanned' })
        : SCAN_SUMMARY
    })
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })

    fireEvent.click(scanButton())
    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe('scan-failed: the project could not be scanned')

    fail = false
    fireEvent.click(scanButton())

    await waitFor(() => {
      expect(scanLine()).not.toBeNull()
    })
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('closes a stale quarantine block on misplaced-unknown and refetches, since its facts name a finding the shell no longer reports', async () => {
    const calls = await openQuarantineBlock((cmd) =>
      cmd === 'misplaced_remedy'
        ? Promise.reject({
            code: 'misplaced-unknown',
            message: 'no scan reported that file at that digest; scan again and retry',
          })
        : undefined,
    )
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
    const before = calls.length
    fireEvent.click(quarantineButton())

    await screen.findByRole('alert')
    expect(screen.queryByLabelText('Quarantine confirmation')).toBeNull()
    await waitFor(() => {
      expect(calls.slice(before).map((call) => call.cmd)).toContain('misplaced_list')
    })
  })
})

describe('AgentPanel the open block is bound to an identity (slice 5d review, R3-028)', () => {
  it('closes the confirmation when a run-end refetch reports the same name at different bytes, rather than swapping the identity facts under a decision the user is part-way through taking', async () => {
    // The block was bound by name alone, and the section refetches on every
    // scan and on every run-end `misplaced` frame -- a frame that arrives
    // with the user doing nothing at all. A refetch returning the same name
    // at other bytes therefore replaced the digest the user was reading and
    // typing, inside an open confirmation that looked unchanged. The block
    // is bound to the pair `misplaced_remedy` itself binds to, so it either
    // still names the finding it was opened for or it is gone.
    let channel: LiveChannel | undefined
    let rows: MisplacedRow[] = [MISPLACED_REPORT]
    mountWrongRoots({ rows: () => rows }, (cmd, args) => {
      if (cmd === 'harness_spawn') {
        channel = (args as { onFrame: LiveChannel }).onFrame
        return 7
      }
      return undefined
    })
    await screen.findByRole('table', { name: 'Misplaced files' })
    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Quarantine' }),
    )
    await screen.findByLabelText('Quarantine confirmation')
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
    expect(quarantineButton().disabled).toBe(false)

    await startRunOverWorkspace()
    if (!channel) throw new Error('harness_spawn was not called')
    // Same project-relative name, different content: a different finding.
    rows = [misplacedRow({ sha256: 'ef'.repeat(32), sha256Short: 'efefefef' })]
    deliverMisplaced(channel)
    await waitFor(() => {
      expect(misplacedRows()[0]?.[4]).toBe('efefefef')
    })

    expect(screen.queryByLabelText('Quarantine confirmation')).toBeNull()
    await endRun(channel)
  })

  it('keeps the confirmation open across a refetch that reports the same finding at the same bytes, so an ordinary run-end frame does not throw away a decision in progress', async () => {
    let channel: LiveChannel | undefined
    mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd, args) => {
      if (cmd === 'harness_spawn') {
        channel = (args as { onFrame: LiveChannel }).onFrame
        return 7
      }
      return undefined
    })
    await screen.findByRole('table', { name: 'Misplaced files' })
    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Quarantine' }),
    )
    await screen.findByLabelText('Quarantine confirmation')
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })

    await startRunOverWorkspace()
    if (!channel) throw new Error('harness_spawn was not called')
    deliverMisplaced(channel)
    await flush()

    expect(screen.queryByLabelText('Quarantine confirmation')).not.toBeNull()
    expect(quarantineGate().value).toBe('abababab')
    await endRun(channel)
  })
})

describe('AgentPanel names that do not render as themselves (slice 5d review, R1-011 / R1-012)', () => {
  it('renders a name carrying U+2028 on one visual line: both separators are forced line breaks under CSS, so a producer could otherwise put a second line inside the delete confirmation and it would be indistinguishable from the block\'s own fixed sentences', async () => {
    const lineSeparator = String.fromCodePoint(0x2028)
    const paragraphSeparator = String.fromCodePoint(0x2029)
    const hostile = `docs/report.pdf${lineSeparator}this line was written by the producer${paragraphSeparator}and so was this`
    mountWrongRoots({ rows: [misplacedRow({ name: hostile })] })
    await screen.findByRole('table', { name: 'Misplaced files' })

    fireEvent.click(misplacedTable().querySelectorAll('button')[0] as HTMLElement)
    await screen.findByLabelText('Quarantine confirmation')

    const region = wrongRootsRegion()
    expect(region.textContent).not.toContain(lineSeparator)
    expect(region.textContent).not.toContain(paragraphSeparator)
    // The textual content itself is kept -- RCS-001-R18 forbids removing
    // it; only the two format controls go, so the smuggled sentences stay
    // visibly part of the file name instead of standing as lines of their
    // own.
    expect(region.textContent).toContain('this line was written by the producer')
    expect(quarantineBlockLines()[0]).toBe(
      'name: docs/report.pdfthis line was written by the producerand so was this (this name contains hidden characters; compare by digest)',
    )
  })

  it('marks a name whose displayed form is not its whole form, so two findings differing only by an invisible character are not left looking identical beside a delete button', async () => {
    // Stripping happens for display only. Two names differing solely by a
    // zero-width space render as the same string, and one of these rows is
    // about to be offered Quarantine: after the fact the user cannot say
    // which of the two left the project. The mark does not print the
    // removed characters -- that would put them back on the screen -- it
    // says the displayed name is not the whole name, which is what sends
    // the user to the digest column.
    const zeroWidth = String.fromCodePoint(0x200b)
    mountWrongRoots({
      rows: [
        MISPLACED_REPORT,
        misplacedRow({
          name: `docs/report${zeroWidth}.pdf`,
          sha256: 'cd'.repeat(32),
          sha256Short: 'cdcdcdcd',
        }),
      ],
    })
    await screen.findByRole('table', { name: 'Misplaced files' })

    const [plain, hidden] = misplacedRows()
    expect(plain?.[0]).toBe('docs/report.pdf')
    expect(hidden?.[0]).toBe(
      'docs/report.pdf (this name contains hidden characters; compare by digest)',
    )
    // The mark is what makes the two rows tellable apart; the digests are
    // what tells them apart.
    expect(plain?.[0]).not.toBe(hidden?.[0])
    expect(plain?.[4]).toBe('abababab')
    expect(hidden?.[4]).toBe('cdcdcdcd')
    expect(wrongRootsRegion().textContent).not.toContain(zeroWidth)
  })

  it('leaves an ordinary name unmarked, so the mark means something when it appears', async () => {
    mountWrongRoots({ rows: [MISPLACED_REPORT] })
    await screen.findByRole('table', { name: 'Misplaced files' })

    expect(misplacedRows()[0]?.[0]).toBe('docs/report.pdf')
    expect(misplacedTable().textContent).not.toContain('hidden characters')
  })
})

describe('AgentPanel wrong roots out-of-order fetches (slice 5d review, R3-022)', () => {
  it('applies only the latest wrongroot_status when an earlier one resolves after it: an overtaken report would otherwise post older scan counts over newer ones', async () => {
    // Both fetches belong to the same project, so the generation guard has
    // nothing to say about this one -- the sequence number is the only
    // thing that orders them. Deleting it left the suite green.
    const settle: ((status: WrongRootStatus) => void)[] = []
    mountWrongRoots(
      {
        status: () =>
          new Promise<WrongRootStatus>((resolve) => {
            settle.push(resolve)
          }),
        rows: [],
      },
      (cmd) => (cmd === 'wrongroot_scan' ? SCAN_SUMMARY : undefined),
    )
    await waitFor(() => {
      expect(settle).toHaveLength(1)
    })

    // A scan refetches the status, so a second fetch is now in flight.
    fireEvent.click(scanButton())
    await waitFor(() => {
      expect(settle).toHaveLength(2)
    })

    // The later fetch answers first, the earlier one second.
    settle[1]?.(wrongRootStatusAdvisory({ scanned: true, findings: 3 }))
    await flush()
    settle[0]?.(wrongRootStatusAdvisory({ scanned: false, findings: 0 }))
    await flush()

    expect(outputDisciplineLine()).toContain('scanned, findings 3')
    expect(outputDisciplineLine()).not.toContain('not scanned yet')
  })

  it('applies only the latest misplaced_list when an earlier one resolves after it: an overtaken listing would otherwise put findings the shell has already remedied back in the table', async () => {
    const settle: ((rows: MisplacedRow[]) => void)[] = []
    mountWrongRoots(
      {
        rows: () =>
          new Promise<MisplacedRow[]>((resolve) => {
            settle.push(resolve)
          }),
      },
      (cmd) => (cmd === 'wrongroot_scan' ? SCAN_SUMMARY : undefined),
    )
    await waitFor(() => {
      expect(settle).toHaveLength(1)
    })

    fireEvent.click(scanButton())
    await waitFor(() => {
      expect(settle).toHaveLength(2)
    })

    settle[1]?.([])
    await flush()
    settle[0]?.([MISPLACED_REPORT])
    await flush()

    expect(screen.queryByRole('table', { name: 'Misplaced files' })).toBeNull()
    expect(misplacedUnavailableLine()).toBeNull()
  })
})

describe('AgentPanel wrong roots unmount guards (slice 5d; slice 5c review, R3-001)', () => {
  /**
   * The four wrong-root commands, each reached the way the surface reaches
   * it: a fetch on mount, the Scan button, or a remedy button. Mounts,
   * drives the panel until `command` is in flight, and hands back the
   * recorded calls and that one deferred promise's settle functions.
   */
  async function driveWrongRootInFlight(command: string): Promise<{
    calls: Call[]
    resolve: (value: unknown) => void
    reject: (reason: unknown) => void
  }> {
    let resolve: (value: unknown) => void = () => {}
    let reject: (reason: unknown) => void = () => {}
    const deferred = (): Promise<never> =>
      new Promise((settle, refuse) => {
        resolve = settle as (value: unknown) => void
        reject = refuse
      })

    if (command === 'wrongroot_status') {
      const calls = mountWrongRoots({ status: () => deferred(), rows: [] })
      await waitFor(() => {
        expect(countCalls(calls, 'misplaced_list')).toBe(1)
      })
      return { calls, resolve, reject }
    }
    if (command === 'misplaced_list') {
      const calls = mountWrongRoots({ rows: () => deferred() })
      await waitFor(() => {
        expect(outputDisciplineLine()).not.toBeNull()
      })
      return { calls, resolve, reject }
    }
    if (command === 'wrongroot_scan') {
      const calls = mountWrongRoots({ rows: [] }, (cmd) =>
        cmd === 'wrongroot_scan' ? deferred() : undefined,
      )
      await waitFor(() => {
        expect(outputDisciplineLine()).not.toBeNull()
      })
      fireEvent.click(scanButton())
      await waitFor(() => {
        expect(countCalls(calls, 'wrongroot_scan')).toBe(1)
      })
      return { calls, resolve, reject }
    }
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) =>
      cmd === 'misplaced_remedy' ? deferred() : undefined,
    )
    await screen.findByRole('table', { name: 'Misplaced files' })
    fireEvent.click(within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Ignore' }))
    await waitFor(() => {
      expect(countCalls(calls, 'misplaced_remedy')).toBe(1)
    })
    return { calls, resolve, reject }
  }

  const UNMOUNT_ANSWERS: Record<string, unknown> = {
    wrongroot_status: wrongRootStatusAdvisory(),
    misplaced_list: [MISPLACED_REPORT],
    wrongroot_scan: SCAN_SUMMARY,
    misplaced_remedy: {
      remedy: 'ignore',
      outcome: 'ignored',
      name: null,
      sha256: 'ab'.repeat(32),
      originalKept: true,
      detail: null,
    },
  }

  /**
   * What each parameter of this test actually proves, stated because two of
   * the four prove less than the name suggests.
   *
   * `wrongroot_scan` and `misplaced_remedy` both refetch on success, so
   * "no IPC call after the unmount" has teeth for them: removing their
   * `mountedRef` guard turns the refetch into two calls after `cleanup()`
   * and this test goes red.
   *
   * `wrongroot_status` and `misplaced_list` issue no follow-up IPC at all
   * -- their continuations only `setState` -- and React 19 makes a
   * `setState` on an unmounted component a silent no-op with no warning.
   * There is therefore nothing observable to assert for those two, and
   * measurement confirms it: deleting `refreshMisplaced`'s `mountedRef`
   * guard leaves the whole suite green. They are kept here for the "nothing
   * thrown, no console.error" half, which is real, and their sequencing --
   * the guard that *is* observable -- is covered by the out-of-order tests
   * above rather than pretended at here.
   */
  it.each(['wrongroot_status', 'misplaced_list', 'wrongroot_scan', 'misplaced_remedy'])(
    "%s's continuation no-ops after unmount, whether it resolves or rejects: nothing thrown and no console.error, and -- for the scan and the remedy, which refetch on success -- no IPC call after the unmount",
    async (command) => {
      for (const outcome of ['resolve', 'reject'] as const) {
        const { calls, resolve, reject } = await driveWrongRootInFlight(command)
        const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
        const before = calls.length

        cleanup()
        if (outcome === 'resolve') resolve(UNMOUNT_ANSWERS[command])
        else reject({ code: 'scan-failed', message: 'the project could not be scanned' })
        await new Promise((settled) => {
          setTimeout(settled, 0)
        })

        expect(consoleErrorSpy).not.toHaveBeenCalled()
        expect(calls.length).toBe(before)
        consoleErrorSpy.mockRestore()
        clearMocks()
      }
    },
  )
})

describe('AgentPanel wrong-root wire tokens it does not recognize (slice 5d review, R1-001)', () => {
  /**
   * Every one of these fields is a plain string on the wire. The renderer's
   * unions are closed; the shell's are not, and a shell one version ahead
   * puts a token here that no `case` matches. An exhaustive switch with no
   * `default` returned `undefined`, which reaches `PlainTextLine` and throws
   * `Array.from(undefined)` mid-render -- and with no boundary above these
   * panels that throw unmounted the whole page, the approval surface and
   * every freeze guard with it.
   */
  it('renders an unrecognized reason token as the raw token, inert, rather than throwing mid-render and taking the panel down', async () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    mountWrongRoots({
      rows: [misplacedRow({ reason: 'in-project-inside-some-later-arm' as WrongRootReason })],
    })
    await screen.findByRole('table', { name: 'Misplaced files' })

    expect(misplacedRows()[0]?.[5]).toBe('in-project-inside-some-later-arm')
    // The surface is whole: the table stands, the region stands, and the
    // remedies are still offered for the row.
    expect(remedyButtons('docs/report.pdf')).toEqual(['Quarantine', 'Publish to outbox', 'Ignore'])
    expect(screen.getByRole('region', { name: 'Wrong roots' })).toBeTruthy()
    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('renders an unrecognized detail token in the receipt as the raw token rather than throwing, so a remedy that has already run can still report what it did', async () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) =>
      cmd === 'misplaced_remedy'
        ? {
            remedy: 'ignore',
            outcome: 'ignored',
            name: null,
            sha256: 'ab'.repeat(32),
            originalKept: true,
            detail: 'a-token-from-a-later-shell' as RemedyDetail,
          }
        : undefined,
    )
    await screen.findByRole('table', { name: 'Misplaced files' })

    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Ignore' }),
    )

    await waitFor(() => {
      expect(remedyResultLine()).not.toBeNull()
    })
    expect(remedyResultLine()).toContain('a-token-from-a-later-shell')
    expect(screen.getByRole('region', { name: 'Wrong roots' })).toBeTruthy()
    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('renders an unrecognized outcome token as the raw token rather than throwing', async () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) =>
      cmd === 'misplaced_remedy'
        ? {
            remedy: 'ignore',
            outcome: 'archived-somewhere-new' as RemedyOutcome,
            name: null,
            sha256: 'ab'.repeat(32),
            originalKept: true,
            detail: null,
          }
        : undefined,
    )
    await screen.findByRole('table', { name: 'Misplaced files' })

    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Ignore' }),
    )

    await waitFor(() => {
      expect(remedyResultLine()).not.toBeNull()
    })
    expect(remedyResultLine()).toContain('archived-somewhere-new')
    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('renders an unrecognized output-discipline token and an unrecognized scope mode as their raw tokens, keeping the status line and the disclosures standing', async () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    mountWrongRoots({
      status: {
        outputDiscipline: 'partially-enforced' as OutputDiscipline,
        scopeMode: 'container-enforced' as ScopeMode,
        disclosures: [IN_PROJECT_DISCLOSURE],
        scanned: false,
        findings: 0,
      },
    })

    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })
    expect(outputDisciplineLine()).toContain('partially-enforced')
    expect(outputDisciplineLine()).toContain('container-enforced')
    // The assertions above are not enough on their own, and measurement
    // said so: the status line interpolates `status.outputDiscipline`
    // verbatim *as well as* through the formatter, so `toContain` passed
    // even with the formatter returning `undefined`. A formatter returning
    // `undefined` into a template literal does not throw -- it renders the
    // word `undefined` beside the token, which is the surface stating a
    // fact about the user's project in a word the shell never sent. That
    // is what this assertion catches.
    expect(outputDisciplineLine()).not.toContain('undefined')
    expect(outputDisciplineLine()).toBe(
      'output discipline: partially-enforced (scope container-enforced) — partially-enforced; not scanned yet',
    )
    expect(disclosureLines()).toEqual([IN_PROJECT_DISCLOSURE])
    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('drops a listing row that is not a finding -- the [{}] payload that threw "Cannot read properties of undefined (reading \'map\')" mid-render -- keeps the rows that are, and says the table is incomplete rather than passing a short list off as the whole one', async () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    mountWrongRoots({
      rows: () => [
        MISPLACED_REPORT,
        {},
        { name: 'half.pdf' },
        { ...MISPLACED_BUNDLE, remedies: null },
        null,
        'a finding',
      ],
    })
    await screen.findByRole('table', { name: 'Misplaced files' })

    // Exactly the one well-formed row, and a line saying the rest could not
    // be read: a shortened table must never read as a shorter list of
    // findings.
    expect(misplacedRows().map((cells) => cells[0])).toEqual(['docs/report.pdf'])
    expect(misplacedIncompleteLine()).toBe(
      'some findings could not be read and are not listed; the table below is incomplete',
    )
    // Never the other name: the table is on the screen, so a screen reader
    // must not be told the findings are unavailable (R3-047).
    expect(misplacedUnavailableLine()).toBeNull()
    expect(screen.getByRole('region', { name: 'Wrong roots' })).toBeTruthy()
    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('offers no button at all for a remedy token it does not recognize, and sends no misplaced_remedy for one: an unknown remedy is not assumed harmless just because it is unknown', async () => {
    // The cell used to ask `remedy === 'quarantine' ? gated : one click`,
    // so every token that was not that exact string got a one-click,
    // ungated button -- the fail-open default, on the one surface in this
    // product that deletes a file inside the user's project.
    const calls = mountWrongRoots({
      rows: [
        misplacedRow({
          remedies: ['purge', 'quarantine', 'shred', 'ignore'] as Remedy[],
        }),
      ],
    })
    await screen.findByRole('table', { name: 'Misplaced files' })

    expect(remedyButtons('docs/report.pdf')).toEqual(['Quarantine', 'Ignore'])
    expect(misplacedTable().textContent).not.toContain('purge')
    expect(misplacedTable().textContent).not.toContain('shred')
    expect(countCalls(calls, 'misplaced_remedy')).toBe(0)
  })

  it("sends nothing for a remedy the row's own remedies array does not carry, even when this renderer knows the token: the offered set is the row's, never the component's", async () => {
    // Reached through the block, which is the one place a remedy is run
    // from something other than the row's own button: open Quarantine over
    // a row whose `remedies` was refetched without it, and the confirm
    // button must send nothing even with the gate typed correctly.
    let rows: MisplacedRow[] = [MISPLACED_REPORT]
    const calls = mountWrongRoots({ rows: () => rows }, (cmd) =>
      cmd === 'wrongroot_scan' ? SCAN_SUMMARY : undefined,
    )
    await screen.findByRole('table', { name: 'Misplaced files' })
    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Quarantine' }),
    )
    await screen.findByLabelText('Quarantine confirmation')
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
    expect(quarantineButton().disabled).toBe(false)

    // The shell stops offering quarantine for this finding; the block is
    // still open over the same identity, and its button is still enabled.
    rows = [misplacedRow({ remedies: ['ignore'] })]
    fireEvent.click(scanButton())
    await waitFor(() => {
      expect(scanLine()).not.toBeNull()
    })
    const before = countCalls(calls, 'misplaced_remedy')
    if (screen.queryByLabelText('Quarantine confirmation') !== null) {
      fireEvent.click(quarantineButton())
      await flush()
    }

    expect(countCalls(calls, 'misplaced_remedy')).toBe(before)
  })
})

describe('AgentPanel quarantine gate refusals (slice 5d review, R3-018 / R1-015)', () => {
  it('refuses to confirm a row whose short digest is empty: an empty input would otherwise match it, and the button that removes a file from the project would become clickable having asked for nothing', async () => {
    const calls = mountWrongRoots({ rows: [misplacedRow({ sha256Short: '' })] })
    await screen.findByRole('table', { name: 'Misplaced files' })
    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Quarantine' }),
    )
    await screen.findByLabelText('Quarantine confirmation')

    // The input opens empty, which is exactly the value that would satisfy
    // an empty expected digest.
    expect(quarantineGate().value).toBe('')
    expect(quarantineButton().disabled).toBe(true)
    fireEvent.change(quarantineGate(), { target: { value: 'x' } })
    fireEvent.change(quarantineGate(), { target: { value: '' } })
    expect(quarantineButton().disabled).toBe(true)

    fireEvent.click(quarantineButton())
    await flush()
    expect(countCalls(calls, 'misplaced_remedy')).toBe(0)
  })

  it('refuses to confirm a row whose short digest is not a prefix of its full digest: the gate checks one fact and the request carries the other, and nothing had ever compared them', async () => {
    // The user reads and types `sha256Short`; `misplaced_remedy` is sent
    // `sha256`. If the two disagree, the user confirms one file and the
    // shell acts on another. They are two views of one identity or the row
    // is incoherent, and an incoherent row is refused.
    const calls = mountWrongRoots({
      rows: [misplacedRow({ sha256: 'ab'.repeat(32), sha256Short: 'cdcdcdcd' })],
    })
    await screen.findByRole('table', { name: 'Misplaced files' })
    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Quarantine' }),
    )
    await screen.findByLabelText('Quarantine confirmation')

    // Typing exactly what the label asks for still does not enable it.
    expect(quarantineBlock().querySelector('label')?.textContent).toBe(
      'Type the short digest (cdcdcdcd) to quarantine',
    )
    fireEvent.change(quarantineGate(), { target: { value: 'cdcdcdcd' } })
    expect(quarantineButton().disabled).toBe(true)

    fireEvent.click(quarantineButton())
    await flush()
    expect(countCalls(calls, 'misplaced_remedy')).toBe(0)
  })

  it('still confirms a coherent row, so the two refusals above are not simply refusing everything', async () => {
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) =>
      cmd === 'misplaced_remedy'
        ? {
            remedy: 'quarantine',
            outcome: 'quarantined',
            name: 'abababab-report.pdf',
            sha256: 'ab'.repeat(32),
            originalKept: false,
            detail: 'renamed',
          }
        : undefined,
    )
    await screen.findByRole('table', { name: 'Misplaced files' })
    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Quarantine' }),
    )
    await screen.findByLabelText('Quarantine confirmation')
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })

    expect(quarantineButton().disabled).toBe(false)
    fireEvent.click(quarantineButton())
    await waitFor(() => {
      expect(countCalls(calls, 'misplaced_remedy')).toBe(1)
    })
  })

  it("opens each row's gate empty and disabled, so a digest typed for one finding never stands as confirmation for another", async () => {
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT, MISPLACED_BUNDLE] })
    await screen.findByRole('table', { name: 'Misplaced files' })

    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Quarantine' }),
    )
    await screen.findByLabelText('Quarantine confirmation')
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
    expect(quarantineButton().disabled).toBe(false)

    fireEvent.click(
      within(misplacedTableRow('build/bundle.zip')).getByRole('button', { name: 'Quarantine' }),
    )
    await flush()

    // The second block is about the second finding, and it starts from
    // nothing: the first row's typed digest neither survives in the input
    // nor confirms this one.
    expect(quarantineBlockLines()[0]).toBe('name: build/bundle.zip')
    expect(quarantineGate().value).toBe('')
    expect(quarantineButton().disabled).toBe(true)
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
    expect(quarantineButton().disabled).toBe(true)
    fireEvent.change(quarantineGate(), { target: { value: 'cdcdcdcd' } })
    expect(quarantineButton().disabled).toBe(false)
    expect(countCalls(calls, 'misplaced_remedy')).toBe(0)
  })
})

describe('AgentPanel remedy outcome null branches (slice 5d review, R3-019 / R1-004)', () => {
  it('states that whether the original was kept was not reported when the wire says null, rather than claiming either -- assuming true under-reports a deletion, and assuming false reports one that never happened', async () => {
    mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) =>
      cmd === 'misplaced_remedy'
        ? {
            remedy: 'quarantine',
            outcome: 'quarantined',
            name: null,
            sha256: null,
            originalKept: null,
            detail: null,
          }
        : undefined,
    )
    await screen.findByRole('table', { name: 'Misplaced files' })
    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Quarantine' }),
    )
    await screen.findByLabelText('Quarantine confirmation')
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
    fireEvent.click(quarantineButton())

    await waitFor(() => {
      expect(remedyResultLine()).not.toBeNull()
    })
    // All three nullable fields absent: no produced name is named, and the
    // deletion question is answered "not reported" rather than answered.
    expect(remedyResultLine()).toBe(
      'quarantine: moved into quarantine, outside any workspace; whether the original was kept was not reported',
    )
    expect(remedyResultLine()).not.toContain('the original is gone from the project')
    expect(remedyResultLine()).not.toContain('this remedy did not remove the original')
    expect(remedyResultLine()).not.toContain('name ')
  })

  it('names no produced file when name is null but still reports the deletion the shell did make', async () => {
    mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) =>
      cmd === 'misplaced_remedy'
        ? {
            remedy: 'quarantine',
            outcome: 'quarantined',
            name: null,
            sha256: 'ab'.repeat(32),
            originalKept: false,
            detail: 'renamed',
          }
        : undefined,
    )
    await screen.findByRole('table', { name: 'Misplaced files' })
    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Quarantine' }),
    )
    await screen.findByLabelText('Quarantine confirmation')
    fireEvent.change(quarantineGate(), { target: { value: 'abababab' } })
    fireEvent.click(quarantineButton())

    await waitFor(() => {
      expect(remedyResultLine()).not.toBeNull()
    })
    expect(remedyResultLine()).toBe(
      'quarantine: moved into quarantine, outside any workspace; the original is gone from the project; moved by rename',
    )
  })
})

describe('AgentPanel misplaced_list fail-safe (slice 5d)', () => {
  it('renders no table, and no thrown render, for a misplaced_list payload that is not the list its DTO promises: the fail-safe-to-nothing discipline the panel already applies to a frame it does not recognize', async () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    mountWrongRoots({ rows: () => null })

    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })
    expect(screen.queryByRole('table', { name: 'Misplaced files' })).toBeNull()
    expect(screen.getByRole('region', { name: 'Wrong roots' })).toBeTruthy()
    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })
})

describe('AgentPanel a failed listing is not a clean project (slice 5d review, R1-009)', () => {
  it('says the findings could not be read when misplaced_list rejects, rather than rendering the empty table that a project with nothing misplaced renders', async () => {
    // A rejection set an empty list, which renders no table at all --
    // pixel-for-pixel what a clean project looks like. "I found nothing"
    // and "I could not look" are opposite facts, and this is the one
    // surface where the second silently rendering as the first is the whole
    // failure: a detection surface that stops detecting and says nothing.
    mountWrongRoots({ rows: () => Promise.reject({ code: 'unexpected', message: 'gone' }) })

    await waitFor(() => {
      expect(misplacedUnavailableLine()).not.toBeNull()
    })
    expect(misplacedUnavailableLine()).toBe(
      'the findings could not be read; this is not a report that nothing is misplaced',
    )
    expect(screen.queryByRole('table', { name: 'Misplaced files' })).toBeNull()
    expect(screen.getByRole('region', { name: 'Wrong roots' })).toBeTruthy()
  })

  it('says it for a payload that is not a list either, and does not say it for a project that genuinely has nothing misplaced', async () => {
    mountWrongRoots({ rows: () => null })
    await waitFor(() => {
      expect(misplacedUnavailableLine()).not.toBeNull()
    })
    expect(misplacedUnavailableLine()).toContain('this is not a report that nothing is misplaced')
    cleanup()
    clearMocks()

    // The other half, without which the line above would just be permanent
    // furniture: an empty listing that *is* the shell's answer says nothing.
    mountWrongRoots({ rows: [] })
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })
    expect(misplacedUnavailableLine()).toBeNull()
    expect(screen.queryByRole('table', { name: 'Misplaced files' })).toBeNull()
  })

  it('is contradicted by nothing: a status line still reporting findings beside an unreadable listing keeps the line that says the listing failed', async () => {
    // The worst reading of the old behaviour: `scanned, findings 2` on the
    // status line, no table under it, and nothing to say the two disagree.
    mountWrongRoots({
      status: wrongRootStatusAdvisory({ scanned: true, findings: 2 }),
      rows: () => Promise.reject({ code: 'unexpected', message: 'gone' }),
    })

    await waitFor(() => {
      expect(misplacedUnavailableLine()).not.toBeNull()
    })
    expect(outputDisciplineLine()).toContain('scanned, findings 2')
    expect(misplacedUnavailableLine()).toContain('this is not a report that nothing is misplaced')
  })

  it('clears the line once a later listing succeeds, so it never outlives the failure it reports', async () => {
    let fail = true
    mountWrongRoots(
      {
        rows: () =>
          fail ? Promise.reject({ code: 'unexpected', message: 'gone' }) : [MISPLACED_REPORT],
      },
      (cmd) => (cmd === 'wrongroot_scan' ? SCAN_SUMMARY : undefined),
    )
    await waitFor(() => {
      expect(misplacedUnavailableLine()).not.toBeNull()
    })

    fail = false
    fireEvent.click(scanButton())

    await screen.findByRole('table', { name: 'Misplaced files' })
    expect(misplacedUnavailableLine()).toBeNull()
  })
})

describe('AgentPanel guidance_snapshots fail-safe (slice 5c review, R3-011)', () => {
  it.each([null, undefined, { id: '0123456789abcdef' }, 'snapshots'])(
    'renders no snapshots table, and no thrown render, for a guidance_snapshots payload that is not the list its DTO promises (%s): the panel does not simply trust this DTO, because a render-time throw takes the whole panel down with it, freeze guards included',
    async (payload) => {
      const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
      mountGuidance({ note: noteStatus({ snapshots: 3, pinned: 1 }), noteSnapshots: () => payload })

      await waitFor(() => {
        expect(noteStatusLine()).not.toBeNull()
      })
      expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).toBeNull()
      // The rest of the section still stands: the failure is confined to
      // the table it would have rendered.
      expect(within(noteBlock()).getByRole('button', { name: 'Preview' })).toBeTruthy()
      expect(screen.getByRole('region', { name: 'Guidance' })).toBeTruthy()
      expect(consoleErrorSpy).not.toHaveBeenCalled()
      consoleErrorSpy.mockRestore()
      cleanup()
      clearMocks()
    },
  )
})

describe('AgentPanel synchronous busy refs (slice 5c review, R3-014)', () => {
  /**
   * Two clicks with no React re-render between them. `fireEvent.click` is
   * `act`-wrapped, so React flushes after the first click and the second
   * one lands on an already-disabled button -- which exercises the
   * `disabled` attribute, not the ref beneath it. Dispatching both inside a
   * single `act` keeps the render batched, so the second click reaches the
   * handler with the button still enabled: what refuses it then is the ref
   * alone, which is the thing under test.
   */
  function doubleClickWithoutRerender(button: HTMLElement): void {
    act(() => {
      button.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      button.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
  }

  it("refuses a second guidance_pin that lands before React has re-rendered the disabled button: the ref, not the attribute, is what makes a double-click pin once", async () => {
    const calls = mountGuidance(
      { note: noteStatus({ snapshots: 3, pinned: 1 }), noteSnapshots: NOTE_SNAPSHOTS },
      (cmd) =>
        cmd === 'guidance_pin'
          ? new Promise<Snapshot>(() => {
              // Never settles: the pin stays in flight for both clicks.
            })
          : undefined,
    )
    await waitFor(() => {
      expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).not.toBeNull()
    })

    const pin = within(snapshotRow('89abcdef01234567')).getByRole('button', { name: 'Pin' })
    expect((pin as HTMLButtonElement).disabled).toBe(false)
    doubleClickWithoutRerender(pin)

    await flush()
    expect(countCalls(calls, 'guidance_pin')).toBe(1)
  })

  it('refuses a second guidance write that lands before React has re-rendered the disabled button: the write must happen once per confirmed gate, never twice', async () => {
    const calls = mountGuidance({ note: noteStatus({ managed: 'outdated' }) }, (cmd) => {
      if (cmd === 'guidance_preview') return PREVIEW_REPLACE
      if (cmd === 'guidance_apply') {
        return new Promise<GuidanceApplied>(() => {
          // Never settles: the write stays in flight for both clicks.
        })
      }
      return undefined
    })
    await openNotePreview()
    typeGate(FILE_SHORT)
    expect(finalButton('Write').disabled).toBe(false)

    doubleClickWithoutRerender(finalButton('Write'))

    await flush()
    expect(countCalls(calls, 'guidance_apply')).toBe(1)
  })

  it('refuses a second wrong-root remedy that lands before React has re-rendered the disabled buttons: quarantine removes the original, so it must run once per choice', async () => {
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) =>
      cmd === 'misplaced_remedy'
        ? new Promise(() => {
            // Never settles: the remedy stays in flight for both clicks.
          })
        : undefined,
    )
    await screen.findByRole('table', { name: 'Misplaced files' })

    const ignore = within(misplacedTableRow('docs/report.pdf')).getByRole('button', {
      name: 'Ignore',
    })
    expect((ignore as HTMLButtonElement).disabled).toBe(false)
    doubleClickWithoutRerender(ignore)

    await flush()
    expect(countCalls(calls, 'misplaced_remedy')).toBe(1)
  })

  it('refuses a second wrongroot_scan that lands before React has re-rendered the disabled button: one walk per click', async () => {
    const calls = mountWrongRoots({ rows: [] }, (cmd) =>
      cmd === 'wrongroot_scan'
        ? new Promise(() => {
            // Never settles: the scan stays in flight for both clicks.
          })
        : undefined,
    )
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })

    expect(scanButton().disabled).toBe(false)
    doubleClickWithoutRerender(scanButton())

    await flush()
    expect(countCalls(calls, 'wrongroot_scan')).toBe(1)
  })
})

describe('AgentPanel pin during a run (slice 5c review, R3-015)', () => {
  it("actually pins while a run is active: the click calls guidance_pin with exactly { id, pinned }, the row takes the store's answer, and the kind's status is refetched -- the shell's own guidance_pin takes no supervisor, so freezing it would be stricter than the contract", async () => {
    let channel: LiveChannel | undefined
    const calls = mountGuidance(
      { note: noteStatus({ snapshots: 3, pinned: 1 }), noteSnapshots: NOTE_SNAPSHOTS },
      (cmd, args) => {
        if (cmd === 'harness_spawn') {
          channel = (args as { onFrame: LiveChannel }).onFrame
          return 7
        }
        if (cmd === 'guidance_pin') return { ...SNAPSHOT_NEWEST, pinned: true }
        return undefined
      },
    )
    await waitFor(() => {
      expect(screen.queryByRole('table', { name: 'Guidance note snapshots' })).not.toBeNull()
    })

    await startRunOverWorkspace()
    const before = calls.length

    fireEvent.click(within(snapshotRow('89abcdef01234567')).getByRole('button', { name: 'Pin' }))

    await waitFor(() => {
      expect(countCalls(calls, 'guidance_pin')).toBe(1)
    })
    expect(calls.find((call) => call.cmd === 'guidance_pin')?.args).toEqual({
      id: '89abcdef01234567',
      pinned: true,
    })
    await waitFor(() => {
      expect(snapshotRows()[0]?.slice(6)).toEqual(['yes', 'Unpin Restore'])
    })
    expect(calls.slice(before).map((call) => call.cmd)).toContain('guidance_status')

    if (!channel) throw new Error('harness_spawn was not called')
    await endRun(channel)
  })
})

describe('AgentPanel one run per tick (slice 5d second review, R3-033 / R3-034)', () => {
  /**
   * Selects an adapter, an approval and a prompt on a panel
   * `mountWithWorkspace` mounted, without clicking Start: the tests below
   * dispatch their own clicks inside a single `act`.
   */
  async function armStart(): Promise<HTMLButtonElement> {
    await selectOption('Adapter', 'claude-code')
    await selectOption('Approval', '42')
    fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: 'do the thing' } })
    const start = screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement
    expect(start.disabled).toBe(false)
    return start
  }

  it('spawns once for two Start clicks that land in the same tick, and holds the process it spawned: the second spawn used to win the generation check, so the first child process got no active id, no Stop button and no transcript -- an orphan this surface could never reach again', async () => {
    // `handleStart` set `runActiveRef` and never read it. Start carries
    // `disabled={runActive || ...}` like every other entry point, and that
    // attribute is one render behind a run that has just started, so two
    // clicks batched into one tick both reached the handler with the button
    // still enabled -- on the one entry point that starts a child process.
    const spawned: number[] = []
    const calls = mountWithWorkspace((cmd) => {
      if (cmd === 'harness_spawn') {
        const id = 7 + spawned.length
        spawned.push(id)
        return id
      }
      if (cmd === 'harness_stop') return { state: 'killed', code: null }
      return undefined
    })
    await screen.findByLabelText('Adapter')
    const start = await armStart()

    act(() => {
      start.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      start.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    await flush()

    expect(countCalls(calls, 'harness_spawn')).toBe(1)
    expect(spawned).toEqual([7])
    // And the id the panel is holding is that one process, not a second one
    // whose spawn the first click's generation would have been discarded
    // for: Stop reaches the process this panel actually started.
    await screen.findByText('running')
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await waitFor(() => {
      expect(countCalls(calls, 'harness_stop')).toBe(1)
    })
    expect(calls.find((call) => call.cmd === 'harness_stop')?.args).toEqual({
      id: 7,
      deadlineMs: 2000,
    })
  })

  it('keeps every remedy frozen while any spawn this panel started is still pending, even once an earlier one has settled: the run-active flag falls when the last spawn settles, never when the first does', async () => {
    // The two halves together. `runActive` is `isSpawning || activeId !==
    // null`, and `isSpawning` was a boolean: a spawn whose generation the
    // next Start had already replaced still cleared it in its own
    // `finally`, the mirroring effect lowered the ref behind it, and the
    // remedies came back while a process was live and a second spawn was
    // still in flight. The Start guard stops the second spawn; the pending
    // count is what makes the flag describe *every* spawn rather than the
    // last one to settle.
    let settleSecond: ((id: number) => void) | undefined
    const spawned: number[] = []
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) => {
      if (cmd === 'harness_spawn') {
        spawned.push(7 + spawned.length)
        if (spawned.length === 1) return 7
        return new Promise<number>((resolve) => {
          settleSecond = resolve
        })
      }
      return undefined
    })
    await screen.findByRole('table', { name: 'Misplaced files' })
    const start = await armStart()

    act(() => {
      start.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      start.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    await flush()

    // The freeze first, because it is what this test is for: whatever
    // happened to the second click, a spawn has settled and the remedies
    // must still refuse. Asserted before the spawn count so that a
    // regression in the freeze is reported as a freeze failure rather than
    // masked by the count assertion firing first.
    const row = misplacedTableRow('docs/report.pdf')
    fireEvent.click(within(row).getByRole('button', { name: 'Ignore' }))
    fireEvent.click(within(row).getByRole('button', { name: 'Quarantine' }))
    await flush()

    expect(countCalls(calls, 'misplaced_remedy')).toBe(0)
    expect(screen.queryByLabelText('Quarantine confirmation')).toBeNull()
    // And there was only ever one spawn to settle.
    expect(countCalls(calls, 'harness_spawn')).toBe(1)
    expect(settleSecond).toBeUndefined()
  })
})

describe('AgentPanel a listing row is checked field by field (slice 5d second review, R3-035)', () => {
  /**
   * One entry per field `isMisplacedRow` checks, each carrying a value of
   * the wrong type for that field alone.
   *
   * The fixture the original test used was `[{}]` and `{ name: 'half.pdf' }`
   * -- rows missing everything, which only ever exercised the first check
   * that happened to fail. Deleting any single field's check left the suite
   * green, and the one for `sha256Short` mattered most: the cell renders it
   * through `PlainTextLine`, which calls `Array.from(text)`, so a row
   * without it threw mid-render and -- before the boundary -- took the whole
   * panel with it. Each field gets its own row here, so each check is what
   * the assertion is about.
   */
  const WRONG_TYPED_FIELDS: [keyof MisplacedRow, unknown][] = [
    ['name', 42],
    ['size', '4096'],
    ['sha256', undefined],
    ['sha256Short', undefined],
    ['detectedType', null],
    ['class', 7],
    ['reason', undefined],
    ['remedies', 'quarantine'],
  ]

  it.each(WRONG_TYPED_FIELDS)(
    'drops a listing row whose %s is not the type the DTO promises, keeps the well-formed row beside it, and says the table is incomplete',
    async (field, value) => {
      const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
      mountWrongRoots({
        rows: () => [{ ...MISPLACED_BUNDLE, [field]: value }, MISPLACED_REPORT],
      })
      await screen.findByRole('table', { name: 'Misplaced files' })

      expect(misplacedRows().map((cells) => cells[0])).toEqual(['docs/report.pdf'])
      expect(misplacedIncompleteLine()).toBe(
        'some findings could not be read and are not listed; the table below is incomplete',
      )
      // The section is still standing, and nothing threw on the way: the
      // half-populated row is refused, not repaired and not rendered.
      expect(screen.getByRole('region', { name: 'Wrong roots' })).toBeTruthy()
      expect(remedyButtons('docs/report.pdf')).toEqual([
        'Quarantine',
        'Publish to outbox',
        'Ignore',
      ])
      expect(consoleErrorSpy).not.toHaveBeenCalled()
      consoleErrorSpy.mockRestore()
      cleanup()
      clearMocks()
    },
  )

  it("drops a row whose remedies array is a list of something other than tokens: a row this surface would offer buttons for is refused before it can offer one", async () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    mountWrongRoots({
      rows: () => [{ ...MISPLACED_BUNDLE, remedies: ['quarantine', 7] }, MISPLACED_REPORT],
    })
    await screen.findByRole('table', { name: 'Misplaced files' })

    expect(misplacedRows().map((cells) => cells[0])).toEqual(['docs/report.pdf'])
    expect(misplacedIncompleteLine()).not.toBeNull()
    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })
})

describe('AgentPanel a remedy clears the banner (slice 5d second review, R3-036)', () => {
  it('clears a refusal when a later remedy succeeds, so a quarantine-unavailable banner never stands above the receipt of the Publish that worked', async () => {
    // R3-030 was recorded as covered, and it is -- for `handleScan`. The
    // scan and the remedies each call `setError(null)` on their own way in,
    // and only the scan's had a test: deleting `runRemedy`'s left the whole
    // suite green, with a refusal about the quarantine directory standing
    // above a receipt saying a copy reached the outbox.
    let fail = true
    mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) => {
      if (cmd !== 'misplaced_remedy') return undefined
      return fail
        ? Promise.reject({
            code: 'quarantine-unavailable',
            message: 'the quarantine directory could not be used',
          })
        : {
            remedy: 'publish',
            outcome: 'copied-to-outbox',
            name: 'abababab-report.pdf',
            sha256: 'ab'.repeat(32),
            originalKept: true,
            detail: null,
          }
    })
    await screen.findByRole('table', { name: 'Misplaced files' })

    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Ignore' }),
    )
    const banner = await screen.findByRole('alert')
    expect(banner.textContent).toBe(
      'quarantine-unavailable: the quarantine directory could not be used',
    )

    fail = false
    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', {
        name: 'Publish to outbox',
      }),
    )

    await waitFor(() => {
      expect(remedyResultLine()).not.toBeNull()
    })
    expect(remedyResultLine()).toContain('copied into the outbox as a new unattributed entry')
    expect(screen.queryByRole('alert')).toBeNull()
  })
})

describe('AgentPanel a state frame is checked before it is read (slice 5d second review, R3-049)', () => {
  /**
   * The listing rows got a structural check this pass; the transcript's own
   * `state` payload did not, and it is the one reachable from a live frame
   * rather than from a fetch. `formatStateEventLine` called
   * `payload.observations.map` at render, so a frame whose `observations`
   * is not a list threw inside the transcript -- which closes the whole
   * Agent panel, taking the wrong-roots table, the remedies, the freeze
   * guards and Stop with it.
   */
  it.each([null, undefined, 'cwd: /work', { key: 'cwd', value: '/work' }, 7])(
    'renders a state frame whose observations is %s as its phase plus a line saying some observations could not be read, rather than throwing mid-render',
    async (observations) => {
      const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
      const channel = await startAndCaptureChannel()

      act(() => {
        channel.onmessage({
          stream: 'event',
          body: {
            id: 7,
            seq: 4,
            droppedBefore: 0,
            kind: 'state',
            payload: { phase: 'init', subtype: null, observations },
          },
        } as unknown as HarnessFrame)
      })

      const transcript = screen.getByLabelText('Agent transcript')
      expect(transcript.textContent).toContain(
        'init, some observations could not be read and are not shown',
      )
      // The panel is still standing, which is the whole point: this frame
      // used to take it down.
      expect(screen.getByRole('button', { name: 'Stop' })).toBeTruthy()
      expect(consoleErrorSpy).not.toHaveBeenCalled()
      consoleErrorSpy.mockRestore()
      cleanup()
      clearMocks()
    },
  )

  it('keeps the observations it can read beside the notice for the ones it cannot, so a partly readable frame is neither dropped whole nor passed off as complete', async () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
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
            phase: 'finished',
            subtype: 'success',
            observations: [
              { key: 'cwd', value: '/work' },
              // One of each half missing, so neither field's check can be
              // dropped without this line noticing: a `key`-less
              // observation renders as `undefined: ...` and a `value`-less
              // one as `...: undefined`, both of which are this surface
              // stating a fact in a word the shell never sent.
              { key: 'model' },
              { value: 'only-a-value' },
              null,
              { key: 'tokens', value: '12' },
            ],
          },
        },
      } as unknown as HarnessFrame)
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toContain(
      'finished, success, cwd: /work, tokens: 12, some observations could not be read and are not shown',
    )
    expect(transcript.textContent).not.toContain('undefined')
    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('says nothing about dropped observations for a frame whose observations are all readable, so the notice is never permanent furniture', async () => {
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
            observations: [{ key: 'cwd', value: '/work' }],
          },
        },
      })
    })

    const transcript = screen.getByLabelText('Agent transcript')
    expect(transcript.textContent).toContain('init, cwd: /work')
    expect(transcript.textContent).not.toContain('could not be read')
  })
})

describe('AgentPanel a remedy that does not answer (slice 5d second review, R3-042)', () => {
  beforeEach(() => {
    // `shouldAdvanceTime` keeps the fake clock following real time, so
    // Testing Library's own `waitFor` polling still runs while
    // `advanceTimersByTime` drives the notice window.
    vi.useFakeTimers({ shouldAdvanceTime: true })
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  /** The line stated while a remedy has been in flight past the notice window. */
  function remedyUnansweredLine(): string | null {
    return screen.queryByRole('status', { name: 'Remedy unanswered' })?.textContent ?? null
  }

  it('says the remedy has not answered once it has been in flight past the notice window, keeps every remedy and the workspace pick frozen because the request cannot be withdrawn, and takes the line back the moment it answers', async () => {
    // `misplaced_remedy` takes `{ name, sha256, remedy }` and no deadline,
    // unlike `harness_stop`, so the renderer has none to hand it and no way
    // to withdraw a request already made. A shell that never settled left
    // every remedy and the pick frozen for the session with nothing said.
    // The freeze is right -- the file may be part-way out of the project --
    // and the silence was not.
    let settle: ((result: unknown) => void) | undefined
    const calls = mountWrongRoots({ rows: [MISPLACED_REPORT] }, (cmd) =>
      cmd === 'misplaced_remedy'
        ? new Promise((resolve) => {
            settle = resolve
          })
        : undefined,
    )
    await screen.findByRole('table', { name: 'Misplaced files' })
    fireEvent.click(
      within(misplacedTableRow('docs/report.pdf')).getByRole('button', { name: 'Ignore' }),
    )

    // Nothing said while the wait is still ordinary.
    expect(remedyUnansweredLine()).toBeNull()

    act(() => {
      vi.advanceTimersByTime(10_000)
    })

    expect(remedyUnansweredLine()).toBe(
      'the remedy has not answered yet; it may still be running, so nothing here can say whether the file has moved, and the findings stay frozen until it answers',
    )
    // And the freeze is still on, which is the half the notice does not
    // change: the request is out, and this surface cannot take it back.
    expect(remedyButtons('docs/report.pdf')).toEqual([
      'Quarantine',
      'Publish to outbox',
      'Ignore',
    ])
    const row = misplacedTableRow('docs/report.pdf')
    expect(
      (within(row).getByRole('button', { name: 'Quarantine' }) as HTMLButtonElement).disabled,
    ).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: 'Pick workspace' }))
    expect(countCalls(calls, 'workspace_pick')).toBe(0)
    expect(countCalls(calls, 'misplaced_remedy')).toBe(1)

    if (!settle) throw new Error('misplaced_remedy was not called')
    await act(async () => {
      settle?.({
        remedy: 'ignore',
        outcome: 'ignored',
        name: null,
        sha256: null,
        originalKept: true,
        detail: null,
      })
    })

    await waitFor(() => {
      expect(remedyResultLine()).not.toBeNull()
    })
    expect(remedyUnansweredLine()).toBeNull()
  })
})

describe('AgentPanel duplicate remedy tokens (slice 5d second review, R3-052)', () => {
  it('renders one button per entry of a row whose remedies array repeats a token, with no duplicate React key: nothing on the wire promises the tokens are distinct', async () => {
    // A duplicate key is reported on `console.error`, which this suite
    // treats as a failure signal -- so the assertion below is what catches
    // the regression, not the button count.
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    mountWrongRoots({
      rows: [misplacedRow({ remedies: ['ignore', 'ignore', 'quarantine'] })],
    })
    await screen.findByRole('table', { name: 'Misplaced files' })

    expect(remedyButtons('docs/report.pdf')).toEqual(['Ignore', 'Ignore', 'Quarantine'])
    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })
})

describe('AgentPanel the other same-tick entry points (slice 5d second review, R3-037)', () => {
  /**
   * `runRemedy`'s synchronous ref read had a test; `openQuarantine`'s and
   * `handlePickWorkspace`'s did not, and both could be deleted with the
   * whole suite green. They are the same window on the same tick -- Start
   * raises the run inside a click handler, and every button that reads
   * `runActive` is one render behind it -- and one of them opens the
   * confirmation block for the remedy that deletes a file.
   */
  async function armStartOver(rows: MisplacedRow[]): Promise<{
    calls: Call[]
    start: HTMLButtonElement
  }> {
    const calls = mountWrongRoots({ rows }, (cmd) =>
      cmd === 'harness_spawn'
        ? new Promise<number>(() => {
            // Never settles: the run stays in its spawning window.
          })
        : undefined,
    )
    // The discipline line rather than the table: one of these tests mounts
    // over a project with nothing misplaced, which renders no table at all.
    await waitFor(() => {
      expect(outputDisciplineLine()).not.toBeNull()
    })
    await selectOption('Adapter', 'claude-code')
    await selectOption('Approval', '42')
    fireEvent.change(screen.getByLabelText('Prompt'), { target: { value: 'do the thing' } })
    const start = screen.getByRole('button', { name: 'Start' }) as HTMLButtonElement
    expect(start.disabled).toBe(false)
    return { calls, start }
  }

  it('opens no quarantine confirmation for a click that lands in the same tick as Start: the block is the first step of the one remedy that deletes a file, and it must not open over a run that has just begun', async () => {
    const { calls, start } = await armStartOver([MISPLACED_REPORT])
    const quarantine = within(misplacedTableRow('docs/report.pdf')).getByRole('button', {
      name: 'Quarantine',
    }) as HTMLButtonElement
    expect(quarantine.disabled).toBe(false)

    act(() => {
      start.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      quarantine.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    await flush()

    expect(screen.queryByLabelText('Quarantine confirmation')).toBeNull()
    expect(countCalls(calls, 'harness_spawn')).toBe(1)
  })

  it('opens no workspace picker for a click that lands in the same tick as Start: the project must not move under a run that has just begun', async () => {
    const { calls, start } = await armStartOver([])
    const pick = screen.getByRole('button', { name: 'Pick workspace' }) as HTMLButtonElement
    expect(pick.disabled).toBe(false)

    act(() => {
      start.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      pick.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    await flush()

    expect(countCalls(calls, 'workspace_pick')).toBe(0)
    expect(countCalls(calls, 'harness_spawn')).toBe(1)
  })
})
