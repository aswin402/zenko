import { useCallback, useEffect, useMemo, useState } from 'react'
import type { EChartsCoreOption } from 'echarts/core'
import { RecapCard } from 'motion-brew/recap-card'
import {
  Activity,
  Cpu,
  KeyRound,
  Palette,
  RefreshCw,
  ShieldCheck,
  Terminal,
  Zap,
} from 'lucide-react'
import { api, escapeHtml, formatBytes, formatCompactTokens } from '@/lib/api'
import type {
  BackgroundJob,
  DaemonTelemetry,
  DailyActivityPoint,
  DashboardAnalytics,
  LogCategoryMeta,
  LogEntry,
  SubagentRecord,
  SubagentRoleDescriptor,
  TaskKindMeta,
  UiCustomization,
} from '@/lib/types'
import { cn } from '@/lib/utils'
import { DataTable } from '@/components/assistant-ui/elements/data-table'
import { ZenkoFoxLogo } from '@/components/beui/ZenkoFoxLogo'
import {
  ZenkoChart,
  buildAccentHeatmapRamp,
  hexWithAlpha,
  readZenkoChartPalette,
  type ZenkoChartPalette,
} from '@/components/charts/ZenkoChart'
import { ActivityGrid } from '@/components/motion-brew/activity-grid'
import { MixChart } from '@/components/motion-brew/mix-chart'
import { QuotaGauge } from '@/components/motion-brew/quota-gauge'
import { UsageRing } from '@/components/motion-brew/usage-ring'
import { Faq as RareFaq, type FaqItem } from '@/components/rareui/faq'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { ZenkoTooltip } from '@/components/ui/Tooltip'
import { TelemetryView } from '@/components/views/SecondaryViews'

function formatUptime(secs: number): string {
  if (secs < 60) return `${secs}s`
  const mins = Math.floor(secs / 60)
  if (mins < 60) return `${mins}m`
  const hrs = Math.floor(mins / 60)
  if (hrs < 24) return `${hrs}h ${mins % 60}m`
  const days = Math.floor(hrs / 24)
  return `${days}d ${hrs % 24}h`
}

function isoDateOffset(daysAgo: number): string {
  const d = new Date()
  d.setUTCDate(d.getUTCDate() - daysAgo)
  return d.toISOString().slice(0, 10)
}

const HEATMAP_RANGE_DAYS: Record<'year' | 'half_year' | 'quarter', number> = {
  year: 364,
  half_year: 182,
  quarter: 90,
}

const HEATMAP_RADIUS_BY_SHAPE: Record<'rounded' | 'square' | 'circle', number> = {
  rounded: 3,
  square: 0,
  circle: 99,
}

const ZENKO_DASHBOARD_FAQ: FaqItem[] = [
  {
    id: 'rss',
    question: 'Why is Zenko memory (VmRSS) under 40 MB?',
    answer:
      'Zenko runs as a single statically compiled native Rust binary with mimalloc (no_thp), bounded Tokio worker threads, rustls, and SQLite WAL with bounded page cache — zero Node.js or Python runtimes.',
  },
  {
    id: 'vault',
    question: 'How does zenko_vault protect my API keys?',
    answer:
      'Every provider key and Telegram token is encrypted at rest in vault.enc with ChaCha20-Poly1305 and 0600 permissions. Only key slot names appear in telemetry — secret values are never exposed or logged.',
  },
  {
    id: 'kv-cache',
    question: 'What does KV Cache Hit measure?',
    answer:
      'It tracks the share of prompt tokens served from provider prompt-caching across active turns, directly cutting latency and spend on long autonomous engineering sessions.',
  },
  {
    id: 'studio',
    question: 'Can I customize every chart, dock, and chat widget?',
    answer:
      'Yes — open Appearance Studio (or press ⌘.) to switch between Apache ECharts and Motion-Brew widgets, toggle Recap Card, Quota Gauges, FAQ, glass docks, context meters, and full 60/30/10 palettes.',
  },
]

function buildActivityGridSeries(points: readonly DailyActivityPoint[], days: number): number[] {
  const byDate = new Map(points.map((p) => [p.date, p.events + p.tool_calls]))
  const out: number[] = []
  for (let i = days - 1; i >= 0; i -= 1) {
    out.push(byDate.get(isoDateOffset(i)) ?? 0)
  }
  return out
}

function buildMixChartRanges(points: readonly DailyActivityPoint[]) {
  const byDate = new Map(points.map((p) => [p.date, p]))
  const makeRange = (id: string, label: string, days: number) => {
    const pts = Array.from({ length: days }, (_, idx) => {
      const iso = isoDateOffset(days - 1 - idx)
      const pt = byDate.get(iso)
      const totalK = pt ? Number((pt.tokens / 1000).toFixed(2)) : 0
      const cachedK = pt ? Number((pt.cache_read_tokens / 1000).toFixed(2)) : 0
      const uncachedK = Number(Math.max(0, totalK - cachedK).toFixed(2))
      const toolWeight = pt ? Number((pt.tool_calls * 0.5).toFixed(2)) : 0
      return {
        label: iso.slice(8),
        detail: iso,
        values: [cachedK, uncachedK, toolWeight],
      }
    })
    const half = Math.max(1, Math.floor(pts.length / 2))
    const prevSum = pts.slice(0, half).reduce((a, p) => a + p.values[0] + p.values[1], 0)
    const currSum = pts.slice(half).reduce((a, p) => a + p.values[0] + p.values[1], 0)
    const pctChange = prevSum > 0 ? ((currSum - prevSum) / prevSum) * 100 : currSum > 0 ? 100 : 0
    return {
      id,
      label,
      points: pts,
      change: Number(pctChange.toFixed(1)),
      caption: `vs previous ${half}d`,
    }
  }
  return [
    makeRange('7d', '7D', 7),
    makeRange('14d', '14D', 14),
    makeRange('30d', '30D', 30),
  ]
}

function buildHeatmapOption(
  points: readonly DailyActivityPoint[],
  palette: ZenkoChartPalette,
  shape: 'rounded' | 'square' | 'circle',
  rangeKey: 'year' | 'half_year' | 'quarter',
  compact = false,
): EChartsCoreOption {
  const daysBack = compact ? 112 : HEATMAP_RANGE_DAYS[rangeKey]
  const startDate = isoDateOffset(daysBack)
  const endDate = isoDateOffset(0)
  const byDate = new Map(points.map((p) => [p.date, p]))

  const data: Array<[string, number]> = []
  let maxScore = 1
  for (let i = daysBack; i >= 0; i -= 1) {
    const key = isoDateOffset(i)
    const pt = byDate.get(key)
    const score = pt ? pt.events + pt.tool_calls : 0
    if (score > maxScore) maxScore = score
    data.push([key, score])
  }

  const ramp = buildAccentHeatmapRamp(palette)
  const radius = HEATMAP_RADIUS_BY_SHAPE[shape]
  const cellSize = compact ? [11, 11] : rangeKey === 'year' ? ['auto', 13] : ['auto', 16]

  return {
    animationDuration: 360,
    tooltip: {
      position: 'top',
      backgroundColor: palette.card,
      borderColor: hexWithAlpha(palette.accent, 0.42),
      borderWidth: 1,
      textStyle: {
        color: palette.fg,
        fontFamily: palette.fontMono,
        fontSize: 11,
      },
      formatter: (params: any) => {
        const [date, val] = params.value as [string, number]
        const safeDate = escapeHtml(String(date ?? ''))
        const safeFg = escapeHtml(palette.fg)
        const safeMuted = escapeHtml(palette.muted)
        const pt = byDate.get(date)
        if (!pt || val === 0) {
          return `<div style="line-height:1.5"><b>${safeDate}</b><br/><span style="color:${safeMuted}">No recorded activity</span></div>`
        }
        return [
          `<div style="line-height:1.55">`,
          `<b style="color:${safeFg}">${safeDate}</b>`,
          `<br/>Events: <b>${pt.events}</b> · Tools: <b>${pt.tool_calls}</b>`,
          `<br/>Tokens: <b>${escapeHtml(formatCompactTokens(pt.tokens))}</b>`,
          pt.cache_read_tokens > 0
            ? ` (${escapeHtml(formatCompactTokens(pt.cache_read_tokens))} cached)`
            : '',
          pt.cost_usd > 0 ? `<br/>Spend: <b>$${pt.cost_usd.toFixed(4)}</b>` : '',
          `</div>`,
        ].join('')
      },
    },
    visualMap: {
      show: false,
      min: 0,
      max: maxScore,
      type: 'piecewise',
      pieces: [
        { min: 0, max: 0, color: ramp[0] },
        { min: 1, max: Math.max(2, Math.ceil(maxScore * 0.25)), color: ramp[1] },
        {
          min: Math.max(2, Math.ceil(maxScore * 0.25)) + 1,
          max: Math.max(4, Math.ceil(maxScore * 0.55)),
          color: ramp[2],
        },
        {
          min: Math.max(4, Math.ceil(maxScore * 0.55)) + 1,
          max: Math.max(6, Math.ceil(maxScore * 0.8)),
          color: ramp[3],
        },
        { min: Math.max(6, Math.ceil(maxScore * 0.8)) + 1, color: ramp[4] },
      ],
    },
    calendar: {
      top: compact ? 22 : 28,
      left: compact ? 24 : 34,
      right: 12,
      bottom: 6,
      range: [startDate, endDate],
      cellSize,
      splitLine: { show: false },
      itemStyle: {
        color: 'transparent',
        borderWidth: 3,
        borderColor: palette.bg,
      },
      yearLabel: { show: false },
      monthLabel: {
        color: palette.muted,
        fontFamily: palette.fontMono,
        fontSize: compact ? 9.5 : 10.5,
        margin: 8,
      },
      dayLabel: {
        firstDay: 1,
        nameMap: ['', 'Mon', '', 'Wed', '', 'Fri', ''],
        color: palette.muted,
        fontFamily: palette.fontMono,
        fontSize: compact ? 9 : 10,
      },
    },
    series: [
      {
        type: 'heatmap',
        coordinateSystem: 'calendar',
        data,
        itemStyle: {
          borderRadius: radius,
          borderColor: palette.bg,
          borderWidth: shape === 'circle' ? 2.5 : 2,
        },
        emphasis: {
          itemStyle: {
            borderColor: palette.fg,
            borderWidth: 1.2,
          },
        },
      },
    ],
  }
}

function buildTokenTrendOption(
  points: readonly DailyActivityPoint[],
  palette: ZenkoChartPalette,
  chartTheme: 'monochrome_accent' | 'soft_gradient' | 'crisp_outline',
): EChartsCoreOption {
  const recent = points.slice(-18)
  const labels = recent.map((p) => p.date.slice(5))
  const totalTok = recent.map((p) => p.tokens)
  const cachedTok = recent.map((p) => p.cache_read_tokens)
  const isOutline = chartTheme === 'crisp_outline'
  const isGradient = chartTheme === 'soft_gradient'

  return {
    animationDuration: 360,
    grid: { top: 24, left: 46, right: 16, bottom: 26 },
    tooltip: {
      trigger: 'axis',
      backgroundColor: palette.card,
      borderColor: hexWithAlpha(palette.accent, 0.4),
      borderWidth: 1,
      textStyle: { color: palette.fg, fontFamily: palette.fontMono, fontSize: 11 },
    },
    legend: {
      top: 0,
      right: 6,
      itemWidth: 10,
      itemHeight: 6,
      textStyle: { color: palette.muted, fontFamily: palette.fontMono, fontSize: 10.5 },
    },
    xAxis: {
      type: 'category',
      data: labels.length > 0 ? labels : ['Today'],
      axisLine: { lineStyle: { color: palette.border } },
      axisTick: { show: false },
      axisLabel: { color: palette.muted, fontFamily: palette.fontMono, fontSize: 10 },
    },
    yAxis: {
      type: 'value',
      splitLine: { lineStyle: { color: hexWithAlpha(palette.border, 0.5), type: 'dashed' } },
      axisLabel: {
        color: palette.muted,
        fontFamily: palette.fontMono,
        fontSize: 10,
        formatter: (v: number) => formatCompactTokens(v),
      },
    },
    series: [
      {
        name: 'Total Tokens',
        type: isOutline ? 'line' : 'bar',
        barMaxWidth: 16,
        smooth: true,
        showSymbol: isOutline,
        data: totalTok.length > 0 ? totalTok : [0],
        itemStyle: {
          color: hexWithAlpha(palette.fg, isOutline ? 0.85 : 0.24),
          borderRadius: [4, 4, 0, 0],
        },
        lineStyle: { width: 1.75, color: hexWithAlpha(palette.fg, 0.78) },
      },
      {
        name: 'KV Cached',
        type: 'line',
        smooth: true,
        showSymbol: false,
        data: cachedTok.length > 0 ? cachedTok : [0],
        lineStyle: { width: 2, color: palette.accent },
        itemStyle: { color: palette.accent },
        areaStyle: isOutline
          ? undefined
          : {
              color: isGradient
                ? {
                    type: 'linear',
                    x: 0,
                    y: 0,
                    x2: 0,
                    y2: 1,
                    colorStops: [
                      { offset: 0, color: hexWithAlpha(palette.accent, 0.34) },
                      { offset: 1, color: hexWithAlpha(palette.accent, 0.02) },
                    ],
                  }
                : hexWithAlpha(palette.accent, 0.14),
            },
      },
    ],
  }
}

function buildStorageDonutOption(
  storage: DashboardAnalytics['storage'],
  palette: ZenkoChartPalette,
  chartTheme: 'monochrome_accent' | 'soft_gradient' | 'crisp_outline',
): EChartsCoreOption {
  const nonEmpty = storage.filter((s) => s.bytes > 0)
  const items = nonEmpty.length > 0 ? nonEmpty : storage.slice(0, 4)
  const shades = [
    palette.accent,
    hexWithAlpha(palette.accent, 0.68),
    hexWithAlpha(palette.fg, 0.68),
    hexWithAlpha(palette.fg, 0.46),
    hexWithAlpha(palette.fg, 0.3),
    hexWithAlpha(palette.muted, 0.55),
    hexWithAlpha(palette.muted, 0.35),
    hexWithAlpha(palette.border, 0.9),
  ]

  return {
    animationDuration: 360,
    tooltip: {
      trigger: 'item',
      backgroundColor: palette.card,
      borderColor: hexWithAlpha(palette.accent, 0.4),
      borderWidth: 1,
      textStyle: { color: palette.fg, fontFamily: palette.fontMono, fontSize: 11 },
      formatter: (p: any) =>
        `<b>${escapeHtml(String(p?.name ?? ''))}</b><br/>${escapeHtml(formatBytes(Number(p?.value || 0)))} (${escapeHtml(String(p?.percent ?? 0))}%)`,
    },
    series: [
      {
        name: 'Storage',
        type: 'pie',
        radius: ['56%', '82%'],
        center: ['50%', '50%'],
        padAngle: 3,
        itemStyle: {
          borderRadius: 6,
          borderColor: chartTheme === 'crisp_outline' ? palette.accent : palette.bg,
          borderWidth: chartTheme === 'crisp_outline' ? 1.5 : 2,
        },
        label: { show: false },
        data: items.map((s, idx) => ({
          name: s.label,
          value: Math.max(s.bytes, nonEmpty.length === 0 ? 1 : 0),
          itemStyle: {
            color: chartTheme === 'crisp_outline' ? 'transparent' : shades[idx % shades.length],
          },
        })),
      },
    ],
  }
}

function buildToolsBarOption(
  tools: DashboardAnalytics['top_tools'],
  palette: ZenkoChartPalette,
  chartTheme: 'monochrome_accent' | 'soft_gradient' | 'crisp_outline',
): EChartsCoreOption {
  const top = [...tools].slice(0, 8).reverse()
  const isOutline = chartTheme === 'crisp_outline'
  return {
    animationDuration: 360,
    grid: { top: 8, left: 108, right: 24, bottom: 20 },
    tooltip: {
      trigger: 'axis',
      axisPointer: { type: 'shadow' },
      backgroundColor: palette.card,
      borderColor: hexWithAlpha(palette.accent, 0.4),
      borderWidth: 1,
      textStyle: { color: palette.fg, fontFamily: palette.fontMono, fontSize: 11 },
    },
    xAxis: {
      type: 'value',
      splitLine: { lineStyle: { color: hexWithAlpha(palette.border, 0.45), type: 'dashed' } },
      axisLabel: { color: palette.muted, fontFamily: palette.fontMono, fontSize: 10 },
    },
    yAxis: {
      type: 'category',
      data: top.map((t) => t.name),
      axisLine: { show: false },
      axisTick: { show: false },
      axisLabel: { color: palette.fg, fontFamily: palette.fontMono, fontSize: 10.5 },
    },
    series: [
      {
        name: 'Calls',
        type: 'bar',
        barMaxWidth: 14,
        data: top.map((t, i) => ({
          value: t.calls,
          itemStyle: {
            color: isOutline
              ? 'transparent'
              : i === top.length - 1
                ? palette.accent
                : hexWithAlpha(palette.accent, 0.56),
            borderColor: palette.accent,
            borderWidth: isOutline ? 1.5 : 0,
            borderRadius: [0, 5, 5, 0],
          },
        })),
      },
    ],
  }
}

const PREVIEW_ACTIVITY_POINTS: DailyActivityPoint[] = Array.from({ length: 112 }, (_, idx) => {
  const date = isoDateOffset(111 - idx)
  const wave = ((idx * 17 + (idx % 7) * 5) % 19)
  const active = wave > 5
  return {
    date,
    events: active ? wave : 0,
    turns: active ? Math.max(1, Math.floor(wave / 3)) : 0,
    tool_calls: active ? Math.floor(wave / 2) : 0,
    tokens: active ? wave * 1850 : 0,
    cache_read_tokens: active ? wave * 1240 : 0,
    cost_usd: active ? wave * 0.0004 : 0,
  }
})

/** Focused animated Live Preview used inside Theme Studio -> Dashboard. */
export function DashboardMiniPreview({ ui }: { ui: UiCustomization }) {
  const dash = ui.components.dashboard || {
    layout: 'bento_grid' as const,
    chart_theme: 'monochrome_accent' as const,
    heatmap_shape: 'rounded' as const,
    heatmap_range: 'year' as const,
    widget_kit: 'echarts' as const,
    show_heatmap: true,
    show_storage_vault: true,
    show_token_trends: true,
    show_tools_table: true,
    show_subagents_jobs: true,
    show_logs: true,
    show_recap_card: false,
    show_quota_gauges: false,
    show_faq: false,
  }
  const widgetKit = dash.widget_kit || 'echarts'
  const showRecap = dash.show_recap_card ?? widgetKit !== 'echarts'
  const showQuota = dash.show_quota_gauges ?? widgetKit !== 'echarts'
  const showFaq = dash.show_faq ?? false

  const palette = useMemo(
    () => readZenkoChartPalette(),
    [ui.colors, ui.mode, ui.typography],
  )
  const heatmapOpt = useMemo(
    () =>
      buildHeatmapOption(
        PREVIEW_ACTIVITY_POINTS,
        palette,
        dash.heatmap_shape,
        dash.heatmap_range,
        true,
      ),
    [palette, dash.heatmap_shape, dash.heatmap_range],
  )
  const previewGrid = useMemo(
    () => buildActivityGridSeries(PREVIEW_ACTIVITY_POINTS, 98),
    [],
  )
  const ramp = buildAccentHeatmapRamp(palette)

  return (
    <div className="flex w-full max-w-[660px] flex-col gap-3">
      <div
        className={cn(
          'grid gap-2',
          dash.layout === 'editorial_stack'
            ? 'grid-cols-2'
            : dash.layout === 'compact_ops'
              ? 'grid-cols-4'
              : 'grid-cols-3',
        )}
      >
        {[
          ['VmRSS', '28.4 MB'],
          ['Storage', '4.2 MB'],
          ['zenko_vault', '3 keys'],
          ...(dash.layout === 'compact_ops' ? [['Cache Hit', '82.1%']] : []),
        ].map(([k, v]) => (
          <div key={k} className="z-surface-card flex flex-col gap-0.5 !p-2.5">
            <span className="font-mono text-[9.5px] uppercase text-[var(--muted)]">{k}</span>
            <span className="font-mono text-[14px] font-semibold text-[var(--fg)]">{v}</span>
          </div>
        ))}
      </div>

      {dash.show_heatmap &&
        (widgetKit === 'motion_brew' ? (
          <ActivityGrid
            title="Zenko Activity Contribution"
            data={previewGrid}
            unit="events"
            cellSize={10}
            colors={[ramp[1], ramp[2], ramp[3], ramp[4]]}
            className="zenko-mb-card w-full"
          />
        ) : (
          <div className="z-surface-card flex flex-col gap-1.5 !p-3">
            <div className="flex items-center justify-between text-[11px]">
              <span className="font-medium text-[var(--fg)]">Zenko Activity Contribution</span>
              <div className="flex items-center gap-1 font-mono text-[9.5px] text-[var(--muted)]">
                <span>Less</span>
                {ramp.map((c, i) => (
                  <span
                    key={i}
                    className="inline-block h-2.5 w-2.5"
                    style={{
                      backgroundColor: c,
                      borderRadius:
                        dash.heatmap_shape === 'circle'
                          ? 99
                          : dash.heatmap_shape === 'square'
                            ? 0
                            : 2,
                    }}
                  />
                ))}
                <span>More</span>
              </div>
            </div>
            <ZenkoChart option={heatmapOpt} height={112} />
          </div>
        ))}

      {showQuota && (
        <QuotaGauge
          value={82}
          goal={100}
          unit="% KV cache"
          color={palette.accent}
          className="zenko-mb-card w-full"
        />
      )}

      <div className="flex flex-wrap items-center gap-1.5 font-mono text-[10px] text-[var(--muted)]">
        <span>kit:{widgetKit}</span>
        <span>·</span>
        <span>layout:{dash.layout}</span>
        <span>·</span>
        <span>charts:{dash.chart_theme}</span>
        <span>·</span>
        <span>
          modules:
          {[
            dash.show_heatmap && 'heatmap',
            showRecap && 'recap',
            showQuota && 'gauges',
            dash.show_token_trends && 'trends',
            dash.show_storage_vault && 'vault',
            dash.show_tools_table && 'tools',
            dash.show_logs && 'logs',
            showFaq && 'faq',
          ]
            .filter(Boolean)
            .join(', ')}
        </span>
      </div>
    </div>
  )
}

export function DashboardView({
  ui,
  telemetry,
  logs,
  jobs = [],
  subagents = [],
  subagentRoles = [],
  taskKinds = [],
  logCategories,
  logCounts,
  logsDir,
  onOpenCustomizeDashboard,
  onKillAgent,
  onMergeAgent,
  onDiscardAgent,
  onPruneWorktrees,
  onMessageAgent,
  onInspectAgent,
  onStopJob,
  onRestartJob,
  onDeleteJob,
  onPruneJobs,
  onQueryLogs,
  onRefresh,
}: {
  ui: UiCustomization
  telemetry?: DaemonTelemetry
  logs: LogEntry[]
  jobs?: BackgroundJob[]
  subagents?: SubagentRecord[]
  subagentRoles?: SubagentRoleDescriptor[]
  taskKinds?: TaskKindMeta[]
  logCategories: LogCategoryMeta[]
  logCounts: Record<string, number>
  logsDir: string
  onOpenCustomizeDashboard?: () => void
  onKillAgent?: (agentId: string) => void
  onMergeAgent?: (agentId: string) => void
  onDiscardAgent?: (agentId: string) => void
  onPruneWorktrees?: () => void
  onMessageAgent?: (agentId: string, message: string, interrupt: boolean) => Promise<void> | void
  onInspectAgent?: (agentId: string) => void
  onStopJob?: (jobId: string) => void
  onRestartJob?: (jobId: string) => void
  onDeleteJob?: (jobId: string) => void
  onPruneJobs?: (kind: string) => void
  onQueryLogs: (params: {
    category: string
    level: string
    sessionId: string
    search: string
  }) => void
  onRefresh: () => void
}) {
  const [data, setData] = useState<DashboardAnalytics | null>(null)
  const [loading, setLoading] = useState(false)

  const dash = ui.components.dashboard || {
    layout: 'bento_grid' as const,
    chart_theme: 'monochrome_accent' as const,
    heatmap_shape: 'rounded' as const,
    heatmap_range: 'year' as const,
    widget_kit: 'echarts' as const,
    show_heatmap: true,
    show_storage_vault: true,
    show_token_trends: true,
    show_tools_table: true,
    show_subagents_jobs: true,
    show_logs: true,
    show_recap_card: false,
    show_quota_gauges: false,
    show_faq: false,
  }
  const widgetKit = dash.widget_kit || 'echarts'
  const showRecap = dash.show_recap_card ?? widgetKit !== 'echarts'
  const showQuota = dash.show_quota_gauges ?? widgetKit !== 'echarts'
  const showFaq = dash.show_faq ?? false

  const loadDashboard = useCallback(async () => {
    setLoading(true)
    try {
      const res = await api<DashboardAnalytics>('/api/dashboard')
      setData(res)
    } catch {
      /* ignore transient fetch errors */
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadDashboard()
  }, [loadDashboard])

  const palette = useMemo(
    () => readZenkoChartPalette(),
    [ui.colors, ui.mode, ui.typography, ui.surface.card_style],
  )
  const ramp = useMemo(() => buildAccentHeatmapRamp(palette), [palette])

  const activeTelemetry = data?.telemetry ?? telemetry
  const rssMb = activeTelemetry ? (activeTelemetry.rss_bytes / (1024 * 1024)).toFixed(1) : '0.0'
  const totalStorage = data ? formatBytes(data.total_storage_bytes) : formatBytes(activeTelemetry?.db_size_bytes ?? 0)
  const vault = data?.vault

  const heatmapOption = useMemo(
    () =>
      buildHeatmapOption(
        data?.daily_activity || [],
        palette,
        dash.heatmap_shape,
        dash.heatmap_range,
      ),
    [data?.daily_activity, palette, dash.heatmap_shape, dash.heatmap_range],
  )

  const tokenTrendOption = useMemo(
    () => buildTokenTrendOption(data?.daily_activity || [], palette, dash.chart_theme),
    [data?.daily_activity, palette, dash.chart_theme],
  )

  const storageDonutOption = useMemo(
    () => buildStorageDonutOption(data?.storage || [], palette, dash.chart_theme),
    [data?.storage, palette, dash.chart_theme],
  )

  const toolsBarOption = useMemo(
    () => buildToolsBarOption(data?.top_tools || [], palette, dash.chart_theme),
    [data?.top_tools, palette, dash.chart_theme],
  )

  const activityGridDays = dash.heatmap_range === 'quarter' ? 91 : 182
  const activityGridData = useMemo(
    () => buildActivityGridSeries(data?.daily_activity || [], activityGridDays),
    [data?.daily_activity, activityGridDays],
  )
  const mixRanges = useMemo(
    () => buildMixChartRanges(data?.daily_activity || []),
    [data?.daily_activity],
  )
  const mixSeries = useMemo(
    () => [
      { key: 'cached', label: 'KV Cached', color: palette.accent },
      { key: 'uncached', label: 'Uncached', color: hexWithAlpha(palette.fg, 0.52) },
      { key: 'tools', label: 'Tool Load', color: hexWithAlpha(palette.accent, 0.45) },
    ],
    [palette],
  )
  const recapPeriod = useMemo(
    () => new Date().toLocaleString('en-US', { month: 'long', year: 'numeric' }),
    [],
  )
  const recapParts = useMemo(
    () => [
      'Zenko processed ',
      { value: Math.round((data?.total_tokens ?? 0) / 1000), suffix: 'k tokens', icon: 'words' as const },
      ' across ',
      { value: data?.active_days ?? 0, suffix: ' active days', icon: 'sun' as const },
      ' with a ',
      { value: data?.streak_days ?? 0, suffix: '-day streak', icon: 'streak' as const },
      ' and ',
      {
        value: Number((activeTelemetry?.cache_hit_pct ?? 0).toFixed(1)),
        decimals: 1,
        suffix: '% KV cache hit',
        icon: 'trend' as const,
      },
      '.',
    ],
    [data?.total_tokens, data?.active_days, data?.streak_days, activeTelemetry?.cache_hit_pct],
  )
  const recapBadges = useMemo(
    () => [
      {
        label: 'Tool Mastery',
        icon: 'spark' as const,
        note: `${data?.counts.tool_calls ?? 0} tool invocations executed cleanly.`,
        progress: Math.min(1, (data?.counts.tool_calls ?? 0) / 100),
      },
      {
        label: 'KV Cache Saver',
        icon: 'trend' as const,
        note: `${formatCompactTokens(data?.total_cache_read_tokens ?? 0)} tokens served from cache.`,
        progress: Math.min(1, (activeTelemetry?.cache_hit_pct ?? 0) / 100),
      },
      {
        label: 'Vault Shield',
        icon: 'feather' as const,
        note: vault?.encrypted
          ? `${vault.key_count} encrypted secret slot(s) locked via ${vault.cipher}.`
          : 'Vault initialized and ready for credentials.',
        progress: vault?.encrypted ? 1 : 0.35,
      },
    ],
    [
      data?.counts.tool_calls,
      data?.total_cache_read_tokens,
      activeTelemetry?.cache_hit_pct,
      vault?.encrypted,
      vault?.key_count,
      vault?.cipher,
    ],
  )

  const storageRows = useMemo(() => {
    const total = Math.max(1, data?.total_storage_bytes ?? 0)
    return (data?.storage || []).map((s) => ({
      component: s.label,
      size: formatBytes(s.bytes),
      files: s.files,
      share: `${((s.bytes / total) * 100).toFixed(1)}%`,
    }))
  }, [data?.storage, data?.total_storage_bytes])

  const toolRows = useMemo(
    () =>
      (data?.top_tools || []).map((t) => ({
        tool: t.name,
        calls: t.calls,
        errors: t.errors,
        latency: `${t.avg_duration_ms}ms`,
      })),
    [data?.top_tools],
  )

  const modelRows = useMemo(
    () =>
      (data?.top_models || []).map((m) => ({
        model: m.model.includes('/') ? m.model.split('/').pop() || m.model : m.model,
        calls: m.calls,
        tokens: formatCompactTokens(m.tokens),
        cached: m.tokens > 0 ? `${Math.round((m.cache_read_tokens / m.tokens) * 100)}%` : '0%',
        cost: `$${m.cost_usd.toFixed(4)}`,
      })),
    [data?.top_models],
  )

  const gridTwoCol =
    dash.layout === 'editorial_stack'
      ? 'grid-cols-1'
      : 'grid-cols-1 lg:grid-cols-2'

  const kpiGrid =
    dash.layout === 'compact_ops'
      ? 'grid-cols-2 sm:grid-cols-3 lg:grid-cols-6'
      : dash.layout === 'editorial_stack'
        ? 'grid-cols-2 sm:grid-cols-3'
        : 'grid-cols-2 sm:grid-cols-3 lg:grid-cols-6'

  return (
    <CompactScrollArea
      smooth={ui.motion.smooth_scroll !== false}
      className="min-h-0 flex-1"
      viewportClassName="px-6 py-8 pr-20 sm:px-10"
    >
      <div className="mx-auto flex w-full max-w-[1120px] flex-col gap-6">
        {/* Header */}
        <div className="flex flex-wrap items-end justify-between gap-3">
          <div>
            <div className="flex items-center gap-2.5">
              <h1
                className="text-[24px] font-semibold tracking-[-0.03em] text-[var(--fg)]"
                style={{ fontFamily: 'var(--font-display)' }}
              >
                Dashboard
              </h1>
              <span className="rounded-full border border-[var(--border)] bg-[var(--surface)] px-2.5 py-0.5 font-mono text-[10.5px] text-[var(--muted)]">
                PID {activeTelemetry?.pid ?? '—'} · up {formatUptime(activeTelemetry?.uptime_secs ?? 0)}
              </span>
              {activeTelemetry?.telegram_connected && (
                <span
                  className="rounded-full border px-2.5 py-0.5 font-mono text-[10.5px]"
                  style={{
                    color: 'var(--accent)',
                    borderColor: 'color-mix(in srgb, var(--accent) 40%, var(--border))',
                    backgroundColor: 'color-mix(in srgb, var(--accent) 8%, transparent)',
                  }}
                >
                  @{activeTelemetry.telegram_bot_username || 'telegram'}
                </span>
              )}
            </div>
            <p className="mt-1 text-[13px] text-[var(--muted)]">
              Live Zenko usage heatmap, token & KV-cache telemetry, storage footprint, encrypted vault status, and flight recorder.
            </p>
          </div>
          <div className="flex items-center gap-2">
            {onOpenCustomizeDashboard && (
              <ZenkoTooltip label="Customize Dashboard charts & modules" side="bottom">
                <button
                  type="button"
                  onClick={onOpenCustomizeDashboard}
                  className="z-btn z-btn-ghost z-btn-sm"
                >
                  <Palette size={13} /> Customize
                </button>
              </ZenkoTooltip>
            )}
            <ZenkoTooltip label="Refresh live telemetry & analytics" side="bottom">
              <button
                type="button"
                onClick={() => {
                  loadDashboard()
                  onRefresh()
                }}
                className="z-btn z-btn-secondary z-btn-sm"
              >
                <RefreshCw size={13} className={loading ? 'animate-spin' : ''} /> Refresh
              </button>
            </ZenkoTooltip>
          </div>
        </div>

        {/* 6-KPI Live Strip */}
        <div className={cn('grid gap-3', kpiGrid)}>
          {[
            {
              label: 'Memory (VmRSS)',
              val: `${rssMb} MB`,
              sub: `${activeTelemetry?.active_jobs ?? 0} active jobs`,
              icon: Cpu,
            },
            {
              label: 'Total Storage',
              val: totalStorage,
              sub: `${data?.storage.reduce((a, b) => a + b.files, 0) ?? 0} files tracked`,
              icon: Activity,
            },
            {
              label: 'zenko_vault',
              val: vault ? `${vault.key_count} ${vault.key_count === 1 ? 'key' : 'keys'}` : '—',
              sub: vault?.encrypted ? 'ChaCha20 locked' : 'No secrets yet',
              icon: ShieldCheck,
            },
            {
              label: 'KV Cache Hit',
              val: `${(activeTelemetry?.cache_hit_pct ?? 0).toFixed(1)}%`,
              sub: `${formatCompactTokens(data?.total_cache_read_tokens ?? 0)} cached`,
              icon: Zap,
            },
            {
              label: 'Spend Ledger',
              val: `$${(activeTelemetry?.daily_cost_usd ?? 0).toFixed(4)}`,
              sub: `Total $${(activeTelemetry?.total_cost_usd ?? 0).toFixed(4)}`,
              icon: Terminal,
            },
            {
              label: 'Activity Streak',
              val: `${data?.streak_days ?? 0}d`,
              sub: `${data?.counts.sessions ?? 0} sessions · ${data?.counts.tool_calls ?? 0} tools`,
              icon: KeyRound,
            },
          ].map((kpi) => {
            const Icon = kpi.icon
            return (
              <div key={kpi.label} className="z-surface-card flex flex-col gap-1.5">
                <div className="flex items-center justify-between text-[var(--muted)]">
                  <span className="font-mono text-[10.5px] uppercase tracking-wider">
                    {kpi.label}
                  </span>
                  <Icon size={13} className="opacity-70" />
                </div>
                <span className="font-mono text-[20px] font-semibold tracking-tight text-[var(--fg)]">
                  {kpi.val}
                </span>
                <span className="truncate font-mono text-[10.5px] text-[var(--muted)]">
                  {kpi.sub}
                </span>
              </div>
            )
          })}
        </div>

        {/* 0. Motion-Brew Narrative Recap Card & Runtime Quota Gauges */}
        {showRecap && (
          <RecapCard
            title="Zenko runtime recap"
            period={recapPeriod}
            parts={recapParts}
            badges={recapBadges}
            actionLabel={onOpenCustomizeDashboard ? 'Customize dashboard' : null}
            onAction={onOpenCustomizeDashboard}
            className="zenko-mb-card w-full"
          />
        )}

        {showQuota && (
          <div className={cn('grid gap-5', gridTwoCol)}>
            <QuotaGauge
              value={Math.round(activeTelemetry?.cache_hit_pct ?? 0)}
              goal={100}
              unit="% KV cache"
              color={palette.accent}
              className="zenko-mb-card w-full"
            />
            <UsageRing
              title="Runtime Budgets"
              metrics={[
                { label: 'VmRSS', value: Number(rssMb) || 0, goal: 64, unit: 'MB' },
                {
                  label: 'KV Cache',
                  value: Math.round(activeTelemetry?.cache_hit_pct ?? 0),
                  goal: 100,
                  unit: '%',
                },
                {
                  label: 'Skills',
                  value: data?.counts.skills_enabled ?? 0,
                  goal: Math.max(1, data?.counts.skills_total ?? 1),
                  unit: 'on',
                },
              ]}
              className="zenko-mb-card w-full"
            />
          </div>
        )}

        {/* 1. GitHub-Style Zenko Usage Activity Contribution Heatmap (ECharts | Motion-Brew | Hybrid) */}
        {dash.show_heatmap && (
          <div className="flex flex-col gap-4">
            {widgetKit !== 'motion_brew' && (
              <div className="z-surface-card flex flex-col gap-3">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div>
                    <h2 className="text-[14px] font-semibold text-[var(--fg)]">
                      Zenko Activity Contribution
                    </h2>
                    <p className="font-mono text-[11px] text-[var(--muted)]">
                      {data?.active_days ?? 0} active days · {data?.streak_days ?? 0}-day streak ·{' '}
                      {formatCompactTokens(data?.total_tokens ?? 0)} total tokens ·{' '}
                      {data?.counts.tool_calls ?? 0} tool runs
                    </p>
                  </div>
                  <div className="flex items-center gap-1.5 font-mono text-[10.5px] text-[var(--muted)]">
                    <span>Less</span>
                    {ramp.map((c, idx) => (
                      <span
                        key={idx}
                        className="inline-block h-3 w-3"
                        style={{
                          backgroundColor: c,
                          borderRadius:
                            dash.heatmap_shape === 'circle'
                              ? 99
                              : dash.heatmap_shape === 'square'
                                ? 0
                                : 3,
                        }}
                      />
                    ))}
                    <span>More</span>
                  </div>
                </div>
                <ZenkoChart
                  option={heatmapOption}
                  height={dash.heatmap_range === 'year' ? 168 : 184}
                />
              </div>
            )}
            {(widgetKit === 'motion_brew' || widgetKit === 'hybrid') && (
              <ActivityGrid
                title="Zenko Activity Grid"
                data={activityGridData}
                unit="events"
                colors={[ramp[1], ramp[2], ramp[3], ramp[4]]}
                className="zenko-mb-card w-full"
              />
            )}
          </div>
        )}

        {/* 2. Storage & zenko_vault Visual Inspector */}
        {dash.show_storage_vault && (
          <div className={cn('grid gap-5', gridTwoCol)}>
            <div className="z-surface-card flex flex-col gap-3">
              <div className="flex items-center justify-between gap-2">
                <div>
                  <h2 className="text-[14px] font-semibold text-[var(--fg)]">
                    Storage Breakdown
                  </h2>
                  <p className="font-mono text-[11px] text-[var(--muted)]">
                    Real disk usage across SQLite WAL, Vault, Logs, Artifacts & Hub ({totalStorage})
                  </p>
                </div>
              </div>
              <div className="grid grid-cols-1 items-center gap-4 sm:grid-cols-[175px_1fr]">
                <ZenkoChart option={storageDonutOption} height={175} />
                <DataTable
                  columns={[
                    { key: 'component', label: 'Store', priority: 'primary' },
                    { key: 'size', label: 'Size', align: 'end' },
                    { key: 'share', label: 'Share', align: 'end' },
                  ]}
                  rows={storageRows}
                />
              </div>
            </div>

            <div className="z-surface-card flex flex-col gap-3.5">
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <ShieldCheck size={16} style={{ color: 'var(--accent)' }} />
                  <h2 className="text-[14px] font-semibold text-[var(--fg)]">
                    zenko_vault Security & Inventory
                  </h2>
                </div>
                <span
                  className="rounded-full border px-2 py-0.5 font-mono text-[10px]"
                  style={{
                    color: vault?.encrypted ? 'var(--accent)' : 'var(--muted)',
                    borderColor: vault?.encrypted
                      ? 'color-mix(in srgb, var(--accent) 40%, var(--border))'
                      : 'var(--border)',
                    backgroundColor: vault?.encrypted
                      ? 'color-mix(in srgb, var(--accent) 10%, transparent)'
                      : 'transparent',
                  }}
                >
                  {vault?.encrypted ? `${vault.cipher} · 0600` : 'Initialized'}
                </span>
              </div>

              <div className="grid grid-cols-3 gap-2 font-mono text-[11px]">
                <div className="z-card-item flex flex-col gap-0.5 p-2.5">
                  <span className="text-[10px] text-[var(--muted)]">Stored Secrets</span>
                  <span className="text-[15px] font-semibold text-[var(--fg)]">
                    {vault?.key_count ?? 0}
                  </span>
                </div>
                <div className="z-card-item flex flex-col gap-0.5 p-2.5">
                  <span className="text-[10px] text-[var(--muted)]">Vault Size</span>
                  <span className="text-[15px] font-semibold text-[var(--fg)]">
                    {formatBytes(vault?.size_bytes ?? 0)}
                  </span>
                </div>
                <div className="z-card-item flex flex-col gap-0.5 p-2.5">
                  <span className="text-[10px] text-[var(--muted)]">Redaction</span>
                  <span className="text-[15px] font-semibold text-[var(--fg)]">Active</span>
                </div>
              </div>

              <div className="flex flex-col gap-1.5">
                <span className="font-mono text-[10.5px] uppercase tracking-wider text-[var(--muted)]">
                  Encrypted Key Slots (Values Never Exposed)
                </span>
                {vault && vault.key_names.length > 0 ? (
                  <div className="flex flex-wrap gap-1.5">
                    {vault.key_names.map((name) => (
                      <span
                        key={name}
                        className="inline-flex items-center gap-1.5 rounded-full border border-[var(--border)] bg-[var(--bg)]/70 px-2.5 py-0.5 font-mono text-[10.5px] text-[var(--fg)]/90"
                      >
                        <KeyRound size={10.5} style={{ color: 'var(--accent)' }} />
                        {name}
                      </span>
                    ))}
                  </div>
                ) : (
                  <span className="font-mono text-[11px] text-[var(--muted)]">
                    No API keys or tokens stored in vault.enc yet.
                  </span>
                )}
              </div>

              {/* Workspace Entity Inventory */}
              <div className="mt-auto grid grid-cols-3 gap-2 border-t border-[var(--border)]/60 pt-3 font-mono text-[11px] sm:grid-cols-6">
                {[
                  ['Memory', data?.counts.memories ?? 0],
                  ['Skills', `${data?.counts.skills_enabled ?? 0}/${data?.counts.skills_total ?? 0}`],
                  ['Cron', data?.counts.cron_jobs ?? 0],
                  ['Goals', data?.counts.goal_loops ?? 0],
                  ['MCP', data?.counts.mcp_servers ?? 0],
                  ['Checkpoints', data?.counts.checkpoints ?? 0],
                ].map(([label, val]) => (
                  <div key={String(label)} className="flex flex-col">
                    <span className="text-[10px] text-[var(--muted)]">{label}</span>
                    <span className="font-semibold text-[var(--fg)]">{val}</span>
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}

        {/* 3. Token & KV-Cache Trend + Top Tools Bar Chart */}
        {(dash.show_token_trends || dash.show_tools_table) && (
          <div className={cn('grid gap-5', gridTwoCol)}>
            {dash.show_token_trends &&
              (widgetKit === 'motion_brew' ? (
                <MixChart
                  title="Token Volume & KV-Cache Mix"
                  series={mixSeries}
                  ranges={mixRanges}
                  unit="k"
                  decimals={1}
                  className="zenko-mb-card w-full"
                />
              ) : (
                <div className="z-surface-card flex flex-col gap-2">
                  <div>
                    <h2 className="text-[14px] font-semibold text-[var(--fg)]">
                      Token Volume & KV-Cache Savings
                    </h2>
                    <p className="font-mono text-[11px] text-[var(--muted)]">
                      Daily total tokens vs prompt-cache hits across active turns
                    </p>
                  </div>
                  <ZenkoChart option={tokenTrendOption} height={210} />
                </div>
              ))}

            {dash.show_tools_table && (
              <div className="z-surface-card flex flex-col gap-2">
                <div>
                  <h2 className="text-[14px] font-semibold text-[var(--fg)]">
                    Top Tool Executions
                  </h2>
                  <p className="font-mono text-[11px] text-[var(--muted)]">
                    Most frequently invoked native & MCP tools ({data?.counts.tool_calls ?? 0} total)
                  </p>
                </div>
                {(data?.top_tools?.length ?? 0) > 0 ? (
                  <ZenkoChart option={toolsBarOption} height={210} />
                ) : (
                  <div className="flex h-[210px] items-center justify-center font-mono text-[11.5px] text-[var(--muted)]">
                    Run a chat turn with tools to populate tool analytics.
                  </div>
                )}
              </div>
            )}
          </div>
        )}

        {dash.show_token_trends && widgetKit === 'hybrid' && (
          <MixChart
            title="Interactive Token & Tool Mix"
            series={mixSeries}
            ranges={mixRanges}
            unit="k"
            decimals={1}
            className="zenko-mb-card w-full"
          />
        )}

        {/* 4. Tool Latency & Model Usage Tables */}
        {dash.show_tools_table && (toolRows.length > 0 || modelRows.length > 0) && (
          <div className={cn('grid gap-5', gridTwoCol)}>
            {toolRows.length > 0 && (
              <div className="z-surface-card flex flex-col gap-2.5">
                <h2 className="text-[13.5px] font-semibold text-[var(--fg)]">
                  Tool Latency & Reliability
                </h2>
                <DataTable
                  columns={[
                    { key: 'tool', label: 'Tool', priority: 'primary' },
                    { key: 'calls', label: 'Calls', align: 'end' },
                    { key: 'errors', label: 'Errors', align: 'end' },
                    { key: 'latency', label: 'Avg', align: 'end' },
                  ]}
                  rows={toolRows}
                />
              </div>
            )}
            {modelRows.length > 0 && (
              <div className="z-surface-card flex flex-col gap-2.5">
                <h2 className="text-[13.5px] font-semibold text-[var(--fg)]">
                  Model Usage & Spend
                </h2>
                <DataTable
                  columns={[
                    { key: 'model', label: 'Model', priority: 'primary' },
                    { key: 'calls', label: 'Calls', align: 'end' },
                    { key: 'tokens', label: 'Tokens', align: 'end' },
                    { key: 'cached', label: 'Cache', align: 'end' },
                    { key: 'cost', label: 'Cost', align: 'end' },
                  ]}
                  rows={modelRows}
                />
              </div>
            )}
          </div>
        )}

        {/* 5. Embedded Operations (Subagents, Background Tasks & Categorized Flight Recorder) */}
        {(dash.show_subagents_jobs || dash.show_logs) && (
          <TelemetryView
            embedded
            showSubagentsAndJobs={dash.show_subagents_jobs}
            showLogs={dash.show_logs}
            telemetry={activeTelemetry}
            logs={logs}
            jobs={jobs}
            subagents={subagents}
            subagentRoles={subagentRoles}
            taskKinds={taskKinds}
            logCategories={logCategories}
            logCounts={logCounts}
            logsDir={logsDir}
            tabsVariant={ui.surface.tabs_variant || 'pill'}
            smoothScroll={ui.motion.smooth_scroll !== false}
            onKillAgent={onKillAgent}
            onMergeAgent={onMergeAgent}
            onDiscardAgent={onDiscardAgent}
            onPruneWorktrees={onPruneWorktrees}
            onMessageAgent={onMessageAgent}
            onInspectAgent={onInspectAgent}
            onStopJob={onStopJob}
            onRestartJob={onRestartJob}
            onDeleteJob={onDeleteJob}
            onPruneJobs={onPruneJobs}
            onQueryLogs={onQueryLogs}
            onRefresh={() => {
              loadDashboard()
              onRefresh()
            }}
          />
        )}

        {/* 6. Interactive Runtime FAQ (RareUI FAQ — opt-in via Theme Studio) */}
        {showFaq && (
          <div className="z-surface-card flex flex-col gap-3">
            <div>
              <h2 className="text-[14px] font-semibold text-[var(--fg)]">
                Zenko Architecture &amp; FAQ
              </h2>
              <p className="font-mono text-[11px] text-[var(--muted)]">
                Click any question to expand live runtime notes
              </p>
            </div>
            <RareFaq
              items={ZENKO_DASHBOARD_FAQ}
              defaultValue={ZENKO_DASHBOARD_FAQ[0].id}
              accent="var(--accent)"
              avatar={<ZenkoFoxLogo size={18} variant="fox" />}
            />
          </div>
        )}
      </div>
    </CompactScrollArea>
  )
}
