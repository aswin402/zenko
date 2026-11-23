'use client'

/**
 * KokonutUI — AI Inline Citations & Expandable Sources Drawer
 * (adapted strictly to Zenko 60/30/10 theme tokens, zero regex).
 */

import { ArrowUpRight, ChevronDown } from 'lucide-react'
import { AnimatePresence, MotionConfig, motion } from 'motion/react'
import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import { cn } from '@/lib/utils'

export interface KokonutCitationSource {
  id: string
  title: string
  url: string
  site?: string
  snippet?: string
}

export interface KokonutAnswerPart {
  text: string
  cite?: string[]
}

export interface KokonutAICitationsProps {
  /** Optional inline cited prose parts. When omitted and `sources` is passed, renders the compact monogram sources bar & drawer. */
  parts?: KokonutAnswerPart[]
  sources: KokonutCitationSource[]
  className?: string
}

const EASE_OUT = [0.23, 1, 0.32, 1] as const
const CARD_WIDTH = 288
const OPEN_DELAY_MS = 120
const CLOSE_DELAY_MS = 90
const WARM_MS = 400
const CARD_GUESS = 200

function hostOf(url: string): string {
  try {
    const h = new URL(url).hostname
    return h.startsWith('www.') ? h.slice(4) : h
  } catch {
    return url
  }
}

function siteOf(source: KokonutCitationSource): string {
  return source.site ?? hostOf(source.url)
}

/** Deterministic last-word split (zero regex) so citation chips never wrap alone onto a new line. */
function splitLastWord(text: string): { head: string; tail: string } {
  const trimmed = text.trimEnd()
  if (!trimmed) return { head: text, tail: '' }
  const spaceIdx = trimmed.lastIndexOf(' ')
  if (spaceIdx < 0) return { head: '', tail: trimmed }
  return { head: trimmed.slice(0, spaceIdx + 1), tail: trimmed.slice(spaceIdx + 1) }
}

function Monogram({
  source,
  index,
  className,
}: {
  source: KokonutCitationSource
  index: number
  className?: string
}) {
  const opacity = 16 + (index % 3) * 8
  return (
    <span
      aria-hidden="true"
      className={cn(
        'flex size-4 shrink-0 items-center justify-center rounded-[4px] font-mono text-[9px] font-semibold uppercase',
        className,
      )}
      style={{
        color: 'var(--accent)',
        backgroundColor: `color-mix(in srgb, var(--accent) ${opacity}%, var(--surface))`,
      }}
    >
      {siteOf(source).charAt(0)}
    </span>
  )
}

interface Preview {
  part: number
  x: number
  y: number
  below: boolean
}

function usePreview() {
  const [preview, setPreview] = useState<Preview | null>(null)
  const [warm, setWarm] = useState(false)
  const openTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lastClosed = useRef(0)

  const clear = useCallback(() => {
    if (openTimer.current) clearTimeout(openTimer.current)
    if (closeTimer.current) clearTimeout(closeTimer.current)
  }, [])

  const show = useCallback(
    (next: Preview, immediate = false) => {
      clear()
      const isWarm = preview !== null || Date.now() - lastClosed.current < WARM_MS
      if (immediate || isWarm) {
        setWarm(preview !== null)
        setPreview(next)
        return
      }
      openTimer.current = setTimeout(() => {
        setWarm(false)
        setPreview(next)
      }, OPEN_DELAY_MS)
    },
    [preview, clear],
  )

  const hide = useCallback(
    (immediate = false) => {
      clear()
      const close = () => {
        lastClosed.current = Date.now()
        setPreview(null)
      }
      if (immediate) {
        close()
        return
      }
      closeTimer.current = setTimeout(close, CLOSE_DELAY_MS)
    },
    [clear],
  )

  useEffect(() => clear, [clear])
  return { preview, warm, show, hide }
}

export default function KokonutAICitations({
  parts,
  sources,
  className,
}: KokonutAICitationsProps) {
  const rootRef = useRef<HTMLDivElement>(null)
  const { preview, warm, show, hide } = usePreview()
  const [hoveredSource, setHoveredSource] = useState<string | null>(null)
  const [listOpen, setListOpen] = useState(false)
  const previewId = useId()
  const listId = useId()

  const byId = new Map(sources.map((s) => [s.id, s]))
  const cited: KokonutCitationSource[] = parts
    ? (() => {
        const order: string[] = []
        for (const part of parts) {
          for (const id of part.cite ?? []) {
            if (!order.includes(id) && byId.has(id)) order.push(id)
          }
        }
        return order
          .map((id) => byId.get(id))
          .filter((s): s is KokonutCitationSource => Boolean(s))
      })()
    : sources

  const cardRef = useRef<HTMLDivElement>(null)
  const cardHeight = useRef(CARD_GUESS)
  const place = (index: number, chip: HTMLElement): Preview => {
    const root = rootRef.current?.getBoundingClientRect()
    const rect = chip.getBoundingClientRect()
    const width = Math.min(CARD_WIDTH, root?.width ?? CARD_WIDTH)
    const left = rect.left - (root?.left ?? 0) + rect.width / 2 - width / 2
    const max = (root?.width ?? width) - width
    const below = rect.top < cardHeight.current + 16
    return {
      part: index,
      x: Math.min(Math.max(0, left), Math.max(0, max)),
      y: below ? rect.bottom - (root?.top ?? 0) + 8 : rect.top - (root?.top ?? 0) - 8,
      below,
    }
  }

  useLayoutEffect(() => {
    if (preview && cardRef.current) {
      cardHeight.current = cardRef.current.offsetHeight
    }
  })

  const active = preview && parts ? parts[preview.part] : null
  const activeSources = (active?.cite ?? [])
    .map((id) => byId.get(id))
    .filter((s): s is KokonutCitationSource => Boolean(s))

  useEffect(() => {
    if (!preview) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') hide(true)
    }
    const onPointer = (event: PointerEvent) => {
      if (!(event.target instanceof Element && event.target.closest('[data-citation-chip]'))) {
        hide(true)
      }
    }
    document.addEventListener('keydown', onKey)
    document.addEventListener('pointerdown', onPointer)
    return () => {
      document.removeEventListener('keydown', onKey)
      document.removeEventListener('pointerdown', onPointer)
    }
  }, [preview, hide])

  if (cited.length === 0 && !parts?.length) return null

  return (
    <MotionConfig reducedMotion="user">
      <div className={cn('relative', parts ? 'w-full' : 'inline-block', className)} ref={rootRef}>
        {parts && parts.length > 0 && (
          <p className="text-pretty text-[14.5px] leading-7 text-[var(--fg)]/92">
            {parts.map((part, index) => {
              const partSources = (part.cite ?? [])
                .map((id) => byId.get(id))
                .filter((s): s is KokonutCitationSource => Boolean(s))
              if (partSources.length === 0) {
                return <span key={index}>{part.text}</span>
              }
              const first = partSources[0]
              const isActive = preview?.part === index
              const isLinked = partSources.some((s) => s.id === hoveredSource)
              const claim = cn(
                'rounded-[3px] underline-offset-4 transition-[background-color,text-decoration-color] duration-150',
                isActive || isLinked ? 'underline' : '',
              )
              const claimStyle =
                isActive || isLinked
                  ? {
                      backgroundColor: 'color-mix(in srgb, var(--accent) 16%, transparent)',
                      textDecorationColor: 'color-mix(in srgb, var(--accent) 70%, transparent)',
                    }
                  : undefined
              const { head, tail } = splitLastWord(part.text)
              return (
                <span key={index}>
                  {head && (
                    <span className={claim} style={claimStyle}>
                      {head}
                    </span>
                  )}
                  <span className="whitespace-nowrap">
                    {tail && (
                      <span className={claim} style={claimStyle}>
                        {tail}
                      </span>
                    )}
                    {'\u00a0'}
                    <button
                      aria-describedby={isActive ? previewId : undefined}
                      aria-label={`Sources: ${partSources.map(siteOf).join(', ')}`}
                      className={cn(
                        'inline-flex h-5 translate-y-[-1px] cursor-pointer items-center gap-1 rounded-full py-0 pr-1.5 pl-0.5 align-middle text-[11px] font-medium outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-[var(--accent)]/50',
                        isActive || isLinked
                          ? 'bg-[var(--primary)] text-[var(--primary-fg)]'
                          : 'border border-[var(--border)] bg-[var(--card)] text-[var(--muted)] hover:border-[var(--accent)]/45 hover:text-[var(--fg)]',
                      )}
                      data-citation-chip=""
                      onBlur={() => hide(true)}
                      onClick={(e) => show(place(index, e.currentTarget), true)}
                      onFocus={(e) => show(place(index, e.currentTarget), true)}
                      onPointerEnter={(e) => {
                        if (e.pointerType === 'mouse') show(place(index, e.currentTarget))
                      }}
                      onPointerLeave={(e) => {
                        if (e.pointerType === 'mouse') hide()
                      }}
                      type="button"
                    >
                      <Monogram source={first} index={index} className="size-4 rounded-full" />
                      {siteOf(first)}
                      {partSources.length > 1 && (
                        <span className="opacity-65">+{partSources.length - 1}</span>
                      )}
                    </button>
                  </span>
                </span>
              )
            })}
          </p>
        )}

        <AnimatePresence>
          {preview && activeSources.length > 0 && (
            <motion.div
              animate={{ opacity: 1, scale: 1, x: preview.x, y: preview.y }}
              className="pointer-events-none absolute top-0 left-0 z-20"
              exit={{ opacity: 0, scale: 0.97, transition: { duration: 0.1, ease: EASE_OUT } }}
              id={previewId}
              initial={{ opacity: 0, scale: 0.97, x: preview.x, y: preview.y }}
              role="tooltip"
              style={{
                transformOrigin: '50% 0%',
                width: Math.min(CARD_WIDTH, rootRef.current?.offsetWidth ?? CARD_WIDTH),
              }}
              transition={{
                opacity: { duration: 0.14, ease: EASE_OUT },
                scale: { duration: 0.14, ease: EASE_OUT },
                x: warm ? { duration: 0.22, ease: EASE_OUT } : { duration: 0 },
                y: warm ? { duration: 0.22, ease: EASE_OUT } : { duration: 0 },
              }}
            >
              <div
                className={cn(
                  'rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)] p-1 shadow-[0_12px_28px_rgba(0,0,0,0.34)]',
                  !preview.below && '-translate-y-full',
                )}
                ref={cardRef}
              >
                {activeSources.map((source, idx) => (
                  <div className="rounded-[var(--radius-sm)] p-2.5" key={source.id}>
                    <div className="flex items-center gap-1.5 text-xs text-[var(--muted)]">
                      <Monogram source={source} index={idx} />
                      <span className="truncate font-mono text-[11px]">{hostOf(source.url)}</span>
                    </div>
                    <p className="mt-1.5 text-[13px] font-medium leading-snug text-[var(--fg)]">
                      {source.title}
                    </p>
                    {source.snippet && (
                      <p className="mt-1 line-clamp-3 text-xs leading-relaxed text-[var(--muted)]">
                        {source.snippet}
                      </p>
                    )}
                  </div>
                ))}
              </div>
            </motion.div>
          )}
        </AnimatePresence>

        {cited.length > 0 && (
          <div className={parts ? 'mt-3' : undefined}>
            <button
              aria-controls={listId}
              aria-expanded={listOpen}
              className="group/sources inline-flex cursor-pointer items-center gap-2 rounded-full border border-[var(--border)]/75 bg-[var(--card)]/70 py-1 pr-2.5 pl-1 text-[11.5px] text-[var(--muted)] outline-none transition-colors hover:border-[var(--accent)]/45 hover:text-[var(--fg)] focus-visible:ring-2 focus-visible:ring-[var(--accent)]/50"
              onClick={() => setListOpen((open) => !open)}
              type="button"
            >
              <span className="flex -space-x-1">
                {cited.slice(0, 4).map((source, idx) => (
                  <Monogram
                    key={source.id}
                    source={source}
                    index={idx}
                    className="size-4.5 rounded-full ring-2 ring-[var(--surface)]"
                  />
                ))}
              </span>
              <span>
                {cited.length} {cited.length === 1 ? 'source' : 'sources'}
              </span>
              <ChevronDown
                aria-hidden="true"
                className={cn(
                  'size-3.5 text-[var(--muted)] transition-transform duration-200',
                  listOpen && 'rotate-180',
                )}
              />
            </button>
            <AnimatePresence initial={false}>
              {listOpen && (
                <motion.ol
                  animate={{ height: 'auto', opacity: 1 }}
                  className="mt-1.5 overflow-hidden rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)] p-1 shadow-lg"
                  exit={{ height: 0, opacity: 0 }}
                  id={listId}
                  initial={{ height: 0, opacity: 0 }}
                  transition={{ duration: 0.22, ease: EASE_OUT }}
                >
                  {cited.map((source, index) => (
                    <li key={source.id}>
                      <a
                        className="group/source flex items-center gap-2.5 rounded-[var(--radius-sm)] px-2 py-1.5 outline-none transition-colors hover:bg-[var(--card)] focus-visible:ring-2 focus-visible:ring-[var(--accent)]/50"
                        href={source.url}
                        onBlur={() => setHoveredSource(null)}
                        onFocus={() => setHoveredSource(source.id)}
                        onPointerEnter={() => setHoveredSource(source.id)}
                        onPointerLeave={() => setHoveredSource(null)}
                        rel="noopener noreferrer"
                        target="_blank"
                      >
                        <span className="w-3 shrink-0 text-right font-mono text-[11px] tabular-nums text-[var(--muted)]">
                          {index + 1}
                        </span>
                        <Monogram source={source} index={index} />
                        <span className="min-w-0 flex-1 truncate text-[12.5px] text-[var(--fg)]">
                          {source.title}
                        </span>
                        <span className="hidden shrink-0 font-mono text-[10.5px] text-[var(--muted)] sm:inline">
                          {hostOf(source.url)}
                        </span>
                        <ArrowUpRight
                          aria-hidden="true"
                          className="size-3.5 shrink-0 text-[var(--muted)] transition-colors group-hover/source:text-[var(--accent)]"
                        />
                      </a>
                    </li>
                  ))}
                </motion.ol>
              )}
            </AnimatePresence>
          </div>
        )}
      </div>
    </MotionConfig>
  )
}

export { KokonutAICitations }
