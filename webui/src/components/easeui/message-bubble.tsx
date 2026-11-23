'use client'
// easeui.dev/components/agents/message-bubble (adapted to Zenko 60/30/10 CSS tokens)

import { motion, useReducedMotion } from 'motion/react'
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

export type MessageBubbleTone = 'neutral' | 'accent'
export type MessageBubbleAlign = 'start' | 'end'

export interface MessageBubbleProps {
  /** Which side the bubble sits on, matching who sent it. Default "start". */
  align?: MessageBubbleAlign
  /** Visual treatment. Default "neutral". */
  tone?: MessageBubbleTone
  children: ReactNode
  className?: string
}

const TONE_CLASS: Record<MessageBubbleTone, string> = {
  neutral: 'bg-[var(--card)] text-[var(--fg)] shadow-[0_0_0_1px_var(--border)]',
  accent: 'bg-[var(--accent)] text-[var(--primary-fg)]',
}

const EASE = [0.23, 1, 0.32, 1] as const

/** A chat message surface with a speech-bubble tail that pops in from the side it belongs to. */
export function MessageBubble({
  align = 'start',
  tone = 'neutral',
  children,
  className,
}: MessageBubbleProps) {
  const reduce = useReducedMotion() ?? false
  const fromX = align === 'end' ? 10 : -10

  return (
    <motion.div
      data-slot="message-bubble-content"
      initial={reduce ? { opacity: 0 } : { opacity: 0, x: fromX, scale: 0.97 }}
      animate={{ opacity: 1, x: 0, scale: 1 }}
      transition={{ duration: reduce ? 0.1 : 0.2, ease: EASE }}
      className={cn(
        'text-pretty rounded-[var(--radius-lg)] px-3.5 py-2.5 text-[14.5px] leading-relaxed',
        align === 'end' ? 'rounded-br-[4px]' : 'rounded-bl-[4px]',
        TONE_CLASS[tone],
        className,
      )}
    >
      {children}
    </motion.div>
  )
}

export default MessageBubble
