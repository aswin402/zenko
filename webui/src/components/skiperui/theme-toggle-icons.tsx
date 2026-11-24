'use client'

import { useId } from 'react'
import { motion, useReducedMotion } from 'motion/react'
import { cn } from '@/lib/utils'

export type ThemeToggleIconVariant =
  | 'default'
  | 'skiper4_eclipse'
  | 'skiper4_rays'
  | 'skiper4_horizon'
  | 'skiper4_classic_fill'
  | 'skiper4_minimal_dot'

const ICON_SPRING = { type: 'spring', stiffness: 360, damping: 24 } as const

/**
 * Skiper4 Animated SVG Theme Toggle Icons (`@skiper-ui/skiper4`, inspired by toggles.dev).
 * Pairs natively with Zenko's View Transition `ThemeToggle` / Skiper26 reveals.
 */
export function Skiper4ToggleIcon({
  variant,
  isDark,
  className,
}: {
  variant: Exclude<ThemeToggleIconVariant, 'default'>
  isDark: boolean
  className?: string
}) {
  const uid = useId().replace(/:/g, '')
  const reduce = useReducedMotion() ?? false
  const tr = reduce ? { duration: 0 } : ICON_SPRING

  if (variant === 'skiper4_eclipse') {
    const maskId = `skiper4-eclipse-${uid}`
    return (
      <svg
        viewBox="0 0 24 24"
        fill="none"
        aria-hidden="true"
        className={cn('size-4', className)}
      >
        <defs>
          <mask id={maskId}>
            <rect x="0" y="0" width="24" height="24" fill="white" />
            <motion.circle
              initial={false}
              animate={isDark ? { cx: 16.5, cy: 7.5, r: 6.5 } : { cx: 24, cy: 0, r: 0 }}
              transition={tr}
              fill="black"
            />
          </mask>
        </defs>
        <motion.circle
          cx="12"
          cy="12"
          initial={false}
          animate={{ r: isDark ? 7.5 : 6.2 }}
          transition={tr}
          fill="currentColor"
          mask={`url(#${maskId})`}
        />
      </svg>
    )
  }

  if (variant === 'skiper4_rays') {
    const maskId = `skiper4-rays-${uid}`
    return (
      <motion.svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
        aria-hidden="true"
        initial={false}
        animate={{ rotate: isDark ? 40 : 90 }}
        transition={tr}
        className={cn('size-4', className)}
      >
        <defs>
          <mask id={maskId}>
            <rect x="0" y="0" width="24" height="24" fill="white" />
            <motion.circle
              initial={false}
              animate={isDark ? { cx: 16, cy: 8, r: 6.5 } : { cx: 24, cy: 0, r: 0 }}
              transition={tr}
              fill="black"
            />
          </mask>
        </defs>
        <motion.circle
          cx="12"
          cy="12"
          initial={false}
          animate={{ r: isDark ? 7 : 4.5 }}
          transition={tr}
          fill="currentColor"
          stroke="none"
          mask={`url(#${maskId})`}
        />
        <motion.g
          initial={false}
          animate={{ opacity: isDark ? 0 : 1, scale: isDark ? 0.45 : 1 }}
          transition={tr}
          style={{ transformOrigin: '12px 12px' }}
        >
          <line x1="12" y1="2.5" x2="12" y2="4.5" />
          <line x1="12" y1="19.5" x2="12" y2="21.5" />
          <line x1="2.5" y1="12" x2="4.5" y2="12" />
          <line x1="19.5" y1="12" x2="21.5" y2="12" />
          <line x1="5.3" y1="5.3" x2="6.7" y2="6.7" />
          <line x1="17.3" y1="17.3" x2="18.7" y2="18.7" />
          <line x1="17.3" y1="6.7" x2="18.7" y2="5.3" />
          <line x1="5.3" y1="18.7" x2="6.7" y2="17.3" />
        </motion.g>
      </motion.svg>
    )
  }

  if (variant === 'skiper4_horizon') {
    return (
      <motion.svg
        viewBox="0 0 24 24"
        fill="none"
        aria-hidden="true"
        initial={false}
        animate={{ rotate: isDark ? 180 : 0 }}
        transition={tr}
        className={cn('size-4', className)}
      >
        <circle cx="12" cy="12" r="8.5" stroke="currentColor" strokeWidth="1.75" />
        <path d="M12 3.5 A8.5 8.5 0 0 1 12 20.5 Z" fill="currentColor" />
      </motion.svg>
    )
  }

  if (variant === 'skiper4_classic_fill') {
    const clipId = `skiper4-fill-${uid}`
    return (
      <svg
        viewBox="0 0 24 24"
        fill="none"
        aria-hidden="true"
        className={cn('size-4', className)}
      >
        <defs>
          <clipPath id={clipId}>
            <circle cx="12" cy="12" r="8.5" />
          </clipPath>
        </defs>
        <circle cx="12" cy="12" r="8.5" stroke="currentColor" strokeWidth="1.75" />
        <motion.rect
          x="3"
          width="18"
          height="18"
          fill="currentColor"
          clipPath={`url(#${clipId})`}
          initial={false}
          animate={{ y: isDark ? 3 : 21 }}
          transition={tr}
        />
      </svg>
    )
  }

  // skiper4_minimal_dot
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
      className={cn('size-4', className)}
    >
      <circle cx="12" cy="12" r="8" stroke="currentColor" strokeWidth="1.6" strokeOpacity="0.45" />
      <motion.circle
        cy="12"
        r="4"
        fill="var(--accent)"
        initial={false}
        animate={{ cx: isDark ? 9.5 : 14.5 }}
        transition={tr}
      />
    </svg>
  )
}
