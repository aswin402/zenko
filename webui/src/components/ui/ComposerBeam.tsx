'use client'

import { type CSSProperties, type ReactNode } from 'react'
import { BorderBeam, type BorderBeamColorVariant } from 'border-beam'
import { useReducedMotion } from 'motion/react'
import type { ComponentSlots } from '@/lib/types'
import { cn } from '@/lib/utils'

export type ComposerBorderAnimation = NonNullable<ComponentSlots['composer']['border_animation']>
export type ComposerAnimationTrigger = NonNullable<ComponentSlots['composer']['animation_trigger']>
export type ComposerBeamColorVariant = NonNullable<ComponentSlots['composer']['beam_color_variant']>

type BeamSizePreset = 'md' | 'sm' | 'line' | 'pulse-outside' | 'pulse-inner'

const LIBRARIES_DEV_SIZE_MAP: Partial<Record<ComposerBorderAnimation, BeamSizePreset>> = {
  border_beam_md: 'md',
  border_beam_sm: 'sm',
  border_beam_line: 'line',
  border_beam_pulse_out: 'pulse-outside',
  border_beam_pulse_in: 'pulse-inner',
}

const VALID_BEAM_COLORS = new Set<BorderBeamColorVariant>([
  'colorful',
  'mono',
  'ocean',
  'sunset',
  'forest',
  'candy',
  'ice',
  'gold',
])

const SHELL_RADIUS_PX: Record<ComponentSlots['composer']['shell_variant'], number> = {
  floating_card: 20,
  pill_capsule: 28,
  minimal_bar: 12,
  glass_dock: 20,
  motion_brew_prompt: 24,
}

/**
 * Wraps Zenko's Chat Input Box with Libraries.dev `<BorderBeam>` or 60/30/10 Accent Task Animations.
 * Zero idle overhead when paused (`active=false`) or when `prefers-reduced-motion` is enabled.
 */
export function ComposerBeam({
  animation = 'border_beam_md',
  trigger = 'while_working',
  colorVariant = 'theme_accent',
  strength = 0.7,
  shellVariant = 'floating_card',
  isWorking = false,
  isFocused = false,
  themeMode = 'dark',
  children,
}: {
  animation?: ComposerBorderAnimation
  trigger?: ComposerAnimationTrigger
  colorVariant?: ComposerBeamColorVariant
  strength?: number
  shellVariant?: ComponentSlots['composer']['shell_variant']
  isWorking?: boolean
  isFocused?: boolean
  themeMode?: 'dark' | 'light'
  children: ReactNode
}) {
  const reduce = useReducedMotion() ?? false

  const activeByTrigger: Record<ComposerAnimationTrigger, boolean> = {
    while_working: isWorking,
    always: true,
    on_focus: isFocused || isWorking,
    off: false,
  }

  const isActive = !reduce && animation !== 'none' && (activeByTrigger[trigger] ?? false)
  const clampedStrength = Math.max(0.2, Math.min(1, Number.isFinite(strength) ? strength : 0.7))
  const radiusPx = SHELL_RADIUS_PX[shellVariant] ?? 20

  const beamSize = LIBRARIES_DEV_SIZE_MAP[animation]
  if (beamSize) {
    // When user selects `theme_accent`, use Libraries.dev's mono preset layered with an accent aura
    // so every custom theme color glows authentically in its own `--accent` token.
    const isThemeAccent = colorVariant === 'theme_accent'
    const resolvedVariant: BorderBeamColorVariant =
      !isThemeAccent && VALID_BEAM_COLORS.has(colorVariant as BorderBeamColorVariant)
        ? (colorVariant as BorderBeamColorVariant)
        : 'mono'

    return (
      <div
        className={cn(
          'relative w-full transition-shadow duration-300',
          isActive && isThemeAccent && 'zenko-composer-accent-halo',
        )}
        style={
          {
            '--beam-radius': `${radiusPx}px`,
            '--beam-strength-val': String(clampedStrength),
          } as CSSProperties
        }
      >
        <BorderBeam
          size={beamSize}
          colorVariant={resolvedVariant}
          strength={clampedStrength}
          active={isActive}
          theme={themeMode}
          borderRadius={radiusPx}
          staticColors={isThemeAccent}
          className="block w-full"
        >
          {children}
        </BorderBeam>
      </div>
    )
  }

  if (
    animation === 'accent_orbit' ||
    animation === 'accent_shimmer_edge' ||
    animation === 'accent_breathe_halo'
  ) {
    return (
      <div
        data-composer-anim={isActive ? animation : 'idle'}
        className="zenko-composer-anim-shell relative w-full"
        style={
          {
            '--beam-radius': `${radiusPx}px`,
            '--beam-strength-val': String(clampedStrength),
          } as CSSProperties
        }
      >
        {isActive && <span aria-hidden="true" className="zenko-composer-anim-layer" />}
        <div className="relative z-10 w-full">{children}</div>
      </div>
    )
  }

  return <>{children}</>
}
