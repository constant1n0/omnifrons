// ADR-0005 sub-slice 1i: the renderer surface for the agent-definition scan
// and verify. Mocked at the Tauri IPC boundary with `mockIPC`, exactly like
// `AgentPanel.test.tsx` and `ipc/harness.test.ts` -- this codebase has no
// `vi.mock('./ipc/harness')` precedent; every panel test drives the real
// wrapper functions against a mocked `invoke`.

import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { StrictMode } from 'react'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { DefinitionSection } from './DefinitionSection'
import type { DefinitionStatus, DefinitionVerify, ShellError } from './ipc/harness'

const FIXTURE_STATUS: DefinitionStatus = {
  schema: 'omnifrons.definition-scan/1',
  root: '/workspace',
  ancestors: [{ path: '/outside', identityFiles: ['AGENTS.md'], hostExtras: ['CLAUDE.local.md'] }],
  agents: [
    {
      path: '.',
      state: 'agents-only',
      identityFiles: [{ kind: 'AGENTS.md', sha256: 'ab'.repeat(32), bytes: 42, symlink: null }],
      title: 'Fixture agent',
      parent: null,
      nested: ['sub'],
      consentRequired: true,
      skills: {
        discovered: ['skills/x/SKILL.md'],
        indexed: [],
        missing: ['skills/x/SKILL.md'],
        dangling: [],
        precedence: false,
      },
      warnings: [
        {
          code: 'extra-host-file',
          hosts: ['claude-code'],
          detail: 'a host-specific file sits alongside the identity files',
        },
      ],
    },
  ],
}

const NESTED_AGENT: DefinitionStatus['agents'][number] = {
  ...FIXTURE_STATUS.agents[0],
  path: 'sub/agent',
  title: 'Nested fixture agent',
  parent: { kind: 'root', path: '.' },
  nested: [],
  consentRequired: false,
  warnings: [],
}

const FIXTURE_STATUS_NESTED: DefinitionStatus = {
  ...FIXTURE_STATUS,
  agents: [FIXTURE_STATUS.agents[0], NESTED_AGENT],
}

const FIXTURE_STATUS_RESCANNED: DefinitionStatus = { ...FIXTURE_STATUS, root: '/workspace-rescanned' }

const FIXTURE_VERIFY: DefinitionVerify = {
  schema: 'omnifrons.definition-verify/1',
  agent: '/workspace',
  state: 'none',
  canonical: false,
  problems: ['state is none'],
}

afterEach(() => {
  clearMocks()
})

describe('DefinitionSection (ADR-0005 sub-slice 1i)', () => {
  it('renders the ancestors, agents, warnings, and consent marker from the scanned status', async () => {
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return FIXTURE_STATUS
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)

    const section = await screen.findByLabelText('Definition')
    expect(within(section).getByText('/workspace')).toBeTruthy()
    expect(within(section).getByText('/outside')).toBeTruthy()
    // "AGENTS.md" appears twice: once in the ancestor's identity files and
    // once in the agent's own identity file row.
    expect(within(section).getAllByText('AGENTS.md').length).toBe(2)
    expect(within(section).getByText('extra-host-file')).toBeTruthy()
    expect(within(section).getByText(/consent required/)).toBeTruthy()
  })

  it('calls definition_status again when "Rescan" is clicked, replacing the shown report', async () => {
    let calls = 0
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        calls += 1
        return calls === 1 ? FIXTURE_STATUS : FIXTURE_STATUS_RESCANNED
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    fireEvent.click(screen.getByRole('button', { name: 'Rescan' }))

    await screen.findByText('/workspace-rescanned')
    expect(calls).toBe(2)
  })

  it('clears a previous verification result when "Rescan" is clicked', async () => {
    let statusCalls = 0
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        statusCalls += 1
        return statusCalls === 1 ? FIXTURE_STATUS : FIXTURE_STATUS_RESCANNED
      }
      if (cmd === 'definition_verify') return FIXTURE_VERIFY
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    fireEvent.click(screen.getByRole('button', { name: 'Verify .' }))
    await screen.findByLabelText('Verification')

    fireEvent.click(screen.getByRole('button', { name: 'Rescan' }))

    await screen.findByText('/workspace-rescanned')
    expect(screen.queryByLabelText('Verification')).toBeNull()
  })

  it("renders the verification's state, canonical flag, and problems when the '.' row's Verify is clicked", async () => {
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return FIXTURE_STATUS
      if (cmd === 'definition_verify') return FIXTURE_VERIFY
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    fireEvent.click(screen.getByRole('button', { name: 'Verify .' }))

    const verification = await screen.findByLabelText('Verification')
    expect(within(verification).getByText('none')).toBeTruthy()
    expect(within(verification).getByText('canonical: no')).toBeTruthy()
    expect(within(verification).getByText('state is none')).toBeTruthy()
  })

  it('clears a stale verification result when a later verify rejects, showing only the new error', async () => {
    let shouldFail = false
    // `scan-failed` is the code `definition_verify`'s own definition-tree
    // read failure shares with `definition_status` (see `ShellErrorCode`'s
    // own doc comment on `scan-failed`), not a dedicated "verify" code.
    const error: ShellError = { code: 'scan-failed', message: 'the root could not be verified' }
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return FIXTURE_STATUS
      if (cmd === 'definition_verify') return shouldFail ? Promise.reject(error) : FIXTURE_VERIFY
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    fireEvent.click(screen.getByRole('button', { name: 'Verify .' }))
    await screen.findByLabelText('Verification')

    shouldFail = true
    fireEvent.click(screen.getByRole('button', { name: 'Verify .' }))

    const banner = await screen.findByTestId('definition-verify-error')
    expect(within(banner).getByText('scan-failed')).toBeTruthy()
    expect(within(banner).getByText(/the root could not be verified/)).toBeTruthy()
    expect(screen.queryByLabelText('Verification')).toBeNull()
  })

  it('ignores a second "Verify ." click while a verify is pending, calling definition_verify exactly once', async () => {
    let calls = 0
    let resolveVerify: ((value: DefinitionVerify) => void) | undefined
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return FIXTURE_STATUS
      if (cmd === 'definition_verify') {
        calls += 1
        return new Promise((resolve) => {
          resolveVerify = resolve as (value: DefinitionVerify) => void
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    const verifyButton = screen.getByRole('button', { name: 'Verify .' }) as HTMLButtonElement
    fireEvent.click(verifyButton)

    await waitFor(() => {
      expect(verifyButton.disabled).toBe(true)
    })
    fireEvent.click(verifyButton)

    if (!resolveVerify) throw new Error('definition_verify was not called')
    await act(async () => {
      resolveVerify!(FIXTURE_VERIFY)
    })

    await screen.findByLabelText('Verification')
    expect(calls).toBe(1)
  })

  it('calls definition_verify exactly once even when two "Verify ." clicks are dispatched before a re-render settles', async () => {
    let calls = 0
    let resolveVerify: ((value: DefinitionVerify) => void) | undefined
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return FIXTURE_STATUS
      if (cmd === 'definition_verify') {
        calls += 1
        return new Promise((resolve) => {
          resolveVerify = resolve as (value: DefinitionVerify) => void
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    const verifyButton = screen.getByRole('button', { name: 'Verify .' })
    // Both clicks are dispatched inside one `act` call, so React never gets
    // a chance to re-render between them -- unlike the double-click test
    // above, which awaits the `disabled` attribute first and so only
    // proves that attribute, never that `runVerify` itself is reentrant-safe
    // against two clicks racing on the pre-re-render `verifying` closure.
    act(() => {
      fireEvent.click(verifyButton)
      fireEvent.click(verifyButton)
    })
    expect(calls).toBe(1)

    if (!resolveVerify) throw new Error('definition_verify was not called')
    await act(async () => {
      resolveVerify!(FIXTURE_VERIFY)
    })
  })

  it('renders a ShellError from definition_status as its code and message, never the raw object', async () => {
    // The mount-time scan suppresses its own error (it meets a rejection
    // with an empty report, like `AgentPanel`'s own automatic fetches) --
    // only the explicit "Rescan" click surfaces it, so this clicks Rescan
    // once the quiet initial scan has settled.
    const error: ShellError = { code: 'scan-failed', message: 'the workspace could not be scanned' }
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return Promise.reject(error)
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await waitFor(() => {
      expect((screen.getByRole('button', { name: 'Rescan' }) as HTMLButtonElement).disabled).toBe(false)
    })
    fireEvent.click(screen.getByRole('button', { name: 'Rescan' }))

    const banner = await screen.findByRole('alert')
    expect(within(banner).getByText('scan-failed')).toBeTruthy()
    expect(within(banner).getByText(/the workspace could not be scanned/)).toBeTruthy()
    expect(banner.textContent).not.toContain('[object Object]')
  })

  it('clears the old report when a user-initiated "Rescan" fails, showing only the error', async () => {
    let shouldFail = false
    const error: ShellError = { code: 'scan-failed', message: 'the workspace could not be rescanned' }
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return shouldFail ? Promise.reject(error) : FIXTURE_STATUS
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    shouldFail = true
    fireEvent.click(screen.getByRole('button', { name: 'Rescan' }))

    const banner = await screen.findByRole('alert')
    expect(within(banner).getByText('scan-failed')).toBeTruthy()
    expect(within(banner).getByText(/the workspace could not be rescanned/)).toBeTruthy()
    expect(screen.queryByText('/workspace')).toBeNull()
  })

  it('shows a loading state before the scan resolves, then replaces it with the report', async () => {
    let resolveStatus: ((value: DefinitionStatus) => void) | undefined
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        return new Promise((resolve) => {
          resolveStatus = resolve as (value: DefinitionStatus) => void
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)

    await screen.findByRole('status')
    expect(screen.queryByText('/workspace')).toBeNull()

    if (!resolveStatus) throw new Error('definition_status was not called')
    await act(async () => {
      resolveStatus!(FIXTURE_STATUS)
    })

    await screen.findByText('/workspace')
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('drops a verify result that settles after a rescan has already replaced the report', async () => {
    let statusCalls = 0
    let resolveVerify: ((value: DefinitionVerify) => void) | undefined
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        statusCalls += 1
        return statusCalls === 1 ? FIXTURE_STATUS : FIXTURE_STATUS_RESCANNED
      }
      if (cmd === 'definition_verify') {
        return new Promise((resolve) => {
          resolveVerify = resolve as (value: DefinitionVerify) => void
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    fireEvent.click(screen.getByRole('button', { name: 'Verify .' }))
    if (!resolveVerify) throw new Error('definition_verify was not called')

    // The rescan's own `definition_status` call resolves while the verify
    // above is still pending -- a verification belongs to the report it
    // was run against, so this rescan invalidates it.
    fireEvent.click(screen.getByRole('button', { name: 'Rescan' }))
    await screen.findByText('/workspace-rescanned')

    await act(async () => {
      resolveVerify!(FIXTURE_VERIFY)
    })

    expect(screen.queryByLabelText('Verification')).toBeNull()
    expect(screen.getByText('/workspace-rescanned')).toBeTruthy()
  })

  it('calls definition_status exactly once even when two "Rescan" clicks are dispatched before a re-render settles', async () => {
    let calls = 0
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        calls += 1
        // The 1st call is the mount scan (resolved right away so the
        // button is enabled); later calls are left pending -- this test
        // only cares how many are dispatched, never how they resolve.
        if (calls === 1) return FIXTURE_STATUS
        return new Promise<DefinitionStatus>(() => {})
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    const rescanButton = screen.getByRole('button', { name: 'Rescan' })
    // Both clicks are dispatched inside one `act` call, so React never gets
    // a chance to re-render between them -- mirrors the existing same-tick
    // the root Verify button test above, this time proving `rescan`'s own ref guard
    // (not just the `disabled` attribute) keeps it to a single dispatch.
    act(() => {
      fireEvent.click(rescanButton)
      fireEvent.click(rescanButton)
    })

    expect(calls).toBe(2)
  })

  it('offers no Verify button while the initial scan is loading, then an enabled one per agent once it settles', async () => {
    let resolveStatus: ((value: DefinitionStatus) => void) | undefined
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        return new Promise((resolve) => {
          resolveStatus = resolve as (value: DefinitionStatus) => void
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)

    // Verify buttons live on the agent rows, so none exists before a report.
    expect(screen.queryByRole('button', { name: 'Verify .' })).toBeNull()

    if (!resolveStatus) throw new Error('definition_status was not called')
    await act(async () => {
      resolveStatus!(FIXTURE_STATUS)
    })

    const verifyButton = screen.getByRole('button', { name: 'Verify .' }) as HTMLButtonElement
    expect(verifyButton.disabled).toBe(false)
  })

  it('refuses a "Verify ." click dispatched in the same tick as "Rescan", before the disabled attribute renders, and still recovers once the rescan settles', async () => {
    let verifyCalls = 0
    let statusCalls = 0
    let resolveRescanStatus: ((value: DefinitionStatus) => void) | undefined
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        statusCalls += 1
        if (statusCalls === 1) return FIXTURE_STATUS
        return new Promise((resolve) => {
          resolveRescanStatus = resolve as (value: DefinitionStatus) => void
        })
      }
      if (cmd === 'definition_verify') {
        verifyCalls += 1
        return FIXTURE_VERIFY
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    const rescanButton = screen.getByRole('button', { name: 'Rescan' })
    const verifyButton = screen.getByRole('button', { name: 'Verify .' })
    // Both clicks are dispatched inside one `act` call, exactly like the
    // existing same-tick the root Verify button double-click test above, so React
    // never gets a chance to commit the root Verify button's own `disabled`
    // attribute (tied to `loading`, which the `rescan` click just set
    // `true`) between them -- the race the bugfix guards against.
    act(() => {
      fireEvent.click(rescanButton)
      fireEvent.click(verifyButton)
    })

    expect(verifyCalls).toBe(0)
    // One call for the mount scan, one for this rescan -- never a third
    // from the raced verify.
    expect(statusCalls).toBe(2)

    if (!resolveRescanStatus) throw new Error('definition_status was not called a second time')
    await act(async () => {
      resolveRescanStatus!(FIXTURE_STATUS_RESCANNED)
    })

    // Before the fix, the raced verify's own generation bump made the
    // rescan's settle handler see itself as superseded, so it returned
    // before ever calling `setLoading(false)` -- "Rescan" stayed disabled
    // and the new report never rendered, with no way to recover short of
    // remounting the section.
    expect((screen.getByRole('button', { name: 'Rescan' }) as HTMLButtonElement).disabled).toBe(false)
    expect(screen.queryByLabelText('Verification')).toBeNull()
    expect(screen.getByText('/workspace-rescanned')).toBeTruthy()
  })

  it('offers no Verify button while the mount-time scan is still pending, and verifies once it settles', async () => {
    let verifyCalls = 0
    let resolveStatus: ((value: DefinitionStatus) => void) | undefined
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        return new Promise((resolve) => {
          resolveStatus = resolve as (value: DefinitionStatus) => void
        })
      }
      if (cmd === 'definition_verify') {
        verifyCalls += 1
        return FIXTURE_VERIFY
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)

    // No report yet, so no agent row and no Verify button to click.
    expect(screen.queryByRole('button', { name: 'Verify .' })).toBeNull()
    expect(verifyCalls).toBe(0)

    if (!resolveStatus) throw new Error('definition_status was not called')
    await act(async () => {
      resolveStatus!(FIXTURE_STATUS)
    })

    const verifyButton = screen.getByRole('button', { name: 'Verify .' }) as HTMLButtonElement
    expect(verifyButton.disabled).toBe(false)
    fireEvent.click(verifyButton)
    await screen.findByLabelText('Verification')
    expect(verifyCalls).toBe(1)
  })

  // React StrictMode (development only; `main.tsx` renders the real app
  // under it) replays a mount effect: setup, cleanup, setup again, before
  // either dispatched `definition_status` call can settle. Both
  // dispatches share this one instance's `generationRef`/`scanningRef`,
  // so the fix has to hold regardless of which of the two settles first.
  it(
    'offers no Verify button and shows no report while a StrictMode-replayed scan is still pending, ' +
      'recovering once both dispatches have settled (first-dispatched settles first)',
    async () => {
      const resolvers: Array<(value: DefinitionStatus) => void> = []
      let verifyCalls = 0
      mockIPC((cmd) => {
        if (cmd === 'definition_status') {
          return new Promise<DefinitionStatus>((resolve) => {
            resolvers.push(resolve)
          })
        }
        if (cmd === 'definition_verify') {
          verifyCalls += 1
          return FIXTURE_VERIFY
        }
        throw new Error(`unexpected command: ${cmd}`)
      })

      render(
        <StrictMode>
          <DefinitionSection />
        </StrictMode>,
      )

      await waitFor(() => {
        expect(resolvers.length).toBe(2)
      })

      // The first of the two replayed dispatches settles first. Before
      // the fix, its own unconditional `setLoading(false)` and
      // `scanningRef` clear made the report usable here, even though the
      // second (authoritative) scan was still pending.
      await act(async () => {
        resolvers[0](FIXTURE_STATUS)
      })

      expect(screen.queryByRole('button', { name: 'Verify .' })).toBeNull()
      expect(verifyCalls).toBe(0)
      expect(screen.queryByText('/workspace')).toBeNull()

      // The second (superseding) dispatch now settles -- this is the one
      // that must win, and nothing should have discarded it.
      await act(async () => {
        resolvers[1](FIXTURE_STATUS)
      })

      await screen.findByText('/workspace')
      const verifyButton = screen.getByRole('button', { name: 'Verify .' }) as HTMLButtonElement
      expect(verifyButton.disabled).toBe(false)
      expect((screen.getByRole('button', { name: 'Rescan' }) as HTMLButtonElement).disabled).toBe(false)

      fireEvent.click(verifyButton)
      await screen.findByLabelText('Verification')
      expect(verifyCalls).toBe(1)
      // Scoped to the report's own root line: the Verification block also
      // prints a path, so an unscoped lookup would not prove the report survived.
      expect(within(screen.getByText('root:', { exact: false })).getByText('/workspace')).toBeTruthy()
    },
  )

  it(
    'renders the report once a StrictMode-replayed scan\'s second dispatch settles first, dropping the stale ' +
      'first dispatch once it later settles too',
    async () => {
      const resolvers: Array<(value: DefinitionStatus) => void> = []
      let verifyCalls = 0
      mockIPC((cmd) => {
        if (cmd === 'definition_status') {
          return new Promise<DefinitionStatus>((resolve) => {
            resolvers.push(resolve)
          })
        }
        if (cmd === 'definition_verify') {
          verifyCalls += 1
          return FIXTURE_VERIFY
        }
        throw new Error(`unexpected command: ${cmd}`)
      })

      render(
        <StrictMode>
          <DefinitionSection />
        </StrictMode>,
      )

      await waitFor(() => {
        expect(resolvers.length).toBe(2)
      })

      // The second dispatch -- the one that actually wins -- settles
      // first this time.
      await act(async () => {
        resolvers[1](FIXTURE_STATUS)
      })

      await screen.findByText('/workspace')
      const verifyButton = screen.getByRole('button', { name: 'Verify .' }) as HTMLButtonElement
      expect(verifyButton.disabled).toBe(false)

      fireEvent.click(verifyButton)
      await screen.findByLabelText('Verification')
      expect(verifyCalls).toBe(1)

      // The stale first dispatch settles last -- it must change nothing:
      // not the report just rendered, and not the verification just
      // shown beside it.
      await act(async () => {
        resolvers[0](FIXTURE_STATUS)
      })

      // Scoped to the report's own root line: the Verification block also
      // prints a path, so an unscoped lookup would not prove the report survived.
      expect(within(screen.getByText('root:', { exact: false })).getByText('/workspace')).toBeTruthy()
      expect(screen.getByLabelText('Verification')).toBeTruthy()
      expect((screen.getByRole('button', { name: 'Rescan' }) as HTMLButtonElement).disabled).toBe(false)
    },
  )

  it('issues exactly one definition_status call on a plain (non-StrictMode) mount', async () => {
    let calls = 0
    mockIPC((cmd) => {
      if (cmd === 'definition_status') {
        calls += 1
        return FIXTURE_STATUS
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    expect(calls).toBe(1)
  })

  it("verifies a nested agent through its own row's button, passing its relative path and naming that target in the result", async () => {
    const verifyArgs: unknown[] = []
    mockIPC((cmd, args) => {
      if (cmd === 'definition_status') return FIXTURE_STATUS_NESTED
      if (cmd === 'definition_verify') {
        verifyArgs.push(args)
        return { ...FIXTURE_VERIFY, agent: '/workspace/sub/agent' }
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    fireEvent.click(screen.getByRole('button', { name: 'Verify sub/agent' }))

    const verification = await screen.findByLabelText('Verification')
    expect(verifyArgs).toEqual([{ agent: 'sub/agent' }])
    expect(within(verification).getByText('sub/agent')).toBeTruthy()
    expect(within(verification).getByText('/workspace/sub/agent')).toBeTruthy()
  })

  it('verifies the root through the "." row, passing "." as the agent', async () => {
    const verifyArgs: unknown[] = []
    mockIPC((cmd, args) => {
      if (cmd === 'definition_status') return FIXTURE_STATUS_NESTED
      if (cmd === 'definition_verify') {
        verifyArgs.push(args)
        return FIXTURE_VERIFY
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    fireEvent.click(screen.getByRole('button', { name: 'Verify .' }))

    const verification = await screen.findByLabelText('Verification')
    expect(verifyArgs).toEqual([{ agent: '.' }])
    expect(within(verification).getByText('/workspace')).toBeTruthy()
  })

  it('disables every Verify button while a verify is pending', async () => {
    let resolveVerify: ((value: DefinitionVerify) => void) | undefined
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return FIXTURE_STATUS_NESTED
      if (cmd === 'definition_verify') {
        return new Promise((resolve) => {
          resolveVerify = resolve as (value: DefinitionVerify) => void
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')
    const rootButton = screen.getByRole('button', { name: 'Verify .' }) as HTMLButtonElement
    const nestedButton = screen.getByRole('button', { name: 'Verify sub/agent' }) as HTMLButtonElement
    expect(rootButton.disabled).toBe(false)
    expect(nestedButton.disabled).toBe(false)

    fireEvent.click(nestedButton)

    await waitFor(() => {
      expect(rootButton.disabled).toBe(true)
      expect(nestedButton.disabled).toBe(true)
    })
    if (!resolveVerify) throw new Error('definition_verify was not called')
    await act(async () => {
      resolveVerify!(FIXTURE_VERIFY)
    })
    await waitFor(() => {
      expect(rootButton.disabled).toBe(false)
      expect(nestedButton.disabled).toBe(false)
    })
  })

  it("renders the code and message of the shell's invalid-request rejection for a verify", async () => {
    const error: ShellError = { code: 'invalid-request', message: 'the agent is not a workspace directory' }
    mockIPC((cmd) => {
      if (cmd === 'definition_status') return FIXTURE_STATUS_NESTED
      if (cmd === 'definition_verify') return Promise.reject(error)
      throw new Error(`unexpected command: ${cmd}`)
    })

    render(<DefinitionSection />)
    await screen.findByText('/workspace')

    fireEvent.click(screen.getByRole('button', { name: 'Verify sub/agent' }))

    const banner = await screen.findByTestId('definition-verify-error')
    expect(within(banner).getByText('invalid-request')).toBeTruthy()
    expect(within(banner).getByText(/the agent is not a workspace directory/)).toBeTruthy()
    expect(screen.queryByLabelText('Verification')).toBeNull()
  })
})
