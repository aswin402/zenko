import { cn } from '@/lib/utils'

interface ZenkoFoxLogoProps {
  className?: string
  size?: number
  status?: 'idle' | 'running' | 'waiting_approval' | 'error'
  variant?: string
  fillColor?: string
}

/**
 * Iconic vector 🦊 Zenko Fox mark & agent emblem.
 * Pure solid silhouette strictly styled in the active theme's dynamic `--accent` color
 * (or `--primary-fg` when seated on an accent-filled badge).
 */
export function ZenkoFoxLogo({
  className,
  size = 22,
  status = 'idle',
  variant = 'fox',
  fillColor,
}: ZenkoFoxLogoProps) {
  if (variant === 'hidden') return null

  const resolvedFill = fillColor || (status === 'error' ? 'var(--destructive)' : 'var(--accent)')

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 32 32"
      fill="none"
      className={cn(
        'shrink-0 transition-transform duration-200',
        status === 'running' && 'animate-pulse',
        className,
      )}
      aria-hidden="true"
    >
      {/* Clean solid Zenko fox silhouette — pure theme accent (no dark eye/nose dots) */}
      <path
        d="M16 8.2C19.2 8.2 22.8 5.2 25.4 3.2C26.2 2.6 27.1 3.1 27.2 4.1C27.4 7.4 27 10.6 26.5 12.5C28.5 14.3 29.6 16.5 29 18.8C28.1 22.1 22.4 26.6 17.4 28.4C16.5 28.7 15.5 28.7 14.6 28.4C9.6 26.6 3.9 22.1 3 18.8C2.4 16.5 3.5 14.3 5.5 12.5C5 10.6 4.6 7.4 4.8 4.1C4.9 3.1 5.8 2.6 6.6 3.2C9.2 5.2 12.8 8.2 16 8.2Z"
        fill={resolvedFill}
      />
    </svg>
  )
}
