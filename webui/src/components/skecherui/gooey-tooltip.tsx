'use client'

/**
 * SkecherUI — Gooey Tooltip (SVG metaball goo filter + spring-detaching droplet)
 * Portal-rendered to document.body at fixed viewport coordinates so it never clips
 * inside overflow-hidden docks, sidebars, scroll containers, or Studio previews.
 */

import { motion, type Transition, useReducedMotion } from 'motion/react'
import {
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { createPortal } from 'react-dom'
import { cn } from '@/lib/utils'

type Side = 'top' | 'bottom' | 'left' | 'right'

export interface SkecherGooeyTooltipProps {
  content: ReactNode
  children: ReactNode
  side?: Side
  offset?: number
  color?: string
  foreground?: string
  radius?: number
  delay?: number
  disabled?: boolean
  className?: string
}

const DIRECTIONS: Record<Side, { x: number; y: number }> = {
  top: { x: 0, y: -1 },
  bottom: { x: 0, y: 1 },
  left: { x: -1, y: 0 },
  right: { x: 1, y: 0 },
}

const NUB = 10
const travelOpen: Transition = { type: 'spring', stiffness: 520, damping: 36, mass: 0.7 }
const alongOpen: Transition = { type: 'spring', stiffness: 620, damping: 38, mass: 0.7 }
const acrossOpen: Transition = { type: 'spring', stiffness: 360, damping: 30, mass: 0.7 }
const tailOpen: Transition = { type: 'spring', stiffness: 260, damping: 26, mass: 0.7 }
const travelClose: Transition = { type: 'spring', stiffness: 640, damping: 52, mass: 0.7 }
const shapeClose: Transition = { type: 'spring', stiffness: 700, damping: 54, mass: 0.7 }
const tailClose: Transition = { type: 'spring', stiffness: 760, damping: 56, mass: 0.7 }

function GooFilter({ id }: { id: string }) {
  return (
    <svg aria-hidden="true" className="pointer-events-none absolute size-0">
      <defs>
        <filter colorInterpolationFilters="sRGB" id={id}>
          <feGaussianBlur in="SourceGraphic" result="blur" stdDeviation="5" />
          <feColorMatrix
            in="blur"
            result="goo"
            type="matrix"
            values="1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 20 -9"
          />
          <feGaussianBlur in="goo" result="shadowBlur" stdDeviation="4" />
          <feOffset dy="3" in="shadowBlur" result="shadowOffset" />
          <feColorMatrix
            in="shadowOffset"
            result="shadow"
            type="matrix"
            values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 0.22 0"
          />
          <feMerge>
            <feMergeNode in="shadow" />
            <feMergeNode in="goo" />
          </feMerge>
        </filter>
      </defs>
    </svg>
  )
}

export function SkecherGooeyTooltip({
  content,
  children,
  side = 'top',
  offset = 14,
  color = 'var(--fg)',
  foreground = 'var(--bg)',
  radius = 9999,
  delay = 60,
  disabled = false,
  className,
}: SkecherGooeyTooltipProps) {
  const id = useId().replace(/[^a-zA-Z0-9]/g, '')
  const filterId = `goo-tip-${id}`
  const tooltipId = `goo-tip-content-${id}`
  const reduceMotion = useReducedMotion() ?? false

  const triggerRef = useRef<HTMLSpanElement>(null)
  const tipRef = useRef<HTMLDivElement>(null)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const [open, setOpen] = useState(false)
  const [mounted, setMounted] = useState(false)
  const [anchor, setAnchor] = useState<{ left: number; top: number; w: number; h: number } | null>(
    null,
  )
  const [tip, setTip] = useState({ w: 0, h: 0 })

  const syncAnchor = useCallback(() => {
    const node = triggerRef.current
    if (!node) return
    const rect = node.getBoundingClientRect()
    setAnchor({ left: rect.left, top: rect.top, w: rect.width, h: rect.height })
  }, [])

  useLayoutEffect(() => {
    if (!mounted) return
    const tipNode = tipRef.current
    if (!tipNode) return
    const measure = () => {
      setTip({ w: tipNode.offsetWidth, h: tipNode.offsetHeight })
    }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(tipNode)
    return () => observer.disconnect()
  }, [mounted, content])

  useEffect(() => {
    if (!open) return
    const dismiss = () => setOpen(false)
    window.addEventListener('scroll', dismiss, true)
    window.addEventListener('resize', dismiss)
    return () => {
      window.removeEventListener('scroll', dismiss, true)
      window.removeEventListener('resize', dismiss)
    }
  }, [open])

  useEffect(() => {
    if (open) {
      setMounted(true)
      return
    }
    if (!mounted) return
    const cleanup = setTimeout(() => setMounted(false), 220)
    return () => clearTimeout(cleanup)
  }, [open, mounted])

  useEffect(() => () => clearTimeout(timer.current), [])

  const show = useCallback(
    (wait: number) => {
      if (disabled) return
      clearTimeout(timer.current)
      timer.current = setTimeout(() => {
        syncAnchor()
        setMounted(true)
        setOpen(true)
      }, wait)
    },
    [disabled, syncAnchor],
  )

  const hide = useCallback(() => {
    clearTimeout(timer.current)
    setOpen(false)
  }, [])

  const triggerW = anchor?.w ?? 0
  const triggerH = anchor?.h ?? 0
  const dir = DIRECTIONS[side]
  const vertical = dir.y !== 0
  const triggerHalf = vertical ? triggerH / 2 : triggerW / 2
  const tipHalf = vertical ? tip.h / 2 : tip.w / 2
  const tipDistance = triggerHalf + offset + tipHalf
  const nubDistance = triggerHalf + offset
  const drop = Math.max(8, Math.min(triggerW, triggerH) * 0.45)
  const restDistance = Math.max(0, triggerHalf - drop / 2 - 2)
  const ready = Boolean(anchor && triggerW > 0 && tip.w > 0)

  const pad = Math.max(tip.w, tip.h) + offset + 24
  const cx = pad + triggerW / 2
  const cy = pad + triggerH / 2
  const at = (distance: number) => ({ x: dir.x * distance, y: dir.y * distance })

  const rest = at(restDistance)
  const final = at(tipDistance)
  const nubRest = at(Math.max(0, triggerHalf - NUB / 2 - 2))
  const nubFinal = at(nubDistance)

  const instant: Transition = { duration: 0 }
  const along = vertical ? 'height' : 'width'
  const across = vertical ? 'width' : 'height'
  const travelTransition = reduceMotion ? instant : open ? travelOpen : travelClose
  const shapeTransition: Transition = reduceMotion
    ? instant
    : open
      ? { [along]: alongOpen, [across]: acrossOpen, borderRadius: acrossOpen }
      : shapeClose
  const tailTransition = reduceMotion ? instant : open ? tailOpen : tailClose

  const end = open ? final : rest
  const blobShape = open
    ? { width: tip.w, height: tip.h, borderRadius: Math.min(tip.h / 2, radius) }
    : { width: drop, height: drop, borderRadius: drop / 2 }

  const portalNode =
    mounted && anchor && !disabled && typeof document !== 'undefined'
      ? createPortal(
          <div
            className="pointer-events-none fixed z-[9999]"
            style={{
              left: anchor.left,
              top: anchor.top,
              width: triggerW,
              height: triggerH,
            }}
          >
            <GooFilter id={filterId} />

            {ready && (
              <span
                aria-hidden="true"
                className="pointer-events-none absolute"
                style={{
                  left: -pad,
                  top: -pad,
                  width: triggerW + pad * 2,
                  height: triggerH + pad * 2,
                  filter: `url(#${filterId})`,
                  opacity: open ? 1 : 0,
                  transition: 'opacity 140ms ease',
                }}
              >
                <motion.span
                  animate={{
                    x: open ? nubFinal.x : nubRest.x,
                    y: open ? nubFinal.y : nubRest.y,
                    scale: open ? 1 : 0.4,
                  }}
                  className="absolute rounded-full"
                  initial={false}
                  style={{
                    left: cx - NUB / 2,
                    top: cy - NUB / 2,
                    width: NUB,
                    height: NUB,
                    backgroundColor: color,
                  }}
                  transition={tailTransition}
                />
                <motion.span
                  animate={{ x: end.x, y: end.y }}
                  className="absolute size-0"
                  initial={{ x: rest.x, y: rest.y }}
                  style={{ left: cx, top: cy }}
                  transition={travelTransition}
                >
                  <motion.span
                    animate={blobShape}
                    className="absolute top-0 left-0 -translate-x-1/2 -translate-y-1/2"
                    initial={{ width: drop, height: drop, borderRadius: drop / 2 }}
                    style={{ backgroundColor: color }}
                    transition={shapeTransition}
                  />
                </motion.span>
              </span>
            )}

            <div
              className="pointer-events-none absolute top-1/2 left-1/2 z-10"
              style={{
                transform: `translate(calc(-50% + ${final.x}px), calc(-50% + ${final.y}px))`,
              }}
            >
              <motion.div
                animate={
                  open
                    ? { opacity: 1, scale: 1, filter: 'blur(0px)' }
                    : { opacity: 0, scale: 0.94, filter: 'blur(3px)' }
                }
                className="flex items-center gap-1.5 whitespace-nowrap px-3 py-1.5 text-[11.5px] font-medium leading-tight"
                id={tooltipId}
                initial={{ opacity: 0, scale: 0.94, filter: 'blur(3px)' }}
                ref={tipRef}
                role="tooltip"
                style={{ color: foreground }}
                transition={
                  reduceMotion
                    ? { duration: 0.1 }
                    : open
                      ? { duration: 0.18, delay: 0.06, ease: [0.23, 1, 0.32, 1] }
                      : { duration: 0.06 }
                }
              >
                {content}
              </motion.div>
            </div>
          </div>,
          document.body,
        )
      : null

  return (
    <span
      ref={triggerRef}
      aria-describedby={open ? tooltipId : undefined}
      className={cn('relative inline-flex', className)}
      onPointerEnter={(e) => {
        if (e.pointerType !== 'touch') show(delay)
      }}
      onPointerLeave={hide}
      onFocus={() => show(0)}
      onBlur={hide}
      onMouseDown={hide}
    >
      {children}
      {portalNode}
    </span>
  )
}

export default SkecherGooeyTooltip
