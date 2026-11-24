'use client'

import { forwardRef, type ReactNode } from 'react'
import { motion, useReducedMotion, type HTMLMotionProps } from 'motion/react'
import { SPRING_PRESS } from '@/lib/ease'
import { cn } from '@/lib/utils'

export interface LiquidButtonProps extends Omit<HTMLMotionProps<'button'>, 'children'> {
  children: ReactNode
}

/**
 * VengeanceUI Liquid Button (`@vengeanceui/liquid-button`).
 * Glossy translucent rim with rising fluid accent wave fill on hover & spring press feedback,
 * strictly inheriting Zenko's 60/30/10 CSS tokens.
 */
export const LiquidButton = forwardRef<HTMLButtonElement, LiquidButtonProps>(function LiquidButton(
  { children, className, disabled, ...props },
  ref,
) {
  const reduce = useReducedMotion() ?? false
  return (
    <motion.button
      ref={ref}
      type="button"
      disabled={disabled}
      whileTap={reduce || disabled ? undefined : { scale: 0.96 }}
      transition={SPRING_PRESS}
      className={cn(
        'group/liquid relative inline-flex cursor-pointer items-center justify-center gap-2 overflow-hidden rounded-[var(--radius)] border px-4 py-2 text-[13px] font-medium transition-all duration-300 disabled:cursor-not-allowed disabled:opacity-45',
        'border-[color-mix(in_srgb,var(--primary)_55%,var(--border))] bg-[color-mix(in_srgb,var(--primary)_14%,var(--surface))] text-[var(--fg)] shadow-[inset_0_1px_0_rgba(255,255,255,0.16),0_6px_20px_color-mix(in_srgb,var(--primary)_18%,transparent)] hover:text-[var(--primary-fg)]',
        className,
      )}
      {...props}
    >
      {/* Rising liquid fill layer */}
      <span
        aria-hidden="true"
        className="pointer-events-none absolute inset-x-[-18%] bottom-0 h-[210%] translate-y-[102%] rounded-[42%] transition-transform duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] group-hover/liquid:translate-y-[12%]"
        style={{ backgroundColor: 'var(--primary)' }}
      />
      {/* Top specular glass glint */}
      <span
        aria-hidden="true"
        className="pointer-events-none absolute inset-x-2 top-0.5 h-px bg-gradient-to-r from-transparent via-white/40 to-transparent opacity-70"
      />
      <span className="relative z-10 inline-flex items-center gap-2">{children}</span>
    </motion.button>
  )
})

export default LiquidButton
