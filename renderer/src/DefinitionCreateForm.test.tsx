// ADR-0005 slice 3, sub-slice 3e: the create interview as a renderer form.
// Mocked at the Tauri IPC boundary with `mockIPC`, like `DefinitionSection.test.tsx`.

import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { DefinitionCreateForm } from './DefinitionCreateForm'
import type {
  DefinitionAnswers,
  DefinitionCreateApply,
  DefinitionCreatePreview,
  DefinitionField,
  ShellError,
} from './ipc/harness'

afterEach(() => {
  clearMocks()
})

const H = (byte: number): string => byte.toString(16).padStart(2, '0').repeat(32)

const FIELDS: DefinitionField[] = [
  { key: 'role', required: true, list: false, prompt: 'What is the role of this agent?' },
  { key: 'tone', required: false, list: false, prompt: 'Which tone should it use?' },
  { key: 'deliverables', required: true, list: true, prompt: 'What does it deliver?' },
  { key: 'open_decisions', required: false, list: true, prompt: 'What is still undecided?' },
]

const PLANNED: DefinitionCreatePreview = {
  schema: 'omnifrons.definition-create-preview/1',
  agent: '.',
  outcome: {
    kind: 'planned',
    planId: H(9),
    workspace: '/ws',
    operations: [
      {
        op: 'write-generated',
        path: 'AGENTS.md',
        templateVersion: 'v1',
        answersDigest: H(6),
        before: { kind: 'absent' },
        after: H(1),
      },
    ],
    notices: [],
    provenance: [
      { line: 'Role: Triage the inbox', said: ["triaje de la bandeja"] },
      { line: 'Deliverables: a daily digest', said: ['un resumen diario', 'cada manana'] },
    ],
  },
}

const APPLIED: DefinitionCreateApply = {
  schema: 'omnifrons.definition-create-apply/1',
  agent: '.',
  outcome: {
    kind: 'applied',
    planId: H(9),
    record: 'rec-9.json',
    files: [{ path: 'AGENTS.md', before: { kind: 'absent' }, after: H(1), snapshot: null }],
  },
}

interface CreateIpc {
  previews: Array<Record<string, unknown>>
  applies: Array<Record<string, unknown>>
}

function mockCreateIpc(
  options: {
    fields?: () => unknown
    preview?: (call: number) => unknown
    apply?: () => unknown
  } = {},
): CreateIpc {
  const ipc: CreateIpc = { previews: [], applies: [] }
  mockIPC((cmd, args) => {
    if (cmd === 'definition_create_fields') return options.fields ? options.fields() : FIELDS
    if (cmd === 'definition_create_preview') {
      ipc.previews.push(args as Record<string, unknown>)
      return options.preview ? options.preview(ipc.previews.length) : PLANNED
    }
    if (cmd === 'definition_create_apply') {
      ipc.applies.push(args as Record<string, unknown>)
      return options.apply ? options.apply() : APPLIED
    }
    throw new Error(`unexpected command: ${cmd}`)
  })
  return ipc
}

function renderForm(props: Partial<Parameters<typeof DefinitionCreateForm>[0]> = {}) {
  const handlers = { onCancel: vi.fn(), onPendingChange: vi.fn(), onFinished: vi.fn(), onApplyFailed: vi.fn() }
  render(
    <DefinitionCreateForm
      withGemini={false}
      blocked={false}
      {...handlers}
      {...props}
    />,
  )
  return handlers
}

const type = (label: string, value: string): void => {
  fireEvent.change(screen.getByLabelText(label), { target: { value } })
}
const click = (name: string): void => {
  fireEvent.click(screen.getByRole('button', { name }))
}

/** Answers the whole catalogue: role (with padding), no tone, two deliverables. */
async function fillInterview(): Promise<void> {
  await screen.findByText('What is the role of this agent?')
  type('What is the role of this agent?', '  Triage the inbox  ')
  click('Next')
  click('Next')
  type('Entry 1', 'a daily digest')
  click('Next')
  click('Preview')
}

const ANSWERS: DefinitionAnswers = {
  role: { text: 'Triage the inbox', said: '  Triage the inbox  ', status: 'confirmed' },
  deliverables: [{ text: 'a daily digest', said: 'a daily digest', status: 'confirmed' }],
} as DefinitionAnswers

describe('DefinitionCreateForm interview', () => {
  it('loads the catalogue and asks the first question with a step counter', async () => {
    mockCreateIpc()
    renderForm()

    expect(screen.getByRole('status').textContent).toBe('loading the questions')
    expect(await screen.findByText('What is the role of this agent?')).toBeTruthy()
    expect(screen.getByText('Question 1 of 4')).toBeTruthy()
  })

  it('shows the error when the catalogue cannot be loaded', async () => {
    const error: ShellError = { code: 'no-workspace', message: 'no active workspace' }
    mockCreateIpc({
      fields: () => {
        throw error
      },
    })
    renderForm()

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toBe('no-workspace: no active workspace')
  })

  it('keeps Next disabled on a required field until something is typed', async () => {
    mockCreateIpc()
    renderForm()
    await screen.findByText('What is the role of this agent?')

    const next = screen.getByRole('button', { name: 'Next' }) as HTMLButtonElement
    expect(next.disabled).toBe(true)
    type('What is the role of this agent?', '   ')
    expect(next.disabled).toBe(true)
    type('What is the role of this agent?', 'Triage')
    expect(next.disabled).toBe(false)
  })

  it('lets an optional field stay empty and keeps typed values across Back and Next', async () => {
    mockCreateIpc()
    renderForm()
    await screen.findByText('What is the role of this agent?')
    type('What is the role of this agent?', 'Triage')
    click('Next')

    expect((screen.getByRole('button', { name: 'Next' }) as HTMLButtonElement).disabled).toBe(false)
    type('Which tone should it use?', 'Warm')
    click('Next')
    click('Back')
    expect((screen.getByLabelText('Which tone should it use?') as HTMLInputElement).value).toBe('Warm')
    click('Back')
    expect((screen.getByLabelText('What is the role of this agent?') as HTMLInputElement).value).toBe('Triage')
    expect((screen.getByRole('button', { name: 'Back' }) as HTMLButtonElement).disabled).toBe(true)
  })

  it('grows and shrinks a list field', async () => {
    mockCreateIpc()
    renderForm()
    await screen.findByText('What is the role of this agent?')
    type('What is the role of this agent?', 'Triage')
    click('Next')
    click('Next')

    type('Entry 1', 'first')
    click('Add another')
    type('Entry 2', 'second')
    expect(screen.getAllByRole('textbox')).toHaveLength(2)
    click('Remove entry 1')

    const remaining = screen.getAllByRole('textbox') as HTMLInputElement[]
    expect(remaining.map((input) => input.value)).toEqual(['second'])
    // The last entry cannot be removed: a list always keeps one input.
    expect((screen.getByRole('button', { name: 'Remove entry 1' }) as HTMLButtonElement).disabled).toBe(true)
  })

  it('previews with the exact request: text trimmed, said verbatim, empty optional fields omitted', async () => {
    const ipc = mockCreateIpc()
    renderForm()

    await fillInterview()

    await screen.findByLabelText('Create preview')
    expect(ipc.previews).toEqual([{ agent: '.', nestedConsent: false, withGemini: false, answers: ANSWERS }])
  })

  it('sends withGemini from the section', async () => {
    const ipc = mockCreateIpc()
    renderForm({ withGemini: true })

    await fillInterview()

    await screen.findByLabelText('Create preview')
    expect(ipc.previews[0].withGemini).toBe(true)
  })
})

describe('DefinitionCreateForm preview', () => {
  it('renders the plan, the operations and the words behind each generated line', async () => {
    mockCreateIpc()
    renderForm()
    await fillInterview()

    const panel = await screen.findByLabelText('Create preview')
    expect(within(panel).getByText(H(9)).tagName).toBe('CODE')
    expect(
      within(within(panel).getByLabelText('Operations'))
        .getAllByRole('listitem')
        .map((item) => item.textContent),
    ).toEqual(['write-generated AGENTS.md (v1); before: absent; after: 010101010101'])
    const rows = within(within(panel).getByLabelText('Provenance')).getAllByRole('listitem')
    expect(rows.map((row) => row.textContent)).toEqual([
      'Role: Triage the inbox you said: triaje de la bandeja',
      'Deliverables: a daily digest you said: un resumen diario | cada manana',
    ])
    expect(within(panel).getByRole('button', { name: 'Apply' })).toBeTruthy()
  })

  it('shows a refusal without an Apply button', async () => {
    mockCreateIpc({
      preview: () => ({ ...PLANNED, outcome: { kind: 'refused', code: 'conflict', message: 'AGENTS.md exists' } }),
    })
    renderForm()
    await fillInterview()

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toBe('conflict: AGENTS.md exists')
    expect(screen.queryByRole('button', { name: 'Apply' })).toBeNull()
    expect(screen.queryByLabelText('Create preview')).toBeNull()
  })

  it("jumps to the first problem's step and shows each message under its field", async () => {
    mockCreateIpc({
      preview: () => ({
        ...PLANNED,
        outcome: {
          kind: 'invalid',
          problems: [
            { field: 'deliverables', index: 0, part: 'text', reason: 'too-long', message: 'that entry is too long' },
            { field: 'role', index: null, part: 'text', reason: 'empty', message: 'the role is empty' },
          ],
        },
      }),
    })
    renderForm()
    await fillInterview()

    expect(await screen.findByText('Question 3 of 4')).toBeTruthy()
    expect(screen.getByText('that entry is too long')).toBeTruthy()
    expect(screen.queryByLabelText('Create preview')).toBeNull()
    click('Back')
    click('Back')
    expect(screen.getByText('the role is empty')).toBeTruthy()
  })

  it('asks for the nested consent after a nested_agent refusal and re-previews with it', async () => {
    const ipc = mockCreateIpc({
      preview: (call) =>
        call === 1
          ? { ...PLANNED, outcome: { kind: 'refused', code: 'nested_agent', message: 'inside another agent' } }
          : PLANNED,
    })
    renderForm()
    await fillInterview()

    const box = await screen.findByRole('checkbox', { name: 'Create this definition on its own' })
    fireEvent.click(box)
    click('Preview')

    await screen.findByLabelText('Create preview')
    expect(ipc.previews.map((call) => call.nestedConsent)).toEqual([false, true])
  })

  it('ignores a second Preview click while one is in flight', async () => {
    let release: () => void = () => {}
    const gate = new Promise<void>((resolve) => {
      release = resolve
    })
    const ipc = mockCreateIpc({ preview: () => gate.then(() => PLANNED) })
    renderForm()
    await fillInterview()

    const preview = screen.getByRole('button', { name: 'Preview' })
    fireEvent.click(preview)
    fireEvent.click(preview)
    release()

    await screen.findByLabelText('Create preview')
    expect(ipc.previews).toHaveLength(1)
  })

  it('drops the shown plan when an answer changes', async () => {
    mockCreateIpc()
    renderForm()
    await fillInterview()
    await screen.findByLabelText('Create preview')

    click('Back')
    type('Entry 1', 'something else')

    expect(screen.queryByLabelText('Create preview')).toBeNull()
  })
})

describe('DefinitionCreateForm apply', () => {
  async function previewed(options: Parameters<typeof mockCreateIpc>[0] = {}) {
    const ipc = mockCreateIpc(options)
    const handlers = renderForm()
    await fillInterview()
    await screen.findByLabelText('Create preview')
    return { ipc, handlers }
  }

  it('applies the same request with the plan id and hands the result over', async () => {
    const { ipc, handlers } = await previewed()

    click('Apply')

    await waitFor(() => expect(handlers.onFinished).toHaveBeenCalledTimes(1))
    expect(ipc.applies).toEqual([
      { agent: '.', nestedConsent: false, withGemini: false, answers: ANSWERS, planId: H(9) },
    ])
    expect(handlers.onFinished).toHaveBeenCalledWith(APPLIED)
  })

  it('hands a noop over too', async () => {
    const noop: DefinitionCreateApply = { ...APPLIED, outcome: { kind: 'noop', planId: H(9) } }
    const { handlers } = await previewed({ apply: () => noop })

    click('Apply')

    await waitFor(() => expect(handlers.onFinished).toHaveBeenCalledWith(noop))
  })

  it('keeps the form and shows a refused apply', async () => {
    const { handlers } = await previewed({
      apply: () => ({ ...APPLIED, outcome: { kind: 'refused', code: 'conflict', message: 'changed meanwhile' } }),
    })

    click('Apply')

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toBe('conflict: changed meanwhile')
    expect(handlers.onFinished).not.toHaveBeenCalled()
  })

  it('shows the stale hint and asks for a new preview on plan-stale', async () => {
    const stale: ShellError = { code: 'plan-stale', message: 'raw shell wording' }
    await previewed({
      apply: () => {
        throw stale
      },
    })

    click('Apply')

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toBe('the plan no longer matches the definition; preview again')
    expect(screen.queryByRole('button', { name: 'Apply' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Preview' })).toBeTruthy()
  })

  it('shows the code and message of any other error', async () => {
    const failure: ShellError = { code: 'run-active', message: 'a run is active' }
    await previewed({
      apply: () => {
        throw failure
      },
    })

    click('Apply')

    expect((await screen.findByRole('alert')).textContent).toBe('run-active: a run is active')
  })

  it('drops the plan, keeps the answers and asks for one rescan on a non-stale apply error', async () => {
    const failure: ShellError = { code: 'run-active', message: 'a run is active' }
    const { handlers } = await previewed({
      apply: () => {
        throw failure
      },
    })

    click('Apply')

    expect((await screen.findByRole('alert')).textContent).toBe('run-active: a run is active')
    expect(screen.queryByLabelText('Create preview')).toBeNull()
    expect(handlers.onApplyFailed).toHaveBeenCalledTimes(1)
    click('Back')
    click('Back')
    click('Back')
    expect((screen.getByLabelText('What is the role of this agent?') as HTMLInputElement).value).toBe(
      '  Triage the inbox  ',
    )
  })

  it('does not ask for a rescan on plan-stale', async () => {
    const stale: ShellError = { code: 'plan-stale', message: 'raw' }
    const { handlers } = await previewed({
      apply: () => {
        throw stale
      },
    })

    click('Apply')

    await screen.findByRole('alert')
    expect(handlers.onApplyFailed).not.toHaveBeenCalled()
  })

  it('reports pending state and applies once on a double click', async () => {
    let release: () => void = () => {}
    const gate = new Promise<void>((resolve) => {
      release = resolve
    })
    const { ipc, handlers } = await previewed({ apply: () => gate.then(() => APPLIED) })

    const apply = screen.getByRole('button', { name: 'Apply' })
    fireEvent.click(apply)
    fireEvent.click(apply)
    expect(handlers.onPendingChange).toHaveBeenLastCalledWith('apply')
    release()

    await waitFor(() => expect(handlers.onFinished).toHaveBeenCalledTimes(1))
    expect(ipc.applies).toHaveLength(1)
  })

  it('cancels from the form', async () => {
    mockCreateIpc()
    const handlers = renderForm()
    await screen.findByText('What is the role of this agent?')

    click('Cancel')

    expect(handlers.onCancel).toHaveBeenCalledTimes(1)
  })
})
