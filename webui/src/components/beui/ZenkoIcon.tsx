import { createContext, forwardRef, useContext, type CSSProperties } from 'react'
import { motion, useReducedMotion, type TargetAndTransition } from 'motion/react'
import {
  ICON_PACK_META_MAP,
  resolveVectorGlyph,
  type IconHoverBehavior,
  type SvgPrimitive,
} from '@/lib/icon-packs'
import type { UiCustomization } from '@/lib/types'
import { cn } from '@/lib/utils'

import ActivityRaw from 'lucide-react/dist/esm/icons/activity.js'
import BellRaw from 'lucide-react/dist/esm/icons/bell.js'
import BookOpenRaw from 'lucide-react/dist/esm/icons/book-open.js'
import BotRaw from 'lucide-react/dist/esm/icons/bot.js'
import BracesRaw from 'lucide-react/dist/esm/icons/braces.js'
import BrainRaw from 'lucide-react/dist/esm/icons/brain.js'
import CheckRaw from 'lucide-react/dist/esm/icons/check.js'
import ClockRaw from 'lucide-react/dist/esm/icons/clock.js'
import CopyRaw from 'lucide-react/dist/esm/icons/copy.js'
import CpuRaw from 'lucide-react/dist/esm/icons/cpu.js'
import DownloadRaw from 'lucide-react/dist/esm/icons/download.js'
import FileCode2Raw from 'lucide-react/dist/esm/icons/file-code-2.js'
import FileTextRaw from 'lucide-react/dist/esm/icons/file-text.js'
import FolderSearchRaw from 'lucide-react/dist/esm/icons/folder-search.js'
import GitBranchRaw from 'lucide-react/dist/esm/icons/git-branch.js'
import GlobeRaw from 'lucide-react/dist/esm/icons/globe.js'
import KeyRoundRaw from 'lucide-react/dist/esm/icons/key-round.js'
import LayoutGridRaw from 'lucide-react/dist/esm/icons/layout-grid.js'
import MessageSquareRaw from 'lucide-react/dist/esm/icons/message-square.js'
import MoonRaw from 'lucide-react/dist/esm/icons/moon.js'
import PaletteRaw from 'lucide-react/dist/esm/icons/palette.js'
import PanelLeftCloseRaw from 'lucide-react/dist/esm/icons/panel-left-close.js'
import PanelLeftOpenRaw from 'lucide-react/dist/esm/icons/panel-left-open.js'
import PanelRightCloseRaw from 'lucide-react/dist/esm/icons/panel-right-close.js'
import PanelRightOpenRaw from 'lucide-react/dist/esm/icons/panel-right-open.js'
import PaperclipRaw from 'lucide-react/dist/esm/icons/paperclip.js'
import PenLineRaw from 'lucide-react/dist/esm/icons/pen-line.js'
import PlusRaw from 'lucide-react/dist/esm/icons/plus.js'
import SearchRaw from 'lucide-react/dist/esm/icons/search.js'
import SendRaw from 'lucide-react/dist/esm/icons/send.js'
import SettingsRaw from 'lucide-react/dist/esm/icons/settings.js'
import ShieldRaw from 'lucide-react/dist/esm/icons/shield.js'
import SlidersHorizontalRaw from 'lucide-react/dist/esm/icons/sliders-horizontal.js'
import SparklesRaw from 'lucide-react/dist/esm/icons/sparkles.js'
import SquareRaw from 'lucide-react/dist/esm/icons/square.js'
import SunRaw from 'lucide-react/dist/esm/icons/sun.js'
import TerminalRaw from 'lucide-react/dist/esm/icons/terminal.js'
import Trash2Raw from 'lucide-react/dist/esm/icons/trash-2.js'
import UploadRaw from 'lucide-react/dist/esm/icons/upload.js'
import UserRaw from 'lucide-react/dist/esm/icons/user.js'
import WrenchRaw from 'lucide-react/dist/esm/icons/wrench.js'
import XRaw from 'lucide-react/dist/esm/icons/x.js'
import ZapRaw from 'lucide-react/dist/esm/icons/zap.js'
import type { LucideIcon } from 'lucide-react'

const ZenkoIconsContext = createContext<UiCustomization['icons'] | undefined>(undefined)

export const ZenkoIconProvider = ZenkoIconsContext.Provider

export function useZenkoIcons(): UiCustomization['icons'] | undefined {
  return useContext(ZenkoIconsContext)
}

const LUCIDE_SLOT_FALLBACK: Readonly<Record<string, LucideIcon>> = {
  chat: MessageSquareRaw,
  grid: LayoutGridRaw,
  plus: PlusRaw,
  send: SendRaw,
  stop: SquareRaw,
  gateway: BotRaw,
  memory: BookOpenRaw,
  skills: SparklesRaw,
  clock: ClockRaw,
  activity: ActivityRaw,
  palette: PaletteRaw,
  settings: SettingsRaw,
  search: SearchRaw,
  shield: ShieldRaw,
  terminal: TerminalRaw,
  sparkle: SparklesRaw,
  bot: BotRaw,
  bell: BellRaw,
  moon: MoonRaw,
  sun: SunRaw,
  check: CheckRaw,
  x: XRaw,
  trash: Trash2Raw,
  copy: CopyRaw,
  edit: PenLineRaw,
  branch: GitBranchRaw,
  download: DownloadRaw,
  upload: UploadRaw,
  attach: PaperclipRaw,
  cpu: CpuRaw,
  zap: ZapRaw,
  panelLeftClose: PanelLeftCloseRaw,
  panelLeftOpen: PanelLeftOpenRaw,
  panelRightClose: PanelRightCloseRaw,
  panelRightOpen: PanelRightOpenRaw,
  globe: GlobeRaw,
  folderSearch: FolderSearchRaw,
  fileCode: FileCode2Raw,
  fileText: FileTextRaw,
  braces: BracesRaw,
  brain: BrainRaw,
  wrench: WrenchRaw,
  user: UserRaw,
  sliders: SlidersHorizontalRaw,
  keyRound: KeyRoundRaw,
}

export const CUSTOMIZABLE_ICON_OPTIONS = [
  'chat',
  'grid',
  'bot',
  'memory',
  'skills',
  'clock',
  'activity',
  'terminal',
  'sparkle',
  'shield',
  'zap',
  'cpu',
  'send',
  'plus',
] as const

export const ICON_GALLERY_SLOTS = [
  { slot: 'chat', label: 'Chat' },
  { slot: 'grid', label: 'Overview' },
  { slot: 'terminal', label: 'Terminal' },
  { slot: 'sparkle', label: 'Skills' },
  { slot: 'palette', label: 'Theme' },
  { slot: 'settings', label: 'Settings' },
  { slot: 'search', label: 'Search' },
  { slot: 'shield', label: 'Policy' },
  { slot: 'bell', label: 'Alerts' },
  { slot: 'send', label: 'Send' },
  { slot: 'trash', label: 'Delete' },
  { slot: 'zap', label: 'Run' },
] as const

/** Declarative per-slot Motion hover recipes for animated packs (PulsoIcon, Hugeicons Animated, ItsHover). */
const PULSO_HOVER_RECIPES: Readonly<Record<string, TargetAndTransition>> = {
  bell: { rotate: [0, -14, 12, -8, 4, 0], transition: { duration: 0.45 } },
  settings: { rotate: 75, transition: { type: 'spring', stiffness: 280, damping: 16 } },
  sun: { rotate: 45, scale: 1.08, transition: { type: 'spring', stiffness: 300, damping: 16 } },
  moon: { rotate: -18, scale: 1.08, transition: { type: 'spring', stiffness: 300, damping: 16 } },
  send: { x: 2.5, y: -2.5, scale: 1.06, transition: { type: 'spring', stiffness: 380, damping: 18 } },
  trash: { y: -1.5, rotate: [-4, 4, 0], transition: { duration: 0.3 } },
  sparkle: { scale: [1, 1.2, 1.06], rotate: [0, 14, 0], transition: { duration: 0.4 } },
  skills: { scale: [1, 1.2, 1.06], rotate: [0, 14, 0], transition: { duration: 0.4 } },
  zap: { scale: 1.15, rotate: -8, transition: { type: 'spring', stiffness: 420, damping: 15 } },
  search: { scale: 1.12, rotate: -10, transition: { type: 'spring', stiffness: 360, damping: 16 } },
  plus: { rotate: 90, transition: { type: 'spring', stiffness: 340, damping: 18 } },
}

const HOVER_DEFAULT_BY_BEHAVIOR: Readonly<Record<IconHoverBehavior, TargetAndTransition | undefined>> = {
  none: undefined,
  pulso: { scale: 1.12, y: -1, transition: { type: 'spring', stiffness: 400, damping: 17 } },
  huge: { scale: 1.14, rotate: 6, transition: { type: 'spring', stiffness: 320, damping: 14 } },
  itshover: { y: -2, scale: 1.1, transition: { type: 'spring', stiffness: 420, damping: 16 } },
}

function renderSvgPrimitive(prim: SvgPrimitive, key: number, strokeWidth: number, isFillPack: boolean) {
  const [tag, rawAttrs] = prim
  const attrs: Record<string, any> = { ...rawAttrs }

  if (!isFillPack) {
    if (!attrs.fill || attrs.fill === 'none') {
      attrs.fill = 'none'
    }
    if (!attrs.stroke || attrs.stroke === 'currentColor') {
      attrs.stroke = 'currentColor'
    }
    if (attrs.stroke !== 'none') {
      attrs.strokeWidth = strokeWidth
    }
  } else {
    if (!attrs.fill) attrs.fill = 'currentColor'
  }

  switch (tag) {
    case 'path':
      return <path key={key} {...attrs} />
    case 'circle':
      return <circle key={key} {...attrs} />
    case 'rect':
      return <rect key={key} {...attrs} />
    case 'line':
      return <line key={key} {...attrs} />
    case 'polyline':
      return <polyline key={key} {...attrs} />
    case 'polygon':
      return <polygon key={key} {...attrs} />
    case 'ellipse':
      return <ellipse key={key} {...attrs} />
  }
}

export interface ZenkoIconProps {
  slot: string
  icons?: UiCustomization['icons']
  className?: string
  size?: number | string
  strokeWidth?: number | string
  style?: CSSProperties
  role?: string
  'aria-label'?: string
  'aria-hidden'?: boolean | 'true' | 'false'
  animateAlways?: boolean
  /** True for direct dock/toolbar usage; false for shimmed Lucide icons so Tailwind size-* classes win cleanly. */
  useGlobalSize?: boolean
  fallbackLucide?: LucideIcon
}

export const ZenkoIcon = forwardRef<SVGSVGElement, ZenkoIconProps>(function ZenkoIcon(
  {
    slot,
    icons,
    className,
    size,
    strokeWidth: explicitStrokeWidth,
    style,
    role,
    'aria-label': ariaLabel,
    'aria-hidden': ariaHidden,
    animateAlways = false,
    useGlobalSize = true,
    fallbackLucide,
  },
  ref,
) {
  const ctxIcons = useZenkoIcons()
  const activeIcons = icons ?? ctxIcons
  const reduce = useReducedMotion() ?? false

  const resolved = activeIcons?.overrides?.[slot] || slot
  const pack = activeIcons?.pack || 'lucide'
  const meta = ICON_PACK_META_MAP[pack]

  const parsedExplicitStroke =
    typeof explicitStrokeWidth === 'number'
      ? explicitStrokeWidth
      : typeof explicitStrokeWidth === 'string' && Number.isFinite(Number(explicitStrokeWidth))
        ? Number(explicitStrokeWidth)
        : undefined
  const baseStroke = parsedExplicitStroke ?? activeIcons?.stroke_width ?? 1.75
  const strokeWidth = Number(
    Math.min(2.6, Math.max(1.0, baseStroke * (meta?.strokeScale ?? 1))).toFixed(2),
  )

  const px = size ?? (useGlobalSize ? (activeIcons?.size_px ?? 16) : 24)
  const isSpinning = Boolean(className && className.includes('animate-spin'))

  const hoverBehavior: IconHoverBehavior = meta?.hoverBehavior ?? 'none'
  const hoverMotion =
    reduce || isSpinning
      ? undefined
      : hoverBehavior === 'pulso'
        ? PULSO_HOVER_RECIPES[resolved] ?? HOVER_DEFAULT_BY_BEHAVIOR.pulso
        : HOVER_DEFAULT_BY_BEHAVIOR[hoverBehavior]

  // 1. Authentic vector glyph packs (PulsoIcon, Iconsax, Hugeicons Animated, ItsHover, Tabler, AiCanvas, Humbleicons, Majesticons, Dooicons)
  const glyph = resolveVectorGlyph(pack, resolved)
  if (glyph) {
    const isFillPack = meta?.fillMode === 'fill'
    const iconStyle = activeIcons?.style || 'outline'
    return (
      <motion.svg
        ref={ref}
        width={px}
        height={px}
        viewBox={glyph.vb}
        fill={isFillPack || iconStyle === 'solid' ? 'currentColor' : 'none'}
        stroke={isFillPack ? 'none' : 'currentColor'}
        strokeWidth={isFillPack ? undefined : strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
        data-icon-fill={isFillPack ? 'true' : undefined}
        whileHover={hoverMotion}
        animate={animateAlways && hoverMotion ? hoverMotion : undefined}
        style={style}
        role={role}
        aria-label={ariaLabel}
        aria-hidden={ariaHidden}
        className={cn('z-icon shrink-0', className)}
      >
        {glyph.p.map((prim, idx) => renderSvgPrimitive(prim, idx, strokeWidth, isFillPack))}
      </motion.svg>
    )
  }

  // 2. Pixel retro fallback
  if (pack === 'pixel') {
    return (
      <svg
        ref={ref}
        width={px}
        height={px}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="square"
        strokeLinejoin="miter"
        style={style}
        role={role}
        aria-label={ariaLabel}
        aria-hidden={ariaHidden}
        className={cn('z-icon shrink-0', className)}
      >
        {resolved === 'chat' && <path d="M3 4h18v12h-9l-5 4v-4H3V4z" />}
        {resolved === 'plus' && <path d="M12 4v16M4 12h16" />}
        {resolved === 'send' && <path d="M3 12l18-8-4 16-5-5-9-3z" />}
        {resolved === 'stop' && <rect x="5" y="5" width="14" height="14" />}
        {resolved === 'grid' && (
          <>
            <rect x="4" y="4" width="6" height="6" />
            <rect x="14" y="4" width="6" height="6" />
            <rect x="4" y="14" width="6" height="6" />
            <rect x="14" y="14" width="6" height="6" />
          </>
        )}
        {!['chat', 'plus', 'send', 'stop', 'grid'].includes(resolved) && (
          <rect x="5" y="5" width="14" height="14" />
        )}
      </svg>
    )
  }

  // 3. Default Lucide React icon
  const Comp = fallbackLucide || LUCIDE_SLOT_FALLBACK[resolved] || MessageSquareRaw
  return (
    <Comp
      ref={ref}
      size={px}
      strokeWidth={strokeWidth}
      style={style}
      role={role}
      aria-label={ariaLabel}
      aria-hidden={ariaHidden}
      className={cn('z-icon shrink-0', className)}
    />
  )
})
