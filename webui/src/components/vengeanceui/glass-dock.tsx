'use client'

import React, { useState } from 'react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { SPRING_LAYOUT } from '@/lib/ease'
import { cn } from '@/lib/utils'
import { useZenkoTooltipConfig, ZenkoTooltipBadge } from '@/components/ui/Tooltip'

export type GlassDockIndicator = 'vermilion_pill' | 'solid_accent' | 'side_bar' | 'dot_indicator'

export interface GlassDockItem {
  id?: string
  title: string
  icon: React.ReactNode
  active?: boolean
  badge?: number
  onClick?: () => void
}

export interface GlassDockProps {
  items: readonly GlassDockItem[]
  orientation?: 'vertical' | 'horizontal'
  activeIndicator?: GlassDockIndicator
  layoutId?: string
  disableTooltip?: boolean
  className?: string
  dockClassName?: string
  trailing?: React.ReactNode
}

/**
 * VengeanceUI-inspired Frosted Glass Dock (`VengeanceUI/glass-dock`) ported cleanly
 * to `motion/react` with directional spring-gliding floating tooltip, reflective glass
 * specular rim, active indicator customization, and strict 60/30/10 Zenko theme inheritance.
 */
export const GlassDock = React.forwardRef<HTMLDivElement, GlassDockProps>(
  (
    {
      items,
      orientation = 'vertical',
      activeIndicator = 'vermilion_pill',
      layoutId = 'zenko-glass-dock-pill',
      disableTooltip = false,
      className,
      dockClassName,
      trailing,
    },
    ref,
  ) => {
    const reduce = useReducedMotion()
    const tooltipCfg = useZenkoTooltipConfig()
    const [hoveredIndex, setHoveredIndex] = useState<number | null>(null)
    const isVertical = orientation === 'vertical'
    const showTips = !disableTooltip && tooltipCfg.enabled

    const handleEnter = (index: number) => {
      setHoveredIndex(index)
    }

    const activeTextClass =
      activeIndicator === 'solid_accent' ? 'text-[var(--primary-fg)]' : 'text-[var(--accent)]'

    const renderIndicator = () => {
      if (activeIndicator === 'solid_accent') {
        return (
          <motion.span
            layoutId={layoutId}
            transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
            className="pointer-events-none absolute inset-0 rounded-[13px]"
            style={{ backgroundColor: 'var(--primary)' }}
          />
        )
      }
      if (activeIndicator === 'side_bar') {
        return (
          <motion.span
            layoutId={layoutId}
            transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
            className="pointer-events-none absolute bottom-1.5 right-0 top-1.5 w-[2.5px] rounded-full"
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
          className="pointer-events-none absolute inset-0 rounded-[13px] border"
          style={{
            backgroundColor: 'color-mix(in srgb, var(--accent) 16%, transparent)',
            borderColor: 'color-mix(in srgb, var(--accent) 48%, transparent)',
          }}
        />
      )
    }

    return (
      <div
        ref={ref}
        className={cn(
          'relative flex items-center justify-center',
          className,
        )}
      >
        <div
          className={cn(
            'relative flex items-center gap-1.5 rounded-[22px] border border-[var(--border)]/85 p-1.5 backdrop-blur-md',
            'bg-[color-mix(in_srgb,var(--card)_72%,transparent)]',
            'shadow-[0_14px_38px_rgba(0,0,0,0.4),inset_0_1px_0_rgba(255,255,255,0.08)]',
            isVertical ? 'flex-col' : 'flex-row px-3',
            dockClassName,
          )}
          onMouseLeave={() => {
            setHoveredIndex(null)
          }}
        >
          <AnimatePresence>
            {showTips && hoveredIndex !== null && items[hoveredIndex] && (
              <motion.div
                initial={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.92 }}
                animate={{
                  opacity: 1,
                  scale: 1,
                  ...(isVertical
                    ? { y: hoveredIndex * 42 + 6, x: 0 }
                    : { x: hoveredIndex * 42, y: -48 }),
                }}
                exit={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.92 }}
                transition={
                  reduce || tooltipCfg.motion === 'instant'
                    ? { duration: 0 }
                    : { type: 'spring', stiffness: 260, damping: 24 }
                }
                className={cn(
                  'pointer-events-none absolute z-30',
                  isVertical ? 'right-full top-0 mr-2.5' : 'left-2 top-0',
                )}
              >
                <ZenkoTooltipBadge
                  label={items[hoveredIndex].title}
                  className="overflow-hidden"
                />
              </motion.div>
            )}
          </AnimatePresence>

          {items.map((el, index) => {
            const isHovered = hoveredIndex === index
            return (
              <button
                key={el.id || el.title}
                type="button"
                aria-label={el.title}
                onMouseEnter={() => handleEnter(index)}
                onFocus={() => handleEnter(index)}
                onClick={el.onClick}
                className={cn(
                  'relative flex h-9 w-9 cursor-pointer items-center justify-center rounded-[13px] transition-colors',
                  el.active
                    ? activeTextClass
                    : 'text-[var(--muted)] hover:bg-[var(--surface)]/80 hover:text-[var(--fg)]',
                )}
              >
                {el.active && renderIndicator()}
                <motion.span
                  whileTap={reduce ? undefined : { scale: 0.92 }}
                  animate={
                    reduce
                      ? undefined
                      : {
                          scale: isHovered ? 1.12 : 1,
                          y: !isVertical && isHovered ? -2 : 0,
                          x: isVertical && isHovered ? -1.5 : 0,
                        }
                  }
                  transition={{ type: 'spring', stiffness: 320, damping: 24 }}
                  className="relative z-10 inline-flex items-center justify-center"
                >
                  {el.icon}
                  {typeof el.badge === 'number' && el.badge > 0 && (
                    <span className="absolute -right-1.5 -top-1.5 flex h-3.5 min-w-3.5 items-center justify-center rounded-full bg-[var(--accent)] px-1 font-mono text-[8.5px] font-bold text-[var(--primary-fg)]">
                      {el.badge}
                    </span>
                  )}
                </motion.span>
              </button>
            )
          })}

          {trailing}
        </div>
      </div>
    )
  },
)

GlassDock.displayName = 'GlassDock'
export default GlassDock
