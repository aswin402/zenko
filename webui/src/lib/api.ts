import type { ComponentSlots, UiCustomization } from './types'
import {
  hydrateSavedCustomFonts,
  resolveMonoFontStack,
  resolveUiFontStack,
} from './fonts'

export const DEFAULT_COMPONENT_SLOTS: ComponentSlots = {
  composer: {
    shell_variant: 'floating_card',
    model_picker_style: 'drilldown',
    send_button_shape: 'rounded_square',
    send_button_icon: 'arrow_up',
    context_meter: 'progress_ring',
    action_pills: 'soft_tint',
    show_attach: true,
    show_policy: true,
    show_streaming_toggle: true,
  },
  left_menubar: {
    frame_style: 'flush_border',
    new_session_style: 'vermilion_soft',
    session_item_style: 'soft_fill',
    show_telemetry_badge: true,
  },
  right_menubar: {
    dock_style: 'floating_capsule',
    active_indicator: 'vermilion_pill',
  },
  chat_page: {
    background: 'clean',
    content_width: 'balanced',
    user_bubble: 'soft_card',
    thinking_style: 'collapsible_steps',
    tool_card_style: 'aui_smart',
    show_avatars: true,
    agent_avatar_frame: 'squircle',
    agent_avatar_color: '',
    user_avatar: 'icon_user',
    user_avatar_frame: 'circle',
    user_avatar_color: '',
    scroller_style: 'beui_rail',
    streaming_style: 'beui_stream',
    message_actions_style: 'aui_message_actions',
    approval_style: 'aui_permission_grant',
    activity_style: 'aui_tool_call_icon',
    loading_style: 'beui_loader',
    diff_style: 'aui_code_diff',
    shell_style: 'aui_terminal_block',
    code_block_style: 'aui_code_runner',
    show_day_separators: true,
    loader_variant: 'dot-matrix',
    show_citations: true,
    skill_badge_style: 'aui_memory_chips',
  },
  dashboard: {
    layout: 'bento_grid',
    chart_theme: 'monochrome_accent',
    heatmap_shape: 'rounded',
    heatmap_range: 'year',
    show_heatmap: true,
    show_storage_vault: true,
    show_token_trends: true,
    show_tools_table: true,
    show_subagents_jobs: true,
    show_logs: true,
  },
}

export function normalizeUiCustomization(cfg: UiCustomization): UiCustomization {
  const c = cfg.components
  return {
    ...cfg,
    surface: {
      scrollbar_style: 'accent_thin',
      tabs_variant: 'pill',
      accordion_style: 'bouncy',
      alert_style: 'beui_alert',
      modal_style: 'center_morph',
      popover_style: 'gooey',
      element_chrome: 'minimal',
      table_style: 'borderless',
      tooltip_style: 'accent_edge',
      tooltip_shape: 'pill',
      tooltip_motion: 'spring',
      show_hover_hints: true,
      ...(cfg.surface || {}),
    },
    motion: {
      theme_transition: 'circle-blur',
      smooth_scroll: true,
      ...(cfg.motion || {}),
    },
    components: {
      composer: { ...DEFAULT_COMPONENT_SLOTS.composer, ...(c?.composer || {}) },
      left_menubar: { ...DEFAULT_COMPONENT_SLOTS.left_menubar, ...(c?.left_menubar || {}) },
      right_menubar: { ...DEFAULT_COMPONENT_SLOTS.right_menubar, ...(c?.right_menubar || {}) },
      chat_page: { ...DEFAULT_COMPONENT_SLOTS.chat_page, ...(c?.chat_page || {}) },
      dashboard: { ...DEFAULT_COMPONENT_SLOTS.dashboard!, ...(c?.dashboard || {}) },
    },
  }
}

export function getByPath(obj: any, path: string): any {
  return path.split('.').reduce((acc, key) => (acc == null ? undefined : acc[key]), obj)
}

const FORBIDDEN_PATH_KEYS = new Set(['__proto__', 'constructor', 'prototype'])

export function setByPath(obj: any, path: string, value: any): void {
  const parts = path.split('.')
  let cur = obj
  for (let i = 0; i < parts.length - 1; i += 1) {
    const key = parts[i]
    if (FORBIDDEN_PATH_KEYS.has(key)) return
    if (cur[key] == null || typeof cur[key] !== 'object') {
      cur[key] = {}
    }
    cur = cur[key]
  }
  const leaf = parts[parts.length - 1]
  if (FORBIDDEN_PATH_KEYS.has(leaf)) return
  cur[leaf] = value
}

export async function api<T = any>(url: string, opts: RequestInit = {}): Promise<T> {
  const { headers, ...rest } = opts
  const res = await fetch(url, {
    ...rest,
    headers: {
      'Content-Type': 'application/json',
      ...(headers || {}),
    },
  })
  const text = await res.text()
  let data: any = {}
  try {
    data = text ? JSON.parse(text) : {}
  } catch {
    data = { raw: text }
  }
  if (!res.ok) {
    throw new Error(data.error || `Request failed (${res.status})`)
  }
  return data as T
}

export function applyUiCustomization(cfg: UiCustomization) {
  const root = document.documentElement
  const c = cfg.colors
  const set = (k: string, v: string) => root.style.setProperty(k, v)

  set('--bg', c.background)
  set('--surface', c.surface)
  set('--card', c.card)
  set('--border', c.border)
  set('--fg', c.foreground)
  set('--muted', c.muted)
  set('--primary', c.primary)
  set('--primary-fg', c.primary_foreground)
  set('--accent', c.accent)
  set('--destructive', c.destructive)
  set('--success', c.success)
  set('--warning', c.warning)

  const radius = Math.min(24, Math.max(0, Number(cfg.surface.radius_px ?? 10)))
  set('--radius', `${radius}px`)
  set('--radius-sm', `${Math.max(2, radius - 4)}px`)
  set('--radius-lg', `${Math.min(28, radius + 6)}px`)

  const baseSize = Math.min(18, Math.max(12, Number(cfg.typography.base_size_px ?? 14)))
  set('--text-base', `${baseSize}px`)
  set('--icon-size', `${Math.min(24, Math.max(12, Number(cfg.icons.size_px ?? 16)))}px`)
  set('--icon-stroke', `${Math.min(2.5, Math.max(1.0, Number(cfg.icons.stroke_width ?? 1.75)))}`)

  hydrateSavedCustomFonts().catch(() => {})
  const uiFontStack = resolveUiFontStack(cfg.typography.ui_font, cfg.typography.custom_ui_font)
  const monoFontStack = resolveMonoFontStack(
    cfg.typography.mono_font,
    cfg.typography.custom_mono_font,
  )
  set('--font-sans', uiFontStack)
  set('--font-display', uiFontStack)
  set('--font-mono', monoFontStack)

  const motionMap: Record<string, [string, string]> = {
    snappy: ['150ms', 'cubic-bezier(0.16, 1, 0.3, 1)'],
    fluid_spring: ['260ms', 'cubic-bezier(0.34, 1.4, 0.64, 1)'],
    reduced: ['80ms', 'linear'],
    instant: ['0ms', 'linear'],
  }
  const [ms, ease] = motionMap[cfg.motion.preset] || motionMap.snappy
  set('--motion-ms', ms)
  set('--motion-ease', ease)

  root.dataset.mode = cfg.mode
  root.classList.toggle('dark', cfg.mode !== 'light')
  root.dataset.surface = cfg.surface.card_style
  root.dataset.bgPattern = cfg.surface.bg_pattern
  root.dataset.scrollbar = cfg.surface.scrollbar_style || 'accent_thin'
  root.dataset.tabsVariant = cfg.surface.tabs_variant || 'pill'
  root.dataset.elementChrome = cfg.surface.element_chrome || 'minimal'
  root.dataset.tableStyle = cfg.surface.table_style || 'borderless'
  root.dataset.tooltipStyle = cfg.surface.tooltip_style || 'accent_edge'
  root.dataset.tooltipShape = cfg.surface.tooltip_shape || 'pill'
  root.dataset.tooltipMotion = cfg.surface.tooltip_motion || 'spring'
  root.dataset.btnStyle = cfg.buttons.style
  root.dataset.btnShape = cfg.buttons.shape
  root.dataset.btnSize = cfg.buttons.size
  root.dataset.btnMotion = cfg.buttons.motion
  root.dataset.iconStyle = cfg.icons.style
  root.dataset.iconPack = cfg.icons.pack
  root.dataset.density = cfg.layout.density
  root.dataset.sidebar = cfg.layout.sidebar_mode
  root.dataset.chatLayout = cfg.layout.chat_layout
  root.dataset.orbStyle = cfg.motion.orb_style
  root.dataset.themeTransition = cfg.motion.theme_transition || 'circle-blur'

  updateDynamicFoxFavicon(c.accent)
}

function sanitizeSvgColor(raw: string): string {
  const trimmed = String(raw || '').trim()
  if (!trimmed || trimmed.length > 64) return '#ea5829'
  for (let i = 0; i < trimmed.length; i += 1) {
    const ch = trimmed.charCodeAt(i)
    const ok =
      (ch >= 48 && ch <= 57) || // 0-9
      (ch >= 65 && ch <= 90) || // A-Z
      (ch >= 97 && ch <= 122) || // a-z
      ch === 35 || // #
      ch === 40 || // (
      ch === 41 || // )
      ch === 37 || // %
      ch === 44 || // ,
      ch === 46 || // .
      ch === 32 || // space
      ch === 45 || // -
      ch === 47 // /
    if (!ok) return '#ea5829'
  }
  const lower = trimmed.toLowerCase()
  if (lower.includes('url(') || lower.includes('expression(') || lower.includes('//')) {
    return '#ea5829'
  }
  return trimmed
}

function updateDynamicFoxFavicon(accent: string) {
  if (typeof document === 'undefined') return
  const safeAccent = sanitizeSvgColor(accent)
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><path d="M16 8.2C19.2 8.2 22.8 5.2 25.4 3.2C26.2 2.6 27.1 3.1 27.2 4.1C27.4 7.4 27 10.6 26.5 12.5C28.5 14.3 29.6 16.5 29 18.8C28.1 22.1 22.4 26.6 17.4 28.4C16.5 28.7 15.5 28.7 14.6 28.4C9.6 26.6 3.9 22.1 3 18.8C2.4 16.5 3.5 14.3 5.5 12.5C5 10.6 4.6 7.4 4.8 4.1C4.9 3.1 5.8 2.6 6.6 3.2C9.2 5.2 12.8 8.2 16 8.2Z" fill="${safeAccent}"/></svg>`
  let link = document.querySelector<HTMLLinkElement>('link[rel="icon"]')
  if (!link) {
    link = document.createElement('link')
    link.rel = 'icon'
    link.type = 'image/svg+xml'
    document.head.appendChild(link)
  }
  link.href = `data:image/svg+xml,${encodeURIComponent(svg)}`
}

export function formatTimeShort(iso?: string): string {
  if (!iso) return ''
  try {
    return new Date(iso).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  } catch {
    return ''
  }
}

export function formatCompactTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return String(n)
}

const HTML_ESCAPE_MAP: Record<string, string> = {
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#39;',
}

export function escapeHtml(raw: string): string {
  const s = String(raw)
  let out = ''
  for (let i = 0; i < s.length; i += 1) {
    const ch = s[i]
    out += HTML_ESCAPE_MAP[ch] ?? ch
  }
  return out
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB'] as const
  let v = bytes
  let idx = 0
  while (v >= 1024 && idx < units.length - 1) {
    v /= 1024
    idx += 1
  }
  return idx === 0 ? `${Math.round(v)} B` : `${v.toFixed(v >= 100 ? 0 : 1)} ${units[idx]}`
}
