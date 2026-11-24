'use client'

import { useEffect, useState } from 'react'
import { motion, useReducedMotion } from 'motion/react'
import { cn } from '@/lib/utils'

export type ScrollIndicatorStyle =
  | 'none'
  | 'skiper87_fade'
  | 'skiper89_bar'
  | 'skiper94_ring'
  | 'skiper87_and_89'

/** Subscribes reactively to `html[data-scroll-indicator]`. */
export function useScrollIndicatorStyle(override?: ScrollIndicatorStyle): ScrollIndicatorStyle {
  const [style, setStyle] = useState<ScrollIndicatorStyle>(() => {
    if (override) return override
    if (typeof document === 'undefined') return 'none'
    return (
      (document.documentElement.dataset.scrollIndicator as ScrollIndicatorStyle | undefined) ||
      'none'
    )
  })

  useEffect(() => {
    if (override) {
      setStyle(override)
      return
    }
    if (typeof document === 'undefined') return
    const sync = () => {
      setStyle(
        (document.documentElement.dataset.scrollIndicator as ScrollIndicatorStyle | undefined) ||
          'none',
      )
    }
    sync()
    const obs = new MutationObserver(sync)
    obs.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-scroll-indicator'],
    })
    return () => obs.disconnect()
  }, [override])

  return override ?? style
}

export interface ScrollProgressMetrics {
  overflow: boolean
  progress: number
  atTop: boolean
  atBottom: boolean
}

/**
 * Skiper87 (Scroll Fade), Skiper89 (Scroll Progress 001 Bar), and Skiper94 (Scroll Progress 002 Ring).
 * Renders purely theme-token-driven overlays inside any relative scroll container with zero idle CPU.
 */
export function ScrollEffectsOverlay({
  metrics,
  scrolling,
  modeOverride,
  className,
}: {
  metrics: ScrollProgressMetrics
  scrolling: boolean
  modeOverride?: ScrollIndicatorStyle
  className?: string
}) {
  const mode = useScrollIndicatorStyle(modeOverride)
  const reduce = useReducedMotion() ?? false

  if (mode === 'none' || !metrics.overflow) return null

  const pct = Math.round(Math.max(0, Math.min(1, metrics.progress)) * 100)
  const showFade = mode === 'skiper87_fade' || mode === 'skiper87_and_89'
  const showBar = mode === 'skiper89_bar' || mode === 'skiper87_and_89'
  const showRing = mode === 'skiper94_ring'

  const ringRadius = 11
  const ringCirc = 2 * Math.PI * ringRadius
  const ringOffset = ringCirc * (1 - Math.max(0, Math.min(1, metrics.progress)))

  return (
    <>
      {/* Skiper87: Top & Bottom smooth boundary fade masks */}
      {showFade && (
        <>
          <div
            aria-hidden="true"
            className={cn(
              'pointer-events-none absolute inset-x-0 top-0 z-10 h-8 transition-opacity duration-200',
              metrics.atTop ? 'opacity-0' : 'opacity-100',
              className,
            )}
            style={{
              background: 'linear-gradient(to bottom, var(--bg) 0%, transparent 100%)',
            }}
          />
          <div
            aria-hidden="true"
            className={cn(
              'pointer-events-none absolute inset-x-0 bottom-0 z-10 h-8 transition-opacity duration-200',
              metrics.atBottom ? 'opacity-0' : 'opacity-100',
              className,
            )}
            style={{
              background: 'linear-gradient(to top, var(--bg) 0%, transparent 100%)',
            }}
          />
        </>
      )}

      {/* Skiper89: Scroll Progress 001 — sleek top accent bar + scrolling percent pill */}
      {showBar && (
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-x-0 top-0 z-20"
        >
          <div className="h-[2px] w-full bg-[var(--border)]/45">
            <motion.div
              className="h-full origin-left rounded-r-full"
              style={{ backgroundColor: 'var(--accent)' }}
              animate={{ scaleX: Math.max(0.01, metrics.progress) }}
              transition={
                reduce
                  ? { duration: 0 }
                  : { type: 'spring', stiffness: 320, damping: 32, mass: 0.4 }
              }
            />
          </div>
          <div
            className={cn(
              'absolute right-3 top-2 rounded-full border border-[var(--border)] bg-[var(--surface)]/92 px-2 py-0.5 font-mono text-[10px] tabular-nums text-[var(--muted)] shadow-sm backdrop-blur-sm transition-opacity duration-200',
              scrolling ? 'opacity-95' : 'opacity-0',
            )}
          >
            <span style={{ color: 'var(--accent)' }}>{pct}%</span>
          </div>
        </div>
      )}

      {/* Skiper94: Scroll Progress 002 — floating radial SVG ring badge */}
      {showRing && (
        <div
          aria-hidden="true"
          className={cn(
            'pointer-events-none absolute right-3 top-3 z-20 flex h-8 w-8 items-center justify-center rounded-full border border-[var(--border)] bg-[var(--surface)]/92 shadow-md backdrop-blur-sm transition-opacity duration-200',
            scrolling || pct > 0 ? 'opacity-90' : 'opacity-0',
          )}
        >
          <svg viewBox="0 0 28 28" className="size-7 -rotate-90">
            <circle
              cx="14"
              cy="14"
              r={ringRadius}
              fill="none"
              stroke="var(--border)"
              strokeWidth="2"
            />
            <circle
              cx="14"
              cy="14"
              r={ringRadius}
              fill="none"
              stroke="var(--accent)"
              strokeWidth="2"
              strokeLinecap="round"
              strokeDasharray={ringCirc}
              strokeDashoffset={ringOffset}
              style={{
                transition: reduce ? 'none' : 'stroke-dashoffset 140ms ease-out',
              }}
            />
          </svg>
          <span className="absolute font-mono text-[8.5px] font-semibold tabular-nums text-[var(--fg)]">
            {pct}
          </span>
        </div>
      )}
    </>
  )
}
