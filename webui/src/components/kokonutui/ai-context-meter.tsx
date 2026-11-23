'use client'

/**
 * KokonutUI — AI Context Meter with 4x4 Bayer-matrix dither bar & breakdown popover
 * (adapted strictly to Zenko 60/30/10 tokens + ContextMeterChrome border/background modes).
 */

import { AnimatePresence, MotionConfig, motion } from 'motion/react'
import { type CSSProperties, useEffect, useId, useRef, useState } from 'react'
import { cn } from '@/lib/utils'
import {
  CONTEXT_METER_CHROME_CLASS,
  type ContextMeterChrome,
} from '@/components/smoothui/ai-context-meter'

export interface KokonutContextSegment {
  id: string
  label: string
  tokens: number
  opacity?: number
}

export interface KokonutContextMeterProps {
  segments: KokonutContextSegment[]
  limit?: number
  compactAt?: number
  onCompact?: () => void
  model?: string
  /** Compact mode renders just the ring + percentage trigger (for PromptComposer toolbar). */
  compact?: boolean
  /** Border & background chrome visibility matching Zenko Context Meter customization. */
  chrome?: ContextMeterChrome
  defaultOpen?: boolean
  className?: string
}

const BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5]

function pixelMask(
  columns: number,
  rows: number,
  densityAt: (row: number) => number,
  cell: number,
): CSSProperties {
  let rects = ''
  for (let y = 0; y < rows; y++) {
    const on = Math.round(densityAt(y) * 16)
    for (let x = 0; x < columns; x++) {
      if (BAYER[(y % 4) * 4 + (x % 4)] < on) {
        rects += `<rect x='${x}' y='${y}' width='1' height='1'/>`
      }
    }
  }
  const svg = `<svg xmlns='http://www.w3.org/2000/svg' width='${columns}' height='${rows}' shape-rendering='crispEdges'>${rects}</svg>`
  const image = `url("data:image/svg+xml,${encodeURIComponent(svg)}")`
  const size = `${columns * cell}px ${rows * cell}px`
  return {
    maskImage: image,
    WebkitMaskImage: image,
    maskSize: size,
    WebkitMaskSize: size,
    maskPosition: 'left bottom',
    WebkitMaskPosition: 'left bottom',
  }
}

const FREE_DITHER = pixelMask(4, 4, () => 0.25, 2)
const LIMIT_DITHER = pixelMask(4, 4, () => 0.25, 2)
const FREE_SWATCH = pixelMask(4, 4, () => 0.5, 1)
const EASE_OUT = [0.23, 1, 0.32, 1] as const

const SEGMENT_TONES = [
  'var(--accent)',
  'color-mix(in srgb, var(--accent) 68%, var(--fg))',
  'color-mix(in srgb, var(--fg) 56%, var(--surface))',
  'color-mix(in srgb, var(--muted) 72%, var(--surface))',
]

function useTweened(target: number, duration = 360) {
  const [value, setValue] = useState(target)
  const fromRef = useRef(target)
  const valueRef = useRef(target)

  useEffect(() => {
    const reduce =
      typeof window !== 'undefined' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches
    if (reduce) {
      valueRef.current = target
      setValue(target)
      return
    }
    fromRef.current = valueRef.current
    const start = performance.now()
    let frame = 0
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / duration)
      const eased = 1 - (1 - t) ** 3
      valueRef.current = fromRef.current + (target - fromRef.current) * eased
      setValue(valueRef.current)
      if (t < 1) frame = requestAnimationFrame(step)
    }
    frame = requestAnimationFrame(step)
    return () => cancelAnimationFrame(frame)
  }, [target, duration])

  return value
}

const formatTokens = (tokens: number) =>
  tokens >= 1000
    ? `${(tokens / 1000).toFixed(tokens >= 10_000 ? 0 : 1)}k`
    : `${Math.round(tokens)}`

const formatPct = (value: number) => `${value.toFixed(1)}%`

function Ring({ share }: { share: number }) {
  const radius = 6
  const circumference = 2 * Math.PI * radius
  return (
    <svg aria-hidden="true" className="size-4 shrink-0 -rotate-90" fill="none" viewBox="0 0 16 16">
      <circle
        cx="8"
        cy="8"
        r={radius}
        stroke="var(--border)"
        strokeWidth="2"
      />
      <circle
        cx="8"
        cy="8"
        r={radius}
        stroke="currentColor"
        strokeDasharray={circumference}
        strokeDashoffset={circumference * (1 - Math.min(1, Math.max(0, share)))}
        strokeLinecap="round"
        strokeWidth="2"
      />
    </svg>
  )
}

export default function KokonutContextMeter({
  segments,
  limit = 128_000,
  compactAt = 0.8,
  onCompact,
  model,
  compact = true,
  chrome = 'bordered_filled',
  defaultOpen = false,
  className,
}: KokonutContextMeterProps) {
  const rootRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const panelId = useId()
  const [open, setOpen] = useState(defaultOpen)
  const [activeSeg, setActiveSeg] = useState<string | null>(null)

  const safeLimit = Math.max(1, limit)
  const used = segments.reduce((sum, s) => sum + Math.max(0, s.tokens), 0)
  const free = Math.max(0, safeLimit - used)
  const share = Math.min(1, used / safeLimit)
  const shownShare = useTweened(share)
  const shownUsed = useTweened(used)

  useEffect(() => {
    if (!open) return
    const onPointer = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        if (rootRef.current?.contains(document.activeElement)) triggerRef.current?.focus()
        setOpen(false)
      }
    }
    document.addEventListener('pointerdown', onPointer)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('pointerdown', onPointer)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  const toneColor =
    share >= compactAt + (1 - compactAt) / 2
      ? 'var(--destructive)'
      : share >= compactAt - 0.1
        ? 'var(--warning)'
        : 'var(--accent)'

  const chromeClass =
    CONTEXT_METER_CHROME_CLASS[chrome] || CONTEXT_METER_CHROME_CLASS.bordered_filled

  const triggerButton = (
    <button
      aria-controls={panelId}
      aria-expanded={open}
      aria-label={`Context ${Math.round(share * 100)}% full. Show breakdown`}
      className={cn(
        'inline-flex cursor-pointer items-center gap-1.5 font-mono text-[11px] outline-none transition-colors focus-visible:ring-2 focus-visible:ring-[var(--accent)]/50',
        chromeClass,
        className,
      )}
      style={{ color: toneColor }}
      onClick={() => setOpen((v) => !v)}
      ref={triggerRef}
      type="button"
    >
      <Ring share={shownShare} />
      <span className="font-medium tabular-nums text-[var(--fg)]">
        {Math.round(shownShare * 100)}%
      </span>
    </button>
  )

  const popoverPanel = (
    <AnimatePresence>
      {open && (
        <motion.div
          animate={{ opacity: 1, scale: 1, y: 0 }}
          className="absolute right-0 bottom-full z-50 mb-2 w-72 origin-bottom-right rounded-[var(--radius-lg)] border border-[var(--border)] bg-[var(--surface)] p-3 shadow-[0_14px_34px_rgba(0,0,0,0.42)]"
          exit={{ opacity: 0, scale: 0.97, y: 4, transition: { duration: 0.12, ease: EASE_OUT } }}
          id={panelId}
          initial={{ opacity: 0, scale: 0.97, y: 4 }}
          transition={{ duration: 0.16, ease: EASE_OUT }}
        >
          <div className="mb-2.5 flex items-baseline justify-between">
            <p className="text-[13px] font-medium text-[var(--fg)]">Context Window</p>
            <p className="font-mono text-[11px] tabular-nums text-[var(--muted)]">
              {formatTokens(shownUsed)} of {formatTokens(safeLimit)}
            </p>
          </div>

          {/* Bayer-dithered segmented bar with compact threshold tick */}
          <div className="relative" onPointerLeave={() => setActiveSeg(null)}>
            <span
              aria-hidden="true"
              className="absolute inset-y-0 left-0 rounded-l-full bg-[var(--muted)]"
              style={{ ...FREE_DITHER, width: `${compactAt * 100}%` }}
            />
            <span
              aria-hidden="true"
              className="absolute inset-y-0 right-0 rounded-r-full bg-[var(--destructive)]"
              style={{ ...LIMIT_DITHER, width: `${(1 - compactAt) * 100}%` }}
            />
            <div className="relative flex h-2 overflow-hidden rounded-full">
              {segments.map((segment, index) => (
                <span
                  key={segment.id}
                  className={cn(
                    'h-full transition-[width,opacity] duration-400',
                    index > 0 && segment.tokens > 0 && 'ml-px',
                    activeSeg && activeSeg !== segment.id && 'opacity-30',
                  )}
                  onPointerEnter={() => setActiveSeg(segment.id)}
                  style={{
                    width: `${(Math.max(0, segment.tokens) / safeLimit) * 100}%`,
                    backgroundColor: SEGMENT_TONES[index % SEGMENT_TONES.length],
                  }}
                />
              ))}
            </div>
            <span
              aria-hidden="true"
              className="absolute -top-1 -bottom-1 w-px bg-[var(--muted)]"
              style={{ left: `${compactAt * 100}%` }}
            />
          </div>
          <p className="mt-1.5 text-right font-mono text-[10.5px] text-[var(--muted)]">
            Compacts at {Math.round(compactAt * 100)}%
          </p>

          <ul className="mt-2 space-y-0.5">
            {segments.map((segment, index) =>
              segment.tokens > 0 ? (
                <li
                  key={segment.id}
                  className={cn(
                    'flex items-center gap-2 rounded-[var(--radius-sm)] px-1.5 py-1 text-[11.5px] transition-colors',
                    activeSeg === segment.id && 'bg-[var(--card)]',
                  )}
                  onPointerEnter={() => setActiveSeg(segment.id)}
                  onPointerLeave={() => setActiveSeg(null)}
                >
                  <span
                    className="size-2 shrink-0 rounded-[3px]"
                    style={{ backgroundColor: SEGMENT_TONES[index % SEGMENT_TONES.length] }}
                  />
                  <span className="truncate text-[var(--fg)]/88">{segment.label}</span>
                  <span className="ml-auto font-mono text-[11px] tabular-nums text-[var(--muted)]">
                    {formatTokens(segment.tokens)}
                  </span>
                  <span className="w-10 text-right font-mono text-[10.5px] tabular-nums text-[var(--muted)]/80">
                    {formatPct((segment.tokens / safeLimit) * 100)}
                  </span>
                </li>
              ) : null,
            )}
            <li className="flex items-center gap-2 px-1.5 py-1 text-[11.5px]">
              <span className="size-2 shrink-0 bg-[var(--muted)]" style={FREE_SWATCH} />
              <span className="text-[var(--muted)]">Free</span>
              <span className="ml-auto font-mono text-[11px] tabular-nums text-[var(--muted)]">
                {formatTokens(free)}
              </span>
              <span className="w-10 text-right font-mono text-[10.5px] tabular-nums text-[var(--muted)]/80">
                {formatPct((free / safeLimit) * 100)}
              </span>
            </li>
          </ul>

          {onCompact && (
            <div className="mt-3 flex items-center justify-between border-t border-[var(--border)] pt-2.5">
              <p className="text-[11px] text-[var(--muted)]">Summarize older turns</p>
              <button
                className="h-7 cursor-pointer rounded-full bg-[var(--primary)] px-3 text-[11px] font-medium text-[var(--primary-fg)] transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
                disabled={used / safeLimit < 0.25}
                onClick={onCompact}
                type="button"
              >
                Compact now
              </button>
            </div>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  )

  return (
    <MotionConfig reducedMotion="user">
      <div className="relative inline-flex" ref={rootRef}>
        {compact ? (
          triggerButton
        ) : (
          <div className="flex h-10 w-full max-w-sm items-center justify-between rounded-full border border-[var(--border)] bg-[var(--card)] pr-1.5 pl-4">
            <span className="truncate font-mono text-xs text-[var(--muted)]">
              {model || 'Active Model'}
            </span>
            {triggerButton}
          </div>
        )}
        {popoverPanel}
      </div>
    </MotionConfig>
  )
}

export { KokonutContextMeter }
