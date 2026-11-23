'use client'

import { useId, useState, type CSSProperties } from 'react'
import { cn } from '@/lib/utils'

export type StretchSegment =
  | { kind: 'text'; text: string }
  | { kind: 'expand'; id: string; trigger: string; detail: string }

export interface StretchtextProps {
  /** Primary accent color for underlines, superscript "+", and expanded triggers. */
  mainColor?: string
  /** Secondary highlight wash flashed during inline expansion. */
  secondaryColor?: string
  /** Base font size in px (clamped 12..24). */
  size?: number
  /** Animation speed multiplier (clamped 0.5..2). */
  speed?: number
  /** Small uppercase kicker above the sentence. */
  kicker?: string
  /** Sentence segments with inline expandable terms. */
  segments?: readonly StretchSegment[]
  /** Label for the expand-all / collapse-all pill button. */
  toggleLabel?: string
  collapseLabel?: string
  /** Whether to show the expand-all pill toggle. */
  showToggleAll?: boolean
  className?: string
}

const DEFAULT_SEGMENTS: readonly StretchSegment[] = [
  { kind: 'text', text: 'We build sites ' },
  {
    kind: 'expand',
    id: 'fast',
    trigger: 'fast',
    detail: ' (they load in under a second, even on a weak signal)',
  },
  { kind: 'text', text: ', ' },
  {
    kind: 'expand',
    id: 'accessible',
    trigger: 'accessible',
    detail: ' (usable with a keyboard, a screen reader or one hand)',
  },
  { kind: 'text', text: ' and ' },
  {
    kind: 'expand',
    id: 'easy',
    trigger: 'easy to change',
    detail: ' (your team edits the copy without calling us)',
  },
  { kind: 'text', text: '.' },
]

/**
 * AIMRAN Text & Scroll — Stretchtext Sentence.
 * Every dotted-underlined word opens inline inside the paragraph itself (no popups).
 * Styles live once in `index.css` under `.zenko-stretchtext`.
 */
export default function Stretchtext({
  mainColor = '#c2410c',
  secondaryColor = '#fde68a',
  size = 16,
  speed = 1,
  kicker = 'Tap the underlined words',
  segments = DEFAULT_SEGMENTS,
  toggleLabel = 'Read it all',
  collapseLabel = 'Collapse all',
  showToggleAll = true,
  className,
}: StretchtextProps) {
  const uid = useId().replace(/[^a-zA-Z0-9]/g, '')
  const clampedSize = Math.min(24, Math.max(12, size))
  const clampedSpeed = Math.min(2, Math.max(0.5, speed))

  const expandableIds = segments
    .filter((s): s is Extract<StretchSegment, { kind: 'expand' }> => s.kind === 'expand')
    .map((s) => s.id)

  const [expanded, setExpanded] = useState<Record<string, boolean>>({})
  const [closing, setClosing] = useState<Record<string, boolean>>({})

  const allExpanded =
    expandableIds.length > 0 && expandableIds.every((id) => Boolean(expanded[id]))

  const setItem = (id: string, nextOpen: boolean) => {
    if (nextOpen) {
      setClosing((prev) => ({ ...prev, [id]: false }))
      setExpanded((prev) => ({ ...prev, [id]: true }))
    } else if (expanded[id]) {
      setClosing((prev) => ({ ...prev, [id]: true }))
      setExpanded((prev) => ({ ...prev, [id]: false }))
    }
  }

  const toggleAll = () => {
    const next = !allExpanded
    for (const id of expandableIds) {
      setItem(id, next)
    }
  }

  const vars = {
    '--c1': mainColor,
    '--c2': secondaryColor,
    '--s': `${clampedSize}px`,
    '--k': String(clampedSpeed),
  } as CSSProperties

  return (
    <div style={vars} className={cn('zenko-stretchtext w-full max-w-full', className)}>
      {kicker && <p className="kc">{kicker}</p>}
      <p className="tx">
        {segments.map((seg, idx) => {
          if (seg.kind === 'text') {
            return <span key={`t-${idx}`}>{seg.text}</span>
          }
          const isOpen = Boolean(expanded[seg.id])
          const isClosing = Boolean(closing[seg.id])
          const visible = isOpen || isClosing
          const panelId = `stretch-${uid}-${seg.id}`
          return (
            <span key={seg.id}>
              <button
                type="button"
                aria-expanded={isOpen}
                aria-controls={panelId}
                onClick={() => setItem(seg.id, !isOpen)}
              >
                {seg.trigger}
              </button>
              <span
                id={panelId}
                hidden={!visible}
                className={cn('ex', isClosing && !isOpen && 'out')}
                onAnimationEnd={() => {
                  if (!isOpen) {
                    setClosing((prev) => ({ ...prev, [seg.id]: false }))
                  }
                }}
              >
                {seg.detail}
              </span>
            </span>
          )
        })}
      </p>
      {showToggleAll && expandableIds.length > 0 && (
        <button
          type="button"
          className="all"
          aria-pressed={allExpanded}
          onClick={toggleAll}
        >
          {allExpanded ? collapseLabel : toggleLabel}
        </button>
      )}
    </div>
  )
}

export { Stretchtext }
