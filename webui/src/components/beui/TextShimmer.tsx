import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

/** Real BeUI (beui.dev) streaming/thinking text shimmer. */
export function TextShimmer({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) {
  return <span className={cn('beui-text-shimmer inline-block font-medium', className)}>{children}</span>
}
