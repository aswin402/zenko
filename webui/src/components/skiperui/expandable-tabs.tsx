'use client'

import type { ReactNode } from 'react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { cn } from '@/lib/utils'

export interface ExpandableTabItem {
  id: string
  label: string
  icon?: ReactNode
  badge?: string | number
}

export interface ExpandableTabsProps {
  items: readonly ExpandableTabItem[]
  value: string
  onValueChange: (id: string) => void
  className?: string
}

const EXPAND_SPRING = { type: 'spring', bounce: 0.14, duration: 0.34 } as const

/**
 * Skiper96-inspired Expandable Tabs Navigation (`@skiper-ui/skiper96`).
 * Active tab expands smoothly via spring physics to reveal its label beside the icon;
 * inactive tabs collapse to compact pill triggers strictly inheriting Zenko's global theme.
 */
export function ExpandableTabs({
  items,
  value,
  onValueChange,
  className,
}: ExpandableTabsProps) {
  const reduce = useReducedMotion()

  return (
    <div
      role="tablist"
      className={cn(
        'inline-flex flex-wrap items-center gap-1 rounded-2xl border border-[var(--border)] bg-[var(--surface)] p-1 shadow-xs',
        className,
      )}
    >
      {items.map((item) => {
        const active = item.id === value
        return (
          <motion.button
            key={item.id}
            type="button"
            role="tab"
            aria-selected={active}
            layout={reduce ? false : 'position'}
            onClick={() => onValueChange(item.id)}
            transition={reduce ? { duration: 0 } : EXPAND_SPRING}
            className={cn(
              'relative inline-flex cursor-pointer items-center gap-1.5 rounded-xl px-2.5 py-1.5 text-[12px] font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]/40',
              active
                ? 'bg-[var(--card)] text-[var(--accent)] shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--accent)_38%,var(--border))]'
                : 'text-[var(--muted)] hover:bg-[var(--card)]/60 hover:text-[var(--fg)]',
            )}
          >
            {item.icon && <span className="inline-flex shrink-0 items-center">{item.icon}</span>}
            <AnimatePresence initial={false} mode="popLayout">
              {(active || !item.icon) && (
                <motion.span
                  key="label"
                  initial={reduce ? { opacity: 1 } : { opacity: 0, width: 0 }}
                  animate={{ opacity: 1, width: 'auto' }}
                  exit={reduce ? { opacity: 0 } : { opacity: 0, width: 0 }}
                  transition={reduce ? { duration: 0 } : EXPAND_SPRING}
                  className="overflow-hidden whitespace-nowrap"
                >
                  {item.label}
                </motion.span>
              )}
            </AnimatePresence>
            {item.badge !== undefined && active && (
              <span className="rounded-full bg-[var(--accent)]/15 px-1.5 py-px font-mono text-[10px] text-[var(--accent)]">
                {item.badge}
              </span>
            )}
          </motion.button>
        )
      })}
    </div>
  )
}
