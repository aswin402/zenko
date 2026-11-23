'use client'

/**
 * KokonutUI — AI Tool Approval (Don't-ask-again SVG check + Hold-to-Allow for destructive calls + TraceLine)
 * Adapted strictly to Zenko 60/30/10 CSS tokens with zero regex.
 */

import { ChevronRight, FilePen, Globe, Send, SquareTerminal, Wrench, type LucideIcon } from 'lucide-react'
import { AnimatePresence, MotionConfig, motion } from 'motion/react'
import { type ReactNode, useEffect, useId, useRef, useState } from 'react'
import { cn } from '@/lib/utils'

export type KokonutApprovalState =
  | 'approval-requested'
  | 'approval-responded'
  | 'output-available'
  | 'output-denied'
  | 'output-error'

export interface KokonutApprovalCall {
  id: string
  tool: string
  title: string
  input: string | Record<string, string | number | boolean>
  reason?: string
  destructive?: boolean
  state: KokonutApprovalState
  output?: string[]
  icon?: ReactNode
}

export interface KokonutAIToolApprovalProps {
  calls: KokonutApprovalCall[]
  onRespond?: (id: string, approved: boolean, always: boolean) => void
  className?: string
}

const EASE_OUT = [0.23, 1, 0.32, 1] as const
const HOLD_MS = 900

const TOOL_CATEGORIES: ReadonlyArray<{
  keywords: readonly string[]
  icon: LucideIcon
  verbs: readonly [string, string]
}> = [
  { keywords: ['bash', 'shell', 'terminal', 'exec', 'command'], icon: SquareTerminal, verbs: ['Running', 'Ran'] },
  { keywords: ['write', 'edit', 'file', 'patch'], icon: FilePen, verbs: ['Editing', 'Edited'] },
  { keywords: ['fetch', 'web', 'browse', 'http', 'search'], icon: Globe, verbs: ['Fetching', 'Fetched'] },
  { keywords: ['mail', 'send', 'message', 'post'], icon: Send, verbs: ['Sending', 'Sent'] },
]

function resolveCategory(tool: string) {
  const lower = tool.toLowerCase()
  return TOOL_CATEGORIES.find((cat) => cat.keywords.some((kw) => lower.includes(kw)))
}

function ToolInput({ input }: { input: KokonutApprovalCall['input'] }) {
  if (typeof input === 'string') {
    return (
      <pre className="zenko-thin-scroll overflow-x-auto whitespace-pre-wrap break-all rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 font-mono text-[12px] leading-relaxed text-[var(--fg)]">
        <span aria-hidden="true" className="select-none text-[var(--muted)]">
          ${' '}
        </span>
        {input}
      </pre>
    )
  }
  return (
    <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 text-[12px]">
      {Object.entries(input).map(([key, value]) => (
        <div className="contents" key={key}>
          <dt className="text-[var(--muted)]">{key}</dt>
          <dd className="min-w-0 truncate font-mono text-[var(--fg)]">{String(value)}</dd>
        </div>
      ))}
    </dl>
  )
}

function HoldToAllow({ onConfirm }: { onConfirm: () => void }) {
  const [holding, setHolding] = useState(false)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const start = () => {
    if (timer.current) clearTimeout(timer.current)
    setHolding(true)
    timer.current = setTimeout(() => {
      setHolding(false)
      onConfirm()
    }, HOLD_MS)
  }
  const stop = () => {
    setHolding(false)
    if (timer.current) clearTimeout(timer.current)
  }
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current)
    },
    [],
  )

  return (
    <button
      aria-label="Allow destructive action. Press and hold to confirm"
      className="relative h-8 cursor-pointer select-none overflow-hidden rounded-full border px-3.5 text-xs font-medium outline-none focus-visible:ring-2 focus-visible:ring-[var(--destructive)]"
      style={{
        borderColor: 'color-mix(in srgb, var(--destructive) 35%, transparent)',
        backgroundColor: 'color-mix(in srgb, var(--destructive) 12%, transparent)',
        color: 'var(--destructive)',
      }}
      onBlur={stop}
      onContextMenu={(e) => e.preventDefault()}
      onKeyDown={(e) => {
        if ((e.key === ' ' || e.key === 'Enter') && !e.repeat) {
          e.preventDefault()
          start()
        }
      }}
      onKeyUp={(e) => {
        if (e.key === ' ' || e.key === 'Enter') stop()
      }}
      onPointerCancel={stop}
      onPointerDown={(e) => {
        if (e.button === 0) start()
      }}
      onPointerLeave={stop}
      onPointerUp={stop}
      type="button"
    >
      Hold to allow
      <span
        aria-hidden="true"
        className="absolute inset-0 flex items-center justify-center bg-[var(--destructive)] text-white"
        style={{
          clipPath: holding ? 'inset(0 0 0 0)' : 'inset(0 100% 0 0)',
          transition: holding
            ? `clip-path ${HOLD_MS}ms linear`
            : 'clip-path 200ms cubic-bezier(0.23, 1, 0.32, 1)',
        }}
      >
        Hold to allow
      </span>
    </button>
  )
}

function ApprovalCard({
  call,
  onRespond,
}: {
  call: KokonutApprovalCall
  onRespond: (approved: boolean, always: boolean) => void
}) {
  const [always, setAlways] = useState(false)
  const alwaysId = useId()
  const titleId = useId()
  const cat = resolveCategory(call.tool)
  const Icon = cat?.icon ?? Wrench

  return (
    <section
      aria-labelledby={titleId}
      className="rounded-[var(--radius)] border border-[var(--border)] bg-[var(--card)] p-3.5 outline-none"
      onKeyDown={(e) => {
        if (e.key === 'Escape') onRespond(false, false)
      }}
      tabIndex={-1}
    >
      <header className="mb-2.5 flex items-center gap-2">
        <span
          className="flex size-4 shrink-0 items-center justify-center"
          style={{ color: call.destructive ? 'var(--destructive)' : 'var(--accent)' }}
        >
          {call.icon ?? <Icon aria-hidden="true" className="size-4" />}
        </span>
        <h2 className="text-[13.5px] font-medium text-[var(--fg)]" id={titleId}>
          {call.title}
        </h2>
        <span className="font-mono text-[11px] text-[var(--muted)]">{call.tool}</span>
        {call.destructive && (
          <span
            className="ml-auto rounded-full px-2 py-0.5 text-[10.5px] font-medium"
            style={{
              color: 'var(--destructive)',
              backgroundColor: 'color-mix(in srgb, var(--destructive) 12%, transparent)',
            }}
          >
            Destructive
          </span>
        )}
      </header>

      <ToolInput input={call.input} />

      {call.reason && (
        <p className="mt-2 text-xs leading-relaxed text-[var(--muted)]">{call.reason}</p>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-2">
        {call.destructive ? (
          <span className="text-xs text-[var(--muted)]">Asks every time</span>
        ) : (
          <label
            className="flex cursor-pointer select-none items-center gap-2 text-xs text-[var(--muted)]"
            htmlFor={alwaysId}
          >
            <input
              checked={always}
              className="peer sr-only"
              id={alwaysId}
              onChange={(e) => setAlways(e.target.checked)}
              type="checkbox"
            />
            <span
              aria-hidden="true"
              className="flex size-4 items-center justify-center rounded-[5px] border border-[var(--border)] bg-[var(--bg)] transition-colors peer-checked:border-[var(--primary)] peer-checked:bg-[var(--primary)]"
            >
              <svg aria-hidden="true" className="size-3 text-[var(--primary-fg)]" fill="none" viewBox="0 0 12 12">
                <path
                  d="M2.5 6.2 5 8.5l4.5-5"
                  pathLength={1}
                  stroke="currentColor"
                  strokeDasharray={1}
                  strokeDashoffset={always ? 0 : 1}
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.7}
                  style={{ transition: `stroke-dashoffset ${always ? 200 : 100}ms cubic-bezier(0.23,1,0.32,1)` }}
                />
              </svg>
            </span>
            Don&apos;t ask again for <code className="font-mono text-[var(--fg)]">{call.tool}</code>
          </label>
        )}

        <div className="ml-auto flex items-center gap-1.5">
          <button
            className="h-8 cursor-pointer rounded-full px-3.5 text-xs font-medium text-[var(--muted)] outline-none transition-colors hover:bg-[var(--surface)] hover:text-[var(--fg)]"
            onClick={() => onRespond(false, false)}
            type="button"
          >
            Deny
          </button>
          {call.destructive ? (
            <HoldToAllow onConfirm={() => onRespond(true, false)} />
          ) : (
            <button
              className="h-8 cursor-pointer rounded-full bg-[var(--primary)] px-4 text-xs font-medium text-[var(--primary-fg)] outline-none transition-opacity hover:opacity-90"
              onClick={() => onRespond(true, always)}
              type="button"
            >
              Allow
            </button>
          )}
        </div>
      </div>
    </section>
  )
}

function TraceLine({ call }: { call: KokonutApprovalCall }) {
  const [open, setOpen] = useState(false)
  const outputId = useId()
  const isRunning = call.state === 'approval-responded'
  const hasOutput = Boolean(call.output?.length) && call.state !== 'output-denied'
  const verbs = resolveCategory(call.tool)?.verbs ?? ['Using', 'Used']
  const verb =
    call.state === 'approval-responded'
      ? verbs[0]
      : call.state === 'output-denied'
        ? 'Denied'
        : call.state === 'output-error'
          ? 'Failed'
          : verbs[1]
  const summary =
    typeof call.input === 'string' ? call.input : Object.values(call.input)[0]?.toString() ?? ''

  return (
    <div>
      <button
        aria-controls={hasOutput ? outputId : undefined}
        aria-expanded={hasOutput ? open : undefined}
        className="flex w-full cursor-pointer items-center gap-2 rounded-md py-1.5 text-left text-[12.5px] disabled:cursor-default"
        disabled={!hasOutput || isRunning}
        onClick={() => setOpen((v) => !v)}
        type="button"
      >
        <span
          className="h-1.5 w-1.5 shrink-0 rounded-full"
          style={{
            backgroundColor:
              call.state === 'output-available'
                ? 'var(--success)'
                : call.state === 'output-error'
                  ? 'var(--destructive)'
                  : call.state === 'approval-responded'
                    ? 'var(--accent)'
                    : 'var(--muted)',
          }}
        />
        <span className="shrink-0 text-[var(--muted)]">{verb}</span>
        <code
          className={cn(
            'min-w-0 truncate font-mono text-[12px]',
            call.state === 'output-denied' ? 'text-[var(--muted)] line-through' : 'text-[var(--fg)]',
          )}
        >
          {summary}
        </code>
        {hasOutput && !isRunning && (
          <ChevronRight
            aria-hidden="true"
            className={cn('ml-auto size-3.5 text-[var(--muted)] transition-transform', open && 'rotate-90')}
          />
        )}
      </button>
      <AnimatePresence initial={false}>
        {hasOutput && open && (
          <motion.div
            animate={{ height: 'auto', opacity: 1 }}
            className="overflow-hidden"
            exit={{ height: 0, opacity: 0 }}
            id={outputId}
            initial={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.2, ease: EASE_OUT }}
          >
            <pre className="zenko-thin-scroll mb-1.5 ml-4 overflow-x-auto rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 font-mono text-[11px] text-[var(--muted)]">
              {call.output?.join('\n')}
            </pre>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  )
}

export default function KokonutAIToolApproval({
  calls,
  onRespond,
  className,
}: KokonutAIToolApprovalProps) {
  const pending = calls.filter((c) => c.state === 'approval-requested')
  const decided = calls.filter((c) => c.state !== 'approval-requested')
  const current = pending[0]

  return (
    <MotionConfig reducedMotion="user">
      <div className={cn('w-full', className)}>
        <div className="zenko-aui-paper rounded-[var(--radius-lg)] border border-[var(--border)] bg-[var(--surface)]/80 p-1.5">
          {decided.length > 0 && (
            <div className="px-2.5 pt-1 pb-1.5">
              {decided.map((call) => (
                <TraceLine key={call.id} call={call} />
              ))}
            </div>
          )}
          <AnimatePresence initial={false} mode="popLayout">
            {current && (
              <motion.div
                key={current.id}
                animate={{ opacity: 1, y: 0, filter: 'blur(0px)' }}
                exit={{ opacity: 0, y: -6, filter: 'blur(2px)' }}
                initial={{ opacity: 0, y: 8, filter: 'blur(2px)' }}
                transition={{ duration: 0.22, ease: EASE_OUT }}
              >
                {pending.length > 1 && (
                  <p className="px-2.5 pb-1.5 text-xs tabular-nums text-[var(--muted)]">
                    1 of {pending.length} waiting
                  </p>
                )}
                <ApprovalCard
                  call={current}
                  onRespond={(approved, always) => onRespond?.(current.id, approved, always)}
                />
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      </div>
    </MotionConfig>
  )
}

export { KokonutAIToolApproval }
