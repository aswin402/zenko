import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
import { createPortal } from 'react-dom'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import type { UiCustomization } from '@/lib/types'
import { cn } from '@/lib/utils'

type TooltipSide = 'top' | 'bottom' | 'left' | 'right'
type TooltipStyleVariant = NonNullable<UiCustomization['surface']['tooltip_style']>
type TooltipShapeVariant = NonNullable<UiCustomization['surface']['tooltip_shape']>
type TooltipMotionVariant = NonNullable<UiCustomization['surface']['tooltip_motion']>

export interface ZenkoTooltipConfig {
  style: TooltipStyleVariant
  shape: TooltipShapeVariant
  motion: TooltipMotionVariant
  enabled: boolean
}

const DEFAULT_TOOLTIP_CONFIG: ZenkoTooltipConfig = {
  style: 'accent_edge',
  shape: 'pill',
  motion: 'spring',
  enabled: true,
}

const ZenkoTooltipContext = createContext<ZenkoTooltipConfig>(DEFAULT_TOOLTIP_CONFIG)

export function ZenkoTooltipProvider({
  surface,
  children,
}: {
  surface?: UiCustomization['surface']
  children: React.ReactNode
}) {
  const style = surface?.tooltip_style || 'accent_edge'
  const shape = surface?.tooltip_shape || 'pill'
  const motion = surface?.tooltip_motion || 'spring'
  const enabled = surface?.show_hover_hints !== false
  const value = useMemo<ZenkoTooltipConfig>(
    () => ({ style, shape, motion, enabled }),
    [style, shape, motion, enabled],
  )
  return <ZenkoTooltipContext.Provider value={value}>{children}</ZenkoTooltipContext.Provider>
}

export function useZenkoTooltipConfig(): ZenkoTooltipConfig {
  return useContext(ZenkoTooltipContext)
}

const SIDE_MOTION: Record<TooltipSide, { x: number; y: number }> = {
  top: { x: 0, y: 5 },
  bottom: { x: 0, y: -5 },
  left: { x: 5, y: 0 },
  right: { x: -5, y: 0 },
}

const SIDE_TRANSFORM: Record<TooltipSide, string> = {
  top: 'translate(-50%, -100%)',
  bottom: 'translate(-50%, 0%)',
  left: 'translate(-100%, -50%)',
  right: 'translate(0%, -50%)',
}

const TOOLTIP_SHAPE_CLASS: Record<TooltipShapeVariant, string> = {
  pill: 'rounded-full px-3 py-1.5',
  rounded: 'rounded-lg px-2.5 py-1.5',
  sharp: 'rounded-[4px] px-2.5 py-1',
}

const TOOLTIP_STYLE_SPEC: Record<
  TooltipStyleVariant,
  { className: string; inlineStyle?: CSSProperties; kbdClass: string }
> = {
  accent_edge: {
    className:
      'border bg-[var(--card)]/96 text-[var(--fg)] shadow-[0_10px_28px_rgba(0,0,0,0.34)] backdrop-blur-md',
    inlineStyle: { borderColor: 'color-mix(in srgb, var(--accent) 36%, var(--border))' },
    kbdClass: 'border-[var(--border)] bg-[var(--surface)] text-[var(--muted)]',
  },
  minimal_glass: {
    className:
      'border border-[var(--border)]/70 bg-[var(--surface)]/72 text-[var(--fg)] shadow-[0_12px_32px_rgba(0,0,0,0.3)] backdrop-blur-xl',
    kbdClass: 'border-[var(--border)]/70 bg-[var(--bg)]/50 text-[var(--muted)]',
  },
  solid_contrast: {
    className:
      'border border-transparent bg-[var(--fg)] text-[var(--bg)] shadow-[0_10px_24px_rgba(0,0,0,0.32)]',
    kbdClass: 'border-[var(--bg)]/25 bg-[var(--bg)]/12 text-[var(--bg)]/85',
  },
  accent_solid: {
    className:
      'border border-transparent bg-[var(--accent)] text-[var(--primary-fg)] shadow-[0_10px_26px_rgba(0,0,0,0.32)]',
    kbdClass: 'border-[var(--primary-fg)]/25 bg-[var(--primary-fg)]/15 text-[var(--primary-fg)]',
  },
  quiet_matte: {
    className:
      'border border-[var(--border)] bg-[var(--surface)] text-[var(--fg)]/90 shadow-lg',
    kbdClass: 'border-[var(--border)] bg-[var(--bg)] text-[var(--muted)]',
  },
}

const GAP_PX = 9

function computeAnchor(rect: DOMRect, side: TooltipSide): { left: number; top: number } {
  const anchors: Record<TooltipSide, { left: number; top: number }> = {
    top: { left: rect.left + rect.width / 2, top: rect.top - GAP_PX },
    bottom: { left: rect.left + rect.width / 2, top: rect.bottom + GAP_PX },
    left: { left: rect.left - GAP_PX, top: rect.top + rect.height / 2 },
    right: { left: rect.right + GAP_PX, top: rect.top + rect.height / 2 },
  }
  const raw = anchors[side]
  return {
    left: Math.max(12, Math.min(window.innerWidth - 12, raw.left)),
    top: Math.max(12, Math.min(window.innerHeight - 12, raw.top)),
  }
}

/** Static visual preview pill used inside Theme Studio -> Hover Hints. */
export function ZenkoTooltipBadge({
  label,
  shortcut,
  config,
  className,
}: {
  label: string
  shortcut?: string
  config?: Partial<ZenkoTooltipConfig>
  className?: string
}) {
  const ctx = useZenkoTooltipConfig()
  const styleKey = config?.style ?? ctx.style
  const shapeKey = config?.shape ?? ctx.shape
  const styleSpec = TOOLTIP_STYLE_SPEC[styleKey] || TOOLTIP_STYLE_SPEC.accent_edge
  const shapeClass = TOOLTIP_SHAPE_CLASS[shapeKey] || TOOLTIP_SHAPE_CLASS.pill

  return (
    <div
      role="tooltip"
      className={cn(
        'inline-flex items-center gap-2 whitespace-nowrap text-[11.5px] font-medium tracking-[-0.01em]',
        shapeClass,
        styleSpec.className,
        className,
      )}
      style={styleSpec.inlineStyle}
    >
      <span>{label}</span>
      {shortcut && (
        <kbd
          className={cn(
            'rounded border px-1.5 py-0.5 font-mono text-[9.5px] leading-none',
            styleSpec.kbdClass,
          )}
        >
          {shortcut}
        </kbd>
      )}
    </div>
  )
}

/**
 * Portal-rendered, globally customizable hover hint badge (replaces raw browser `title` tooltips).
 * Honors Theme Studio -> Hover Hints (`tooltip_style`, `tooltip_shape`, `tooltip_motion`, `show_hover_hints`).
 */
export function ZenkoTooltip({
  label,
  shortcut,
  side = 'top',
  disabled = false,
  className,
  children,
}: {
  label: string
  shortcut?: string
  side?: TooltipSide
  disabled?: boolean
  className?: string
  children: React.ReactNode
}) {
  const cfg = useZenkoTooltipConfig()
  const [open, setOpen] = useState(false)
  const [coords, setCoords] = useState<{ left: number; top: number } | null>(null)
  const triggerRef = useRef<HTMLDivElement>(null)
  const timer = useRef<number | undefined>(undefined)
  const reduce = useReducedMotion() ?? false

  const effectivelyDisabled = disabled || !cfg.enabled || !label

  const syncCoords = useCallback(() => {
    const el = triggerRef.current
    if (!el) return
    setCoords(computeAnchor(el.getBoundingClientRect(), side))
  }, [side])

  const show = () => {
    if (effectivelyDisabled) return
    window.clearTimeout(timer.current)
    const delayMs = cfg.motion === 'instant' ? 40 : 105
    timer.current = window.setTimeout(() => {
      syncCoords()
      setOpen(true)
    }, delayMs)
  }

  const hide = () => {
    window.clearTimeout(timer.current)
    setOpen(false)
  }

  useEffect(() => {
    if (!open) return
    const dismiss = () => setOpen(false)
    window.addEventListener('scroll', dismiss, true)
    window.addEventListener('resize', dismiss)
    return () => {
      window.removeEventListener('scroll', dismiss, true)
      window.removeEventListener('resize', dismiss)
    }
  }, [open])

  useEffect(() => () => window.clearTimeout(timer.current), [])

  const offset = SIDE_MOTION[side]
  const styleSpec = TOOLTIP_STYLE_SPEC[cfg.style] || TOOLTIP_STYLE_SPEC.accent_edge
  const shapeClass = TOOLTIP_SHAPE_CLASS[cfg.shape] || TOOLTIP_SHAPE_CLASS.pill
  const useInstant = reduce || cfg.motion === 'instant'
  const useFadeOnly = cfg.motion === 'fade'

  return (
    <div
      ref={triggerRef}
      className={cn('relative inline-flex', className)}
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocus={show}
      onBlur={hide}
      onMouseDown={hide}
    >
      {children}
      {typeof document !== 'undefined' &&
        createPortal(
          <AnimatePresence>
            {open && !effectivelyDisabled && coords && (
              <div
                className="pointer-events-none fixed z-[10000]"
                style={{
                  left: coords.left,
                  top: coords.top,
                  transform: SIDE_TRANSFORM[side],
                }}
              >
                <motion.div
                  role="tooltip"
                  initial={
                    useInstant
                      ? { opacity: 1 }
                      : useFadeOnly
                        ? { opacity: 0 }
                        : { opacity: 0, x: offset.x, y: offset.y, scale: 0.95 }
                  }
                  animate={{ opacity: 1, x: 0, y: 0, scale: 1 }}
                  exit={
                    useInstant
                      ? { opacity: 0 }
                      : useFadeOnly
                        ? { opacity: 0 }
                        : { opacity: 0, x: offset.x, y: offset.y, scale: 0.96 }
                  }
                  transition={
                    useInstant
                      ? { duration: 0.01 }
                      : useFadeOnly
                        ? { duration: 0.12, ease: 'easeOut' }
                        : { type: 'spring', stiffness: 460, damping: 28, mass: 0.65 }
                  }
                  className={cn(
                    'flex items-center gap-2 whitespace-nowrap text-[11.5px] font-medium tracking-[-0.01em]',
                    shapeClass,
                    styleSpec.className,
                  )}
                  style={styleSpec.inlineStyle}
                >
                  <span>{label}</span>
                  {shortcut && (
                    <kbd
                      className={cn(
                        'rounded border px-1.5 py-0.5 font-mono text-[9.5px] leading-none',
                        styleSpec.kbdClass,
                      )}
                    >
                      {shortcut}
                    </kbd>
                  )}
                </motion.div>
              </div>
            )}
          </AnimatePresence>,
          document.body,
        )}
    </div>
  )
}
