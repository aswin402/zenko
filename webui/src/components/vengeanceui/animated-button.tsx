'use client'

import { forwardRef, type ReactNode } from 'react'
import { motion, useReducedMotion, type HTMLMotionProps } from 'motion/react'
import { cn } from '@/lib/utils'

export interface AnimatedButtonProps extends Omit<HTMLMotionProps<'button'>, 'children'> {
  children: ReactNode
}

/**
 * VengeanceUI Animated Button (`@vengeanceui/animated-button`).
 * Smooth traveling diagonal sheen sweep with border luminance & tactile spring tap,
 * strictly inheriting Zenko's global theme variables.
 */
export const AnimatedButton = forwardRef<HTMLButtonElement, AnimatedButtonProps>(
  function AnimatedButton({ children, className, disabled, ...props }, ref) {
    const reduce = useReducedMotion() ?? false
    return (
      <motion.button
        ref={ref}
        type="button"
        disabled={disabled}
        whileTap={reduce || disabled ? undefined : { scale: 0.97 }}
        transition={{ type: 'spring', stiffness: 420, damping: 24 }}
        className={cn(
          'z-btn-vengeance-animated group/animbtn relative inline-flex cursor-pointer items-center justify-center gap-2 overflow-hidden rounded-[var(--radius)] border px-4 py-2 text-[13px] font-medium transition-all duration-200 disabled:cursor-not-allowed disabled:opacity-45',
          'border-[color-mix(in_srgb,var(--primary)_62%,var(--border))] bg-[var(--primary)] text-[var(--primary-fg)] shadow-[0_6px_22px_color-mix(in_srgb,var(--primary)_28%,transparent)] hover:brightness-105',
          className,
        )}
        {...props}
      >
        <span
          aria-hidden="true"
          className="zenko-animated-btn-sheen pointer-events-none absolute inset-0"
        />
        <span className="relative z-10 inline-flex items-center gap-2">{children}</span>
      </motion.button>
    )
  },
)

export default AnimatedButton
