import { useEffect, useMemo, useState, type ReactNode } from 'react'
import {
  ArrowUpRight,
  Braces,
  Check,
  Copy,
  FileCode2,
  FolderSearch,
  Globe,
  RotateCcw,
  ShieldAlert,
  SquareTerminal,
  Terminal,
  Wrench,
  Zap,
  type LucideIcon,
} from 'lucide-react'
import { cn } from '@/lib/utils'
import {
  classifySmartTool,
  extractJsonField,
  extractMarkdownTableData,
  parseFileTreeFromTool,
  parsePermissionGrantProps,
  parseShellOrCodeTool,
  parseUnifiedDiff,
  parseWebSearchFromTool,
} from '@/lib/elements'
import type { ApprovalRecord, ComponentSlots, UiCustomization } from '@/lib/types'
import { AgentActivity, type AgentActivityItem } from '@/components/agents/agent-activity'
import { Citations, type CitationItem } from '@/components/agents/citations'
import { ThinkingShimmer } from '@/components/agents/loading-states/thinking-shimmer'
import { ToolApproval, ToolApprovalCode } from '@/components/agents/tool-approval'
import {
  ToolResult,
  ToolResultOutput,
  type ToolResultKind,
} from '@/components/agents/tool-result'
import { CodeBlock as EaseCodeBlock } from '@/components/easeui/code-block'
import { PixelLoader as EasePixelLoader } from '@/components/easeui/pixel-loader'
import { ToolApproval as EaseToolApproval } from '@/components/easeui/tool-approval'
import { ToolChip as EaseToolChip } from '@/components/easeui/tool-chip'
import { CopyCommand as AimranCopyCommand } from '@/components/aimran/copy-command'
import { KokonutAICitations } from '@/components/kokonutui/ai-citations'
import {
  KokonutAIDiffReview,
  unifiedDiffToKokonutFiles,
} from '@/components/kokonutui/ai-diff-review'
import { KokonutAILoading } from '@/components/kokonutui/ai-loading'
import { KokonutAIToolApproval } from '@/components/kokonutui/ai-tool-approval'
import { KokonutAIToolCalls } from '@/components/kokonutui/ai-tool-calls'
import { AiStatus as RareAiStatus, type AiStatusState } from '@/components/rareui/ai-status'
import { CodeBlock as RareCodeBlock } from '@/components/rareui/code-block'
import { WebSearch as RareWebSearch } from '@/components/rareui/web-search'
import { Loader, type LoaderVariant } from '@/components/motion/loader'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { CodeDiff } from '@/components/assistant-ui/elements/code-diff'
import { CodeRunner } from '@/components/assistant-ui/elements/code-runner'
import { DataTable } from '@/components/assistant-ui/elements/data-table'
import { FileTree } from '@/components/assistant-ui/elements/file-tree'
import {
  GenerationLoader,
  type GenerationLoaderVariant,
} from '@/components/assistant-ui/elements/loading-state'
import { MemoryChips, type MemoryChip } from '@/components/assistant-ui/elements/memory-chips'
import { MessageActions } from '@/components/assistant-ui/elements/message-actions'
import {
  PermissionGrant,
  type GrantScope,
} from '@/components/assistant-ui/elements/permission-grant'
import {
  ReviewableDiff,
  type DiffHunk,
} from '@/components/assistant-ui/elements/reviewable-diff'
import {
  RunActivity,
  type RunActivityEntry,
  type RunActivityStatus,
} from '@/components/assistant-ui/elements/run-activity'
import { TerminalBlock } from '@/components/assistant-ui/elements/terminal-block'
import { ThinkingIndicator } from '@/components/assistant-ui/elements/thinking-indicator'
import { ToolCall } from '@/components/assistant-ui/elements/tool-call'
import { ToolError } from '@/components/assistant-ui/elements/tool-error'
import {
  ToolTimeline,
  type TimelineStat,
  type TimelineStep,
} from '@/components/assistant-ui/elements/tool-timeline'
import { WebSearch } from '@/components/assistant-ui/elements/web-search'

export interface NormalizedToolStep {
  id: string
  name: string
  args_preview?: string
  status: 'running' | 'ok' | 'error'
  duration_ms?: number
  diff?: string | null
  content: string
}

type LoadingStyle = NonNullable<ComponentSlots['chat_page']['loading_style']>
type SkillBadgeStyle = NonNullable<ComponentSlots['chat_page']['skill_badge_style']>
type DiffStyle = NonNullable<ComponentSlots['chat_page']['diff_style']>
type ShellStyle = NonNullable<ComponentSlots['chat_page']['shell_style']>
type CodeBlockStyle = NonNullable<ComponentSlots['chat_page']['code_block_style']>
type WebSearchStyle = NonNullable<ComponentSlots['chat_page']['web_search_style']>
type MessageActionsStyle = NonNullable<ComponentSlots['chat_page']['message_actions_style']>
type CitationsStyle = NonNullable<ComponentSlots['chat_page']['citations_style']>

function resolveRareAiStatusState(label: string, active: boolean): AiStatusState {
  if (!active) return 'done'
  const lower = label.toLowerCase()
  if (lower.includes('search') || lower.includes('web') || lower.includes('research')) {
    return 'researching'
  }
  if (lower.includes('writ') || lower.includes('response')) return 'writing'
  if (lower.includes('run') || lower.includes('tool') || lower.includes('analyz')) {
    return 'analyzing'
  }
  return 'thinking'
}

function parseElapsedSeconds(label?: string): number | undefined {
  if (!label) return undefined
  let secs = 0
  for (const part of label.trim().split(' ')) {
    if (part.endsWith('m')) {
      const n = Number.parseFloat(part.slice(0, -1))
      if (Number.isFinite(n)) secs += n * 60
    } else if (part.endsWith('s')) {
      const n = Number.parseFloat(part.slice(0, -1))
      if (Number.isFinite(n)) secs += n
    }
  }
  return secs > 0 ? secs : undefined
}

function readThemeAccentHex(): string {
  if (typeof window === 'undefined') return '#ea5829'
  const raw = window.getComputedStyle(document.documentElement).getPropertyValue('--accent').trim()
  return raw.startsWith('#') && (raw.length === 4 || raw.length === 7) ? raw : '#ea5829'
}

const GENERATION_VARIANT_MAP: Partial<Record<LoadingStyle, GenerationLoaderVariant>> = {
  aui_loading_dots: 'dots',
  aui_loading_squares: 'squares',
  aui_loading_rounded: 'rounded',
}

/** 1. Loading & Thinking Indicator (BeUI Loader | ThinkingIndicator | GenerationLoader | EaseUI PixelLoader | RareUI AiStatus) */
export function LoadingStatusBar({
  loadingStyle = 'beui_loader',
  loaderVariant = 'dot-matrix',
  label,
  elapsedLabel,
  active = true,
}: {
  loadingStyle?: LoadingStyle
  loaderVariant?: LoaderVariant
  label: string
  elapsedLabel?: string
  active?: boolean
}) {
  const [tick, setTick] = useState(0)
  const genVariant = GENERATION_VARIANT_MAP[loadingStyle]

  useEffect(() => {
    if (!active || !genVariant) return
    const id = window.setInterval(() => setTick((t) => t + 1), 110)
    return () => window.clearInterval(id)
  }, [active, genVariant])

  if (loadingStyle === 'kokonut_loading') {
    return (
      <KokonutAILoading
        label={label}
        elapsedLabel={elapsedLabel}
        active={active}
      />
    )
  }

  if (loadingStyle === 'easeui_pixel_loader') {
    return (
      <div className="inline-flex items-center gap-2.5">
        <EasePixelLoader label={label} showElapsed={false} className="text-[13px]" />
        {elapsedLabel && (
          <span className="font-mono text-[11px] tabular-nums text-[var(--muted)]/80">
            {elapsedLabel}
          </span>
        )}
      </div>
    )
  }

  if (loadingStyle === 'rareui_ai_status') {
    const elapsedSecs = parseElapsedSeconds(elapsedLabel)
    return (
      <RareAiStatus
        state={resolveRareAiStatusState(label, active)}
        label={label}
        elapsed={elapsedSecs}
        timer={elapsedSecs !== undefined}
        shimmer={active}
        className="text-[13px]"
      />
    )
  }

  if (loadingStyle === 'aui_thinking_indicator') {
    return <ThinkingIndicator label={label} elapsed={elapsedLabel} className="text-[13px]" />
  }

  if (genVariant) {
    return (
      <div className="inline-flex items-center gap-2.5">
        <GenerationLoader inline label={label} tick={tick} variant={genVariant} />
        {elapsedLabel && (
          <span className="font-mono text-[11px] tabular-nums text-[var(--muted)]/80">
            {elapsedLabel}
          </span>
        )}
      </div>
    )
  }

  return (
    <div className="inline-flex items-center gap-2.5">
      <Loader
        variant={loaderVariant}
        size={16}
        className="shrink-0 text-[var(--accent)]"
        label={label}
      />
      <ThinkingShimmer>{label}</ThinkingShimmer>
      {elapsedLabel && (
        <span className="font-mono text-[11px] tabular-nums text-[var(--muted)]/80">
          {elapsedLabel}
        </span>
      )}
    </div>
  )
}

const TIMELINE_ICON_BY_ACTION: Record<string, LucideIcon> = {
  read: FolderSearch,
  edit: FileCode2,
  run: Terminal,
  web: Globe,
}

function resolveStepIcon(toolName: string): LucideIcon {
  const lower = toolName.toLowerCase()
  if (lower.includes('web') || lower.includes('http') || lower.includes('search')) return TIMELINE_ICON_BY_ACTION.web
  if (lower.includes('shell') || lower.includes('exec') || lower.includes('run')) return TIMELINE_ICON_BY_ACTION.run
  if (lower.includes('edit') || lower.includes('write') || lower.includes('patch')) return TIMELINE_ICON_BY_ACTION.edit
  if (lower.includes('read') || lower.includes('list') || lower.includes('find') || lower.includes('grep')) {
    return TIMELINE_ICON_BY_ACTION.read
  }
  return Wrench
}

function nodeToPlainLabel(node: ReactNode): string {
  if (typeof node === 'string' || typeof node === 'number') return String(node)
  return ''
}

function describeActivityItem(item: AgentActivityItem): {
  label: string
  labelNode: ReactNode
  detailNode?: ReactNode
} {
  switch (item.type) {
    case 'tool': {
      const targetText = nodeToPlainLabel(item.target)
      return {
        label: `${item.action}${targetText ? ` ${targetText}` : ''}`,
        labelNode: item.target,
      }
    }
    case 'text':
      return { label: nodeToPlainLabel(item.content) || 'note', labelNode: item.content }
    case 'search':
      return { label: nodeToPlainLabel(item.query) || 'search', labelNode: item.query }
    case 'trace':
      return {
        label: nodeToPlainLabel(item.label) || item.kind,
        labelNode: item.label,
        detailNode: item.detail,
      }
    case 'step':
      return {
        label: nodeToPlainLabel(item.label) || 'step',
        labelNode: item.label,
        detailNode: item.meta,
      }
  }
}

/**
 * 2. Unified Agent Activity & Tool Execution Display.
 *    Handles all activity AND tool execution variants in one clean surface (zero duplicate lists!):
 *    Collapsible+ (`aui_tool_call_icon`) | Collapsible (`aui_tool_call`) | Smart Elements (`aui_smart`)
 *    | BeUI ToolResult | BeUI Activity | RunActivity | ToolTimeline | TerminalBlock | CodeRunner
 *    | Detailed Cards | Hono Log Line | Minimal Pills | Hidden.
 */
const TOOL_CARD_ACTIVITY_STYLES: ReadonlySet<string> = new Set([
  'aui_tool_call_icon',
  'aui_tool_call',
  'aui_smart',
  'beui_tool_result',
  'aui_terminal_block',
  'aui_code_runner',
  'detailed_card',
  'terminal_line',
])

export function ActivityDisplay({
  style,
  items,
  tools,
  status,
  statusLabel,
  durationSeconds,
  durationLabel,
  defaultOpen = true,
  diffStyle = 'aui_code_diff',
  shellStyle = 'aui_terminal_block',
  webSearchStyle = 'aui_web_search',
  onRetryTool,
}: {
  style: ComponentSlots['chat_page']['activity_style']
  items: AgentActivityItem[]
  tools: readonly NormalizedToolStep[]
  status: 'working' | 'complete'
  statusLabel: string
  durationSeconds?: number
  durationLabel?: string
  defaultOpen?: boolean
  diffStyle?: DiffStyle
  shellStyle?: ShellStyle
  webSearchStyle?: WebSearchStyle
  onRetryTool?: (toolName: string, argsPreview?: string) => void
}) {
  const [open, setOpen] = useState(defaultOpen)

  const runEntries = useMemo<RunActivityEntry[]>(
    () =>
      items.map((item) => {
        const desc = describeActivityItem(item)
        return {
          id: item.id,
          kind: item.type === 'tool' ? 'tool' : 'commentary',
          label: desc.label,
          content:
            item.type === 'tool' ? (
              <span className="inline-flex flex-wrap items-center gap-1.5 text-[12.5px] text-[var(--muted)]">
                <span className="font-medium text-[var(--fg)]/85">{item.action}</span>
                <span className="rounded bg-[var(--card)] px-1.5 py-0.5 font-mono text-[11px]">
                  {item.target}
                </span>
              </span>
            ) : (
              <span className="text-[12.5px] text-[var(--muted)]">
                {desc.labelNode}
                {desc.detailNode ? <> — {desc.detailNode}</> : null}
              </span>
            ),
        }
      }),
    [items],
  )

  const timelineSteps = useMemo<TimelineStep[]>(
    () =>
      tools.map((t) => ({
        id: t.id,
        verb: t.status === 'running' ? `Running ${t.name}` : t.name,
        chip:
          extractJsonField(t.args_preview, ['path', 'command', 'cmd', 'query', 'name', 'url']) ||
          (t.duration_ms !== undefined ? `${t.duration_ms}ms` : t.status),
        icon: resolveStepIcon(t.name),
      })),
    [tools],
  )

  const timelineStats = useMemo<TimelineStat[]>(() => {
    const out: TimelineStat[] = []
    for (const t of tools) {
      if (!t.diff) continue
      const parsed = parseUnifiedDiff(t.diff, t.name)
      out.push({
        file: parsed.filename,
        added: parsed.additions || undefined,
        removed: parsed.deletions || undefined,
      })
    }
    return out
  }, [tools])

  if (style === 'hidden' || (items.length === 0 && tools.length === 0)) return null

  if (TOOL_CARD_ACTIVITY_STYLES.has(style)) {
    if (tools.length === 0) return null
    return (
      <div className="flex flex-col gap-1.5">
        {tools.map((tool) => (
          <SmartToolElement
            key={tool.id}
            tool={tool}
            toolCardStyle={style as ComponentSlots['chat_page']['tool_card_style']}
            diffStyle={diffStyle}
            shellStyle={shellStyle}
            webSearchStyle={webSearchStyle}
            onRetryTool={onRetryTool}
          />
        ))}
      </div>
    )
  }

  const ACTIVITY_RENDERERS: Partial<Record<typeof style, () => ReactNode>> = {
    kokonut_tool_calls: () =>
      tools.length === 0 ? null : (
        <KokonutAIToolCalls
          defaultOpen={defaultOpen}
          calls={tools.map((t) => {
            const detail =
              extractJsonField(t.args_preview, ['path', 'command', 'cmd', 'query', 'url']) ||
              (t.args_preview ? t.args_preview.slice(0, 48) : undefined)
            return {
              id: t.id,
              tool: t.name,
              detail,
              state:
                t.status === 'running'
                  ? 'input-available'
                  : t.status === 'error'
                    ? 'output-error'
                    : 'output-available',
              durationMs: t.duration_ms,
            }
          })}
        />
      ),
    aui_run_activity: () => {
      const runStatus: RunActivityStatus = status === 'working' ? 'running' : 'complete'
      return (
        <RunActivity
          status={runStatus}
          statusLabel={statusLabel}
          durationLabel={durationLabel}
          entries={runEntries}
          open={open}
          onOpenChange={setOpen}
        />
      )
    },
    aui_tool_timeline: () =>
      timelineSteps.length === 0 ? null : (
        <ToolTimeline
          steps={timelineSteps}
          visibleSteps={timelineSteps.length}
          streaming={status === 'working'}
          open={open}
          onOpenChange={setOpen}
          restingLabel={`${timelineSteps.length} ${timelineSteps.length === 1 ? 'tool step' : 'tool steps'}${durationLabel ? ` · ${durationLabel}` : ''}`}
          activeLabel={statusLabel}
          stats={timelineStats}
        />
      ),
    easeui_tool_chip: () =>
      tools.length === 0 ? null : (
        <div className="flex flex-wrap gap-1.5">
          {tools.map((t) => {
            const IconComp = resolveToolLeadingIcon(t.name)
            const detail =
              extractJsonField(t.args_preview, ['path', 'command', 'cmd', 'query', 'url']) || ''
            const chipLabel = detail
              ? `${t.name} · ${detail.length > 26 ? `${detail.slice(0, 26)}…` : detail}`
              : t.duration_ms !== undefined
                ? `${t.name} · ${t.duration_ms}ms`
                : t.name
            return (
              <EaseToolChip
                key={t.id}
                icon={<IconComp className="h-3.5 w-3.5" />}
                label={chipLabel}
                status={t.status === 'ok' ? 'done' : t.status}
              />
            )
          })}
        </div>
      ),
    minimal_pills: () =>
      tools.length === 0 ? null : (
        <div className="flex flex-wrap gap-1.5">
          {tools.map((t) => (
            <span
              key={t.id}
              className="inline-flex items-center gap-1.5 rounded-full border border-[var(--border)] bg-[var(--card)] px-2.5 py-0.5 font-mono text-[11px] text-[var(--muted)]"
            >
              <Terminal
                size={10.5}
                style={{
                  color:
                    t.status === 'running'
                      ? 'var(--accent)'
                      : t.status === 'error'
                        ? 'var(--destructive)'
                        : 'var(--muted)',
                }}
              />
              <span>{t.name}</span>
              {t.duration_ms !== undefined && <span>{t.duration_ms}ms</span>}
            </span>
          ))}
        </div>
      ),
  }

  const customActivity = ACTIVITY_RENDERERS[style]
  if (customActivity) return <>{customActivity()}</>

  return (
    <div className="zenko-aui-paper rounded-[var(--radius-sm)] border border-[var(--border)]/80 bg-[var(--surface)]/70 px-3 py-1.5">
      <AgentActivity
        items={items}
        status={status}
        activeLabel={statusLabel}
        duration={durationSeconds}
        defaultOpen={defaultOpen}
        collapseOnComplete={false}
        maxHeight={180}
        summary={
          <span className="font-mono text-[11px] tabular-nums">
            {items.length} {items.length === 1 ? 'activity event' : 'activity events'}
            {durationLabel ? ` · ${durationLabel}` : ''}
          </span>
        }
        className="text-[12px]"
      />
    </div>
  )
}

function InteractiveReviewableDiff({
  diff,
  fallbackName,
}: {
  diff: string
  fallbackName: string
}) {
  const parsed = useMemo(() => parseUnifiedDiff(diff, fallbackName), [diff, fallbackName])
  const [hunks, setHunks] = useState<DiffHunk[]>(() => parsed.hunks)

  useEffect(() => {
    setHunks(parsed.hunks)
  }, [parsed.hunks])

  return (
    <ReviewableDiff
      filename={parsed.filename}
      hunks={hunks}
      onKeep={(id) =>
        setHunks((prev) => prev.map((h) => (h.id === id ? { ...h, decision: 'kept' } : h)))
      }
      onDiscard={(id) =>
        setHunks((prev) => prev.map((h) => (h.id === id ? { ...h, decision: 'discarded' } : h)))
      }
    />
  )
}

const TOOL_KIND_RULES: ReadonlyArray<{ keywords: readonly string[]; icon: LucideIcon }> = [
  { keywords: ['shell', 'exec', 'bash', 'cmd', 'run', 'terminal'], icon: SquareTerminal },
  { keywords: ['web', 'http', 'search', 'fetch', 'url'], icon: Globe },
  { keywords: ['mcp', 'json', 'api', 'rpc'], icon: Braces },
  { keywords: ['edit', 'write', 'patch', 'create', 'diff'], icon: FileCode2 },
  { keywords: ['read', 'list', 'find', 'grep', 'glob', 'file', 'dir'], icon: FolderSearch },
]

function resolveToolLeadingIcon(toolName: string): LucideIcon {
  const lower = toolName.toLowerCase()
  const hit = TOOL_KIND_RULES.find((rule) => rule.keywords.some((kw) => lower.includes(kw)))
  return hit ? hit.icon : Wrench
}

function CollapsibleAuiToolCall({
  tool,
  showIcon = false,
}: {
  tool: NormalizedToolStep
  showIcon?: boolean
}) {
  // Never auto-open tool cards — only expand when explicitly clicked by the user.
  const [open, setOpen] = useState(false)
  const query =
    extractJsonField(tool.args_preview, ['path', 'command', 'cmd', 'query', 'url', 'name']) ||
    (tool.args_preview ? tool.args_preview.slice(0, 48) : '')
  const IconComp = showIcon ? resolveToolLeadingIcon(tool.name) : null

  return (
    <ToolCall
      label={tool.name}
      activeLabel={`Running ${tool.name}`}
      query={query}
      request={tool.args_preview || ''}
      result={(tool.diff || tool.content || (tool.status === 'running' ? 'Running...' : '(empty)')).slice(
        0,
        2400,
      )}
      running={tool.status === 'running'}
      failed={tool.status === 'error'}
      open={open}
      onOpenChange={setOpen}
      leadingIcon={IconComp ? <IconComp className="size-4" /> : undefined}
    />
  )
}

function classifyBeuiToolKind(toolName: string): ToolResultKind {
  const lower = toolName.toLowerCase()
  if (lower.includes('shell') || lower.includes('exec') || lower.includes('bash') || lower.includes('cmd')) {
    return 'terminal'
  }
  if (lower.includes('web') || lower.includes('http') || lower.includes('mcp') || lower.includes('fetch')) {
    return 'request'
  }
  return 'custom'
}

/**
 * 3. Smart Tool Element Renderer — dispatches cleanly to:
 *    ToolError | WebSearch | FileTree | CodeDiff | ReviewableDiff | TerminalBlock | CodeRunner | ToolCall | BeUI ToolResult
 */
export function SmartToolElement({
  tool,
  toolCardStyle = 'aui_smart',
  diffStyle = 'aui_code_diff',
  shellStyle = 'aui_terminal_block',
  webSearchStyle = 'aui_web_search',
  onRetryTool,
}: {
  tool: NormalizedToolStep
  toolCardStyle?: ComponentSlots['chat_page']['tool_card_style']
  diffStyle?: DiffStyle
  shellStyle?: ShellStyle
  webSearchStyle?: WebSearchStyle
  onRetryTool?: (toolName: string, argsPreview?: string) => void
}) {
  const [retrying, setRetrying] = useState(false)

  const renderDetailedCard = () => (
    <div className="zenko-aui-paper rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] p-3 text-[12px]">
      <div className="flex items-center justify-between gap-2">
        <span className="inline-flex items-center gap-1.5 font-mono font-medium text-[var(--fg)]">
          <Terminal size={12.5} style={{ color: 'var(--accent)' }} />
          {tool.name}
        </span>
        <span
          className="font-mono text-[11px]"
          style={{ color: tool.status === 'error' ? 'var(--destructive)' : 'var(--muted)' }}
        >
          {tool.status}
          {tool.duration_ms !== undefined ? ` · ${tool.duration_ms}ms` : ''}
        </span>
      </div>
      <CompactScrollArea autoHeight className="mt-2" viewportClassName="max-h-48 pr-2.5">
        <pre className="whitespace-pre-wrap font-mono text-[11.5px] text-[var(--muted)]">
          {(tool.diff || tool.content).slice(0, 1400)}
        </pre>
      </CompactScrollArea>
    </div>
  )

  const STYLE_RENDERERS: Partial<
    Record<ComponentSlots['chat_page']['tool_card_style'], () => ReactNode>
  > = {
    terminal_line: () => (
      <div className="flex items-center gap-2 font-mono text-[11.5px] text-[var(--muted)]">
        <span style={{ color: 'var(--accent)' }}>--&gt;</span>
        <span className="font-semibold text-[var(--fg)]">{tool.name}</span>
        <span style={{ color: tool.status === 'error' ? 'var(--destructive)' : 'var(--muted)' }}>
          {tool.status}
        </span>
        {tool.duration_ms !== undefined && <span>{tool.duration_ms}ms</span>}
      </div>
    ),
    beui_tool_result: () => (
      <ToolResult
        tool={tool.name}
        title={tool.args_preview ? tool.args_preview.slice(0, 72) : tool.name}
        kind={classifyBeuiToolKind(tool.name)}
        status={tool.status === 'ok' ? 'success' : tool.status}
        meta={tool.duration_ms !== undefined ? `${tool.duration_ms}ms` : undefined}
        defaultOpen={false}
        autoOpenOnRun={false}
        collapseOnComplete={false}
        copyText={tool.diff || tool.content}
      >
        <ToolResultOutput language={tool.diff ? 'diff' : 'bash'}>
          {(tool.diff || tool.content || (tool.status === 'running' ? 'Running...' : '(empty output)')).slice(
            0,
            2400,
          )}
        </ToolResultOutput>
      </ToolResult>
    ),
    aui_tool_call: () => <CollapsibleAuiToolCall tool={tool} />,
    aui_tool_call_icon: () => <CollapsibleAuiToolCall tool={tool} showIcon />,
    aui_terminal_block: () => {
      const sh = parseShellOrCodeTool(tool.name, tool.args_preview, tool.content, tool.status === 'error')
      return (
        <TerminalBlock
          command={sh.command}
          cwd={sh.cwd}
          lines={sh.stdoutLines}
          stderr={sh.stderrLines}
          visibleCount={sh.stdoutLines.length + sh.stderrLines.length}
          done={tool.status !== 'running'}
          exitCode={sh.exitCode}
          durationMs={tool.duration_ms}
          maxCollapsedLines={12}
        />
      )
    },
    aui_code_runner: () => {
      const sh = parseShellOrCodeTool(tool.name, tool.args_preview, tool.content, tool.status === 'error')
      const outLines = [...sh.stdoutLines, ...sh.stderrLines]
      return (
        <CodeRunner
          language={sh.language}
          code={sh.command}
          state={tool.status === 'running' ? 'running' : tool.status === 'error' ? 'error' : 'ok'}
          output={outLines.length > 0 ? outLines : [tool.status === 'running' ? 'Running...' : '(no output)']}
          durationMs={tool.duration_ms}
          onRun={onRetryTool ? () => onRetryTool(tool.name, tool.args_preview) : undefined}
        />
      )
    },
    detailed_card: renderDetailedCard,
    pill_chips: renderDetailedCard,
  }

  const styledRenderer = STYLE_RENDERERS[toolCardStyle]
  if (styledRenderer) return <>{styledRenderer()}</>

  // Default: `aui_smart` — declarative dispatch across specialized Assistant-UI Elements
  const smartKind = classifySmartTool(tool.name, Boolean(tool.diff), tool.status === 'error')

  const SMART_DISPATCH: Record<typeof smartKind, () => ReactNode> = {
    error: () => {
      const target =
        extractJsonField(tool.args_preview, ['path', 'command', 'cmd', 'query', 'url']) ||
        tool.args_preview?.slice(0, 64) ||
        'execution failed'
      return (
        <ToolError
          name={tool.name}
          target={target}
          message={(tool.content || 'Tool returned an error').slice(0, 800)}
          attempt={1}
          maxAttempts={3}
          retrying={retrying}
          onRetry={
            onRetryTool
              ? () => {
                  setRetrying(true)
                  onRetryTool(tool.name, tool.args_preview)
                  window.setTimeout(() => setRetrying(false), 1200)
                }
              : undefined
          }
        />
      )
    },
    web_search: () => {
      const ws = parseWebSearchFromTool(tool.args_preview, tool.content)
      if (webSearchStyle === 'rareui_web_search') {
        return (
          <RareWebSearch
            query={ws.query}
            status={tool.status === 'running' ? 'searching' : 'done'}
            defaultOpen={true}
            sources={ws.results.map((r, idx) => ({
              id: `${tool.id}-${idx}`,
              title: r.title,
              url: r.url || `https://${r.domain}`,
              snippet: r.domain,
            }))}
          />
        )
      }
      return (
        <WebSearch
          query={ws.query}
          results={ws.results}
          visibleResults={ws.results.length}
          searching={tool.status === 'running'}
          cycle={1}
        />
      )
    },
    file_tree: () => {
      const tree = parseFileTreeFromTool(tool.name, tool.args_preview, tool.content)
      if (tree.nodes.length === 0) return <CollapsibleAuiToolCall tool={tool} />
      return (
        <FileTree
          title={tree.title}
          nodes={tree.nodes}
          visibleCount={tree.nodes.length}
          totalAdditions={tree.totalAdditions}
          totalDeletions={tree.totalDeletions}
        />
      )
    },
    diff: () => {
      const diffText = tool.diff || ''
      const fallbackName =
        extractJsonField(tool.args_preview, ['path', 'file', 'target']) || tool.name
      if (diffStyle === 'kokonut_diff') {
        return <KokonutAIDiffReview files={unifiedDiffToKokonutFiles(diffText, fallbackName)} />
      }
      if (diffStyle === 'aui_reviewable_diff') {
        return <InteractiveReviewableDiff diff={diffText} fallbackName={fallbackName} />
      }
      if (diffStyle === 'minimal_inline') {
        return (
          <pre className="overflow-x-auto border-l-2 border-[var(--border)] pl-3 font-mono text-[11.5px] leading-relaxed text-[var(--muted)]">
            {diffText.slice(0, 2000)}
          </pre>
        )
      }
      const parsed = parseUnifiedDiff(diffText, fallbackName)
      return (
        <CodeDiff
          filename={parsed.filename}
          additions={parsed.additions}
          deletions={parsed.deletions}
          lines={parsed.lines}
          cycle={1}
        />
      )
    },
    code_runner: () => {
      const sh = parseShellOrCodeTool(tool.name, tool.args_preview, tool.content, false)
      const out = [...sh.stdoutLines, ...sh.stderrLines]
      return (
        <CodeRunner
          language={sh.language}
          code={sh.command}
          state={tool.status === 'running' ? 'running' : 'ok'}
          output={out.length > 0 ? out : [tool.status === 'running' ? 'Running...' : '(ok)']}
          durationMs={tool.duration_ms}
          onRun={onRetryTool ? () => onRetryTool(tool.name, tool.args_preview) : undefined}
        />
      )
    },
    shell: () => {
      const sh = parseShellOrCodeTool(tool.name, tool.args_preview, tool.content, false)
      if (shellStyle === 'aimran_copy_cmd') {
        return (
          <div className="flex flex-col gap-2">
            <AimranCopyCommand command={sh.command} accent="var(--accent)" fullWidth />
            {sh.stdoutLines.length > 0 && (
              <pre className="zenko-thin-scroll max-h-40 overflow-auto rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] px-3 py-2 font-mono text-[11.5px] leading-relaxed text-[var(--muted)]">
                {sh.stdoutLines.slice(0, 20).join('\n')}
              </pre>
            )}
          </div>
        )
      }
      if (shellStyle === 'aui_code_runner') {
        const out = [...sh.stdoutLines, ...sh.stderrLines]
        return (
          <CodeRunner
            language={sh.language}
            code={sh.command}
            state={tool.status === 'running' ? 'running' : 'ok'}
            output={out.length > 0 ? out : [tool.status === 'running' ? 'Running...' : '(ok)']}
            durationMs={tool.duration_ms}
            onRun={onRetryTool ? () => onRetryTool(tool.name, tool.args_preview) : undefined}
          />
        )
      }
      if (shellStyle === 'minimal_mono') {
        return (
          <div className="border-l-2 border-[var(--border)] pl-3 font-mono text-[11.5px]">
            <div className="text-[var(--fg)]/85">$ {sh.command}</div>
            {sh.stdoutLines.length > 0 && (
              <pre className="mt-1 whitespace-pre-wrap text-[var(--muted)]">
                {sh.stdoutLines.slice(0, 20).join('\n')}
              </pre>
            )}
          </div>
        )
      }
      return (
        <TerminalBlock
          command={sh.command}
          cwd={sh.cwd}
          lines={sh.stdoutLines}
          stderr={sh.stderrLines}
          visibleCount={sh.stdoutLines.length + sh.stderrLines.length}
          done={tool.status !== 'running'}
          exitCode={sh.exitCode}
          durationMs={tool.duration_ms}
          maxCollapsedLines={12}
        />
      )
    },
    generic: () => <CollapsibleAuiToolCall tool={tool} />,
  }

  return <>{SMART_DISPATCH[smartKind]()}</>
}

const SKILL_PILL_CLASS: Record<
  Exclude<SkillBadgeStyle, 'hidden' | 'aui_memory_chips'>,
  string
> = {
  pill_glow:
    'inline-flex w-fit cursor-pointer items-center gap-1.5 rounded-full border border-[var(--accent)]/35 bg-[color-mix(in_srgb,var(--accent)_10%,var(--card))] px-2.5 py-0.5 text-[11.5px] text-[var(--fg)] transition-colors hover:border-[var(--accent)]/60',
  minimal_outline:
    'inline-flex w-fit cursor-pointer items-center gap-1.5 rounded-full border border-[var(--border)] bg-transparent px-2.5 py-0.5 text-[11.5px] text-[var(--muted)] transition-colors hover:border-[var(--accent)]/45 hover:text-[var(--fg)]',
  accent_solid:
    'inline-flex w-fit cursor-pointer items-center gap-1.5 rounded-full bg-[var(--accent)] px-2.5 py-0.5 text-[11.5px] font-medium text-[var(--primary-fg)] transition-opacity hover:opacity-90',
  compact_chip:
    'inline-flex w-fit cursor-pointer items-center gap-1 rounded-md border border-[var(--border)] bg-[var(--surface)] px-2 py-0.5 font-mono text-[10.5px] text-[var(--muted)] transition-colors hover:border-[var(--accent)]/45 hover:text-[var(--fg)]',
}

/** 4. Skill & Memory Badges (MemoryChips default | themed skill pills) */
export function SkillAndMemoryBadges({
  style = 'aui_memory_chips',
  chips,
  skills,
  onToggleSteps,
}: {
  style?: SkillBadgeStyle
  chips: readonly MemoryChip[]
  skills: readonly string[]
  onToggleSteps?: () => void
}) {
  if (style === 'hidden') return null

  if (style === 'aui_memory_chips') {
    if (chips.length === 0) return null
    const label =
      skills.length > 0 && chips.length === skills.length
        ? skills.length === 1
          ? 'skill used'
          : `${skills.length} skills`
        : undefined
    return (
      <div className="mb-2">
        <MemoryChips compact label={label} chips={chips} />
      </div>
    )
  }

  if (skills.length === 0) return null

  return (
    <div className="mb-2 flex flex-wrap items-center gap-1.5">
      {skills.map((skill) => (
        <button
          key={skill}
          type="button"
          onClick={onToggleSteps}
          className={SKILL_PILL_CLASS[style]}
        >
          <Zap
            size={12}
            style={{ color: style === 'accent_solid' ? 'var(--primary-fg)' : 'var(--accent)' }}
          />
          <span className="font-medium">Skill used</span>
          <span className="opacity-65">·</span>
          <span className="font-mono text-[11px]">{skill}</span>
          <ArrowUpRight size={11.5} className="opacity-65" />
        </button>
      ))}
    </div>
  )
}

/** 5. Source Citations Renderer (BeUI Sources | KokonutUI Monogram Citations | Minimal Pills) */
export function CitationsDisplay({
  citations,
  style = 'beui_citations',
}: {
  citations: CitationItem[]
  style?: CitationsStyle
}) {
  if (citations.length === 0) return null

  if (style === 'kokonut_citations') {
    return (
      <KokonutAICitations
        sources={citations.map((c) => {
          const domainStr = typeof c.domain === 'string' ? c.domain : undefined
          const titleStr = typeof c.title === 'string' ? c.title : domainStr || c.url || c.id
          return {
            id: c.id,
            title: titleStr,
            url: c.url || (domainStr ? `https://${domainStr}` : '#'),
            site: domainStr,
          }
        })}
      />
    )
  }

  if (style === 'minimal_pills') {
    return (
      <div className="inline-flex flex-wrap items-center gap-1.5">
        {citations.map((c) => (
          <a
            key={c.id}
            href={c.url || '#'}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1 rounded-full border border-[var(--border)] bg-[var(--card)]/70 px-2 py-0.5 font-mono text-[10.5px] text-[var(--muted)] transition-colors hover:border-[var(--accent)]/50 hover:text-[var(--fg)]"
          >
            <Globe size={10} style={{ color: 'var(--accent)' }} />
            <span className="max-w-[140px] truncate">{c.domain || c.title}</span>
          </a>
        ))}
      </div>
    )
  }

  return <Citations citations={citations} className="w-auto" />
}

/** 5b. Assistant Message Actions Bar (Copy + Retry + inline Citations + tok · cached) */
export function AssistantMessageFooter({
  style = 'aui_message_actions',
  citationsStyle = 'beui_citations',
  text,
  citations,
  tokenMeta,
  onRegenerate,
  onToggleSteps,
}: {
  style?: MessageActionsStyle
  citationsStyle?: CitationsStyle
  text: string
  citations: CitationItem[]
  tokenMeta?: ReactNode
  onRegenerate?: () => void
  onToggleSteps?: () => void
}) {
  const [copied, setCopied] = useState(false)
  const [regenerating, setRegenerating] = useState(false)

  const handleCopy = () => {
    navigator.clipboard?.writeText(text).catch(() => {})
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1600)
  }

  const handleRegenerate = () => {
    if (!onRegenerate) return
    setRegenerating(true)
    onRegenerate()
    window.setTimeout(() => setRegenerating(false), 900)
  }

  if (style === 'minimal') {
    return (
      <div className="mt-2.5 flex flex-wrap items-center gap-3 text-[11.5px] text-[var(--muted)]/80">
        <button
          type="button"
          onClick={handleCopy}
          className="inline-flex cursor-pointer items-center gap-1 hover:text-[var(--fg)]"
        >
          {copied ? <Check size={13} style={{ color: 'var(--accent)' }} /> : <Copy size={13} />}
          {copied ? 'Copied' : 'Copy'}
        </button>
        {onRegenerate && (
          <button
            type="button"
            onClick={handleRegenerate}
            className="inline-flex cursor-pointer items-center gap-1 hover:text-[var(--fg)]"
          >
            <RotateCcw size={12.5} className={cn(regenerating && 'animate-spin')} />
            Retry
          </button>
        )}
        <CitationsDisplay citations={citations} style={citationsStyle} />
        {tokenMeta}
      </div>
    )
  }

  return (
    <div className="mt-2 flex flex-wrap items-center gap-2.5">
      <MessageActions
        copied={copied}
        regenerating={regenerating}
        onCopy={handleCopy}
        onRegenerate={onRegenerate ? handleRegenerate : undefined}
        onMore={onToggleSteps}
      />
      <CitationsDisplay citations={citations} style={citationsStyle} />
      {tokenMeta}
    </div>
  )
}

/** 6. HITL Permission Grant Card (PermissionGrant default | BeUI ToolApproval | EaseUI ToolApproval | Compact Card) */
export function ApprovalElement({
  approval,
  style = 'aui_permission_grant',
  onResolve,
}: {
  approval: ApprovalRecord
  style?: ComponentSlots['chat_page']['approval_style']
  onResolve: (id: string, decision: 'approve' | 'deny' | 'always_allow') => void
}) {
  const [scope, setScope] = useState<GrantScope | 'pending'>('pending')
  const [easeStatus, setEaseStatus] = useState<'pending' | 'approved' | 'denied'>('pending')
  const [kokoState, setKokoState] = useState<
    'approval-requested' | 'approval-responded' | 'output-denied'
  >('approval-requested')

  if (style === 'kokonut_approval') {
    const cmdOrArgs =
      extractJsonField(approval.args_preview, ['command', 'cmd']) || approval.args_preview || '{}'
    return (
      <KokonutAIToolApproval
        calls={[
          {
            id: approval.id,
            tool: approval.tool_name,
            title: `Allow ${approval.tool_name}`,
            input: cmdOrArgs,
            reason: `Risk: ${approval.risk} · Synced live with Zenko policy guard`,
            destructive: approval.risk === 'dangerous',
            state: kokoState,
          },
        ]}
        onRespond={(_id, approved, always) => {
          setKokoState(approved ? 'approval-responded' : 'output-denied')
          onResolve(approval.id, !approved ? 'deny' : always ? 'always_allow' : 'approve')
        }}
      />
    )
  }

  if (style === 'easeui_approval') {
    return (
      <div className="flex flex-col gap-1.5">
        <EaseToolApproval
          title={`Allow ${approval.tool_name} (${approval.risk} risk)`}
          description={approval.args_preview || '{}'}
          status={easeStatus}
          onApprove={() => {
            setEaseStatus('approved')
            onResolve(approval.id, 'approve')
          }}
          onDeny={() => {
            setEaseStatus('denied')
            onResolve(approval.id, 'deny')
          }}
        />
        {easeStatus === 'pending' && (
          <div className="flex justify-end">
            <button
              type="button"
              onClick={() => {
                setEaseStatus('approved')
                onResolve(approval.id, 'always_allow')
              }}
              className="cursor-pointer font-mono text-[11px] text-[var(--muted)] transition-colors hover:text-[var(--accent)]"
            >
              Always allow {approval.tool_name} in this session
            </button>
          </div>
        )}
      </div>
    )
  }

  if (style === 'aui_permission_grant') {
    const parsed = parsePermissionGrantProps(approval)
    return (
      <PermissionGrant
        capability={parsed.capability}
        requester={parsed.requester}
        reach={parsed.reach}
        scope={scope}
        onGrant={(chosen) => {
          setScope(chosen)
          const DECISION_MAP: Record<GrantScope, 'approve' | 'always_allow' | 'deny'> = {
            session: 'approve',
            always: 'always_allow',
            denied: 'deny',
          }
          onResolve(approval.id, DECISION_MAP[chosen])
        }}
      />
    )
  }

  if (style === 'beui_approval') {
    return (
      <ToolApproval
        tool={approval.tool_name}
        title={`Allow ${approval.tool_name} to run?`}
        description={`Risk: ${approval.risk} · Approval synced live with Telegram`}
        status="pending"
        defaultOpen={true}
        parameters={[
          { id: 'risk', label: 'Risk Level', value: approval.risk },
          {
            id: 'args',
            label: 'Arguments',
            value: <ToolApprovalCode code={approval.args_preview || '{}'} language="json" />,
          },
        ]}
        onApprove={() => onResolve(approval.id, 'approve')}
        onAlwaysAllow={() => onResolve(approval.id, 'always_allow')}
        onDeny={() => onResolve(approval.id, 'deny')}
      />
    )
  }

  return (
    <div
      className="zenko-aui-paper rounded-[var(--radius)] border p-4"
      style={{
        borderColor: 'color-mix(in srgb, var(--accent) 48%, var(--border))',
        backgroundColor: 'color-mix(in srgb, var(--accent) 7%, var(--card))',
      }}
    >
      <div className="mb-2 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-[13.5px] font-semibold">
          <ShieldAlert size={15} style={{ color: 'var(--accent)' }} />
          <span>Approval Required</span>
          <code className="rounded bg-[var(--bg)] px-1.5 py-0.5 font-mono text-[12px]">
            {approval.tool_name}
          </code>
        </div>
        <span className="rounded-full border border-[var(--border)] px-2 py-0.5 font-mono text-[11px] text-[var(--muted)]">
          {approval.risk}
        </span>
      </div>
      <pre className="mb-3 overflow-x-auto rounded-lg bg-[var(--bg)] p-2.5 font-mono text-[12px] text-[var(--muted)]">
        {approval.args_preview}
      </pre>
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={() => onResolve(approval.id, 'approve')}
          className="z-btn z-btn-primary z-btn-sm"
        >
          Approve
        </button>
        <button
          type="button"
          onClick={() => onResolve(approval.id, 'always_allow')}
          className="z-btn z-btn-secondary z-btn-sm"
        >
          Always Allow
        </button>
        <button
          type="button"
          onClick={() => onResolve(approval.id, 'deny')}
          className="z-btn z-btn-danger z-btn-sm"
        >
          Deny
        </button>
      </div>
    </div>
  )
}

/** 7. Markdown Code Block Element (CodeRunner default | EaseUI CodeBlock | RareUI CodeBlock | Borderless Minimal | Classic Bordered) */
export function MarkdownCodeBlock({
  children,
  className,
  style = 'aui_code_runner',
}: {
  children: ReactNode
  className?: string
  style?: CodeBlockStyle
}) {
  const [copied, setCopied] = useState(false)
  const raw = String(children).endsWith('\n') ? String(children).slice(0, -1) : String(children)
  const lang = className?.startsWith('language-') ? className.slice('language-'.length) : 'code'

  if (style === 'aimran_copy_cmd') {
    const firstLine = raw.split('\n').find((l) => l.trim().length > 0) || raw
    const isMultiLine = raw.includes('\n')
    return (
      <div className="my-3 flex flex-col gap-2">
        <AimranCopyCommand
          command={isMultiLine ? firstLine : raw}
          prefix={lang === 'bash' || lang === 'sh' || lang === 'shell' ? '$' : '>'}
          accent="var(--accent)"
          fullWidth
        />
        {isMultiLine && (
          <pre className="zenko-thin-scroll overflow-x-auto rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)] px-3.5 py-2.5 font-mono text-[12.5px] leading-relaxed text-[var(--fg)]">
            <code>{raw}</code>
          </pre>
        )}
      </div>
    )
  }

  if (style === 'easeui_code') {
    return (
      <div className="my-3">
        <EaseCodeBlock code={raw} label={lang} />
      </div>
    )
  }

  if (style === 'rareui_code') {
    return (
      <div className="my-3">
        <RareCodeBlock
          code={raw}
          language={lang === 'code' ? 'text' : lang}
          accent={readThemeAccentHex()}
          showLineNumbers
        />
      </div>
    )
  }

  if (style === 'aui_code_runner') {
    return (
      <div className="my-3">
        <CodeRunner language={lang} code={raw} state="idle" output={[]} />
      </div>
    )
  }

  const isMinimal = style === 'minimal_code'

  return (
    <div
      className={cn(
        'my-3 overflow-hidden',
        isMinimal
          ? 'border-t border-b border-[var(--border)]/65 bg-transparent'
          : 'rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)]',
      )}
    >
      <div
        className={cn(
          'flex items-center justify-between px-3 py-1.5 text-[11px] text-[var(--muted)]',
          !isMinimal && 'border-b border-[var(--border)]',
        )}
      >
        <span className="font-mono">{lang}</span>
        <button
          type="button"
          onClick={() => {
            navigator.clipboard?.writeText(raw).catch(() => {})
            setCopied(true)
            window.setTimeout(() => setCopied(false), 1600)
          }}
          className="inline-flex cursor-pointer items-center gap-1 text-[11px] text-[var(--muted)] hover:text-[var(--fg)]"
        >
          {copied ? <Check size={12} style={{ color: 'var(--accent)' }} /> : <Copy size={12} />}
          {copied ? 'Copied' : 'Copy'}
        </button>
      </div>
      <pre className="overflow-x-auto px-3.5 py-2.5 font-mono text-[12.5px] leading-relaxed text-[var(--fg)]">
        <code>{raw}</code>
      </pre>
    </div>
  )
}

/** 8. Markdown DataTable Wrapper (uses Assistant-UI sortable DataTable or EaseUI Table with resizable columns) */
export function MarkdownDataTable({
  children,
  tableStyle,
}: {
  children?: ReactNode
  tableStyle?: UiCustomization['surface']['table_style']
}) {
  const extracted = useMemo(() => extractMarkdownTableData(children), [children])
  if (!extracted) {
    return (
      <div className="my-3 overflow-x-auto">
        <table className="w-full border-collapse text-[13px]">{children}</table>
      </div>
    )
  }
  return (
    <div className="my-3.5">
      <DataTable
        columns={extracted.columns}
        rows={extracted.rows}
        tableStyle={tableStyle}
      />
    </div>
  )
}
