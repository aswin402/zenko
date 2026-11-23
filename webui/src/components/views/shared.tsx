import { type ReactNode } from 'react'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'

export type SecondaryTabsVariant = 'pill' | 'segment' | 'underline'

export function ScrollPage({
  smoothScroll = true,
  children,
}: {
  smoothScroll?: boolean
  children: ReactNode
}) {
  return (
    <CompactScrollArea
      smooth={smoothScroll}
      className="min-h-0 flex-1"
      viewportClassName="px-6 py-8 pr-20 sm:px-10"
    >
      {children}
    </CompactScrollArea>
  )
}

/// Deterministic quote-aware argument splitter (zero regex, preserves quoted paths/headers).
export function splitShellArgs(raw: string): string[] {
  const out: string[] = []
  let cur = ''
  let quote: '"' | "'" | null = null
  let escaped = false
  for (let i = 0; i < raw.length; i++) {
    const ch = raw[i]
    if (escaped) {
      cur += ch
      escaped = false
      continue
    }
    if (ch === '\\' && quote !== "'") {
      escaped = true
      continue
    }
    if (quote) {
      if (ch === quote) quote = null
      else cur += ch
      continue
    }
    if (ch === '"' || ch === "'") {
      quote = ch
    } else if (ch === ' ' || ch === '\t' || ch === '\n' || ch === '\r') {
      if (cur.length > 0) {
        out.push(cur)
        cur = ''
      }
    } else {
      cur += ch
    }
  }
  if (cur.length > 0) out.push(cur)
  return out
}

export function InputField({
  label,
  value,
  onChange,
  placeholder,
  type = 'text',
  mono = false,
}: {
  label: string
  value: string
  onChange: (v: string) => void
  placeholder?: string
  type?: string
  mono?: boolean
}) {
  return (
    <label className="flex flex-col gap-1 text-[11.5px] text-[var(--muted)]">
      <span>{label}</span>
      <input
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className={`w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-1.5 text-[13px] text-[var(--fg)] focus:border-[var(--accent)] focus:outline-none ${mono ? 'font-mono' : ''}`}
      />
    </label>
  )
}
