'use client'

/**
 * KokonutUI — AI Loading State (Step Rail + WaveText + auto-scrolling StepLog)
 * Adapted strictly to Zenko 60/30/10 CSS tokens. Wave CSS lives once in `index.css`.
 */

import { ChevronDown } from 'lucide-react'
import { AnimatePresence, MotionConfig, motion } from 'motion/react'
import { useEffect, useId, useState } from 'react'
import { cn } from '@/lib/utils'

export type KokonutAgentStepStatus = 'pending' | 'running' | 'done' | 'error'

export interface KokonutAgentStep {
  id: string
  label: string
  status: KokonutAgentStepStatus
  detail?: string[]
}

export interface KokonutAILoadingProps {
  steps?: KokonutAgentStep[]
  label?: string
  elapsedLabel?: string
  active?: boolean
  collapseWhenDone?: boolean
  className?: string
}

const EASE_OUT = [0.23, 1, 0.32, 1] as const
const LINE_HEIGHT = 22
const VISIBLE_LINES = 3

function WaveText({ text }: { text: string }) {
  return (
    <>
      {Array.from(text).map((char, i) => (
        <span
          key={i}
          className="kokonut-ai-wave whitespace-pre"
          style={{ animationDelay: `${i * 0.045}s` }}
        >
          {char}
        </span>
      ))}
    </>
  )
}

function StepIcon({ status }: { status: KokonutAgentStepStatus }) {
  return (
    <span className="relative flex size-4 shrink-0 items-center justify-center">
      {status === 'pending' && <span className="size-1.5 rounded-full bg-[var(--border)]" />}
      {status === 'running' && (
        <svg
          aria-hidden="true"
          className="size-3.5 motion-safe:animate-spin"
          fill="none"
          style={{ animationDuration: '0.8s' }}
          viewBox="0 0 16 16"
        >
          <circle cx="8" cy="8" r="6.25" stroke="var(--border)" strokeWidth="1.5" />
          <path d="M8 1.75a6.25 6.25 0 0 1 6.25 6.25" stroke="var(--accent)" strokeLinecap="round" strokeWidth="1.5" />
        </svg>
      )}
      {status === 'done' && (
        <svg aria-hidden="true" className="size-3.5 text-[var(--accent)]" fill="none" viewBox="0 0 16 16">
          <motion.path
            animate={{ pathLength: 1 }}
            d="M3.5 8.5 6.5 11.5 12.5 4.5"
            initial={{ pathLength: 0 }}
            stroke="currentColor"
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth="1.75"
            transition={{ duration: 0.3, ease: EASE_OUT }}
          />
        </svg>
      )}
      {status === 'error' && (
        <svg aria-hidden="true" className="size-3.5 text-[var(--destructive)]" fill="none" viewBox="0 0 16 16">
          <path d="M4.5 4.5l7 7m0-7-7 7" stroke="currentColor" strokeLinecap="round" strokeWidth="1.75" />
        </svg>
      )}
    </span>
  )
}

function StepLog({ lines }: { lines: string[] }) {
  const offset = Math.max(0, lines.length - VISIBLE_LINES)
  return (
    <div
      aria-hidden="true"
      className="relative overflow-hidden font-mono text-[11px]"
      style={{
        height: LINE_HEIGHT * Math.min(lines.length, VISIBLE_LINES),
        maskImage:
          lines.length > VISIBLE_LINES - 1
            ? 'linear-gradient(to bottom, rgb(0 0 0 / 0.25), black 70%)'
            : undefined,
        WebkitMaskImage:
          lines.length > VISIBLE_LINES - 1
            ? 'linear-gradient(to bottom, rgb(0 0 0 / 0.25), black 70%)'
            : undefined,
        transition: 'height 220ms cubic-bezier(0.23, 1, 0.32, 1)',
      }}
    >
      <div
        className="transition-transform duration-300 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none"
        style={{ transform: `translateY(${-offset * LINE_HEIGHT}px)` }}
      >
        {lines.map((line, idx) => (
          <div
            key={`${idx}-${line}`}
            className="flex items-center gap-2 text-[var(--muted)]"
            style={{ height: LINE_HEIGHT }}
          >
            <span className="w-4 select-none text-right tabular-nums opacity-50">{idx + 1}</span>
            <span className="truncate">{line}</span>
          </div>
        ))}
      </div>
    </div>
  )
}

export default function KokonutAILoading({
  steps: stepsProp,
  label = 'Thinking & running tools...',
  elapsedLabel,
  active = true,
  collapseWhenDone = false,
  className,
}: KokonutAILoadingProps) {
  const listId = useId()
  const [pulseStep, setPulseStep] = useState(0)

  // Compact animated single-line or stepped rail when no explicit step list is provided
  useEffect(() => {
    if (stepsProp || !active) return
    const id = window.setInterval(() => setPulseStep((s) => (s + 1) % 3), 1600)
    return () => window.clearInterval(id)
  }, [stepsProp, active])

  const derivedSteps: KokonutAgentStep[] = stepsProp ?? [
    {
      id: 'ctx',
      label: 'Analyzing workspace context',
      status: pulseStep > 0 || !active ? 'done' : 'running',
      detail: ['loading prompt tiers', 'scanning active symbols'],
    },
    {
      id: 'exec',
      label,
      status: !active ? 'done' : pulseStep >= 1 ? 'running' : 'pending',
      detail: ['streaming deltas', 'evaluating tool output'],
    },
    {
      id: 'compose',
      label: 'Synthesizing final answer',
      status: !active ? 'done' : pulseStep === 2 ? 'running' : 'pending',
    },
  ]

  const running = derivedSteps.find((s) => s.status === 'running')
  const isDone = derivedSteps.every((s) => s.status === 'done')
  const [isOpen, setIsOpen] = useState(!(isDone && collapseWhenDone))

  const title = running ? running.label : isDone ? 'Completed' : label

  return (
    <MotionConfig reducedMotion="user">
      <div className={cn('w-full max-w-md', className)}>
        <div
          role="button"
          tabIndex={0}
          aria-controls={listId}
          aria-expanded={isOpen}
          onClick={(e) => {
            e.stopPropagation()
            setIsOpen((o) => !o)
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault()
              e.stopPropagation()
              setIsOpen((o) => !o)
            }
          }}
          className="group/header flex w-full cursor-pointer items-center gap-2 rounded-md py-0.5 text-left text-[13px] font-medium text-[var(--fg)] outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]/40"
        >
          <StepIcon status={running ? 'running' : isDone ? 'done' : 'pending'} />
          <output className="min-w-0 truncate">
            {running ? <WaveText text={title} /> : title}
          </output>
          {elapsedLabel && (
            <span className="font-mono text-[11px] tabular-nums text-[var(--muted)]">
              {elapsedLabel}
            </span>
          )}
          <ChevronDown
            aria-hidden="true"
            className={cn(
              'ml-auto size-3.5 text-[var(--muted)] transition-transform duration-200',
              isOpen && 'rotate-180',
            )}
          />
        </div>

        <AnimatePresence initial={false}>
          {isOpen && (
            <motion.ol
              animate={{ height: 'auto', opacity: 1 }}
              className="overflow-hidden"
              exit={{ height: 0, opacity: 0 }}
              id={listId}
              initial={{ height: 0, opacity: 0 }}
              transition={{ duration: 0.22, ease: EASE_OUT }}
            >
              {derivedSteps.map((step, i) => {
                const isLast = i === derivedSteps.length - 1
                return (
                  <li key={step.id} className="relative flex gap-2.5 pt-2.5">
                    {!isLast && (
                      <span
                        aria-hidden="true"
                        className="absolute top-[28px] bottom-[-6px] left-[7.5px] w-px overflow-hidden bg-[var(--border)]"
                      >
                        <span
                          className={cn(
                            'block h-full w-full origin-top transition-transform duration-500',
                            step.status === 'done' ? 'scale-y-100' : 'scale-y-0',
                          )}
                          style={{ backgroundColor: 'var(--accent)' }}
                        />
                      </span>
                    )}
                    <StepIcon status={step.status} />
                    <div className="-mt-0.5 min-w-0 flex-1">
                      <p
                        className={cn(
                          'truncate text-[12.5px] transition-colors',
                          step.status === 'running'
                            ? 'font-medium text-[var(--fg)]'
                            : step.status === 'error'
                              ? 'text-[var(--destructive)]'
                              : 'text-[var(--muted)]',
                        )}
                      >
                        {step.status === 'running' ? <WaveText text={step.label} /> : step.label}
                      </p>
                      {step.status === 'running' && step.detail?.length ? (
                        <div className="mt-0.5">
                          <StepLog lines={step.detail} />
                        </div>
                      ) : null}
                    </div>
                  </li>
                )
              })}
            </motion.ol>
          )}
        </AnimatePresence>
      </div>
    </MotionConfig>
  )
}

export { KokonutAILoading }
