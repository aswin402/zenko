'use client'

import React, { useState } from 'react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { cn } from '@/lib/utils'

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
  className?: string
  dockClassName?: string
  trailing?: React.ReactNode
}

/**
 * VengeanceUI-inspired Frosted Glass Dock (`VengeanceUI/glass-dock`) ported cleanly
 * to `motion/react` with directional spring-gliding floating tooltip, reflective glass
 * specular rim, and strict 60/30/10 Zenko theme inheritance.
 */
export const GlassDock = React.forwardRef<HTMLDivElement, GlassDockProps>(
  ({ items, orientation = 'vertical', className, dockClassName, trailing }, ref) => {
    const reduce = useReducedMotion()
    const [hoveredIndex, setHoveredIndex] = useState<number | null>(null)
    const [direction, setDirection] = useState<number>(0)
    const isVertical = orientation === 'vertical'

    const handleEnter = (index: number) => {
      if (hoveredIndex !== null && index !== hoveredIndex) {
        setDirection(index > hoveredIndex ? 1 : -1)
      }
      setHoveredIndex(index)
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
            setDirection(0)
          }}
        >
          <AnimatePresence>
            {hoveredIndex !== null && items[hoveredIndex] && (
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
                transition={reduce ? { duration: 0 } : { type: 'spring', stiffness: 260, damping: 24 }}
                className={cn(
                  'pointer-events-none absolute z-30',
                  isVertical ? 'right-full top-0 mr-2.5' : 'left-2 top-0',
                )}
              >
                <div className="flex min-w-[76px] items-center justify-center overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--card)]/96 px-3 py-1.5 text-[var(--fg)] shadow-lg">
                  <AnimatePresence mode="popLayout" custom={direction}>
                    <motion.span
                      key={items[hoveredIndex].title}
                      custom={direction}
                      initial={
                        reduce
                          ? { opacity: 0 }
                          : { y: direction > 0 ? 10 : -10, opacity: 0, filter: 'blur(3px)' }
                      }
                      animate={{ y: 0, opacity: 1, filter: 'blur(0px)' }}
                      exit={
                        reduce
                          ? { opacity: 0 }
                          : { y: direction > 0 ? -10 : 10, opacity: 0, filter: 'blur(3px)' }
                      }
                      transition={{ duration: 0.16, ease: 'easeOut' }}
                      className="whitespace-nowrap text-[11.5px] font-medium tracking-wide"
                    >
                      {items[hoveredIndex].title}
                    </motion.span>
                  </AnimatePresence>
                </div>
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
                    ? 'bg-[color-mix(in_srgb,var(--accent)_18%,var(--card))] text-[var(--accent)] shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--accent)_45%,transparent)]'
                    : 'text-[var(--muted)] hover:bg-[var(--surface)]/80 hover:text-[var(--fg)]',
                )}
              >
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
                  className="relative inline-flex items-center justify-center"
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
