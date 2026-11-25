import { memo, type CSSProperties } from 'react'
import { ChevronDown, RotateCcw, Zap } from 'lucide-react'
import type {
  ComponentSlots,
  ConciseMode,
  ConciseModeDescriptor,
  ZMode,
  ZModeDescriptor,
} from '@/lib/types'
import { cn } from '@/lib/utils'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/motion/popover'
import { ZenkoTooltip } from '@/components/ui/Tooltip'

export type ZModeButtonStyle = NonNullable<ComponentSlots['composer']['concise_button_style']>
export type ZModeButtonShape = NonNullable<ComponentSlots['composer']['concise_button_shape']>
export type ZModeButtonChrome = NonNullable<ComponentSlots['composer']['concise_button_chrome']>

export type ConciseButtonStyle = ZModeButtonStyle
export type ConciseButtonShape = ZModeButtonShape
export type ConciseButtonChrome = ZModeButtonChrome

export interface ConciseUpdatePatch {
  concise_mode?: ConciseMode
  read_less?: boolean
  auto_clarity?: boolean
}

export const FALLBACK_CONCISE_MODES: readonly ConciseModeDescriptor[] = [
  {
    mode: 'off',
    id: 'off',
    label: 'Off',
    description: 'Standard full English prose with no output compression',
    target_reduction_pct: 0,
    directive: '',
  },
  {
    mode: 'lite',
    id: 'lite',
    label: 'Lite',
    description: 'Filler-free professional English: drop pleasantries and hedging, keep clear sentences',
    target_reduction_pct: 30,
    directive: '',
  },
  {
    mode: 'full',
    id: 'full',
    label: 'Full',
    description: 'Telegraphic English: short active-voice fragments, zero fluff',
    target_reduction_pct: 60,
    directive: '',
  },
  {
    mode: 'ultra',
    id: 'ultra',
    label: 'Ultra',
    description: 'Maximum English token density: minimal fragments and causal arrows',
    target_reduction_pct: 75,
    directive: '',
  },
]

export const FALLBACK_ZMODES: readonly ZModeDescriptor[] = [
  {
    mode: 'lite',
    id: 'lite',
    label: 'Lite',
    tagline: 'Max clarity · light compression',
    description:
      'Full natural English prose, light reuse-first minicode, lossless dedup/scrub (4 KB Read-Less), longer raw history (78%/90%), and micro-skills with tool pruning off.',
    concise_mode: 'off',
    minimalism_mode: 'lite',
    auto_bloat_hint: false,
    read_less: true,
    read_less_min_bytes: 4096,
    microcompact_trigger_ratio: 0.78,
    compaction_trigger_ratio: 0.9,
    auto_micro_skills: true,
    auto_tool_prune: false,
    tool_prune_min_prompts: 16,
    tool_prune_min_sessions: 4,
    p90_loop_guard: true,
    p90_multiplier: 1.4,
  },
  {
    mode: 'balanced',
    id: 'balanced',
    label: 'Balanced',
    tagline: 'Recommended default · all features',
    description:
      'All context, minicode, and token-economy engines harmonized: filler-free prose, Lite minicode + bloat guard, 3-tier dedup/scrub/Read-Less (1.5 KB), soft tool pruning, and 1.25x P90 loop guard.',
    concise_mode: 'lite',
    minimalism_mode: 'lite',
    auto_bloat_hint: true,
    read_less: true,
    read_less_min_bytes: 1500,
    microcompact_trigger_ratio: 0.65,
    compaction_trigger_ratio: 0.85,
    auto_micro_skills: true,
    auto_tool_prune: true,
    tool_prune_min_prompts: 10,
    tool_prune_min_sessions: 3,
    p90_loop_guard: true,
    p90_multiplier: 1.25,
  },
  {
    mode: 'high',
    id: 'high',
    label: 'High',
    tagline: 'Strict minicode · high token savings',
    description:
      'Tight telegraphic prose, strict 7-rung Full minicode (140 LOC bloat guard), aggressive 900 B Read-Less + dedup, earlier 55%/78% compaction, faster tool pruning, and 1.15x P90 loop guard.',
    concise_mode: 'full',
    minimalism_mode: 'full',
    auto_bloat_hint: true,
    read_less: true,
    read_less_min_bytes: 900,
    microcompact_trigger_ratio: 0.55,
    compaction_trigger_ratio: 0.78,
    auto_micro_skills: true,
    auto_tool_prune: true,
    tool_prune_min_prompts: 6,
    tool_prune_min_sessions: 2,
    p90_loop_guard: true,
    p90_multiplier: 1.15,
  },
  {
    mode: 'ultra',
    id: 'ultra',
    label: 'Ultra',
    tagline: 'Maximum density & consolidation',
    description:
      'Ultra-dense prose, ruthless Ultra minicode (70 LOC bloat guard), 512 B 3-tier compression, tight 45%/70% compaction, strict tool pruning, and 1.10x P90 loop guard while preserving all safety checks.',
    concise_mode: 'ultra',
    minimalism_mode: 'ultra',
    auto_bloat_hint: true,
    read_less: true,
    read_less_min_bytes: 512,
    microcompact_trigger_ratio: 0.45,
    compaction_trigger_ratio: 0.7,
    auto_micro_skills: true,
    auto_tool_prune: true,
    tool_prune_min_prompts: 4,
    tool_prune_min_sessions: 2,
    p90_loop_guard: true,
    p90_multiplier: 1.1,
  },
]

const ZMODE_CYCLE_ORDER: readonly ZMode[] = ['lite', 'balanced', 'high', 'ultra']

export function formatZModeSpecChips(desc: ZModeDescriptor): string[] {
  const rlKb =
    desc.read_less_min_bytes >= 1024
      ? `${(desc.read_less_min_bytes / 1024).toFixed(desc.read_less_min_bytes % 1024 === 0 ? 0 : 1)}KB`
      : `${desc.read_less_min_bytes}B`
  const mcPct = Math.round(desc.microcompact_trigger_ratio * 100)
  const cmpPct = Math.round(desc.compaction_trigger_ratio * 100)
  return [
    `prose:${desc.concise_mode}`,
    `minicode:${desc.minimalism_mode}`,
    `read-less:${rlKb}`,
    `compact:${mcPct}/${cmpPct}%`,
    `prune:${desc.auto_tool_prune ? 'on' : 'off'}`,
    `p90:${desc.p90_multiplier.toFixed(2)}x`,
  ]
}

export const ZMODE_SHAPE_CLASS: Record<ZModeButtonShape, string> = {
  pill: 'rounded-full',
  rounded: 'rounded-lg',
  square: 'rounded-[5px]',
}

const ZMODE_INNER_SHAPE_CLASS: Record<ZModeButtonShape, string> = {
  pill: 'rounded-full',
  rounded: 'rounded-[6px]',
  square: 'rounded-[3px]',
}

export const ZMODE_CHROME_BASE_CLASS: Record<ZModeButtonChrome, string> = {
  bordered_filled: 'border border-[var(--border)] bg-[var(--card)]',
  border_only: 'border border-[var(--border)] bg-transparent',
  background_only: 'border border-transparent bg-[var(--card)]',
  minimal_bare: 'border border-transparent bg-transparent',
}

interface StyleRecipe {
  containerClass: string
  activeStyle: (chrome: ZModeButtonChrome) => CSSProperties
  activeTextClass: string
  inactiveTextClass: string
  dividerClass: string
  modeBadgeClass: (emphasized: boolean) => string
}

const ZMODE_STYLE_RECIPES: Record<Exclude<ZModeButtonStyle, 'segmented_bar'>, StyleRecipe> = {
  split_pill: {
    containerClass: 'transition-colors',
    activeStyle: (chrome) => ({
      borderColor:
        chrome === 'background_only' || chrome === 'minimal_bare'
          ? 'transparent'
          : 'color-mix(in srgb, var(--zmode-accent) 48%, var(--border))',
      backgroundColor:
        chrome === 'border_only' || chrome === 'minimal_bare'
          ? 'transparent'
          : 'color-mix(in srgb, var(--zmode-accent) 11%, var(--card))',
    }),
    activeTextClass: 'text-[var(--fg)]',
    inactiveTextClass: 'text-[var(--muted)] hover:text-[var(--fg)]',
    dividerClass: 'bg-[var(--border)]/80',
    modeBadgeClass: (emphasized) =>
      emphasized
        ? 'font-mono text-[10.5px] font-semibold text-[var(--zmode-accent)]'
        : 'font-mono text-[10.5px] text-[var(--muted)]',
  },
  accent_solid: {
    containerClass: 'transition-all shadow-xs',
    activeStyle: (chrome) => ({
      borderColor:
        chrome === 'background_only' || chrome === 'minimal_bare'
          ? 'transparent'
          : 'var(--zmode-accent)',
      backgroundColor:
        chrome === 'border_only' || chrome === 'minimal_bare'
          ? 'transparent'
          : 'var(--zmode-accent)',
      color:
        chrome === 'border_only' || chrome === 'minimal_bare'
          ? 'var(--zmode-accent)'
          : 'var(--primary-fg)',
    }),
    activeTextClass: 'text-current',
    inactiveTextClass: 'text-[var(--muted)] hover:text-[var(--fg)]',
    dividerClass: 'bg-current/25',
    modeBadgeClass: (emphasized) =>
      emphasized
        ? 'font-mono text-[10.5px] font-semibold opacity-95'
        : 'font-mono text-[10.5px] text-[var(--muted)]',
  },
  soft_tint: {
    containerClass: 'transition-colors',
    activeStyle: (chrome) => ({
      borderColor:
        chrome === 'background_only' || chrome === 'minimal_bare'
          ? 'transparent'
          : 'color-mix(in srgb, var(--zmode-accent) 42%, transparent)',
      backgroundColor:
        chrome === 'border_only' || chrome === 'minimal_bare'
          ? 'transparent'
          : 'color-mix(in srgb, var(--zmode-accent) 16%, transparent)',
    }),
    activeTextClass: 'text-[var(--zmode-accent)]',
    inactiveTextClass: 'text-[var(--muted)] hover:text-[var(--fg)]',
    dividerClass: 'bg-[var(--zmode-accent)]/25',
    modeBadgeClass: (emphasized) =>
      emphasized
        ? 'font-mono text-[10.5px] font-semibold text-[var(--zmode-accent)]'
        : 'font-mono text-[10.5px] text-[var(--muted)]',
  },
  outline_mono: {
    containerClass: 'font-mono tracking-[0.02em] transition-colors',
    activeStyle: (chrome) => ({
      borderColor:
        chrome === 'background_only' || chrome === 'minimal_bare'
          ? 'transparent'
          : 'var(--zmode-accent)',
      backgroundColor:
        chrome === 'border_only' || chrome === 'minimal_bare'
          ? 'transparent'
          : 'color-mix(in srgb, var(--zmode-accent) 8%, var(--bg))',
    }),
    activeTextClass: 'text-[var(--fg)]',
    inactiveTextClass: 'text-[var(--muted)] hover:text-[var(--fg)]',
    dividerClass: 'bg-[var(--border)]',
    modeBadgeClass: (emphasized) =>
      emphasized
        ? 'text-[10.5px] font-semibold uppercase text-[var(--zmode-accent)]'
        : 'text-[10.5px] uppercase text-[var(--muted)]',
  },
  minimal_ghost: {
    containerClass: 'transition-colors hover:bg-[var(--card)]/70',
    activeStyle: () => ({
      borderColor: 'transparent',
      backgroundColor: 'transparent',
    }),
    activeTextClass: 'text-[var(--zmode-accent)]',
    inactiveTextClass: 'text-[var(--muted)] hover:text-[var(--fg)]',
    dividerClass: 'bg-[var(--border)]/60',
    modeBadgeClass: (emphasized) =>
      emphasized
        ? 'font-mono text-[10.5px] font-semibold text-[var(--zmode-accent)]'
        : 'font-mono text-[10.5px] text-[var(--muted)]',
  },
}

/**
 * Unified, Theme-Customizable ZMode Selector Control (`Lite` / `Balanced` / `High` / `Ultra`).
 * Consumes `ZMODE_REGISTRY` dynamically and strictly inherits global 60/30/10 theme tokens
 * unless overridden in Appearance Studio.
 */
export const ZModeControl = memo(function ZModeControl({
  zmodes,
  zmode = 'balanced',
  zmodeCustomized = false,
  onSelectZMode,
  composerStyle,
  popoverStyle = 'gooey',
  open = false,
  onOpenChange,
}: {
  zmodes: readonly ZModeDescriptor[]
  zmode?: ZMode
  zmodeCustomized?: boolean
  onSelectZMode?: (mode: ZMode) => Promise<void> | void
  composerStyle: ComponentSlots['composer']
  popoverStyle?: 'gooey' | 'crisp'
  open?: boolean
  onOpenChange?: (next: boolean) => void
}) {
  const modes = zmodes.length > 0 ? zmodes : FALLBACK_ZMODES
  const activeDesc = modes.find((m) => m.id === zmode) ?? modes[1] ?? modes[0]
  const isEmphasized = zmode !== 'lite'

  const styleKey: ZModeButtonStyle = composerStyle.concise_button_style || 'split_pill'
  const shapeKey: ZModeButtonShape = composerStyle.concise_button_shape || 'pill'
  const chromeKey: ZModeButtonChrome = composerStyle.concise_button_chrome || 'bordered_filled'
  const accentOverride = composerStyle.concise_button_accent?.trim() || ''

  const shapeClass = ZMODE_SHAPE_CLASS[shapeKey] || ZMODE_SHAPE_CLASS.pill
  const innerShapeClass = ZMODE_INNER_SHAPE_CLASS[shapeKey] || ZMODE_INNER_SHAPE_CLASS.pill
  const chromeBaseClass =
    ZMODE_CHROME_BASE_CLASS[chromeKey] || ZMODE_CHROME_BASE_CLASS.bordered_filled

  const rootStyle = {
    '--zmode-accent': accentOverride || 'var(--accent)',
  } as CSSProperties

  const handleCycleMode = () => {
    const idx = ZMODE_CYCLE_ORDER.indexOf(zmode)
    const nextMode = ZMODE_CYCLE_ORDER[(idx + 1) % ZMODE_CYCLE_ORDER.length]
    onSelectZMode?.(nextMode)
  }

  const handleSelectMode = (nextMode: ZMode) => {
    onSelectZMode?.(nextMode)
    onOpenChange?.(false)
  }

  // Inline Segmented Level Bar layout: ZMode label + inline Lite / Balanced / High / Ultra pills
  if (styleKey === 'segmented_bar') {
    const segStyle: CSSProperties = isEmphasized
      ? {
          ...rootStyle,
          borderColor:
            chromeKey === 'background_only' || chromeKey === 'minimal_bare'
              ? 'transparent'
              : 'color-mix(in srgb, var(--zmode-accent) 45%, var(--border))',
          backgroundColor:
            chromeKey === 'border_only' || chromeKey === 'minimal_bare'
              ? 'transparent'
              : 'color-mix(in srgb, var(--zmode-accent) 8%, var(--card))',
        }
      : rootStyle

    return (
      <div
        style={segStyle}
        className={cn(
          'inline-flex h-7 items-center gap-0.5 px-1 text-[11px] font-medium transition-colors',
          chromeBaseClass,
          shapeClass,
        )}
      >
        <ZenkoTooltip
          label={`ZMode · ${activeDesc.label}${zmodeCustomized ? ' (custom)' : ''} — ${activeDesc.tagline}`}
          side="top"
        >
          <span
            className={cn(
              'inline-flex h-5.5 items-center gap-1 px-1.5',
              innerShapeClass,
              isEmphasized ? 'text-[var(--zmode-accent)]' : 'text-[var(--muted)]',
            )}
          >
            <Zap
              size={11.5}
              className={isEmphasized ? 'text-[var(--zmode-accent)]' : 'text-[var(--muted)]'}
            />
            <span>ZMode</span>
            {zmodeCustomized && (
              <span className="font-mono text-[9.5px] opacity-80" title="Custom knob overrides active">
                *
              </span>
            )}
          </span>
        </ZenkoTooltip>

        <span className="mx-0.5 h-3.5 w-px bg-[var(--border)]/80" aria-hidden="true" />

        <div className="inline-flex items-center gap-0.5" role="group" aria-label="ZMode profile">
          {modes.map((m) => {
            const selected = zmode === m.id
            return (
              <ZenkoTooltip key={m.id} label={`${m.label} — ${m.tagline}`} side="top">
                <button
                  type="button"
                  aria-pressed={selected}
                  onClick={() => handleSelectMode(m.id)}
                  className={cn(
                    'h-5 cursor-pointer px-1.5 font-mono text-[10px] transition-colors',
                    innerShapeClass,
                    selected
                      ? 'font-semibold text-[var(--primary-fg)]'
                      : 'text-[var(--muted)] hover:bg-[var(--surface)] hover:text-[var(--fg)]',
                  )}
                  style={selected ? { backgroundColor: 'var(--zmode-accent)' } : undefined}
                >
                  {m.label}
                </button>
              </ZenkoTooltip>
            )
          })}
        </div>
      </div>
    )
  }

  const recipe = ZMODE_STYLE_RECIPES[styleKey] || ZMODE_STYLE_RECIPES.split_pill
  const activeContainerStyle: CSSProperties = isEmphasized
    ? { ...rootStyle, ...recipe.activeStyle(chromeKey) }
    : rootStyle
  const isMono = styleKey === 'outline_mono'
  const isSolidActive =
    styleKey === 'accent_solid' &&
    isEmphasized &&
    chromeKey !== 'border_only' &&
    chromeKey !== 'minimal_bare'

  const nextIdx = (ZMODE_CYCLE_ORDER.indexOf(zmode) + 1) % ZMODE_CYCLE_ORDER.length
  const nextLabel = modes.find((m) => m.id === ZMODE_CYCLE_ORDER[nextIdx])?.label || 'Balanced'

  return (
    <div
      style={activeContainerStyle}
      className={cn(
        'inline-flex h-7 items-center text-[11.5px] font-medium',
        chromeBaseClass,
        shapeClass,
        recipe.containerClass,
        isEmphasized ? recipe.activeTextClass : recipe.inactiveTextClass,
      )}
    >
      {/* Left Action: 1-Click ZMode Cycle (Lite -> Balanced -> High -> Ultra) */}
      <ZenkoTooltip
        label={`ZMode ${activeDesc.label}${zmodeCustomized ? ' (custom)' : ''} — click to switch to ${nextLabel}`}
        side="top"
        disabled={open}
      >
        <button
          type="button"
          aria-label={`Cycle ZMode (current ${activeDesc.label})`}
          onClick={handleCycleMode}
          className="inline-flex h-full cursor-pointer items-center gap-1.5 pl-2.5 pr-2 transition-opacity hover:opacity-90"
        >
          {isMono ? (
            <span
              className="h-1.5 w-1.5 shrink-0 rounded-full"
              style={{
                backgroundColor: isEmphasized ? 'var(--zmode-accent)' : 'var(--muted)',
              }}
            />
          ) : (
            <Zap
              size={11.5}
              className="shrink-0"
              style={{
                color: isEmphasized
                  ? isSolidActive
                    ? 'var(--primary-fg)'
                    : 'var(--zmode-accent)'
                  : 'var(--muted)',
              }}
            />
          )}
          <span className={isMono ? 'text-[10.5px] uppercase' : undefined}>ZMode</span>
        </button>
      </ZenkoTooltip>

      <span className={cn('h-3.5 w-px shrink-0', recipe.dividerClass)} aria-hidden="true" />

      {/* Right Action: Full ZMode Profile Selector Popover */}
      <ZenkoTooltip label="Select ZMode efficiency profile" side="top" disabled={open}>
        <Popover
          open={open}
          onOpenChange={onOpenChange}
          side="top"
          align="start"
          sideOffset={10}
          panelRadius={14}
          gooStrength={popoverStyle === 'gooey' ? 7 : 0}
        >
          <PopoverTrigger>
            <button
              type="button"
              aria-label="Select ZMode profile"
              className="inline-flex h-full cursor-pointer items-center gap-1 pl-2 pr-2.5 transition-opacity hover:opacity-90"
            >
              <span className={recipe.modeBadgeClass(isEmphasized)}>
                {activeDesc.label}
                {zmodeCustomized ? '*' : ''}
              </span>
              <ChevronDown
                size={11}
                className={cn('transition-transform duration-150', open && 'rotate-180')}
                style={{ opacity: 0.72 }}
              />
            </button>
          </PopoverTrigger>
          <PopoverContent className="w-76 rounded-[14px] border border-[var(--border)] bg-[var(--surface)] p-1.5 shadow-2xl">
            <div className="mb-1 flex items-center justify-between px-2 py-1">
              <div className="flex items-center gap-1.5">
                <Zap size={12} style={{ color: 'var(--zmode-accent)' }} />
                <span className="text-[11.5px] font-semibold text-[var(--fg)]">ZMode</span>
              </div>
              <span className="font-mono text-[10px] text-[var(--muted)]">
                {zmodeCustomized ? `${activeDesc.label} · custom` : activeDesc.label}
              </span>
            </div>

            <div className="flex flex-col gap-0.5">
              {modes.map((m) => {
                const selected = zmode === m.id
                const specs = formatZModeSpecChips(m)
                return (
                  <button
                    key={m.id}
                    type="button"
                    onClick={() => handleSelectMode(m.id)}
                    className={cn(
                      'flex w-full cursor-pointer flex-col gap-0.5 rounded-lg px-2.5 py-2 text-left transition-colors hover:bg-[var(--card)]',
                      selected && 'bg-[var(--card)]',
                    )}
                  >
                    <span className="flex items-center justify-between text-[12px] font-medium text-[var(--fg)]">
                      <span className="flex items-center gap-1.5">
                        <span>{m.label}</span>
                        {m.id === 'balanced' && (
                          <span className="rounded border border-[var(--border)] bg-[var(--bg)] px-1 py-px font-mono text-[9.5px] text-[var(--muted)]">
                            default
                          </span>
                        )}
                      </span>
                      {selected && (
                        <span
                          className="h-1.5 w-1.5 rounded-full"
                          style={{ backgroundColor: 'var(--zmode-accent)' }}
                        />
                      )}
                    </span>
                    <span className="text-[10.5px] leading-snug text-[var(--muted)]">
                      {m.tagline}
                    </span>
                    <span className="mt-0.5 font-mono text-[9.5px] text-[var(--muted)]/85">
                      {specs.slice(0, 4).join(' · ')}
                    </span>
                  </button>
                )
              })}
            </div>

            {zmodeCustomized && onSelectZMode && (
              <div className="mt-1.5 border-t border-[var(--border)]/70 px-1.5 pt-1.5">
                <button
                  type="button"
                  onClick={() => handleSelectMode(zmode)}
                  className="flex w-full cursor-pointer items-center justify-center gap-1.5 rounded-md border border-[var(--border)] bg-[var(--card)] px-2 py-1 font-mono text-[10px] text-[var(--muted)] transition-colors hover:border-[var(--accent)]/50 hover:text-[var(--fg)]"
                >
                  <RotateCcw size={10} />
                  <span>Reset custom knobs to {activeDesc.label}</span>
                </button>
              </div>
            )}
          </PopoverContent>
        </Popover>
      </ZenkoTooltip>
    </div>
  )
})
