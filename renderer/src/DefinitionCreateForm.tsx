import { Fragment, useEffect, useRef, useState } from 'react'

import { DefinitionErrorLine, type DefinitionSectionError } from './DefinitionErrorLine'
import { operationText, PLAN_STALE_HINT, toSectionError } from './definitionText'
import {
  definitionCreateApply,
  definitionCreateFields,
  definitionCreatePreview,
  isShellError,
  type DefinitionAnswer,
  type DefinitionAnswerProblem,
  type DefinitionCreateApply,
  type DefinitionCreatePreview,
  type DefinitionCreateRequest,
  type DefinitionField,
} from './ipc/harness'
import { PlainTextLine } from './PlainTextLine'

/**
 * ADR-0005 slice 3, sub-slice 3e: the create interview as a form. One
 * question per step; the answers live only in this component's state (never
 * stored, never logged). A preview shows the user's own words beside each
 * generated line -- those words are never written to the file -- and an
 * apply repeats the request the preview was planned from.
 */

/** What the form has in flight, reported to the section so it can block a rescan or an adapt call. */
export type DefinitionCreatePending = 'preview' | 'apply' | null

export interface DefinitionCreateFormProps {
  /** The section's "Also write GEMINI.md" box. */
  withGemini: boolean
  /** A scan, verify, adapt preview or adapt apply is in flight: disables Preview and Apply. */
  blocked: boolean
  onCancel: () => void
  onPendingChange: (pending: DefinitionCreatePending) => void
  /** An apply that wrote (`applied`) or found nothing to write (`noop`). */
  onFinished: (result: DefinitionCreateApply) => void
  /**
   * An apply failed with something other than a stale plan: it may have
   * written before failing, so the section should re-read the rows. The form
   * stays open with its answers.
   */
  onApplyFailed: () => void
}

/** The field keys whose items carry no status (the answers type has no status for them). */
const STATUSLESS_KEYS: ReadonlySet<string> = new Set(['open_decisions'])

/** A problem placed on the entry the user typed, which can differ from the sent index when blanks were dropped. */
interface PlacedProblem {
  problem: DefinitionAnswerProblem
  /** The index into the typed entries of a list field, or `null` for the field as a whole. */
  entry: number | null
}

interface BuiltAnswers {
  answers: Record<string, unknown>
  /** Per list field: the typed index of each sent item. */
  sentIndices: Record<string, number[]>
  /** True when a required field has no non-blank entry. */
  missingRequired: boolean
}

function buildAnswers(fields: DefinitionField[], values: Record<string, string[]>): BuiltAnswers {
  const answers: Record<string, unknown> = {}
  const sentIndices: Record<string, number[]> = {}
  let missingRequired = false
  for (const field of fields) {
    const typed = values[field.key] ?? ['']
    const kept: Array<{ index: number; said: string }> = []
    typed.forEach((said, index) => {
      if (said.trim() !== '') kept.push({ index, said })
    })
    if (kept.length === 0) {
      if (field.required) missingRequired = true
      continue
    }
    const items = kept.map(({ said }) =>
      STATUSLESS_KEYS.has(field.key)
        ? { text: said.trim(), said }
        : ({ text: said.trim(), said, status: 'confirmed' } satisfies DefinitionAnswer),
    )
    answers[field.key] = field.list ? items : items[0]
    sentIndices[field.key] = kept.map(({ index }) => index)
  }
  return { answers, sentIndices, missingRequired }
}

/** A scalar field has no entries to point at: an index the shell reports for one still belongs to the whole field. */
function placeProblems(
  problems: DefinitionAnswerProblem[],
  sentIndices: Record<string, number[]>,
  fields: DefinitionField[],
): PlacedProblem[] {
  const lists = new Set(fields.filter((field) => field.list).map((field) => field.key))
  return problems.map((problem) => ({
    problem,
    entry:
      problem.index === null || !lists.has(problem.field) ? null : (sentIndices[problem.field]?.[problem.index] ?? null),
  }))
}

function isBlank(entries: string[] | undefined): boolean {
  return (entries ?? ['']).every((entry) => entry.trim() === '')
}

/** A refused outcome or a failed call, as `<code>: <message>`. */
function CodeMessageLine({ code, message }: { code: string; message: string }) {
  return (
    <>
      <strong>
        <PlainTextLine text={code} />
      </strong>
      {': '}
      <PlainTextLine text={message} />
    </>
  )
}

type PlannedPreview = Extract<DefinitionCreatePreview['outcome'], { kind: 'planned' }>

interface CreatePreviewPanelProps {
  outcome: PlannedPreview
  busy: boolean
  onApply: () => void
}

function CreatePreviewPanel({ outcome, busy, onApply }: CreatePreviewPanelProps) {
  return (
    <div aria-label="Create preview">
      <p>
        plan id: <code>{outcome.planId}</code>
      </p>
      <ul aria-label="Operations">
        {outcome.operations.map((operation, index) => (
          <li key={index}>
            <PlainTextLine text={operationText(operation)} />
          </li>
        ))}
      </ul>
      {outcome.notices.length > 0 && (
        <ul aria-label="Notices">
          {outcome.notices.map((notice, index) => (
            <li key={index}>
              <PlainTextLine text={`portable agent folder: ${notice.path}`} />
            </li>
          ))}
        </ul>
      )}
      <ul aria-label="Provenance">
        {outcome.provenance.map((row, index) => (
          <li key={index}>
            <PlainTextLine text={row.line} /> <span>you said:</span>{' '}
            {row.said.map((words, wordsIndex) => (
              <Fragment key={wordsIndex}>
                {wordsIndex > 0 && ' | '}
                <PlainTextLine text={words} />
              </Fragment>
            ))}
          </li>
        ))}
      </ul>
      <button type="button" onClick={onApply} disabled={busy}>
        Apply
      </button>
    </div>
  )
}

function ProblemLines({ problems }: { problems: PlacedProblem[] }) {
  return (
    <>
      {problems.map(({ problem }, index) => (
        <p key={index} role="alert">
          <PlainTextLine text={problem.message} />
        </p>
      ))}
    </>
  )
}

interface FieldStepProps {
  field: DefinitionField
  entries: string[]
  problems: PlacedProblem[]
  onChange: (index: number, value: string) => void
  onAdd: () => void
  onRemove: (index: number) => void
}

function FieldStep({ field, entries, problems, onChange, onAdd, onRemove }: FieldStepProps) {
  const wholeField = problems.filter((placed) => placed.entry === null)
  if (!field.list) {
    const id = `definition-create-${field.key}`
    return (
      <div>
        <label htmlFor={id}>
          <PlainTextLine text={field.prompt} />
        </label>{' '}
        <input id={id} type="text" value={entries[0] ?? ''} onChange={(event) => onChange(0, event.target.value)} />
        <ProblemLines problems={wholeField} />
      </div>
    )
  }
  return (
    <fieldset>
      <legend>
        <PlainTextLine text={field.prompt} />
      </legend>
      {entries.map((entry, index) => (
        <div key={index}>
          <input
            type="text"
            aria-label={`Entry ${index + 1}`}
            value={entry}
            onChange={(event) => onChange(index, event.target.value)}
          />{' '}
          <button
            type="button"
            aria-label={`Remove entry ${index + 1}`}
            onClick={() => onRemove(index)}
            disabled={entries.length === 1}
          >
            Remove
          </button>
          <ProblemLines problems={problems.filter((placed) => placed.entry === index)} />
        </div>
      ))}
      <button type="button" onClick={onAdd}>
        Add another
      </button>
      <ProblemLines problems={wholeField} />
    </fieldset>
  )
}

/** The plan the form is showing: the preview, the request it came from, and where its items sat in the typed lists. */
interface ShownPlan {
  outcome: PlannedPreview
  request: DefinitionCreateRequest
  sentIndices: Record<string, number[]>
}

export function DefinitionCreateForm({
  withGemini,
  blocked,
  onCancel,
  onPendingChange,
  onFinished,
  onApplyFailed,
}: DefinitionCreateFormProps) {
  const [fields, setFields] = useState<DefinitionField[] | null>(null)
  const [loadError, setLoadError] = useState<DefinitionSectionError | null>(null)
  const [values, setValues] = useState<Record<string, string[]>>({})
  const [step, setStep] = useState(0)
  const [nestedConsent, setNestedConsent] = useState(false)
  const [consentOffered, setConsentOffered] = useState(false)
  const [plan, setPlan] = useState<ShownPlan | null>(null)
  const [refusal, setRefusal] = useState<{ code: string; message: string } | null>(null)
  const [problems, setProblems] = useState<PlacedProblem[]>([])
  const [callError, setCallError] = useState<DefinitionSectionError | null>(null)
  const [pending, setPending] = useState<DefinitionCreatePending>(null)

  /**
   * The GEMINI.md box belongs to the request a plan was made from: when it
   * changes, the shown plan and refusal no longer describe what an apply
   * would do, so the user previews again.
   */
  const [seenWithGemini, setSeenWithGemini] = useState(withGemini)
  if (seenWithGemini !== withGemini) {
    setSeenWithGemini(withGemini)
    setPlan(null)
    setRefusal(null)
  }

  const mountedRef = useRef(true)
  /** Read synchronously, so two clicks in one tick start one call. */
  const pendingRef = useRef<DefinitionCreatePending>(null)
  /**
   * Bumped by everything that changes what a plan describes: an answer edit,
   * the GEMINI.md box, the nested consent. A response whose captured value no
   * longer matches was planned from a request the form has moved past.
   */
  const generationRef = useRef(0)

  useEffect(() => {
    generationRef.current += 1
  }, [withGemini])

  useEffect(() => {
    mountedRef.current = true
    let live = true
    definitionCreateFields().then(
      (loaded) => {
        if (live) setFields(loaded)
      },
      (reason: unknown) => {
        if (live) setLoadError(toSectionError(reason))
      },
    )
    return () => {
      live = false
      mountedRef.current = false
    }
  }, [])

  const begin = (kind: Exclude<DefinitionCreatePending, null>) => {
    pendingRef.current = kind
    setPending(kind)
    onPendingChange(kind)
  }
  const end = () => {
    pendingRef.current = null
    if (!mountedRef.current) return
    setPending(null)
    onPendingChange(null)
  }

  const showProblems = (placed: PlacedProblem[], catalogue: DefinitionField[]) => {
    setProblems(placed)
    const first = placed.length > 0 ? catalogue.findIndex((field) => field.key === placed[0].problem.field) : -1
    if (first >= 0) setStep(first)
  }

  /** An edit invalidates the shown plan, the refusal and the problems of the field it touched. */
  const edited = (key: string, next: string[]) => {
    generationRef.current += 1
    setValues((current) => ({ ...current, [key]: next }))
    setPlan(null)
    setRefusal(null)
    setCallError(null)
    setProblems((current) => current.filter((placed) => placed.problem.field !== key))
  }

  const runPreview = () => {
    if (pendingRef.current !== null || blocked || fields === null) return
    const built = buildAnswers(fields, values)
    if (built.missingRequired) return
    const request: DefinitionCreateRequest = {
      agent: '.',
      nestedConsent,
      withGemini,
      answers: built.answers as unknown as DefinitionCreateRequest['answers'],
    }
    const generation = generationRef.current
    begin('preview')
    setPlan(null)
    setRefusal(null)
    setProblems([])
    setCallError(null)
    definitionCreatePreview(request).then(
      (result) => {
        end()
        if (!mountedRef.current || generation !== generationRef.current) return
        const { outcome } = result
        if (outcome.kind === 'planned') {
          setPlan({ outcome, request, sentIndices: built.sentIndices })
        } else if (outcome.kind === 'refused') {
          setRefusal({ code: outcome.code, message: outcome.message })
          if (outcome.code === 'nested_agent') setConsentOffered(true)
        } else {
          showProblems(placeProblems(outcome.problems, built.sentIndices, fields), fields)
        }
      },
      (reason: unknown) => {
        end()
        if (mountedRef.current && generation === generationRef.current) setCallError(toSectionError(reason))
      },
    )
  }

  const runApply = () => {
    if (pendingRef.current !== null || blocked || plan === null || fields === null) return
    const { outcome, request, sentIndices } = plan
    const generation = generationRef.current
    begin('apply')
    setRefusal(null)
    setCallError(null)
    definitionCreateApply(request, outcome.planId).then(
      (result) => {
        end()
        if (!mountedRef.current) return
        const applied = result.outcome
        // A write that happened is reported even when an edit overtook the
        // request: hiding it would leave the rows and the result stale.
        if (applied.kind === 'applied' || applied.kind === 'noop') {
          onFinished(result)
        } else if (generation !== generationRef.current) {
          return
        } else if (applied.kind === 'refused') {
          setRefusal({ code: applied.code, message: applied.message })
        } else {
          setPlan(null)
          showProblems(placeProblems(applied.problems, sentIndices, fields), fields)
        }
      },
      (reason: unknown) => {
        end()
        if (!mountedRef.current) return
        const overtaken = generation !== generationRef.current
        // A stale plan could only fail again, and any other failure may have
        // happened after a first write: either way the plan is dropped. Only
        // the latter also asks the section to re-read the rows.
        if (!overtaken) {
          setPlan(null)
          setCallError(toSectionError(reason))
        }
        if (!(isShellError(reason) && reason.code === 'plan-stale')) onApplyFailed()
      },
    )
  }

  const cancel = (
    <button type="button" onClick={onCancel} disabled={pending === 'apply'}>
      Cancel
    </button>
  )

  if (loadError !== null) {
    return (
      <section aria-label="Create definition">
        <div role="alert">
          <DefinitionErrorLine error={loadError} />
        </div>
        {cancel}
      </section>
    )
  }
  if (fields === null) {
    return (
      <section aria-label="Create definition">
        <p role="status">loading the questions</p>
        {cancel}
      </section>
    )
  }
  if (fields.length === 0) {
    return (
      <section aria-label="Create definition">
        <p>the interview has no questions</p>
        {cancel}
      </section>
    )
  }

  const field = fields[step]
  const entries = values[field.key] ?? ['']
  const last = step === fields.length - 1
  const incomplete = fields.some((candidate) => candidate.required && isBlank(values[candidate.key]))
  const strayProblems = problems.filter((placed) => !fields.some((candidate) => candidate.key === placed.problem.field))

  return (
    <section aria-label="Create definition">
      <p>
        Question {step + 1} of {fields.length}
      </p>
      <FieldStep
        field={field}
        entries={entries}
        problems={problems.filter((placed) => placed.problem.field === field.key)}
        onChange={(index, value) => edited(field.key, entries.map((entry, at) => (at === index ? value : entry)))}
        onAdd={() => edited(field.key, [...entries, ''])}
        onRemove={(index) =>
          edited(
            field.key,
            entries.filter((_, at) => at !== index),
          )
        }
      />
      <ProblemLines problems={strayProblems} />
      <p>
        <button type="button" onClick={() => setStep(step - 1)} disabled={step === 0}>
          Back
        </button>{' '}
        {last ? (
          <button type="button" onClick={runPreview} disabled={blocked || pending !== null || incomplete}>
            Preview
          </button>
        ) : (
          <button
            type="button"
            onClick={() => setStep(step + 1)}
            disabled={field.required && isBlank(values[field.key])}
          >
            Next
          </button>
        )}{' '}
        {cancel}
      </p>
      {consentOffered && (
        <label>
          <input
            type="checkbox"
            checked={nestedConsent}
            onChange={(event) => {
              generationRef.current += 1
              setNestedConsent(event.target.checked)
              setPlan(null)
              setRefusal(null)
            }}
          />{' '}
          Create this definition on its own
        </label>
      )}
      {refusal !== null && (
        <div role="alert">
          <CodeMessageLine code={refusal.code} message={refusal.message} />
        </div>
      )}
      {callError !== null && (
        <div role="alert">
          {callError !== 'unexpected' && callError.code === 'plan-stale' ? (
            PLAN_STALE_HINT
          ) : (
            <DefinitionErrorLine error={callError} />
          )}
        </div>
      )}
      {plan !== null && (
        <CreatePreviewPanel
          outcome={plan.outcome}
          busy={blocked || pending !== null} onApply={runApply} />
      )}
    </section>
  )
}
