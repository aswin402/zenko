'use client'

import {
  forwardRef,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type InputHTMLAttributes,
  type TextareaHTMLAttributes,
} from 'react'
import { motion, useReducedMotion } from 'motion/react'
import { cn } from '@/lib/utils'

export type InputCaretMode = 'skiper106_smooth' | 'native_accent' | 'native_default'

const CARET_SPRING = {
  type: 'spring',
  stiffness: 620,
  damping: 38,
  mass: 0.35,
} as const

/** Reads active global input caret mode from `html[data-input-caret]` reactively. */
export function useInputCaretMode(override?: InputCaretMode): InputCaretMode {
  const [mode, setMode] = useState<InputCaretMode>(() => {
    if (override) return override
    if (typeof document === 'undefined') return 'skiper106_smooth'
    const raw = document.documentElement.dataset.inputCaret as InputCaretMode | undefined
    return raw || 'skiper106_smooth'
  })

  useEffect(() => {
    if (override) {
      setMode(override)
      return
    }
    if (typeof document === 'undefined') return
    const sync = () => {
      const raw = document.documentElement.dataset.inputCaret as InputCaretMode | undefined
      setMode(raw || 'skiper106_smooth')
    }
    sync()
    const obs = new MutationObserver(sync)
    obs.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-input-caret'],
    })
    return () => obs.disconnect()
  }, [override])

  return override ?? mode
}

let sharedMeasureCanvas: HTMLCanvasElement | null = null

function measureTextWidthPx(text: string, font: string, letterSpacing: string): number {
  if (!text) return 0
  if (typeof document === 'undefined') return text.length * 8
  if (!sharedMeasureCanvas) {
    sharedMeasureCanvas = document.createElement('canvas')
  }
  const ctx = sharedMeasureCanvas.getContext('2d')
  if (!ctx) return text.length * 8
  ctx.font = font
  let width = ctx.measureText(text).width
  const ls = Number.parseFloat(letterSpacing)
  if (Number.isFinite(ls) && ls !== 0 && text.length > 0) {
    width += ls * text.length
  }
  return width
}

function assignRef<T>(ref: React.ForwardedRef<T>, value: T | null) {
  if (typeof ref === 'function') {
    ref(value)
  } else if (ref) {
    ref.current = value
  }
}

export interface SmoothCaretInputProps extends InputHTMLAttributes<HTMLInputElement> {
  /** Optional explicit override; defaults to global `surface.input_caret_style`. */
  caretMode?: InputCaretMode
  /** Extra classes on the outer positioning wrapper. */
  wrapperClassName?: string
}

/**
 * Skiper106 Smooth Caret Input (`@skiper-ui/skiper106`).
 * Replaces the native blinking cursor with a spring-animated accent caret tracked via
 * Canvas 2D text measurement, synchronized scroll offset, and reduced-motion support.
 */
export const SmoothCaretInput = forwardRef<HTMLInputElement, SmoothCaretInputProps>(
  function SmoothCaretInput(
    {
      caretMode: modeOverride,
      wrapperClassName,
      className,
      value,
      defaultValue,
      onChange,
      onFocus,
      onBlur,
      onSelect,
      onKeyUp,
      onMouseUp,
      onScroll,
      ...rest
    },
    forwardedRef,
  ) {
    const mode = useInputCaretMode(modeOverride)
    const reduce = useReducedMotion() ?? false
    const smoothActive = mode === 'skiper106_smooth' && !reduce
    const inputRef = useRef<HTMLInputElement | null>(null)
    const [focused, setFocused] = useState(false)
    const [hasSelection, setHasSelection] = useState(false)
    const [typing, setTyping] = useState(false)
    const [caret, setCaret] = useState({ x: 0, top: 6, height: 16, visible: false })
    const idleTimerRef = useRef<number | undefined>(undefined)

    const measureCaret = useCallback(() => {
      const el = inputRef.current
      if (!el || !smoothActive) return
      const start = el.selectionStart ?? 0
      const end = el.selectionEnd ?? start
      if (start !== end) {
        setHasSelection(true)
        return
      }
      setHasSelection(false)
      const cs = window.getComputedStyle(el)
      const font =
        cs.font && cs.font !== 'normal'
          ? cs.font
          : `${cs.fontStyle} ${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`
      const padLeft = Number.parseFloat(cs.paddingLeft) || 0
      const padRight = Number.parseFloat(cs.paddingRight) || 0
      const borderLeft = Number.parseFloat(cs.borderLeftWidth) || 0
      const fontSize = Number.parseFloat(cs.fontSize) || 14
      const caretHeight = Math.round(Math.min(el.clientHeight - 6, Math.max(13, fontSize * 1.28)))
      const top = Math.max(2, Math.round((el.offsetHeight - caretHeight) / 2))

      const raw = el.type === 'password' ? '•'.repeat(el.value.length) : el.value
      const prefix = raw.slice(0, start)
      const textWidth = measureTextWidthPx(prefix, font, cs.letterSpacing)
      const rawX = borderLeft + padLeft + textWidth - el.scrollLeft
      const minX = borderLeft + padLeft - 1
      const maxX = Math.max(minX, el.offsetWidth - padRight - 3)
      const x = Math.max(minX, Math.min(maxX, rawX))

      setCaret({ x, top, height: caretHeight, visible: true })
    }, [smoothActive])

    const bumpTyping = useCallback(() => {
      setTyping(true)
      if (idleTimerRef.current) window.clearTimeout(idleTimerRef.current)
      idleTimerRef.current = window.setTimeout(() => setTyping(false), 420)
    }, [])

    useLayoutEffect(() => {
      if (focused && smoothActive) {
        measureCaret()
      }
    }, [focused, smoothActive, value, measureCaret])

    useEffect(
      () => () => {
        if (idleTimerRef.current) window.clearTimeout(idleTimerRef.current)
      },
      [],
    )

    return (
      <div className={cn('relative flex w-full items-center', wrapperClassName)}>
        <input
          {...rest}
          ref={(node) => {
            inputRef.current = node
            assignRef(forwardedRef, node)
          }}
          value={value}
          defaultValue={defaultValue}
          onChange={(e) => {
            onChange?.(e)
            bumpTyping()
            requestAnimationFrame(measureCaret)
          }}
          onFocus={(e) => {
            onFocus?.(e)
            setFocused(true)
            requestAnimationFrame(measureCaret)
          }}
          onBlur={(e) => {
            onBlur?.(e)
            setFocused(false)
          }}
          onSelect={(e) => {
            onSelect?.(e)
            measureCaret()
          }}
          onKeyUp={(e) => {
            onKeyUp?.(e)
            bumpTyping()
            measureCaret()
          }}
          onMouseUp={(e) => {
            onMouseUp?.(e)
            measureCaret()
          }}
          onScroll={(e) => {
            onScroll?.(e)
            measureCaret()
          }}
          className={cn(
            className,
            smoothActive && !hasSelection && 'caret-transparent',
            mode === 'native_accent' && 'caret-[var(--accent)]',
          )}
        />
        {smoothActive && focused && !hasSelection && caret.visible && (
          <motion.span
            aria-hidden="true"
            initial={false}
            animate={{
              x: caret.x,
              y: caret.top,
              height: caret.height,
            }}
            transition={CARET_SPRING}
            className={cn(
              'pointer-events-none absolute left-0 top-0 z-10 w-[2px] rounded-full',
              !typing && 'zenko-caret-blink',
            )}
            style={{
              backgroundColor: 'var(--accent)',
              boxShadow: '0 0 8px color-mix(in srgb, var(--accent) 55%, transparent)',
            }}
          />
        )}
      </div>
    )
  },
)

export interface SmoothCaretTextareaProps extends TextareaHTMLAttributes<HTMLTextAreaElement> {
  /** Optional explicit override; defaults to global `surface.input_caret_style`. */
  caretMode?: InputCaretMode
  wrapperClassName?: string
}

/**
 * Multiline Skiper106 Smooth Caret Textarea for Zenko's Chat Input Box.
 * Uses a synchronized hidden mirror box to resolve exact (x, y) wrapped caret coordinates
 * and spring-glides the custom accent caret seamlessly across lines.
 */
export const SmoothCaretTextarea = forwardRef<HTMLTextAreaElement, SmoothCaretTextareaProps>(
  function SmoothCaretTextarea(
    {
      caretMode: modeOverride,
      wrapperClassName,
      className,
      value,
      defaultValue,
      onChange,
      onFocus,
      onBlur,
      onSelect,
      onKeyUp,
      onMouseUp,
      onScroll,
      ...rest
    },
    forwardedRef,
  ) {
    const mode = useInputCaretMode(modeOverride)
    const reduce = useReducedMotion() ?? false
    const smoothActive = mode === 'skiper106_smooth' && !reduce
    const textareaRef = useRef<HTMLTextAreaElement | null>(null)
    const mirrorRef = useRef<HTMLDivElement | null>(null)
    const [focused, setFocused] = useState(false)
    const [hasSelection, setHasSelection] = useState(false)
    const [typing, setTyping] = useState(false)
    const [caret, setCaret] = useState({ x: 0, y: 2, height: 20, visible: false })
    const idleTimerRef = useRef<number | undefined>(undefined)

    const measureCaret = useCallback(() => {
      const el = textareaRef.current
      const mirror = mirrorRef.current
      if (!el || !mirror || !smoothActive) return

      const start = el.selectionStart ?? 0
      const end = el.selectionEnd ?? start
      if (start !== end) {
        setHasSelection(true)
        return
      }
      setHasSelection(false)

      const cs = window.getComputedStyle(el)
      mirror.style.width = `${el.clientWidth}px`
      mirror.style.fontFamily = cs.fontFamily
      mirror.style.fontSize = cs.fontSize
      mirror.style.fontWeight = cs.fontWeight
      mirror.style.fontStyle = cs.fontStyle
      mirror.style.lineHeight = cs.lineHeight
      mirror.style.letterSpacing = cs.letterSpacing
      mirror.style.paddingTop = cs.paddingTop
      mirror.style.paddingRight = cs.paddingRight
      mirror.style.paddingBottom = cs.paddingBottom
      mirror.style.paddingLeft = cs.paddingLeft
      mirror.style.borderWidth = cs.borderWidth
      mirror.style.boxSizing = cs.boxSizing

      const before = el.value.slice(0, start)
      mirror.textContent = before
      const marker = document.createElement('span')
      marker.textContent = '\u200b'
      mirror.appendChild(marker)

      const fontSize = Number.parseFloat(cs.fontSize) || 14.5
      const rawLineHeight = Number.parseFloat(cs.lineHeight)
      const lineH = Number.isFinite(rawLineHeight) ? rawLineHeight : fontSize * 1.55
      const caretHeight = Math.round(Math.max(15, fontSize * 1.24))
      const yOffset = Math.max(0, Math.round((lineH - caretHeight) / 2))

      const rawX = marker.offsetLeft - el.scrollLeft
      const rawY = marker.offsetTop - el.scrollTop + yOffset
      const visible = rawY + caretHeight >= 0 && rawY <= el.clientHeight

      setCaret({
        x: Math.max(0, Math.min(el.clientWidth - 2, rawX)),
        y: Math.max(0, rawY),
        height: caretHeight,
        visible,
      })
    }, [smoothActive])

    const bumpTyping = useCallback(() => {
      setTyping(true)
      if (idleTimerRef.current) window.clearTimeout(idleTimerRef.current)
      idleTimerRef.current = window.setTimeout(() => setTyping(false), 420)
    }, [])

    useLayoutEffect(() => {
      if (focused && smoothActive) {
        measureCaret()
      }
    }, [focused, smoothActive, value, measureCaret])

    useEffect(() => {
      const el = textareaRef.current
      if (!el || !smoothActive || typeof ResizeObserver === 'undefined') return
      const ro = new ResizeObserver(() => {
        if (focused) measureCaret()
      })
      ro.observe(el)
      return () => ro.disconnect()
    }, [focused, smoothActive, measureCaret])

    useEffect(
      () => () => {
        if (idleTimerRef.current) window.clearTimeout(idleTimerRef.current)
      },
      [],
    )

    return (
      <div className={cn('relative w-full', wrapperClassName)}>
        {/* Synchronized invisible mirror for wrapped multiline caret measurement */}
        <div
          ref={mirrorRef}
          aria-hidden="true"
          className="pointer-events-none invisible fixed left-[-9999px] top-[-9999px] whitespace-pre-wrap break-words"
        />
        <textarea
          {...rest}
          ref={(node) => {
            textareaRef.current = node
            assignRef(forwardedRef, node)
          }}
          value={value}
          defaultValue={defaultValue}
          onChange={(e) => {
            onChange?.(e)
            bumpTyping()
            requestAnimationFrame(measureCaret)
          }}
          onFocus={(e) => {
            onFocus?.(e)
            setFocused(true)
            requestAnimationFrame(measureCaret)
          }}
          onBlur={(e) => {
            onBlur?.(e)
            setFocused(false)
          }}
          onSelect={(e) => {
            onSelect?.(e)
            measureCaret()
          }}
          onKeyUp={(e) => {
            onKeyUp?.(e)
            bumpTyping()
            measureCaret()
          }}
          onMouseUp={(e) => {
            onMouseUp?.(e)
            measureCaret()
          }}
          onScroll={(e) => {
            onScroll?.(e)
            measureCaret()
          }}
          className={cn(
            className,
            smoothActive && !hasSelection && 'caret-transparent',
            mode === 'native_accent' && 'caret-[var(--accent)]',
          )}
        />
        {smoothActive && focused && !hasSelection && caret.visible && (
          <motion.span
            aria-hidden="true"
            initial={false}
            animate={{
              x: caret.x,
              y: caret.y,
              height: caret.height,
            }}
            transition={CARET_SPRING}
            className={cn(
              'pointer-events-none absolute left-0 top-0 z-10 w-[2px] rounded-full',
              !typing && 'zenko-caret-blink',
            )}
            style={{
              backgroundColor: 'var(--accent)',
              boxShadow: '0 0 8px color-mix(in srgb, var(--accent) 55%, transparent)',
            }}
          />
        )}
      </div>
    )
  },
)
