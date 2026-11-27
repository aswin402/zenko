/**
 * Global Icon Family Shim (`lucide-react` alias target in `vite.config.ts`).
 *
 * Every component across the entire WebUI (Assistant-UI Elements, BeUI Motion, Chat,
 * Sidebar, Dock, Theme Studio, Settings, Memory, Jobs, Gateways) imports icons from
 * `lucide-react`. This shim dynamically renders each icon using the user's selected
 * global Icon Family from `ZenkoIconProvider` (Lucide, PulsoIcon, Iconsax, Hugeicons Animated,
 * ItsHover, Tabler, AiCanvas, Humbleicons, Majesticons, Dooicons).
 *
 * Imports individual ESM files from `lucide-react/dist/esm/icons/*.js` so Vite tree-shakes
 * cleanly and only bundles the exact icons used.
 */
import { forwardRef } from 'react'
import type { LucideIcon, LucideProps } from 'lucide-react'
import { ZenkoIcon, useZenkoIcons } from '@/components/beui/ZenkoIcon'
import { cn } from '@/lib/utils'

import ActivityRaw from 'lucide-react/dist/esm/icons/activity.js'
import ArrowDownToLineRaw from 'lucide-react/dist/esm/icons/arrow-down-to-line.js'
import ArrowLeftRaw from 'lucide-react/dist/esm/icons/arrow-left.js'
import ArrowRightRaw from 'lucide-react/dist/esm/icons/arrow-right.js'
import ArrowUpRaw from 'lucide-react/dist/esm/icons/arrow-up.js'
import ArrowUpRightRaw from 'lucide-react/dist/esm/icons/arrow-up-right.js'
import BanRaw from 'lucide-react/dist/esm/icons/ban.js'
import BellRaw from 'lucide-react/dist/esm/icons/bell.js'
import BookOpenRaw from 'lucide-react/dist/esm/icons/book-open.js'
import BookOpenTextRaw from 'lucide-react/dist/esm/icons/book-open-text.js'
import BotRaw from 'lucide-react/dist/esm/icons/bot.js'
import BracesRaw from 'lucide-react/dist/esm/icons/braces.js'
import BrainRaw from 'lucide-react/dist/esm/icons/brain.js'
import CalendarCheckRaw from 'lucide-react/dist/esm/icons/calendar-check.js'
import CheckRaw from 'lucide-react/dist/esm/icons/check.js'
import ChevronDownRaw from 'lucide-react/dist/esm/icons/chevron-down.js'
import ChevronLeftRaw from 'lucide-react/dist/esm/icons/chevron-left.js'
import ChevronRightRaw from 'lucide-react/dist/esm/icons/chevron-right.js'
import ChevronsUpDownRaw from 'lucide-react/dist/esm/icons/chevrons-up-down.js'
import CircleAlertRaw from 'lucide-react/dist/esm/icons/circle-alert.js'
import CircleCheckRaw from 'lucide-react/dist/esm/icons/circle-check.js'
import CircleRaw from 'lucide-react/dist/esm/icons/circle.js'
import CircleXRaw from 'lucide-react/dist/esm/icons/circle-x.js'
import ClockRaw from 'lucide-react/dist/esm/icons/clock.js'
import CopyRaw from 'lucide-react/dist/esm/icons/copy.js'
import CornerDownLeftRaw from 'lucide-react/dist/esm/icons/corner-down-left.js'
import CpuRaw from 'lucide-react/dist/esm/icons/cpu.js'
import DownloadRaw from 'lucide-react/dist/esm/icons/download.js'
import EarthRaw from 'lucide-react/dist/esm/icons/earth.js'
import EllipsisRaw from 'lucide-react/dist/esm/icons/ellipsis.js'
import ExternalLinkRaw from 'lucide-react/dist/esm/icons/external-link.js'
import FileCodeRaw from 'lucide-react/dist/esm/icons/file-code.js'
import FileCode2Raw from 'lucide-react/dist/esm/icons/file-code-2.js'
import FilePenRaw from 'lucide-react/dist/esm/icons/file-pen.js'
import FileRaw from 'lucide-react/dist/esm/icons/file.js'
import FileTextRaw from 'lucide-react/dist/esm/icons/file-text.js'
import FlameRaw from 'lucide-react/dist/esm/icons/flame.js'
import ScissorsRaw from 'lucide-react/dist/esm/icons/scissors.js'
import FolderRaw from 'lucide-react/dist/esm/icons/folder.js'
import FolderSearchRaw from 'lucide-react/dist/esm/icons/folder-search.js'
import GitBranchRaw from 'lucide-react/dist/esm/icons/git-branch.js'
import GlobeRaw from 'lucide-react/dist/esm/icons/globe.js'
import GripHorizontalRaw from 'lucide-react/dist/esm/icons/grip-horizontal.js'
import HistoryRaw from 'lucide-react/dist/esm/icons/history.js'
import ImageRaw from 'lucide-react/dist/esm/icons/image.js'
import InfoRaw from 'lucide-react/dist/esm/icons/info.js'
import KeyRoundRaw from 'lucide-react/dist/esm/icons/key-round.js'
import LayersRaw from 'lucide-react/dist/esm/icons/layers.js'
import LayoutGridRaw from 'lucide-react/dist/esm/icons/layout-grid.js'
import ListChecksRaw from 'lucide-react/dist/esm/icons/list-checks.js'
import LoaderCircleRaw from 'lucide-react/dist/esm/icons/loader-circle.js'
import MessageSquareRaw from 'lucide-react/dist/esm/icons/message-square.js'
import MinusRaw from 'lucide-react/dist/esm/icons/minus.js'
import MoonRaw from 'lucide-react/dist/esm/icons/moon.js'
import PaletteRaw from 'lucide-react/dist/esm/icons/palette.js'
import PanelLeftCloseRaw from 'lucide-react/dist/esm/icons/panel-left-close.js'
import PanelLeftOpenRaw from 'lucide-react/dist/esm/icons/panel-left-open.js'
import PanelRightCloseRaw from 'lucide-react/dist/esm/icons/panel-right-close.js'
import PanelRightOpenRaw from 'lucide-react/dist/esm/icons/panel-right-open.js'
import PaperclipRaw from 'lucide-react/dist/esm/icons/paperclip.js'
import PenLineRaw from 'lucide-react/dist/esm/icons/pen-line.js'
import PencilLineRaw from 'lucide-react/dist/esm/icons/pencil-line.js'
import PencilRaw from 'lucide-react/dist/esm/icons/pencil.js'
import PipetteRaw from 'lucide-react/dist/esm/icons/pipette.js'
import PlayRaw from 'lucide-react/dist/esm/icons/play.js'
import PlusRaw from 'lucide-react/dist/esm/icons/plus.js'
import RefreshCwRaw from 'lucide-react/dist/esm/icons/refresh-cw.js'
import RotateCcwRaw from 'lucide-react/dist/esm/icons/rotate-ccw.js'
import RotateCwRaw from 'lucide-react/dist/esm/icons/rotate-cw.js'
import SearchRaw from 'lucide-react/dist/esm/icons/search.js'
import SendRaw from 'lucide-react/dist/esm/icons/send.js'
import SettingsRaw from 'lucide-react/dist/esm/icons/settings.js'
import Share2Raw from 'lucide-react/dist/esm/icons/share-2.js'
import ShieldAlertRaw from 'lucide-react/dist/esm/icons/shield-alert.js'
import ShieldCheckRaw from 'lucide-react/dist/esm/icons/shield-check.js'
import ShieldRaw from 'lucide-react/dist/esm/icons/shield.js'
import SlidersVerticalRaw from 'lucide-react/dist/esm/icons/sliders-vertical.js'
import SparklesRaw from 'lucide-react/dist/esm/icons/sparkles.js'
import SquareRaw from 'lucide-react/dist/esm/icons/square.js'
import SquareTerminalRaw from 'lucide-react/dist/esm/icons/square-terminal.js'
import SunRaw from 'lucide-react/dist/esm/icons/sun.js'
import TerminalRaw from 'lucide-react/dist/esm/icons/terminal.js'
import ThumbsDownRaw from 'lucide-react/dist/esm/icons/thumbs-down.js'
import ThumbsUpRaw from 'lucide-react/dist/esm/icons/thumbs-up.js'
import Trash2Raw from 'lucide-react/dist/esm/icons/trash-2.js'
import TriangleAlertRaw from 'lucide-react/dist/esm/icons/triangle-alert.js'
import TypeRaw from 'lucide-react/dist/esm/icons/type.js'
import Undo2Raw from 'lucide-react/dist/esm/icons/undo-2.js'
import UploadRaw from 'lucide-react/dist/esm/icons/upload.js'
import UserRaw from 'lucide-react/dist/esm/icons/user.js'
import WrenchRaw from 'lucide-react/dist/esm/icons/wrench.js'
import XRaw from 'lucide-react/dist/esm/icons/x.js'
import ZapRaw from 'lucide-react/dist/esm/icons/zap.js'

export type { LucideIcon, LucideProps }

function createThemedIcon(slot: string, Fallback: LucideIcon): LucideIcon {
  const Themed = forwardRef<SVGSVGElement, LucideProps>(function ThemedIcon(
    { size, strokeWidth, className, style, role, 'aria-label': ariaLabel, 'aria-hidden': ariaHidden, ...rest },
    ref,
  ) {
    const activeIcons = useZenkoIcons()
    const pack = activeIcons?.pack || 'lucide'

    if (pack === 'lucide') {
      const effectiveStroke = strokeWidth ?? activeIcons?.stroke_width ?? 1.75
      return (
        <Fallback
          ref={ref}
          size={size}
          strokeWidth={effectiveStroke}
          style={style}
          role={role}
          aria-label={ariaLabel}
          aria-hidden={ariaHidden}
          className={cn('z-icon shrink-0', className)}
          {...rest}
        />
      )
    }

    return (
      <ZenkoIcon
        {...rest}
        ref={ref}
        slot={slot}
        icons={activeIcons}
        size={size}
        strokeWidth={strokeWidth}
        style={style}
        role={role}
        aria-label={ariaLabel}
        aria-hidden={ariaHidden}
        useGlobalSize={false}
        fallbackLucide={Fallback}
        className={className}
      />
    )
  })
  Themed.displayName = `ZenkoThemed(${slot})`
  return Themed as LucideIcon
}

export const Activity = createThemedIcon('activity', ActivityRaw)
export const AlertCircle = createThemedIcon('alertCircle', CircleAlertRaw)
export const AlertCircleIcon = AlertCircle
export const CircleAlert = AlertCircle
export const CircleAlertIcon = AlertCircle
export const AlertTriangle = createThemedIcon('alertTriangle', TriangleAlertRaw)
export const TriangleAlert = AlertTriangle
export const ArrowDownToLine = createThemedIcon('arrowDownToLine', ArrowDownToLineRaw)
export const ArrowLeft = createThemedIcon('arrowLeft', ArrowLeftRaw)
export const ArrowRight = createThemedIcon('arrowRight', ArrowRightRaw)
export const ArrowRightIcon = ArrowRight
export const ArrowUp = createThemedIcon('arrowUp', ArrowUpRaw)
export const ArrowUpRight = createThemedIcon('arrowUpRight', ArrowUpRightRaw)
export const Ban = createThemedIcon('ban', BanRaw)
export const Bell = createThemedIcon('bell', BellRaw)
export const BookOpen = createThemedIcon('memory', BookOpenRaw)
export const BookOpenText = createThemedIcon('memory', BookOpenTextRaw)
export const Bot = createThemedIcon('bot', BotRaw)
export const Braces = createThemedIcon('braces', BracesRaw)
export const BrainIcon = createThemedIcon('brain', BrainRaw)
export const CalendarCheck = createThemedIcon('calendar', CalendarCheckRaw)
export const Check = createThemedIcon('check', CheckRaw)
export const CheckIcon = Check
export const CheckCircle2 = createThemedIcon('checkCircle', CircleCheckRaw)
export const CircleCheck = CheckCircle2
export const ChevronDown = createThemedIcon('chevronDown', ChevronDownRaw)
export const ChevronDownIcon = ChevronDown
export const ChevronLeft = createThemedIcon('chevronLeft', ChevronLeftRaw)
export const ChevronRight = createThemedIcon('chevronRight', ChevronRightRaw)
export const ChevronRightIcon = ChevronRight
export const ChevronsUpDown = createThemedIcon('chevronDown', ChevronsUpDownRaw)
export const Circle = createThemedIcon('circle', CircleRaw)
export const CircleX = createThemedIcon('circleX', CircleXRaw)
export const Clock = createThemedIcon('clock', ClockRaw)
export const Copy = createThemedIcon('copy', CopyRaw)
export const CopyIcon = Copy
export const CornerDownLeft = createThemedIcon('cornerDownLeft', CornerDownLeftRaw)
export const Cpu = createThemedIcon('cpu', CpuRaw)
export const Download = createThemedIcon('download', DownloadRaw)
export const Edit3 = createThemedIcon('edit', PenLineRaw)
export const Pencil = createThemedIcon('edit', PencilRaw)
export const PencilLine = createThemedIcon('edit', PencilLineRaw)
export const EllipsisIcon = createThemedIcon('ellipsis', EllipsisRaw)
export const ExternalLink = createThemedIcon('externalLink', ExternalLinkRaw)
export const FileCode = createThemedIcon('fileCode', FileCodeRaw)
export const FileCode2 = createThemedIcon('fileCode', FileCode2Raw)
export const FilePen = createThemedIcon('edit', FilePenRaw)
export const FileIcon = createThemedIcon('file', FileRaw)
export const FileText = createThemedIcon('fileText', FileTextRaw)
export const Flame = createThemedIcon('zap', FlameRaw)
export const Scissors = createThemedIcon('edit', ScissorsRaw)
export const FolderIcon = createThemedIcon('folder', FolderRaw)
export const FolderSearch = createThemedIcon('folderSearch', FolderSearchRaw)
export const GitBranch = createThemedIcon('branch', GitBranchRaw)
export const Globe = createThemedIcon('globe', GlobeRaw)
export const Globe2 = createThemedIcon('globe', EarthRaw)
export const GripHorizontal = createThemedIcon('gripHorizontal', GripHorizontalRaw)
export const History = createThemedIcon('history', HistoryRaw)
export const ImageIcon = createThemedIcon('image', ImageRaw)
export const Info = createThemedIcon('info', InfoRaw)
export const KeyRound = createThemedIcon('keyRound', KeyRoundRaw)
export const KeyRoundIcon = KeyRound
export const Layers = createThemedIcon('grid', LayersRaw)
export const LayoutGrid = createThemedIcon('grid', LayoutGridRaw)
export const ListChecks = createThemedIcon('listChecks', ListChecksRaw)
export const LoaderCircle = createThemedIcon('loader', LoaderCircleRaw)
export const Loader2 = LoaderCircle
export const Loader2Icon = LoaderCircle
export const MessageSquare = createThemedIcon('chat', MessageSquareRaw)
export const Minus = createThemedIcon('minus', MinusRaw)
export const MinusIcon = Minus
export const Moon = createThemedIcon('moon', MoonRaw)
export const Palette = createThemedIcon('palette', PaletteRaw)
export const PanelLeftClose = createThemedIcon('panelLeftClose', PanelLeftCloseRaw)
export const PanelLeftOpen = createThemedIcon('panelLeftOpen', PanelLeftOpenRaw)
export const PanelRightClose = createThemedIcon('panelRightClose', PanelRightCloseRaw)
export const PanelRightOpen = createThemedIcon('panelRightOpen', PanelRightOpenRaw)
export const Paperclip = createThemedIcon('attach', PaperclipRaw)
export const Pipette = createThemedIcon('pipette', PipetteRaw)
export const PlayIcon = createThemedIcon('play', PlayRaw)
export const Plus = createThemedIcon('plus', PlusRaw)
export const RefreshCw = createThemedIcon('refreshCw', RefreshCwRaw)
export const RefreshCwIcon = RefreshCw
export const RotateCwIcon = createThemedIcon('refreshCw', RotateCwRaw)
export const RotateCcw = createThemedIcon('rotateCcw', RotateCcwRaw)
export const Search = createThemedIcon('search', SearchRaw)
export const SearchIcon = Search
export const Send = createThemedIcon('send', SendRaw)
export const Settings = createThemedIcon('settings', SettingsRaw)
export const Share2 = createThemedIcon('externalLink', Share2Raw)
export const Shield = createThemedIcon('shield', ShieldRaw)
export const ShieldAlert = createThemedIcon('shieldAlert', ShieldAlertRaw)
export const ShieldCheck = createThemedIcon('shieldCheck', ShieldCheckRaw)
export const Sliders = createThemedIcon('sliders', SlidersVerticalRaw)
export const Sparkles = createThemedIcon('sparkle', SparklesRaw)
export const Square = createThemedIcon('stop', SquareRaw)
export const SquareIcon = Square
export const SquareTerminal = createThemedIcon('terminal', SquareTerminalRaw)
export const Terminal = createThemedIcon('terminal', TerminalRaw)
export const Sun = createThemedIcon('sun', SunRaw)
export const ThumbsDown = createThemedIcon('thumbsDown', ThumbsDownRaw)
export const ThumbsDownIcon = ThumbsDown
export const ThumbsUp = createThemedIcon('thumbsUp', ThumbsUpRaw)
export const ThumbsUpIcon = ThumbsUp
export const Trash2 = createThemedIcon('trash', Trash2Raw)
export const Type = createThemedIcon('type', TypeRaw)
export const Undo2 = createThemedIcon('rotateCcw', Undo2Raw)
export const Upload = createThemedIcon('upload', UploadRaw)
export const User = createThemedIcon('user', UserRaw)
export const Wrench = createThemedIcon('wrench', WrenchRaw)
export const X = createThemedIcon('x', XRaw)
export const XIcon = X
export const Zap = createThemedIcon('zap', ZapRaw)
