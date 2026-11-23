'use client'

import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react'
import { cn } from '@/lib/utils'

export interface MarkerSegment {
  text: string
  /** Optional hand-drawn marker stroke group (1, 2, or 3 for staggered animation). */
  mark?: 1 | 2 | 3
}

export interface MarkerHighlightProps {
  /** Primary highlighter stroke color (marks 1 & 3). */
  mainColor?: string
  /** Secondary highlighter stroke color (mark 2). */
  secondaryColor?: string
  /** Text color over the mark (defaults to current theme foreground). */
  markTextColor?: string
  /** Base font size in px (clamped 12..24). */
  size?: number
  /** Animation speed multiplier (clamped 0.5..2). */
  speed?: number
  /** Paragraph segments with up to 3 staggered highlight strokes. */
  segments?: readonly MarkerSegment[]
  /** Label for the Replay trigger button. */
  replayLabel?: string
  /** Show the replay button below the paragraph. */
  showReplay?: boolean
  className?: string
}

const DEFAULT_SEGMENTS: readonly MarkerSegment[] = [
  { text: "We don't sell templates,", mark: 1 },
  { text: ' we design websites people get at first glance, ' },
  { text: 'load in a second', mark: 2 },
  { text: ' and look good on any phone. We do it all ' },
  { text: 'with your team, not for your team', mark: 3 },
  { text: '.' },
]

/**
 * AIMRAN Text & Scroll — Marker Pen Highlight.
 * Highlights key phrases sequentially with hand-drawn marker strokes once scrolled into view.
 * Styles live once in `index.css` under `.zenko-marker-highlight`.
 */
export default function MarkerHighlight({
  mainColor = '#fde047',
  secondaryColor = '#f9a8d4',
  markTextColor,
  size = 16,
  speed = 1,
  segments = DEFAULT_SEGMENTS,
  replayLabel = 'Replay',
  showReplay = true,
  className,
}: MarkerHighlightProps) {
  const rootRef = useRef<HTMLDivElement>(null)
  const [active, setActive] = useState(false)
  const clampedSize = Math.min(24, Math.max(12, size))
  const clampedSpeed = Math.min(2, Math.max(0.5, speed))

  const replay = useCallback(() => {
    setActive(false)
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        setActive(true)
      })
    })
  }, [])

  useEffect(() => {
    const el = rootRef.current
    if (!el) return
    if (typeof IntersectionObserver === 'undefined') {
      setActive(true)
      return
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0]?.isIntersecting) {
          replay()
          observer.disconnect()
        }
      },
      { threshold: 0.35 },
    )
    observer.observe(el)
    return () => observer.disconnect()
  }, [replay])

  const vars = {
    '--c1': mainColor,
    '--c2': secondaryColor,
    '--s': `${clampedSize}px`,
    '--k': String(clampedSpeed),
    ...(markTextColor ? { '--mark-fg': markTextColor } : {}),
  } as CSSProperties

  return (
    <div
      ref={rootRef}
      style={vars}
      className={cn('zenko-marker-highlight w-full max-w-full', active && 'on', className)}
    >
      <p>
        {segments.map((seg, idx) =>
          seg.mark ? (
            <mark key={idx} className={`m${seg.mark}`}>
              {seg.text}
            </mark>
          ) : (
            <span key={idx}>{seg.text}</span>
          ),
        )}
      </p>
      {showReplay && (
        <button type="button" className="rp" onClick={replay}>
          {replayLabel}
        </button>
      )}
    </div>
  )
}

export { MarkerHighlight }
