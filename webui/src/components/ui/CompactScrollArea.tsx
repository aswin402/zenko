import {
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import Lenis from 'lenis'
import { useReducedMotion } from 'motion/react'
import { cn } from '@/lib/utils'

const MIN_THUMB_PX = 28
const MAX_THUMB_PX = 44
const TRACK_INSET_PX = 8
const EASE_SCROLL = (t: number) => Math.min(1, 1.001 - 2 ** (-10 * t))

/**
 * Universal compact overlay scroll container with a short, slightly wider rounded pill thumb
 * tinted by the active global theme (`--scrollbar-thumb` / `--accent`).
 * Hides native browser scrollbars, supports smooth pointer dragging, optional Lenis smooth wheel
 * scrolling (`smooth`), auto-height bounded containers (`autoHeight`), and streaming bottom-stick (`stickToBottom`).
 */
export function CompactScrollArea({
  children,
  className,
  viewportClassName,
  smooth = false,
  autoHeight = false,
  stickToBottom = false,
}: {
  children: ReactNode
  className?: string
  viewportClassName?: string
  smooth?: boolean
  autoHeight?: boolean
  stickToBottom?: boolean
}) {
  const reduceMotion = useReducedMotion() ?? false
  const viewportRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)
  const lenisRef = useRef<Lenis | null>(null)
  const pinnedBottomRef = useRef(true)
  const [metrics, setMetrics] = useState({
    overflow: false,
    thumbHeight: 36,
    thumbTop: TRACK_INSET_PX,
  })
  const [dragging, setDragging] = useState(false)
  const [scrolling, setScrolling] = useState(false)
  const hideTimerRef = useRef<number | undefined>(undefined)
  const dragStateRef = useRef<{
    pointerId: number
    startY: number
    startScrollTop: number
    maxScroll: number
    travel: number
  } | null>(null)

  const syncMetrics = useCallback(() => {
    const el = viewportRef.current
    if (!el) return
    const { scrollTop, scrollHeight, clientHeight } = el
    const maxScroll = Math.max(0, scrollHeight - clientHeight)
    if (stickToBottom && !dragStateRef.current) {
      pinnedBottomRef.current = maxScroll - scrollTop < 28
    }
    if (maxScroll <= 1 || clientHeight <= TRACK_INSET_PX * 2) {
      setMetrics((prev) =>
        prev.overflow ? { overflow: false, thumbHeight: 36, thumbTop: TRACK_INSET_PX } : prev,
      )
      return
    }
    const trackHeight = Math.max(24, clientHeight - TRACK_INSET_PX * 2)
    const proportional = Math.round((clientHeight / scrollHeight) * trackHeight)
    const thumbHeight = Math.max(MIN_THUMB_PX, Math.min(MAX_THUMB_PX, proportional))
    const travel = Math.max(1, trackHeight - thumbHeight)
    const thumbTop = TRACK_INSET_PX + (scrollTop / maxScroll) * travel
    setMetrics((prev) =>
      prev.overflow &&
      prev.thumbHeight === thumbHeight &&
      Math.abs(prev.thumbTop - thumbTop) < 0.5
        ? prev
        : { overflow: true, thumbHeight, thumbTop },
    )
  }, [stickToBottom])

  // Optional Lenis smooth wheel scrolling bound directly to viewportRef (zero native scrollbar!)
  useEffect(() => {
    if (!smooth || reduceMotion) {
      lenisRef.current = null
      return
    }
    const wrapper = viewportRef.current
    const content = contentRef.current
    if (!wrapper || !content) return

    const lenis = new Lenis({
      wrapper,
      content,
      lerp: 0.1,
      duration: 1.1,
      smoothWheel: true,
      syncTouch: false,
      easing: EASE_SCROLL,
    })
    lenisRef.current = lenis

    let rafId = 0
    const tick = (time: number) => {
      lenis.raf(time)
      rafId = requestAnimationFrame(tick)
    }
    rafId = requestAnimationFrame(tick)

    return () => {
      cancelAnimationFrame(rafId)
      lenis.destroy()
      lenisRef.current = null
    }
  }, [smooth, reduceMotion])

  useLayoutEffect(() => {
    syncMetrics()
    const el = viewportRef.current
    const content = contentRef.current
    if (!el) return

    const onChange = () => {
      if (stickToBottom && pinnedBottomRef.current && !dragStateRef.current) {
        el.scrollTop = el.scrollHeight
      }
      lenisRef.current?.resize()
      syncMetrics()
    }

    const ro = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(onChange) : null
    ro?.observe(el)
    if (content) ro?.observe(content)

    const mo = typeof MutationObserver !== 'undefined' ? new MutationObserver(onChange) : null
    mo?.observe(content ?? el, { childList: true, subtree: true, characterData: stickToBottom })

    return () => {
      ro?.disconnect()
      mo?.disconnect()
    }
  }, [syncMetrics, stickToBottom])

  useEffect(
    () => () => {
      if (hideTimerRef.current) window.clearTimeout(hideTimerRef.current)
    },
    [],
  )

  const handleScroll = () => {
    syncMetrics()
    setScrolling(true)
    if (hideTimerRef.current) window.clearTimeout(hideTimerRef.current)
    hideTimerRef.current = window.setTimeout(() => setScrolling(false), 900)
  }

  const scrollToOffset = (top: number, immediate: boolean) => {
    const el = viewportRef.current
    if (!el) return
    const clamped = Math.max(0, Math.min(top, Math.max(0, el.scrollHeight - el.clientHeight)))
    if (lenisRef.current) {
      lenisRef.current.scrollTo(clamped, { immediate })
    } else if (immediate) {
      el.scrollTop = clamped
    } else {
      el.scrollTo({ top: clamped, behavior: 'smooth' })
    }
  }

  return (
    <div className={cn('group/scroll relative min-h-0 overflow-hidden', className)}>
      <div
        ref={viewportRef}
        onScroll={handleScroll}
        className={cn(
          'scrollbar-hide w-full overflow-y-auto overscroll-contain',
          !autoHeight && 'h-full',
          viewportClassName,
        )}
      >
        <div ref={contentRef} className="min-w-0">
          {children}
        </div>
      </div>

      {metrics.overflow && (
        <div
          aria-hidden="true"
          onPointerDown={(e) => {
            if (e.target !== e.currentTarget) return
            const el = viewportRef.current
            if (!el) return
            const rect = e.currentTarget.getBoundingClientRect()
            const clickY = e.clientY - rect.top - TRACK_INSET_PX - metrics.thumbHeight / 2
            const trackHeight = Math.max(24, el.clientHeight - TRACK_INSET_PX * 2)
            const travel = Math.max(1, trackHeight - metrics.thumbHeight)
            const ratio = Math.max(0, Math.min(1, clickY / travel))
            scrollToOffset(ratio * (el.scrollHeight - el.clientHeight), false)
          }}
          className="absolute inset-y-0 right-0 z-10 w-2.5 select-none"
        >
          <div
            onPointerDown={(e) => {
              e.preventDefault()
              e.stopPropagation()
              const el = viewportRef.current
              if (!el) return
              const trackHeight = Math.max(24, el.clientHeight - TRACK_INSET_PX * 2)
              const travel = Math.max(1, trackHeight - metrics.thumbHeight)
              dragStateRef.current = {
                pointerId: e.pointerId,
                startY: e.clientY,
                startScrollTop: el.scrollTop,
                maxScroll: Math.max(1, el.scrollHeight - el.clientHeight),
                travel,
              }
              setDragging(true)
              e.currentTarget.setPointerCapture(e.pointerId)
            }}
            onPointerMove={(e) => {
              const drag = dragStateRef.current
              if (!drag || drag.pointerId !== e.pointerId) return
              const deltaY = e.clientY - drag.startY
              scrollToOffset(drag.startScrollTop + (deltaY / drag.travel) * drag.maxScroll, true)
            }}
            onPointerUp={(e) => {
              if (dragStateRef.current?.pointerId === e.pointerId) {
                dragStateRef.current = null
                setDragging(false)
                e.currentTarget.releasePointerCapture(e.pointerId)
              }
            }}
            onPointerCancel={() => {
              dragStateRef.current = null
              setDragging(false)
            }}
            style={{
              height: metrics.thumbHeight,
              transform: `translateY(${metrics.thumbTop}px)`,
              backgroundColor: dragging
                ? 'var(--accent)'
                : 'var(--scrollbar-thumb, color-mix(in srgb, var(--accent) 55%, transparent))',
            }}
            className={cn(
              'mx-auto rounded-full transition-[width,opacity,background-color] duration-150',
              dragging
                ? 'w-[4.5px] opacity-100 cursor-grabbing'
                : scrolling
                  ? 'w-[3.5px] opacity-88 cursor-grab hover:w-[4.5px] hover:opacity-100'
                  : 'w-[3.5px] opacity-55 cursor-grab group-hover/scroll:opacity-85 hover:w-[4.5px] hover:opacity-100',
            )}
          />
        </div>
      )}
    </div>
  )
}
