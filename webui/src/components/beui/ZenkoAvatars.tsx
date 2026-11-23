import type { CSSProperties, ReactNode } from 'react'
import type { AvatarFrameStyle, UiCustomization } from '@/lib/types'
import { cn } from '@/lib/utils'
import { ZenkoFoxLogo } from './ZenkoFoxLogo'
import { ZenkoIcon } from './ZenkoIcon'

/** Handcrafted pure-SVG Fox Emblems inspired by diverse Flaticon fox icon styles (32x32 viewBox). */
const FOX_AVATAR_RENDERERS: Readonly<
  Record<string, (size: number, color: string) => ReactNode>
> = {
  liquid_halo: (size, color) => <ZenkoFoxLogo size={size} fillColor={color} />,

  // Sharp faceted geometric / low-poly fox emblem
  fox_geometric: (size, color) => (
    <svg width={size} height={size} viewBox="0 0 32 32" fill="none" aria-hidden="true">
      <polygon points="5.5,3.5 12.6,11.2 4.5,16.5" fill={color} fillOpacity="0.92" />
      <polygon points="26.5,3.5 19.4,11.2 27.5,16.5" fill={color} fillOpacity="0.92" />
      <polygon points="12.6,11.2 19.4,11.2 16,22.5" fill={color} />
      <polygon points="4.5,16.5 12.6,11.2 16,28.5" fill={color} fillOpacity="0.7" />
      <polygon points="27.5,16.5 19.4,11.2 16,28.5" fill={color} fillOpacity="0.7" />
      <polygon points="12.6,11.2 16,22.5 16,28.5" fill={color} fillOpacity="0.86" />
      <polygon points="19.4,11.2 16,22.5 16,28.5" fill={color} fillOpacity="0.6" />
    </svg>
  ),

  // Crisp single-weight minimalist vector outline fox face
  fox_line: (size, color) => (
    <svg
      width={size}
      height={size}
      viewBox="0 0 32 32"
      fill="none"
      stroke={color}
      strokeWidth="2.1"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M6.5 5.2L12.4 10.6C14.7 9.8 17.3 9.8 19.6 10.6L25.5 5.2L27.2 16.5C27.2 21.4 21.3 26.2 16 28C10.7 26.2 4.8 21.4 4.8 16.5L6.5 5.2Z" />
      <path d="M5.4 17.2Q10.8 18.8 16 27.4" strokeOpacity="0.75" />
      <path d="M26.6 17.2Q21.2 18.8 16 27.4" strokeOpacity="0.75" />
      <circle cx="16" cy="22.8" r="1.4" fill={color} stroke="none" />
    </svg>
  ),

  // Friendly rounded Flaticon-style cute fox portrait with soft inner ears & cheeks
  fox_cute: (size, color) => (
    <svg width={size} height={size} viewBox="0 0 32 32" fill="none" aria-hidden="true">
      <path
        d="M6.4 5.4C7.2 4.2 8.7 4.4 9.7 5.4L12.9 8.7C14.9 8.2 17.1 8.2 19.1 8.7L22.3 5.4C23.3 4.4 24.8 4.2 25.6 5.4C26.7 7.4 27.3 11.2 27 14.6C28.7 17.1 28.2 20.6 25.5 22.8C22.7 25.2 19.2 27.2 16 28C12.8 27.2 9.3 25.2 6.5 22.8C3.8 20.6 3.3 17.1 5 14.6C4.7 11.2 5.3 7.4 6.4 5.4Z"
        fill={color}
      />
      <path d="M7.6 8.3L10.4 11.2L7.3 13.1Z" fill="var(--bg)" fillOpacity="0.3" />
      <path d="M24.4 8.3L21.6 11.2L24.7 13.1Z" fill="var(--bg)" fillOpacity="0.3" />
      <path
        d="M5.8 18.2C8.9 18.4 12.2 20.8 14.3 25.5C10.5 24.4 7.1 21.8 5.8 18.2Z"
        fill="var(--bg)"
        fillOpacity="0.3"
      />
      <path
        d="M26.2 18.2C23.1 18.4 19.8 20.8 17.7 25.5C21.5 24.4 24.9 21.8 26.2 18.2Z"
        fill="var(--bg)"
        fillOpacity="0.3"
      />
    </svg>
  ),

  // Folded angular origami paper-craft fox
  fox_origami: (size, color) => (
    <svg width={size} height={size} viewBox="0 0 32 32" fill="none" aria-hidden="true">
      <polygon points="6,3.2 14,11 7.2,14.4" fill={color} />
      <polygon points="26,3.2 18,11 24.8,14.4" fill={color} fillOpacity="0.88" />
      <polygon points="14,11 16,9.2 18,11 16,19.4" fill={color} />
      <polygon points="7.2,14.4 14,11 16,19.4 3.8,18.4" fill={color} fillOpacity="0.72" />
      <polygon points="24.8,14.4 18,11 16,19.4 28.2,18.4" fill={color} fillOpacity="0.62" />
      <polygon points="3.8,18.4 16,19.4 16,28.8" fill={color} fillOpacity="0.52" />
      <polygon points="28.2,18.4 16,19.4 16,28.8" fill={color} fillOpacity="0.84" />
    </svg>
  ),

  // Ceremonial Japanese Kitsune fox mask silhouette
  fox_kitsune: (size, color) => (
    <svg width={size} height={size} viewBox="0 0 32 32" fill="none" aria-hidden="true">
      <path
        d="M7.6 2.6L13.1 10.2Q16 9.3 18.9 10.2L24.4 2.6C25.5 6.8 25.8 10.5 25.1 13.8C27 16 26.7 18.9 24.4 21.6L17.4 28.6C16.6 29.4 15.4 29.4 14.6 28.6L7.6 21.6C5.3 18.9 5 16 6.9 13.8C6.2 10.5 6.5 6.8 7.6 2.6Z"
        fill={color}
      />
      <polygon points="16,11.2 17.8,13.6 16,16 14.2,13.6" fill="var(--bg)" fillOpacity="0.42" />
      <path d="M9.6 16.5L13.4 18.2" stroke="var(--bg)" strokeOpacity="0.48" strokeWidth="1.6" strokeLinecap="round" />
      <path d="M22.4 16.5L18.6 18.2" stroke="var(--bg)" strokeOpacity="0.48" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  ),

  // Sleek side-profile alert fox head
  fox_side: (size, color) => (
    <svg width={size} height={size} viewBox="0 0 32 32" fill="none" aria-hidden="true">
      <path
        d="M9 4.4L15.6 9.8L20.6 5.6L20.4 12L28.6 16.9C29.4 17.4 29.2 18.7 28.2 19L20.4 21.1C18.8 24.3 15.3 27.1 10.4 28.2C7.3 24.8 6.1 20.5 6.9 15.3L7.9 6C8.1 4.8 8.4 4 9 4.4Z"
        fill={color}
      />
      <path d="M10.2 8.2L13.8 11.4L9.8 13.4Z" fill="var(--bg)" fillOpacity="0.32" />
      <path
        d="M11.4 22.8C14.8 22.6 18.2 21.1 20.2 18.6C18.4 22.5 15.1 25.1 11.4 26.2Z"
        fill="var(--bg)"
        fillOpacity="0.3"
      />
    </svg>
  ),

  // Cozy minimalist curled sleeping fox & fluffy tail ring
  fox_sleeping: (size, color) => (
    <svg width={size} height={size} viewBox="0 0 32 32" fill="none" aria-hidden="true">
      <path
        d="M16 5.2C22 5.2 26.8 10 26.8 16C26.8 22 22 26.8 16 26.8C10 26.8 5.2 22 5.2 16C5.2 13.6 6 11.3 7.3 9.5L6.5 5.4L10.8 7C12.4 5.9 14.1 5.2 16 5.2Z"
        fill={color}
      />
      <path
        d="M16 10.6C19 10.6 21.4 13 21.4 16C21.4 19 19 21.4 16 21.4C13.6 21.4 11.6 19.9 10.9 17.7C13.4 18.3 16.5 17.4 17.8 14.9C18.4 13.7 17.8 11.9 16 10.6Z"
        fill="var(--bg)"
        fillOpacity="0.36"
      />
      <path
        d="M21.8 19.8C20.2 22.4 17.3 24 14 23.8C16.2 21.6 19.1 20.2 21.8 19.8Z"
        fill="var(--bg)"
        fillOpacity="0.45"
      />
    </svg>
  ),

  // Bold symmetrical heraldic shield fox crest
  fox_crest: (size, color) => (
    <svg width={size} height={size} viewBox="0 0 32 32" fill="none" aria-hidden="true">
      <path
        d="M5 4L12.5 9.5L16 8.5L19.5 9.5L27 4L25.6 14.2L27.8 17.4L16 29L4.2 17.4L6.4 14.2L5 4Z"
        fill={color}
      />
      <path
        d="M16 11.5L19.8 15.8L16 23.5L12.2 15.8L16 11.5Z"
        fill="var(--bg)"
        fillOpacity="0.28"
      />
    </svg>
  ),

  // Futuristic hexagonal angular cyber-fox mark
  fox_cyber: (size, color) => (
    <svg width={size} height={size} viewBox="0 0 32 32" fill="none" aria-hidden="true">
      <path
        d="M6 4.5L12.5 9.5H19.5L26 4.5L27.5 16L22.5 23.5L16 28.5L9.5 23.5L4.5 16L6 4.5Z"
        stroke={color}
        strokeWidth="2"
        strokeLinejoin="round"
        fill={color}
        fillOpacity="0.16"
      />
      <polygon points="9.5,14.5 14.5,16.5 12,18.2" fill={color} />
      <polygon points="22.5,14.5 17.5,16.5 20,18.2" fill={color} />
      <polygon points="16,21 18.2,23.2 16,25.4 13.8,23.2" fill={color} />
    </svg>
  ),
}

/** Authentic User / Human / Profile vector avatars (24x24 viewBox, inheriting icon stroke & color). */
const USER_PROFILE_RENDERERS: Readonly<
  Record<string, (size: number, color: string, sw: number) => ReactNode>
> = {
  // Profile Circle: person inside circular badge
  user_circle: (size, color, sw) => (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <circle cx="12" cy="12" r="9.5" />
      <circle cx="12" cy="9.5" r="3" />
      <path d="M6.8 18.4C8.1 16.3 9.9 15.2 12 15.2C14.1 15.2 15.9 16.3 17.2 18.4" />
    </svg>
  ),

  // Minimal Human: smooth solid silhouette
  user_round: (size, color) => (
    <svg width={size} height={size} viewBox="0 0 24 24" fill={color} aria-hidden="true">
      <circle cx="12" cy="8" r="4" />
      <path d="M4.5 20C4.5 16.2 7.8 13.8 12 13.8C16.2 13.8 19.5 16.2 19.5 20C19.5 20.6 19 21 18.4 21H5.6C5 21 4.5 20.6 4.5 20Z" />
    </svg>
  ),

  // Verified Member: human profile + crisp tick
  user_check: (size, color, sw) => (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <circle cx="9.5" cy="7.5" r="3.5" />
      <path d="M3.5 20C3.5 16.7 6.2 14.5 9.5 14.5C11.4 14.5 13 15.2 14.1 16.4" />
      <path d="M15.5 17.5L17.5 19.5L21.5 15" />
    </svg>
  ),

  // Dev Profile: human profile + </> mark
  user_code: (size, color, sw) => (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <circle cx="9.5" cy="7.5" r="3.5" />
      <path d="M3.5 20C3.5 16.7 6.2 14.5 9.5 14.5C11 14.5 12.3 15 13.3 15.8" />
      <path d="M16.5 15L14.5 17.5L16.5 20" />
      <path d="M19.5 15L21.5 17.5L19.5 20" />
    </svg>
  ),

  // Workspace Owner: human profile with minimalist crown
  user_crown: (size, color, sw) => (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M8 6.5L9.4 3.5L12 5.2L14.6 3.5L16 6.5H8Z" fill={color} fillOpacity="0.22" />
      <circle cx="12" cy="11.2" r="3.1" />
      <path d="M5.5 20.5C5.5 17.4 8.4 15.8 12 15.8C15.6 15.8 18.5 17.4 18.5 20.5" />
    </svg>
  ),

  // Architect Persona: minimal human portrait with spectacles
  user_glasses: (size, color, sw) => (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <circle cx="12" cy="12" r="9" strokeOpacity="0.55" />
      <rect x="6.6" y="9.5" width="4.4" height="3.2" rx="1.2" />
      <rect x="13" y="9.5" width="4.4" height="3.2" rx="1.2" />
      <path d="M11 11H13" />
      <path d="M9.5 15.8C10.3 16.5 11.1 16.8 12 16.8C12.9 16.8 13.7 16.5 14.5 15.8" />
    </svg>
  ),

  // Friendly Portrait: approachable minimalist human face
  user_smile: (size, color, sw) => (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <circle cx="12" cy="12" r="9.2" />
      <circle cx="9.2" cy="10" r="1" fill={color} stroke="none" />
      <circle cx="14.8" cy="10" r="1" fill={color} stroke="none" />
      <path d="M8.8 14.6C9.6 15.9 10.7 16.6 12 16.6C13.3 16.6 14.4 15.9 15.2 14.6" />
    </svg>
  ),

  // Builder Silhouette: modern hooded developer portrait
  user_hoodie: (size, color, sw) => (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M5.5 20.2C5.5 16.2 7.8 14 12 14C16.2 14 18.5 16.2 18.5 20.2" />
      <path d="M7.5 11.5C7.5 7.4 9.2 4.5 12 4.5C14.8 4.5 16.5 7.4 16.5 11.5C16.5 13.2 15.4 14.5 14 14.8" />
      <circle cx="12" cy="9.8" r="2.6" fill={color} fillOpacity="0.18" />
      <path d="M10.2 14.8V17.5M13.8 14.8V17.5" strokeOpacity="0.7" />
    </svg>
  ),
}

interface FrameVisualSpec {
  className: string
  styleForColor: (tint: string) => CSSProperties | undefined
  solidAccent?: boolean
  bare?: boolean
}

const AVATAR_FRAME_SPEC: Readonly<Record<AvatarFrameStyle, FrameVisualSpec>> = {
  bare: {
    className: 'bg-transparent border-transparent shadow-none!',
    styleForColor: () => undefined,
    bare: true,
  },
  squircle: {
    className: 'rounded-[30%] border border-[var(--border)] bg-[var(--surface)]/90 shadow-xs',
    styleForColor: () => undefined,
  },
  circle: {
    className: 'rounded-full border border-[var(--border)] bg-[var(--surface)]/90',
    styleForColor: () => undefined,
  },
  soft_tint: {
    className: 'rounded-[30%] border',
    styleForColor: (tint) => ({
      backgroundColor: `color-mix(in srgb, ${tint} 13%, var(--surface))`,
      borderColor: `color-mix(in srgb, ${tint} 38%, var(--border))`,
    }),
  },
  accent_solid: {
    className: 'rounded-[30%] border border-transparent shadow-xs',
    styleForColor: (tint) => ({
      backgroundColor: tint,
      color: 'var(--primary-fg)',
    }),
    solidAccent: true,
  },
  ring: {
    className: 'rounded-full border-[1.5px] bg-transparent',
    styleForColor: (tint) => ({
      borderColor: `color-mix(in srgb, ${tint} 72%, var(--border))`,
    }),
  },
}

function resolveFrame(frame?: string): FrameVisualSpec {
  return AVATAR_FRAME_SPEC[(frame as AvatarFrameStyle) || 'squircle'] || AVATAR_FRAME_SPEC.squircle
}

function normalizeCustomColor(raw?: string): string | null {
  const trimmed = (raw || '').trim()
  return trimmed.length > 0 ? trimmed : null
}

/**
 * Unified customizable Zenko Fox Agent Avatar.
 * Supports 10 handcrafted Flaticon-inspired Fox emblems, custom color overrides,
 * and toggleable container frames (including `bare` = Hidden / No Frame).
 */
export function AgentAvatar({
  variant = 'liquid_halo',
  frame = 'squircle',
  color,
  size = 28,
  status = 'idle',
  className,
}: {
  variant?: string
  frame?: AvatarFrameStyle | string
  color?: string
  icons?: UiCustomization['icons']
  size?: number
  status?: 'idle' | 'running' | 'waiting_approval' | 'error'
  className?: string
}) {
  if (variant === 'hidden') return null

  const frameSpec = resolveFrame(frame)
  const customTint = normalizeCustomColor(color)
  const effectiveTint =
    status === 'error' ? 'var(--destructive)' : customTint || 'var(--accent)'
  const glyphSize = frameSpec.bare ? size : Math.max(14, Math.round(size * 0.62))
  const glyphColor = frameSpec.solidAccent ? 'var(--primary-fg)' : effectiveTint
  const renderFox = FOX_AVATAR_RENDERERS[variant] || FOX_AVATAR_RENDERERS.liquid_halo

  return (
    <span
      className={cn(
        'relative inline-flex shrink-0 items-center justify-center transition-colors',
        frameSpec.className,
        status === 'running' && 'animate-pulse',
        className,
      )}
      style={{
        width: size,
        height: size,
        ...frameSpec.styleForColor(effectiveTint),
      }}
    >
      {renderFox(glyphSize, glyphColor)}
    </span>
  )
}

/**
 * Unified customizable User Profile Avatar.
 * Features dedicated human/user/profile icons (plus active Icon Family `user` & Monogram),
 * custom color overrides, and toggleable frames (including `bare` = Hidden / No Frame).
 */
export function UserAvatar({
  variant = 'icon_user',
  frame = 'circle',
  color,
  icons,
  size = 28,
  monogram = 'U',
  className,
}: {
  variant?: string
  frame?: AvatarFrameStyle | string
  color?: string
  icons?: UiCustomization['icons']
  size?: number
  monogram?: string
  className?: string
}) {
  if (variant === 'hidden') return null

  const frameSpec = resolveFrame(frame)
  const customTint = normalizeCustomColor(color)
  const effectiveTint = customTint || 'var(--accent)'
  const glyphSize = frameSpec.bare
    ? Math.max(16, Math.round(size * 0.76))
    : Math.max(13, Math.round(size * 0.56))
  const glyphColor = frameSpec.solidAccent
    ? 'var(--primary-fg)'
    : customTint ||
      (frame === 'soft_tint' || frame === 'ring' ? 'var(--accent)' : 'var(--muted)')
  const strokeWidth = icons?.stroke_width ?? 1.75
  const profileRenderer = USER_PROFILE_RENDERERS[variant]

  return (
    <span
      className={cn(
        'relative inline-flex shrink-0 items-center justify-center transition-colors',
        frameSpec.className,
        className,
      )}
      style={{
        width: size,
        height: size,
        ...frameSpec.styleForColor(effectiveTint),
      }}
    >
      {variant === 'monogram' ? (
        <span
          className="font-semibold leading-none tracking-tight select-none"
          style={{
            fontFamily: 'var(--font-display)',
            fontSize: Math.max(10, Math.round(size * 0.42)),
            color: frameSpec.solidAccent ? 'var(--primary-fg)' : customTint || 'var(--fg)',
          }}
        >
          {monogram.slice(0, 2).toUpperCase()}
        </span>
      ) : profileRenderer ? (
        profileRenderer(glyphSize, glyphColor, strokeWidth)
      ) : (
        <ZenkoIcon
          slot="user"
          icons={icons}
          size={glyphSize}
          useGlobalSize={false}
          style={{ color: glyphColor }}
        />
      )}
    </span>
  )
}
