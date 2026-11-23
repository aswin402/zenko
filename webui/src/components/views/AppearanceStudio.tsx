import { useEffect, useMemo, useRef, useState } from 'react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import {
  ArrowLeft,
  ArrowRight,
  Check,
  ChevronDown,
  ChevronRight,
  Palette,
  RotateCcw,
  Search,
  Sliders,
  X,
} from 'lucide-react'
import { getByPath, setByPath } from '@/lib/api'
import { EASE_OUT } from '@/lib/ease'
import { resolveMonoFontStack, resolveUiFontStack } from '@/lib/fonts'
import { ICON_PACK_META_MAP } from '@/lib/icon-packs'
import type {
  ActiveView,
  ColorTokens,
  ModelInfo,
  ProviderDescriptor,
  StudioCategory,
  StudioControl,
  StudioSubComponent,
  ThemePresetMeta,
  UiCustomization,
} from '@/lib/types'
import { cn } from '@/lib/utils'
import { CUSTOMIZABLE_ICON_OPTIONS, ZenkoIcon } from '@/components/beui/ZenkoIcon'
import { AgentAvatar, UserAvatar } from '@/components/beui/ZenkoAvatars'
import { BouncyAccordion } from '@/components/motion/bouncy-accordion'
import {
  ColorPicker,
  ColorPickerArea,
  ColorPickerChannels,
  ColorPickerContent,
  ColorPickerEyeDropper,
  ColorPickerHexInput,
  ColorPickerHue,
  ColorPickerPreset,
  ColorPickerPresets,
  ColorPickerTrigger,
} from '@/components/motion/color-picker'
import {
  ColorSelector,
  ColorSelectorItem,
  ColorSelectorLabel,
  ColorSelectorList,
} from '@/components/motion/color-selector'
import { Loader, type LoaderVariant } from '@/components/motion/loader'
import { InlineSlider } from '@/components/motion/range-slider-inline'
import { ThemeToggle } from '@/components/motion/theme-toggle'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { CustomFontStudio } from '@/components/ui/CustomFontStudio'
import { ThemeSelector } from '@/components/ui/ThemeSelector'
import { ZenkoTooltip } from '@/components/ui/Tooltip'
import { ZenkoSelect } from '@/components/ui/ZenkoSelect'
import { renderStudioPreview } from '@/components/views/StudioPreviews'

const CURATED_ACCENT_SWATCHES: ReadonlyArray<{ id: string; label: string; color: string }> = [
  { id: '#ea5829', label: 'Zenko Vermilion', color: '#ea5829' },
  { id: '#ff5a26', label: 'OLED Flame', color: '#ff5a26' },
  { id: '#f59e0b', label: 'Amber Forge', color: '#f59e0b' },
  { id: '#10b981', label: 'Emerald Term', color: '#10b981' },
  { id: '#06b6d4', label: 'Arctic Cyan', color: '#06b6d4' },
  { id: '#818cf8', label: 'Midnight Indigo', color: '#818cf8' },
  { id: '#a855f7', label: 'Ultraviolet', color: '#a855f7' },
  { id: '#e11d48', label: 'Crimson Noir', color: '#e11d48' },
  { id: '#e4e4e7', label: 'Obsidian Mono', color: '#e4e4e7' },
]

interface CustomColorSlot {
  key: keyof ColorTokens
  label: string
  hint: string
  syncPrimaryAccent?: boolean
  presets: string[]
}

const CUSTOM_COLOR_SLOTS: ReadonlyArray<CustomColorSlot> = [
  {
    key: 'accent',
    label: 'Accent (10%)',
    hint: 'Primary buttons, active indicators & scrollbars',
    syncPrimaryAccent: true,
    presets: ['#ea5829', '#ff5a26', '#f59e0b', '#10b981', '#06b6d4', '#818cf8', '#e11d48'],
  },
  {
    key: 'background',
    label: 'Canvas Background (60%)',
    hint: 'Main matte workspace background',
    presets: ['#0a0a0b', '#000000', '#0e0c0a', '#0a0c14', '#fafafa', '#f7f5f0'],
  },
  {
    key: 'surface',
    label: 'Secondary Surface (30%)',
    hint: 'Sidebars, docks & popovers',
    presets: ['#0f0f11', '#08080a', '#15120f', '#0f121d', '#f1f1f4', '#ede9e0'],
  },
  {
    key: 'card',
    label: 'Card Surface',
    hint: 'Elevated cards, pills & bubbles',
    presets: ['#141417', '#0e0e11', '#1c1814', '#141826', '#ffffff', '#f5f5f7'],
  },
  {
    key: 'border',
    label: 'Hairline Border',
    hint: 'Subtle structural dividers',
    presets: ['#222227', '#1e1e24', '#2e2721', '#242a40', '#e2e2e7', '#dfdad0'],
  },
  {
    key: 'foreground',
    label: 'Primary Text',
    hint: 'Headings & editorial prose',
    presets: ['#f5f5f6', '#ffffff', '#f8f4ef', '#f1f5f9', '#0b0b0d', '#1c1917'],
  },
  {
    key: 'muted',
    label: 'Muted Text',
    hint: 'Metadata, hints & timestamps',
    presets: ['#8a8a93', '#8e8e97', '#9d9287', '#8c96b0', '#6b6b76', '#64748b'],
  },
  {
    key: 'destructive',
    label: 'Destructive',
    hint: 'Errors, deny actions & delete warnings',
    presets: ['#ef4444', '#dc2626', '#e11d48', '#f43f5e'],
  },
]

type NavNode =
  | { kind: 'global_theme' }
  | { kind: 'component'; categoryId: string; subId: string }

type GroupedSubEntry =
  | { kind: 'leaf'; sub: StudioSubComponent }
  | { kind: 'group'; name: string; items: StudioSubComponent[] }

/** Deterministically partitions a category's sub-components into direct leaves and collapsible hierarchy groups. */
function buildCategoryHierarchy(subs: readonly StudioSubComponent[]): GroupedSubEntry[] {
  const out: GroupedSubEntry[] = []
  const groupBuckets = new Map<string, StudioSubComponent[]>()

  for (const sub of subs) {
    const grp = (sub.group || '').trim()
    if (!grp) {
      out.push({ kind: 'leaf', sub })
      continue
    }
    let bucket = groupBuckets.get(grp)
    if (!bucket) {
      bucket = []
      groupBuckets.set(grp, bucket)
      out.push({ kind: 'group', name: grp, items: bucket })
    }
    bucket.push(sub)
  }
  return out
}

/**
 * Full-page Hierarchical Appearance & Component Studio.
 * - Left menubar is the single hierarchical navigation tree (no duplicate in-page tab bars!)
 * - Every inside component has its own dedicated focused page
 * - Live preview stage smoothly animates on page switches & option changes
 */
export function AppearanceStudio({
  ui,
  presets,
  studioSchema,
  providers,
  configuredProviders,
  models,
  activeProvider,
  activeModel,
  initialTarget,
  onConsumeInitialTarget,
  onPatchUi,
  onSelectPreset,
  onBackToChat,
}: {
  ui: UiCustomization
  presets: ThemePresetMeta[]
  studioSchema: StudioCategory[]
  providers: ProviderDescriptor[]
  configuredProviders: Record<string, boolean>
  models: ModelInfo[]
  activeProvider: string
  activeModel: string
  initialTarget?: { categoryId: string; subId: string } | null
  onConsumeInitialTarget?: () => void
  onPatchUi: (mutator: (draft: UiCustomization) => void) => void
  onSelectPreset: (presetId: string, mode?: 'dark' | 'light') => void
  onBackToChat: () => void
}) {
  const reduce = useReducedMotion() ?? false
  const [selected, setSelected] = useState<NavNode>(() =>
    initialTarget
      ? { kind: 'component', categoryId: initialTarget.categoryId, subId: initialTarget.subId }
      : { kind: 'global_theme' },
  )
  const [openAccordionId, setOpenAccordionId] = useState<string | null>(
    initialTarget?.categoryId ?? studioSchema[0]?.id ?? null,
  )
  const [expandedCategories, setExpandedCategories] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(
      studioSchema.map((cat, idx) => [
        cat.id,
        initialTarget ? cat.id === initialTarget.categoryId : idx === 0,
      ]),
    ),
  )

  useEffect(() => {
    if (!initialTarget) return
    setSelected({
      kind: 'component',
      categoryId: initialTarget.categoryId,
      subId: initialTarget.subId,
    })
    setOpenAccordionId(initialTarget.categoryId)
    setExpandedCategories((prev) => ({ ...prev, [initialTarget.categoryId]: true }))
    onConsumeInitialTarget?.()
  }, [initialTarget, onConsumeInitialTarget])
  const [expandedGroups, setExpandedGroups] = useState<Record<string, boolean>>({
    'chat_page::Thinking & Activity': true,
    'chat_page::Responses & Elements': true,
    'primitives::Overlays & Controls': true,
  })
  const [previewPrompt, setPreviewPrompt] = useState('Inspect workspace architecture and run tests')
  const [previewDockView, setPreviewDockView] = useState<ActiveView>('chat')
  const [previewTab, setPreviewTab] = useState('overview')
  const [previewAlertOpen, setPreviewAlertOpen] = useState(true)
  const [searchQuery, setSearchQuery] = useState('')
  const [highlightedPath, setHighlightedPath] = useState<string | null>(null)
  const searchInputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === '/' && !e.metaKey && !e.ctrlKey && !e.altKey) {
        const tag = (e.target as HTMLElement | null)?.tagName
        if (tag !== 'INPUT' && tag !== 'TEXTAREA' && !(e.target as HTMLElement | null)?.isContentEditable) {
          e.preventDefault()
          searchInputRef.current?.focus()
        }
      } else if (e.key === 'Escape' && document.activeElement === searchInputRef.current) {
        setSearchQuery('')
        searchInputRef.current?.blur()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])

  useEffect(() => {
    if (!highlightedPath) return
    const timer = window.setTimeout(() => {
      const el = document.querySelector<HTMLElement>(
        `[data-studio-control="${CSS.escape(highlightedPath)}"]`,
      )
      el?.scrollIntoView({ behavior: 'smooth', block: 'center' })
    }, 80)
    const clear = window.setTimeout(() => setHighlightedPath(null), 2200)
    return () => {
      window.clearTimeout(timer)
      window.clearTimeout(clear)
    }
  }, [highlightedPath, selected])

  const searchResults = useMemo(() => {
    const q = searchQuery.trim().toLowerCase()
    if (!q) return []
    const tokens = q.split(/\s+/).filter(Boolean)
    const matchesAll = (haystack: string) => tokens.every((t) => haystack.includes(t))

    interface StudioSearchHit {
      key: string
      node: NavNode
      title: string
      breadcrumb: string
      matchedHint: string
      controlPath?: string
      score: number
    }

    const hits: StudioSearchHit[] = []

    const globalKeywords = [
      'color themes',
      'preset',
      'dark mode',
      'light mode',
      'custom palette studio',
      'accent color',
      'custom font studio',
      'obel mono',
      'fontzz',
      'upload font',
      ...presets.map((p) => `${p.label} ${p.id}`),
    ]
      .join(' ')
      .toLowerCase()

    if (matchesAll(globalKeywords)) {
      const matchedPreset = presets.find((p) => p.label.toLowerCase().includes(q))
      hits.push({
        key: 'global_theme',
        node: { kind: 'global_theme' },
        title: 'Color Themes, Palette & Custom Fonts',
        breadcrumb: 'Global Theme',
        matchedHint: matchedPreset
          ? `Preset: ${matchedPreset.label}`
          : q.includes('font') || q.includes('obel')
            ? 'Custom Font Studio'
            : 'Presets & 60/30/10 Palette',
        score: 100,
      })
    }

    for (const cat of studioSchema) {
      for (const sub of cat.sub_components) {
        const subBread = `${cat.label}${sub.group ? ` / ${sub.group}` : ''}`
        const subHay = `${cat.label} ${sub.group || ''} ${sub.label} ${sub.description}`.toLowerCase()
        let bestControl: { label: string; path: string; score: number } | null = null

        for (const ctrl of sub.controls) {
          const ctrlHay = `${ctrl.label} ${ctrl.description} ${ctrl.path}`.toLowerCase()
          const matchedOpt = ctrl.options.find((opt) =>
            matchesAll(`${ctrlHay} ${opt.label} ${opt.hint} ${opt.value}`.toLowerCase()),
          )
          if (matchesAll(ctrlHay)) {
            const sc = ctrl.label.toLowerCase().includes(q) ? 90 : 75
            if (!bestControl || sc > bestControl.score) {
              bestControl = { label: ctrl.label, path: ctrl.path, score: sc }
            }
          } else if (matchedOpt) {
            const sc = matchedOpt.label.toLowerCase().includes(q) ? 85 : 65
            if (!bestControl || sc > bestControl.score) {
              bestControl = {
                label: `${ctrl.label} → ${matchedOpt.label}`,
                path: ctrl.path,
                score: sc,
              }
            }
          }
        }

        if (bestControl || matchesAll(subHay)) {
          const baseScore = sub.label.toLowerCase().includes(q)
            ? 95
            : bestControl
              ? bestControl.score
              : 60
          hits.push({
            key: `${cat.id}::${sub.id}`,
            node: { kind: 'component', categoryId: cat.id, subId: sub.id },
            title: sub.label,
            breadcrumb: subBread,
            matchedHint: bestControl ? bestControl.label : sub.description,
            controlPath: bestControl?.path,
            score: baseScore,
          })
        }
      }
    }

    return hits.sort((a, b) => b.score - a.score).slice(0, 18)
  }, [searchQuery, studioSchema, presets])

  const activeMode: 'dark' | 'light' = ui.mode === 'light' ? 'light' : 'dark'
  const themeTransition = ui.motion.theme_transition || 'circle-blur'

  const flatPages = useMemo<NavNode[]>(() => {
    const pages: NavNode[] = [{ kind: 'global_theme' }]
    for (const cat of studioSchema) {
      for (const sub of cat.sub_components) {
        pages.push({ kind: 'component', categoryId: cat.id, subId: sub.id })
      }
    }
    return pages
  }, [studioSchema])

  const currentIdx = flatPages.findIndex((p) => {
    if (p.kind !== selected.kind) return false
    if (p.kind === 'component' && selected.kind === 'component') {
      return p.categoryId === selected.categoryId && p.subId === selected.subId
    }
    return true
  })

  const selectSubComponent = (catId: string, sub: StudioSubComponent) => {
    setSelected({ kind: 'component', categoryId: catId, subId: sub.id })
    setOpenAccordionId(catId)
    setExpandedCategories((prev) => ({ ...prev, [catId]: true }))
    if (sub.group) {
      setExpandedGroups((prev) => ({ ...prev, [`${catId}::${sub.group}`]: true }))
    }
  }

  const goStep = (delta: number) => {
    if (flatPages.length === 0) return
    const next = flatPages[(currentIdx + delta + flatPages.length) % flatPages.length]
    setSelected(next)
    if (next.kind === 'component') {
      const cat = studioSchema.find((c) => c.id === next.categoryId)
      const sub = cat?.sub_components.find((s) => s.id === next.subId)
      if (cat && sub) selectSubComponent(cat.id, sub)
    }
  }

  const activeCategory =
    selected.kind === 'component'
      ? studioSchema.find((c) => c.id === selected.categoryId) || studioSchema[0]
      : null
  const activeSub =
    selected.kind === 'component' && activeCategory
      ? activeCategory.sub_components.find((s) => s.id === selected.subId) ||
        activeCategory.sub_components[0]
      : null

  // Transition only when switching pages so live previews & ECharts update smoothly in-place without unmounting on every slider/color drag tick.
  const previewAnimKey = useMemo(() => {
    if (!activeCategory || !activeSub) return 'global'
    return `${activeCategory.id}::${activeSub.id}`
  }, [activeCategory, activeSub])

  const toggleMode = () => {
    onSelectPreset(ui.preset_id || 'zenko_vermilion', activeMode === 'light' ? 'dark' : 'light')
  }

  const applyColorToken = (slot: CustomColorSlot, hex: string) => {
    onPatchUi((draft) => {
      draft.colors[slot.key] = hex
      if (slot.syncPrimaryAccent) {
        draft.colors.primary = hex
        draft.colors.accent = hex
      }
    })
  }

  const renderControl = (ctrl: StudioControl) => {
    const currentVal = getByPath(ui, ctrl.path)
    const isHighlighted = highlightedPath === ctrl.path

    if (ctrl.kind === 'cards') {
      const isLoaderOption = ctrl.path === 'components.chat_page.loader_variant'
      const isIconPackOption = ctrl.path === 'icons.pack'
      const isUiFontOption = ctrl.path === 'typography.ui_font'
      const isMonoFontOption = ctrl.path === 'typography.mono_font'
      const isAgentAvatarOption = ctrl.path === 'motion.orb_style'
      const isAgentFrameOption = ctrl.path === 'components.chat_page.agent_avatar_frame'
      const isUserAvatarOption = ctrl.path === 'components.chat_page.user_avatar'
      const isUserFrameOption = ctrl.path === 'components.chat_page.user_avatar_frame'

      return (
        <div
          key={ctrl.path}
          data-studio-control={ctrl.path}
          className={cn(
            'flex flex-col gap-2.5 rounded-2xl transition-shadow',
            isHighlighted && 'ring-1 ring-[var(--accent)] ring-offset-4 ring-offset-[var(--bg)]',
          )}
        >
          <div>
            <div className="text-[13px] font-semibold text-[var(--fg)]">{ctrl.label}</div>
            <div className="text-[11.5px] text-[var(--muted)]">{ctrl.description}</div>
          </div>
          <div className="grid grid-cols-2 gap-2.5 lg:grid-cols-4">
            {ctrl.options.map((opt) => {
              const isSelected = String(currentVal) === opt.value
              const packMeta = isIconPackOption ? ICON_PACK_META_MAP[opt.value] : undefined
              const cardFontFamily = isUiFontOption
                ? resolveUiFontStack(opt.value)
                : isMonoFontOption
                  ? resolveMonoFontStack(opt.value)
                  : undefined

              return (
                <button
                  key={opt.value}
                  type="button"
                  onClick={() =>
                    onPatchUi((draft) => {
                      setByPath(draft, ctrl.path, opt.value)
                      if (isUiFontOption) draft.typography.custom_ui_font = ''
                      if (isMonoFontOption) draft.typography.custom_mono_font = ''
                    })
                  }
                  className={cn(
                    'flex cursor-pointer flex-col gap-1.5 rounded-xl p-3 text-left transition-all',
                    isSelected
                      ? 'border border-[var(--accent)] bg-[var(--accent)]/10 text-[var(--fg)]'
                      : 'z-card-item text-[var(--fg)]/85 hover:text-[var(--fg)]',
                  )}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span
                      className="flex min-w-0 items-center gap-2 text-[12.5px] font-semibold"
                      style={cardFontFamily ? { fontFamily: cardFontFamily } : undefined}
                    >
                      {isLoaderOption && (
                        <Loader
                          variant={opt.value as LoaderVariant}
                          size={15}
                          className="shrink-0 text-[var(--accent)]"
                          label={opt.label}
                        />
                      )}
                      {isIconPackOption && (
                        <ZenkoIcon
                          slot="sparkle"
                          icons={{ ...ui.icons, pack: opt.value as UiCustomization['icons']['pack'] }}
                          size={16}
                          className="shrink-0 text-[var(--accent)]"
                        />
                      )}
                      {isAgentAvatarOption && opt.value !== 'hidden' && (
                        <AgentAvatar
                          variant={opt.value}
                          frame={ui.components.chat_page.agent_avatar_frame || 'squircle'}
                          color={ui.components.chat_page.agent_avatar_color}
                          icons={ui.icons}
                          size={22}
                        />
                      )}
                      {isAgentFrameOption && (
                        <AgentAvatar
                          variant={ui.motion.orb_style === 'hidden' ? 'liquid_halo' : ui.motion.orb_style}
                          frame={opt.value}
                          color={ui.components.chat_page.agent_avatar_color}
                          icons={ui.icons}
                          size={22}
                        />
                      )}
                      {isUserAvatarOption && opt.value !== 'hidden' && (
                        <UserAvatar
                          variant={opt.value}
                          frame={ui.components.chat_page.user_avatar_frame || 'circle'}
                          color={ui.components.chat_page.user_avatar_color}
                          icons={ui.icons}
                          size={22}
                        />
                      )}
                      {isUserFrameOption && (
                        <UserAvatar
                          variant={
                            ui.components.chat_page.user_avatar === 'hidden'
                              ? 'icon_user'
                              : ui.components.chat_page.user_avatar || 'icon_user'
                          }
                          frame={opt.value}
                          color={ui.components.chat_page.user_avatar_color}
                          icons={ui.icons}
                          size={22}
                        />
                      )}
                      <span className="truncate">{opt.label}</span>
                    </span>
                    <div className="flex shrink-0 items-center gap-1.5">
                      {packMeta?.animated && !isSelected && (
                        <span className="rounded-full bg-[color-mix(in_srgb,var(--accent)_12%,transparent)] px-1.5 py-px font-mono text-[9.5px] text-[var(--accent)]">
                          anim
                        </span>
                      )}
                      {isSelected && (
                        <span
                          className="flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-[var(--primary-fg)]"
                          style={{ backgroundColor: 'var(--accent)' }}
                        >
                          <Check size={10} strokeWidth={2.8} />
                        </span>
                      )}
                    </div>
                  </div>
                  <span className="text-[11px] leading-snug text-[var(--muted)]">{opt.hint}</span>
                </button>
              )
            })}
          </div>
        </div>
      )
    }

    if (ctrl.kind === 'toggle') {
      const checked = Boolean(currentVal)
      return (
        <div
          key={ctrl.path}
          data-studio-control={ctrl.path}
          className={cn(
            'z-card-item flex items-center justify-between gap-4 px-4 py-3',
            isHighlighted && 'ring-1 ring-[var(--accent)] ring-offset-2 ring-offset-[var(--bg)]',
          )}
        >
          <div>
            <div className="text-[13px] font-medium text-[var(--fg)]">{ctrl.label}</div>
            <div className="text-[11.5px] text-[var(--muted)]">{ctrl.description}</div>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={checked}
            onClick={() => onPatchUi((draft) => setByPath(draft, ctrl.path, !checked))}
            className="relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full border transition-colors"
            style={{
              backgroundColor: checked ? 'var(--accent)' : 'var(--bg)',
              borderColor: checked ? 'var(--accent)' : 'var(--border)',
            }}
          >
            <span
              className={cn(
                'inline-block h-4 w-4 rounded-full bg-white shadow-xs transition-transform',
                checked ? 'translate-x-5.5' : 'translate-x-1',
              )}
            />
          </button>
        </div>
      )
    }

    if (ctrl.kind === 'range') {
      const numVal = Number(currentVal ?? ctrl.min ?? 0)
      const step = ctrl.step ?? 1
      return (
        <div
          key={ctrl.path}
          data-studio-control={ctrl.path}
          className={cn(
            'z-card-item flex flex-col gap-2 p-3.5',
            isHighlighted && 'ring-1 ring-[var(--accent)] ring-offset-2 ring-offset-[var(--bg)]',
          )}
        >
          <div className="text-[11.5px] text-[var(--muted)]">{ctrl.description}</div>
          <InlineSlider
            label={ctrl.label}
            min={ctrl.min ?? 0}
            max={ctrl.max ?? 24}
            step={step}
            value={numVal}
            format={(v) => `${Number.isInteger(step) ? Math.round(v) : v.toFixed(1)}${ctrl.unit}`}
            onValueChange={(v) =>
              onPatchUi((draft) =>
                setByPath(draft, ctrl.path, Number.isInteger(step) ? Math.round(v) : Number(v.toFixed(1))),
              )
            }
            className="border border-[var(--border)] bg-[var(--surface)]"
          />
        </div>
      )
    }

    if (ctrl.kind === 'color') {
      const savedHex = String(currentVal || '').trim()
      const displayHex = savedHex || ui.colors.accent
      return (
        <div
          key={ctrl.path}
          data-studio-control={ctrl.path}
          className={cn(
            'z-card-item relative flex flex-col gap-3 p-4 focus-within:z-30',
            isHighlighted && 'ring-1 ring-[var(--accent)] ring-offset-2 ring-offset-[var(--bg)]',
          )}
        >
          <div className="flex flex-wrap items-start justify-between gap-2">
            <div>
              <div className="text-[13px] font-semibold text-[var(--fg)]">{ctrl.label}</div>
              <div className="text-[11.5px] text-[var(--muted)]">{ctrl.description}</div>
            </div>
            <div className="flex items-center gap-2">
              <span className="font-mono text-[10.5px] text-[var(--muted)]">
                {savedHex ? savedHex.toUpperCase() : 'Theme Default'}
              </span>
              {savedHex && (
                <button
                  type="button"
                  onClick={() => onPatchUi((draft) => setByPath(draft, ctrl.path, ''))}
                  className="z-btn z-btn-secondary z-btn-sm font-mono text-[10.5px]"
                >
                  <RotateCcw size={11} /> Theme Default
                </button>
              )}
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-3">
            <div className="w-[180px] shrink-0">
              <ColorPicker
                value={displayHex}
                alpha={false}
                onValueChange={(hex) => onPatchUi((draft) => setByPath(draft, ctrl.path, hex))}
                aria-label={ctrl.label}
              >
                <ColorPickerTrigger className="h-9 w-full justify-start border-[var(--border)] bg-[var(--bg)] text-[12px]" />
                <ColorPickerContent side="right-up">
                  <ColorPickerArea />
                  <ColorPickerHue />
                  <div className="flex items-center gap-2">
                    <ColorPickerHexInput />
                    <ColorPickerEyeDropper />
                  </div>
                  <ColorPickerChannels format="rgb" />
                  <ColorPickerPresets>
                    {CURATED_ACCENT_SWATCHES.map((s) => (
                      <ColorPickerPreset key={s.id} value={s.color} />
                    ))}
                  </ColorPickerPresets>
                </ColorPickerContent>
              </ColorPicker>
            </div>

            <div className="flex flex-wrap items-center gap-1.5">
              {CURATED_ACCENT_SWATCHES.map((swatch) => {
                const active = savedHex.toLowerCase() === swatch.color.toLowerCase()
                return (
                  <ZenkoTooltip key={swatch.id} label={swatch.label} side="top">
                    <button
                      type="button"
                      aria-label={swatch.label}
                      onClick={() =>
                        onPatchUi((draft) => setByPath(draft, ctrl.path, swatch.color))
                      }
                      className={cn(
                        'flex h-7 w-7 cursor-pointer items-center justify-center rounded-full border transition-transform hover:scale-110',
                        active
                          ? 'scale-110 border-[var(--fg)] ring-2 ring-[var(--accent)]/40'
                          : 'border-[var(--border)]',
                      )}
                      style={{ backgroundColor: swatch.color }}
                    >
                      {active && <Check size={12} className="text-black/80" strokeWidth={3} />}
                    </button>
                  </ZenkoTooltip>
                )
              })}
            </div>
          </div>
        </div>
      )
    }

    if (ctrl.kind === 'icon_overrides') {
      const iconOptions = CUSTOMIZABLE_ICON_OPTIONS.map((opt) => ({
        value: opt,
        label: opt,
        icon: <ZenkoIcon slot={opt} icons={{ ...ui.icons, overrides: {} }} size={14} />,
      }))
      return (
        <div key={ctrl.path} className="flex flex-col gap-2.5">
          <div>
            <div className="text-[13px] font-semibold text-[var(--fg)]">{ctrl.label}</div>
            <div className="text-[11.5px] text-[var(--muted)]">{ctrl.description}</div>
          </div>
          <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
            {(['grid', 'chat', 'terminal', 'palette'] as const).map((slot) => (
              <div
                key={slot}
                className="z-card-item flex flex-col gap-1.5 p-3 text-[11.5px] text-[var(--muted)]"
              >
                <span className="flex items-center gap-1.5 font-medium capitalize text-[var(--fg)]">
                  <ZenkoIcon slot={slot} icons={ui.icons} size={14} /> {slot} slot
                </span>
                <ZenkoSelect
                  mono
                  ariaLabel={`${slot} icon slot override`}
                  value={ui.icons.overrides?.[slot] || slot}
                  options={iconOptions}
                  onChange={(nextVal) =>
                    onPatchUi((c) => {
                      c.icons.overrides = { ...(c.icons.overrides || {}), [slot]: nextVal }
                    })
                  }
                />
              </div>
            ))}
          </div>
        </div>
      )
    }

    return null
  }

  /** Renders a category's multi-level hierarchical children inside the Left Menubar. */
  const renderCategoryTree = (cat: StudioCategory) => {
    const entries = buildCategoryHierarchy(cat.sub_components)
    return (
      <div className="flex flex-col gap-0.5 border-l border-[var(--border)] pl-2.5">
        {entries.map((entry) => {
          if (entry.kind === 'leaf') {
            const sub = entry.sub
            const isSubActive =
              selected.kind === 'component' &&
              selected.categoryId === cat.id &&
              selected.subId === sub.id
            return (
              <button
                key={sub.id}
                type="button"
                onClick={() => selectSubComponent(cat.id, sub)}
                className={cn(
                  'flex w-full cursor-pointer items-center justify-between rounded-md px-2 py-1 text-left text-[12px] transition-colors',
                  isSubActive
                    ? 'bg-[var(--bg)] font-medium text-[var(--accent)]'
                    : 'text-[var(--muted)] hover:text-[var(--fg)]',
                )}
              >
                <span className="truncate">{sub.label}</span>
              </button>
            )
          }

          const groupKey = `${cat.id}::${entry.name}`
          const isGroupOpen = expandedGroups[groupKey] ?? true
          const groupHasActive =
            selected.kind === 'component' &&
            selected.categoryId === cat.id &&
            entry.items.some((s) => s.id === selected.subId)

          return (
            <div key={groupKey} className="mt-1 flex flex-col">
              <button
                type="button"
                onClick={() =>
                  setExpandedGroups((prev) => ({ ...prev, [groupKey]: !isGroupOpen }))
                }
                className={cn(
                  'flex w-full cursor-pointer items-center justify-between rounded-md px-2 py-1 text-left text-[11.5px] font-medium transition-colors',
                  groupHasActive
                    ? 'text-[var(--fg)]'
                    : 'text-[var(--muted)]/85 hover:text-[var(--fg)]',
                )}
              >
                <span className="truncate">{entry.name}</span>
                <ChevronRight
                  size={11.5}
                  className={cn(
                    'shrink-0 text-[var(--muted)] transition-transform duration-150',
                    isGroupOpen && 'rotate-90',
                  )}
                />
              </button>
              {isGroupOpen && (
                <div className="ml-2 mt-0.5 flex flex-col gap-0.5 border-l border-[var(--border)]/70 pl-2">
                  {entry.items.map((sub) => {
                    const isSubActive =
                      selected.kind === 'component' &&
                      selected.categoryId === cat.id &&
                      selected.subId === sub.id
                    return (
                      <button
                        key={sub.id}
                        type="button"
                        onClick={() => selectSubComponent(cat.id, sub)}
                        className={cn(
                          'flex w-full cursor-pointer items-center justify-between rounded-md px-2 py-1 text-left text-[11.5px] transition-colors',
                          isSubActive
                            ? 'bg-[var(--bg)] font-medium text-[var(--accent)]'
                            : 'text-[var(--muted)] hover:text-[var(--fg)]',
                        )}
                      >
                        <span className="truncate">{sub.label}</span>
                      </button>
                    )
                  })}
                </div>
              )}
            </div>
          )
        })}
      </div>
    )
  }

  const stageBody = (
    <div className="mx-auto flex w-full max-w-[1040px] flex-col gap-7">
      {/* Top Header + Breadcrumb Hierarchy + Prev/Next Controls */}
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          {selected.kind === 'component' && activeCategory && activeSub && (
            <div className="mb-1 font-mono text-[11px] text-[var(--muted)]">
              Components / {activeCategory.label}
              {activeSub.group ? ` / ${activeSub.group}` : ''}
            </div>
          )}
          <h1
            className="text-[28px] font-semibold tracking-[-0.03em] text-[var(--fg)]"
            style={{ fontFamily: 'var(--font-display)' }}
          >
            {selected.kind === 'global_theme'
              ? 'Color Themes'
              : activeSub?.label || activeCategory?.label || 'Component'}
          </h1>
          <p className="mt-1 max-w-2xl text-[13.5px] leading-relaxed text-[var(--muted)]">
            {selected.kind === 'global_theme'
              ? 'Choose a curated preset, craft custom 60/30/10 colors, or pick Obel Mono & Fontzz typefaces.'
              : activeSub?.description || activeCategory?.description}
          </p>
        </div>

        <div className="flex items-center gap-1.5">
          <ZenkoTooltip label="Reset to Zenko Vermilion" side="bottom">
            <button
              type="button"
              onClick={() => onSelectPreset('zenko_vermilion', 'dark')}
              className="z-btn z-btn-secondary z-btn-sm"
            >
              <RotateCcw size={12.5} /> Reset
            </button>
          </ZenkoTooltip>
          <ZenkoTooltip label="Previous page" side="bottom">
            <button
              type="button"
              onClick={() => goStep(-1)}
              aria-label="Previous component"
              className="z-btn z-btn-secondary z-btn-icon h-8 w-8 rounded-lg"
            >
              <ArrowLeft size={13.5} />
            </button>
          </ZenkoTooltip>
          <ZenkoTooltip label="Next page" side="bottom">
            <button
              type="button"
              onClick={() => goStep(1)}
              aria-label="Next component"
              className="z-btn z-btn-secondary z-btn-icon h-8 w-8 rounded-lg"
            >
              <ArrowRight size={13.5} />
            </button>
          </ZenkoTooltip>
        </div>
      </div>

      {/* GLOBAL THEME PAGE — Presets + Custom Palette + Custom Font Studio (with Obel Mono & Fontzz) */}
      {selected.kind === 'global_theme' && (
        <div className="flex flex-col gap-7">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border)] pb-3">
            <div className="flex items-center gap-2.5">
              <ThemeToggle
                mode={activeMode}
                variant={themeTransition}
                start="top-left"
                onToggle={toggleMode}
                className="h-8 w-8 rounded-lg border border-[var(--border)] bg-[var(--surface)] text-[var(--fg)] hover:border-[var(--accent)]/50"
                iconClassName="size-4"
              />
              <span className="text-[12.5px] font-medium text-[var(--fg)]">
                {activeMode === 'dark' ? 'Dark Mode' : 'Light Mode'}
              </span>
              <span className="rounded-md border border-[var(--border)] bg-[var(--surface)] px-2 py-0.5 font-mono text-[10.5px] text-[var(--muted)]">
                {themeTransition}
              </span>
            </div>

            <span className="font-mono text-[11px] text-[var(--muted)]">
              {presets.find((p) => p.id === ui.preset_id)?.label || ui.preset_id} · {activeMode}
            </span>
          </div>

          <ThemeSelector
            themes={presets}
            value={ui.preset_id}
            mode={activeMode}
            onChange={onSelectPreset}
          />

          {/* Custom Colors Section */}
          <div className="z-surface-card flex flex-col gap-5 p-5">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div>
                <div className="flex items-center gap-2 text-[15px] font-semibold text-[var(--fg)]">
                  <Sliders size={15} style={{ color: 'var(--accent)' }} />
                  <span>Custom Palette Studio</span>
                </div>
                <p className="mt-0.5 text-[12px] text-[var(--muted)]">
                  Quick-swap your 10% accent color or fine-tune any surface token with the interactive picker.
                </p>
              </div>
              <button
                type="button"
                onClick={() => onSelectPreset(ui.preset_id || 'zenko_vermilion', activeMode)}
                className="z-btn z-btn-secondary z-btn-sm"
              >
                <RotateCcw size={12} /> Restore Preset Colors
              </button>
            </div>

            <ColorSelector
              value={ui.colors.accent.toLowerCase()}
              onValueChange={(hex) =>
                onPatchUi((draft) => {
                  draft.colors.accent = hex
                  draft.colors.primary = hex
                })
              }
            >
              <ColorSelectorLabel className="mb-2 text-[12px] text-[var(--muted)]">
                Quick Accent Swatches
              </ColorSelectorLabel>
              <ColorSelectorList className="gap-2.5">
                {CURATED_ACCENT_SWATCHES.map((swatch) => (
                  <ColorSelectorItem
                    key={swatch.id}
                    value={swatch.id}
                    color={swatch.color}
                    label={swatch.label}
                    className="size-9 border-[var(--border)] bg-[var(--card)]"
                  />
                ))}
              </ColorSelectorList>
            </ColorSelector>

            <div className="grid grid-cols-1 gap-3 border-t border-[var(--border)]/70 pt-4 sm:grid-cols-2 lg:grid-cols-4">
              {CUSTOM_COLOR_SLOTS.map((slot) => (
                <div
                  key={slot.key}
                  className="z-card-item relative flex flex-col gap-2 p-3 focus-within:z-30"
                >
                  <div>
                    <div className="text-[12px] font-medium text-[var(--fg)]">{slot.label}</div>
                    <div className="text-[10.5px] text-[var(--muted)]">{slot.hint}</div>
                  </div>
                  <ColorPicker
                    value={ui.colors[slot.key]}
                    alpha={false}
                    onValueChange={(hex) => applyColorToken(slot, hex)}
                    aria-label={slot.label}
                  >
                    <ColorPickerTrigger className="h-9 w-full justify-start border-[var(--border)] bg-[var(--bg)] text-[12px]" />
                    <ColorPickerContent side="right-up">
                      <ColorPickerArea />
                      <ColorPickerHue />
                      <div className="flex items-center gap-2">
                        <ColorPickerHexInput />
                        <ColorPickerEyeDropper />
                      </div>
                      <ColorPickerChannels format="rgb" />
                      <ColorPickerPresets>
                        {slot.presets.map((p) => (
                          <ColorPickerPreset key={p} value={p} />
                        ))}
                      </ColorPickerPresets>
                    </ColorPickerContent>
                  </ColorPicker>
                </div>
              ))}
            </div>
          </div>

          {/* Global Theme Custom Font Studio (Obel Mono + Fontzz + OS + Custom Upload) */}
          <CustomFontStudio
            typography={ui.typography}
            onPatchTypography={(mutator) =>
              onPatchUi((draft) => {
                mutator(draft.typography)
              })
            }
          />
        </div>
      )}

      {/* DEDICATED FOCUSED COMPONENT PAGE — ZERO redundant top tabs + Animated Live Preview! */}
      {selected.kind === 'component' && activeCategory && activeSub && (
        <div className="flex flex-col gap-6">
          <div className="relative overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--bg)]">
            <div className="flex items-center justify-between border-b border-[var(--border)]/70 px-4 py-2 font-mono text-[11px] text-[var(--muted)]">
              <span>
                Live Preview · {activeCategory.label}
                {activeSub.group ? ` / ${activeSub.group}` : ''} / {activeSub.label}
              </span>
              <ZenkoTooltip
                label={ui.mode === 'light' ? 'Preview in Dark' : 'Preview in Light'}
                side="left"
              >
                <ThemeToggle
                  mode={activeMode}
                  variant={themeTransition}
                  start="top-right"
                  onToggle={toggleMode}
                  className="h-6 w-6 rounded-md border border-[var(--border)] bg-[var(--surface)] text-[var(--muted)] hover:text-[var(--fg)]"
                  iconClassName="size-3"
                />
              </ZenkoTooltip>
            </div>

            <div className="flex min-h-[250px] items-center justify-center p-6 sm:p-8">
              <AnimatePresence mode="wait" initial={false}>
                <motion.div
                  key={previewAnimKey}
                  initial={reduce ? { opacity: 1 } : { opacity: 0, y: 8, scale: 0.985, filter: 'blur(4px)' }}
                  animate={{ opacity: 1, y: 0, scale: 1, filter: 'blur(0px)' }}
                  exit={reduce ? { opacity: 0 } : { opacity: 0, y: -6, scale: 0.985, filter: 'blur(3px)' }}
                  transition={{ duration: reduce ? 0 : 0.22, ease: EASE_OUT }}
                  className="flex w-full items-center justify-center"
                >
                  {renderStudioPreview(activeSub.preview, {
                    ui,
                    providers,
                    configuredProviders,
                    models,
                    activeProvider,
                    activeModel,
                    activeMode,
                    toggleMode,
                    previewPrompt,
                    setPreviewPrompt,
                    previewDockView,
                    setPreviewDockView,
                    previewTab,
                    setPreviewTab,
                    previewAlertOpen,
                    setPreviewAlertOpen,
                  })}
                </motion.div>
              </AnimatePresence>
            </div>
          </div>

          <div className="flex flex-col gap-5">
            {activeSub.controls.map((ctrl) => renderControl(ctrl))}
          </div>
        </div>
      )}
    </div>
  )

  return (
    <div className="flex h-full w-full flex-1 overflow-hidden bg-[var(--bg)]">
      {/* Left Hierarchical Component Tree Sidebar (single source of truth for navigation!) */}
      <aside className="flex w-[274px] shrink-0 flex-col border-r border-[var(--border)] bg-[var(--bg)]">
        <div className="flex items-center justify-between border-b border-[var(--border)] px-4 py-3.5">
          <button
            type="button"
            onClick={onBackToChat}
            className="inline-flex cursor-pointer items-center gap-1.5 text-[12.5px] font-medium text-[var(--muted)] transition-colors hover:text-[var(--fg)]"
          >
            <ArrowLeft size={14} />
            <span>Back to Chat</span>
          </button>
          <ZenkoTooltip
            label={ui.mode === 'light' ? 'Switch to Dark mode' : 'Switch to Light mode'}
            side="bottom"
          >
            <ThemeToggle
              mode={activeMode}
              variant={themeTransition}
              start="top-left"
              onToggle={toggleMode}
              aria-label="Switch global Dark / Light theme"
              className="h-7 w-7 rounded-lg border border-[var(--border)] bg-[var(--surface)] text-[var(--muted)] hover:text-[var(--fg)]"
              iconClassName="size-3.5"
            />
          </ZenkoTooltip>
        </div>

        <div className="border-b border-[var(--border)]/70 px-3 py-2.5">
          <div className="relative flex items-center">
            <Search
              size={13}
              className="pointer-events-none absolute left-2.5 text-[var(--muted)]"
            />
            <input
              ref={searchInputRef}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && searchResults.length > 0) {
                  const first = searchResults[0]
                  const targetNode = first.node
                  if (targetNode.kind === 'global_theme') {
                    setSelected({ kind: 'global_theme' })
                  } else {
                    const cat = studioSchema.find((c) => c.id === targetNode.categoryId)
                    const sub = cat?.sub_components.find((s) => s.id === targetNode.subId)
                    if (cat && sub) selectSubComponent(cat.id, sub)
                  }
                  if (first.controlPath) setHighlightedPath(first.controlPath)
                }
              }}
              placeholder="Search settings... (/)"
              aria-label="Search Theme Studio settings"
              className="w-full rounded-lg border border-[var(--border)] bg-[var(--surface)]/80 py-1.5 pl-8 pr-7 text-[12px] text-[var(--fg)] placeholder:text-[var(--muted)]/70 transition-colors focus:border-[var(--accent)] focus:outline-none"
            />
            {searchQuery && (
              <button
                type="button"
                aria-label="Clear search"
                onClick={() => setSearchQuery('')}
                className="absolute right-2 flex h-4 w-4 cursor-pointer items-center justify-center rounded text-[var(--muted)] hover:text-[var(--fg)]"
              >
                <X size={11.5} />
              </button>
            )}
          </div>
        </div>

        <CompactScrollArea className="flex-1" viewportClassName="px-3 py-4 pr-3.5">
          <nav>
            {searchQuery.trim() ? (
              <div className="flex flex-col gap-1.5">
                <div className="mb-1 flex items-center justify-between px-2 font-mono text-[10.5px] text-[var(--muted)]">
                  <span>Results ({searchResults.length})</span>
                  <button
                    type="button"
                    onClick={() => setSearchQuery('')}
                    className="cursor-pointer text-[var(--accent)] hover:underline"
                  >
                    Reset
                  </button>
                </div>
                {searchResults.length === 0 ? (
                  <div className="rounded-lg border border-dashed border-[var(--border)] px-3 py-4 text-center text-[11.5px] text-[var(--muted)]">
                    No customization controls match &ldquo;{searchQuery.trim()}&rdquo;.
                  </div>
                ) : (
                  searchResults.map((hit) => (
                    <button
                      key={hit.key}
                      type="button"
                      onClick={() => {
                        const targetNode = hit.node
                        if (targetNode.kind === 'global_theme') {
                          setSelected({ kind: 'global_theme' })
                        } else {
                          const cat = studioSchema.find((c) => c.id === targetNode.categoryId)
                          const sub = cat?.sub_components.find((s) => s.id === targetNode.subId)
                          if (cat && sub) selectSubComponent(cat.id, sub)
                        }
                        if (hit.controlPath) setHighlightedPath(hit.controlPath)
                      }}
                      className="flex w-full cursor-pointer flex-col gap-0.5 rounded-lg border border-transparent px-2.5 py-2 text-left transition-colors hover:border-[var(--border)] hover:bg-[var(--surface)]"
                    >
                      <span className="text-[12.5px] font-medium text-[var(--fg)]">{hit.title}</span>
                      <span className="font-mono text-[10px] text-[var(--muted)]">{hit.breadcrumb}</span>
                      <span className="truncate text-[11px] text-[var(--accent)]/90">
                        {hit.matchedHint}
                      </span>
                    </button>
                  ))
                )}
              </div>
            ) : (
              <>
            {/* Section 1: Global Theme */}
            <div className="mb-5">
              <div className="mb-1.5 px-2.5 text-[11px] font-medium text-[var(--muted)]">
                Global Theme
              </div>
              <button
                type="button"
                onClick={() => setSelected({ kind: 'global_theme' })}
                className={cn(
                  'flex w-full cursor-pointer items-center justify-between rounded-lg px-2.5 py-2 text-left text-[13px] transition-colors',
                  selected.kind === 'global_theme'
                    ? 'bg-[var(--card)] font-medium text-[var(--fg)]'
                    : 'text-[var(--muted)] hover:bg-[var(--surface)] hover:text-[var(--fg)]',
                )}
              >
                <span className="inline-flex items-center gap-2">
                  <Palette size={13.5} style={{ color: 'var(--accent)' }} />
                  Color Themes
                </span>
                <span className="font-mono text-[10px] uppercase text-[var(--muted)]">
                  {ui.mode}
                </span>
              </button>
            </div>

            {/* Section 2: Hierarchical Data-Driven Components Tree */}
            <div>
              <div className="mb-1.5 px-2.5 text-[11px] font-medium text-[var(--muted)]">
                Components
              </div>
              {ui.surface.accordion_style !== 'compact' ? (
                <BouncyAccordion
                  collapsible
                  radius={0}
                  value={openAccordionId}
                  onValueChange={(nextId) => {
                    setOpenAccordionId(nextId)
                    if (nextId) {
                      const cat = studioSchema.find((c) => c.id === nextId)
                      const alreadyInCat =
                        selected.kind === 'component' && selected.categoryId === nextId
                      if (cat && !alreadyInCat && cat.sub_components[0]) {
                        selectSubComponent(cat.id, cat.sub_components[0])
                      }
                    }
                  }}
                  items={studioSchema.map((cat) => {
                    const isCatActive =
                      selected.kind === 'component' && selected.categoryId === cat.id
                    return {
                      id: cat.id,
                      title: (
                        <span
                          className={cn(
                            'text-[13px]',
                            isCatActive
                              ? 'font-semibold text-[var(--fg)]'
                              : 'font-medium text-[var(--fg)]/85',
                          )}
                        >
                          {cat.label}
                        </span>
                      ),
                      description: renderCategoryTree(cat),
                    }
                  })}
                  classNames={{
                    root: 'gap-0',
                    item: 'rounded-none border-0 border-b border-[var(--border)]/55 bg-transparent shadow-none last:border-b-0',
                    trigger: 'min-h-[38px] gap-2 px-1.5',
                    chevron: 'h-4 w-4',
                    content: 'px-0',
                    contentInner: 'px-1.5 pb-2.5 pt-0.5',
                    description: 'text-[12px]',
                  }}
                />
              ) : (
                <div className="flex flex-col">
                  {studioSchema.map((cat) => {
                    const isCatActive =
                      selected.kind === 'component' && selected.categoryId === cat.id
                    const isExpanded = Boolean(expandedCategories[cat.id])
                    return (
                      <div
                        key={cat.id}
                        className="flex flex-col border-b border-[var(--border)]/55 py-1 last:border-b-0"
                      >
                        <button
                          type="button"
                          onClick={() => {
                            const nextExpanded = !isExpanded
                            setExpandedCategories((prev) => ({ ...prev, [cat.id]: nextExpanded }))
                            if (nextExpanded && !isCatActive && cat.sub_components[0]) {
                              selectSubComponent(cat.id, cat.sub_components[0])
                            }
                          }}
                          className={cn(
                            'flex w-full cursor-pointer items-center justify-between rounded-lg px-2.5 py-1.5 text-left text-[13px] transition-colors',
                            isCatActive
                              ? 'bg-[var(--card)] font-medium text-[var(--fg)]'
                              : 'text-[var(--fg)]/80 hover:bg-[var(--surface)] hover:text-[var(--fg)]',
                          )}
                        >
                          <span className="truncate">{cat.label}</span>
                          <ChevronDown
                            size={12.5}
                            className={cn(
                              'shrink-0 text-[var(--muted)] transition-transform duration-150',
                              isExpanded && 'rotate-180',
                            )}
                          />
                        </button>

                        {isExpanded && <div className="ml-2 mt-1">{renderCategoryTree(cat)}</div>}
                      </div>
                    )
                  })}
                </div>
              )}
            </div>
              </>
            )}
          </nav>
        </CompactScrollArea>
      </aside>

      {/* Right Full Studio Stage */}
      <CompactScrollArea
        smooth={ui.motion.smooth_scroll !== false}
        className="min-h-0 flex-1"
        viewportClassName="px-8 py-8 lg:px-14"
      >
        {stageBody}
      </CompactScrollArea>
    </div>
  )
}
