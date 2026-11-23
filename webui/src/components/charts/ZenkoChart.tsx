/**
 * Modular, tree-shaken Apache ECharts React wrapper (EvilCharts-inspired blocks).
 * Strictly inherits live CSS theme tokens (--bg, --surface, --card, --border, --fg, --muted, --accent)
 * with zero hardcoded hex colors and automatic ResizeObserver layout sync.
 */
import { useEffect, useRef } from 'react'
import * as echarts from 'echarts/core'
import type { EChartsCoreOption } from 'echarts/core'
import { BarChart, HeatmapChart, LineChart, PieChart } from 'echarts/charts'
import {
  CalendarComponent,
  GridComponent,
  LegendComponent,
  TooltipComponent,
  VisualMapComponent,
} from 'echarts/components'
import { CanvasRenderer } from 'echarts/renderers'
import { cn } from '@/lib/utils'

echarts.use([
  HeatmapChart,
  BarChart,
  LineChart,
  PieChart,
  CalendarComponent,
  GridComponent,
  TooltipComponent,
  VisualMapComponent,
  LegendComponent,
  CanvasRenderer,
])

export interface ZenkoChartPalette {
  bg: string
  surface: string
  card: string
  border: string
  fg: string
  muted: string
  accent: string
  destructive: string
  fontSans: string
  fontMono: string
  isDark: boolean
}

/** Canonical default Obsidian Vermilion fallback tokens aligned 1:1 with `index.css`. */
const DEFAULT_ZENKO_PALETTE: ZenkoChartPalette = {
  bg: '#0a0a0b',
  surface: '#0e0e10',
  card: '#131316',
  border: '#1f1f24',
  fg: '#f5f5f6',
  muted: '#8a8a93',
  accent: '#ea5829',
  destructive: '#ef4444',
  fontSans: 'system-ui, sans-serif',
  fontMono: 'monospace',
  isDark: true,
}

export function readZenkoChartPalette(): ZenkoChartPalette {
  if (typeof document === 'undefined') {
    return DEFAULT_ZENKO_PALETTE
  }
  const styles = getComputedStyle(document.documentElement)
  const get = (k: string, fb: string) => styles.getPropertyValue(k).trim() || fb
  return {
    bg: get('--bg', DEFAULT_ZENKO_PALETTE.bg),
    surface: get('--surface', DEFAULT_ZENKO_PALETTE.surface),
    card: get('--card', DEFAULT_ZENKO_PALETTE.card),
    border: get('--border', DEFAULT_ZENKO_PALETTE.border),
    fg: get('--fg', DEFAULT_ZENKO_PALETTE.fg),
    muted: get('--muted', DEFAULT_ZENKO_PALETTE.muted),
    accent: get('--accent', DEFAULT_ZENKO_PALETTE.accent),
    destructive: get('--destructive', DEFAULT_ZENKO_PALETTE.destructive),
    fontSans: get('--font-sans', DEFAULT_ZENKO_PALETTE.fontSans),
    fontMono: get('--font-mono', DEFAULT_ZENKO_PALETTE.fontMono),
    isDark: document.documentElement.dataset.mode !== 'light',
  }
}

/** Deterministic hex-to-rgba converter (zero regex) for smooth 60/30/10 monochrome + accent ramps. */
export function hexWithAlpha(rawHex: string, alpha: number): string {
  const s = rawHex.trim().startsWith('#') ? rawHex.trim().slice(1) : rawHex.trim()
  if (s.length !== 6 && s.length !== 3) return rawHex
  const full =
    s.length === 3 ? `${s[0]}${s[0]}${s[1]}${s[1]}${s[2]}${s[2]}` : s
  const r = Number.parseInt(full.slice(0, 2), 16)
  const g = Number.parseInt(full.slice(2, 4), 16)
  const b = Number.parseInt(full.slice(4, 6), 16)
  if (!Number.isFinite(r) || !Number.isFinite(g) || !Number.isFinite(b)) return rawHex
  return `rgba(${r}, ${g}, ${b}, ${Math.min(1, Math.max(0, alpha)).toFixed(3)})`
}

/** Builds a 5-step GitHub-style intensity ramp strictly from the active theme's surface + accent. */
export function buildAccentHeatmapRamp(palette: ZenkoChartPalette): [string, string, string, string, string] {
  const emptyCell = palette.isDark
    ? hexWithAlpha(palette.fg, 0.055)
    : hexWithAlpha(palette.fg, 0.075)
  return [
    emptyCell,
    hexWithAlpha(palette.accent, 0.24),
    hexWithAlpha(palette.accent, 0.48),
    hexWithAlpha(palette.accent, 0.74),
    palette.accent,
  ]
}

export function ZenkoChart({
  option,
  height = 200,
  className,
}: {
  option: EChartsCoreOption
  height?: number
  className?: string
}) {
  const containerRef = useRef<HTMLDivElement>(null)
  const instanceRef = useRef<echarts.ECharts | null>(null)

  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    const chart = echarts.init(el, undefined, { renderer: 'canvas' })
    instanceRef.current = chart

    let resizeRaf = 0
    const ro = new ResizeObserver(() => {
      if (resizeRaf) return
      resizeRaf = requestAnimationFrame(() => {
        resizeRaf = 0
        if (!chart.isDisposed()) {
          chart.resize()
        }
      })
    })
    ro.observe(el)

    return () => {
      if (resizeRaf) cancelAnimationFrame(resizeRaf)
      ro.disconnect()
      chart.dispose()
      instanceRef.current = null
    }
  }, [])

  useEffect(() => {
    instanceRef.current?.setOption(option, { notMerge: true, lazyUpdate: true })
  }, [option])

  return (
    <div
      ref={containerRef}
      style={{ height }}
      className={cn('w-full min-w-0 select-none', className)}
    />
  )
}
