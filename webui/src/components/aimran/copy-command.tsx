'use client'

import { useEffect, useRef, useState } from 'react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { Copy } from 'lucide-react'
import { cn } from '@/lib/utils'

export interface CopyCommandProps {
  /** Shell or CLI command text to display and copy. */
  command?: string
  /** Highlight sweep & checkmark stroke accent color. */
  accent?: string
  /** Optional prefix glyph shown before the command (default "$"). */
  prefix?: string
  /** Stretch to fill parent container width instead of 19rem card width. */
  fullWidth?: boolean
  className?: string
}

/**
 * AIMRAN Microinteractions — Copy Command Button.
 * On copy, a selection highlight sweeps across the command and the copy icon
 * morphs into a self-drawing SVG checkmark.
 */
export default function CopyCommand({
  command = 'npm i @aimran/ui',
  accent = '#22c55e',
  prefix = '$',
  fullWidth = false,
  className,
}: CopyCommandProps) {
  const [copied, setCopied] = useState(false)
  const [sweep, setSweep] = useState(0)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const reduce = useReducedMotion() ?? false

  useEffect(() => () => clearTimeout(timer.current), [])

  const copy = () => {
    try {
      navigator.clipboard?.writeText(command).catch(() => {})
    } catch {
      // Clipboard unavailable: animation still runs cleanly.
    }
    setCopied(true)
    setSweep((s) => s + 1)
    clearTimeout(timer.current)
    timer.current = setTimeout(() => setCopied(false), 1800)
  }

  return (
    <div
      className={cn(
        'flex w-full min-w-0 items-center gap-2.5 rounded-[var(--radius-lg)] border border-[var(--border)] bg-[var(--card)] py-1.5 pl-4 pr-1.5 font-mono text-[13px] text-[var(--fg)] shadow-[0_1px_2px_rgba(0,0,0,0.06),0_8px_24px_rgba(0,0,0,0.14)]',
        !fullWidth && 'max-w-[19rem]',
        className,
      )}
    >
      <span className="shrink-0 select-none text-[var(--muted)]" aria-hidden="true">
        {prefix}
      </span>
      <span className="relative min-w-0 flex-1 overflow-hidden">
        {sweep > 0 && !reduce && (
          <motion.span
            key={sweep}
            aria-hidden="true"
            className="pointer-events-none absolute -inset-x-1 -inset-y-0.5 origin-left rounded-[5px]"
            style={{ backgroundColor: accent }}
            initial={{ scaleX: 0, opacity: 0.24 }}
            animate={{ scaleX: 1, opacity: [0.24, 0.24, 0] }}
            transition={{
              scaleX: { duration: 0.34, ease: [0.22, 1, 0.36, 1] },
              opacity: { duration: 1.1, times: [0, 0.55, 1] },
            }}
          />
        )}
        <span className="relative block truncate" title={command}>
          {command}
        </span>
      </span>
      <motion.button
        type="button"
        onClick={copy}
        aria-label={copied ? 'Copied' : 'Copy command'}
        whileTap={reduce ? undefined : { scale: 0.88 }}
        transition={{ type: 'spring', stiffness: 600, damping: 30 }}
        className="relative grid h-11 w-11 shrink-0 cursor-pointer place-items-center rounded-[var(--radius)] text-[var(--muted)] outline-none transition-colors hover:bg-[var(--surface)] hover:text-[var(--fg)] focus-visible:ring-2 focus-visible:ring-[var(--accent)]/60 sm:h-9 sm:w-9"
      >
        <motion.span
          aria-hidden="true"
          className="absolute inset-0 rounded-[var(--radius)]"
          style={{ backgroundColor: accent }}
          initial={false}
          animate={{ opacity: copied ? 0.14 : 0 }}
          transition={{ duration: reduce ? 0.01 : 0.2 }}
        />
        <AnimatePresence mode="popLayout" initial={false}>
          {copied ? (
            <motion.svg
              key="check"
              viewBox="0 0 24 24"
              className="relative h-[18px] w-[18px]"
              fill="none"
              stroke={accent}
              strokeWidth={2.25}
              strokeLinecap="round"
              strokeLinejoin="round"
              initial={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.5 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.5, filter: 'blur(2px)' }}
              transition={{ type: 'spring', stiffness: 500, damping: 26 }}
            >
              <motion.path
                d="M5 12.5l4.5 4.5L19 7.5"
                initial={{ pathLength: reduce ? 1 : 0 }}
                animate={{ pathLength: 1 }}
                transition={{
                  duration: reduce ? 0.01 : 0.3,
                  delay: reduce ? 0 : 0.05,
                  ease: [0.65, 0, 0.35, 1],
                }}
              />
            </motion.svg>
          ) : (
            <motion.span
              key="copy"
              className="relative grid place-items-center"
              initial={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.5, filter: 'blur(2px)' }}
              animate={{ opacity: 1, scale: 1, filter: 'blur(0px)' }}
              exit={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.5, filter: 'blur(2px)' }}
              transition={{ type: 'spring', stiffness: 500, damping: 30 }}
            >
              <Copy className="h-[18px] w-[18px]" strokeWidth={1.5} />
            </motion.span>
          )}
        </AnimatePresence>
      </motion.button>
      <span className="sr-only" aria-live="polite">
        {copied ? 'Copied to clipboard' : ''}
      </span>
    </div>
  )
}

export { CopyCommand }
