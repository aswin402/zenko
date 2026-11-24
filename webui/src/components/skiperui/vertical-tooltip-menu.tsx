'use client'

import { createContext, useContext, useState, type ReactNode } from 'react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { SPRING_LAYOUT, SPRING_PRESS } from '@/lib/ease'
import { cn } from '@/lib/utils'

interface VtooltipContextValue {
  hoveredId: string | null
  setHoveredId: (id: string | null) => void
  disableLabels: boolean
}

const VtooltipContext = createContext<VtooltipContextValue>({
  hoveredId: null,
  setHoveredId: () => {},
  disableLabels: false,
})

/**
 * Skiper98 Vertical Tooltip Menu (`@skiper-ui/skiper98`).
 * Smooth horizontal clip-path tooltip labels sliding out alongside a sleek vertical icon menu,
 * strictly inheriting Zenko's 60/30/10 CSS tokens.
 */
export function VtooltipRoot({
  children,
  className,
  disableLabels = false,
}: {
  children: ReactNode
  className?: string
  disableLabels?: boolean
}) {
  const [hoveredId, setHoveredId] = useState<string | null>(null)
  return (
    <VtooltipContext.Provider value={{ hoveredId, setHoveredId, disableLabels }}>
      <div
        onMouseLeave={() => setHoveredId(null)}
        className={cn(
          'flex flex-col items-center gap-1 rounded-[20px] border border-[var(--border)] bg-[var(--surface)]/95 p-1.5 shadow-[0_14px_38px_rgba(0,0,0,0.4)] backdrop-blur-md',
          className,
        )}
      >
        {children}
      </div>
    </VtooltipContext.Provider>
  )
}

export function VtooltipItem({
  id,
  children,
  className,
}: {
  id: string
  children: ReactNode
  className?: string
}) {
  const { setHoveredId } = useContext(VtooltipContext)
  return (
    <div
      onMouseEnter={() => setHoveredId(id)}
      onFocus={() => setHoveredId(id)}
      onBlur={() => setHoveredId(null)}
      className={cn('relative flex items-center', className)}
    >
      {children}
    </div>
  )
}

export function VtooltipContent({
  id,
  label,
  shortcut,
  side = 'left',
}: {
  id: string
  label: string
  shortcut?: string
  side?: 'left' | 'right'
}) {
  const { hoveredId, disableLabels } = useContext(VtooltipContext)
  const reduce = useReducedMotion() ?? false
  const open = !disableLabels && hoveredId === id

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          role="tooltip"
          initial={
            reduce
              ? { opacity: 0 }
              : {
                  opacity: 0,
                  x: side === 'left' ? 8 : -8,
                  clipPath: side === 'left' ? 'inset(0 0 0 100%)' : 'inset(0 100% 0 0)',
                }
          }
          animate={{
            opacity: 1,
            x: 0,
            clipPath: 'inset(0 0% 0 0%)',
          }}
          exit={
            reduce
              ? { opacity: 0 }
              : {
                  opacity: 0,
                  x: side === 'left' ? 6 : -6,
                  clipPath: side === 'left' ? 'inset(0 0 0 100%)' : 'inset(0 100% 0 0)',
                }
          }
          transition={{ duration: reduce ? 0.01 : 0.22, ease: [0.16, 1, 0.3, 1] }}
          className={cn(
            'pointer-events-none absolute top-1/2 z-40 flex -translate-y-1/2 items-center gap-1.5 whitespace-nowrap rounded-xl border border-[color-mix(in_srgb,var(--accent)_38%,var(--border))] bg-[var(--card)]/98 px-3 py-1.5 text-[11.5px] font-medium text-[var(--fg)] shadow-[0_10px_28px_rgba(0,0,0,0.36)]',
            side === 'left' ? 'right-full mr-2.5' : 'left-full ml-2.5',
          )}
        >
          <span>{label}</span>
          {shortcut && (
            <kbd className="rounded border border-[var(--border)] bg-[var(--surface)] px-1 py-0.5 font-mono text-[9.5px] leading-none text-[var(--muted)]">
              {shortcut}
            </kbd>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  )
}

export interface Skiper98DockItem {
  id: string
  label: string
  shortcut?: string
  icon: ReactNode
  active?: boolean
  badge?: number
  onClick?: () => void
}

export function Skiper98VerticalDock({
  items,
  activeIndicator = 'vermilion_pill',
  layoutId = 'skiper98-dock-pill',
  disableLabels = false,
  trailing,
  className,
}: {
  items: readonly Skiper98DockItem[]
  activeIndicator?: 'vermilion_pill' | 'solid_accent' | 'side_bar' | 'dot_indicator'
  layoutId?: string
  disableLabels?: boolean
  trailing?: ReactNode
  className?: string
}) {
  const reduce = useReducedMotion() ?? false

  const renderPill = () => {
    if (activeIndicator === 'solid_accent') {
      return (
        <motion.span
          layoutId={layoutId}
          transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
          className="pointer-events-none absolute inset-0 rounded-[12px]"
          style={{ backgroundColor: 'var(--primary)' }}
        />
      )
    }
    if (activeIndicator === 'side_bar') {
      return (
        <motion.span
          layoutId={layoutId}
          transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
          className="pointer-events-none absolute right-0 top-1.5 bottom-1.5 w-[2.5px] rounded-full"
          style={{ backgroundColor: 'var(--accent)' }}
        />
      )
    }
    if (activeIndicator === 'dot_indicator') {
      return (
        <motion.span
          layoutId={layoutId}
          transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
          className="pointer-events-none absolute bottom-1 h-1 w-1 rounded-full"
          style={{ backgroundColor: 'var(--accent)' }}
        />
      )
    }
    return (
      <motion.span
        layoutId={layoutId}
        transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
        className="pointer-events-none absolute inset-0 rounded-[12px] border"
        style={{
          backgroundColor: 'color-mix(in srgb, var(--accent) 14%, transparent)',
          borderColor: 'color-mix(in srgb, var(--accent) 52%, transparent)',
        }}
      />
    )
  }

  return (
    <VtooltipRoot disableLabels={disableLabels} className={className}>
      {items.map((item) => {
        const activeText =
          activeIndicator === 'solid_accent'
            ? 'text-[var(--primary-fg)]'
            : 'text-[var(--accent)]'
        return (
          <VtooltipItem key={item.id} id={item.id}>
            <motion.button
              type="button"
              aria-label={item.label}
              whileTap={reduce ? undefined : { scale: 0.92 }}
              transition={SPRING_PRESS}
              onClick={item.onClick}
              className={cn(
                'relative flex h-9 w-9 cursor-pointer items-center justify-center rounded-[12px] transition-colors',
                item.active
                  ? activeText
                  : 'text-[var(--muted)] hover:bg-[var(--card)]/85 hover:text-[var(--fg)]',
              )}
            >
              {item.active && renderPill()}
              <span className="relative z-10">{item.icon}</span>
              {item.badge !== undefined && item.badge > 0 && (
                <span
                  className="absolute right-1 top-1 z-20 flex h-3.5 min-w-[14px] items-center justify-center rounded-full px-1 font-mono text-[9px] font-semibold leading-none"
                  style={{
                    backgroundColor: 'var(--accent)',
                    color: 'var(--primary-fg)',
                  }}
                >
                  {item.badge > 9 ? '9+' : item.badge}
                </span>
              )}
            </motion.button>
            <VtooltipContent id={item.id} label={item.label} shortcut={item.shortcut} side="left" />
          </VtooltipItem>
        )
      })}
      {trailing}
    </VtooltipRoot>
  )
}
