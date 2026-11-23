'use client'

/**
 * SkecherUI — Gooey Tooltip (SVG metaball goo filter + spring-detaching droplet)
 * Adapted cleanly to Zenko 60/30/10 CSS tokens and non-nested trigger wrapping.
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
  const [trigger, setTrigger] = useState({ w: 0, h: 0 })
  const [tip, setTip] = useState({ w: 0, h: 0 })

  useLayoutEffect(() => {
    const triggerNode = triggerRef.current
    const tipNode = tipRef.current
    if (!triggerNode || !tipNode) return
    const measure = () => {
      setTrigger({ w: triggerNode.offsetWidth, h: triggerNode.offsetHeight })
      setTip({ w: tipNode.offsetWidth, h: tipNode.offsetHeight })
    }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(triggerNode)
    observer.observe(tipNode)
    return () => observer.disconnect()
  }, [])

  useEffect(() => () => clearTimeout(timer.current), [])

  const show = useCallback(
    (wait: number) => {
      if (disabled) return
      clearTimeout(timer.current)
      timer.current = setTimeout(() => setOpen(true), wait)
    },
    [disabled],
  )
  const hide = useCallback(() => {
    clearTimeout(timer.current)
    setOpen(false)
  }, [])

  const dir = DIRECTIONS[side]
  const vertical = dir.y !== 0
  const triggerHalf = vertical ? trigger.h / 2 : trigger.w / 2
  const tipHalf = vertical ? tip.h / 2 : tip.w / 2
  const tipDistance = triggerHalf + offset + tipHalf
  const nubDistance = triggerHalf + offset
  const drop = Math.max(8, Math.min(trigger.w, trigger.h) * 0.45)
  const restDistance = Math.max(0, triggerHalf - drop / 2 - 2)
  const ready = trigger.w > 0 && tip.w > 0

  const pad = Math.max(tip.w, tip.h) + offset + 24
  const cx = pad + trigger.w / 2
  const cy = pad + trigger.h / 2
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
    ? { width: tip.w, height: tip.h, borderRadius: Math.min(tip.h / 2, 14) }
    : { width: drop, height: drop, borderRadius: drop / 2 }

  return (
    <span
      className={cn('relative inline-flex', open && 'z-[80]', className)}
      onPointerEnter={(e) => {
        if (e.pointerType !== 'touch') show(delay)
      }}
      onPointerLeave={hide}
      onFocus={() => show(0)}
      onBlur={hide}
      onMouseDown={hide}
    >
      <GooFilter id={filterId} />

      {ready && !disabled && (
        <span
          aria-hidden="true"
          className="pointer-events-none absolute"
          style={{
            left: -pad,
            top: -pad,
            width: trigger.w + pad * 2,
            height: trigger.h + pad * 2,
            filter: `url(#${filterId})`,
            opacity: open ? 1 : 0,
            transition: 'opacity 140ms ease',
          }}
        >
          <span
            className="absolute"
            style={{
              left: cx - trigger.w / 2,
              top: cy - trigger.h / 2,
              width: trigger.w,
              height: trigger.h,
              borderRadius: Math.min(radius, trigger.w / 2, trigger.h / 2),
              backgroundColor: color,
            }}
          />
          <motion.span
            animate={{ x: open ? nubFinal.x : nubRest.x, y: open ? nubFinal.y : nubRest.y }}
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

      <span ref={triggerRef} aria-describedby={open ? tooltipId : undefined} className="relative inline-flex">
        {children}
      </span>

      <div
        className="pointer-events-none absolute top-1/2 left-1/2 z-10"
        style={{ transform: `translate(calc(-50% + ${final.x}px), calc(-50% + ${final.y}px))` }}
      >
        <motion.div
          animate={
            open
              ? { opacity: 1, scale: 1, filter: 'blur(0px)' }
              : { opacity: 0, scale: 0.94, filter: 'blur(3px)' }
          }
          className="flex items-center gap-1.5 whitespace-nowrap px-3 py-1.5 text-[11.5px] font-medium leading-tight"
          id={tooltipId}
          initial={false}
          ref={tipRef}
          role="tooltip"
          style={{ color: foreground }}
          transition={
            reduceMotion
              ? { duration: 0.12 }
              : open
                ? { duration: 0.18, delay: 0.07, ease: [0.23, 1, 0.32, 1] }
                : { duration: 0.06 }
          }
        >
          {content}
        </motion.div>
      </div>
    </span>
  )
}

export default SkecherGooeyTooltip
