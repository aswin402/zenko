import { useState, type ReactNode } from 'react'
import { Activity, BrainIcon, ChevronDown, Clock, Layers, RotateCcw, Settings } from 'lucide-react'
import { ICON_PACK_META_MAP } from '@/lib/icon-packs'
import type {
  ActiveView,
  ModelInfo,
  ProviderDescriptor,
  Session,
  UiCustomization,
} from '@/lib/types'
import { cn } from '@/lib/utils'
import { StreamingResponse } from '@/components/agents/streaming-response'
import { Stretchtext } from '@/components/aimran/stretchtext'
import { FloatingDock } from '@/components/beui/FloatingDock'
import { AgentAvatar, UserAvatar } from '@/components/beui/ZenkoAvatars'
import { ZenkoFoxLogo } from '@/components/beui/ZenkoFoxLogo'
import { ICON_GALLERY_SLOTS, ZenkoIcon } from '@/components/beui/ZenkoIcon'
import {
  Alert as EaseAlert,
  AlertDescription as EaseAlertDescription,
  AlertTitle as EaseAlertTitle,
} from '@/components/easeui/alert'
import {
  CommandEmpty as EaseCommandEmpty,
  CommandGroup as EaseCommandGroup,
  CommandInput as EaseCommandInput,
  CommandItem as EaseCommandItem,
  CommandList as EaseCommandList,
  CommandPalette as EaseCommandPalette,
} from '@/components/easeui/command-palette'
import { MessageBubble as EaseMessageBubble } from '@/components/easeui/message-bubble'
import { StreamingResponse as EaseStreamingResponse } from '@/components/easeui/streaming-response'
import { Faq as RareFaq } from '@/components/rareui/faq'
import { StreamingResponse as RareStreamingResponse } from '@/components/rareui/streaming-response'
import { WebSearch as RareWebSearch } from '@/components/rareui/web-search'
import { ExpandableTabs } from '@/components/skiperui/expandable-tabs'
import {
  LineHoverLink,
  type LineHoverVariant,
} from '@/components/vengeanceui/line-hover-link'
import { ModelPicker } from '@/components/chat/ModelPicker'
import { PromptComposer } from '@/components/chat/PromptComposer'
import { Sidebar } from '@/components/chat/Sidebar'
import { MODEL_BADGE_CLASS, USER_BUBBLE_CLASS } from '@/components/chat/ChatStream'
import {
  ActivityDisplay,
  ApprovalElement,
  AssistantMessageFooter,
  CitationsDisplay,
  LoadingStatusBar,
  MarkdownCodeBlock,
  SkillAndMemoryBadges,
  SmartToolElement,
  type NormalizedToolStep,
} from '@/components/chat/ChatElements'
import {
  Alert,
  AlertClose,
  AlertContent,
  AlertDescription,
  AlertIcon,
  AlertTitle,
} from '@/components/motion/alert'
import { BouncyAccordion } from '@/components/motion/bouncy-accordion'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
  CenterMorphModalTrigger,
} from '@/components/motion/center-morph-modal'
import type { LoaderVariant } from '@/components/motion/loader'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/motion/popover'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import { ThemeToggle } from '@/components/motion/theme-toggle'
import { DataTable } from '@/components/assistant-ui/elements/data-table'
import { DayDivider } from '@/components/assistant-ui/elements/day-separator'
import { ErrorState } from '@/components/assistant-ui/elements/error-state'
import { FileTree } from '@/components/assistant-ui/elements/file-tree'
import { StoppedRun } from '@/components/assistant-ui/elements/stopped-run'
import { StreamingText } from '@/components/assistant-ui/elements/streaming-text'
import { WebSearch } from '@/components/assistant-ui/elements/web-search'
import { ZenkoTooltip, ZenkoTooltipBadge } from '@/components/ui/Tooltip'
import { DashboardMiniPreview } from '@/components/views/DashboardView'

export interface StudioPreviewContext {
  ui: UiCustomization
  providers: ProviderDescriptor[]
  configuredProviders: Record<string, boolean>
  models: ModelInfo[]
  activeProvider: string
  activeModel: string
  activeMode: 'dark' | 'light'
  toggleMode: () => void
  previewPrompt: string
  setPreviewPrompt: (v: string) => void
  previewDockView: ActiveView
  setPreviewDockView: (v: ActiveView) => void
  previewTab: string
  setPreviewTab: (v: string) => void
  previewAlertOpen: boolean
  setPreviewAlertOpen: (v: boolean) => void
  onTestToast?: (msg: string, kind?: 'info' | 'success' | 'error') => void
}

const PREVIEW_SESSIONS: Session[] = [
  {
    id: 'preview-1',
    title: 'Refactor auth middleware',
    channel: 'webui',
    permission_mode: 'balanced',
    status: 'idle',
    total_tokens: 4200,
    total_cost_usd: 0.002,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
  },
  {
    id: 'preview-2',
    title: 'Telegram bot setup',
    channel: 'telegram',
    permission_mode: 'balanced',
    status: 'idle',
    total_tokens: 1800,
    total_cost_usd: 0,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
  },
  {
    id: 'preview-3',
    title: 'Inspect workspace memory',
    channel: 'webui',
    permission_mode: 'balanced',
    status: 'idle',
    total_tokens: 900,
    total_cost_usd: 0,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
  },
]

const PREVIEW_CITATIONS = [
  { id: 'c1', title: 'Zenko Native Runtime Architecture', domain: 'docs.zenko.local', url: 'https://rust-lang.org' },
  { id: 'c2', title: 'SQLite WAL Memory Tuning Guide', domain: 'sqlite.org', url: 'https://sqlite.org/wal.html' },
]

const PREVIEW_SHELL_TOOL: NormalizedToolStep = {
  id: 'pt-shell',
  name: 'shell_exec',
  args_preview: '{"command":"cargo test --workspace --offline","cwd":"zenko"}',
  status: 'ok',
  duration_ms: 412,
  content: 'running 101 tests\ntest result: ok. 101 passed; 0 failed',
}

const PREVIEW_READ_TOOL: NormalizedToolStep = {
  id: 'pt-read',
  name: 'read_file',
  args_preview: '{"path":"crates/zenko-core/src/studio_schema.rs"}',
  status: 'ok',
  duration_ms: 8,
  content: '//! Declarative Appearance & Component Studio schema served dynamically to the WebUI.',
}

const PREVIEW_DIFF_TOOL: NormalizedToolStep = {
  id: 'pt-diff',
  name: 'edit_file',
  args_preview: '{"path":"crates/zenko-core/src/ui.rs"}',
  status: 'ok',
  duration_ms: 14,
  diff: '+++ b/crates/zenko-core/src/ui.rs\n@@ -281,4 +281,6 @@\n pub struct SurfaceCustomization {\n+    pub element_chrome: String,\n+    pub table_style: String,\n }',
  content: 'Updated ui.rs',
}

const PREVIEW_TABLE_COLUMNS = [
  { key: 'crate', label: 'Crate', priority: 'primary' as const },
  { key: 'rss', label: 'RSS MB', format: { kind: 'number' as const, decimals: 1 } },
  { key: 'status', label: 'Status', format: { kind: 'badge' as const, tones: { Healthy: 'success' as const } } },
]

const PREVIEW_TABLE_ROWS = [
  { crate: 'zenko-runtime', rss: 18.4, status: 'Healthy' },
  { crate: 'zenko-memory', rss: 6.2, status: 'Healthy' },
  { crate: 'zenko-webui', rss: 3.8, status: 'Healthy' },
]

const PREVIEW_APPROVAL = {
  id: 'preview-apr',
  session_id: 'preview',
  call_id: 'call-1',
  tool_name: 'shell_exec',
  args_preview: '{"command":"cargo test --workspace --offline"}',
  risk: 'moderate' as const,
  status: 'pending',
  created_at: new Date().toISOString(),
}

function renderUserBubblePreview(
  style: UiCustomization['components']['chat_page']['user_bubble'],
  text: string,
  extraClass?: string,
): ReactNode {
  if (style === 'easeui_bubble') {
    return (
      <EaseMessageBubble align="end" tone="neutral" className={extraClass}>
        {text}
      </EaseMessageBubble>
    )
  }
  return (
    <div className={cn(extraClass, USER_BUBBLE_CLASS[style] || USER_BUBBLE_CLASS.soft_card)}>
      {text}
    </div>
  )
}

const TOAST_POSITION_CELLS: ReadonlyArray<{
  value: NonNullable<UiCustomization['surface']['toast_position']>
  label: string
}> = [
  { value: 'top_left', label: 'Top Left' },
  { value: 'top_center', label: 'Top Center' },
  { value: 'top_right', label: 'Top Right' },
  { value: 'bottom_left', label: 'Bottom Left' },
  { value: 'bottom_center', label: 'Bottom Center' },
  { value: 'bottom_right', label: 'Bottom Right' },
]

const TOAST_STYLE_PREVIEW_CLASS: Record<
  NonNullable<UiCustomization['surface']['toast_style']>,
  string
> = {
  default:
    'rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)] px-3.5 py-2.5 shadow-lg',
  easeui_toast:
    'rounded-xl border border-[var(--border)] bg-[var(--surface)] px-3.5 py-2.5 shadow-[0_12px_30px_rgba(0,0,0,0.28)]',
  minimal_pill:
    'rounded-full border border-[var(--border)] bg-[var(--surface)] px-4 py-1.5 shadow-md',
  accent_bar:
    'rounded-[var(--radius)] border border-[var(--border)] border-l-[3px] border-l-[var(--accent)] bg-[var(--surface)] px-3.5 py-2.5 shadow-lg',
  glass_toast:
    'rounded-xl border border-[var(--border)]/80 bg-[color-mix(in_srgb,var(--surface)_74%,transparent)] px-3.5 py-2.5 shadow-xl backdrop-blur-md',
}

function ToastPreview({ ctx }: { ctx: StudioPreviewContext }) {
  const { ui, onTestToast } = ctx
  const toastStyle = ui.surface.toast_style || 'default'
  const toastPosition = ui.surface.toast_position || 'top_center'

  return (
    <div className="flex w-full max-w-[600px] flex-col gap-4">
      <div className="flex flex-wrap items-center justify-center gap-2">
        <button
          type="button"
          onClick={() =>
            onTestToast?.('Theme synchronized across Zenko WebUI.', 'info')
          }
          className="z-btn z-btn-secondary z-btn-sm"
        >
          Test Info Toast
        </button>
        <button
          type="button"
          onClick={() =>
            onTestToast?.('Saved to ~/.zenko/config.toml cleanly.', 'success')
          }
          className="z-btn z-btn-primary z-btn-sm"
        >
          Test Success Toast
        </button>
        <button
          type="button"
          onClick={() =>
            onTestToast?.('Upstream provider timed out — retrying.', 'error')
          }
          className="z-btn z-btn-danger z-btn-sm"
        >
          Test Error Toast
        </button>
      </div>

      {/* Static live-style toast sample */}
      <div className="flex justify-center">
        <div
          className={cn(
            'flex max-w-[380px] items-center gap-2.5 text-[12.5px] text-[var(--fg)] transition-all',
            TOAST_STYLE_PREVIEW_CLASS[toastStyle] || TOAST_STYLE_PREVIEW_CLASS.default,
          )}
        >
          <span
            className="h-2 w-2 shrink-0 rounded-full"
            style={{ backgroundColor: 'var(--accent)' }}
            aria-hidden="true"
          />
          <span className="truncate font-medium">
            Theme preferences synced · {toastStyle}
          </span>
        </div>
      </div>

      {/* 6-slot viewport anchor diagram */}
      <div className="rounded-xl border border-[var(--border)]/80 bg-[var(--bg)]/80 p-3">
        <div className="mb-2 flex items-center justify-between font-mono text-[10.5px] text-[var(--muted)]">
          <span>Viewport Anchor</span>
          <span style={{ color: 'var(--accent)' }}>{toastPosition}</span>
        </div>
        <div className="grid grid-cols-3 gap-2">
          {TOAST_POSITION_CELLS.map((cell) => {
            const active = toastPosition === cell.value
            return (
              <div
                key={cell.value}
                className={cn(
                  'flex h-10 items-center justify-center rounded-lg border font-mono text-[10.5px] transition-colors',
                  active
                    ? 'border-[var(--accent)] bg-[color-mix(in_srgb,var(--accent)_14%,transparent)] font-semibold text-[var(--accent)]'
                    : 'border-[var(--border)]/60 bg-[var(--surface)]/50 text-[var(--muted)]',
                )}
              >
                {cell.label}
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}

function IconFamiliesPreview({ ui }: { ui: UiCustomization }) {
  const packMeta = ICON_PACK_META_MAP[ui.icons.pack] || ICON_PACK_META_MAP.lucide
  const [pulseAll, setPulseAll] = useState(false)

  return (
    <div className="flex w-full max-w-[660px] flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="text-[13.5px] font-semibold text-[var(--fg)]">{packMeta.label}</span>
          {packMeta.animated && (
            <span className="rounded-full bg-[color-mix(in_srgb,var(--accent)_14%,transparent)] px-2 py-0.5 font-mono text-[10px] font-medium text-[var(--accent)]">
              Hover Animated
            </span>
          )}
          <span className="font-mono text-[11px] text-[var(--muted)]">
            {ui.icons.style} · {ui.icons.stroke_width}px · {ui.icons.size_px}px
          </span>
        </div>
        <button
          type="button"
          onClick={() => {
            setPulseAll(true)
            window.setTimeout(() => setPulseAll(false), 500)
          }}
          className="inline-flex cursor-pointer items-center gap-1 rounded-full border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1 font-mono text-[10.5px] text-[var(--muted)] transition-colors hover:border-[var(--accent)]/50 hover:text-[var(--fg)]"
        >
          <RotateCcw size={11} />
          <span>{packMeta.animated ? 'Play Hover Motion' : packMeta.source}</span>
        </button>
      </div>

      <div className="grid grid-cols-3 gap-2.5 sm:grid-cols-4 md:grid-cols-6">
        {ICON_GALLERY_SLOTS.map(({ slot, label }) => (
          <div
            key={`${ui.icons.pack}-${slot}`}
            className="group flex flex-col items-center justify-center gap-2 rounded-xl border border-[var(--border)]/75 bg-[var(--surface)]/75 px-2 py-3.5 transition-colors hover:border-[var(--accent)]/45 hover:bg-[var(--card)]"
          >
            <ZenkoIcon
              slot={slot}
              icons={ui.icons}
              size={Math.max(18, ui.icons.size_px + 2)}
              animateAlways={pulseAll}
              className="text-[var(--fg)] transition-colors group-hover:text-[var(--accent)]"
            />
            <span className="font-mono text-[10px] text-[var(--muted)]">{label}</span>
          </div>
        ))}
      </div>
    </div>
  )
}

function StreamingPreview({ ui }: { ui: UiCustomization }) {
  const [replayCount, setReplayCount] = useState(0)
  const style = ui.components.chat_page.streaming_style
  const sample = 'Daemon is healthy at 28.4 MB VmRSS with SQLite WAL mode and zero warnings.'

  return (
    <div className="flex w-full max-w-[600px] flex-col gap-3">
      <div className="flex items-center justify-between font-mono text-[11px] text-[var(--muted)]">
        <span>Active stream mode: {style}</span>
        <button
          type="button"
          onClick={() => setReplayCount((c) => c + 1)}
          className="inline-flex cursor-pointer items-center gap-1 rounded-full border border-[var(--border)] bg-[var(--surface)] px-2.5 py-0.5 text-[10.5px] hover:border-[var(--accent)]/50 hover:text-[var(--fg)]"
        >
          <RotateCcw size={10.5} /> Replay
        </button>
      </div>
      <div key={replayCount} className="rounded-xl border border-[var(--border)]/70 bg-[var(--surface)]/55 p-4">
        {style === 'aui_streaming_text' ? (
          <StreamingText
            segments={[
              { text: 'Daemon is healthy at ' },
              { text: '28.4 MB VmRSS', mono: true },
              { text: ' with SQLite WAL mode and zero warnings.' },
            ]}
            count={16}
            streaming={true}
          />
        ) : style === 'easeui_stream' ? (
          <EaseStreamingResponse status="complete" copyText={sample} onRetry={() => setReplayCount((c) => c + 1)} showFeedback>
            <p className="text-[13.5px] leading-relaxed text-[var(--fg)]/92">{sample}</p>
          </EaseStreamingResponse>
        ) : style === 'rareui_stream' ? (
          <RareStreamingResponse
            text={sample}
            status="streaming"
            ink="var(--accent)"
            actions={true}
            onRetry={() => setReplayCount((c) => c + 1)}
          />
        ) : style === 'beui_stream' ? (
          <StreamingResponse status="complete" copyText={sample} showActions={false} announce={false}>
            <p className="text-[13.5px] leading-relaxed text-[var(--fg)]/92">{sample}</p>
          </StreamingResponse>
        ) : (
          <p className="text-[13.5px] leading-relaxed text-[var(--fg)]/92">{sample}</p>
        )}
      </div>
    </div>
  )
}

function ElementStatesPreview({ ui }: { ui: UiCustomization }) {
  const [retrying, setRetrying] = useState(false)
  const webSearchStyle = ui.components.chat_page.web_search_style || 'aui_web_search'
  return (
    <div className="grid w-full max-w-[680px] gap-3 md:grid-cols-2">
      <div className="flex flex-col gap-3">
        <ErrorState
          title="Provider request timed out"
          detail="Upstream stream closed after 30s — click retry to resume."
          retrying={retrying}
          onRetry={() => {
            setRetrying(true)
            window.setTimeout(() => setRetrying(false), 1100)
          }}
        />
        <StoppedRun
          words={['Analyzing', 'workspace', 'crates', 'and', 'compiling', 'the', 'release...']}
          reason="Stopped by user"
          onContinue={() => {}}
          onDiscard={() => {}}
        />
      </div>
      <div className="flex flex-col gap-3">
        <FileTree
          title="crates/zenko-core · 3 files"
          visibleCount={4}
          totalAdditions={48}
          totalDeletions={6}
          nodes={[
            { path: 'src', name: 'src', depth: 0, kind: 'folder' },
            { path: 'src/ui.rs', name: 'ui.rs', depth: 1, kind: 'file', additions: 32, deletions: 4 },
            { path: 'src/studio_schema.rs', name: 'studio_schema.rs', depth: 1, kind: 'file', additions: 16, deletions: 2 },
          ]}
        />
        {webSearchStyle === 'rareui_web_search' ? (
          <RareWebSearch
            query="zenko rust agent architecture"
            status="done"
            defaultOpen={true}
            sources={[
              {
                id: 's1',
                title: 'Zenko Native Rust Runtime',
                url: 'https://rust-lang.org',
                snippet: 'Single-binary Tokio + SQLite WAL agent orchestrator under 30 MB VmRSS.',
              },
              {
                id: 's2',
                title: 'SQLite WAL Concurrency Guide',
                url: 'https://sqlite.org/wal.html',
                snippet: 'Concurrent readers and non-blocking writes for local-first AI state.',
              },
            ]}
          />
        ) : (
          <WebSearch
            query="zenko rust agent architecture"
            searching={false}
            visibleResults={2}
            cycle={1}
            results={[
              { title: 'Zenko Native Rust Runtime', domain: 'rust-lang.org', url: 'https://rust-lang.org' },
              { title: 'SQLite WAL Concurrency Guide', domain: 'sqlite.org', url: 'https://sqlite.org/wal.html' },
            ]}
          />
        )}
      </div>
    </div>
  )
}

const ACTION_PILL_PREVIEW_CLASS: Record<
  UiCustomization['components']['composer']['action_pills'],
  string
> = {
  soft_tint: 'border border-transparent bg-[var(--card)] hover:border-[var(--border)]',
  outline: 'border border-[var(--border)] bg-transparent hover:bg-[var(--card)]',
  ghost: 'border border-transparent bg-transparent hover:bg-[var(--card)]',
  solid_chip: 'border border-[var(--border)] bg-[var(--bg)] shadow-xs',
}

function OverlaysPreview({ ctx }: { ctx: StudioPreviewContext }) {
  const { ui } = ctx
  const [paletteOpen, setPaletteOpen] = useState(false)
  const paletteStyle = ui.surface.command_palette_style || 'default'

  return (
    <div className="flex w-full max-w-[600px] flex-col gap-4">
      <div className="flex flex-wrap items-center justify-center gap-2.5">
        <Popover
          side="bottom"
          align="center"
          gooStrength={ui.surface.popover_style === 'gooey' ? 7 : 0}
        >
          <PopoverTrigger>
            <button type="button" className="z-btn z-btn-secondary z-btn-sm">
              Popover ({ui.surface.popover_style || 'gooey'})
            </button>
          </PopoverTrigger>
          <PopoverContent className="w-56 rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)] p-3 text-[12px] shadow-xl">
            <div className="font-medium text-[var(--fg)]">BeUI Popover</div>
            <p className="mt-1 text-[11.5px] text-[var(--muted)]">
              Mode: <span className="font-mono text-[var(--accent)]">{ui.surface.popover_style || 'gooey'}</span>
            </p>
          </PopoverContent>
        </Popover>

        <CenterMorphModal>
          <CenterMorphModalTrigger>
            <button type="button" className="z-btn z-btn-primary z-btn-sm">
              Modal ({ui.surface.modal_style || 'center_morph'})
            </button>
          </CenterMorphModalTrigger>
          <CenterMorphModalContent ariaLabel="Center Morph Preview" className="p-6">
            <h3 className="text-[16px] font-semibold text-[var(--fg)]">Center Morph Modal</h3>
            <p className="mt-2 text-[13px] text-[var(--muted)]">
              Unfolds smoothly outward from the center using Zenko&apos;s active global theme.
            </p>
            <div className="mt-5 flex justify-end">
              <CenterMorphModalClose>
                <button type="button" className="z-btn z-btn-primary z-btn-sm">
                  Done
                </button>
              </CenterMorphModalClose>
            </div>
          </CenterMorphModalContent>
        </CenterMorphModal>

        <button
          type="button"
          onClick={() => setPaletteOpen(true)}
          className="z-btn z-btn-secondary z-btn-sm"
        >
          ⌘K Palette ({paletteStyle === 'easeui_palette' ? 'EaseUI' : 'Morph'})
        </button>
      </div>

      {paletteStyle === 'easeui_palette' ? (
        <EaseCommandPalette
          open={paletteOpen}
          onOpenChange={setPaletteOpen}
          label="Studio Preview Command Palette"
        >
          <EaseCommandInput placeholder="Search Zenko views..." />
          <EaseCommandList>
            <EaseCommandEmpty>No matching view.</EaseCommandEmpty>
            <EaseCommandGroup heading="Navigation">
              <EaseCommandItem value="Chat Workbench" onSelect={() => {}}>
                Chat Workbench
              </EaseCommandItem>
              <EaseCommandItem value="Dashboard Telemetry" onSelect={() => {}}>
                Dashboard Telemetry
              </EaseCommandItem>
              <EaseCommandItem value="Appearance Studio" onSelect={() => {}}>
                Appearance Studio
              </EaseCommandItem>
            </EaseCommandGroup>
          </EaseCommandList>
        </EaseCommandPalette>
      ) : (
        <CenterMorphModal open={paletteOpen} onOpenChange={setPaletteOpen}>
          <CenterMorphModalContent
            ariaLabel="Zenko Center Morph Command Palette Preview"
            showCloseButton={false}
            className="max-w-[460px] rounded-[var(--radius-lg)] bg-[var(--surface)] p-4"
          >
            <div className="mb-2 border-b border-[var(--border)] pb-2 text-[13px] font-medium text-[var(--fg)]">
              Zenko Center Morph Command Palette
            </div>
            <div className="flex flex-col gap-1 text-[12.5px] text-[var(--muted)]">
              {['Chat Workbench', 'Dashboard Telemetry', 'Appearance Studio'].map((item) => (
                <button
                  key={item}
                  type="button"
                  onClick={() => setPaletteOpen(false)}
                  className="flex cursor-pointer items-center justify-between rounded-[var(--radius-sm)] px-2.5 py-2 text-left text-[var(--fg)] hover:bg-[var(--card)]"
                >
                  <span>{item}</span>
                  <span className="font-mono text-[10.5px] text-[var(--muted)]">jump</span>
                </button>
              ))}
            </div>
          </CenterMorphModalContent>
        </CenterMorphModal>
      )}

      {ui.surface.alert_style === 'easeui_alert' ? (
        <EaseAlert variant="success">
          <div>
            <EaseAlertTitle>EaseUI Accent Stripe Alert</EaseAlertTitle>
            <EaseAlertDescription>
              Neutral body contrast paired with a crisp theme-aware left stripe.
            </EaseAlertDescription>
          </div>
        </EaseAlert>
      ) : ui.surface.alert_style === 'compact' ? (
        <div
          className="rounded-[var(--radius)] border bg-[var(--surface)] px-3.5 py-2 text-[12.5px] text-[var(--fg)]"
          style={{ borderColor: 'color-mix(in srgb, var(--accent) 45%, var(--border))' }}
        >
          Compact Strip Alert · inherits active 60/30/10 theme tokens.
        </div>
      ) : (
        <>
          <Alert
            variant="info"
            open={ctx.previewAlertOpen}
            onOpenChange={ctx.setPreviewAlertOpen}
          >
            <AlertIcon />
            <AlertContent>
              <AlertTitle>Theme synchronized</AlertTitle>
              <AlertDescription>
                Alerts, modals, and popovers strictly inherit your active 60/30/10 palette.
              </AlertDescription>
            </AlertContent>
            <AlertClose />
          </Alert>
          {!ctx.previewAlertOpen && (
            <button
              type="button"
              onClick={() => ctx.setPreviewAlertOpen(true)}
              className="w-fit cursor-pointer font-mono text-[11px] text-[var(--accent)] hover:underline"
            >
              Show preview alert again
            </button>
          )}
        </>
      )}
    </div>
  )
}

/**
 * Declarative focused preview dispatcher — each inside component page gets its own clean,
 * focused live preview with zero clutter.
 */
export function renderStudioPreview(previewKey: string, ctx: StudioPreviewContext): ReactNode {
  const { ui, activeModel, activeMode, toggleMode } = ctx
  const chatSlot = ui.components.chat_page
  const loaderVariant = (chatSlot.loader_variant || 'dot-matrix') as LoaderVariant
  const themeTransition = ui.motion.theme_transition || 'circle-blur'
  const tabsVariant = ui.surface.tabs_variant || 'pill'

  const MAP: Record<string, () => ReactNode> = {
    icon_families: () => <IconFamiliesPreview ui={ui} />,

    chat_background: () => (
      <div
        data-chat-bg={chatSlot.background}
        className="zenko-chat-bg flex h-[220px] w-full max-w-[640px] flex-col items-center justify-center gap-2 rounded-xl border border-[var(--border)] bg-[var(--bg)] p-6 text-center"
      >
        <AgentAvatar
          variant={ui.motion.orb_style === 'hidden' ? 'liquid_halo' : ui.motion.orb_style}
          frame={chatSlot.agent_avatar_frame || 'squircle'}
          color={chatSlot.agent_avatar_color}
          icons={ui.icons}
          size={40}
        />
        <div className="text-[14px] font-semibold text-[var(--fg)]">Chat Canvas Backdrop</div>
        <div className="font-mono text-[11px] text-[var(--muted)]">
          pattern: {chatSlot.background} · inherits global 60/30/10 palette
        </div>
      </div>
    ),

    chat_layout: () => {
      const badgeStyle = chatSlot.model_badge_style || 'bordered_chip'
      return (
        <div
          data-chat-bg={chatSlot.background}
          className={cn(
            'zenko-chat-bg flex w-full max-w-[640px] flex-col gap-3.5 rounded-xl border border-[var(--border)] bg-[var(--bg)] p-5',
            ui.layout.chat_layout === 'terminal_stream' && 'font-mono',
          )}
        >
          {chatSlot.show_day_separators !== false && <DayDivider label="Today" />}
          <div className="flex items-start justify-end gap-2.5">
            {renderUserBubblePreview(
              chatSlot.user_bubble,
              'Audit runtime memory and check tests',
              'text-[13px]',
            )}
            {chatSlot.show_avatars && chatSlot.user_avatar !== 'hidden' && (
              <UserAvatar
                variant={chatSlot.user_avatar || 'icon_user'}
                frame={chatSlot.user_avatar_frame || 'circle'}
                color={chatSlot.user_avatar_color}
                icons={ui.icons}
                size={26}
                className="mt-0.5"
              />
            )}
          </div>
          <div className="flex items-start gap-3">
            {chatSlot.show_avatars && ui.motion.orb_style !== 'hidden' && (
              <AgentAvatar
                variant={ui.motion.orb_style}
                frame={chatSlot.agent_avatar_frame || 'squircle'}
                color={chatSlot.agent_avatar_color}
                icons={ui.icons}
                size={26}
                className="mt-0.5 shrink-0"
              />
            )}
            <div
              className={cn(
                'min-w-0 flex-1',
                ui.layout.chat_layout === 'bubble_split' &&
                  'rounded-2xl border border-[var(--border)] bg-[var(--surface)] p-3.5',
              )}
            >
              <div className="mb-1 flex items-center gap-2 text-[11.5px]">
                <span className="font-medium text-[var(--fg)]">Zenko</span>
                {badgeStyle !== 'hidden' && (
                  <span
                    className={
                      MODEL_BADGE_CLASS[badgeStyle] || MODEL_BADGE_CLASS.bordered_chip
                    }
                  >
                    {activeModel || 'live-model'}
                  </span>
                )}
                {ui.widgets.show_timestamps && (
                  <span className="text-[10.5px] text-[var(--muted)]">16:04</span>
                )}
              </div>
              <p className="text-[13px] leading-relaxed text-[var(--fg)]/90">
                All 101 Rust tests pass at 28.4 MB VmRSS.
              </p>
              {ui.widgets.show_cost_badges && (
                <div className="mt-1.5 font-mono text-[10.5px] text-[var(--muted)]">
                  33202 tok · 27264 cached
                </div>
              )}
            </div>
          </div>
        </div>
      )
    },

    user_bubble: () => (
      <div className="flex w-full max-w-[540px] flex-col items-end gap-1.5">
        <span className="mr-10 font-mono text-[10.5px] text-[var(--muted)]">You · 16:04</span>
        <div className="flex items-start gap-2.5">
          {renderUserBubblePreview(
            chatSlot.user_bubble,
            'Refactor the streaming parser to zero-regex deterministic byte scanning.',
            'text-[14px] leading-relaxed',
          )}
          {chatSlot.show_avatars && chatSlot.user_avatar !== 'hidden' && (
            <UserAvatar
              variant={chatSlot.user_avatar || 'icon_user'}
              frame={chatSlot.user_avatar_frame || 'circle'}
              color={chatSlot.user_avatar_color}
              icons={ui.icons}
              size={28}
              className="mt-0.5"
            />
          )}
        </div>
      </div>
    ),

    avatars: () => (
      <div className="flex w-full max-w-[620px] flex-col gap-5">
        {/* Welcome Hero Emblem + User Profile Readout */}
        <div className="z-card-item flex flex-wrap items-center justify-between gap-4 px-5 py-4">
          <div className="flex items-center gap-4">
            <AgentAvatar
              variant={ui.motion.orb_style === 'hidden' ? 'liquid_halo' : ui.motion.orb_style}
              frame={chatSlot.agent_avatar_frame || 'squircle'}
              color={chatSlot.agent_avatar_color}
              icons={ui.icons}
              size={52}
            />
            <div>
              <div className="text-[14px] font-semibold text-[var(--fg)]">Zenko Fox Avatar</div>
              <div className="mt-0.5 font-mono text-[11px] text-[var(--muted)]">
                {ui.motion.orb_style} · frame:{chatSlot.agent_avatar_frame === 'bare' ? 'hidden' : chatSlot.agent_avatar_frame || 'squircle'}
                {chatSlot.agent_avatar_color ? ` · ${chatSlot.agent_avatar_color}` : ' · theme'}
              </div>
            </div>
          </div>
          <div className="flex items-center gap-3">
            <div className="text-right">
              <div className="text-[13px] font-semibold text-[var(--fg)]">User Profile</div>
              <div className="font-mono text-[10.5px] text-[var(--muted)]">
                {chatSlot.user_avatar || 'icon_user'} · {chatSlot.user_avatar_frame === 'bare' ? 'hidden' : chatSlot.user_avatar_frame || 'circle'}
                {chatSlot.user_avatar_color ? ` · ${chatSlot.user_avatar_color}` : ''}
              </div>
            </div>
            <UserAvatar
              variant={chatSlot.user_avatar === 'hidden' ? 'icon_user' : chatSlot.user_avatar || 'icon_user'}
              frame={chatSlot.user_avatar_frame || 'circle'}
              color={chatSlot.user_avatar_color}
              icons={ui.icons}
              size={40}
            />
          </div>
        </div>

        {/* Live Chat Pair */}
        <div className="flex flex-col gap-3.5 px-1">
          <div className="flex items-start justify-end gap-2.5">
            {renderUserBubblePreview(
              chatSlot.user_bubble,
              'Run the workspace build in zenko_task and check RSS.',
              'text-[13px]',
            )}
            {chatSlot.show_avatars && chatSlot.user_avatar !== 'hidden' && (
              <UserAvatar
                variant={chatSlot.user_avatar || 'icon_user'}
                frame={chatSlot.user_avatar_frame || 'circle'}
                color={chatSlot.user_avatar_color}
                icons={ui.icons}
                size={28}
                className="mt-0.5"
              />
            )}
          </div>
          <div className="flex items-start gap-3">
            {chatSlot.show_avatars && ui.motion.orb_style !== 'hidden' && (
              <AgentAvatar
                variant={ui.motion.orb_style}
                frame={chatSlot.agent_avatar_frame || 'squircle'}
                color={chatSlot.agent_avatar_color}
                icons={ui.icons}
                size={28}
                className="mt-0.5"
              />
            )}
            <div className="flex flex-col gap-0.5">
              <span className="text-[12px] font-medium text-[var(--fg)]">Zenko</span>
              <span className="text-[13px] text-[var(--muted)]">
                {chatSlot.show_avatars
                  ? 'Fox emblems, human profile icons, frames, and custom colors update live.'
                  : 'Chat avatars are currently hidden (toggle Show Chat Avatars above).'}
              </span>
            </div>
          </div>
        </div>
      </div>
    ),

    loading_state: () => (
      <div className="flex w-full max-w-[520px] flex-col items-center gap-4">
        <div className="flex w-full items-center justify-center rounded-xl border border-[var(--border)]/75 bg-[var(--surface)]/70 px-5 py-6">
          <LoadingStatusBar
            loadingStyle={chatSlot.loading_style}
            loaderVariant={loaderVariant}
            label="Thinking & running tools..."
            elapsedLabel="4s"
            active={true}
          />
        </div>
        <span className="font-mono text-[11px] text-[var(--muted)]">
          {chatSlot.loading_style || 'beui_loader'}
          {(chatSlot.loading_style || 'beui_loader') === 'beui_loader' ? ` · ${loaderVariant}` : ''}
        </span>
      </div>
    ),

    agent_activity: () => (
      <div className="w-full max-w-[600px]">
        <ActivityDisplay
          style={chatSlot.activity_style}
          diffStyle={chatSlot.diff_style}
          shellStyle={chatSlot.shell_style}
          webSearchStyle={chatSlot.web_search_style}
          status="complete"
          statusLabel="Completed 3 steps"
          durationSeconds={3}
          durationLabel="3s"
          defaultOpen={true}
          tools={[PREVIEW_SHELL_TOOL, PREVIEW_READ_TOOL, PREVIEW_DIFF_TOOL]}
          items={[
            { id: 'a1', type: 'trace', kind: 'thinking', label: 'Thought for 2s', detail: 'Checking crate layout and test matrix' },
            { id: 'a2', type: 'tool', action: 'run', target: 'cargo test --workspace · ok · 412ms' },
            { id: 'a3', type: 'tool', action: 'read', target: 'crates/zenko-core/src/studio_schema.rs · 8ms' },
            { id: 'a4', type: 'tool', action: 'edit', target: 'crates/zenko-core/src/ui.rs', additions: 2, deletions: 0 },
          ]}
        />
      </div>
    ),

    thinking_trace: () => (
      <div className="flex w-full max-w-[580px] flex-col gap-2.5">
        {!ui.widgets.show_thinking || chatSlot.thinking_style === 'hidden' ? (
          <div className="rounded-xl border border-dashed border-[var(--border)] px-4 py-5 text-center font-mono text-[11.5px] text-[var(--muted)]">
            Reasoning traces hidden in chat.
          </div>
        ) : chatSlot.thinking_style === 'aimran_stretchtext' ? (
          <div className="rounded-xl border border-[var(--border)]/70 bg-[var(--surface)]/55 p-4">
            <Stretchtext
              kicker="Click underlined phrases to expand inline"
              size={14}
              mainColor="var(--accent)"
              secondaryColor="color-mix(in srgb, var(--accent) 26%, var(--surface))"
              segments={[
                { kind: 'text', text: 'Verified ' },
                {
                  kind: 'expand',
                  id: 'wal',
                  trigger: 'SQLite WAL memory',
                  detail: ' (28.4 MB VmRSS, zero page leaks)',
                },
                { kind: 'text', text: ' and ' },
                {
                  kind: 'expand',
                  id: 'tests',
                  trigger: '101 workspace tests',
                  detail: ' across all 8 Rust crates',
                },
                { kind: 'text', text: ' before emitting the patch.' },
              ]}
            />
          </div>
        ) : chatSlot.thinking_style === 'inline_quote' ? (
          <div className="border-l-2 border-[var(--accent)]/50 pl-3.5 font-mono text-[12px] leading-relaxed text-[var(--muted)]">
            Checking SQLite WAL mode and verifying zero-regex path checks before emitting the patch.
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            <div
              className={cn(
                'inline-flex w-fit items-center gap-2 text-[12.5px] text-[var(--muted)]',
                chatSlot.thinking_style === 'minimal_bar' &&
                  'rounded-full border border-[var(--border)] bg-[var(--surface)] px-3 py-1 font-mono text-[11px]',
              )}
            >
              <Clock size={12.5} />
              <span className="font-medium">Completed 2 steps</span>
              <span className="font-mono text-[10.5px] text-[var(--accent)]">Hide</span>
              <ChevronDown size={13} className="rotate-180" />
            </div>
            <div className="border-l border-[var(--border)] pl-3.5 font-mono text-[11.5px] leading-relaxed text-[var(--muted)]">
              <div className="mb-1 flex items-center gap-1.5 text-[10.5px] uppercase tracking-wider">
                <BrainIcon size={12} style={{ color: 'var(--accent)' }} /> Thought for 3s
              </div>
              Verifying workspace tests and memory RSS before finalizing response.
            </div>
          </div>
        )}
      </div>
    ),

    streaming_response: () => <StreamingPreview ui={ui} />,

    message_actions: () => {
      const citationsStyle = chatSlot.citations_style || 'beui_citations'
      const useBeuiSources = chatSlot.show_citations && citationsStyle === 'beui_citations'
      return (
        <div className="flex w-full max-w-[600px] flex-col gap-2 rounded-xl border border-[var(--border)]/70 bg-[var(--surface)]/55 p-4">
          <p className="text-[13.5px] leading-relaxed text-[var(--fg)]/90">
            Daemon is healthy at 28.4 MB VmRSS with WAL mode enabled and zero active warnings.
          </p>
          {chatSlot.message_actions_style === 'beui_inline' ? (
            <>
              <StreamingResponse
                status="complete"
                copyText="Daemon is healthy at 28.4 MB VmRSS."
                onRetry={() => {}}
                sources={useBeuiSources ? PREVIEW_CITATIONS : []}
                announce={false}
                footerMeta={
                  <span className="font-mono text-[11px] tabular-nums text-[var(--muted)]/80">
                    33202 tok · 27264 cached
                  </span>
                }
              >
                {null}
              </StreamingResponse>
              {chatSlot.show_citations && !useBeuiSources && (
                <CitationsDisplay
                  citations={PREVIEW_CITATIONS}
                  style={citationsStyle}
                />
              )}
            </>
          ) : (
            <AssistantMessageFooter
              style={chatSlot.message_actions_style}
              text="Daemon is healthy at 28.4 MB VmRSS."
              citations={chatSlot.show_citations ? PREVIEW_CITATIONS : []}
              citationsStyle={citationsStyle}
              onRegenerate={() => {}}
              tokenMeta={
                <span className="font-mono text-[11px] tabular-nums text-[var(--muted)]/80">
                  33202 tok · 27264 cached
                </span>
              }
            />
          )}
        </div>
      )
    },

    tool_execution: () => {
      const showMultiCollapsible =
        chatSlot.tool_card_style === 'aui_tool_call' ||
        chatSlot.tool_card_style === 'aui_tool_call_icon' ||
        chatSlot.tool_card_style === 'beui_tool_result'
      const sampleTools = showMultiCollapsible
        ? [PREVIEW_SHELL_TOOL, PREVIEW_READ_TOOL, PREVIEW_DIFF_TOOL]
        : [PREVIEW_SHELL_TOOL]
      return (
        <div className="flex w-full max-w-[600px] flex-col gap-2">
          {sampleTools.map((tool) => (
            <SmartToolElement
              key={tool.id}
              tool={tool}
              toolCardStyle={chatSlot.tool_card_style}
              diffStyle={chatSlot.diff_style}
              shellStyle={chatSlot.shell_style}
            />
          ))}
        </div>
      )
    },

    terminal_block: () => (
      <div className="w-full max-w-[600px]">
        <SmartToolElement
          tool={PREVIEW_SHELL_TOOL}
          toolCardStyle="aui_smart"
          shellStyle={chatSlot.shell_style}
        />
      </div>
    ),

    code_blocks: () => (
      <div className="w-full max-w-[600px]">
        <MarkdownCodeBlock
          className="language-rust"
          style={chatSlot.code_block_style || 'aui_code_runner'}
        >
          {'pub fn resolve_auto_send_mode(bg_active: bool) -> SendMode {\n    if bg_active { SendMode::Steer } else { SendMode::Queue }\n}'}
        </MarkdownCodeBlock>
      </div>
    ),

    code_diffs: () => (
      <div className="w-full max-w-[600px]">
        <SmartToolElement
          tool={PREVIEW_DIFF_TOOL}
          toolCardStyle="aui_smart"
          diffStyle={chatSlot.diff_style}
        />
      </div>
    ),

    permission_approvals: () => (
      <div className="w-full max-w-[540px]">
        <ApprovalElement
          approval={PREVIEW_APPROVAL}
          style={chatSlot.approval_style}
          onResolve={() => {}}
        />
      </div>
    ),

    memory_badges: () => (
      <div className="flex w-full max-w-[560px] flex-col items-start gap-2 rounded-xl border border-[var(--border)]/70 bg-[var(--surface)]/50 p-4">
        <span className="font-mono text-[11px] text-[var(--muted)]">Zenko · MiniMax-M3.1-Flash</span>
        {chatSlot.skill_badge_style === 'hidden' ? (
          <span className="font-mono text-[11px] text-[var(--muted)]">Skill &amp; memory badges hidden</span>
        ) : (
          <SkillAndMemoryBadges
            style={chatSlot.skill_badge_style}
            skills={['rust-review', 'tauri-audit']}
            chips={[
              { id: 's1', text: 'skill: rust-review', change: 'added' },
              { id: 'm1', text: 'SQLite WAL tuned', change: 'updated' },
              { id: 'm2', text: 'Zero-regex policy', change: 'existing' },
            ]}
          />
        )}
      </div>
    ),

    data_tables: () => (
      <div className="w-full max-w-[620px]">
        <DataTable
          columns={PREVIEW_TABLE_COLUMNS}
          rows={PREVIEW_TABLE_ROWS}
          tableStyle={ui.surface.table_style}
        />
      </div>
    ),

    element_states: () => <ElementStatesPreview ui={ui} />,

    chat_input: () => (
      <div className="w-full max-w-[660px]">
        <PromptComposer
          compactWrapper
          value={ctx.previewPrompt}
          onChange={ctx.setPreviewPrompt}
          onSend={() => {}}
          onStop={() => {}}
          isRunning={false}
          activeProvider={ctx.activeProvider}
          activeModel={activeModel}
          providers={ctx.providers}
          configuredProviders={ctx.configuredProviders}
          models={ctx.models}
          onSelectProviderModel={() => {}}
          permissionMode="autonomous"
          onChangePermission={() => {}}
          streamingEnabled={true}
          onToggleStreaming={() => {}}
          totalTokens={10300}
          branchEditBanner={false}
          onCancelBranchEdit={() => {}}
          composerStyle={ui.components.composer}
          popoverStyle={ui.surface.popover_style || 'gooey'}
        />
      </div>
    ),

    model_picker: () => (
      <ModelPicker
        inlinePreview
        activeProvider={ctx.activeProvider}
        activeModel={activeModel}
        providers={ctx.providers}
        configuredProviders={ctx.configuredProviders}
        activeModels={ctx.models}
        pickerStyle={ui.components.composer.model_picker_style}
        triggerChrome={ui.components.composer.model_trigger_chrome}
        pillClass={
          ACTION_PILL_PREVIEW_CLASS[ui.components.composer.action_pills] ||
          ACTION_PILL_PREVIEW_CLASS.soft_tint
        }
        open={true}
        onOpenChange={() => {}}
        onSelectProviderModel={() => {}}
      />
    ),

    left_menubar: () => (
      <Sidebar
        inlinePreview
        open={true}
        onToggle={() => {}}
        sessions={PREVIEW_SESSIONS}
        activeSessionId="preview-1"
        activeView="chat"
        ui={ui}
        onSelectSession={() => {}}
        onNewSession={() => {}}
        onClearActiveSession={() => {}}
        onDeleteSession={() => {}}
        onOpenSettings={() => {}}
      />
    ),

    right_menubar: () => (
      <div className="flex items-center gap-8">
        <FloatingDock
          inlinePreview
          activeView={ctx.previewDockView}
          onSelectView={ctx.setPreviewDockView}
          ui={ui}
          onToggleMode={toggleMode}
          onOpenQuickCustomizer={() => {}}
          collapsed={false}
          onToggleCollapse={() => {}}
        />
        <div className="flex max-w-[260px] flex-col gap-2 text-left">
          <span className="text-[12.5px] font-semibold text-[var(--fg)]">
            Interactive Dock Preview
          </span>
          <span className="text-[11.5px] leading-relaxed text-[var(--muted)]">
            Click any icon or toggle Dark/Light mode to test the spring indicator and active icon family.
          </span>
        </div>
      </div>
    ),

    typography: () => (
      <div className="flex w-full max-w-[620px] flex-col gap-3 rounded-xl border border-[var(--border)]/75 bg-[var(--surface)]/65 p-5">
        <div className="flex items-center justify-between font-mono text-[11px] text-[var(--muted)]">
          <span>UI: {ui.typography.custom_ui_font || ui.typography.ui_font}</span>
          <span>Code: {ui.typography.custom_mono_font || ui.typography.mono_font}</span>
          <span>{ui.typography.base_size_px}px</span>
        </div>
        <h3
          className="text-[20px] font-semibold tracking-[-0.02em] text-[var(--fg)]"
          style={{ fontFamily: 'var(--font-display)' }}
        >
          Local-first autonomous engineering, zero friction.
        </h3>
        <p className="text-[13.5px] leading-relaxed text-[var(--fg)]/85">
          Typography updates instantly across every chat message, sidebar session, and Studio panel using self-hosted SIL OFL variable fonts.
        </p>
        <div className="rounded-lg border border-[var(--border)]/65 bg-[var(--bg)] px-3.5 py-2.5 font-mono text-[12px] text-[var(--fg)]">
          <span style={{ color: 'var(--accent)' }}>--&gt;</span> GET /api/bootstrap 200 1.4ms · 0123456789 !== ==&gt;
        </div>
      </div>
    ),

    layout: () => (
      <div className="zenko-canvas flex w-full max-w-[600px] flex-col gap-4 rounded-xl border border-[var(--border)] bg-[var(--bg)] p-6">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <AgentAvatar
              variant={ui.motion.orb_style === 'hidden' ? 'liquid_halo' : ui.motion.orb_style}
              frame={chatSlot.agent_avatar_frame || 'squircle'}
              color={chatSlot.agent_avatar_color}
              icons={ui.icons}
              size={30}
              status="running"
            />
            <div>
              <div className="text-[14px] font-semibold text-[var(--fg)]">Zenko Canvas &amp; Motion</div>
              <div className="font-mono text-[11px] text-[var(--muted)]">
                {ui.surface.bg_pattern} · {ui.layout.density} · {ui.motion.preset}
              </div>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <ThemeToggle
              mode={activeMode}
              variant={themeTransition}
              start="center"
              onToggle={toggleMode}
              className="h-7 w-7 rounded-lg border border-[var(--border)] bg-[var(--card)] text-[var(--fg)]"
              iconClassName="size-3.5"
            />
            <span className="rounded-full border border-[var(--border)] bg-[var(--card)] px-2.5 py-0.5 font-mono text-[11px] text-[var(--muted)]">
              {themeTransition}
            </span>
          </div>
        </div>
        <div className="z-surface-card text-[13px] leading-relaxed text-[var(--fg)]/90">
          The quick brown fox ships 24/7 native Rust agents.{' '}
          <code className="font-mono text-[12px]" style={{ color: 'var(--accent)' }}>
            --&gt; AGENT ok 1.1s
          </code>
        </div>
      </div>
    ),

    buttons: () => (
      <div className="flex flex-wrap items-center justify-center gap-2.5">
        <button type="button" className="z-btn z-btn-primary">
          <ZenkoIcon slot="sparkle" icons={ui.icons} size={14} /> Primary
        </button>
        <button type="button" className="z-btn z-btn-vermilion-soft">
          <ZenkoIcon slot="plus" icons={ui.icons} size={14} /> Accent Soft
        </button>
        <button type="button" className="z-btn z-btn-secondary">
          <ZenkoIcon slot="terminal" icons={ui.icons} size={14} /> Secondary
        </button>
        <button type="button" className="z-btn z-btn-ghost">
          Ghost
        </button>
        <button type="button" className="z-btn z-btn-danger">
          Danger
        </button>
      </div>
    ),

    surfaces: () => {
      const linkHoverStyle = ui.surface.link_hover_style || 'zenko_underline'
      const previewLink = (label: string) =>
        linkHoverStyle === 'zenko_underline' ? (
          <a
            href="#preview-link"
            onClick={(e) => e.preventDefault()}
            className="zenko-prose-link font-medium"
          >
            {label}
          </a>
        ) : (
          <LineHoverLink
            href="#preview-link"
            onClick={(e) => e.preventDefault()}
            variant={linkHoverStyle as LineHoverVariant}
            className="font-medium text-[var(--accent)]"
          >
            {label}
          </LineHoverLink>
        )

      return (
        <div className="flex w-full max-w-[580px] flex-col gap-3.5">
          <div className="z-surface-card flex items-center justify-between text-[13px]">
            <div>
              <div className="font-medium text-[var(--fg)]">Primary Surface Container</div>
              <div className="mt-0.5 text-[11.5px] text-[var(--muted)]">
                Applied across Dashboard charts, Memory, Settings, and Telemetry panels.
              </div>
            </div>
            <span className="font-mono text-[11px] text-[var(--muted)]">
              {ui.surface.card_style} · {ui.surface.radius_px}px
            </span>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="z-card-item p-3 text-[12px]">
              <div className="font-semibold text-[var(--fg)]">Option Card Item</div>
              <div className="mt-0.5 text-[11px] text-[var(--muted)]">
                Studio tiles, KPI cells &amp; vault stats
              </div>
            </div>
            <div className="z-card-item p-3 text-[12px]">
              <div className="font-semibold text-[var(--fg)]">Link Hover ({linkHoverStyle})</div>
              <div className="mt-1.5 flex flex-wrap items-center gap-3 text-[12px]">
                {previewLink('Zenko Docs')}
                {previewLink('SQLite WAL')}
              </div>
            </div>
          </div>
        </div>
      )
    },

    hover_hints: () => (
      <div className="flex w-full max-w-[580px] flex-col items-center gap-5">
        {ui.surface.show_hover_hints === false ? (
          <div className="rounded-xl border border-dashed border-[var(--border)] px-5 py-4 font-mono text-[11.5px] text-[var(--muted)]">
            Hover hints are currently disabled across the WebUI.
          </div>
        ) : (
          <div className="flex flex-wrap items-center justify-center gap-3">
            <ZenkoTooltipBadge label="Dashboard" />
            <ZenkoTooltipBadge label="Chat Workbench" />
            <ZenkoTooltipBadge label="Customization" shortcut="⌘." />
          </div>
        )}

        <div className="flex flex-wrap items-center justify-center gap-3">
          <ZenkoTooltip label="Dashboard" side="left">
            <button type="button" className="z-btn z-btn-secondary z-btn-sm">
              <ZenkoIcon slot="grid" icons={ui.icons} size={14} /> Hover: Dock Item
            </button>
          </ZenkoTooltip>
          <ZenkoTooltip label="Context used: 18% · 36.2k / 200k tokens" side="top">
            <button type="button" className="z-btn z-btn-secondary z-btn-sm font-mono text-[11.5px]">
              Hover: Context Meter
            </button>
          </ZenkoTooltip>
          <ZenkoTooltip label="Quick Theme Customizer" shortcut="⌘." side="right">
            <button type="button" className="z-btn z-btn-secondary z-btn-sm">
              <ZenkoIcon slot="palette" icons={ui.icons} size={14} /> Hover: Shortcut
            </button>
          </ZenkoTooltip>
        </div>

        <span className="font-mono text-[10.5px] text-[var(--muted)]">
          style:{ui.surface.tooltip_style || 'accent_edge'} · shape:{ui.surface.tooltip_shape || 'pill'} · motion:{ui.surface.tooltip_motion || 'spring'}
        </span>
      </div>
    ),

    dashboard_preview: () => <DashboardMiniPreview ui={ui} />,

    tabs_accordions: () => (
      <div className="flex w-full max-w-[560px] flex-col items-center gap-4">
        {tabsVariant === 'skiper96' ? (
          <ExpandableTabs
            value={ctx.previewTab}
            onValueChange={ctx.setPreviewTab}
            items={[
              { id: 'overview', label: 'Overview', icon: <Layers size={14} /> },
              { id: 'activity', label: 'Activity', icon: <Activity size={14} /> },
              { id: 'settings', label: 'Settings', icon: <Settings size={14} /> },
            ]}
          />
        ) : (
          <Tabs value={ctx.previewTab} onValueChange={ctx.setPreviewTab} variant={tabsVariant}>
            <TabsList>
              <TabsTrigger value="overview">Overview</TabsTrigger>
              <TabsTrigger value="activity">Activity</TabsTrigger>
              <TabsTrigger value="settings">Settings</TabsTrigger>
            </TabsList>
          </Tabs>
        )}
        <div className="w-full">
          {ui.surface.accordion_style === 'rareui_faq' ? (
            <RareFaq
              defaultValue="faq-1"
              accent="var(--accent)"
              avatar={<ZenkoFoxLogo size={16} variant="fox" />}
              items={[
                {
                  id: 'faq-1',
                  question: 'How does RareUI FAQ animate?',
                  answer: 'Questions pop out as conversational message bubbles with spring replies matching your 60/30/10 theme.',
                },
                {
                  id: 'faq-2',
                  question: 'Where does it appear in Zenko?',
                  answer: 'Enable Dashboard → Show Interactive FAQ or use it across collapsible studio sections.',
                },
              ]}
            />
          ) : ui.surface.accordion_style === 'compact' ? (
            <div className="flex flex-col gap-1.5 border-l border-[var(--border)] pl-3 text-[12.5px]">
              <div className="flex items-center gap-1.5 font-medium text-[var(--fg)]">
                <ChevronDown size={13} className="text-[var(--accent)]" />
                <span>Accordion style: compact</span>
              </div>
              <p className="pl-5 text-[12px] text-[var(--muted)]">
                Tight indented disclosure tree with zero outer card chrome.
              </p>
            </div>
          ) : (
            <BouncyAccordion
              collapsible
              defaultValue="acc-1"
              items={[
                {
                  id: 'acc-1',
                  title: <span className="text-[13px] font-medium">Accordion style: {ui.surface.accordion_style || 'bouncy'}</span>,
                  description: 'Click to test smooth spring disclosure transitions.',
                },
              ]}
            />
          )}
        </div>
      </div>
    ),

    overlays: () => <OverlaysPreview ctx={ctx} />,

    toasts: () => <ToastPreview ctx={ctx} />,
  }

  return (MAP[previewKey] || MAP.surfaces)()
}
