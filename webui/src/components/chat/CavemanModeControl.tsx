import { memo, useEffect, useRef, type CSSProperties } from 'react'
import { ChevronDown, Flame } from 'lucide-react'
import type { ComponentSlots, ConciseMode, ConciseModeDescriptor } from '@/lib/types'
import { cn } from '@/lib/utils'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/motion/popover'
import { ZenkoTooltip } from '@/components/ui/Tooltip'

export type ConciseButtonStyle = NonNullable<ComponentSlots['composer']['concise_button_style']>
export type ConciseButtonShape = NonNullable<ComponentSlots['composer']['concise_button_shape']>
export type ConciseButtonChrome = NonNullable<ComponentSlots['composer']['concise_button_chrome']>

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

export const CONCISE_SHAPE_CLASS: Record<ConciseButtonShape, string> = {
  pill: 'rounded-full',
  rounded: 'rounded-lg',
  square: 'rounded-[5px]',
}

const CONCISE_INNER_SHAPE_CLASS: Record<ConciseButtonShape, string> = {
  pill: 'rounded-full',
  rounded: 'rounded-[6px]',
  square: 'rounded-[3px]',
}

export const CONCISE_CHROME_BASE_CLASS: Record<ConciseButtonChrome, string> = {
  bordered_filled: 'border border-[var(--border)] bg-[var(--card)]',
  border_only: 'border border-[var(--border)] bg-transparent',
  background_only: 'border border-transparent bg-[var(--card)]',
  minimal_bare: 'border border-transparent bg-transparent',
}

interface StyleRecipe {
  containerClass: string
  activeStyle: (chrome: ConciseButtonChrome) => CSSProperties
  activeTextClass: string
  inactiveTextClass: string
  dividerClass: string
  modeBadgeClass: (isOn: boolean) => string
}

const CONCISE_STYLE_RECIPES: Record<Exclude<ConciseButtonStyle, 'segmented_bar'>, StyleRecipe> = {
  split_pill: {
    containerClass: 'transition-colors',
    activeStyle: (chrome) => ({
      borderColor:
        chrome === 'background_only' || chrome === 'minimal_bare'
          ? 'transparent'
          : 'color-mix(in srgb, var(--caveman-accent) 48%, var(--border))',
      backgroundColor:
        chrome === 'border_only' || chrome === 'minimal_bare'
          ? 'transparent'
          : 'color-mix(in srgb, var(--caveman-accent) 11%, var(--card))',
    }),
    activeTextClass: 'text-[var(--fg)]',
    inactiveTextClass: 'text-[var(--muted)] hover:text-[var(--fg)]',
    dividerClass: 'bg-[var(--border)]/80',
    modeBadgeClass: (isOn) =>
      isOn ? 'font-mono text-[10.5px] font-semibold text-[var(--caveman-accent)]' : 'font-mono text-[10.5px] text-[var(--muted)]',
  },
  accent_solid: {
    containerClass: 'transition-all shadow-xs',
    activeStyle: (chrome) => ({
      borderColor: chrome === 'background_only' || chrome === 'minimal_bare' ? 'transparent' : 'var(--caveman-accent)',
      backgroundColor: chrome === 'border_only' || chrome === 'minimal_bare' ? 'transparent' : 'var(--caveman-accent)',
      color: chrome === 'border_only' || chrome === 'minimal_bare' ? 'var(--caveman-accent)' : 'var(--primary-fg)',
    }),
    activeTextClass: 'text-current',
    inactiveTextClass: 'text-[var(--muted)] hover:text-[var(--fg)]',
    dividerClass: 'bg-current/25',
    modeBadgeClass: (isOn) => (isOn ? 'font-mono text-[10.5px] font-semibold opacity-95' : 'font-mono text-[10.5px] text-[var(--muted)]'),
  },
  soft_tint: {
    containerClass: 'transition-colors',
    activeStyle: (chrome) => ({
      borderColor:
        chrome === 'background_only' || chrome === 'minimal_bare'
          ? 'transparent'
          : 'color-mix(in srgb, var(--caveman-accent) 42%, transparent)',
      backgroundColor:
        chrome === 'border_only' || chrome === 'minimal_bare'
          ? 'transparent'
          : 'color-mix(in srgb, var(--caveman-accent) 16%, transparent)',
    }),
    activeTextClass: 'text-[var(--caveman-accent)]',
    inactiveTextClass: 'text-[var(--muted)] hover:text-[var(--fg)]',
    dividerClass: 'bg-[var(--caveman-accent)]/25',
    modeBadgeClass: (isOn) =>
      isOn ? 'font-mono text-[10.5px] font-semibold text-[var(--caveman-accent)]' : 'font-mono text-[10.5px] text-[var(--muted)]',
  },
  outline_mono: {
    containerClass: 'font-mono tracking-[0.02em] transition-colors',
    activeStyle: (chrome) => ({
      borderColor:
        chrome === 'background_only' || chrome === 'minimal_bare'
          ? 'transparent'
          : 'var(--caveman-accent)',
      backgroundColor:
        chrome === 'border_only' || chrome === 'minimal_bare'
          ? 'transparent'
          : 'color-mix(in srgb, var(--caveman-accent) 8%, var(--bg))',
    }),
    activeTextClass: 'text-[var(--fg)]',
    inactiveTextClass: 'text-[var(--muted)] hover:text-[var(--fg)]',
    dividerClass: 'bg-[var(--border)]',
    modeBadgeClass: (isOn) =>
      isOn ? 'text-[10.5px] font-semibold uppercase text-[var(--caveman-accent)]' : 'text-[10.5px] uppercase text-[var(--muted)]',
  },
  minimal_ghost: {
    containerClass: 'transition-colors hover:bg-[var(--card)]/70',
    activeStyle: () => ({
      borderColor: 'transparent',
      backgroundColor: 'transparent',
    }),
    activeTextClass: 'text-[var(--caveman-accent)]',
    inactiveTextClass: 'text-[var(--muted)] hover:text-[var(--fg)]',
    dividerClass: 'bg-[var(--border)]/60',
    modeBadgeClass: (isOn) =>
      isOn ? 'font-mono text-[10.5px] font-semibold text-[var(--caveman-accent)]' : 'font-mono text-[10.5px] text-[var(--muted)]',
  },
}

/**
 * Reusable, Theme-Customizable Caveman Mode On/Off + Mode Selector Control.
 * Consumes `CONCISE_MODE_REGISTRY` dynamically (zero hardcoded levels or colors) and
 * strictly inherits global 60/30/10 theme tokens unless overridden in Appearance Studio.
 */
export const CavemanModeControl = memo(function CavemanModeControl({
  conciseModes,
  conciseMode,
  readLessEnabled = true,
  autoClarityEnabled = true,
  onUpdateConcise,
  composerStyle,
  popoverStyle = 'gooey',
  open = false,
  onOpenChange,
}: {
  conciseModes: readonly ConciseModeDescriptor[]
  conciseMode: ConciseMode
  readLessEnabled?: boolean
  autoClarityEnabled?: boolean
  onUpdateConcise?: (patch: ConciseUpdatePatch) => Promise<void> | void
  composerStyle: ComponentSlots['composer']
  popoverStyle?: 'gooey' | 'crisp'
  open?: boolean
  onOpenChange?: (next: boolean) => void
}) {
  const modes = conciseModes.length > 0 ? conciseModes : FALLBACK_CONCISE_MODES
  const activeLevels = modes.filter((m) => m.id !== 'off')
  const isOn = conciseMode !== 'off'
  const lastActiveRef = useRef<ConciseMode>(isOn ? conciseMode : 'lite')

  useEffect(() => {
    if (conciseMode !== 'off') {
      lastActiveRef.current = conciseMode
    }
  }, [conciseMode])

  const activeDesc = modes.find((m) => m.id === conciseMode) ?? modes[0]
  const styleKey: ConciseButtonStyle = composerStyle.concise_button_style || 'split_pill'
  const shapeKey: ConciseButtonShape = composerStyle.concise_button_shape || 'pill'
  const chromeKey: ConciseButtonChrome = composerStyle.concise_button_chrome || 'bordered_filled'
  const accentOverride = composerStyle.concise_button_accent?.trim() || ''

  const shapeClass = CONCISE_SHAPE_CLASS[shapeKey] || CONCISE_SHAPE_CLASS.pill
  const innerShapeClass = CONCISE_INNER_SHAPE_CLASS[shapeKey] || CONCISE_INNER_SHAPE_CLASS.pill
  const chromeBaseClass = CONCISE_CHROME_BASE_CLASS[chromeKey] || CONCISE_CHROME_BASE_CLASS.bordered_filled

  const rootStyle = {
    '--caveman-accent': accentOverride || 'var(--accent)',
  } as CSSProperties

  const handleToggleOnOff = () => {
    const nextMode: ConciseMode = isOn ? 'off' : lastActiveRef.current
    onUpdateConcise?.({ concise_mode: nextMode })
  }

  const handleSelectMode = (nextMode: ConciseMode) => {
    if (nextMode !== 'off') {
      lastActiveRef.current = nextMode
    }
    onUpdateConcise?.({ concise_mode: nextMode })
    onOpenChange?.(false)
  }

  // Inline Segmented Level Bar layout: 1-click On/Off + inline Lite / Full / Ultra pills
  if (styleKey === 'segmented_bar') {
    const segStyle: CSSProperties = isOn
      ? {
          ...rootStyle,
          borderColor:
            chromeKey === 'background_only' || chromeKey === 'minimal_bare'
              ? 'transparent'
              : 'color-mix(in srgb, var(--caveman-accent) 45%, var(--border))',
          backgroundColor:
            chromeKey === 'border_only' || chromeKey === 'minimal_bare'
              ? 'transparent'
              : 'color-mix(in srgb, var(--caveman-accent) 8%, var(--card))',
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
          label={isOn ? `Turn Caveman Mode OFF (currently ${activeDesc.label})` : `Turn Caveman Mode ON (${lastActiveRef.current})`}
          side="top"
        >
          <button
            type="button"
            aria-label="Toggle Caveman Mode"
            aria-pressed={isOn}
            onClick={handleToggleOnOff}
            className={cn(
              'inline-flex h-5.5 cursor-pointer items-center gap-1 px-1.5 transition-colors',
              innerShapeClass,
              isOn ? 'text-[var(--caveman-accent)]' : 'text-[var(--muted)] hover:text-[var(--fg)]',
            )}
          >
            <Flame size={11.5} className={isOn ? 'text-[var(--caveman-accent)]' : 'text-[var(--muted)]'} />
            <span>Caveman</span>
            <span className="font-mono text-[9.5px] opacity-80">{isOn ? 'ON' : 'OFF'}</span>
          </button>
        </ZenkoTooltip>

        <span className="mx-0.5 h-3.5 w-px bg-[var(--border)]/80" aria-hidden="true" />

        <div className="inline-flex items-center gap-0.5" role="group" aria-label="Caveman compression level">
          {activeLevels.map((m) => {
            const selected = conciseMode === m.id
            return (
              <ZenkoTooltip key={m.id} label={`${m.label} (~${m.target_reduction_pct}% fewer tokens)`} side="top">
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
                  style={selected ? { backgroundColor: 'var(--caveman-accent)' } : undefined}
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

  const recipe = CONCISE_STYLE_RECIPES[styleKey] || CONCISE_STYLE_RECIPES.split_pill
  const activeContainerStyle: CSSProperties = isOn
    ? { ...rootStyle, ...recipe.activeStyle(chromeKey) }
    : rootStyle
  const isMono = styleKey === 'outline_mono'
  const isSolidActive =
    styleKey === 'accent_solid' &&
    isOn &&
    chromeKey !== 'border_only' &&
    chromeKey !== 'minimal_bare'

  return (
    <div
      style={activeContainerStyle}
      className={cn(
        'inline-flex h-7 items-center text-[11.5px] font-medium',
        chromeBaseClass,
        shapeClass,
        recipe.containerClass,
        isOn ? recipe.activeTextClass : recipe.inactiveTextClass,
      )}
    >
      {/* Left Action: 1-Click Caveman Mode ON / OFF Toggle */}
      <ZenkoTooltip
        label={
          isOn
            ? `Caveman Mode ON (${activeDesc.label}) — click to turn OFF`
            : `Caveman Mode OFF — click to enable (${lastActiveRef.current})`
        }
        side="top"
        disabled={open}
      >
        <button
          type="button"
          aria-label="Toggle Caveman Mode on or off"
          aria-pressed={isOn}
          onClick={handleToggleOnOff}
          className="inline-flex h-full cursor-pointer items-center gap-1.5 pl-2.5 pr-2 transition-opacity hover:opacity-90"
        >
          {isMono ? (
            <span
              className="h-1.5 w-1.5 shrink-0 rounded-full"
              style={{ backgroundColor: isOn ? 'var(--caveman-accent)' : 'var(--muted)' }}
            />
          ) : (
            <Flame
              size={11.5}
              className="shrink-0"
              style={{
                color: isOn
                  ? isSolidActive
                    ? 'var(--primary-fg)'
                    : 'var(--caveman-accent)'
                  : 'var(--muted)',
              }}
            />
          )}
          <span className={isMono ? 'text-[10.5px] uppercase' : undefined}>
            Caveman {isOn ? 'ON' : 'OFF'}
          </span>
        </button>
      </ZenkoTooltip>

      <span className={cn('h-3.5 w-px shrink-0', recipe.dividerClass)} aria-hidden="true" />

      {/* Right Action: Mode Selector Popover (Off / Lite / Full / Ultra + Efficiency Toggles) */}
      <ZenkoTooltip label="Select Caveman compression level" side="top" disabled={open}>
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
              aria-label="Select Caveman mode level"
              className="inline-flex h-full cursor-pointer items-center gap-1 pl-2 pr-2.5 transition-opacity hover:opacity-90"
            >
              <span className={recipe.modeBadgeClass(isOn)}>{activeDesc.label}</span>
              <ChevronDown
                size={11}
                className={cn('transition-transform duration-150', open && 'rotate-180')}
                style={{ opacity: 0.72 }}
              />
            </button>
          </PopoverTrigger>
          <PopoverContent className="w-64 rounded-[14px] border border-[var(--border)] bg-[var(--surface)] p-1.5 shadow-2xl">
            <div className="mb-1 flex items-center justify-between px-2 py-1">
              <span className="text-[11px] font-semibold text-[var(--fg)]">Caveman Mode</span>
              <button
                type="button"
                onClick={handleToggleOnOff}
                className="cursor-pointer rounded-md border px-1.5 py-0.5 font-mono text-[10px] transition-colors"
                style={
                  isOn
                    ? {
                        borderColor: 'color-mix(in srgb, var(--caveman-accent) 55%, transparent)',
                        backgroundColor: 'color-mix(in srgb, var(--caveman-accent) 14%, transparent)',
                        color: 'var(--caveman-accent)',
                      }
                    : {
                        borderColor: 'var(--border)',
                        backgroundColor: 'var(--card)',
                        color: 'var(--muted)',
                      }
                }
              >
                {isOn ? 'ON' : 'OFF'}
              </button>
            </div>

            <div className="flex flex-col gap-0.5">
              {modes.map((m) => {
                const selected = conciseMode === m.id
                return (
                  <button
                    key={m.id}
                    type="button"
                    onClick={() => handleSelectMode(m.id)}
                    className={cn(
                      'flex w-full cursor-pointer flex-col rounded-lg px-2.5 py-1.5 text-left transition-colors hover:bg-[var(--card)]',
                      selected && 'bg-[var(--card)]',
                    )}
                  >
                    <span className="flex items-center justify-between text-[12px] font-medium text-[var(--fg)]">
                      <span className="flex items-center gap-1.5">
                        <span>{m.label}</span>
                        {m.target_reduction_pct > 0 && (
                          <span className="font-mono text-[10px] text-[var(--muted)]">
                            -{m.target_reduction_pct}% tok
                          </span>
                        )}
                      </span>
                      {selected && (
                        <span
                          className="h-1.5 w-1.5 rounded-full"
                          style={{ backgroundColor: 'var(--caveman-accent)' }}
                        />
                      )}
                    </span>
                    <span className="text-[10.5px] leading-snug text-[var(--muted)]">
                      {m.description}
                    </span>
                  </button>
                )
              })}
            </div>

            {onUpdateConcise && (
              <div className="mt-1.5 flex items-center justify-between gap-1.5 border-t border-[var(--border)]/70 px-1.5 pt-1.5">
                <button
                  type="button"
                  onClick={() => onUpdateConcise({ read_less: !readLessEnabled })}
                  className="flex-1 cursor-pointer rounded-md border px-2 py-1 font-mono text-[10px] transition-colors"
                  style={
                    readLessEnabled
                      ? {
                          borderColor: 'color-mix(in srgb, var(--caveman-accent) 48%, transparent)',
                          backgroundColor: 'color-mix(in srgb, var(--caveman-accent) 12%, transparent)',
                          color: 'var(--caveman-accent)',
                        }
                      : {
                          borderColor: 'var(--border)',
                          backgroundColor: 'var(--card)',
                          color: 'var(--muted)',
                        }
                  }
                >
                  Read-Less {readLessEnabled ? 'ON' : 'OFF'}
                </button>
                <button
                  type="button"
                  onClick={() => onUpdateConcise({ auto_clarity: !autoClarityEnabled })}
                  className="flex-1 cursor-pointer rounded-md border px-2 py-1 font-mono text-[10px] transition-colors"
                  style={
                    autoClarityEnabled
                      ? {
                          borderColor: 'color-mix(in srgb, var(--caveman-accent) 48%, transparent)',
                          backgroundColor: 'color-mix(in srgb, var(--caveman-accent) 12%, transparent)',
                          color: 'var(--caveman-accent)',
                        }
                      : {
                          borderColor: 'var(--border)',
                          backgroundColor: 'var(--card)',
                          color: 'var(--muted)',
                        }
                  }
                >
                  Auto-Clarity {autoClarityEnabled ? 'ON' : 'OFF'}
                </button>
              </div>
            )}
          </PopoverContent>
        </Popover>
      </ZenkoTooltip>
    </div>
  )
})
