'use client'

/**
 * KokonutUI — AI Tool Calls (IconStack + folding tool timeline + live spinner)
 * Adapted strictly to Zenko 60/30/10 CSS tokens with zero regex.
 */

import { CalendarCheck, ChevronRight, FileText, Search, Terminal, Wrench, type LucideIcon } from 'lucide-react'
import { AnimatePresence, MotionConfig, motion } from 'motion/react'
import { type ReactNode, useEffect, useId, useState } from 'react'
import { cn } from '@/lib/utils'

export type KokonutToolCallState =
  | 'input-streaming'
  | 'input-available'
  | 'approval-requested'
  | 'approval-responded'
  | 'output-available'
  | 'output-error'
  | 'output-denied'

export interface KokonutToolCallItem {
  id: string
  tool: string
  running?: string
  done?: string
  detail?: string
  result?: string
  state?: KokonutToolCallState
  error?: string
  durationMs?: number
  icon?: ReactNode
}

export interface KokonutAIToolCallsProps {
  calls: KokonutToolCallItem[]
  defaultOpen?: boolean
  className?: string
}

const EASE_OUT = [0.23, 1, 0.32, 1] as const

const TOOL_ICON_RULES: ReadonlyArray<{ keywords: readonly string[]; icon: LucideIcon }> = [
  { keywords: ['search', 'web', 'grep', 'find'], icon: Search },
  { keywords: ['read', 'fetch', 'page', 'file', 'edit', 'write'], icon: FileText },
  { keywords: ['shell', 'exec', 'bash', 'run', 'cmd', 'terminal'], icon: Terminal },
  { keywords: ['calendar', 'cron', 'schedule', 'task', 'book'], icon: CalendarCheck },
]

function iconFor(tool: string): LucideIcon {
  const lower = tool.toLowerCase()
  const hit = TOOL_ICON_RULES.find((r) => r.keywords.some((kw) => lower.includes(kw)))
  return hit ? hit.icon : Wrench
}

function humanizeToolName(name: string): string {
  return name.split('_').join(' ').split('-').join(' ')
}

const isEnded = (state?: KokonutToolCallState) =>
  state === 'output-available' || state === 'output-error' || state === 'output-denied'

const isFailed = (state?: KokonutToolCallState) =>
  state === 'output-error' || state === 'output-denied'

function formatDurationMs(ms?: number): string | null {
  if (ms === undefined || ms < 0) return null
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${Math.round(ms)}ms`
}

function Spinner() {
  return (
    <motion.svg
      animate={{ rotate: 360 }}
      aria-hidden="true"
      className="size-3.5 text-[var(--accent)]"
      fill="none"
      transition={{ duration: 0.7, ease: 'linear', repeat: Number.POSITIVE_INFINITY }}
      viewBox="0 0 16 16"
    >
      <circle cx="8" cy="8" opacity="0.2" r="6.25" stroke="currentColor" strokeWidth="1.5" />
      <path d="M8 1.75a6.25 6.25 0 0 1 6.25 6.25" stroke="currentColor" strokeLinecap="round" strokeWidth="1.5" />
    </motion.svg>
  )
}

export default function KokonutAIToolCalls({
  calls,
  defaultOpen = true,
  className,
}: KokonutAIToolCallsProps) {
  const listId = useId()
  const finished = calls.length > 0 && calls.every((c) => isEnded(c.state))
  const failures = calls.filter((c) => isFailed(c.state)).length
  const totalMs = calls.reduce((acc, c) => acc + (c.durationMs ?? 0), 0)
  const [open, setOpen] = useState(defaultOpen || !finished)

  useEffect(() => {
    if (!finished) setOpen(true)
  }, [finished])

  const noun = calls.length === 1 ? 'tool' : 'tools'
  const summary = `Used ${calls.length} ${noun}${failures > 0 ? `, ${failures} failed` : ''}`

  return (
    <MotionConfig reducedMotion="user">
      <div className={cn('flex w-full flex-col', className)} data-state={finished ? 'done' : 'running'}>
        <button
          aria-controls={listId}
          aria-expanded={open}
          className="group flex h-7 w-fit cursor-pointer items-center gap-2 rounded-md text-[12.5px] outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]/40"
          onClick={() => setOpen((o) => !o)}
          type="button"
        >
          {finished ? (
            <span className="flex items-center">
              {calls.slice(0, 5).map((call, i) => {
                const Icon = iconFor(call.tool)
                const failed = isFailed(call.state)
                return (
                  <span
                    key={call.id}
                    className="-ml-1.5 flex size-5 items-center justify-center rounded-full border bg-[var(--card)] first:ml-0"
                    style={{
                      zIndex: calls.length - i,
                      borderColor: failed ? 'var(--destructive)' : 'var(--border)',
                      color: failed ? 'var(--destructive)' : 'var(--accent)',
                    }}
                  >
                    {call.icon ?? <Icon className="size-2.5" />}
                  </span>
                )
              })}
            </span>
          ) : (
            <span className="flex size-5 items-center justify-center">
              <Spinner />
            </span>
          )}
          <span
            className={cn(
              'font-medium transition-colors',
              finished
                ? 'text-[var(--muted)] group-hover:text-[var(--fg)]'
                : 'text-[var(--fg)]',
            )}
          >
            {finished ? summary : 'Working'}
          </span>
          {totalMs > 0 && (
            <span className="font-mono text-[11px] tabular-nums text-[var(--muted)]">
              {formatDurationMs(totalMs)}
            </span>
          )}
          <ChevronRight
            aria-hidden="true"
            className={cn(
              'size-3.5 text-[var(--muted)] transition-transform duration-200',
              open && 'rotate-90',
            )}
          />
        </button>

        <AnimatePresence initial={false}>
          {open && calls.length > 0 && (
            <motion.div
              animate={{ height: 'auto', opacity: 1 }}
              className="overflow-hidden"
              exit={{ height: 0, opacity: 0 }}
              id={listId}
              initial={{ height: 0, opacity: 0 }}
              transition={{ duration: 0.24, ease: EASE_OUT }}
            >
              <ol className="mt-1 ml-[9.5px] border-l border-[var(--border)] pl-4">
                {calls.map((call) => {
                  const Icon = iconFor(call.tool)
                  const running = !isEnded(call.state)
                  const error = isFailed(call.state)
                  const label = running
                    ? call.running ?? humanizeToolName(call.tool)
                    : call.done ?? call.running ?? humanizeToolName(call.tool)
                  const trailing = running
                    ? 'running'
                    : error
                      ? call.error ?? (call.state === 'output-denied' ? 'Denied' : 'Failed')
                      : call.result ?? formatDurationMs(call.durationMs) ?? 'ok'

                  return (
                    <li key={call.id} className="list-none">
                      <div className="flex h-7.5 items-center gap-2 text-[12.5px]">
                        <Icon
                          aria-hidden="true"
                          className="size-3.5 shrink-0"
                          style={{
                            color: error
                              ? 'var(--destructive)'
                              : running
                                ? 'var(--accent)'
                                : 'var(--muted)',
                          }}
                        />
                        <span
                          className={cn(
                            'shrink-0',
                            running ? 'font-medium text-[var(--fg)]' : 'text-[var(--fg)]/80',
                          )}
                        >
                          {label}
                        </span>
                        {call.detail && (
                          <span className="min-w-0 truncate font-mono text-[11px] text-[var(--muted)]">
                            {call.detail}
                          </span>
                        )}
                        <span
                          className="ml-auto shrink-0 pl-2 font-mono text-[11px] tabular-nums"
                          style={{
                            color: error
                              ? 'var(--destructive)'
                              : running
                                ? 'var(--accent)'
                                : 'var(--muted)',
                          }}
                        >
                          {trailing}
                        </span>
                      </div>
                    </li>
                  )
                })}
              </ol>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </MotionConfig>
  )
}

export { KokonutAIToolCalls }
