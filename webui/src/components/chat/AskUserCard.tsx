import { useMemo, useState, type ReactNode } from 'react'
import { Check, Info, ListChecks, MessageSquare, Sliders, Sparkles } from 'lucide-react'
import { FeedbackDialog } from '@/components/assistant-ui/elements/feedback-dialog'
import { OptionList } from '@/components/assistant-ui/elements/option-list'
import { QuestionFlow } from '@/components/assistant-ui/elements/question-flow'
import { RecommendationCard } from '@/components/assistant-ui/elements/recommendation-card'
import { field as fieldSurface, inkButton, mono } from '@/components/assistant-ui/elements/surfaces'
import { cn } from '@/lib/utils'
import type {
  AskUserKind,
  AskUserResolution,
  ClarificationDecisionNote,
  ClarificationRecord,
} from '@/lib/types'

const CONFIDENCE_LEVEL: Record<string, 1 | 2 | 3> = {
  low: 1,
  tentative: 1,
  medium: 2,
  moderate: 2,
  high: 3,
  strong: 3,
  recommended: 3,
}

const KIND_HEADER: Record<AskUserKind, { label: string; icon: typeof Sliders }> = {
  option_list: { label: 'Choice Needed', icon: Sliders },
  question_flow: { label: 'Guided Questions', icon: ListChecks },
  recommendation: { label: 'Recommendation', icon: Sparkles },
  data_input: { label: 'Information Needed', icon: Info },
  feedback: { label: 'Feedback', icon: MessageSquare },
}

const DEFAULT_FEEDBACK_REASONS: readonly string[] = [
  'Looks great',
  'Needs adjustments',
  'Missing details',
  'Different approach',
]

function isMultiSelection(mode?: string): boolean {
  return mode === 'multi' || mode === 'multiple'
}

function resolveConfidenceLevel(raw?: string): 1 | 2 | 3 {
  const key = (raw || '').trim().toLowerCase()
  return CONFIDENCE_LEVEL[key] ?? 3
}

function OptionListPrompt({
  item,
  onResolve,
}: {
  item: ClarificationRecord
  onResolve: (id: string, resolution: AskUserResolution) => void | Promise<void>
}) {
  const [custom, setCustom] = useState('')
  const selectionMode = isMultiSelection(item.selection_mode) ? 'multiple' : 'single'
  const header = KIND_HEADER.option_list
  const HeaderIcon = header.icon
  const recommendedIds = useMemo(
    () => item.options.filter((o) => o.recommended).map((o) => o.id),
    [item.options],
  )

  return (
    <div
      data-slot="ask-user-card"
      data-kind="option_list"
      className="flex w-full flex-col gap-2.5 border-l border-[var(--accent)]/55 pl-3.5 py-0.5"
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-wider text-[var(--muted)]">
          <HeaderIcon className="size-3.5 text-[var(--accent)]" />
          <span>{item.title || header.label}</span>
        </div>
        {item.recommended_option_id ? (
          <span className={cn(mono, 'text-[var(--muted)]')}>
            rec: {item.recommended_option_id}
          </span>
        ) : null}
      </div>

      <div className="flex flex-col gap-1">
        <p className="text-[13.5px] font-medium leading-snug text-[var(--fg)]">{item.question}</p>
        {item.context ? (
          <p className="text-[12px] leading-relaxed text-[var(--muted)]">{item.context}</p>
        ) : null}
      </div>

      <OptionList
        options={item.options}
        selectionMode={selectionMode}
        defaultValue={selectionMode === 'multiple' ? recommendedIds : undefined}
        confirmLabel="Confirm selection"
        className="max-w-none border-0 bg-transparent p-0"
        onConfirm={(ids) =>
          onResolve(item.id, {
            selected_option_id: ids[0] || '',
            selected_option_ids: ids,
            custom_answer: custom.trim() || null,
          })
        }
      />

      <div className="flex items-center gap-2">
        <input
          value={custom}
          onChange={(e) => setCustom(e.target.value)}
          placeholder="Optional note or extra constraint..."
          className={cn(
            fieldSurface,
            'w-full rounded-xl px-3 py-1.5 text-xs text-[var(--fg)] placeholder:text-[var(--muted)] outline-none focus-visible:ring-1 focus-visible:ring-[var(--accent)]/45',
          )}
        />
        {item.allow_custom && custom.trim() ? (
          <button
            type="button"
            onClick={() =>
              onResolve(item.id, {
                selected_option_id: 'custom',
                custom_answer: custom.trim(),
              })
            }
            className={cn(
              inkButton,
              'flex h-8 shrink-0 cursor-pointer items-center rounded-full px-3 text-xs font-medium',
            )}
          >
            Send custom
          </button>
        ) : null}
      </div>
    </div>
  )
}

function QuestionFlowPrompt({
  item,
  onResolve,
}: {
  item: ClarificationRecord
  onResolve: (id: string, resolution: AskUserResolution) => void | Promise<void>
}) {
  const steps = item.steps || []
  const header = KIND_HEADER.question_flow
  const HeaderIcon = header.icon
  const defaultValue = useMemo(() => {
    const defaults: Record<string, string[]> = {}
    for (const s of steps) {
      const rec = s.options.find((o) => o.recommended)
      if (rec) defaults[s.id] = [rec.id]
    }
    return defaults
  }, [steps])

  return (
    <div
      data-slot="ask-user-card"
      data-kind="question_flow"
      className="flex w-full flex-col gap-2 border-l border-[var(--accent)]/55 pl-3.5 py-0.5"
    >
      <div className="flex items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-wider text-[var(--muted)]">
        <HeaderIcon className="size-3.5 text-[var(--accent)]" />
        <span>{item.title || item.question || header.label}</span>
      </div>
      <QuestionFlow
        steps={steps.map((s) => ({
          id: s.id,
          question: s.question,
          description: s.description,
          options: s.options,
          selectionMode: isMultiSelection(s.selection_mode) ? 'multiple' : 'single',
          minSelections: s.min_selections ?? undefined,
          maxSelections: s.max_selections ?? undefined,
        }))}
        defaultValue={defaultValue}
        completeLabel="Submit answers"
        className="max-w-none"
        onComplete={(stepAnswers) =>
          onResolve(item.id, {
            selected_option_id: 'flow_completed',
            step_answers: stepAnswers,
          })
        }
      />
    </div>
  )
}

function RecommendationPrompt({
  item,
  onResolve,
}: {
  item: ClarificationRecord
  onResolve: (id: string, resolution: AskUserResolution) => void | Promise<void>
}) {
  const [showAlternatives, setShowAlternatives] = useState(false)
  const [accepted, setAccepted] = useState(false)
  const [custom, setCustom] = useState('')
  const rec = item.recommendation
  const primaryAcceptId =
    item.options.find((o) => o.recommended)?.id || item.options[0]?.id || 'accept'
  const hasAlternatives =
    item.options.length > 1 || Boolean(item.allow_custom) || Boolean(rec?.alternatives_label)

  return (
    <div
      data-slot="ask-user-card"
      data-kind="recommendation"
      className="flex w-full flex-col gap-2.5 border-l border-[var(--accent)]/55 pl-3.5 py-0.5"
    >
      <RecommendationCard
        state={accepted ? 'accepted' : 'idle'}
        question={rec?.title || item.title || item.question}
        confidenceLabel={rec?.confidence || 'recommended'}
        confidenceLevel={resolveConfidenceLevel(rec?.confidence)}
        acceptedLabel="Recommendation accepted"
        acceptLabel={rec?.accept_label || 'Accept'}
        alternativesLabel={rec?.alternatives_label || 'Alternatives'}
        className="max-w-none"
        onAccept={() => {
          setAccepted(true)
          void onResolve(item.id, {
            selected_option_id: primaryAcceptId,
            selected_option_ids: [primaryAcceptId],
          })
        }}
        onAlternatives={hasAlternatives ? () => setShowAlternatives((v) => !v) : undefined}
      >
        <div className="flex flex-col gap-2">
          {rec?.summary || item.context ? (
            <p className="text-[13px] leading-relaxed text-[var(--muted)]">
              {rec?.summary || item.context}
            </p>
          ) : null}
          {rec?.rationale ? (
            <p className="text-[12px] leading-relaxed text-[var(--fg)]/80">{rec.rationale}</p>
          ) : null}
          {((rec?.pros && rec.pros.length > 0) || (rec?.cons && rec.cons.length > 0)) ? (
            <div className="grid gap-2 pt-1 sm:grid-cols-2">
              {rec?.pros && rec.pros.length > 0 ? (
                <div className={cn(fieldSurface, 'rounded-xl p-2.5')}>
                  <div className={cn(mono, 'mb-1 uppercase text-[var(--accent)]')}>Pros</div>
                  <ul className="flex flex-col gap-1 text-[11.5px] text-[var(--fg)]/85">
                    {rec.pros.map((p) => (
                      <li key={p}>+ {p}</li>
                    ))}
                  </ul>
                </div>
              ) : null}
              {rec?.cons && rec.cons.length > 0 ? (
                <div className={cn(fieldSurface, 'rounded-xl p-2.5')}>
                  <div className={cn(mono, 'mb-1 uppercase text-[var(--muted)]')}>Tradeoffs</div>
                  <ul className="flex flex-col gap-1 text-[11.5px] text-[var(--muted)]">
                    {rec.cons.map((c) => (
                      <li key={c}>- {c}</li>
                    ))}
                  </ul>
                </div>
              ) : null}
            </div>
          ) : null}
        </div>
      </RecommendationCard>

      {showAlternatives && !accepted ? (
        <div className="mt-1 flex flex-col gap-2 border-l border-[var(--border)] pl-3">
          {item.options.length > 0 ? (
            <OptionList
              options={item.options}
              selectionMode="single"
              className="max-w-none border-0 bg-transparent p-0"
              onConfirm={(ids) =>
                onResolve(item.id, {
                  selected_option_id: ids[0] || '',
                  selected_option_ids: ids,
                  custom_answer: custom.trim() || null,
                })
              }
            />
          ) : null}
          <div className="flex items-center gap-2">
            <input
              value={custom}
              onChange={(e) => setCustom(e.target.value)}
              placeholder="Or specify a different preference..."
              className={cn(
                fieldSurface,
                'w-full rounded-xl px-3 py-1.5 text-xs text-[var(--fg)] placeholder:text-[var(--muted)] outline-none focus-visible:ring-1 focus-visible:ring-[var(--accent)]/45',
              )}
            />
            <button
              type="button"
              disabled={!custom.trim()}
              onClick={() =>
                onResolve(item.id, {
                  selected_option_id: 'custom',
                  custom_answer: custom.trim(),
                })
              }
              className={cn(
                inkButton,
                'flex h-8 shrink-0 cursor-pointer items-center rounded-full px-3 text-xs font-medium disabled:opacity-40',
              )}
            >
              Send
            </button>
          </div>
        </div>
      ) : null}
    </div>
  )
}

function DataInputPrompt({
  item,
  onResolve,
}: {
  item: ClarificationRecord
  onResolve: (id: string, resolution: AskUserResolution) => void | Promise<void>
}) {
  const fields = item.fields || []
  const [values, setValues] = useState<Record<string, string>>(() => {
    const init: Record<string, string> = {}
    for (const f of fields) {
      init[f.id] = f.default_value || ''
    }
    return init
  })
  const [freeform, setFreeform] = useState('')
  const [submitting, setSubmitting] = useState(false)
  const header = KIND_HEADER.data_input
  const HeaderIcon = header.icon

  const missingRequired = fields.some(
    (f) => f.required !== false && !(values[f.id] || '').trim(),
  )
  const canSubmit = fields.length > 0 ? !missingRequired : Boolean(freeform.trim())

  const handleSubmit = async () => {
    if (!canSubmit || submitting) return
    setSubmitting(true)
    try {
      const cleanValues: Record<string, string> = {}
      for (const f of fields) {
        const v = (values[f.id] || '').trim()
        if (v) cleanValues[f.id] = v
      }
      await onResolve(item.id, {
        selected_option_id: 'data_submitted',
        field_values: cleanValues,
        custom_answer: freeform.trim() || null,
      })
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <div
      data-slot="ask-user-card"
      data-kind="data_input"
      className="flex w-full flex-col gap-2.5 border-l border-[var(--accent)]/55 pl-3.5 py-0.5"
    >
      <div className="flex items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-wider text-[var(--muted)]">
        <HeaderIcon className="size-3.5 text-[var(--accent)]" />
        <span>{item.title || header.label}</span>
      </div>

      <div className="flex flex-col gap-1">
        <p className="text-[13.5px] font-medium leading-snug text-[var(--fg)]">{item.question}</p>
        {item.context ? (
          <p className="text-[12px] leading-relaxed text-[var(--muted)]">{item.context}</p>
        ) : null}
      </div>

      {fields.length > 0 ? (
        <div className="flex flex-col gap-2.5">
          {fields.map((f) => (
            <label key={f.id} className="flex flex-col gap-1">
              <span className="flex items-center gap-1.5 text-[12px] font-medium text-[var(--fg)]/90">
                <span>{f.label}</span>
                {f.required !== false ? (
                  <span className={cn(mono, 'text-[var(--accent)]')}>required</span>
                ) : (
                  <span className={cn(mono, 'text-[var(--muted)]')}>optional</span>
                )}
              </span>
              {f.multiline ? (
                <textarea
                  rows={3}
                  value={values[f.id] || ''}
                  placeholder={f.placeholder || ''}
                  onChange={(e) =>
                    setValues((prev) => ({ ...prev, [f.id]: e.target.value }))
                  }
                  className={cn(
                    fieldSurface,
                    'resize-y rounded-xl px-3 py-2 text-xs text-[var(--fg)] placeholder:text-[var(--muted)] outline-none focus-visible:ring-1 focus-visible:ring-[var(--accent)]/45',
                  )}
                />
              ) : (
                <input
                  type={f.secret ? 'password' : 'text'}
                  value={values[f.id] || ''}
                  placeholder={f.placeholder || ''}
                  onChange={(e) =>
                    setValues((prev) => ({ ...prev, [f.id]: e.target.value }))
                  }
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && canSubmit) void handleSubmit()
                  }}
                  className={cn(
                    fieldSurface,
                    'rounded-xl px-3 py-2 text-xs text-[var(--fg)] placeholder:text-[var(--muted)] outline-none focus-visible:ring-1 focus-visible:ring-[var(--accent)]/45',
                  )}
                />
              )}
            </label>
          ))}
        </div>
      ) : (
        <textarea
          rows={3}
          value={freeform}
          onChange={(e) => setFreeform(e.target.value)}
          placeholder="Type the requested information..."
          className={cn(
            fieldSurface,
            'resize-y rounded-xl px-3 py-2 text-xs text-[var(--fg)] placeholder:text-[var(--muted)] outline-none focus-visible:ring-1 focus-visible:ring-[var(--accent)]/45',
          )}
        />
      )}

      <div className="flex items-center justify-end gap-2">
        <button
          type="button"
          disabled={!canSubmit || submitting}
          onClick={() => void handleSubmit()}
          className={cn(
            inkButton,
            'flex h-8 cursor-pointer items-center rounded-full px-4 text-xs font-medium disabled:cursor-default disabled:opacity-40',
          )}
        >
          {submitting ? 'Submitting...' : 'Submit'}
        </button>
      </div>
    </div>
  )
}

function FeedbackPrompt({
  item,
  onResolve,
}: {
  item: ClarificationRecord
  onResolve: (id: string, resolution: AskUserResolution) => void | Promise<void>
}) {
  const reasons = useMemo(
    () =>
      item.options.length > 0
        ? item.options.map((o) => o.label)
        : DEFAULT_FEEDBACK_REASONS,
    [item.options],
  )
  const [selected, setSelected] = useState<string[]>([])
  const [note, setNote] = useState('')
  const [sent, setSent] = useState(false)

  const toggleReason = (reason: string) => {
    setSelected((cur) =>
      cur.includes(reason) ? cur.filter((r) => r !== reason) : [...cur, reason],
    )
  }

  return (
    <div
      data-slot="ask-user-card"
      data-kind="feedback"
      className="w-full border-l border-[var(--accent)]/55 pl-3.5 py-0.5"
    >
      <FeedbackDialog
        title={item.title || item.question}
        subtitle={item.context ? 'feedback' : 'optional'}
        placeholder={item.context || 'Add details or adjustments...'}
        submitLabel="Send feedback"
        sentMessage="Feedback shared with Zenko."
        reasons={reasons}
        selected={selected}
        note={note}
        sent={sent}
        className="max-w-none"
        onToggleReason={toggleReason}
        onNoteChange={setNote}
        onSubmit={() => {
          setSent(true)
          const chosenIds = item.options
            .filter((o) => selected.includes(o.label))
            .map((o) => o.id)
          void onResolve(item.id, {
            selected_option_id: chosenIds[0] || selected[0] || 'accept',
            selected_option_ids: chosenIds.length > 0 ? chosenIds : selected,
            custom_answer: note.trim() || null,
            rating: selected[0] || null,
          })
        }}
      />
    </div>
  )
}

const PROMPT_RENDERERS: Record<
  AskUserKind,
  (props: {
    item: ClarificationRecord
    onResolve: (id: string, resolution: AskUserResolution) => void | Promise<void>
  }) => ReactNode
> = {
  option_list: (p) => <OptionListPrompt {...p} />,
  question_flow: (p) => <QuestionFlowPrompt {...p} />,
  recommendation: (p) => <RecommendationPrompt {...p} />,
  data_input: (p) => <DataInputPrompt {...p} />,
  feedback: (p) => <FeedbackPrompt {...p} />,
}

/**
 * Interactive Ask-User surface dispatching declaratively across the 5 installed
 * `@assistant-ui` elements (`OptionList`, `QuestionFlow`, `RecommendationCard`,
 * `FeedbackDialog`) plus structured `data_input`.
 */
export function AskUserCard({
  item,
  onResolve,
}: {
  item: ClarificationRecord
  onResolve: (id: string, resolution: AskUserResolution) => void | Promise<void>
}) {
  const kind: AskUserKind = item.kind || 'option_list'
  const render = PROMPT_RENDERERS[kind] || PROMPT_RENDERERS.option_list
  return <>{render({ item, onResolve })}</>
}

/**
 * Compact persistent receipt rendered in the chat transcript after an `ask_user` prompt resolves.
 */
export function AskUserReceipt({ decision }: { decision: ClarificationDecisionNote }) {
  const kind: AskUserKind = decision.kind || 'option_list'

  if (
    kind === 'question_flow' &&
    decision.steps &&
    decision.steps.length > 0 &&
    decision.step_answers &&
    Object.keys(decision.step_answers).length > 0
  ) {
    return (
      <QuestionFlow
        steps={decision.steps.map((s) => ({
          id: s.id,
          question: s.question,
          description: s.description,
          options: s.options,
        }))}
        choice={decision.step_answers}
        className="max-w-none"
      />
    )
  }

  if (
    kind === 'option_list' &&
    decision.options &&
    decision.options.length > 0 &&
    (decision.selected_option_ids?.length || decision.selected_option_id)
  ) {
    const choiceIds =
      decision.selected_option_ids && decision.selected_option_ids.length > 0
        ? decision.selected_option_ids
        : [decision.selected_option_id]
    return (
      <div className="flex flex-col gap-1">
        <div className="flex flex-wrap items-center justify-between gap-2 text-[12px]">
          <span className="font-medium text-[var(--fg)]/90">{decision.question}</span>
          <span className={cn(mono, 'text-[var(--muted)]')}>{decision.resolved_by}</span>
        </div>
        <OptionList
          options={decision.options}
          choice={choiceIds}
          className="max-w-none p-0"
        />
        {decision.custom_answer ? (
          <p className="text-[11.5px] text-[var(--muted)]">{decision.custom_answer}</p>
        ) : null}
      </div>
    )
  }

  if (kind === 'recommendation' && decision.recommendation) {
    return (
      <RecommendationCard
        state="accepted"
        question={decision.recommendation.title || decision.question}
        confidenceLabel={decision.recommendation.confidence || 'recommended'}
        acceptedLabel={`${decision.selected_option_id} · ${decision.resolved_by}`}
        className="max-w-none"
      >
        {decision.recommendation.summary || decision.context || ''}
      </RecommendationCard>
    )
  }

  const fieldEntries = Object.entries(decision.field_values || {})
  return (
    <div className="flex flex-col gap-1 text-[12px]">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-1.5 text-[var(--fg)]/90">
          <Check size={13} style={{ color: 'var(--accent)' }} />
          <span className="font-medium">{decision.title || decision.question}</span>
          {decision.selected_option_id ? (
            <span className={cn(mono, 'text-[var(--accent)]')}>
              {decision.selected_option_ids && decision.selected_option_ids.length > 1
                ? decision.selected_option_ids.join(', ')
                : decision.selected_option_id}
            </span>
          ) : null}
        </div>
        <span className="font-mono text-[10.5px] text-[var(--muted)]">
          {decision.resolved_by}
          {decision.custom_answer ? ` · ${decision.custom_answer}` : ''}
        </span>
      </div>
      {fieldEntries.length > 0 ? (
        <div className="flex flex-wrap gap-x-3 gap-y-0.5 pl-4 font-mono text-[10.5px] text-[var(--muted)]">
          {fieldEntries.map(([k, v]) => (
            <span key={k}>
              {k}: <span className="text-[var(--fg)]/85">{v}</span>
            </span>
          ))}
        </div>
      ) : null}
    </div>
  )
}
